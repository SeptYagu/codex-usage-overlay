[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$archiveName = 'CodexUsageOverlay-Portable.zip'
$archivePath = Join-Path $OutputDirectory $archiveName
$checksumPath = Join-Path $OutputDirectory 'SHA256SUMS.txt'
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

$scripts = @(
    Get-ChildItem -LiteralPath $repositoryRoot -Filter '*.ps1' -File
    Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1' -File
)
foreach ($scriptFile in $scripts) {
    $tokens = $null
    $errors = $null
    [Management.Automation.Language.Parser]::ParseFile(
        $scriptFile.FullName, [ref]$tokens, [ref]$errors) | Out-Null
    if ($errors.Count -gt 0) {
        throw "PowerShell syntax error in $($scriptFile.Name): $($errors -join '; ')"
    }
}

. (Join-Path $repositoryRoot 'Localization.ps1')
$missingTranslations = @(Compare-Object @($script:OverlayText.zh.Keys) @($script:OverlayText.en.Keys))
if ($missingTranslations.Count -gt 0) {
    throw "Chinese and English localization keys differ: $($missingTranslations -join ', ')"
}

if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) { throw 'Portable archive is missing.' }
if (-not (Test-Path -LiteralPath $checksumPath -PathType Leaf)) { throw 'Checksum file is missing.' }

$archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
try {
    $entries = @($archive.Entries | ForEach-Object { $_.FullName })
    if (@(Compare-Object $files $entries).Count -gt 0) {
        throw 'Portable archive file list does not match the release file list.'
    }
    foreach ($file in $files) {
        $entry = $archive.GetEntry($file)
        $entryStream = $entry.Open()
        try {
            $buffer = [IO.MemoryStream]::new()
            try {
                $entryStream.CopyTo($buffer)
                $source = [IO.File]::ReadAllBytes((Join-Path $repositoryRoot $file))
                if (-not [Linq.Enumerable]::SequenceEqual([byte[]]$source, [byte[]]$buffer.ToArray())) {
                    throw "Archive content differs from source: $file"
                }
            }
            finally { $buffer.Dispose() }
        }
        finally { $entryStream.Dispose() }
    }
}
finally { $archive.Dispose() }

$expectedHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
$checksum = [IO.File]::ReadAllText($checksumPath, [Text.Encoding]::ASCII).Trim()
if ($checksum -cne "$expectedHash  $archiveName") { throw 'SHA256 checksum does not match the archive.' }

Write-Output "Verified $($scripts.Count) PowerShell scripts, $($files.Count) archive files, translations, and SHA256."
