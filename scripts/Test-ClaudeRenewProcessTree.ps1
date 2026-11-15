param(
    [Parameter(Mandatory = $true)]
    [string] $BridgePath
)

$ErrorActionPreference = 'Stop'
$bridge = (Resolve-Path -LiteralPath $BridgePath).Path
$root = Join-Path ([System.IO.Path]::GetTempPath()) ("claude-renew-tree-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$oldPidFile = $env:CUO_CHILD_PID_FILE
try {
    $env:CUO_CHILD_PID_FILE = Join-Path $root 'child.pid'
    $childScript = Join-Path $root 'child.ps1'
    $shim = Join-Path $root 'claude.cmd'
    # The shim returns while its PowerShell descendant is still sleeping.
    # The bridge's job must terminate that descendant on exit.
    Set-Content -LiteralPath $childScript -Encoding ascii -Value @'
$PID | Set-Content -LiteralPath $env:CUO_CHILD_PID_FILE
Start-Sleep -Seconds 60
'@
    Set-Content -LiteralPath $shim -Encoding ascii -Value @'
@echo off
start "" /B powershell.exe -NoProfile -NonInteractive -File "%~dp0child.ps1"
powershell.exe -NoProfile -NonInteractive -Command "Start-Sleep -Seconds 3"
'@

    & $bridge renew plugin-list $shim
    if ($LASTEXITCODE -ne 0) { throw "Renewal worker failed: $LASTEXITCODE" }
    if (-not (Test-Path -LiteralPath $env:CUO_CHILD_PID_FILE)) {
        throw 'The fixture descendant did not start.'
    }
    $childProcessId = [int](Get-Content -LiteralPath $env:CUO_CHILD_PID_FILE -Raw)
    Start-Sleep -Milliseconds 250
    if (Get-Process -Id $childProcessId -ErrorAction SilentlyContinue) {
        throw "Renewal worker left descendant $childProcessId running."
    }
    Write-Output 'PASS: Claude renewal reaps a batch shim descendant'
} finally {
    $env:CUO_CHILD_PID_FILE = $oldPidFile
    Remove-Item -LiteralPath $root -Recurse -Force
}
