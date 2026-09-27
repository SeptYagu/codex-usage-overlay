$ErrorActionPreference = 'Stop'
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

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = Get-PowerShellExecutable
$startInfo.Arguments = '-NoProfile -STA -WindowStyle Hidden -File "' + $overlay + '"'
$startInfo.UseShellExecute = $true
$startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
$null = [System.Diagnostics.Process]::Start($startInfo)
