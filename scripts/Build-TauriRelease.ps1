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
$installerName = "codex-usage-overlay_${version}_x64-setup.exe"
$msiName = "codex-usage-overlay_${version}_x64_en-US.msi"
$installer = Join-Path $repositoryRoot "src-tauri\target\release\bundle\nsis\$installerName"
$msi = Join-Path $repositoryRoot "src-tauri\target\release\bundle\msi\$msiName"
foreach ($source in @($binary, $installer, $msi)) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing release build: $source" }
}
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
Copy-Item -LiteralPath $installer -Destination (Join-Path $outputPath $installerName) -Force
Copy-Item -LiteralPath $msi -Destination (Join-Path $outputPath $msiName) -Force
$zipName = "CodexUsageOverlay-$Tag-Windows-x64.zip"
$zipPath = Join-Path $outputPath $zipName
$archiveSources = [ordered]@{
    'CodexUsageOverlay.exe' = $binary
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

$checksumLines = foreach ($assetName in @($installerName, $msiName, $zipName)) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $outputPath $assetName) -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $assetName"
}
[IO.File]::WriteAllText((Join-Path $outputPath 'SHA256SUMS.txt'), (($checksumLines -join "`n") + "`n"), [Text.Encoding]::ASCII)
Write-Output "Verified Windows release assets in $outputPath"
