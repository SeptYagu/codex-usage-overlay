//! App-managed installation through DSH Desktop's own CLI, never its private RPC.
//!
//! No credential store is inspected. Only process executable metadata, the public
//! profile manifest, and our own embedded package are read. Successful installation
//! is NOT a health check: callers must still discover and authenticate the bridge.
use serde_json::Value;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::Mutex, time::timeout};

const BRIDGE_NAME: &str = "dsh-codex-usage-overlay-bridge";
const EXE_NAME: &str = "DeepSeek Harness.exe";
const CLI_RELATIVE: &str =
    "resources/app.asar/dsh/node_modules/@deepseek-ai/dsh-desktop-host/lib/cli.js";
const INSTALL_BUDGET: Duration = Duration::from_secs(175);
const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_DISCOVERY: usize = 64 * 1024;
static INSTALL_LOCK: Mutex<()> = Mutex::const_new(());
const FILES: [(&str, &str); 5] = [
    (
        "package.json",
        include_str!("../../../integrations/dsh-bridge/package.json"),
    ),
    (
        "index.js",
        include_str!("../../../integrations/dsh-bridge/index.js"),
    ),
    (
        "file-security.js",
        include_str!("../../../integrations/dsh-bridge/file-security.js"),
    ),
    (
        "cordis.patch.yml",
        include_str!("../../../integrations/dsh-bridge/cordis.patch.yml"),
    ),
    (
        "README.md",
        include_str!("../../../integrations/dsh-bridge/README.md"),
    ),
];
const DISCOVER_SCRIPT: &str = r#"$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); $p=@(Get-CimInstance Win32_Process -Filter "Name = 'DeepSeek Harness.exe'" | ForEach-Object { $_.ExecutablePath } | Where-Object { $_ } | Sort-Object -Unique); ConvertTo-Json -InputObject $p -Compress"#;
// Paths are supplied as environment data, never interpolated into PowerShell.
const SECURE_DIRECTORY_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
try {
  $p = $env:CODEX_OVERLAY_SETUP_DIRECTORY
  if (([IO.File]::GetAttributes($p) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { exit 1 }
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $me = $identity.User
  $system = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
  $before = [IO.Directory]::GetAccessControl($p)
  $owner = $before.GetOwner([Security.Principal.SecurityIdentifier]).Value
  if ($owner -ne $me.Value) {
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if ($owner -ne 'S-1-5-32-544' -or -not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { exit 1 }
  }
  $acl = [Security.AccessControl.DirectorySecurity]::new()
  $acl.SetAccessRuleProtection($true, $false)
  $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
  foreach ($sid in @($me, $system)) {
    $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid,
      [Security.AccessControl.FileSystemRights]::FullControl, $inherit,
      [Security.AccessControl.PropagationFlags]::None,
      [Security.AccessControl.AccessControlType]::Allow)
    $acl.AddAccessRule($rule)
  }
  # Modify DACL only; preserve an elevated creator's Administrators owner.
  [IO.Directory]::SetAccessControl($p, $acl)
  $actual = [IO.Directory]::GetAccessControl($p)
  if (-not $actual.AreAccessRulesProtected -or $actual.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $owner) { exit 1 }
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
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupOutcome {
    pub already_installed: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum BridgeState {
    Absent,
    Selected,
    Disabled,
    Inconsistent,
}

struct Layout {
    executable: PathBuf,
    cli: PathBuf,
}

/// Checks public process metadata without staging a package or modifying a profile.
/// A stopped Desktop is an ordinary waiting state, not an exhausted setup attempt.
pub async fn desktop_available() -> Result<bool, String> {
    #[cfg(windows)]
    {
        discover_running_layout()
            .await
            .map(|layout| layout.is_some())
    }
    #[cfg(not(windows))]
    {
        Err("Automatic DSH Desktop setup is supported on Windows only".into())
    }
}

/// Installs only a new bridge via the official Desktop CLI. Existing disabled
/// packages are never silently re-enabled. Cancellation kills only our child,
/// never the running Desktop or a process tree; DSH owns its package-manager lock.
pub async fn install_bridge() -> Result<SetupOutcome, String> {
    #[cfg(not(windows))]
    {
        Err("Automatic DSH Desktop setup is supported on Windows only".into())
    }
    #[cfg(windows)]
    {
        timeout(INSTALL_BUDGET, async {
            let _guard = INSTALL_LOCK.lock().await;
            install_inner().await
        })
        .await
        .map_err(|_| "DSH bridge setup timed out; DSH was not stopped. Its package manager may still be finishing; wait before retrying and do not remove its locks".to_owned())?
    }
}

#[cfg(windows)]
async fn install_inner() -> Result<SetupOutcome, String> {
    validate_embedded_package()?;
    let layout = discover_layout().await?;
    // Desktop's resolveDesktopPaths defaults to resolveDshHome()/profiles/desktop.
    // Never inherit an agent's profile-local DSH_HOME or DSH_PROFILES_HOME into
    // this separate Desktop CLI. We deliberately support the OS-default Desktop
    // home only, rather than guessing a custom running process's environment.
    let default_home = dirs::home_dir()
        .ok_or("Cannot locate the default DSH Desktop home")?
        .join(".dsh");
    let home = validated_desktop_home(
        &default_home,
        std::env::var_os("DSH_HOME").as_deref(),
        std::env::var_os("DSH_PROFILES_HOME").as_deref(),
    )?;
    let profile = home.join("profiles").join("desktop");
    let manifest = profile.join("package.json");
    match read_profile_state(&manifest)? {
        BridgeState::Selected => return Ok(SetupOutcome { already_installed: true }),
        BridgeState::Disabled => return Err(disabled_error()),
        BridgeState::Inconsistent => return Err("DSH bridge selection is inconsistent; repair it in DSH's plugin manager before retrying".into()),
        BridgeState::Absent => {}
    }
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| local_absolute(p))
        .ok_or("Local application data is unavailable for DSH bridge setup")?;
    let root = local.join("CodexUsageOverlay").join("dsh-setup");
    ensure_directory(&root)?;
    secure_directory(&root).await?;
    let root_lock = lock_directory(&root)?;
    let package = root.join(format!("bridge-{}", package_fingerprint()));
    ensure_directory(&package)?;
    secure_directory(&package).await?;
    let package_lock = lock_directory(&package)?;
    stage_package(&package)?;
    // Hold deny-write/delete handles through pnpm so validated bytes cannot be
    // swapped after checking. pnpm can still read/copy the local package.
    let file_locks = verify_package(&package)?;
    let mut command = Command::new(&layout.executable);
    command
        .args(install_args(&layout.cli, &package)?)
        .env("ELECTRON_RUN_AS_NODE", "1")
        .env("DSH_HOME", &home)
        .env_remove("DSH_PROFILES_HOME")
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .env_remove("ELECTRON_NO_ASAR")
        .current_dir(&package)
        .stdin(Stdio::null())
        // Installer output may contain private paths or credentials. Never buffer,
        // log, or propagate it, even when the official CLI exits unsuccessfully.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    hide_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "Cannot start DSH's official Desktop installer")?;
    let status = child
        .wait()
        .await
        .map_err(|_| "Cannot wait for DSH's Desktop installer")?;
    drop((file_locks, package_lock, root_lock));
    if !status.success() {
        return Err("DSH's official plugin installer did not complete successfully; inspect its plugin-manager diagnostics in DSH. No installer output was copied".into());
    }
    match read_profile_state(&manifest)? {
        BridgeState::Selected => Ok(SetupOutcome { already_installed: false }),
        BridgeState::Disabled => Err(disabled_error()),
        _ => Err("DSH's installer exited without selecting the bridge; check DSH's plugin manager before retrying".into()),
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.as_os_str()
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(
                right
                    .as_os_str()
                    .to_string_lossy()
                    .trim_end_matches(['\\', '/']),
            )
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn validated_desktop_home(
    default_home: &Path,
    inherited_home: Option<&std::ffi::OsStr>,
    inherited_profiles: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, String> {
    if !local_absolute(default_home) {
        return Err("Cannot locate the default DSH Desktop home".into());
    }
    for (configured, expected) in [
        (inherited_home, default_home.to_path_buf()),
        (inherited_profiles, default_home.join("profiles")),
    ] {
        if let Some(configured) = configured.filter(|v| !v.to_string_lossy().trim().is_empty()) {
            let configured = Path::new(configured);
            if !local_absolute(configured) || !same_path(configured, &expected) {
                return Err("Automatic DSH setup cannot verify a custom or profile-local inherited DSH home; launch the overlay normally with the default DSH Desktop home".into());
            }
        }
    }
    Ok(default_home.to_path_buf())
}

fn disabled_error() -> String {
    "The DSH bridge is already installed but disabled; enable it explicitly in DSH's plugin manager. Automatic setup will not override that choice".into()
}

fn classify_manifest(bytes: &[u8]) -> Result<BridgeState, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "Invalid DSH Desktop profile manifest")?;
    if !value.is_object() {
        return Err("Invalid DSH Desktop profile manifest".into());
    }
    for field in [value.get("dsh"), value.pointer("/dsh/profile")] {
        if field.is_some_and(|v| !v.is_object()) {
            return Err("Invalid DSH Desktop profile configuration".into());
        }
    }
    let mut installed = false;
    for field in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(dependencies) = value.get(field) {
            let object = dependencies
                .as_object()
                .ok_or("Invalid DSH Desktop dependency declarations")?;
            if let Some(spec) = object.get(BRIDGE_NAME) {
                if spec.as_str().filter(|s| !s.trim().is_empty()).is_none() {
                    return Err("Invalid DSH bridge dependency declaration".into());
                }
                installed = true;
            }
        }
    }
    let selected = match value.pointer("/dsh/profile/bundles") {
        None => false,
        Some(bundles) => {
            let bundles = bundles
                .as_array()
                .ok_or("Invalid DSH Desktop bundle selection")?;
            if bundles.iter().any(|v| !v.is_string()) {
                return Err("Invalid DSH Desktop bundle selection".into());
            }
            bundles.iter().any(|v| v.as_str() == Some(BRIDGE_NAME))
        }
    };
    Ok(match (installed, selected) {
        (false, false) => BridgeState::Absent,
        (true, true) => BridgeState::Selected,
        (true, false) => BridgeState::Disabled,
        (false, true) => BridgeState::Inconsistent,
    })
}

fn read_profile_state(path: &Path) -> Result<BridgeState, String> {
    // package.json is public configuration, not a DSH authentication artifact.
    let file = File::open(path).map_err(|_| "The default DSH Desktop profile is unavailable; open DSH Desktop first. Custom DSH homes are not auto-installed")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect the DSH Desktop profile manifest")?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST {
        return Err("Invalid DSH Desktop profile manifest size".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read the DSH Desktop profile manifest")?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("Invalid DSH Desktop profile manifest size".into());
    }
    classify_manifest(&bytes)
}

fn layout_paths(executable: &Path) -> Result<Layout, String> {
    if !local_absolute(executable)
        || !executable
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(EXE_NAME))
    {
        return Err("No supported DSH Desktop installation was found".into());
    }
    let root = executable
        .parent()
        .ok_or("Invalid DSH Desktop installation layout")?;
    Ok(Layout {
        executable: executable.to_path_buf(),
        cli: root.join(CLI_RELATIVE),
    })
}

fn verify_layout(executable: &Path) -> Result<Layout, String> {
    let layout = layout_paths(executable)?;
    let root = executable
        .parent()
        .ok_or("Invalid DSH Desktop installation layout")?;
    // ASAR contents are virtual files: verify the physical archive and official
    // companion launcher/pnpm layout without trying to open cli.js through std::fs.
    for path in [
        layout.executable.clone(),
        root.join("resources/app.asar"),
        root.join("resources/runtime/cli/bin/dsh.cmd"),
        root.join("resources/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs"),
    ] {
        if !fs::metadata(path).is_ok_and(|m| m.is_file()) {
            return Err(
                "The running DSH Desktop does not provide the supported official CLI layout".into(),
            );
        }
    }
    Ok(layout)
}

#[cfg(windows)]
async fn discover_layout() -> Result<Layout, String> {
    discover_running_layout().await?.ok_or_else(|| {
        "Open a supported DSH Desktop installation before connecting the overlay".into()
    })
}

#[cfg(windows)]
async fn discover_running_layout() -> Result<Option<Layout>, String> {
    let mut command = powershell()?;
    command.arg(DISCOVER_SCRIPT);
    command.stdout(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|_| "Cannot discover running DSH Desktop installations")?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("Cannot read DSH Desktop discovery metadata")?;
    let discovery = timeout(Duration::from_secs(15), async {
        let mut bytes = Vec::new();
        (&mut stdout)
            .take(MAX_DISCOVERY as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "Cannot read DSH Desktop discovery metadata")?;
        if bytes.len() > MAX_DISCOVERY {
            return Err("DSH Desktop discovery exceeded its metadata limit");
        }
        if !child
            .wait()
            .await
            .map_err(|_| "Cannot finish DSH Desktop discovery")?
            .success()
        {
            return Err("Cannot enumerate running DSH Desktop executable metadata");
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| "DSH Desktop discovery timed out")??;
    let paths = parse_discovered_paths(&discovery)?;
    if paths.is_empty() {
        return Ok(None);
    }
    let mut layouts = Vec::new();
    for path in paths {
        if let Ok(layout) = verify_layout(&path) {
            if !layouts
                .iter()
                .any(|entry: &Layout| same_path(&entry.executable, &layout.executable))
            {
                layouts.push(layout);
            }
        }
    }
    match layouts.len() {
        0 => Err("Open a supported DSH Desktop installation before connecting the overlay".into()),
        1 => Ok(Some(layouts.remove(0))),
        _ => Err("Multiple DSH Desktop installations are running; close the unused installation before automatic setup".into()),
    }
}

fn parse_discovered_paths(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    if bytes.len() > MAX_DISCOVERY {
        return Err("DSH Desktop discovery exceeded its metadata limit".into());
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "Invalid DSH Desktop discovery metadata")?;
    let values = value
        .as_array()
        .ok_or("Invalid DSH Desktop discovery metadata")?;
    if values.len() > 128 {
        return Err("Too many DSH Desktop executable records".into());
    }
    values
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 32768 && !s.chars().any(char::is_control))
                .map(PathBuf::from)
                .ok_or_else(|| "Invalid DSH Desktop executable metadata".into())
        })
        .collect()
}

fn install_args(cli: &Path, package: &Path) -> Result<Vec<OsString>, String> {
    if !local_absolute(package) || !local_absolute(cli) {
        return Err("DSH setup requires absolute local installation paths".into());
    }
    let mut spec = OsString::from("file:");
    spec.push(package.as_os_str());
    Ok(vec![
        "--expose-internals".into(),
        cli.as_os_str().to_owned(),
        "plugin".into(),
        "--profile".into(),
        "desktop".into(),
        "add".into(),
        "--ignore-scripts".into(),
        "--reporter=silent".into(),
        spec,
    ])
}

fn validate_embedded_package() -> Result<(), String> {
    let manifest: Value =
        serde_json::from_str(FILES[0].1).map_err(|_| "Invalid embedded DSH bridge package")?;
    if manifest["name"] != BRIDGE_NAME
        || manifest
            .pointer("/dsh/bundle/patch")
            .and_then(Value::as_str)
            != Some("./cordis.patch.yml")
        || manifest["version"]
            .as_str()
            .and_then(|s| semver::Version::parse(s).ok())
            .is_none()
    {
        return Err("Invalid embedded DSH bridge package identity".into());
    }
    // This bundled bridge must remain standalone and prebuilt. Never execute
    // package-manager lifecycle hooks or fetch a hidden dependency to prepare it.
    for field in ["dependencies", "optionalDependencies"] {
        if manifest
            .get(field)
            .is_some_and(|v| !v.as_object().is_some_and(|o| o.is_empty()))
        {
            return Err("The embedded DSH bridge must be a standalone prebuilt package".into());
        }
    }
    for script in ["preinstall", "install", "postinstall", "prepare"] {
        if manifest
            .get("scripts")
            .and_then(|v| v.get(script))
            .is_some()
        {
            return Err("The embedded DSH bridge must not require install scripts".into());
        }
    }
    Ok(())
}

fn package_fingerprint() -> String {
    // A deterministic cache key only, not a trust decision. Every staged byte is
    // separately compared with the binary's embedded content before use.
    let mut hash = 0xcbf29ce484222325_u64;
    for (name, content) in FILES {
        for byte in name.bytes().chain([0]).chain(content.bytes()).chain([0]) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    format!("{hash:016x}")
}

fn local_absolute(path: &Path) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    #[cfg(windows)]
    {
        matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    if !local_absolute(path) {
        return Err("DSH setup storage must be an absolute local directory".into());
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        // A Windows drive prefix alone is drive-relative (C:), not its root.
        if !current.has_root() {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
            Ok(_) => return Err("DSH setup refuses linked or non-directory storage".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err("Cannot create DSH setup storage".into()),
                }
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|_| "Cannot inspect DSH setup storage")?;
                if !metadata.is_dir() || is_link(&metadata) {
                    return Err("DSH setup refuses linked or non-directory storage".into());
                }
            }
            Err(_) => return Err("Cannot inspect DSH setup storage".into()),
        }
    }
    Ok(())
}

