# Codex Usage Overlay (Tauri v2)

[![简体中文](https://img.shields.io/badge/Language-%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-2F6FEB?style=for-the-badge)](README.md) [![English](https://img.shields.io/badge/Language-English-lightgrey?style=for-the-badge)](README.en.md)

现代化、超轻量级的 OpenAI Codex 桌面悬浮窗小部件。

通过本机 Codex `app-server` 的 `account/rateLimits/read` 接口获取实时用量与配额，无需配置 `auth.json`，不调用远程未授权网页接口。

基于 **Tauri v2 (Rust + React + Tailwind CSS + Windows WebView2)** 构建，常驻内存仅需 ~25MB（原 PowerShell 方案的 1/4），无控制台黑框，不触发杀毒软件误报。

## 下载

从 [最新版本](https://github.com/SeptYagu/codex-usage-overlay/releases/latest) 下载编译好的安装包或便携版本。发布页还提供 [SHA256 校验值](https://github.com/SeptYagu/codex-usage-overlay/releases/latest/download/SHA256SUMS.txt)。

---

## 核心特性

- **置顶透明悬浮窗**：固定置顶、无边框、无任务栏图标、支持亚克力/毛玻璃磨砂质感；背景透明度可调，文字与数值始终保持高对比度清晰易读。
- **平滑自由拖动**：鼠标左键按住悬浮窗即可随心拖曳摆放；程序智能识别多显示器虚拟屏幕边缘，关闭后自动将坐标保存至 `%LOCALAPPDATA%\CodexUsageOverlay\overlay-position.json`。
- **实时用量监控**：
  - **5H / WK 剩余百分比**：直观展示 5 小时与每周用量配额，依据阈值动态着色（$\ge 50\%$ 绿色、$\ge 20\%$ 黄色、$< 20\%$ 红色警示）。
  - **动态倒计时**：前端本地秒级计算重置倒计时（例如 `167H 05min`），无需重复频繁向 Codex 轮询。
  - **Credit 余额显示**：支持查看 Credit 余额（`CR`）或无限额度（`∞`）。
- **系统托盘集成**：
  - 动态双环仪表盘托盘图标，随配额消耗动态缩短与变色。
  - Hover Tooltip 实时显示 `5H 80% | WK 60% | CR 12.50`。
  - 左键双击托盘图标快速显隐悬浮窗。
  - 右键托盘菜单支持：立即刷新用量、浮窗设置、开机自启切换、退出程序。
- **现代化设置面板**：
  - **多语言界面**：支持简体中文 (`zh-CN`) 与 English (`en-US`)。
  - **浮窗缩放**：100% ~ 250% 自由无级缩放。
  - **背景透明度**：0%（实体黑底）~ 80%（高透玻璃）即时调节预览。
  - **刷新频率控制**：可按需选择 15 秒、30 秒、60 秒、2 分钟、5 分钟。
  - **开机自动启动**：一键开启 Windows 登录自启动。
- **进程与通讯优化**：
  - Rust 异步 Tokio 直接处理 stdio JSON-RPC 通信，彻底消除旧脚本每分钟反复唤醒 PowerShell 子进程带来的 CPU 抖动。
  - 单实例互斥保护：已运行时再次双击直接呼出已有窗口。

---

## 开发与构建

### 运行环境要求
- **Windows 10 / 11**
- **Node.js** >= 18
- **Rust** >= 1.78 (`cargo`, `rustc`)

### 启动开发模式
```powershell
# 1. 安装前端依赖
npm install

# 2. 启动 Tauri 开发模式（支持热重载）
npm run tauri dev
```

### 构建独立发布版本
```powershell
npm run tauri build
```
编译产物位于 `src-tauri/target/release/codex-usage-overlay.exe` 及 NSIS 安装包。

---

## 历史版本（PowerShell 脚本版）

原基于 PowerShell 5.1 + WPF 的独立脚本及打包测试工具依然完整保留在项目根目录与 `scripts/` 中：
- **启动与控制**：`Start-CodexUsageOverlay.cmd`、`Start-CodexPetUsage.ps1`、`Stop-CodexPetUsage.ps1`
- **核心逻辑**：`CodexPetUsageOverlay.ps1`、`Get-CodexUsage.ps1`、`Run-CodexUsageOverlay.ps1`、`Localization.ps1`
- **便携打包与验证**：`scripts/Build-Portable.ps1`、`scripts/Test-Portable.ps1`

如果需要在无 WebView2 的极端受限环境运行，依然可直接双击根目录下的 `Start-CodexUsageOverlay.cmd` 启动 PowerShell 脚本版。

### 兼容性说明
- 适用于 Windows 10/11。
- 默认使用 Windows 自带的 Windows PowerShell 5.1，无需安装 PowerShell 7，无需管理员权限。
- 需要安装并登录 Codex 桌面应用。程序会自动查找 PATH 中的 Codex、用户目录下的 Codex CLI，或 Microsoft Store 版本的 Codex。
- 如果 Codex 安装在自定义位置，可在启动前设置 `CODEX_CLI_PATH`。

---

## 反馈与交流
如有建议或问题，欢迎联系：`septwind@agent.qq.com`。
