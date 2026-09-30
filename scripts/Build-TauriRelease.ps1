[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^v\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?$')][string]$Tag,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\output\tauri-release')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
$config = Get-Content -LiteralPath (Join-Path $repositoryRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$version = $config.version
if (($Tag -replace '^v', '' -replace '-.*$', '') -ne $version) { throw 'Tag and app versions must match.' }

$binary = Join-Path $repositoryRoot 'src-tauri\target\release\codex-usage-overlay.exe'
$updaterHelper = Join-Path $repositoryRoot 'src-tauri\target\release\codex-usage-updater.exe'
$installerName = "codex-usage-overlay_${version}_x64-setup.exe"
$msiName = "codex-usage-overlay_${version}_x64_en-US.msi"
$installer = Join-Path $repositoryRoot "src-tauri\target\release\bundle\nsis\$installerName"
$msi = Join-Path $repositoryRoot "src-tauri\target\release\bundle\msi\$msiName"
$installerSignature = "$installer.sig"
$msiSignature = "$msi.sig"
foreach ($source in @($binary, $updaterHelper, $installer, $msi, $installerSignature, $msiSignature)) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing release build: $source" }
}
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
Copy-Item -LiteralPath $installer -Destination (Join-Path $outputPath $installerName) -Force
Copy-Item -LiteralPath $msi -Destination (Join-Path $outputPath $msiName) -Force
$installerSignatureName = "$installerName.sig"
$msiSignatureName = "$msiName.sig"
Copy-Item -LiteralPath $installerSignature -Destination (Join-Path $outputPath $installerSignatureName) -Force
Copy-Item -LiteralPath $msiSignature -Destination (Join-Path $outputPath $msiSignatureName) -Force
$zipName = "CodexUsageOverlay-$Tag-Windows-x64.zip"
$zipPath = Join-Path $outputPath $zipName
$archiveSources = [ordered]@{
    'CodexUsageOverlay.exe' = $binary
    'CodexUsageUpdater.exe' = $updaterHelper
    'README.md' = (Join-Path $repositoryRoot 'README.md')
    'README.en.md' = (Join-Path $repositoryRoot 'README.en.md')
}
$zipStream = [IO.File]::Open($zipPath, [IO.FileMode]::Create)
$archive = [IO.Compression.ZipArchive]::new($zipStream, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($entryName in $archiveSources.Keys) {
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive, $archiveSources[$entryName], $entryName, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally { $archive.Dispose(); $zipStream.Dispose() }

$archive = [IO.Compression.ZipFile]::OpenRead($zipPath)
try {
    if ($archive.Entries.Count -ne $archiveSources.Count) { throw 'Unexpected portable archive file count.' }
    foreach ($entryName in $archiveSources.Keys) {
        $entry = $archive.GetEntry($entryName)
        if (-not $entry) { throw "Missing portable archive entry: $entryName" }
        $stream = $entry.Open()
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            $entryHash = ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
            $sourceHash = (Get-FileHash -LiteralPath $archiveSources[$entryName] -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($entryHash -ne $sourceHash) { throw "Archive verification failed: $entryName" }
        } finally { $sha.Dispose(); $stream.Dispose() }
    }
} finally { $archive.Dispose() }

function Read-TauriSignature([string]$SignaturePath) {
    if (-not (Test-Path -LiteralPath $SignaturePath -PathType Leaf)) {
        throw "Missing updater signature: $SignaturePath"
    }
    $encoded = [IO.File]::ReadAllText($SignaturePath).Trim()
    if (-not $encoded) { throw "Empty updater signature: $SignaturePath" }
    try {
        $decoded = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($encoded))
        $lines = @($decoded.Trim() -split '\r?\n')
        if ($lines.Count -ne 4 -or -not $lines[0].StartsWith('untrusted comment: ') -or
            -not $lines[2].StartsWith('trusted comment: ')) { throw 'Invalid signature format' }
        $packet = [Convert]::FromBase64String($lines[1])
        $globalSignature = [Convert]::FromBase64String($lines[3])
        if ($packet.Length -ne 74 -or $globalSignature.Length -ne 64 -or
            $packet[0] -ne 69 -or $packet[1] -notin @(68, 100)) { throw 'Invalid signature packet' }
    } catch {
        throw "Malformed updater signature: $SignaturePath ($($_.Exception.Message))"
    }
    return $encoded
}

$tauriCli = Join-Path $repositoryRoot 'node_modules\.bin\tauri.cmd'
& $tauriCli signer sign $zipPath --app-version $version | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Failed to sign the portable updater archive.' }
# CLI stdout also contains human-readable instructions; the .sig file is authoritative.
$portableSignature = Read-TauriSignature "$zipPath.sig"
$nsisSignature = Read-TauriSignature $installerSignature
$windowsInstallerSignature = Read-TauriSignature $msiSignature

$releaseBaseUrl = "https://github.com/SeptYagu/codex-usage-overlay/releases/download/$Tag"
$manifest = [ordered]@{
    version = $version
    notes = "Codex Usage Overlay $version"
    pub_date = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = [ordered]@{
        'windows-x86_64-nsis' = @{
            url = "$releaseBaseUrl/$installerName"
            signature = $nsisSignature
        }
        'windows-x86_64-msi' = @{
            url = "$releaseBaseUrl/$msiName"
            signature = $windowsInstallerSignature
        }
        'windows-x86_64-portable' = @{
            url = "$releaseBaseUrl/$zipName"
            signature = $portableSignature
        }
    }
}
$manifestPath = Join-Path $outputPath 'latest.json'
$manifestJson = $manifest | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText($manifestPath, $manifestJson, [Text.UTF8Encoding]::new($false))

$checksumLines = foreach ($assetName in @($installerName, $msiName, $zipName)) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $outputPath $assetName) -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $assetName"
}
[IO.File]::WriteAllText((Join-Path $outputPath 'SHA256SUMS.txt'), (($checksumLines -join "`n") + "`n"), [Text.Encoding]::ASCII)
Write-Output "Verified Windows release assets in $outputPath"
