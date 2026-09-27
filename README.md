# Codex Usage Overlay

[![简体中文](https://img.shields.io/badge/Language-%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-2F6FEB?style=for-the-badge)](README.md) [![English](https://img.shields.io/badge/Language-English-lightgrey?style=for-the-badge)](README.en.md)

一个独立的 Windows 用量悬浮窗。它不读取 Codex Pets 设置、宠物位置或 Codex 界面状态，也不修改 Codex 安装文件。悬浮窗通过本机 Codex `app-server` 的 `account/rateLimits/read` 获取数据，不读取或保存 `auth.json`，也不调用网页接口。

窗口固定置顶，不在任务栏显示 PowerShell 窗口，可用鼠标左键拖动；位置会保存在当前 Windows 用户的 `%LOCALAPPDATA%\CodexUsageOverlay\window-position.json`。整体界面约为原迷你横栏的 1.75 倍，默认每 60 秒更新一次。第二行显示 5H 和 WK 的重置倒计时，不再显示周期标签；每段格式为累计小时和分钟，例如 `167H 05min`，两段之间留两个空格，数字使用加粗的大号字体。第一行各组之间留一个空格；5H/WK 数值栏预留三位数字和百分号宽度，减少数值位数变化时的整体宽度波动。两行共用同一底板，整体宽度随较长一行调整，内容左对齐。

程序带明亮的仪表盘样式系统托盘图标。双击图标可显示或隐藏窗口；右键菜单可显示/隐藏、立即刷新、调整浮窗设置、设置登录时自动启动或退出。打开“浮窗设置”可拖动滑块即时调整大小（100%–250%）和背景透明度（0%–80%，默认 23%），也可勾选或取消“显示 Credit 余额”；该选择同时控制悬浮窗和托盘悬停提示中的余额显示。关闭设置窗口后会保存选择，透明度只影响背景，文字保持清晰。设置保存在当前 Windows 用户的 `%LOCALAPPDATA%\CodexUsageOverlay\settings.json`。设置窗口底部提供反馈邮箱 septwind@agent.qq.com。首次启动时会为当前 Windows 用户创建登录启动项，不需要管理员权限。取消托盘菜单中的“登录时自动启动”即可关闭，选择会保存。

## 启动

双击 `Start-CodexUsageOverlay.cmd` 即可启动。也可以在解压后的目录中打开 PowerShell，运行：

```powershell
.\Start-CodexPetUsage.ps1
```

首次启动会在屏幕右上方显示悬浮窗。启动后可以关闭 PowerShell 窗口；悬浮窗会继续运行。

## 停止

在 PowerShell 中运行：

```powershell
.\Stop-CodexPetUsage.ps1
```

也可以右键悬浮窗并选择 **退出悬浮窗**。

## 兼容性

- 适用于 Windows 10/11。
- 默认使用 Windows 自带的 Windows PowerShell 5.1，因此无需安装 PowerShell 7，也无需管理员权限。可通过 `CODEX_USAGE_POWERSHELL_PATH` 指定其他 PowerShell。
- 需要安装并登录 Codex 桌面应用。程序会自动查找 PATH 中的 Codex、用户目录下的 Codex CLI，或 Microsoft Store 版本的 Codex。
- 如果 Codex 安装在自定义位置，可在启动前设置 `CODEX_CLI_PATH`，指向该安装中的 `codex.exe`。
- 如果要指定 PowerShell，可设置 `CODEX_USAGE_POWERSHELL_PATH`，指向 `pwsh.exe` 或 `powershell.exe`。
- 如果使用自定义 `CODEX_HOME`，请在启动悬浮窗前设置该环境变量；用量读取子进程会继承它。
- 如果悬浮窗启动失败，会显示错误提示，并将详细信息写入 `%LOCALAPPDATA%\CodexUsageOverlay\startup-error.log`。
- 用量读取状态会记录在 `%LOCALAPPDATA%\CodexUsageOverlay\usage-status.json`，包含最近更新时间和错误信息，不包含用量数值。
- 如果移动了程序文件夹，手动启动一次即可更新登录启动项中的路径。

启动时不需要打开 Codex Pets，也不需要管理员权限。该工具只支持 Windows，目前需要 PowerShell 和 Codex 桌面应用保持可用。
