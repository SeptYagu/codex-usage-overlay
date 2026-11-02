import { constants } from 'node:fs';
import * as fs from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';

const securityError = () => new Error('Bridge discovery security check failed');

// Run without captured stdio: no credentials, subprocess output, or paths enter logs.
// Build a new, protected DACL rather than trusting inherited permissions or chmod.
const WINDOWS_ACL = String.raw`
$ErrorActionPreference = 'Stop'
try {
  $p = $env:CODEX_OVERLAY_ACL_PATH
  $dir = $env:CODEX_OVERLAY_ACL_KIND -eq 'directory'
  if (([IO.File]::GetAttributes($p) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { exit 1 }
  $me = [Security.Principal.WindowsIdentity]::GetCurrent().User
  $system = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
  if ($dir) {
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
  } else {
    $acl = [Security.AccessControl.FileSecurity]::new()
    $inherit = [Security.AccessControl.InheritanceFlags]::None
  }
  $acl.SetAccessRuleProtection($true, $false)
  # An unelevated creator already owns the path; do not request WRITE_OWNER
  # just to rewrite it. Elevated Windows creators (including Actions) instead
  # default to BUILTIN\Administrators ownership. Only that elevated case may
  # set its own user as owner; never adopt a path owned by another user.
  if ($dir) { $before = [IO.Directory]::GetAccessControl($p) }
  else { $before = [IO.File]::GetAccessControl($p) }
  $owner = $before.GetOwner([Security.Principal.SecurityIdentifier]).Value
  if ($owner -ne $me.Value) {
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if ($owner -ne 'S-1-5-32-544' -or -not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { exit 1 }
    $acl.SetOwner($me)
  }
  foreach ($sid in @($me, $system)) {
    $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid,
      [Security.AccessControl.FileSystemRights]::FullControl, $inherit,
      [Security.AccessControl.PropagationFlags]::None,
      [Security.AccessControl.AccessControlType]::Allow)
    $acl.AddAccessRule($rule)
  }
  # The Windows PowerShell Set-Acl provider also attempts audit-section writes
  # and can require SeSecurityPrivilege. Write only modified DACL/owner sections.
  if ($dir) { [IO.Directory]::SetAccessControl($p, $acl) }
  else { [IO.File]::SetAccessControl($p, $acl) }
  if ($dir) { $actual = [IO.Directory]::GetAccessControl($p) }
  else { $actual = [IO.File]::GetAccessControl($p) }
  if (-not $actual.AreAccessRulesProtected) { exit 1 }
  if ($actual.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $me.Value) { exit 1 }
  $rules = @($actual.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
  if ($rules.Count -ne 2) { exit 1 }
  $found = @{}
  foreach ($r in $rules) {
    $sid = $r.IdentityReference.Value
    if ($sid -ne $me.Value -and $sid -ne $system.Value) { exit 1 }
    if ($r.IsInherited -or $r.AccessControlType -ne 'Allow' -or
        $r.FileSystemRights -ne [Security.AccessControl.FileSystemRights]::FullControl -or
        $r.InheritanceFlags -ne $inherit -or
        $r.PropagationFlags -ne [Security.AccessControl.PropagationFlags]::None) { exit 1 }
    $found[$sid] = $true
  }
  if (-not $found.ContainsKey($me.Value) -or -not $found.ContainsKey($system.Value)) { exit 1 }
  exit 0
} catch { exit 1 }
`;

export async function restrictPath(target, kind, signal) {
  signal?.throwIfAborted();
  if (process.platform === 'win32') {
    const systemRoot = process.env.SystemRoot;
    if (!systemRoot || !path.isAbsolute(systemRoot)) throw securityError();
    const executable = path.join(systemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
    await new Promise((resolve, reject) => {
      let child;
      let timer;
      const fail = () => reject(securityError());
      try {
        child = spawn(executable, ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand',
          Buffer.from(WINDOWS_ACL, 'utf16le').toString('base64')], {
          stdio: 'ignore', windowsHide: true, signal,
          env: { ...process.env, CODEX_OVERLAY_ACL_PATH: target, CODEX_OVERLAY_ACL_KIND: kind },
        });
        timer = setTimeout(() => { child.kill(); fail(); }, 15_000);
        timer.unref();
        child.once('error', () => { clearTimeout(timer); fail(); });
        child.once('exit', (code) => {
          clearTimeout(timer);
          if (code === 0) resolve(); else fail();
        });
      } catch { clearTimeout(timer); fail(); }
    });
  } else {
    const mode = kind === 'directory' ? 0o700 : 0o600;
    await fs.chmod(target, mode);
    const stat = await fs.lstat(target);
    if ((stat.mode & 0o777) !== mode || stat.uid !== process.getuid()) throw securityError();
  }
  signal?.throwIfAborted();
  const stat = await fs.lstat(target);
  if (stat.isSymbolicLink() || (kind === 'directory' ? !stat.isDirectory() : !stat.isFile())) throw securityError();
}

