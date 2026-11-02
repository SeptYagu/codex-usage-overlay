[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$packageRoot = Join-Path $repositoryRoot 'integrations\dsh-bridge'
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
$metadata = Get-Content -LiteralPath (Join-Path $packageRoot 'package.json') -Raw | ConvertFrom-Json
if ($metadata.name -cne 'dsh-codex-usage-overlay-bridge' -or $metadata.version -notmatch '^\d+\.\d+\.\d+$') {
    throw 'Unexpected DSH companion package identity.'
}
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
$packOutput = & npm.cmd pack $packageRoot --ignore-scripts --json --pack-destination $outputPath
if ($LASTEXITCODE -ne 0) { throw 'DSH companion npm pack failed.' }
$packages = @($packOutput | ConvertFrom-Json)
$expectedName = "$($metadata.name)-$($metadata.version).tgz"
if ($packages.Count -ne 1 -or $packages[0].filename -cne $expectedName) {
    throw 'Unexpected DSH companion tarball name.'
}
$tarball = Join-Path $outputPath $expectedName
$expectedEntries = @('package/README.md', 'package/cordis.patch.yml', 'package/file-security.js', 'package/index.js', 'package/package.json')
$entries = @(& tar -tzf $tarball)
if ($LASTEXITCODE -ne 0) { throw 'DSH companion archive cannot be inspected.' }
if (@(Compare-Object ($expectedEntries | Sort-Object) ($entries | Sort-Object) -CaseSensitive).Count -ne 0) {
    throw 'DSH companion archive contains missing or unexpected files.'
}
$packedJson = & tar -xOf $tarball 'package/package.json'
if ($LASTEXITCODE -ne 0) { throw 'DSH companion package metadata cannot be read.' }
$packedMetadata = $packedJson | ConvertFrom-Json
if ($packedMetadata.name -cne $metadata.name -or $packedMetadata.version -cne $metadata.version -or
    $packedMetadata.dsh.bundle.patch -cne './cordis.patch.yml') {
    throw 'DSH companion package metadata does not match the source.'
}
$hash = (Get-FileHash -LiteralPath $tarball -Algorithm SHA256).Hash.ToLowerInvariant()
$checksumPath = Join-Path $outputPath 'SHA256SUMS.txt'
$existing = if (Test-Path -LiteralPath $checksumPath) { @(Get-Content -LiteralPath $checksumPath | Where-Object { $_ -and -not $_.EndsWith("  $expectedName") }) } else { @() }
[IO.File]::WriteAllText($checksumPath, (((@($existing) + "$hash  $expectedName") -join "`n") + "`n"), [Text.Encoding]::ASCII)
Write-Output "Verified DSH companion: $expectedName ($hash)"
