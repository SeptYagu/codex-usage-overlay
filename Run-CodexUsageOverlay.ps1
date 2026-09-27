$ErrorActionPreference = 'Stop'
$runtimeDir = Join-Path $env:LOCALAPPDATA 'CodexUsageOverlay'
$logPath = Join-Path $runtimeDir 'startup-error.log'
$overlay = Join-Path $PSScriptRoot 'CodexPetUsageOverlay.ps1'
New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null

try {
    & $overlay
}
catch {
    $details = $_ | Out-String
    try {
        [IO.File]::AppendAllText(
            $logPath,
            "[$([DateTime]::Now.ToString('s'))]`r`n$details`r`n",
            [Text.Encoding]::UTF8)
    }
    catch { }

    $message = "悬浮窗启动失败：$($_.Exception.Message)`r`n详细信息已保存到：$logPath"
    try {
        Add-Type -AssemblyName PresentationFramework
        [Windows.MessageBox]::Show($message, 'Codex 用量悬浮窗') | Out-Null
    }
    catch { }
    exit 1
}
