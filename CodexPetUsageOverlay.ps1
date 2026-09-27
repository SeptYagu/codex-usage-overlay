[CmdletBinding()]
param(
    [int]$RefreshSeconds = 60
)

$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'Localization.ps1')

Add-Type -AssemblyName PresentationFramework
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$createdNew = $false
$mutex = [Threading.Mutex]::new($true, 'Local\CodexUsageOverlay', [ref]$createdNew)
if (-not $createdNew) {
    exit 0
}

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$usageScript = Join-Path $scriptRoot 'Get-CodexUsage.ps1'
$runtimeDir = Join-Path $env:LOCALAPPDATA 'CodexUsageOverlay'
$pidPath = Join-Path $runtimeDir 'overlay.pid'
$usageStatusPath = Join-Path $runtimeDir 'usage-status.json'
$settingsPath = Join-Path $runtimeDir 'settings.json'
$positionPath = Join-Path $runtimeDir 'window-position.json'
$script:diagnosticPath = Join-Path $runtimeDir 'overlay-error.log'
New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null
$script:overlayScalePercent = 175
$script:backgroundTransparencyPercent = 23
$script:showCredits = $true
$script:settingsValues = [ordered]@{}
$script:language = Get-DefaultOverlayLanguage
$script:languageOverride = $false
if (Test-Path -LiteralPath $settingsPath) {
    try {
        $savedSettings = Get-Content -Raw -LiteralPath $settingsPath | ConvertFrom-Json
        foreach ($property in $savedSettings.PSObject.Properties) {
            $script:settingsValues[$property.Name] = $property.Value
        }
        $savedScale = $savedSettings.PSObject.Properties['scalePercent']
        if ($null -ne $savedScale -and [int]::TryParse([string]$savedScale.Value, [ref]$script:overlayScalePercent)) {
            $script:overlayScalePercent = [Math]::Max(100, [Math]::Min(250, $script:overlayScalePercent))
        }
        else {
            $script:overlayScalePercent = 175
        }
        $savedTransparency = $savedSettings.PSObject.Properties['backgroundTransparencyPercent']
        $parsedTransparency = 23
        if ($null -ne $savedTransparency -and
            [int]::TryParse([string]$savedTransparency.Value, [ref]$parsedTransparency)) {
            $script:backgroundTransparencyPercent = [Math]::Max(0, [Math]::Min(80, $parsedTransparency))
        }
        $savedCredits = $savedSettings.PSObject.Properties['showCredits']
        $parsedCredits = $true
        if ($null -ne $savedCredits -and
            [bool]::TryParse([string]$savedCredits.Value, [ref]$parsedCredits)) {
            $script:showCredits = $parsedCredits
        }
        $savedLanguage = $savedSettings.PSObject.Properties['language']
        if ($null -ne $savedLanguage -and ($savedLanguage.Value -eq 'zh' -or $savedLanguage.Value -eq 'en')) {
            $script:language = [string]$savedLanguage.Value
            $script:languageOverride = $true
        }
    }
    catch {
        $script:overlayScalePercent = 175
        $script:backgroundTransparencyPercent = 23
        $script:showCredits = $true
        $script:settingsValues = [ordered]@{}
        $script:language = Get-DefaultOverlayLanguage
        $script:languageOverride = $false
    }
}

[xml]$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        Title="Codex 用量悬浮窗" WindowStyle="None" AllowsTransparency="True"
        Background="Transparent" Topmost="True" Focusable="True"
        ShowInTaskbar="False" ResizeMode="NoResize" SizeToContent="WidthAndHeight"
        WindowStartupLocation="Manual">
  <Window.ContextMenu>
    <ContextMenu>
      <MenuItem Header="立即刷新用量"/>
      <Separator/>
      <MenuItem Header="隐藏悬浮窗"/>
      <Separator/>
      <MenuItem Header="退出悬浮窗"/>
    </ContextMenu>
  </Window.ContextMenu>
  <Border Name="Root" Background="#C41B1D24" CornerRadius="12" Padding="8,3"
          BorderThickness="1" BorderBrush="#664C5362"
          ToolTip="">
    <Border.LayoutTransform>
      <ScaleTransform ScaleX="1.75" ScaleY="1.75"/>
    </Border.LayoutTransform>
    <StackPanel>
      <Grid Name="UsageRow" HorizontalAlignment="Left">
        <Grid.ColumnDefinitions>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
        </Grid.ColumnDefinitions>
        <TextBlock Name="FiveHourLabel" Grid.Column="0" Text="5H" Foreground="White" FontSize="14"
                   FontFamily="Consolas" FontWeight="SemiBold" VerticalAlignment="Center"/>
        <TextBlock Name="FiveHourValue" Grid.Column="1" Text="--%" Foreground="White" FontSize="16" MinWidth="40" Margin="5,0,0,0"
                   FontFamily="Consolas" FontWeight="SemiBold" TextAlignment="Left" VerticalAlignment="Center"/>
        <TextBlock Grid.Column="2" Text=" " Foreground="White" FontSize="16"
                   FontFamily="Consolas" VerticalAlignment="Center"/>
        <TextBlock Name="WeekLabel" Grid.Column="3" Text="WK" Foreground="White" FontSize="14"
                   FontFamily="Consolas" FontWeight="SemiBold" VerticalAlignment="Center"/>
        <TextBlock Name="WeekValue" Grid.Column="4" Text="--%" Foreground="White" FontSize="16" MinWidth="40" Margin="5,0,0,0"
                   FontFamily="Consolas" FontWeight="SemiBold" TextAlignment="Left" VerticalAlignment="Center"/>
        <TextBlock Name="CreditsSeparator" Grid.Column="5" Text=" " Foreground="White" FontSize="16"
                   FontFamily="Consolas" VerticalAlignment="Center"/>
        <TextBlock Name="CreditsLabel" Grid.Column="6" Text="CR" Foreground="White" FontSize="14"
                   FontFamily="Consolas" FontWeight="SemiBold" VerticalAlignment="Center"/>
        <TextBlock Name="CreditsValue" Grid.Column="7" Text="—" Foreground="White" FontSize="16" Margin="5,0,0,0"
                   FontFamily="Consolas" FontWeight="SemiBold" VerticalAlignment="Center"/>
      </Grid>
      <Grid Name="ResetRow" HorizontalAlignment="Left" Margin="0,2,0,0">
        <Grid.ColumnDefinitions>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
          <ColumnDefinition Width="Auto"/>
        </Grid.ColumnDefinitions>
        <TextBlock Name="FiveHourReset" Grid.Column="0" Text="--H --min" Foreground="#FFC4CCD8"
                   FontSize="12" FontWeight="Bold" FontFamily="Consolas" VerticalAlignment="Center"/>
        <TextBlock Grid.Column="1" Text="  " Foreground="#FFC4CCD8" FontSize="12"
                   FontWeight="Bold" FontFamily="Consolas" VerticalAlignment="Center"/>
        <TextBlock Name="WeekReset" Grid.Column="2" Text="--H --min" Foreground="#FFC4CCD8"
                   FontSize="12" FontWeight="Bold" FontFamily="Consolas" VerticalAlignment="Center"/>
      </Grid>
    </StackPanel>
  </Border>
