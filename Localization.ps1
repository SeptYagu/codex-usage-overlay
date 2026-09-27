$script:OverlayText = @{
    zh = @{
        AppTitle = 'Codex 用量悬浮窗'
        WindowShow = '显示悬浮窗'
        WindowHide = '隐藏悬浮窗'
        RefreshUsage = '立即刷新用量'
        ExitOverlay = '退出悬浮窗'
        OverlaySettings = '浮窗设置…'
        AutoStart = '登录时自动启动'
        TrayLoading = '正在读取 Codex 用量…'
        TrayUsageError = 'Codex 用量读取失败'
        TipDrag = '按住鼠标左键拖动；右键打开菜单。'
        TipLoading = '正在读取 Codex 用量。按住鼠标左键可移动，右键打开菜单。'
        TipUpdatedAt = '最近更新：{0}。按住鼠标左键可移动，右键打开菜单。'
        TipUsageFailed = '读取 Codex 用量失败，将在下次刷新时重试。按住鼠标左键可移动，右键打开菜单。'
        TipUsageStartFailed = '无法启动用量读取程序，将在下次刷新时重试。按住鼠标左键可移动，右键打开菜单。'
        TipUsageProcessFailed = '用量读取程序运行失败，将在下次刷新时重试。按住鼠标左键可移动，右键打开菜单。'
        SettingsTitle = '浮窗设置'
        OverlaySize = '浮窗大小'
        BackgroundTransparency = '背景透明度'
        ShowCreditBalance = '显示 Credit 余额'
        SettingsHelp = '调整时即时预览，关闭窗口后保存。透明度只影响背景，文字保持清晰。'
        Feedback = '反馈和建议请联系 septwind@agent.qq.com'
        SaveSettingsFailed = '保存浮窗设置失败。请检查设置文件是否可写；详细信息已记录到 overlay-error.log。'
        SaveLanguageFailed = '语言已在本次运行中切换，但保存失败，重启后可能恢复默认语言。详细信息已记录到 overlay-error.log。'
        AutoStartFailed = '设置登录时自动启动失败。详细信息已记录到 overlay-error.log。'
        StartupAutoStartFailed = '无法设置登录时自动启动。详细信息已记录到 overlay-error.log。'
        StartupFailed = "悬浮窗启动失败。`r`n详细信息已保存到：{0}"
        StartupErrorTitle = 'Codex 用量悬浮窗'
    }
    en = @{
        AppTitle = 'Codex Usage Overlay'
        WindowShow = 'Show Overlay'
        WindowHide = 'Hide Overlay'
        RefreshUsage = 'Refresh Usage Now'
        ExitOverlay = 'Exit Overlay'
        OverlaySettings = 'Overlay Settings…'
        AutoStart = 'Launch at sign-in'
        TrayLoading = 'Reading Codex usage…'
        TrayUsageError = 'Could not read Codex usage'
        TipDrag = 'Hold the left mouse button to drag; right-click for the menu.'
        TipLoading = 'Reading Codex usage. Hold the left mouse button to move; right-click for the menu.'
        TipUpdatedAt = 'Last updated: {0}. Hold the left mouse button to move; right-click for the menu.'
        TipUsageFailed = 'Could not read Codex usage. It will retry at the next refresh. Hold the left mouse button to move; right-click for the menu.'
        TipUsageStartFailed = 'Could not start the usage reader. It will retry at the next refresh. Hold the left mouse button to move; right-click for the menu.'
        TipUsageProcessFailed = 'The usage reader failed. It will retry at the next refresh. Hold the left mouse button to move; right-click for the menu.'
        SettingsTitle = 'Overlay Settings'
        OverlaySize = 'Overlay size'
        BackgroundTransparency = 'Background opacity'
        ShowCreditBalance = 'Show Credit balance'
        SettingsHelp = 'Changes preview immediately and are saved when this window closes. Opacity affects only the background; text stays clear.'
        Feedback = 'For feedback and suggestions, contact septwind@agent.qq.com'
        SaveSettingsFailed = 'Could not save overlay settings. Check that the settings file is writable. Details are in overlay-error.log.'
        SaveLanguageFailed = 'The language changed for this session, but could not be saved. It may return to the default after restart. Details are in overlay-error.log.'
        AutoStartFailed = 'Could not change the sign-in startup setting. Details are in overlay-error.log.'
        StartupAutoStartFailed = 'Could not configure launch at sign-in. Details are in overlay-error.log.'
        StartupFailed = "The overlay could not start.`r`nDetails were saved to: {0}"
        StartupErrorTitle = 'Codex Usage Overlay'
    }
}

function Get-DefaultOverlayLanguage {
    try {
        if ([Globalization.CultureInfo]::CurrentUICulture.Name -match '^zh(?:-|$)') {
            return 'zh'
        }
    }
    catch { }
    return 'en'
}

function Resolve-OverlayLanguage {
    param([AllowNull()][string]$Language)
    if ($Language -eq 'zh' -or $Language -eq 'en') { return $Language }
    return Get-DefaultOverlayLanguage
}

function Get-OverlayLanguageFromSettings {
    param([Parameter(Mandatory)][string]$SettingsPath)
    try {
        if (Test-Path -LiteralPath $SettingsPath) {
            $settings = Get-Content -Raw -LiteralPath $SettingsPath | ConvertFrom-Json
            $property = $settings.PSObject.Properties['language']
            if ($null -ne $property -and ($property.Value -eq 'zh' -or $property.Value -eq 'en')) {
                return [string]$property.Value
            }
        }
    }
    catch { }
    return Get-DefaultOverlayLanguage
}

function Get-OverlayText {
    param(
        [Parameter(Mandatory)][string]$Language,
        [Parameter(Mandatory)][string]$Key,
        [object[]]$FormatValues = @()
    )
    $resolvedLanguage = Resolve-OverlayLanguage $Language
    $value = [string]$script:OverlayText[$resolvedLanguage][$Key]
    if ($FormatValues.Count -gt 0) {
        return [string]::Format([Globalization.CultureInfo]::InvariantCulture, $value, $FormatValues)
    }
    return $value
}

function Get-OverlayDisplayCulture {
    param([Parameter(Mandatory)][string]$Language)
    $name = if ((Resolve-OverlayLanguage $Language) -eq 'zh') { 'zh-CN' } else { 'en-US' }
    return [Globalization.CultureInfo]::GetCultureInfo($name)
}
