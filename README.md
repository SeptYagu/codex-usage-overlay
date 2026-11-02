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
- 可选开启“额度不足时暂停 Codex”：任一剩余额度达到或低于阈值时（默认 5 小时 5%、每周 2%），应用通过 Codex 桌面客户端的控制面发送协作式“暂停”请求。设置、托盘菜单和托盘提示显示保护状态；设置详情区分“请求已接收”和“已确认中断”，并显示各会话结果。保护默认关闭，请求去重状态保存在内存中。
- 开启保护后，任一额度距离阈值不超过 5 个百分点时，刷新间隔缩短为 15 秒；连续两次成功读取确认两项额度均高于各自阈值 7 个百分点后，恢复你选择的正常间隔。读取失败时逐步延长重试间隔。设置中显示样本时间和实际刷新间隔。
- 左键拖动浮窗；右键可刷新、打开设置、隐藏或退出。启用“自动贴边隐藏”后，将浮窗拖到屏幕工作区边缘会收成两条配额指示，悬停时展开。支持多显示器自由拖拽，松手相交面积超过一半时自动归属到对应屏幕，内部接缝平滑漫游不误折叠，屏幕外侧物理边缘安全吸附。贴边指示条支持开启“百分比格子”（10 等分刻度线），便于直观推算用量。左键单击托盘图标可立即显示或隐藏浮窗（包括贴边状态）；左键双击只切换一次。右键单击可打开或收起托盘菜单。
- 设置窗口采用双栏并排布局（960×620），支持自由调整大小与几何位置记忆。展开保护详情或缩小窗口时可滚动查看。
- 可在设置页或托盘菜单开启“鼠标穿透”，使鼠标操作落到浮窗下方的窗口。穿透时可随时从托盘菜单关闭；系统托盘不可用时不会开启穿透。
- 仅安装版提供开机自启设置。启动便携版或开发版不会修改安装版的自启注册表项。
- 托盘菜单可切换语言、检查更新；发现新版本时，“检查更新”下方的状态行可点击安装，安装失败后可直接重试。设置中可关闭自动检查，或允许自动安装更新（默认关闭；开启后也会开启自动检查）：新版会在后台下载、验签，并在正常退出时安装，使用中不会因自动检查而突然关闭。安装版在下次打开时使用新版；便携版退出后由助手替换主程序。系统安装进度或提权提示仍可能出现。更新已就绪时，也可从托盘状态行立即安装。开启“更新时包含预览版”后也会收到预览版（默认关闭）。预览版带有编号（`1.5.0-preview.1` < `1.5.0-preview.2` < `1.5.0`），因此每个预览版都能更新到下一个预览版或正式版。
- Codex 可执行文件遵循原有的自定义路径/PATH/CLI/Store 查找顺序。文件移动或消失后，后续轮询可重新查找。自定义位置可通过 `CODEX_CLI_PATH` 指定。

