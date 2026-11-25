$ErrorActionPreference = 'Stop'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('codex-release-test-' + [Guid]::NewGuid().ToString('N'))
$originalMode = $env:CODEX_PACKAGING_TEST_MODE
try {
    foreach ($relative in @('scripts', 'node_modules\.bin', 'src-tauri\target\release\bundle\nsis', 'src-tauri\target\release\bundle\msi')) {
        New-Item -ItemType Directory -Path (Join-Path $fixtureRoot $relative) -Force | Out-Null
    }
    $buildScript = Join-Path $fixtureRoot 'scripts\Build-TauriRelease.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Build-TauriRelease.ps1') -Destination $buildScript
    $mergeScript = Join-Path $fixtureRoot 'scripts\Merge-TauriRelease.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Merge-TauriRelease.ps1') -Destination $mergeScript
    [IO.File]::WriteAllText((Join-Path $fixtureRoot 'src-tauri\tauri.conf.json'), '{"version":"1.2.1"}')
    foreach ($relative in @('README.md', 'README.en.md')) {
        [IO.File]::WriteAllText((Join-Path $fixtureRoot $relative), "test fixture: $relative")
    }
    $executables = @('codex-usage-overlay.exe', 'codex-usage-updater.exe', 'claude-usage-bridge.exe', 'antigravity-usage-bridge.exe')
    # Minimal PE headers: the packaging script checks each binary's machine type.
    function Write-FixtureBinaries([int]$Machine) {
        foreach ($name in $executables) {
            $bytes = [byte[]]::new(0x80)
            $bytes[0] = 0x4D; $bytes[1] = 0x5A
            [BitConverter]::GetBytes([int]0x40).CopyTo($bytes, 0x3C)
            [BitConverter]::GetBytes([uint32]0x4550).CopyTo($bytes, 0x40)
            [BitConverter]::GetBytes([uint16]$Machine).CopyTo($bytes, 0x44)
            [Text.Encoding]::ASCII.GetBytes("fixture $name").CopyTo($bytes, 0x48)
            [IO.File]::WriteAllBytes((Join-Path $fixtureRoot "src-tauri\target\release\$name"), $bytes)
        }
    }
    Write-FixtureBinaries 0x8664
    foreach ($arch in @('x64', 'arm64')) {
        foreach ($relative in @("src-tauri\target\release\bundle\nsis\codex-usage-overlay_1.2.1_$arch-setup.exe",
            "src-tauri\target\release\bundle\msi\codex-usage-overlay_1.2.1_${arch}_en-US.msi")) {
            [IO.File]::WriteAllText((Join-Path $fixtureRoot $relative), "test fixture: $relative")
        }
    }
    # Structurally valid Minisign fixture, not a cryptographic signature.
    $packet = [byte[]]::new(74)
    $packet[0] = 69; $packet[1] = 68
    $signatureText = "untrusted comment: packaging test`n$([Convert]::ToBase64String($packet))`ntrusted comment: fixture`n$([Convert]::ToBase64String([byte[]]::new(64)))`n"
    $encodedSignature = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($signatureText))
    $cliDirectory = Join-Path $fixtureRoot 'node_modules\.bin'
    [IO.File]::WriteAllText((Join-Path $cliDirectory 'signature.fixture.txt'), $encodedSignature)
    foreach ($arch in @('x64', 'arm64')) {
        foreach ($relative in @("src-tauri\target\release\bundle\nsis\codex-usage-overlay_1.2.1_$arch-setup.exe.sig",
            "src-tauri\target\release\bundle\msi\codex-usage-overlay_1.2.1_${arch}_en-US.msi.sig")) {
            [IO.File]::WriteAllText((Join-Path $fixtureRoot $relative), $encodedSignature)
        }
    }
    [IO.File]::WriteAllText((Join-Path $cliDirectory 'tauri.cmd'), '@echo off' + "`r`n" + 'pwsh.exe -NoProfile -File "%~dp0signer-stub.ps1" "%~3"' + "`r`n" + 'exit /b %errorlevel%' + "`r`n")
    $stub = @'