</Window>
'@

$reader = [System.Xml.XmlNodeReader]::new($xaml)
$window = [Windows.Markup.XamlReader]::Load($reader)
$root = $window.FindName('Root')
$script:overlayScaleTransform = [Windows.Media.ScaleTransform]::new(
    $script:overlayScalePercent / 100.0,
    $script:overlayScalePercent / 100.0)
$root.LayoutTransform = $script:overlayScaleTransform
$backgroundAlpha = [byte][Math]::Round(255 * (100 - $script:backgroundTransparencyPercent) / 100.0)
$root.Background = [Windows.Media.SolidColorBrush]::new(
    [Windows.Media.Color]::FromArgb($backgroundAlpha, 27, 29, 36))
$fiveHourText = $window.FindName('FiveHourValue')
$weekText = $window.FindName('WeekValue')
$fiveHourResetText = $window.FindName('FiveHourReset')
$weekResetText = $window.FindName('WeekReset')
$creditsLabel = $window.FindName('CreditsLabel')
$creditsSeparator = $window.FindName('CreditsSeparator')
$creditsText = $window.FindName('CreditsValue')
$creditsVisibility = if ($script:showCredits) { [Windows.Visibility]::Visible } else { [Windows.Visibility]::Collapsed }
$creditsSeparator.Visibility = $creditsVisibility
$creditsLabel.Visibility = $creditsVisibility
$creditsText.Visibility = $creditsVisibility
$refreshNowItem = $window.ContextMenu.Items[0]
$hideItem = $window.ContextMenu.Items[2]
$exitItem = $window.ContextMenu.Items[4]
$script:window = $window
$script:root = $root
$script:refreshNowItem = $refreshNowItem
$script:hideItem = $hideItem
$script:exitItem = $exitItem
$script:usageUiState = 'loading'
$script:lastUsageUpdatedAt = $null

$greenBrush = [Windows.Media.SolidColorBrush]::new([Windows.Media.ColorConverter]::ConvertFromString('#FF52D273'))
$yellowBrush = [Windows.Media.SolidColorBrush]::new([Windows.Media.ColorConverter]::ConvertFromString('#FFFFC857'))
$redBrush = [Windows.Media.SolidColorBrush]::new([Windows.Media.ColorConverter]::ConvertFromString('#FFFF5C5C'))

function Get-UsageBrush {
    param($RemainingPercent)
    if ($null -eq $RemainingPercent) { return [Windows.Media.Brushes]::White }
    if ([double]$RemainingPercent -ge 50) { return $greenBrush }
    if ([double]$RemainingPercent -ge 20) { return $yellowBrush }
    return $redBrush
}

if (-not ('CodexUsageOverlayNativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class CodexUsageOverlayNativeMethods
{
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool DestroyIcon(IntPtr hIcon);
}
'@
}

function New-DashboardIcon {
    $bitmap = [System.Drawing.Bitmap]::new(32, 32)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $backgroundBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 35, 55, 82))
    $centerBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 102, 230, 255))
    $outlinePen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(255, 244, 247, 252), 2)
    $redPen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(255, 255, 101, 111), 3)
    $yellowPen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(255, 255, 222, 98), 3)
    $greenPen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(255, 99, 250, 145), 3)
    $needlePen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(255, 236, 250, 255), 2)
    $nativeHandle = [IntPtr]::Zero

    try {
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.Clear([System.Drawing.Color]::Transparent)
        $graphics.FillEllipse($backgroundBrush, 1, 1, 30, 30)
        $graphics.DrawEllipse($outlinePen, 1, 1, 30, 30)
        $graphics.DrawArc($redPen, 6, 6, 20, 20, 135, 70)
        $graphics.DrawArc($yellowPen, 6, 6, 20, 20, 207, 70)
        $graphics.DrawArc($greenPen, 6, 6, 20, 20, 279, 126)
        $graphics.DrawLine($needlePen, 16, 16, 22, 10)
        $graphics.FillEllipse($centerBrush, 13.5, 13.5, 5, 5)

        $nativeHandle = $bitmap.GetHicon()
        $iconView = [System.Drawing.Icon]::FromHandle($nativeHandle)
        return [System.Drawing.Icon]$iconView.Clone()
    }
    finally {
        if ($nativeHandle -ne [IntPtr]::Zero) {
            [CodexUsageOverlayNativeMethods]::DestroyIcon($nativeHandle) | Out-Null
        }
        $graphics.Dispose()
        $bitmap.Dispose()
        $backgroundBrush.Dispose()
        $centerBrush.Dispose()
        $outlinePen.Dispose()
        $redPen.Dispose()
        $yellowPen.Dispose()
        $greenPen.Dispose()
        $needlePen.Dispose()
    }
}

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