设置、展开位置、贴边状态和用量状态保存在 `%LOCALAPPDATA%\CodexUsageOverlay\` 下的 `settings.json`、`window-position.json`、`dock-state.json` 和 `usage-status.json` 中。自定义音效文件保留在你选择的位置。

智能暂停依赖 Codex 配合。请求已接收并不代表已确认推理、排队任务或后台命令停止。关闭保护、读取失败或新的不再符合条件的额度数据会取消尚未发送的请求；已经提交的请求仍可能完成。

“重置后自动恢复”是独立选项，默认关闭。当前连接无法验证指定回合的中断、完成及后续用户操作，因此暂不可用。恢复计划保存在同一运行目录下的 `pause-resume-state.json`，协作式请求已接收不足以创建可执行计划。重启后待处理计划需要人工检查，提交结果未知的计划不会自动重试。此版本只发送暂停请求，不再创建外部“继续”定时任务；旧版本由 Codex 创建的任务仍可能运行，关闭浮窗保护不会取消这些外部任务。

## DSH 插件订阅用量

新增独立 **DSH** 来源：安装同一发布页附带的 companion 插件，明确选择 DSH 实例、订阅账户和普通会话。仅读取该实例的订阅插件额度，不会混用或回退到本机 Codex 读数；历史也按实例和账户隔离。DSH 的两个暂停阈值独立设置，默认均为 0%。

开启保护后，额度耗尽时只向所选运行会话发送协作式“暂停”消息；不是强制中断，不支持自动恢复。详见 [DSH 安装、配对与行为边界](docs/dsh-subscription.md)。升级 overlay 不会自动安装或重启 DSH。

## Claude 用量

在设置中将“用量来源”切换为 **Claude**，浮窗即显示 Claude 订阅额度（5 小时会话窗口和每周窗口），代替 Codex。同一时间只使用一个来源，默认仍为 Codex。

- 点击“连接 Claude Code”。浮窗会将附带的 `claude-usage-bridge.exe` 注册为 Claude Code 的状态栏命令，并在 `~/.claude/settings.json`（遵循 `CLAUDE_CONFIG_DIR`）中添加 `UserPromptSubmit` 和 `PreToolUse` 两个钩子。原文件会备份一次为 `settings.json.codex-overlay.bak`。原有状态栏通过助手继续显示；“断开连接”会恢复它，并只移除浮窗添加的条目。钩子改动对新的 Claude Code 会话生效。
- 默认情况下，额度来自 Claude Code 传给状态栏的 `rate_limits` 数据，需要 Claude Code 2.1.80 或更高版本，并使用 Pro 或 Max 套餐登录；API Key 会话不提供该数据。此模式下浮窗不读取 Claude 凭据，也不发起网络请求。额度只在使用**终端 `claude` CLI** 时更新：桌面版和 IDE 扩展不会运行状态栏。
- “使用 Claude 登录实时读取额度”（默认关闭）适用于桌面版。浮窗最多每分钟请求一次 Claude Code `/usage` 背后的用量接口，使用终端 CLI 保存在 `~/.claude/.credentials.json` 中的登录信息。令牌每次请求时读取，不会保存、记录或发送到其他地方。该接口未公开，可能变化。桌面版的登录不会共享，因此请先用 `claude` → `/login` 登录一次。CLI 登录几小时后会过期；此时浮窗会运行 `claude plugin list`，必要时再运行 `claude doctor`（最多每十分钟一次），由 Claude Code 自行续期，不启动会话，也不消耗额度。浮窗从不写入登录凭据。实时读取的结果会写入同一本地文件，额度保护钩子也会使用。实时读取失败或接口要求稍后再试时，浮窗继续显示该文件中的上次读数及其时间，设置中显示失败原因。重置时间已过的窗口显示为已重置；重置通知在收到新周期的第一份样本时发出。
- 开启保护开关后，5 小时或每周剩余额度达到或低于阈值时，钩子会拒绝新的提问和工具调用，Claude Code 会显示原因。Claude 不使用 Codex 智能暂停。旧样本在其重置时间之前仍然有效，因为窗口内已用额度只增不减。缺少或无法读取样本、重置时间已过或开关关闭时，请求一律放行。
- 浮窗开头显示当前来源的图标（Codex 为 OpenAI，Claude 为 Claude），可用“显示来源图标”隐藏。图标取自 Simple Icons（Claude，CC0）和 LobeHub Icons（OpenAI，MIT），商标归各自所有者。Claude 不显示 Credit 余额。消耗速率与重置状态保存在以 `claude-` 开头的独立文件中，Claude 样本不会混入 Codex 历史。
- 助手写入 `claude-rate-limits.json` 和 `claude-guard-state.json`，浮窗写入 `claude-guard.json` 和 `claude-bridge.json`，均位于上述运行目录。卸载前请先断开连接，否则 Claude Code 会继续调用已删除的助手。

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
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin claude-usage-bridge
npm run tauri -- build --ci --bundles nsis,msi
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin codex-usage-updater
.\scripts\Build-TauriRelease.ps1 -Tag v1.5.1
```

发布前需设置 Tauri 更新签名私钥环境变量。标签发布流程会生成 NSIS、MSI、便携 ZIP、签名和 `latest.json`；便携版更新助手也会一并打包。安装包会包含打包时已存在的所有 Cargo 可执行文件，因此需在 `tauri build` 之前构建 Claude 助手。
预览版的自动化结果与尚未完成的 Windows 实机检查见[预览说明](docs/v1.1.0-preview-notes.md)。

## 开源协议

本项目采用 [MIT License](LICENSE) 许可协议开源。

反馈邮箱：septwind@agent.qq.com
