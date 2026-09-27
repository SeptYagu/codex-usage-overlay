[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\output')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
$archiveName = 'CodexUsageOverlay-Portable.zip'
$archivePath = Join-Path $outputPath $archiveName
$checksumPath = Join-Path $outputPath 'SHA256SUMS.txt'
$temporaryPath = Join-Path $outputPath ($archiveName + '.' + [Guid]::NewGuid().ToString('N') + '.tmp')
$files = @(
    'README.md'
    'README.en.md'
    'Start-CodexUsageOverlay.cmd'
    'Start-CodexPetUsage.ps1'
    'Run-CodexUsageOverlay.ps1'
    'CodexPetUsageOverlay.ps1'
    'Get-CodexUsage.ps1'
    'Stop-CodexPetUsage.ps1'
    'Localization.ps1'
)

New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
try {
    $archive = [IO.Compression.ZipFile]::Open($temporaryPath, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in $files) {
            $sourcePath = Join-Path $repositoryRoot $file
            if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
                throw "Missing portable file: $file"
            }
            [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive, $sourcePath, $file, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
    }
    finally {
        $archive.Dispose()
    }

    Move-Item -LiteralPath $temporaryPath -Destination $archivePath -Force
    $hash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($checksumPath, "$hash  $archiveName`n", [Text.Encoding]::ASCII)
}
finally {
    Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
}

Write-Output $archivePath
Write-Output $checksumPath
