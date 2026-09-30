$ErrorActionPreference = 'Stop'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('codex-release-test-' + [Guid]::NewGuid().ToString('N'))
$originalMode = $env:CODEX_PACKAGING_TEST_MODE
try {
    foreach ($relative in @('scripts', 'node_modules\.bin', 'src-tauri\target\release\bundle\nsis', 'src-tauri\target\release\bundle\msi')) {
        New-Item -ItemType Directory -Path (Join-Path $fixtureRoot $relative) -Force | Out-Null
    }
    $buildScript = Join-Path $fixtureRoot 'scripts\Build-TauriRelease.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Build-TauriRelease.ps1') -Destination $buildScript
    [IO.File]::WriteAllText((Join-Path $fixtureRoot 'src-tauri\tauri.conf.json'), '{"version":"1.2.1"}')
    foreach ($relative in @('README.md', 'README.en.md', 'src-tauri\target\release\codex-usage-overlay.exe',
        'src-tauri\target\release\codex-usage-updater.exe',
        'src-tauri\target\release\bundle\nsis\codex-usage-overlay_1.2.1_x64-setup.exe',
        'src-tauri\target\release\bundle\msi\codex-usage-overlay_1.2.1_x64_en-US.msi')) {
        [IO.File]::WriteAllText((Join-Path $fixtureRoot $relative), "test fixture: $relative")
    }
    # Structurally valid Minisign fixture, not a cryptographic signature.
    $packet = [byte[]]::new(74)
    $packet[0] = 69; $packet[1] = 68
    $signatureText = "untrusted comment: packaging test`n$([Convert]::ToBase64String($packet))`ntrusted comment: fixture`n$([Convert]::ToBase64String([byte[]]::new(64)))`n"
    $encodedSignature = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($signatureText))
    $cliDirectory = Join-Path $fixtureRoot 'node_modules\.bin'
    [IO.File]::WriteAllText((Join-Path $cliDirectory 'signature.fixture.txt'), $encodedSignature)
    foreach ($relative in @('src-tauri\target\release\bundle\nsis\codex-usage-overlay_1.2.1_x64-setup.exe.sig',
        'src-tauri\target\release\bundle\msi\codex-usage-overlay_1.2.1_x64_en-US.msi.sig')) {
        [IO.File]::WriteAllText((Join-Path $fixtureRoot $relative), $encodedSignature)
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