fn stage_package(directory: &Path) -> Result<(), String> {
    for (name, content) in FILES {
        let path = directory.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(content.as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "Cannot stage the embedded DSH bridge")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("Cannot stage the embedded DSH bridge".into()),
        }
    }
    Ok(())
}

fn verify_package(directory: &Path) -> Result<Vec<File>, String> {
    let entries = fs::read_dir(directory).map_err(|_| "Cannot inspect staged DSH bridge")?;
    for entry in entries {
        let entry = entry.map_err(|_| "Cannot inspect staged DSH bridge")?;
        if !FILES.iter().any(|(name, _)| entry.file_name() == *name) {
            return Err(
                "Unexpected content in DSH setup storage; automatic installation was refused"
                    .into(),
            );
        }
    }
    let mut locks = Vec::new();
    for (name, content) in FILES {
        let path = directory.join(name);
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| "Cannot inspect staged DSH bridge")?;
        if !metadata.is_file() || is_link(&metadata) || metadata.len() != content.len() as u64 {
            return Err("Staged DSH bridge integrity validation failed".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1); // FILE_SHARE_READ only: no write or delete.
        }
        let mut file = options
            .open(&path)
            .map_err(|_| "Cannot lock staged DSH bridge contents")?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(content.len() as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot verify staged DSH bridge contents")?;
        if bytes != content.as_bytes() {
            return Err("Staged DSH bridge integrity validation failed".into());
        }
        locks.push(file);
    }
    Ok(locks)
}