$script:runKeyPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$script:preferenceKeyPath = 'HKCU:\Software\CodexUsageOverlay'
$script:autoStartValueName = 'CodexUsageOverlay'
$script:runnerPath = Join-Path $scriptRoot 'Run-CodexUsageOverlay.ps1'
$script:autoStartMenuItem = $null
$script:notifyIcon = $null
$script:trayIcon = $null
$script:windowMenuItem = $null
$script:trayMenu = $null
$script:autoStartEnabled = $false
$script:settingsMenuItem = $null
$script:refreshMenuItem = $null
$script:trayExitMenuItem = $null
$script:languageSelectorHost = $null
$script:languageChineseButton = $null
$script:languageEnglishButton = $null
$script:settingsWindow = $null
$script:activeBalloonKey = $null
$script:activeBalloonUntil = [DateTime]::MinValue

function Set-AutoStart {
    param(
        [bool]$Enable,
        [bool]$RememberChoice = $true
    )

    if ($Enable) {
        $powerShellPath = Get-PowerShellExecutable
        $runCommand = '"' + $powerShellPath + '" -NoProfile -STA -WindowStyle Hidden -File "' + $script:runnerPath + '"'
        if (-not (Test-Path -LiteralPath $script:runKeyPath)) {
            New-Item -Path $script:runKeyPath -Force | Out-Null
        }
        New-ItemProperty -Path $script:runKeyPath -Name $script:autoStartValueName `
            -PropertyType String -Value $runCommand -Force | Out-Null
    }
    else {
        Remove-ItemProperty -Path $script:runKeyPath -Name $script:autoStartValueName -ErrorAction SilentlyContinue
    }

    if ($RememberChoice) {
        if (-not (Test-Path -LiteralPath $script:preferenceKeyPath)) {
            New-Item -Path $script:preferenceKeyPath -Force | Out-Null
        }
        New-ItemProperty -Path $script:preferenceKeyPath -Name 'AutoStartDisabled' `
            -PropertyType DWord -Value ([int](-not $Enable)) -Force | Out-Null
    }

    $script:autoStartEnabled = $Enable
    if ($null -ne $script:autoStartMenuItem) {
        $script:autoStartMenuItem.Checked = $Enable
    }
}

function Initialize-AutoStart {
    $preference = Get-ItemProperty -Path $script:preferenceKeyPath -Name 'AutoStartDisabled' -ErrorAction SilentlyContinue
    $disabledProperty = if ($null -ne $preference) { $preference.PSObject.Properties['AutoStartDisabled'] } else { $null }
    $enabled = $true
    if ($null -ne $disabledProperty) {
        $enabled = -not [bool]$disabledProperty.Value
    }
    Set-AutoStart -Enable:$enabled -RememberChoice:$false
}

function Update-WindowMenuText {
    if ($null -eq $script:windowMenuItem) { return }
    $script:windowMenuItem.Text = if ($window.IsVisible) {
        Get-OverlayText -Language $script:language -Key 'WindowHide'
    }
    else {
        Get-OverlayText -Language $script:language -Key 'WindowShow'
    }
}

function Write-OverlayDiagnostic {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Detail
    )
    try {
        $entry = "[$([DateTimeOffset]::Now.ToString('o'))] [$Source]`r`n$Detail`r`n"
        [IO.File]::AppendAllText($script:diagnosticPath, $entry, [Text.Encoding]::UTF8)
    }
    catch { }
}

function Show-OverlayBalloon {
    param([Parameter(Mandatory)][string]$Key)
    if ($null -eq $script:notifyIcon) { return }
    $script:activeBalloonKey = $Key
    $script:activeBalloonUntil = [DateTime]::UtcNow.AddSeconds(5)
    $script:notifyIcon.BalloonTipTitle = Get-OverlayText -Language $script:language -Key 'AppTitle'
    $script:notifyIcon.BalloonTipText = Get-OverlayText -Language $script:language -Key $Key
    $script:notifyIcon.ShowBalloonTip(5000)
}

function Update-OverlayToolTip {
    if ($null -eq $script:root) { return }
    switch ($script:usageUiState) {
        'ok' {
            $culture = Get-OverlayDisplayCulture -Language $script:language
            $formattedTime = $script:lastUsageUpdatedAt.ToString('g', $culture)
            $message = Get-OverlayText -Language $script:language -Key 'TipUpdatedAt' -FormatValues @($formattedTime)
        }
        'read-error' { $message = Get-OverlayText -Language $script:language -Key 'TipUsageFailed' }
        'start-error' { $message = Get-OverlayText -Language $script:language -Key 'TipUsageStartFailed' }
        'process-error' { $message = Get-OverlayText -Language $script:language -Key 'TipUsageProcessFailed' }
        default { $message = Get-OverlayText -Language $script:language -Key 'TipLoading' }
    }
    $script:root.ToolTip = $message
}

function Update-TrayStatusText {
    if ($null -eq $script:notifyIcon) { return }
    switch ($script:usageUiState) {
        'ok' { Update-TrayUsageText }
        'read-error' { Set-TrayUsageText (Get-OverlayText -Language $script:language -Key 'TrayUsageError') }
        'start-error' { Set-TrayUsageText (Get-OverlayText -Language $script:language -Key 'TrayUsageError') }
        'process-error' { Set-TrayUsageText (Get-OverlayText -Language $script:language -Key 'TrayUsageError') }
        default { Set-TrayUsageText (Get-OverlayText -Language $script:language -Key 'TrayLoading') }
    }
}

function Update-LanguageSelector {
    if ($null -ne $script:languageChineseButton) {
        $script:languageChineseButton.Text = if ($script:language -eq 'zh') { '✓ 中文' } else { '中文' }
        $script:languageChineseButton.AccessibleName = '中文'
    }
    if ($null -ne $script:languageEnglishButton) {
        $script:languageEnglishButton.Text = if ($script:language -eq 'en') { '✓ English' } else { 'English' }
        $script:languageEnglishButton.AccessibleName = 'English'
    }
}

