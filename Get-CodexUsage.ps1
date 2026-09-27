[CmdletBinding()]
param(
    [int]$TimeoutSeconds = 20
)

$ErrorActionPreference = 'Stop'

function Stop-AppServerProcess {
    param([System.Diagnostics.Process]$Process)

    if ($null -eq $Process -or $Process.HasExited) {
        return
    }

    try {
        $Process.Kill($true)
    }
    catch {
        try { $Process.Kill() } catch { }
    }
}

$codexExecutable = $null

if (-not [string]::IsNullOrWhiteSpace($env:CODEX_CLI_PATH)) {
    if (-not (Test-Path -LiteralPath $env:CODEX_CLI_PATH -PathType Leaf)) {
        throw "CODEX_CLI_PATH 指向的文件不存在：$env:CODEX_CLI_PATH"
    }
    $codexExecutable = [IO.Path]::GetFullPath($env:CODEX_CLI_PATH)
}

if (-not $codexExecutable) {
    $codexCommand = Get-Command codex.exe, codex -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandType -eq 'Application' -and $_.Source -and [IO.Path]::GetExtension($_.Source) -ieq '.exe' } |
        Select-Object -First 1
    if ($codexCommand) { $codexExecutable = $codexCommand.Source }
}

if (-not $codexExecutable) {
    $localCodexBin = Join-Path $env:LOCALAPPDATA 'OpenAI\Codex\bin'
    if (Test-Path -LiteralPath $localCodexBin) {
        $candidate = Get-ChildItem -LiteralPath $localCodexBin -Directory -ErrorAction SilentlyContinue |
            ForEach-Object { Join-Path $_.FullName 'codex.exe' } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
            Sort-Object { (Get-Item -LiteralPath $_).LastWriteTimeUtc } -Descending |
            Select-Object -First 1
        if ($candidate) { $codexExecutable = $candidate }
    }
}

if (-not $codexExecutable) {
    $windowsPowerShell = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (Test-Path -LiteralPath $windowsPowerShell) {
        $query = '$p=Get-AppxPackage -Name OpenAI.Codex | Select-Object -First 1; if($p){Join-Path $p.InstallLocation "app\resources\codex.exe"}'
        $encodedQuery = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($query))
        $lookup = [Diagnostics.ProcessStartInfo]::new()
        $lookup.FileName = $windowsPowerShell
        $lookup.Arguments = "-NoProfile -NonInteractive -EncodedCommand $encodedQuery"
        $lookup.UseShellExecute = $false
        $lookup.CreateNoWindow = $true
        $lookup.RedirectStandardOutput = $true
        $lookup.RedirectStandardError = $true
        $lookupProcess = [Diagnostics.Process]::new()
        $lookupProcess.StartInfo = $lookup
        try {
            $null = $lookupProcess.Start()
            $lookupOutput = $lookupProcess.StandardOutput.ReadToEndAsync()
            $lookupError = $lookupProcess.StandardError.ReadToEndAsync()
            if ($lookupProcess.WaitForExit(10000)) {
                $null = $lookupError.Result
                $candidate = @($lookupOutput.Result -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ }) | Select-Object -Last 1
                if ($lookupProcess.ExitCode -eq 0 -and
                    -not [string]::IsNullOrWhiteSpace([string]$candidate) -and
                    (Test-Path -LiteralPath $candidate -PathType Leaf)) {
                    $codexExecutable = [IO.Path]::GetFullPath($candidate)
                }
            }
            else {
                try { $lookupProcess.Kill($true) } catch { try { $lookupProcess.Kill() } catch { } }
            }
        }
        finally {
            $lookupProcess.Dispose()
        }
    }
}

if (-not $codexExecutable) {
    throw '找不到 Codex app-server。请确认 Codex 已安装，或设置 CODEX_CLI_PATH 指向 codex.exe。'
}

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $codexExecutable
$startInfo.Arguments = 'app-server --stdio'
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardInput = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true

$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $startInfo
$null = $process.Start()
$stderrTask = $process.StandardError.ReadToEndAsync()

try {
    $process.StandardInput.WriteLine('{"id":1,"method":"initialize","params":{"clientInfo":{"name":"codex-usage-overlay","title":"Codex Usage Overlay","version":"0.2.0"},"capabilities":{"experimentalApi":true}}}')
    $process.StandardInput.WriteLine('{"method":"initialized","params":{}}')
    $process.StandardInput.WriteLine('{"id":2,"method":"account/rateLimits/read","params":{"excludeResetCreditDetails":true,"supportsLunaReserveFallback":false}}')
    $process.StandardInput.Flush()

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $usageResult = $null

    while ([DateTime]::UtcNow -lt $deadline) {
        $remainingMs = [Math]::Max(1, [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds)
        $lineTask = $process.StandardOutput.ReadLineAsync()
        if (-not $lineTask.Wait($remainingMs)) {
            throw "Timed out waiting for Codex usage data."
        }

        $line = $lineTask.Result
        if ($null -eq $line) {
            throw "Codex app-server closed before returning usage data."
        }

        try {
            $message = $line | ConvertFrom-Json
        }
        catch {
            continue
        }

        if ($message.id -eq 2) {
            if ($null -ne $message.error) {
                throw "Codex usage request failed: $($message.error | ConvertTo-Json -Compress)"
            }
            $usageResult = $message.result
            break
        }
    }

    if ($null -eq $usageResult) {
        throw "Codex did not return usage data before the timeout."
    }

    $bucket = $null
    if ($null -ne $usageResult.rateLimitsByLimitId) {
        $codexBucket = $usageResult.rateLimitsByLimitId.PSObject.Properties['codex']
        if ($null -ne $codexBucket) {
            $bucket = $codexBucket.Value
        }
    }
    if ($null -eq $bucket) {
        $bucket = $usageResult.rateLimits
    }

    $fiveHourRemaining = $null
    $weekRemaining = $null
    if ($null -ne $bucket.primary) {
        $fiveHourRemaining = [Math]::Max(0, [Math]::Min(100, 100 - [int]$bucket.primary.usedPercent))
    }
    if ($null -ne $bucket.secondary) {
        $weekRemaining = [Math]::Max(0, [Math]::Min(100, 100 - [int]$bucket.secondary.usedPercent))
    }

    $creditsDisplay = '—'
    $creditsBalance = $null
    if ($null -ne $bucket.credits) {
        if ($bucket.credits.unlimited -eq $true) {
            $creditsDisplay = '∞'
        }
        elseif ($null -ne $bucket.credits.balance -and [string]$bucket.credits.balance -ne '') {
            $creditsBalance = [string]$bucket.credits.balance
            $creditsDisplay = $creditsBalance
        }
    }

    [ordered]@{
        fiveHourRemainingPercent = $fiveHourRemaining
        weekRemainingPercent     = $weekRemaining
        creditsDisplay           = $creditsDisplay
        creditsBalance           = $creditsBalance
        hasCredits               = if ($null -ne $bucket.credits) { [bool]$bucket.credits.hasCredits } else { $false }
        fiveHourResetsAt         = if ($null -ne $bucket.primary) { $bucket.primary.resetsAt } else { $null }
        weekResetsAt             = if ($null -ne $bucket.secondary) { $bucket.secondary.resetsAt } else { $null }
        fetchedAt                = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    } | ConvertTo-Json -Compress
}
finally {
    Stop-AppServerProcess -Process $process
    $process.Dispose()
}
