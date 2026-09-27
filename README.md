# Codex Usage Overlay（Tauri v2）

简体中文 · [English](README.en.md)

Windows 桌面悬浮窗，显示 Codex 配额和 Credit 余额。通过本机 Codex `app-server` 的 `account/rateLimits/read` 接口获取用量，不读取或保存 `auth.json`，不修改 Codex 安装文件。

## 下载与启动

从[发布页](https://github.com/SeptYagu/codex-usage-overlay/releases)下载 Tauri 预览版，可选择 NSIS 安装程序、MSI 安装包或 Windows x64 便携 ZIP。`SHA256SUMS.txt` 包含全部三个包的校验值。

安装程序直接安装；便携版解压后运行 `CodexUsageOverlay.exe`。需要 Windows 10/11、WebView2，以及已安装并登录的 Codex 应用。无需 PowerShell 启动脚本；查找 Microsoft Store 版 Codex 时会使用 Windows 自带的 PowerShell。

稳定版 `v0.1.x` 是旧 PowerShell 版本；Tauri 重写版本目前以预览版发布。

## 浮窗与设置

- 显示 5 小时和每周配额的**剩余百分比**、重置倒计时及可选的 Credit 余额。数值余额保留两位小数，无限余额显示 `∞`。
- 默认使用**分组胶囊**布局：配额值下方显示倒计时，余额居中排列。**指标堆叠**布局在数值上方显示标签，下方显示倒计时或余额说明。可在浮窗设置中切换。
- 两种布局均跟随 Windows 浅色/深色主题。支持缩放（100%–250%）、背景透明度（0%–80%）、余额显隐、刷新间隔（30/60/120/300 秒）及中英文。
- 设置即时生效，并在各窗口间同步。旧设置文件保留原有偏好，布局默认为分组胶囊。
- 按住鼠标左键拖动；右键浮窗可刷新、打开设置、隐藏或退出。单击托盘图标可显示或隐藏浮窗。
- 仅安装版提供开机自启设置。启动便携版或开发版不会修改安装版的自启注册表项。
- Codex 可执行文件遵循原有的自定义路径/PATH/CLI/Store 查找顺序。文件移动或消失后，后续轮询可重新查找。自定义位置可通过 `CODEX_CLI_PATH` 指定。

设置、位置和用量状态保存在 `%LOCALAPPDATA%\CodexUsageOverlay\` 下的 `settings.json`、`window-position.json` 和 `usage-status.json` 中。

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
.\scripts\Build-TauriRelease.ps1 -Tag v0.3.2-preview
```

标签发布流程会测试并构建 Tauri 应用，再发布 NSIS、MSI、便携 ZIP 和校验文件。打包时会验证便携包内容与源文件一致。

本分支已移除旧 PowerShell 实现和启动脚本；历史 `v0.1.x` 发布版仍可下载。

反馈邮箱：septwind@agent.qq.com