function Apply-OverlayLanguage {
    if ($null -ne $script:window) {
        $script:window.Title = Get-OverlayText -Language $script:language -Key 'AppTitle'
    }
    if ($null -ne $script:refreshNowItem) {
        $script:refreshNowItem.Header = Get-OverlayText -Language $script:language -Key 'RefreshUsage'
        $script:hideItem.Header = Get-OverlayText -Language $script:language -Key 'WindowHide'
        $script:exitItem.Header = Get-OverlayText -Language $script:language -Key 'ExitOverlay'
    }
    if ($null -ne $script:settingsWindow) {
        $script:settingsWindow.Title = Get-OverlayText -Language $script:language -Key 'SettingsTitle'
        $script:settingsOverlaySizeLabel.Text = Get-OverlayText -Language $script:language -Key 'OverlaySize'
        $script:settingsTransparencyLabel.Text = Get-OverlayText -Language $script:language -Key 'BackgroundTransparency'
        $script:settingsCreditsCheckBox.Content = Get-OverlayText -Language $script:language -Key 'ShowCreditBalance'
        $script:settingsHelpText.Text = Get-OverlayText -Language $script:language -Key 'SettingsHelp'
        $script:settingsFeedbackText.Text = Get-OverlayText -Language $script:language -Key 'Feedback'
    }
    if ($null -ne $script:settingsMenuItem) {
        $script:settingsMenuItem.Text = Get-OverlayText -Language $script:language -Key 'OverlaySettings'
        $script:autoStartMenuItem.Text = Get-OverlayText -Language $script:language -Key 'AutoStart'
        $script:refreshMenuItem.Text = Get-OverlayText -Language $script:language -Key 'RefreshUsage'
        $script:trayExitMenuItem.Text = Get-OverlayText -Language $script:language -Key 'ExitOverlay'
    }
    Update-LanguageSelector
    Update-WindowMenuText
    Update-OverlayToolTip
    Update-TrayStatusText
    if ($null -ne $script:activeBalloonKey -and [DateTime]::UtcNow -lt $script:activeBalloonUntil) {
        $script:notifyIcon.BalloonTipTitle = Get-OverlayText -Language $script:language -Key 'AppTitle'
        $script:notifyIcon.BalloonTipText = Get-OverlayText -Language $script:language -Key $script:activeBalloonKey
        $script:notifyIcon.ShowBalloonTip(5000)
    }
}

function Set-OverlayLanguage {
    param([Parameter(Mandatory)][ValidateSet('zh', 'en')][string]$Language)
    $script:language = $Language
    $script:languageOverride = $true
    Apply-OverlayLanguage
    $null = Save-OverlaySettings -FailureTextKey 'SaveLanguageFailed'
}

function Toggle-OverlayWindow {
    if ($window.IsVisible) {
        $window.Hide()
    }
    else {
        $window.Show()
        $window.Activate()
    }
    Update-WindowMenuText
}

function Set-TrayUsageText {
    param([string]$Text)
    if ($null -eq $script:notifyIcon) { return }
    if ([string]::IsNullOrWhiteSpace($Text)) { $Text = Get-OverlayText -Language $script:language -Key 'AppTitle' }
    if ($Text.Length -gt 63) { $Text = $Text.Substring(0, 63) }
    $script:notifyIcon.Text = $Text
}

function Format-UsageTrayText {
    param(
        [string]$FiveHour,
        [string]$Week,
        [string]$Credits,
        [bool]$ShowCredits
    )

    $text = "5H $FiveHour% | WK $Week%"
    if ($ShowCredits) { $text += " | CR $Credits" }
    return $text
}

function Update-TrayUsageText {
    $text = Format-UsageTrayText `
        -FiveHour $script:lastFiveUsageDisplay `
        -Week $script:lastWeekUsageDisplay `
        -Credits $script:lastCreditDisplay `
        -ShowCredits:$script:showCredits
    Set-TrayUsageText $text
}

function Keep-OverlayOnScreen {
    if ([double]::IsNaN($window.Left) -or [double]::IsNaN($window.Top)) { return }

    $virtualLeft = [Windows.SystemParameters]::VirtualScreenLeft
    $virtualTop = [Windows.SystemParameters]::VirtualScreenTop
    $virtualRight = $virtualLeft + [Windows.SystemParameters]::VirtualScreenWidth
    $virtualBottom = $virtualTop + [Windows.SystemParameters]::VirtualScreenHeight
    $window.Left = [Math]::Max($virtualLeft + 12, [Math]::Min($window.Left, $virtualRight - $window.ActualWidth - 12))
    $window.Top = [Math]::Max($virtualTop + 12, [Math]::Min($window.Top, $virtualBottom - $window.ActualHeight - 12))
}

function Set-OverlayScale {
    param([int]$Percent)

    $script:overlayScalePercent = [Math]::Max(100, [Math]::Min(250, $Percent))
    $factor = $script:overlayScalePercent / 100.0
    $script:overlayScaleTransform.ScaleX = $factor
    $script:overlayScaleTransform.ScaleY = $factor
    $window.UpdateLayout()
    Keep-OverlayOnScreen
    if ($window.IsVisible) { Save-WindowPosition }
}

function Set-OverlayTransparency {
    param([int]$Percent)

    $script:backgroundTransparencyPercent = [Math]::Max(0, [Math]::Min(80, $Percent))
    $alpha = [byte][Math]::Round(255 * (100 - $script:backgroundTransparencyPercent) / 100.0)
    $root.Background = [Windows.Media.SolidColorBrush]::new(
        [Windows.Media.Color]::FromArgb($alpha, 27, 29, 36))
}

