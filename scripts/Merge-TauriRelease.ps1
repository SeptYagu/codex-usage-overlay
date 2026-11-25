[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^v\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?$')][string]$Tag,
    # One Build-TauriRelease.ps1 output per architecture.
    [Parameter(Mandatory)][string[]]$InputDirectory,
    [Parameter(Mandatory)][string]$OutputDirectory
)

# Combines the per-architecture release outputs into one set of assets with a
# single latest.json and SHA256SUMS.txt, checking every file against its hash.
$ErrorActionPreference = 'Stop'
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
$generated = @('latest.json', 'SHA256SUMS.txt')
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null

$version = $null
$notes = $null
$platforms = [ordered]@{}
$checksums = [ordered]@{}
foreach ($directory in $InputDirectory) {
    $inputPath = [IO.Path]::GetFullPath($directory)
    $manifestPath = Join-Path $inputPath 'latest.json'
    $checksumPath = Join-Path $inputPath 'SHA256SUMS.txt'
    foreach ($required in @($manifestPath, $checksumPath)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing release file: $required" }
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ("v$($manifest.version)" -cne $Tag) { throw "Manifest version $($manifest.version) does not match $Tag" }
    if ($null -eq $version) { $version = $manifest.version; $notes = $manifest.notes }
    foreach ($platform in $manifest.platforms.PSObject.Properties) {
        if ($platforms.Contains($platform.Name)) { throw "Duplicate updater platform: $($platform.Name)" }
        $platforms[$platform.Name] = [ordered]@{ url = $platform.Value.url; signature = $platform.Value.signature }
    }
    foreach ($line in Get-Content -LiteralPath $checksumPath) {
        if (-not $line) { continue }
        if ($line -notmatch '^([0-9a-f]{64})  (\S+)$') { throw "Malformed checksum line: $line" }
        $hash, $name = $Matches[1], $Matches[2]
        if ($checksums.Contains($name) -and $checksums[$name] -cne $hash) { throw "Conflicting checksums for $name" }
        $checksums[$name] = $hash
    }
    foreach ($file in Get-ChildItem -LiteralPath $inputPath -File) {
        if ($file.Name -in $generated) { continue }
        $target = Join-Path $outputPath $file.Name
        if (Test-Path -LiteralPath $target) {
            $existing = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
            if ($existing -ne (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash) {
                throw "Conflicting release asset: $($file.Name)"
            }
        } else {
            Copy-Item -LiteralPath $file.FullName -Destination $target
        }
    }
}
if ($platforms.Count -eq 0) { throw 'No updater platforms to publish.' }

$releaseBaseUrl = "https://github.com/SeptYagu/codex-usage-overlay/releases/download/$Tag"
foreach ($platform in $platforms.Keys) {
    $url = $platforms[$platform].url
    if (-not $url.StartsWith("$releaseBaseUrl/")) { throw "Unexpected download URL for ${platform}: $url" }
    $name = $url.Substring($releaseBaseUrl.Length + 1)
    if (-not $checksums.Contains($name)) { throw "No checksum for $name" }
    if (-not (Test-Path -LiteralPath (Join-Path $outputPath "$name.sig") -PathType Leaf)) { throw "Missing signature: $name.sig" }
}
foreach ($name in $checksums.Keys) {
    $path = Join-Path $outputPath $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing release asset: $name" }
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $checksums[$name]) {
        throw "Checksum mismatch: $name"
    }
}

$manifest = [ordered]@{
    version = $version
    notes = $notes
    pub_date = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = $platforms
}
[IO.File]::WriteAllText((Join-Path $outputPath 'latest.json'), ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
$checksumLines = foreach ($name in $checksums.Keys) { "$($checksums[$name])  $name" }
[IO.File]::WriteAllText((Join-Path $outputPath 'SHA256SUMS.txt'), (($checksumLines -join "`n") + "`n"), [Text.Encoding]::ASCII)
Write-Output "Merged $($platforms.Count) updater platforms and $($checksums.Count) checksummed assets into $outputPath"
