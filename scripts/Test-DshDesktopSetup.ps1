[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$DesktopExecutable
)
# Exercise the installed, official Desktop CLI against a fresh, isolated home.
# Never launches a Host, reads credentials, or changes the real Desktop profile.
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$exe = (Resolve-Path -LiteralPath $DesktopExecutable).Path
if ((Split-Path $exe -Leaf) -ne 'DeepSeek Harness.exe') { throw 'Expected the official Desktop executable.' }
$launcher = Join-Path (Split-Path $exe -Parent) 'resources\runtime\cli\bin\dsh.cmd'
if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) { throw 'Desktop CLI launcher is unavailable.' }
$fixtureParent = [IO.Path]::GetFullPath((Join-Path $repo 'integrations\dsh-bridge\.test-tmp'))
$fixture = Join-Path $fixtureParent ('desktop-cli-' + [guid]::NewGuid().ToString('N'))
$homeDir = Join-Path $fixture 'home'
$profile = Join-Path $homeDir 'profiles\desktop'
$package = Join-Path $fixture 'embedded-bridge'
$manifest = Join-Path $profile 'package.json'
New-Item -ItemType Directory -Path $profile, $package -Force | Out-Null
[IO.File]::WriteAllText($manifest, '{"name":"overlay-isolated-desktop-probe","private":true,"type":"module","dsh":{"profile":{"bundles":[]}}}', [Text.UTF8Encoding]::new($false))
foreach ($name in @('package.json', 'index.js', 'file-security.js', 'cordis.patch.yml', 'README.md')) {
    Copy-Item -LiteralPath (Join-Path $repo "integrations\dsh-bridge\$name") -Destination (Join-Path $package $name)
}
$previousHome = $env:DSH_HOME
$previousProfilesHome = $env:DSH_PROFILES_HOME
try {
    $env:DSH_HOME = $homeDir
    Remove-Item Env:DSH_PROFILES_HOME -ErrorAction SilentlyContinue
    & $launcher plugin --profile desktop add --ignore-scripts --offline "file:$package"
    if ($LASTEXITCODE -ne 0) { throw "Official Desktop CLI failed in the isolated profile (exit $LASTEXITCODE)." }
    $installed = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
    if (-not $installed.dependencies.'dsh-codex-usage-overlay-bridge') { throw 'CLI did not register the dependency.' }
    if ('dsh-codex-usage-overlay-bridge' -notin $installed.dsh.profile.bundles) { throw 'CLI did not activate the new bundle.' }
    $entry = Join-Path $profile 'node_modules\dsh-codex-usage-overlay-bridge\index.js'
    if (-not (Test-Path -LiteralPath $entry -PathType Leaf)) { throw 'Installed Host entry is missing.' }
    Write-Output 'PASS: official Desktop CLI installed and selected the embedded bundle without manual pairing.'
    Write-Output 'Boundary: isolated profile only; no Host/HMR, authentication, live quota or pause was exercised.'
} finally {
    $env:DSH_HOME = $previousHome
    $env:DSH_PROFILES_HOME = $previousProfilesHome
    # Verify the exact absolute subtree minted above before removing anything.
    $resolved = [IO.Path]::GetFullPath($fixture)
    if ([IO.Path]::GetDirectoryName($resolved) -ne $fixtureParent -or [IO.Path]::GetFileName($resolved) -notmatch '^desktop-cli-[a-f0-9]{32}$') {
        throw 'Refusing cleanup outside the owned Desktop CLI test subtree.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