function Set-CreditsVisibility {
    param([bool]$Show)

    $script:showCredits = $Show
    $visibility = if ($Show) { [Windows.Visibility]::Visible } else { [Windows.Visibility]::Collapsed }
    $creditsSeparator.Visibility = $visibility
    $creditsLabel.Visibility = $visibility
    $creditsText.Visibility = $visibility
    $window.UpdateLayout()
    Keep-OverlayOnScreen
    Update-TrayUsageText
}

function Save-OverlaySettings {
    param([string]$FailureTextKey = 'SaveSettingsFailed')

    $temporaryPath = "$settingsPath.tmp"
    $script:settingsValues['scalePercent'] = $script:overlayScalePercent
    $script:settingsValues['backgroundTransparencyPercent'] = $script:backgroundTransparencyPercent
    $script:settingsValues['showCredits'] = $script:showCredits
    if ($script:languageOverride) { $script:settingsValues['language'] = $script:language }
    elseif ($script:settingsValues.Contains('language')) { $script:settingsValues.Remove('language') }
    try {
        [IO.File]::WriteAllText($temporaryPath, ($script:settingsValues | ConvertTo-Json -Compress -Depth 100), [Text.Encoding]::UTF8)
        Move-Item -LiteralPath $temporaryPath -Destination $settingsPath -Force
        return $true
    }
    catch {
        Write-OverlayDiagnostic -Source 'settings-save' -Detail ($_ | Out-String)
        Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
        Show-OverlayBalloon -Key $FailureTextKey
        return $false
    }
}

function Show-OverlaySettings {
    [xml]$settingsXaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Title="浮窗设置" Width="340" SizeToContent="Height"
        ResizeMode="NoResize" WindowStartupLocation="CenterOwner"
        ShowInTaskbar="False" Topmost="True" Background="#FFF5F6F8">
  <StackPanel Margin="18,14,18,16">
    <Grid Margin="0,0,0,2">
      <TextBlock Name="OverlaySizeLabel" Text="浮窗大小" FontSize="14" FontWeight="SemiBold"/>
      <TextBlock Name="ScaleValue" HorizontalAlignment="Right" FontSize="14" FontWeight="SemiBold"/>
    </Grid>
    <Slider Name="ScaleSlider" Minimum="100" Maximum="250" TickFrequency="5"
            IsSnapToTickEnabled="True" TickPlacement="BottomRight" Margin="0,0,0,14"/>
    <Grid Margin="0,0,0,2">
      <TextBlock Name="BackgroundTransparencyLabel" Text="背景透明度" FontSize="14" FontWeight="SemiBold"/>
      <TextBlock Name="TransparencyValue" HorizontalAlignment="Right" FontSize="14" FontWeight="SemiBold"/>
    </Grid>
    <Slider Name="TransparencySlider" Minimum="0" Maximum="80" TickFrequency="5"
            IsSnapToTickEnabled="True" TickPlacement="BottomRight" Margin="0,0,0,4"/>
    <CheckBox Name="ShowCreditsCheckBox" Content="显示 Credit 余额" FontSize="13" Margin="0,12,0,0"/>
    <TextBlock Name="SettingsHelpText" Text="调整时即时预览，关闭窗口后保存。透明度只影响背景，文字保持清晰。"
               Foreground="#FF626A78" FontSize="11" TextWrapping="Wrap" Margin="0,6,0,0"/>
    <TextBlock Name="FeedbackText" Text="反馈和建议请联系 septwind@agent.qq.com"
               Foreground="#FF626A78" FontSize="11" TextWrapping="Wrap" Margin="0,10,0,0"/>
  </StackPanel>
</Window>
'@

    $reader = [System.Xml.XmlNodeReader]::new($settingsXaml)
    $settingsWindow = [Windows.Markup.XamlReader]::Load($reader)
    $settingsWindow.Owner = $window
    $script:settingsScaleSlider = $settingsWindow.FindName('ScaleSlider')
    $script:settingsScaleValue = $settingsWindow.FindName('ScaleValue')
    $script:settingsTransparencySlider = $settingsWindow.FindName('TransparencySlider')
    $script:settingsTransparencyValue = $settingsWindow.FindName('TransparencyValue')
    $script:settingsCreditsCheckBox = $settingsWindow.FindName('ShowCreditsCheckBox')
    $script:settingsWindow = $settingsWindow
    $script:settingsOverlaySizeLabel = $settingsWindow.FindName('OverlaySizeLabel')
    $script:settingsTransparencyLabel = $settingsWindow.FindName('BackgroundTransparencyLabel')
    $script:settingsHelpText = $settingsWindow.FindName('SettingsHelpText')
    $script:settingsFeedbackText = $settingsWindow.FindName('FeedbackText')

    $script:settingsScaleSlider.Value = $script:overlayScalePercent
    $script:settingsTransparencySlider.Value = $script:backgroundTransparencyPercent
    $script:settingsCreditsCheckBox.IsChecked = $script:showCredits
    $script:settingsScaleValue.Text = "$script:overlayScalePercent%"
    $script:settingsTransparencyValue.Text = "$script:backgroundTransparencyPercent%"
    Apply-OverlayLanguage

    $script:settingsScaleSlider.add_ValueChanged({
        $percent = [int][Math]::Round($script:settingsScaleSlider.Value)
        $script:settingsScaleValue.Text = "$percent%"
        Set-OverlayScale -Percent $percent
    })
    $script:settingsTransparencySlider.add_ValueChanged({
        $percent = [int][Math]::Round($script:settingsTransparencySlider.Value)
        $script:settingsTransparencyValue.Text = "$percent%"
        Set-OverlayTransparency -Percent $percent
    })
    $script:settingsCreditsCheckBox.add_Checked({ Set-CreditsVisibility -Show:$true })
    $script:settingsCreditsCheckBox.add_Unchecked({ Set-CreditsVisibility -Show:$false })
    try {
        $null = $settingsWindow.ShowDialog()
    }
    finally {
        Save-OverlaySettings
        $script:settingsWindow = $null
    }
}