export async function assertSafeDirectory(target) {
  if (typeof target !== 'string' || !path.isAbsolute(target) || target.includes('\0')) throw securityError();
  const absolute = path.resolve(target);
  const root = path.parse(absolute).root;
  // Discovery must stay on a local filesystem, never a share/network root.
  if (process.platform === 'win32' && root.startsWith('\\\\')) throw securityError();
  let current = root;
  const rootStat = await fs.lstat(current);
  if (rootStat.isSymbolicLink() || !rootStat.isDirectory()) throw securityError();
  for (const part of absolute.slice(root.length).split(path.sep).filter(Boolean)) {
    current = path.join(current, part);
    const stat = await fs.lstat(current);
    if (stat.isSymbolicLink() || !stat.isDirectory()) throw securityError();
  }
  return absolute;
}

export async function prepareDirectory(target, signal, secure = restrictPath) {
  if (typeof target !== 'string' || !path.isAbsolute(target)) throw securityError();
  const absolute = path.resolve(target);
  const root = path.parse(absolute).root;
  if (process.platform === 'win32' && root.startsWith('\\\\')) throw securityError();
  let current = root;
  await assertSafeDirectory(current);
  for (const part of absolute.slice(root.length).split(path.sep).filter(Boolean)) {
    signal?.throwIfAborted();
    current = path.join(current, part);
    let created = false;
    try { await fs.mkdir(current, { mode: 0o700 }); created = true; }
    catch (error) { if (error.code !== 'EEXIST') throw securityError(); }
    await assertSafeDirectory(current);
    if (created || current === absolute) await secure(current, 'directory', signal);
  }
  await assertSafeDirectory(absolute);
  return absolute;
}

const sameFile = (a, b) => a.dev === b.dev && a.ino === b.ino;

export async function publishDiscovery(root, record, signal, secure = restrictPath) {
  await assertSafeDirectory(root);
  const finalPath = path.join(root, `${record.instanceId}.json`);
  const temporary = path.join(root, `.${record.instanceId}.${randomBytes(8).toString('hex')}.tmp`);
  let handle;
  let identity;
  let published = false;
  try {
    handle = await fs.open(temporary, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | (constants.O_NOFOLLOW ?? 0), 0o600);
    identity = await handle.stat();
    await secure(temporary, 'file', signal);
    signal?.throwIfAborted();
    await handle.writeFile(`${JSON.stringify(record)}\n`, 'utf8');
    await handle.sync();
    await handle.close();
    handle = undefined;
    await assertSafeDirectory(root);
    const current = await fs.lstat(temporary);
    if (current.isSymbolicLink() || !current.isFile() || !sameFile(current, identity)) throw securityError();
    try { await fs.lstat(finalPath); throw securityError(); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    signal?.throwIfAborted();
    await fs.rename(temporary, finalPath);
    published = true;
    signal?.throwIfAborted();
    return finalPath;
  } catch {
    if (published) await removeOwnedDiscovery(root, record);
    throw securityError();
  } finally {
    await handle?.close().catch(() => {});
    try {
      await assertSafeDirectory(root);
      const stat = await fs.lstat(temporary);
      if (!stat.isSymbolicLink() && stat.isFile() && identity && sameFile(stat, identity)) await fs.unlink(temporary);
    } catch { /* Do not follow/erase a replacement path. */ }
  }
}

export async function removeOwnedDiscovery(root, record) {
  // Never remove the directory, another instance, a replaced record, or a symlink.
  try {
    await assertSafeDirectory(root);
    const target = path.join(root, `${record.instanceId}.json`);
    const stat = await fs.lstat(target);
    if (stat.isSymbolicLink() || !stat.isFile() || stat.size > 16_384) return;
    const handle = await fs.open(target, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    let value;
    let identity;
    try { identity = await handle.stat(); value = JSON.parse(await handle.readFile('utf8')); }
    finally { await handle.close(); }
    if (!sameFile(stat, identity) || value.instanceId !== record.instanceId ||
        value.origin !== record.origin || value.token !== record.token) return;
    const current = await fs.lstat(target);
    if (!current.isSymbolicLink() && current.isFile() && sameFile(current, identity)) await fs.unlink(target);
  } catch { /* Missing, replaced, insecure or unreadable records are never removed. */ }
}