fn lock_directory(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x02000000); // BACKUP_SEMANTICS, no DELETE share.
    }
    options
        .open(path)
        .map_err(|_| "Cannot lock DSH setup storage against replacement".into())
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW; never attach to Desktop's console.
}

#[cfg(windows)]
fn powershell() -> Result<Command, String> {
    let windows = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|p| local_absolute(p))
        .ok_or("Windows process discovery is unavailable")?;
    let path = windows.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    if !path.is_file() {
        return Err("Windows process discovery is unavailable".into());
    }
    let mut command = Command::new(path);
    command
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    hide_window(&mut command);
    Ok(command)
}

#[cfg(windows)]
async fn secure_directory(path: &Path) -> Result<(), String> {
    let mut command = powershell()?;
    command
        .arg(SECURE_DIRECTORY_SCRIPT)
        .env("CODEX_OVERLAY_SETUP_DIRECTORY", path);
    let status = timeout(Duration::from_secs(10), command.status())
        .await
        .map_err(|_| "Securing DSH setup storage timed out")?
        .map_err(|_| "Cannot secure DSH setup storage")?;
    if !status.success() {
        return Err("Cannot restrict DSH setup storage to the current Windows user".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn embedded_package_is_prebuilt_and_fingerprint_is_stable() {
        validate_embedded_package().unwrap();
        assert_eq!(package_fingerprint(), package_fingerprint());
        assert_eq!(package_fingerprint().len(), 16);
    }

    #[test]
    fn manifest_states_do_not_silently_enable_disabled_packages() {
        let mut value = json!({"dependencies":{},"dsh":{"profile":{"bundles":[]}}});
        assert_eq!(
            classify_manifest(&serde_json::to_vec(&value).unwrap()).unwrap(),
            BridgeState::Absent
        );
        value["dependencies"][BRIDGE_NAME] = json!("file:PRIVATE_PATH_SECRET");
        assert_eq!(
            classify_manifest(&serde_json::to_vec(&value).unwrap()).unwrap(),
            BridgeState::Disabled
        );
        value["dsh"]["profile"]["bundles"] = json!([BRIDGE_NAME]);
        assert_eq!(
            classify_manifest(&serde_json::to_vec(&value).unwrap()).unwrap(),
            BridgeState::Selected
        );
        value["dependencies"] = json!({});
        assert_eq!(
            classify_manifest(&serde_json::to_vec(&value).unwrap()).unwrap(),
            BridgeState::Inconsistent
        );
        assert!(!disabled_error().contains("PRIVATE_PATH_SECRET"));
    }

    #[test]
    fn malformed_data_never_appears_in_errors() {
        for bytes in [
            b"PRIVATE_TOKEN_SECRET".as_slice(),
            br#"{"dependencies":"PRIVATE_TOKEN_SECRET"}"#,
            br#"{"dsh":{"profile":{"bundles":"PRIVATE_TOKEN_SECRET"}}}"#,
        ] {
            assert!(!classify_manifest(bytes)
                .unwrap_err()
                .contains("PRIVATE_TOKEN_SECRET"));
        }
        assert!(!parse_discovered_paths(b"PRIVATE_TOKEN_SECRET")
            .unwrap_err()
            .contains("PRIVATE_TOKEN_SECRET"));
        assert!(!parse_discovered_paths(br#"[42,"PRIVATE_TOKEN_SECRET"]"#)
            .unwrap_err()
            .contains("PRIVATE_TOKEN_SECRET"));
    }

    #[test]
    fn discovery_consumes_executable_paths_not_command_lines() {
        assert!(DISCOVER_SCRIPT.contains("ExecutablePath"));
        assert!(!DISCOVER_SCRIPT.contains("CommandLine"));
        assert!(!DISCOVER_SCRIPT.contains("Environment"));
        assert!(parse_discovered_paths(b"[]").unwrap().is_empty());
        assert_eq!(
            parse_discovered_paths(br#"["C:\\Tools\\DeepSeek Harness.exe"]"#)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn shell_metacharacters_are_single_literal_argument_data() {
        #[cfg(windows)]
        let root = PathBuf::from(r"C:\Apps & Users\$(never-execute)\DSH");
        #[cfg(not(windows))]
        let root = PathBuf::from("/Apps & Users/$(never-execute)/DSH");
        let layout = layout_paths(&root.join(EXE_NAME)).unwrap();
        let package = root.join("bridge & literal");
        let args = install_args(&layout.cli, &package).unwrap();
        assert_eq!(args[0], "--expose-internals");
        assert_eq!(args[1], layout.cli.as_os_str());
        assert_eq!(
            args[2..8],
            [
                "plugin",
                "--profile",
                "desktop",
                "add",
                "--ignore-scripts",
                "--reporter=silent"
            ]
        );
        let mut expected = OsString::from("file:");
        expected.push(&package);
        assert_eq!(args[8], expected);
        assert!(layout_paths(&root.join("not-dsh.exe")).is_err());
        assert!(install_args(Path::new("relative"), &package).is_err());
    }

    #[test]
    fn staging_checks_every_embedded_byte_and_refuses_extra_files() {
        let directory =
            std::env::temp_dir().join(format!("overlay-dsh-setup-test-{}", uuid::Uuid::new_v4()));
        ensure_directory(&directory).unwrap();
        stage_package(&directory).unwrap();
        drop(verify_package(&directory).unwrap());
        // Calling stage again never overwrites content that needs verification.
        fs::write(directory.join("index.js"), "PRIVATE_CONTENT_SECRET").unwrap();
        stage_package(&directory).unwrap();
        assert!(!verify_package(&directory)
            .unwrap_err()
            .contains("PRIVATE_CONTENT_SECRET"));
        fs::write(directory.join("index.js"), FILES[1].1).unwrap();
        fs::write(directory.join(".npmrc"), "PRIVATE_CONTENT_SECRET").unwrap();
        assert!(!verify_package(&directory)
            .unwrap_err()
            .contains("PRIVATE_CONTENT_SECRET"));
        // This is a unique, verified temporary test directory, never a live profile.
        assert_eq!(directory.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inherited_agent_homes_cannot_redirect_desktop_installation() {
        #[cfg(windows)]
        let home = PathBuf::from(r"C:\Users\Overlay Test\.dsh");
        #[cfg(not(windows))]
        let home = PathBuf::from("/users/overlay/.dsh");
        assert_eq!(validated_desktop_home(&home, None, None).unwrap(), home);
        assert!(validated_desktop_home(
            &home,
            Some(home.as_os_str()),
            Some(home.join("profiles").as_os_str())
        )
        .is_ok());
        let profile_home = home.join("profiles/desktop");
        assert!(validated_desktop_home(&home, Some(profile_home.as_os_str()), None).is_err());
        assert!(!validated_desktop_home(
            &home,
            Some(std::ffi::OsStr::new("PRIVATE_HOME_SECRET")),
            None
        )
        .unwrap_err()
        .contains("PRIVATE_HOME_SECRET"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn staging_dacl_and_file_locks_allow_copy_but_not_mutation() {
        let root = std::env::temp_dir().join(format!(
            "overlay-dsh-setup-acl-test-{}",
            uuid::Uuid::new_v4()
        ));
        ensure_directory(&root).unwrap();
        secure_directory(&root).await.unwrap();
        // Reapplying must not require WRITE_OWNER/SeSecurityPrivilege.
        secure_directory(&root).await.unwrap();
        let directory_lock = lock_directory(&root).unwrap();
        stage_package(&root).unwrap();
        let locks = verify_package(&root).unwrap();
        let copy = std::env::temp_dir().join(format!(
            "overlay-dsh-setup-copy-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::copy(root.join("index.js"), &copy).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), FILES[1].1.as_bytes());
        assert!(fs::write(root.join("index.js"), "changed").is_err());
        assert!(fs::remove_file(root.join("index.js")).is_err());
        drop((locks, directory_lock));
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        assert_eq!(copy.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_file(copy).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    /// Opt-in integration test: executes the real official CLI, but ONLY against
    /// an owned empty home. It cannot read or mutate the running Host's profile.
    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "requires a running installed DSH Desktop; isolated profile only"]
    async fn official_desktop_cli_installs_locked_embedded_package_in_isolated_home() {
        let layout = discover_layout().await.unwrap();
        let root = std::env::temp_dir().join(format!(
            "overlay-dsh-isolated-cli-test-{}",
            uuid::Uuid::new_v4()
        ));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if self.0.parent() == Some(std::env::temp_dir().as_path())
                    && self
                        .0
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("overlay-dsh-isolated-cli-test-")
                {
                    let _ = fs::remove_dir_all(&self.0);
                }
            }
        }
        ensure_directory(&root).unwrap();
        let _cleanup = Cleanup(root.clone());
        secure_directory(&root).await.unwrap();
        let package = root.join("embedded-bridge");
        ensure_directory(&package).unwrap();
        secure_directory(&package).await.unwrap();
        let directory_lock = lock_directory(&package).unwrap();
        stage_package(&package).unwrap();
        let locks = verify_package(&package).unwrap();
        let home = root.join("isolated-home");
        let profile = home.join("profiles/desktop");
        ensure_directory(&profile).unwrap();
        let manifest = profile.join("package.json");
        fs::write(&manifest, br#"{"name":"overlay-isolated-native-probe","private":true,"type":"module","dsh":{"profile":{"bundles":[]}}}"#).unwrap();
        let mut command = Command::new(&layout.executable);
        command
            .args(install_args(&layout.cli, &package).unwrap())
            .env("ELECTRON_RUN_AS_NODE", "1")
            .env("DSH_HOME", &home)
            .env_remove("DSH_PROFILES_HOME")
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .env_remove("ELECTRON_NO_ASAR")
            .current_dir(&package)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        hide_window(&mut command);
        let result = timeout(Duration::from_secs(90), command.status()).await;
        drop((locks, directory_lock));
        assert!(
            result.unwrap().unwrap().success(),
            "Official CLI rejected locked local package"
        );
        assert_eq!(
            read_profile_state(&manifest).unwrap(),
            BridgeState::Selected
        );
        assert!(profile
            .join("node_modules")
            .join(BRIDGE_NAME)
            .join("index.js")
            .is_file());
    }

    #[cfg(unix)]
    #[test]
    fn staging_refuses_symbolic_links() {
        let root = std::env::temp_dir().join(format!(
            "overlay-dsh-setup-link-test-{}",
            uuid::Uuid::new_v4()
        ));
        ensure_directory(&root).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), root.join("linked")).unwrap();
        assert!(ensure_directory(&root.join("linked")).is_err());
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staging_refuses_a_file_as_directory() {
        let root = std::env::temp_dir().join(format!(
            "overlay-dsh-setup-path-test-{}",
            uuid::Uuid::new_v4()
        ));
        ensure_directory(&root).unwrap();
        let path = root.join("not-directory");
        fs::write(&path, "PRIVATE_PATH_SECRET").unwrap();
        assert!(!ensure_directory(&path)
            .unwrap_err()
            .contains("PRIVATE_PATH_SECRET"));
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(root).unwrap();
    }
}