function Initialize-SystemTray {
    $script:trayMenu = [System.Windows.Forms.ContextMenuStrip]::new()
    $script:windowMenuItem = [System.Windows.Forms.ToolStripMenuItem]::new()
    $settingsMenuItem = [System.Windows.Forms.ToolStripMenuItem]::new('浮窗设置…')
    $script:settingsMenuItem = $settingsMenuItem
    $script:autoStartMenuItem = [System.Windows.Forms.ToolStripMenuItem]::new('登录时自动启动')
    $script:autoStartMenuItem.CheckOnClick = $true
    $refreshMenuItem = [System.Windows.Forms.ToolStripMenuItem]::new('立即刷新用量')
    $exitMenuItem = [System.Windows.Forms.ToolStripMenuItem]::new('退出悬浮窗')
    $script:refreshMenuItem = $refreshMenuItem
    $script:trayExitMenuItem = $exitMenuItem

    $null = $script:trayMenu.Items.Add($script:windowMenuItem)
    $null = $script:trayMenu.Items.Add($refreshMenuItem)
    $null = $script:trayMenu.Items.Add([System.Windows.Forms.ToolStripSeparator]::new())
    $null = $script:trayMenu.Items.Add($settingsMenuItem)
    $null = $script:trayMenu.Items.Add([System.Windows.Forms.ToolStripSeparator]::new())
    $null = $script:trayMenu.Items.Add($script:autoStartMenuItem)
    $null = $script:trayMenu.Items.Add([System.Windows.Forms.ToolStripSeparator]::new())

    $menuItemHeight = $script:windowMenuItem.GetPreferredSize([System.Drawing.Size]::Empty).Height
    $languageButtonHeight = $menuItemHeight - 2
    $languagePanel = [System.Windows.Forms.FlowLayoutPanel]::new()
    $languagePanel.FlowDirection = [System.Windows.Forms.FlowDirection]::LeftToRight
    $languagePanel.WrapContents = $false
    $languagePanel.Dock = [System.Windows.Forms.DockStyle]::Fill
    $languagePanel.Size = [System.Drawing.Size]::new(210, $menuItemHeight)
    $languagePanel.Padding = [System.Windows.Forms.Padding]::new(28, 1, 0, 0)
    $languagePanel.Margin = [System.Windows.Forms.Padding]::Empty
    $languagePanel.BackColor = $script:trayMenu.BackColor

    $script:languageChineseButton = [System.Windows.Forms.Button]::new()
    $script:languageChineseButton.Font = $script:trayMenu.Font
    $script:languageChineseButton.AutoSize = $false
    $script:languageChineseButton.Size = [System.Drawing.Size]::new(
        [System.Windows.Forms.TextRenderer]::MeasureText('✓ 中文', $script:trayMenu.Font).Width + 8,
        $languageButtonHeight)
    $script:languageChineseButton.FlatStyle = [System.Windows.Forms.FlatStyle]::Flat
    $script:languageChineseButton.FlatAppearance.BorderSize = 0
    $script:languageChineseButton.UseVisualStyleBackColor = $false
    $script:languageChineseButton.BackColor = $languagePanel.BackColor
    $script:languageChineseButton.Margin = [System.Windows.Forms.Padding]::Empty
    $script:languageChineseButton.Padding = [System.Windows.Forms.Padding]::Empty
    $script:languageChineseButton.add_Click({ Set-OverlayLanguage -Language 'zh' })

    $script:languageEnglishButton = [System.Windows.Forms.Button]::new()
    $script:languageEnglishButton.Font = $script:trayMenu.Font
    $script:languageEnglishButton.AutoSize = $false
    $script:languageEnglishButton.Size = [System.Drawing.Size]::new(
        [System.Windows.Forms.TextRenderer]::MeasureText('✓ English', $script:trayMenu.Font).Width + 8,
        $languageButtonHeight)
    $script:languageEnglishButton.FlatStyle = [System.Windows.Forms.FlatStyle]::Flat
    $script:languageEnglishButton.FlatAppearance.BorderSize = 0
    $script:languageEnglishButton.UseVisualStyleBackColor = $false
    $script:languageEnglishButton.BackColor = $languagePanel.BackColor
    $script:languageEnglishButton.Margin = [System.Windows.Forms.Padding]::Empty
    $script:languageEnglishButton.Padding = [System.Windows.Forms.Padding]::Empty
    $script:languageEnglishButton.add_Click({ Set-OverlayLanguage -Language 'en' })

    $null = $languagePanel.Controls.Add($script:languageChineseButton)
    $languageSeparator = [System.Windows.Forms.Label]::new()
    $languageSeparator.Font = $script:trayMenu.Font
    $languageSeparator.Text = '|'
    $languageSeparator.Size = [System.Drawing.Size]::new(12, $languageButtonHeight)
    $languageSeparator.TextAlign = [System.Drawing.ContentAlignment]::MiddleCenter
    $languageSeparator.Margin = [System.Windows.Forms.Padding]::Empty
    $null = $languagePanel.Controls.Add($languageSeparator)
    $null = $languagePanel.Controls.Add($script:languageEnglishButton)

    $script:languageSelectorHost = [System.Windows.Forms.ToolStripControlHost]::new($languagePanel)
    $script:languageSelectorHost.AutoSize = $false
    $script:languageSelectorHost.Size = [System.Drawing.Size]::new(210, $menuItemHeight)
    $script:languageSelectorHost.Margin = [System.Windows.Forms.Padding]::Empty
    $script:languageSelectorHost.Padding = [System.Windows.Forms.Padding]::Empty
    $null = $script:trayMenu.Items.Add($script:languageSelectorHost)
    $null = $script:trayMenu.Items.Add([System.Windows.Forms.ToolStripSeparator]::new())
    $null = $script:trayMenu.Items.Add($exitMenuItem)

    $script:notifyIcon = [System.Windows.Forms.NotifyIcon]::new()
    $script:trayIcon = New-DashboardIcon
    $script:notifyIcon.Icon = $script:trayIcon
    $script:notifyIcon.Text = Get-OverlayText -Language $script:language -Key 'AppTitle'
    $script:notifyIcon.ContextMenuStrip = $script:trayMenu
    $script:notifyIcon.Visible = $true

    $script:windowMenuItem.add_Click({ Toggle-OverlayWindow })
    $refreshMenuItem.add_Click({ $script:nextRefresh = [DateTime]::MinValue })
    $settingsMenuItem.add_Click({ Show-OverlaySettings })
    $script:autoStartMenuItem.add_Click({
        try {
            Set-AutoStart -Enable:$script:autoStartMenuItem.Checked
        }
        catch {
            $script:autoStartMenuItem.Checked = $script:autoStartEnabled
            Write-OverlayDiagnostic -Source 'autostart-toggle' -Detail ($_ | Out-String)
            Show-OverlayBalloon -Key 'AutoStartFailed'
        }
    })
    $exitMenuItem.add_Click({ $script:allowExit = $true; $window.Close() })
    $script:notifyIcon.add_DoubleClick({ Toggle-OverlayWindow })
    $window.add_IsVisibleChanged({ Update-WindowMenuText })
    Apply-OverlayLanguage
}

