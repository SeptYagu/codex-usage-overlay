$ErrorActionPreference = 'Stop'
$runtimeDir = Join-Path $env:LOCALAPPDATA 'CodexUsageOverlay'
$logPath = Join-Path $runtimeDir 'startup-error.log'
$settingsPath = Join-Path $runtimeDir 'settings.json'
$overlay = Join-Path $PSScriptRoot 'CodexPetUsageOverlay.ps1'
$language = 'en'

try {
    . (Join-Path $PSScriptRoot 'Localization.ps1')
    $language = Get-OverlayLanguageFromSettings -SettingsPath $settingsPath
    New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null
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

    try {
        Add-Type -AssemblyName PresentationFramework
        if (Get-Command Get-OverlayText -ErrorAction SilentlyContinue) {
            $message = Get-OverlayText -Language $language -Key 'StartupFailed' -FormatValues @($logPath)
            $title = Get-OverlayText -Language $language -Key 'StartupErrorTitle'
        }
        else {
            $message = "悬浮窗启动失败 / The overlay could not start.`r`n详细信息：$logPath / Details: $logPath"
            $title = 'Codex 用量悬浮窗 / Codex Usage Overlay'
        }
        [Windows.MessageBox]::Show($message, $title) | Out-Null
    }
    catch { }
    exit 1
}
