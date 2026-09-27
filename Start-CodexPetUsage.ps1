$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Localization.ps1')
$runtimeDir = Join-Path $env:LOCALAPPDATA 'CodexUsageOverlay'
$logPath = Join-Path $runtimeDir 'startup-error.log'
$settingsPath = Join-Path $runtimeDir 'settings.json'
$language = Get-OverlayLanguageFromSettings -SettingsPath $settingsPath
$overlay = Join-Path $PSScriptRoot 'Run-CodexUsageOverlay.ps1'

function Get-PowerShellExecutable {
    if (-not [string]::IsNullOrWhiteSpace($env:CODEX_USAGE_POWERSHELL_PATH)) {
        if (-not (Test-Path -LiteralPath $env:CODEX_USAGE_POWERSHELL_PATH -PathType Leaf)) {
            throw "CODEX_USAGE_POWERSHELL_PATH 指向的文件不存在：$env:CODEX_USAGE_POWERSHELL_PATH"
        }
        return [IO.Path]::GetFullPath($env:CODEX_USAGE_POWERSHELL_PATH)
    }

    $windowsPowerShell = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf) { return $windowsPowerShell }

    $powerShellCommand = Get-Command pwsh.exe, pwsh -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandType -eq 'Application' -and $_.Source } |
        Select-Object -First 1
    if ($powerShellCommand) { return $powerShellCommand.Source }

    throw 'PowerShell was not found. Install PowerShell or set CODEX_USAGE_POWERSHELL_PATH.'
}

try {
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = Get-PowerShellExecutable
    $startInfo.Arguments = '-NoProfile -STA -WindowStyle Hidden -File "' + $overlay + '"'
    $startInfo.UseShellExecute = $true
    $startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $null = [System.Diagnostics.Process]::Start($startInfo)
}
catch {
    $details = $_ | Out-String
    try {
        New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null
        [IO.File]::AppendAllText(
            $logPath,
            "[$([DateTime]::Now.ToString('s'))]`r`n$details`r`n",
            [Text.Encoding]::UTF8)
    }
    catch { }

    try {
        Add-Type -AssemblyName PresentationFramework
        $message = Get-OverlayText -Language $language -Key 'StartupFailed' -FormatValues @($logPath)
        $title = Get-OverlayText -Language $language -Key 'StartupErrorTitle'
        [Windows.MessageBox]::Show($message, $title) | Out-Null
    }
    catch { }
    exit 1
}