function Save-WindowPosition {
    if (-not $window.IsVisible -or [double]::IsNaN($window.Left) -or [double]::IsNaN($window.Top)) {
        return
    }

    $temporaryPath = "$positionPath.tmp"
    $position = [ordered]@{
        left = [Math]::Round($window.Left, 2)
        top  = [Math]::Round($window.Top, 2)
    }
    try {
        [IO.File]::WriteAllText($temporaryPath, ($position | ConvertTo-Json -Compress), [Text.Encoding]::UTF8)
        Move-Item -LiteralPath $temporaryPath -Destination $positionPath -Force
    }
    catch {
        Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
    }
}

function Set-InitialWindowPosition {
    $window.UpdateLayout()
    $savedPosition = $null
    if (Test-Path -LiteralPath $positionPath) {
        try { $savedPosition = Get-Content -Raw -LiteralPath $positionPath | ConvertFrom-Json } catch { }
    }

    $virtualLeft = [Windows.SystemParameters]::VirtualScreenLeft
    $virtualTop = [Windows.SystemParameters]::VirtualScreenTop
    $virtualRight = $virtualLeft + [Windows.SystemParameters]::VirtualScreenWidth
    $virtualBottom = $virtualTop + [Windows.SystemParameters]::VirtualScreenHeight
    $hasSavedPosition = $null -ne $savedPosition -and
        $null -ne $savedPosition.PSObject.Properties['left'] -and
        $null -ne $savedPosition.PSObject.Properties['top']
    $savedLeft = [double]::NaN
    $savedTop = [double]::NaN
    if ($hasSavedPosition) {
        try {
            $savedLeft = [double]$savedPosition.left
            $savedTop = [double]$savedPosition.top
        }
        catch { }
    }

    $positionIsVisible = -not [double]::IsNaN($savedLeft) -and -not [double]::IsNaN($savedTop) -and
        ($savedLeft + $window.ActualWidth -ge $virtualLeft + 80) -and
        ($savedLeft -le $virtualRight - 80) -and
        ($savedTop + $window.ActualHeight -ge $virtualTop + 40) -and
        ($savedTop -le $virtualBottom - 40)

    if ($positionIsVisible) {
        $window.Left = $savedLeft
        $window.Top = $savedTop
        return
    }

    $workArea = [Windows.SystemParameters]::WorkArea
    $window.Left = [Math]::Max($workArea.Left + 12, $workArea.Right - $window.ActualWidth - 24)
    $window.Top = $workArea.Top + 24
}

$script:usageProcess = $null
$script:nextRefresh = [DateTime]::MinValue
$script:fiveHourResetAt = $null
$script:weekResetAt = $null
$script:lastFiveUsageDisplay = '--'
$script:lastWeekUsageDisplay = '--'
$script:lastCreditDisplay = '—'

function Format-ResetCountdown {
    param([object]$ResetAt)

    if ($null -eq $ResetAt -or [string]::IsNullOrWhiteSpace([string]$ResetAt)) {
        return '--H --min'
    }

    try {
        $resetTime = [DateTimeOffset]::FromUnixTimeSeconds([long]$ResetAt)
        $remainingSeconds = [long][Math]::Max(0, [Math]::Ceiling(($resetTime - [DateTimeOffset]::UtcNow).TotalSeconds))
        $totalMinutes = [long][Math]::Ceiling($remainingSeconds / 60.0)
        $hours = [long][Math]::Floor($totalMinutes / 60.0)
        $minutes = [int]($totalMinutes % 60)
        return '{0}H {1:00}min' -f $hours, $minutes
    }
    catch {
        return '--H --min'
    }
}

function Update-ResetCountdowns {
    $fiveHourResetText.Text = Format-ResetCountdown $script:fiveHourResetAt
    $weekResetText.Text = Format-ResetCountdown $script:weekResetAt
}

function Start-UsageRefresh {
    if ($null -ne $script:usageProcess -and -not $script:usageProcess.HasExited) {
        return
    }

    $powerShellPath = Get-PowerShellExecutable
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $powerShellPath
    $startInfo.Arguments = '-NoProfile -File "' + $usageScript + '"'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $script:usageProcess = [System.Diagnostics.Process]::new()
    $script:usageProcess.StartInfo = $startInfo
    $null = $script:usageProcess.Start()
    Write-UsageStatus -State 'reading'
}

function Write-UsageStatus {
    param(
        [Parameter(Mandatory)][string]$State,
        [string]$ErrorMessage = ''
    )

    try {
        $status = [ordered]@{
            state = $State
            updatedAt = [DateTimeOffset]::Now.ToString('o')
            error = $ErrorMessage
        }
        [IO.File]::WriteAllText($usageStatusPath, ($status | ConvertTo-Json -Compress), [Text.Encoding]::UTF8)
    }
    catch { }
}

