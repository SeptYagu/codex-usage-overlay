# Codex Usage Overlay（Tauri v2）

[![GitHub Release](https://img.shields.io/github/v/release/SeptYagu/codex-usage-overlay?color=3b82f6&logo=github)](https://github.com/SeptYagu/codex-usage-overlay/releases/latest)
[![CI](https://github.com/SeptYagu/codex-usage-overlay/actions/workflows/ci.yml/badge.svg)](https://github.com/SeptYagu/codex-usage-overlay/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-10b981.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows-0078D6?logo=windows)](https://github.com/SeptYagu/codex-usage-overlay/releases/latest)

简体中文 · [English](README.en.md)

Windows 桌面悬浮窗，显示 Codex 配额和 Credit 余额。通过本机 Codex `app-server` 的 `account/rateLimits/read` 接口获取用量，不读取或保存 `auth.json`，不修改 Codex 安装文件。

## 下载与启动

从[发布页](https://github.com/SeptYagu/codex-usage-overlay/releases)下载最新发布版，可选择 NSIS 安装程序、MSI 安装包或 Windows x64 便携 ZIP。`SHA256SUMS.txt` 包含全部三个包的校验值。

安装程序直接安装；便携版解压后运行 `CodexUsageOverlay.exe`。需要 Windows 10/11、WebView2，以及已安装并登录的 Codex 应用。无需额外 PowerShell 启动脚本；查找 Microsoft Store 版 Codex 时会使用 Windows 自带的 PowerShell。

## 浮窗与设置

- 显示 5 小时和每周配额的**剩余百分比**、重置倒计时及可选的 Credit 余额。数值余额保留两位小数，无限余额显示 `∞`。
- 默认使用**分组胶囊**布局：配额值下方显示倒计时，余额居中排列。**指标堆叠**布局在数值上方显示标签，下方显示倒计时或余额说明。可在设置中切换。
- 两种布局均跟随 Windows 浅色/深色主题。支持缩放（100%–250%）、背景透明度（0%–80%）、余额显隐、刷新间隔（30/60/120/300 秒）及中、英、繁体中文。
- 设置即时生效，并在各窗口间同步。旧设置文件保留原有偏好，布局默认为分组胶囊。
- 每个配额都可单独开启重置通知，只有确认进入新周期后提醒，每周期最多一次。可用 Windows 默认提示音，或选择 MP3、AAC、M4A、WAV 文件试听；自定义音效最多播放 10 秒，应用只保存原文件路径。
- 左键拖动浮窗；右键可刷新、打开设置、隐藏或退出。启用“自动贴边隐藏”后，将浮窗拖到屏幕工作区边缘会收成两条配额指示，悬停时展开。左键或右键单击托盘图标都会打开菜单，可在菜单中显示或隐藏浮窗。
- 可在设置页或托盘菜单开启“鼠标穿透”，使鼠标操作落到浮窗下方的窗口。穿透时可随时从托盘菜单关闭；系统托盘不可用时不会开启穿透。
- 仅安装版提供开机自启设置。启动便携版或开发版不会修改安装版的自启注册表项。
- 托盘菜单可切换语言、检查更新；发现新版本时，“检查更新”下方的状态行可点击安装，安装失败后可直接重试。设置中可关闭自动检查，或允许自动安装更新（默认关闭；开启后也会开启自动检查）：新版会在后台下载、验签，并在正常退出时安装，使用中不会因自动检查而突然关闭。安装版在下次打开时使用新版；便携版退出后由助手替换主程序。系统安装进度或提权提示仍可能出现。更新已就绪时，也可从托盘状态行立即安装。
- Codex 可执行文件遵循原有的自定义路径/PATH/CLI/Store 查找顺序。文件移动或消失后，后续轮询可重新查找。自定义位置可通过 `CODEX_CLI_PATH` 指定。

设置、展开位置、贴边状态和用量状态保存在 `%LOCALAPPDATA%\CodexUsageOverlay\` 下的 `settings.json`、`window-position.json`、`dock-state.json` 和 `usage-status.json` 中。自定义音效文件保留在你选择的位置。

## 开发与构建

安装 Node.js 22、当前稳定 Rust 工具链及 Windows Tauri 构建依赖。

```powershell
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run tauri dev
```

构建并打包发布版：

```powershell
npm run tauri -- build --ci --bundles nsis,msi
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin codex-usage-updater
.\scripts\Build-TauriRelease.ps1 -Tag v1.1.0-preview
```

发布前需设置 Tauri 更新签名私钥环境变量。标签发布流程会生成 NSIS、MSI、便携 ZIP、签名和 `latest.json`；便携版更新助手也会一并打包。
预览版的自动化结果与尚未完成的 Windows 实机检查见[预览说明](docs/v1.1.0-preview-notes.md)。

## 开源协议

本项目采用 [MIT License](LICENSE) 许可协议开源。

反馈邮箱：septwind@agent.qq.com
