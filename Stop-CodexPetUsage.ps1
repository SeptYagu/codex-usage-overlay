$pidPath = Join-Path $env:LOCALAPPDATA 'CodexUsageOverlay\overlay.pid'
if (-not (Test-Path -LiteralPath $pidPath)) {
    return
}

$overlayPid = 0
if ([int]::TryParse((Get-Content -Raw -LiteralPath $pidPath).Trim(), [ref]$overlayPid)) {
    Stop-Process -Id $overlayPid -ErrorAction SilentlyContinue
}
Remove-Item -LiteralPath $pidPath -Force -ErrorAction SilentlyContinue