function Complete-UsageRefresh {
    if ($null -eq $script:usageProcess -or -not $script:usageProcess.HasExited) {
        return
    }

    try {
        $output = $script:usageProcess.StandardOutput.ReadToEnd().Trim()
        $errorOutput = $script:usageProcess.StandardError.ReadToEnd().Trim()
        if ($script:usageProcess.ExitCode -eq 0 -and $output -ne '') {
            $usage = $output | ConvertFrom-Json
            $five = if ($null -eq $usage.fiveHourRemainingPercent) { '--' } else { [string]$usage.fiveHourRemainingPercent }
            $week = if ($null -eq $usage.weekRemainingPercent) { '--' } else { [string]$usage.weekRemainingPercent }
            $credit = if ([string]::IsNullOrWhiteSpace([string]$usage.creditsDisplay)) { '—' } else { [string]$usage.creditsDisplay }
            $creditAmount = [decimal]0
            if ([decimal]::TryParse(
                $credit,
                [Globalization.NumberStyles]::Number,
                [Globalization.CultureInfo]::InvariantCulture,
                [ref]$creditAmount)) {
                $credit = $creditAmount.ToString('F2', [Globalization.CultureInfo]::InvariantCulture)
            }
            $fiveHourText.Text = "$five%"
            $weekText.Text = "$week%"
            $creditsText.Text = $credit
            $script:lastFiveUsageDisplay = $five
            $script:lastWeekUsageDisplay = $week
            $script:lastCreditDisplay = $credit
            $script:fiveHourResetAt = $usage.fiveHourResetsAt
            $script:weekResetAt = $usage.weekResetsAt
            $script:lastUsageUpdatedAt = [DateTimeOffset]::FromUnixTimeSeconds([long]$usage.fetchedAt).ToLocalTime()
            $script:usageUiState = 'ok'
            Update-ResetCountdowns
            $fiveHourText.Foreground = Get-UsageBrush $usage.fiveHourRemainingPercent
            $weekText.Foreground = Get-UsageBrush $usage.weekRemainingPercent
            Update-TrayUsageText
            Update-OverlayToolTip
            Write-UsageStatus -State 'ok'
        }
        elseif ($errorOutput -ne '') {
            $script:usageUiState = 'read-error'
            Update-OverlayToolTip
            Update-TrayStatusText
            Write-UsageStatus -State 'error' -ErrorMessage $errorOutput
        }
        else {
            $script:usageUiState = 'process-error'
            Update-OverlayToolTip
            Update-TrayStatusText
            Write-UsageStatus -State 'error' -ErrorMessage "退出代码 $($script:usageProcess.ExitCode)"
        }
    }
    catch {
        $script:usageUiState = 'read-error'
        Update-OverlayToolTip
        Update-TrayStatusText
        Write-UsageStatus -State 'error' -ErrorMessage $_.Exception.Message
    }
    finally {
        $script:usageProcess.Dispose()
        $script:usageProcess = $null
        $script:nextRefresh = [DateTime]::UtcNow.AddSeconds([Math]::Max(15, $RefreshSeconds))
    }
}

$root.add_MouseLeftButtonDown({
    if ($_.ButtonState -eq [Windows.Input.MouseButtonState]::Pressed) {
        try {
            $window.DragMove()
            Save-WindowPosition
        }
        catch { }
    }
})
$refreshNowItem.add_Click({ $script:nextRefresh = [DateTime]::MinValue })
$hideItem.add_Click({ $window.Hide() })
$exitItem.add_Click({ $window.Close() })

Initialize-SystemTray
try {
    Initialize-AutoStart
}
catch {
    $script:autoStartMenuItem.Checked = $false
    $script:autoStartEnabled = $false
    Write-OverlayDiagnostic -Source 'autostart-initialize' -Detail ($_ | Out-String)
    Show-OverlayBalloon -Key 'StartupAutoStartFailed'
}

$timer = [Windows.Threading.DispatcherTimer]::new()
$timer.Interval = [TimeSpan]::FromMilliseconds(250)
$timer.add_Tick({
    Complete-UsageRefresh
    Update-ResetCountdowns
    if ([DateTime]::UtcNow -ge $script:nextRefresh) {
        try {
            Start-UsageRefresh
            $script:nextRefresh = [DateTime]::UtcNow.AddSeconds([Math]::Max(15, $RefreshSeconds))
        }
        catch {
            $script:usageUiState = 'start-error'
            Update-OverlayToolTip
            Update-TrayStatusText
            Write-UsageStatus -State 'error' -ErrorMessage $_.Exception.Message
            $script:nextRefresh = [DateTime]::UtcNow.AddSeconds([Math]::Max(15, $RefreshSeconds))
        }
    }
})
$timer.Start()

$app = [Windows.Application]::new()
$app.ShutdownMode = [Windows.ShutdownMode]::OnExplicitShutdown
$window.add_Closed({
    Save-WindowPosition
    $timer.Stop()
    $script:notifyIcon.Visible = $false
    $script:notifyIcon.Dispose()
    if ($null -ne $script:trayIcon) { $script:trayIcon.Dispose() }
    $script:trayMenu.Dispose()
    $app.Shutdown()
})

try {
    Set-Content -LiteralPath $pidPath -Value $PID -NoNewline
    $window.Show()
    $window.UpdateLayout()
    Set-InitialWindowPosition
    Start-UsageRefresh
    $script:nextRefresh = [DateTime]::UtcNow.AddSeconds([Math]::Max(15, $RefreshSeconds))
    $null = $app.Run()
}
finally {
    if ($null -ne $script:usageProcess -and -not $script:usageProcess.HasExited) {
        try { $script:usageProcess.Kill($true) } catch { try { $script:usageProcess.Kill() } catch { } }
    }
    if (Test-Path -LiteralPath $pidPath) {
        Remove-Item -LiteralPath $pidPath -Force -ErrorAction SilentlyContinue
    }
    $mutex.ReleaseMutex()
    $mutex.Dispose()
}