param([string]$ArchivePath)
$signature = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'signature.fixture.txt') -Raw
switch ($env:CODEX_PACKAGING_TEST_MODE) {
    'missing' { }
    'empty' { [IO.File]::WriteAllText("$ArchivePath.sig", '') }
    'invalid-base64' { [IO.File]::WriteAllText("$ArchivePath.sig", 'not a signature!') }
    'invalid-packet' {
        $text = "untrusted comment: test`nAA==`ntrusted comment: test`nAA==`n"
        [IO.File]::WriteAllText("$ArchivePath.sig", [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($text)))
    }
    'signer-failure' { exit 7 }
    default { [IO.File]::WriteAllText("$ArchivePath.sig", $signature.Trim()) }
}
Write-Output $signature.Trim()
Write-Output 'Make sure to include this into the signature field of your update server.'
exit 0
'@
    [IO.File]::WriteAllText((Join-Path $cliDirectory 'signer-stub.ps1'), $stub)
    foreach ($mode in @('valid', 'missing', 'empty', 'invalid-base64', 'invalid-packet', 'signer-failure')) {
        $env:CODEX_PACKAGING_TEST_MODE = $mode
        $caseOutput = Join-Path $fixtureRoot "output-$mode"
        $failure = $null
        try { & $buildScript -Tag v1.2.1 -OutputDirectory $caseOutput | Out-Null }
        catch { $failure = $_.Exception.Message }
        $manifestPath = Join-Path $caseOutput 'latest.json'
        if ($mode -eq 'valid') {
            if ($failure) { throw "Valid packaging failed: $failure" }
            $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
            if ($manifest.platforms.'windows-x86_64-portable'.signature -cne $encodedSignature) {
                throw 'Portable manifest signature came from console output instead of the .sig file'
            }
            if (@(Get-ChildItem -LiteralPath $caseOutput -File).Count -ne 8) { throw 'Expected eight assets' }
        } else {
            if (-not $failure) { throw "Packaging accepted $mode signature" }
            if (Test-Path -LiteralPath $manifestPath) { throw "Manifest generated for $mode signature" }
            $expectedFailure = switch ($mode) {
                'missing' { 'Missing updater signature' }
                'empty' { 'Empty updater signature' }
                'signer-failure' { 'Failed to sign the portable updater archive' }
                default { 'Malformed updater signature' }
            }
            if (-not $failure.Contains($expectedFailure)) { throw "Unexpected $mode failure: $failure" }
        }
        Write-Output "PASS: packaging $mode"
    }
    # A preview's manifest must carry the preview's own version.
    $env:CODEX_PACKAGING_TEST_MODE = 'valid'
    $failure = $null
    try { & $buildScript -Tag v1.2.1-preview.1 -OutputDirectory (Join-Path $fixtureRoot 'output-tag') | Out-Null }
    catch { $failure = $_.Exception.Message }
    if (-not $failure -or -not $failure.Contains('Tag and app versions must match')) { throw 'Packaging accepted a preview tag for a stable version' }
    Write-Output 'PASS: packaging tag mismatch'

    # x64 binaries are never packaged as arm64.
    $failure = $null
    try { & $buildScript -Tag v1.2.1 -Arch arm64 -OutputDirectory (Join-Path $fixtureRoot 'output-mislabelled') | Out-Null }
    catch { $failure = $_.Exception.Message }
    if (-not $failure -or -not $failure.Contains('Executable is not arm64')) { throw "Packaging accepted x64 binaries as arm64: $failure" }
    Write-Output 'PASS: packaging architecture mismatch'

    # An arm64 build names its own assets and updater platforms.
    Write-FixtureBinaries 0xAA64
    $armOutput = Join-Path $fixtureRoot 'output-arm64'
    & $buildScript -Tag v1.2.1 -Arch arm64 -OutputDirectory $armOutput | Out-Null
    $armManifest = Get-Content -LiteralPath (Join-Path $armOutput 'latest.json') -Raw | ConvertFrom-Json
    $armPlatforms = @($armManifest.platforms.PSObject.Properties.Name | Sort-Object)
    if (($armPlatforms -join ',') -cne 'windows-aarch64-msi,windows-aarch64-nsis,windows-aarch64-portable') {
        throw "Unexpected arm64 platforms: $($armPlatforms -join ',')"
    }
    if ($armManifest.platforms.'windows-aarch64-portable'.url -notlike '*/CodexUsageOverlay-v1.2.1-Windows-arm64.zip') {
        throw 'Unexpected arm64 portable URL'
    }
    if (@(Get-ChildItem -LiteralPath $armOutput -File).Count -ne 8) { throw 'Expected eight arm64 assets' }
    Write-Output 'PASS: packaging arm64'

    # Both architectures merge into one manifest and checksum list.
    $mergedOutput = Join-Path $fixtureRoot 'output-merged'
    & $mergeScript -Tag v1.2.1 -InputDirectory @((Join-Path $fixtureRoot 'output-valid'), $armOutput) -OutputDirectory $mergedOutput | Out-Null
    $merged = Get-Content -LiteralPath (Join-Path $mergedOutput 'latest.json') -Raw | ConvertFrom-Json
    if (@($merged.platforms.PSObject.Properties).Count -ne 6 -or $merged.version -cne '1.2.1') { throw 'Merged manifest is incomplete' }
    if (@(Get-Content -LiteralPath (Join-Path $mergedOutput 'SHA256SUMS.txt')).Count -ne 6) { throw 'Merged checksums are incomplete' }
    if (@(Get-ChildItem -LiteralPath $mergedOutput -File).Count -ne 14) { throw 'Expected fourteen merged assets' }
    $failure = $null
    try { & $mergeScript -Tag v1.2.1 -InputDirectory @($armOutput, $armOutput) -OutputDirectory (Join-Path $fixtureRoot 'output-duplicate') | Out-Null }
    catch { $failure = $_.Exception.Message }
    if (-not $failure -or -not $failure.Contains('Duplicate updater platform')) { throw "Merge accepted a duplicate platform: $failure" }
    $failure = $null
    try { & $mergeScript -Tag v1.2.2 -InputDirectory @($armOutput) -OutputDirectory (Join-Path $fixtureRoot 'output-wrong-tag') | Out-Null }
    catch { $failure = $_.Exception.Message }
    if (-not $failure -or -not $failure.Contains('does not match')) { throw "Merge accepted another version: $failure" }
    Write-Output 'PASS: packaging merge'
} finally {
    $env:CODEX_PACKAGING_TEST_MODE = $originalMode
    # Delete only this newly-created fixture under the system temp root.
    $absoluteFixture = [IO.Path]::GetFullPath($fixtureRoot)
    $tempPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $absoluteFixture.StartsWith($tempPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($absoluteFixture) -notmatch '^codex-release-test-[a-f0-9]{32}$') {
        throw 'Unexpected test fixture cleanup path'
    }
    if (Test-Path -LiteralPath $absoluteFixture) { Remove-Item -LiteralPath $absoluteFixture -Recurse -Force }
}
exit 0
