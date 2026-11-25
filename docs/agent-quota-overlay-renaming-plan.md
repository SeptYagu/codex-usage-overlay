# Codex Usage Overlay 改名为 Agent Quota Overlay：步骤规划

**状态：** 规划文档；本文件不执行改名、迁移、发布或安装

**基线：** 2026-11-24，公开版 `main` 的 `e2aa5c0`（应用版本 1.10.0）

## 1. 目标和命名约定

用户可见的英文名称拟定为 **Agent Quota Overlay**，中文名称为 **Agent 额度悬浮窗**；仓库目标名称拟定为 `agent-quota-overlay`。名称说明产品同时监控 Codex、Claude、DSH 和 Antigravity，也允许以后增加来源。首次改名的发布说明和 README 使用“Agent Quota Overlay（原 Codex Usage Overlay）”，方便旧用户确认这是同一产品。

本规划把**品牌名称**与**安装身份**分开处理。现有程序、签名更新和桥接配置都包含旧名；首次品牌发布保留内部安装身份、数据目录和既有更新入口。任何物理文件名或标识迁移，都以旧版能原地升级、数据无损和回退路径明确为前提。

## 2. 现状与影响清单

| 对象 | 当前锚点 | 改名处理 |
| --- | --- | --- |
| 界面和说明 | [`tauri.conf.json`](../src-tauri/tauri.conf.json) 的窗口标题；[`i18n.ts`](../src/i18n.ts)、[`ru-RU.json`](../src/locales/ru-RU.json)、托盘与通知文案；中英文 README | 第一阶段更新用户可见名称和说明。 |
| Tauri/安装身份 | `productName=codex-usage-overlay`、`identifier=com.codex.usage.overlay`；[`Cargo.toml`](../src-tauri/Cargo.toml) 的包与主二进制名 | 第一阶段保留。以后逐项验证 MSI、NSIS、开机自启和旧版升级，再决定是否迁移。 |
| 本地持久化 | [`config.rs`](../src-tauri/src/config.rs) 使用 `%LOCALAPPDATA%\CodexUsageOverlay\`；[`agent-bridge`](../src-tauri/crates/agent-bridge/src/lib.rs) 和 DSH 安装器也使用该目录 | 默认长期保留旧目录作为兼容路径；若确需迁移，另做有回滚的专门设计。 |
| 便携版更新 | [`Build-TauriRelease.ps1`](../scripts/Build-TauriRelease.ps1) 打包 `CodexUsageOverlay.exe`、`CodexUsageUpdater.exe`；[`commands/updater.rs`](../src-tauri/src/commands/updater.rs) 按这两个条目取包并替换 | 首次品牌发布保留 ZIP 内名称；以后先交付能同时识别旧、新条目的过渡版。 |
| 安装版与发布资产 | 构建脚本、[`Test-TauriRelease.ps1`](../scripts/Test-TauriRelease.ps1)、[`release.yml`](../.github/workflows/release.yml)、[`RELEASING.md`](../RELEASING.md) 使用旧二进制、安装包、ZIP 与签名名称 | 名称变更要同步构建、校验、`latest.json`、签名和发布流程，不能只改产物显示名。 |
| 更新服务地址 | [`tauri.conf.json`](../src-tauri/tauri.conf.json)、[`update_channel.rs`](../src-tauri/src/update_channel.rs)、构建脚本包含 `SeptYagu/codex-usage-overlay` | 仓库改名之前，用已安装旧版实测旧 URL 的更新链路；不能仅凭网页跳转判断更新可用。 |
| 外部桥接标识 | Antigravity 钩子名 `codex-usage-overlay`、Claude/Antigravity helper 文件名、DSH 包名 `dsh-codex-usage-overlay-bridge` | 第一阶段保留。若改名，须能识别并清理自己安装的旧条目，同时保留用户原有钩子和备份。 |
| 历史资料 | 旧版发布说明、历史评审和测试快照 | 保留历史名称与当时证据；只更新当前产品说明和仍执行的脚本。 |

GitHub 官方说明：仓库改名后通常会重定向旧仓库的网页与 Git 访问，但**引用该仓库的 Action/可复用工作流不会随之重定向**，旧仓库名也不应被重新占用。因此更新器和工作流都需要按实际调用路径核验，不能把重定向当成唯一保障。[Renaming a repository](https://docs.github.com/en/repositories/creating-and-managing-repositories/renaming-a-repository)

## 3. 旧版用户的无感升级契约

这里的“无感”指旧用户沿用应用内更新，不需要手动卸载、重装、重新配置或处理第二份安装。Windows 安装程序仍可能显示进度或系统提权提示，Tauri 在安装更新时会退出正在运行的应用，不能承诺完全没有界面和重启。[Tauri Updater](https://v2.tauri.app/plugin/updater/)

1. 已发布旧版的正式通道先从硬编码的旧仓库 URL 读取 `releases/latest/download/latest.json`，再按安装形态选取清单中的 NSIS、MSI 或便携包，校验原公钥对应的签名，且只接受更高版本。启用预览通道的旧版还使用旧仓库的 Release API 地址，需单独验收。旧客户端的这些入口不能靠未来代码修补；仓库改名后，旧 URL 和清单内的实际资产地址必须继续可访问。
2. 新品牌首版继续使用旧 Tauri `identifier`、`productName`、安装身份、运行目录、更新签名公钥和便携 ZIP 条目名。用户在窗口、托盘和帮助中看到新名称，现有安装位置、Windows 安装列表及可执行文件名在这一版仍可能显示旧名称。Tauri 的 `productName` 会影响 Windows 安装目录、开始菜单、注册表项和默认 MSI UpgradeCode，因此不能把它当作单纯的界面文案替换。[Tauri Configuration](https://v2.tauri.app/reference/config/#productname)
3. 现有便携版更新器固定读取 ZIP 内的 `CodexUsageOverlay.exe` 和 `CodexUsageUpdater.exe`。当前更新入口是静态 `latest.json`，旧用户可以跳过任何中间版本而直接拿到将来的最新版。因此**一次过渡版不够**：此入口指向的每一个后续便携包都必须保留旧客户端可读取的条目，且 NSIS/MSI 包仍能升级旧安装；除非先有经过真实旧版验证的其他兼容分流机制。包内可以额外提供新名称，但不能提前删除旧条目。
4. 新版应用继续读取原 `%LOCALAPPDATA%\CodexUsageOverlay\` 配置和历史，沿用原桥接标识或提供已验证的无损迁移。不要因改名自动生成空的新设置目录，或让旧版开机自启项同时启动第二份程序。
5. 用实际已发布旧版分别执行“旧 NSIS → 新版”“旧 MSI → 新版”“旧便携版 → 新版”的直接跳版更新；核对同一安装、同一设置/历史、同一自动启动项、可用桥接和下一次更新。只验证相邻版本升级不足以证明旧用户无感升级。

## 4. 分阶段执行

### 阶段 0：锁定基线与兼容矩阵

1. 在开始改代码前记录当时的最新正式版、预览版、远端分支、签名公钥、当前安装包三种形态（NSIS、MSI、便携 ZIP）及其实际更新地址；只使用可复现的已发布包做旧版样本。
2. 建立旧版 → 新版矩阵：NSIS 原地升级、MSI 原地升级、便携版更新助手替换、开机自启、设置/额度历史/窗口位置、Claude 与 Antigravity 钩子、DSH 连接。记录每项原始路径、文件名和验收结果。
3. 全局检索旧名称，但按“用户文案、构建产物、持久化身份、外部协议、历史资料”归类。保留 `Codex` 来源本身的名称；不做无差别替换。

**通过条件：** 有一份可核对的基线和升级矩阵，能够区分哪些旧名是品牌，哪些旧名是兼容标识。

### 阶段 1：只更改用户可见品牌

1. 更新窗口标题、设置标题、托盘、通知和额度保护提示中的产品名、多语言文案、中英文 README、当前截图/图标说明和发布说明；来源标签仍分别叫 Codex、Claude、DSH、Antigravity。
2. 第一版保留 Tauri `identifier`、`productName`、Cargo 包/二进制名、`%LOCALAPPDATA%\CodexUsageOverlay\`、helper/钩子/DSH 包名、GitHub 仓库地址与更新清单路径。应用内显示新品牌，旧安装和自动更新继续使用原身份；Windows 安装列表和文件名的旧名称在发布说明中解释。
3. 检查英语、简体、繁体、俄语界面和 Windows 通知；确认当前帮助文本准确描述多来源功能，不再把 Codex 登录写成所有来源的唯一前提。

**通过条件：** 自动化检查通过；三种旧版安装形态升级到该版本后仍只保留一份程序，设置与历史不变，更新签名有效，桥接连接和开机自启正常。发布说明明确告知旧文件名/数据目录仍为兼容名称。

### 阶段 2：评估并执行仓库改名

1. 先用独立测试仓库演练仓库改名、旧 Release URL 重定向、`latest.json` 下载和工作流引用，记录预期行为。演练不能代替实际旧版升级验收；若测试环境已暴露更新路径问题，暂不改正式仓库名，先设计可持续的旧地址兼容入口。
2. 检查其他仓库/文档是否以 `SeptYagu/codex-usage-overlay@...` 调用本仓库 Action 或可复用工作流；这些引用不能依赖 GitHub 重定向。检查徽章、下载链接、发布脚本、更新通道、Git remote 和外部安装说明。
3. 前两项通过后才将仓库改为 `agent-quota-overlay`，更新当前文档、CI/Release 工作流及本地 `origin`。立即用已安装旧版验证原 URL 的更新查询、资产下载、签名核验和预览版查询；失败则暂停新版本发布并恢复可用的旧地址。旧名不再创建同名新仓库，以免切断重定向。保留历史发布说明中的旧链接语境，逐项验证仍可访问。

**通过条件：** 旧版客户端仍能从原地址发现并安装新版本；新版客户端使用新地址；三种发布资产、校验值和签名均可下载且匹配；CI 和发布工作流在新仓库执行成功。

### 阶段 3：决定是否更改二进制、安装身份和数据目录

1. 先评估用户收益。若旧内部名称不影响使用，保留它们并在维护文档中标记为兼容常量；项目完成品牌改名不要求强制搬迁每个旧路径。
2. 若决定改可执行文件或安装包名称，新版更新器先兼容**旧、新便携 ZIP 条目**，但面向已发布旧版的静态更新入口必须持续提供旧版可读取的条目；不能只发布一次过渡版后删除旧条目。真正迁移便携版主程序文件名时，还需处理原路径上的旧启动入口和更新助手，不能只在 ZIP 内新增一个新名字。对 NSIS/MSI 分别核对升级身份、卸载记录和升级后的应用数量；如要更改 `productName`，先从已发布 MSI 核对并固定原 UpgradeCode，再实测升级。不能假定改 `productName` 或 `identifier` 后仍会原地升级。
3. 若决定迁移数据目录，设计幂等的单向迁移：确认旧目录所有权、原子复制与校验、失败回滚；旧版本再次运行或更新时不能产生两份相互覆盖的设置。没有可靠方案时继续使用旧目录。
4. 若决定改 Claude/Antigravity 钩子名或 DSH 包名，以明确的旧标识识别自家条目，保留用户自定义条目和备份；断开、升级、卸载都要在旧/新配置上验证。任何迁移不得通过杀进程或清空用户配置实现。

**通过条件：** 所有已发布旧版到新版本的安装、便携更新和卸载回归通过；没有重复开机项、重复钩子、丢失历史、错误安装类型识别或不可恢复的半迁移。未通过时保持旧内部名称。

## 5. 每阶段的交付与发布门槛

- 各阶段分别提交，只纳入本阶段改动并审阅 diff。代码阶段运行 `npm run build`、`npm test`、`cargo test --manifest-path src-tauri/Cargo.toml --workspace --locked` 和 `./scripts/Test-TauriRelease.ps1`；仅文档阶段做链接与 `git diff --check`。
- 推送 `main` 后等待该提交的 Windows `CI` 成功。按 [`AGENTS.md`](../AGENTS.md) 和 [`RELEASING.md`](../RELEASING.md) 更新版本及发布说明，再为**同一已验证提交**创建标签；正式 Windows 包只由 `Release overlay` 工作流签名发布。
- 对阶段 1–3 的正式包核对 NSIS、MSI、便携 ZIP、三份更新签名、`latest.json`、`SHA256SUMS.txt` 与公开 Release，并用已安装旧版实测升级。未完成的实机验收写入发布说明，不用单元测试代替升级证明。
- 任何阶段发现旧版无法升级、目标文件名不匹配、签名失败或设置分叉，停止该阶段发布，修复并重新验收；保留上一版可用的下载与更新路径。

## 6. 本次文档任务边界

本规划只确定顺序、兼容要求和验收门槛。实际品牌、仓库、安装身份、运行目录和外部桥接配置的改动将在后续阶段分别实施与验证。
