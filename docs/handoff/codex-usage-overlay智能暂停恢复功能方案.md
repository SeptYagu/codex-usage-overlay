# codex-usage-overlay v1.3.1 智能暂停恢复功能制作方案

- 编制日期：2026-10-18
- 目标项目：`D:\OneDrive\AiPrograms\codex tools\codex-usage-overlay`
- 目标版本：`1.3.1`（当前 `1.3.0` 的下一个补丁版本）
- 文档性质：功能需求、技术设计、实施步骤与验收标准
- 实施状态：方案已确认，待实现
- 方案范围：低额度自动暂停、5 小时额度重置后的智能恢复、多种 Codex 客户端桥接兼容

## 1. 目标与非目标

### 1.1 目标

在用户明确启用功能后，应用依据最新且成功读取的 Codex 配额数据，在 5 小时或 Weekly 剩余额度到达各自阈值时，向当前正在运行的 Codex 主会话发送协作式暂停提示。对于只有 5 小时额度触发且重置时间可靠的情况，同时要求 Codex 创建一次性定时任务，在重置后 2 分钟只发送 `继续`，从而降低额度重置前继续消耗以及重置瞬间过早恢复的风险。

应用应同时兼容当前与较旧的 Codex 桌面客户端桥接方式，并在桥接不可用、请求超时或会话状态变化时保持额度显示、设置保存、托盘更新和应用退出不受阻塞。整个功能必须是可选增强能力；关闭开关或桥接不可用时，应用应保持现有行为。

### 1.2 非目标

本功能不实现：

- 强制终止 Codex 进程、线程或模型推理；
- 绕过 Codex 客户端自身的会话控制面直接修改会话文件；
- 为闲置会话启动一个仅用于发送暂停提示的新回合；
- 启动独立 app-server 与桌面客户端竞争会话写入；
- 可编辑的任意提示词、多个告警级别或基于 Burn Rate 的预测触发；
- 持久化暂停消息去重状态；
- 对所有历史 Codex 客户端版本作无条件兼容承诺。

这里的“暂停”是用户消息驱动的协作请求，不是硬中断保证。

### 1.3 当前代码基线与差距

本方案已按目标项目 2026-10-18 的 `main` 分支代码校准；检查时本地与 `origin/main` 一致，基线提交为 `61566eb`，项目版本元数据为 `1.3.0`。这是编写方案时的定位基线，不是未来实施时必须使用的固定版本；开始编码前仍须重新同步并复核。

当前代码与本功能的对应关系如下：

| 当前模块 | 已有能力 | 本功能缺口 | 计划接入方式 |
| --- | --- | --- | --- |
| `src-tauri/src/codex/client.rs` | `RawCodexUsage` 已含 `account_id`、两个额度窗口及 `fetched_at`；`CodexUsage` 已含剩余百分比和重置时间 | 没有会话消息桥接 | 保持额度读取职责不变，仅在转换前复制 `account_id` |
| `src-tauri/src/commands.rs::refresh_usage` | 所有定时、托盘和前端刷新最终汇入同一条成功路径，并有 `refresh_guard` | 成功样本发布后没有自动暂停调度 | 在 `process_usage_success` 完成后调用一个立即返回的调度入口 |
| `src-tauri/src/notify.rs::process_usage_success` | 写入最新用量、更新托盘、发出 `usage_updated`，并处理重置通知 | 重置通知与低额度会话控制语义不同 | 不把自动暂停塞入通知模块，避免持久化通知状态与内存消息去重互相污染 |
| `src-tauri/src/config.rs::OverlaySettings` | Serde 默认值可让旧配置自然迁移 | 缺少开关和两个阈值 | 增加三个带默认值的字段，不新增配置文件版本 |
| `src-tauri/src/commands.rs::validate_settings_patch` | 闭合白名单、整包拒绝、补丁合并与持久化链路已经存在 | 新字段尚未获准 | 在原白名单中增加严格 boolean/整数范围校验 |
| `src-tauri/src/commands.rs::AppState` | 已集中持有异步运行态 | 没有发送确认、在途和去重状态 | 增加一个只保护短时内存状态的 `StdMutex<AutoPauseController>`，生产初始化和 `for_test` 同步补齐 |
| `src/components/SettingsView.tsx` | 右栏已有刷新间隔、系统集成和通知设置，列本身可滚动 | 没有自动暂停控件 | 插在刷新间隔之后、自动启动之前；不扩大窗口，不依赖固定高度 |
| `src/types.ts` 与 `src/i18n.ts` | 设置类型、默认值和三语言资源集中维护 | 缺少新字段和文案 | 与 Rust 字段一次性对齐，并用现有设置补丁通道保存 |
| `src-tauri/Cargo.toml` | 已有 `tokio = { features = ["full"] }`、`serde_json` 和 `dirs` | 无 | 命名管道、超时和会话目录发现不需要新增依赖 |

当前源码中没有 `pause_control` 模块，也没有 `list_threads`、`send_message_to_thread`、`codex-app-tools-*` 或 `codex-browser-use-*` 相关路径；因此实施应以新增隔离模块和最小接线为主，不能假设目标项目已有半成品逻辑。

## 2. 用户设置

功能包含三个设置，并复用现有设置补丁、版本修订和广播链路：

| 设置 | 类型 | 默认值 | 有效范围 | 说明 |
| --- | --- | --- | --- | --- |
| `autoPauseOnLowQuota` | boolean | `false` | `true` / `false` | 总开关，默认关闭 |
| `autoPauseFiveHourThresholdPercent` | integer | `5` | 1–15 | 5 小时剩余额度阈值 |
| `autoPauseWeeklyThresholdPercent` | integer | `2` | 1–10 | Weekly 剩余额度阈值 |

界面要求：

- 控件位于设置页系统集成区域、刷新间隔设置下方；
- 总开关关闭时，两个阈值控件在视觉和交互语义上均禁用；
- 支持英文、简体中文和繁体中文；
- 旧设置文件迁移后使用上述默认值，且不得静默开启功能；
- 修改阈值不额外发起配额请求，在下一次成功的计划刷新或手动刷新时评估；
- 可显示“可用”“未连接”或最近一次确认成功的数量/时间，但未收到 Codex 确认前不得显示发送成功。

## 3. 触发数据与资格条件

### 3.1 数据来源

触发判断只能使用已经通过现有解析、账户处理和有序发布边界的最新成功 `RawCodexUsage` 样本。以下数据不得触发：

- 请求失败或未认证；
- 过期结果或被较新请求取代的结果；
- 仅来自缓存的显示数据；
- 格式错误、缺少有效剩余百分比或无法建立额度周期身份的数据。

有效额度字段：

- 5 小时额度：`fiveHourRemainingPercent` 与 `fiveHourResetsAt`；
- Weekly 额度：`weekRemainingPercent` 与 `weekResetsAt`。

自动暂停还要求样本含有非空 `account_id`。账户身份缺失时仍正常发布和显示额度，但本次样本不进入自动暂停发送，避免用不明确的占位账户构造跨账户去重键。

### 3.2 触发条件

只有同时满足以下条件才允许进入发送：

1. `autoPauseOnLowQuota` 为 `true`；
2. 至少一个有效额度的剩余百分比小于或等于自己的阈值；
3. Codex 桌面桥接握手成功，并返回至少一个状态为 `active` 的 Codex 主会话；
4. 目标会话在相应账户、额度种类和重置周期内尚未收到确认成功的提示；
5. 目标会话在发送前最近一次 `list_threads` 观察中仍为活动状态。

子代理、ChatGPT 会话、`idle`、`notLoaded` 或其他非活动记录不作为目标。

活动状态检查属于尽力而为的保护：`list_threads` 与 `send_message_to_thread` 之间存在短暂竞态，不能声称这是原子条件发送。实现应在发送前使用最新可得状态并尽量缩短两次调用之间的间隔；若会话恰在此间转为空闲，由客户端最终行为决定是否接受消息，不为这一边界引入额外控制面或复杂重试协议。

## 4. 消息选择与定时继续语义

### 4.1 决策矩阵

| 触发情况 | 发送内容 |
| --- | --- |
| 仅 5 小时额度触发，且重置时间有效 | `暂停，设置一次性定时任务，在Xh Ymin +2min后发送‘继续’两个字` |
| 仅 5 小时额度触发，但重置时间缺失、过期或不可用 | `暂停` |
| 仅 Weekly 额度触发 | `暂停` |
| 5 小时与 Weekly 同时触发 | `暂停`，Weekly 优先 |

消息不得附加解释、标点变体、隐藏指令或第二条立即发送的跟进消息。

### 4.2 5 小时剩余时间计算

仅 5 小时额度触发时：

1. 使用导致本次触发的同一新鲜样本中的 `fiveHourResetsAt`；
2. 以该样本的 `fetchedAt` 为计算起点，避免用稍后的本地当前时间引入不一致；
3. 计算 `fiveHourResetsAt - fetchedAt`；
4. 对不足一分钟的部分向上取整，防止提前恢复；
5. 将总分钟数转换为 `Xh Ymin`；
6. 提示中的 `+2min` 表示一次性任务应在检测到的重置时间之后再等待 2 分钟；
7. 一次性任务最终只发送两个汉字：`继续`。

如果时间差不为正、字段缺失或不能可靠解析，安全回退为 `暂停`，不得猜测恢复时间。

## 5. 多种 Codex 客户端兼容策略

### 5.1 支持的桥接族

- 当前 Codex 桌面客户端：`codex-app-tools-*`；
- 较旧 Codex 桌面客户端：`codex-browser-use-*`；
- 特殊受控启动：显式环境变量 `CODEX_APP_TOOLS_PIPE_PATH`。

### 5.2 候选顺序

桥接候选必须按以下优先级构建：

1. 非空且显式传入的 `CODEX_APP_TOOLS_PIPE_PATH`；
2. 按名称稳定排序的 `codex-app-tools-*` 当前客户端管道；
3. 按名称稳定排序的 `codex-browser-use-*` 旧客户端管道。

自动发现只接受带合法 UUID 形状后缀的支持前缀。路径以不区分大小写的方式去重，避免显式路径和枚举路径指向同一管道时重复探测。

### 5.3 可用性判定

发现管道不等于桥接可用。每个候选都必须通过与生产协议一致的、带确认的只读 `list_threads` 握手：

- JSON-RPC/工具调用被对端明确确认；
- 返回内容可以解析；
- 能识别合法的本地 Codex 会话身份；
- 只筛选 `kind=codex` 且 `status=active` 的会话。

握手失败后尝试下一个候选。不能仅凭管道名称存在就发送消息或宣称兼容成功。

该策略使新版 Overlay 能同时适配新旧桥接族，但实际兼容仍取决于目标 Codex 客户端是否支持并正确响应 `list_threads` 与 `send_message_to_thread`。没有逐版本实测的客户端只能称为“保留兼容路径”，不能称为“已验证支持”。

### 5.4 调用方线程身份发现

`tools/call` 还需要一个合法的本地 Codex 调用方线程 ID。候选按以下顺序生成：

1. 若环境变量 `CODEX_THREAD_ID` 是合法 UUID，置于第一位；
2. 从 `%USERPROFILE%\.codex\sessions` 递归收集 `.jsonl`，最大目录深度为 4；
3. 按修改时间倒序，只检查最近 32 个文件；
4. 从文件名末尾提取 36 字符 UUID，并去重；
5. 不读取会话正文，不解析提示内容，不把文件内容写入日志。

每一个“管道候选 × 调用方线程 ID”组合都只能通过 `list_threads` 握手来判定。环境变量或文件名只提供候选，不能作为授权成功的证据。

### 5.5 命名管道协议

Windows 适配器采用下列固定协议，不引入第二个 app-server：

1. 通过 `tokio::net::windows::named_pipe::ClientOptions` 打开管道；若 Windows 返回管道忙错误 231，以 40 ms 间隔重试，但受外层截止时间约束；
2. 请求体为 UTF-8 JSON；先写 4 字节 little-endian `u32` 长度，再写 JSON 正文并刷新；
3. 响应同样先读 4 字节长度，再读指定字节数；响应上限固定为 8 MiB，超过立即拒绝；
4. JSON-RPC 外层字段为 `jsonrpc: "2.0"`、单调递增 `id`、`method: "tools/call"`；
5. `params` 固定包含 `callerSource: "codex"`、`namespace: "codex_app"`、唯一 `callId`、`turnId`、调用方 `threadId`、工具名和参数；
6. JSON-RPC `error`、缺少 `result`、无法解析或读写中断均视为未确认失败。

`list_threads` 使用 `{ "limit": 50 }`。只有工具结果中的 `success` 为 `true`，且 `contentItems[0].text` 能解析为线程列表时才算握手成功。目标从 `pinnedThreads` 与 `threads` 两组中合并，按 `(hostId, id)` 去重，只保留 `kind == "codex" && status == "active"` 且 ID 非空的记录。

`send_message_to_thread` 参数固定为 `{ threadId, prompt }`，存在 `hostId` 时一并传入。只有工具结果 `success == true` 才算正向确认；其余返回、断线、超时和未知结果都不得写入已发送集合。

## 6. 发送、并发与故障隔离

### 6.1 发送路径

对每个符合条件的活动 Codex 主会话，通过桌面客户端自己的控制面调用 `send_message_to_thread`。不得使用额度读取所拥有的独立 app-server 子进程发送会话消息，也不得直接写 transcript/session JSONL。

发送前应以最新一次可得的 `list_threads` 结果跳过已变为空闲的会话，不得主动为提示创建新线程或显式启动闲置会话。该检查是尽力而为，不承诺消除状态检查与消息提交之间的竞态。

### 6.2 时限与异步执行

- 每一批桥接发现、握手和发送必须由一个外层 `tokio::time::timeout(Duration::from_secs(5), ...)` 统一约束；不能只给单次管道读写设置 5 秒，否则多个管道与线程身份组合会把总等待放大到数十秒；
- 发送在额度读取互斥区之外异步执行；
- 同一时间只允许一个有界发送任务；
- 新刷新遇到发送任务仍在进行时应合并，而不是并发创建重复发送；
- 超时、拒绝、桥接消失或未知结果均视为未确认失败，允许在下一次正常刷新中重试；
- 任何桥接故障都不得阻塞配额轮询、Burn Rate 更新、设置保存、托盘刷新或应用退出。

调度器采用“一个运行任务 + 一个最新待处理快照”的模型：

```text
schedule(fresh sample)
  ├─ 没有任务运行：标记 running，启动 detached worker
  └─ 已有任务运行：用最新样本覆盖 queued，不再 spawn

worker 完成
  ├─ queued 存在：取走 queued，在同一 worker 中继续下一批
  └─ queued 不存在：清除 running 并退出
```

待处理快照至少包括设置副本、UI 用量、账户标识和样本时间。只保留最新快照，避免额度快速刷新时积压旧消息。worker 在任何退出路径都必须释放 `pending` 预留并恢复可重试状态。

## 7. 确认与去重

`AutoPauseController` 至少维护 `sent`、`pending`、`running` 和 `queued`。额度去重基础键为：

```text
(account identity, quota kind, resetsAt, threadId)
```

如果客户端能够提供稳定的活动回合 ID，应将其加入活动回合键；若只有线程 ID，则采用保守策略：同一线程持续出现在活动列表中时视为同一连续活动期，只有实际观察到它离开活动列表后，才允许同一额度周期中的新活动期重新具备资格。若两个真实回合在两次观察之间完成切换而未被观察到空闲，宁可少发一次，也不得重复打断。

去重规则：

- 只有 `send_message_to_thread` 获得正向确认后才写入去重键；
- 拒绝、超时或结果未知不写入，因此可以重试；
- 账户或重置周期变化时清除过期键；
- 应用重启后内存去重自然清空，这是有意设计；
- 两种额度同时触发时每个会话只发送一条消息，并在确认后记录两个周期键；
- 一种额度已触发、另一种稍后在同一连续活动回合触发时，不重复发送；应在已有确认关系下补记第二种额度的键；
- 新出现的活动主会话在仍低于阈值时可以单独收到一次提示。

预留与确认流程固定为：

1. `reserve(account, target, triggers)` 为每个触发构造键；已在 `sent` 或 `pending` 中的键不再预留；
2. 若所有键都已覆盖，不调用发送工具；
3. 若至少一个键可发送，先把全部本次键加入 `pending`，再执行一次消息提交；
4. 正向确认时从 `pending` 移除并加入 `sent`；
5. 失败、取消、超时或 worker 异常退出时只从 `pending` 移除，不加入 `sent`；
6. 同时触发时一条消息代表两个额度键，确认结果必须原子地作用于两者。

集合必须有清理规则，不能仅依赖进程重启：

- 每次新鲜样本到达时，只保留当前账户和仍可能相关的重置周期；
- 观察到账户切换时，清空上一账户的 `sent`、`pending` 和活动期状态；
- 目标线程不再活动时，清理它的活动期辅助状态；
- 额外设置防御性上限，例如每个集合最多 512 项；超过上限时先移除最旧且不在 `pending` 的键；
- 不把这些键写入磁盘，也不复用现有重置通知状态文件。

## 8. 安全、隐私与日志

- 不接受来自 Overlay 设置或外部调用方的任意提示词；生产逻辑只生成本方案定义的固定消息；
- 不记录提示正文、线程标题、对话内容、账户标识或令牌；
- 只保存经过清理的本地状态，例如桥接可用性、确认数量和时间；
- 不启动或恢复独立会话，不创建竞争写入者，不杀死 Codex 进程；
- 命名管道只用于当前 Windows 用户的本地客户端控制面；
- 显式环境路径优先，但仍必须完成协议握手，不因环境变量存在而跳过验证。

## 9. 详细实现设计与代码落点

### 9.1 新增后端模块 `src-tauri/src/pause_control.rs`

该模块独立于 `codex/client.rs` 和 `notify.rs`，集中保存以下内容：

```rust
const PAUSE_MESSAGE: &str = "暂停";
const APP_TOOLS_PIPE_PREFIX: &str = "codex-app-tools-";
const LEGACY_BROWSER_USE_PIPE_PREFIX: &str = "codex-browser-use-";
const DELIVERY_DEADLINE: Duration = Duration::from_secs(5);

enum QuotaKind { FiveHour, Weekly }
struct DeliveryKey { account: String, quota: QuotaKind, resets_at: Option<i64>, thread_id: String }
struct ActiveThread { id: String, host_id: Option<String>, turn_id: Option<String> }
struct PendingSample { settings: OverlaySettings, usage: CodexUsage, account_id: String }
struct AutoPauseController { sent: HashSet<DeliveryKey>, pending: HashSet<DeliveryKey>, running: bool, queued: Option<PendingSample> }
```

建议函数边界：

- `triggered_quotas(settings, usage) -> Vec<Trigger>`：只做开关、百分比和重置周期判定；
- `pause_message(triggers, observed_at) -> String`：实现消息矩阵和分钟向上取整；
- `schedule(state, settings, usage, account_id)`：账户身份非空时同步完成快速资格判断和任务合并，绝不等待管道；
- `run_delivery_worker(state, first_sample)`：消费首个及最新排队快照，外层应用 5 秒总截止时间；
- `deliver_batch(state, sample)`：发现桥接、列出活动线程、预留键、逐目标发送并完成确认；
- `parse_active_threads(text)`：解析、过滤并去重目标；
- `ordered_candidate_pipe_paths(explicit, names)`：纯函数生成确定性候选；
- `candidate_caller_thread_ids()`：只生成本地调用方身份候选；
- `looks_like_thread_id(value)`：严格检查 8-4-4-4-12 UUID 形状。

模块在非 Windows 平台保留可编译的失败实现，返回“桥接仅适用于 Windows”，但纯函数测试应跨平台运行；只有命名管道枚举与连接测试使用 `#[cfg(windows)]`。

### 9.2 后端设置 `src-tauri/src/config.rs`

在 `OverlaySettings` 的 `refresh_interval_seconds` 后增加：

```rust
#[serde(default)]
pub auto_pause_on_low_quota: bool,
#[serde(default = "default_auto_pause_five_hour_threshold")]
pub auto_pause_five_hour_threshold_percent: u32,
#[serde(default = "default_auto_pause_weekly_threshold")]
pub auto_pause_weekly_threshold_percent: u32,
```

默认函数分别返回 `5` 和 `2`，`Default` 中总开关必须为 `false`。由于目标项目直接以 Serde 默认值加载旧 JSON，缺失字段会安全补齐，不需要改文件格式或写迁移脚本。配置测试必须同时覆盖完全缺失字段、部分缺失字段和显式已有值，确保升级不会静默开启。

### 9.3 设置补丁 `src-tauri/src/commands.rs`

在 `validate_settings_patch` 的闭合白名单中：

- 把 `autoPauseOnLowQuota` 加入 boolean 分支；
- `autoPauseFiveHourThresholdPercent` 只接受 JSON 无符号整数 1–15；
- `autoPauseWeeklyThresholdPercent` 只接受 JSON 无符号整数 1–10；
- 小数、字符串、`null`、0、越界值和未知字段必须使整个补丁失败。

继续使用现有 `merge_settings_patch -> save_settings_checked -> revision/event` 链路，不增加单独 command。补丁落盘再重载测试要断言三个值真实写入 `OverlaySettings`，避免只通过白名单但被 Serde 静默丢弃。

### 9.4 `AppState` 与初始化

在 `commands.rs::AppState` 增加一个只保护短时内存状态交换的标准互斥量：

```rust
pub auto_pause: StdMutex<crate::pause_control::AutoPauseController>,
```

这里明确使用现有别名 `std::sync::Mutex as StdMutex`，不使用 `tokio::sync::Mutex`。`schedule` 只在锁内更新 `running`、`queued`、`sent` 和 `pending` 等轻量状态，绝不执行管道连接、等待或文件 I/O；异步 worker 取出所需快照后立即释放锁。这样同步调度入口可以原子完成“检查并标记 running”，不会因先 spawn 后加异步锁而创建重复 worker。

并在两个构造位置同步初始化：

1. `commands.rs::AppState::for_test`；
2. `lib.rs::run` 中的生产 `AppState`。

`lib.rs` 顶部增加 `mod pause_control;`。漏掉任一初始化点都应导致编译或单元测试失败。该模块使用现有 Tokio、Serde、dirs 依赖，不修改 Cargo 依赖，也不触碰应用标识、更新端点、安装包身份或发布配置。

### 9.5 新鲜样本接线 `commands.rs::refresh_usage`

目标项目当前成功路径为：读取 `RawCodexUsage` → Burn Rate → `raw.to_ui(...)` → `notify::process_usage_success` → 返回。因此按以下顺序接线：

```rust
let account_id = raw.account_id.clone();
let usage = raw.to_ui(...);
crate::notify::process_usage_success(sink, state, &usage).await;
let settings = state.settings.lock().await.clone();
crate::pause_control::schedule(state.clone(), settings, usage.clone(), account_id);
Ok(usage)
```

关键约束：

- 必须在 `raw.to_ui` 前复制 `account_id`，因为 `CodexUsage` 不携带账户标识；
- `account_id` 缺失或为空时，`schedule` 立即返回，不发现桥接、不发送消息；
- 只接在 `Ok(raw)` 分支，不接错误、缓存返回或并发刷新等待者路径；
- 调度发生在现有成功发布完成之后，保证使用的是已接受样本；
- `schedule` 本身不得 `await` 管道，不得把发送放进 `state.client` 或 Burn Rate 锁；
- 不修改 `fetch_usage_background`、托盘监听器或 React 刷新逻辑，因为它们已经汇入统一入口。

### 9.6 前端类型与默认值 `src/types.ts`

在 `OverlaySettings` 和 `DEFAULT_SETTINGS` 同时增加：

```ts
autoPauseOnLowQuota: false,
autoPauseFiveHourThresholdPercent: 5,
autoPauseWeeklyThresholdPercent: 2,
```

字段必须为必填，避免前端把迁移后的真实后端设置与本地 fallback 混淆。所有测试构造设置时继续从 `DEFAULT_SETTINGS` 展开，不手工复制整套对象。

### 9.7 设置界面 `src/components/SettingsView.tsx`

在右栏“刷新间隔”之后、“开机启动”之前插入一个独立卡片：

- 顶部为总开关 `autoPauseOnLowQuota`；
- 5 小时 range：`min=1`、`max=15`、`step=1`，实时显示百分比；
- Weekly range：`min=1`、`max=10`、`step=1`，实时显示百分比；
- 总开关关闭时两个 range 同时 `disabled`，容器降低透明度并使用禁用光标；
- 下方说明精确描述 5 小时定时恢复、Weekly 仅暂停和同时触发优先级；
- 阈值滑块沿用 `onPatchSettings(..., true)` 防抖，总开关使用即时保存。

目标设置窗口当前为两列布局且每列独立滚动，因此不应通过压缩其他控件或增大固定窗口来“腾位置”；需在 960×620 与最小 760×500 两种尺寸、三种语言下检查可滚动与焦点可达性。

### 9.8 三语言资源 `src/i18n.ts`

增加四个键：

- `autoPauseOnLowQuota`；
- `autoPauseFiveHourThreshold`；
- `autoPauseWeeklyThreshold`；
- `autoPauseHint`。

英文、简体中文和繁体中文必须表达相同语义。UI 翻译只用于设置说明；发送给 Codex 的两种消息由 Rust 固定生成，不得从翻译资源读取，以免语言切换改变协议行为。

## 10. 自动化测试

### 10.1 设置与迁移

- 旧配置迁移后：开关关闭、5 小时为 5%、Weekly 为 2%；
- 开关仅接受 boolean；
- 5 小时阈值仅接受整数 1–15；
- Weekly 阈值仅接受整数 1–10；
- 拒绝 0、越界值、小数、字符串、null 和未知字段；
- 不含新字段的旧 `settings.json` 加载后不能开启功能；显式保存的新值必须能重新加载。

### 10.2 触发与消息

- 开关关闭时永不调用桥接；
- `account_id` 缺失或为空时永不调用桥接；
- 默认阈值下，5 小时 5% 触发而 6% 不触发，Weekly 2% 触发而 3% 不触发；
- 两个阈值互不影响；
- 仅 5 小时有效重置时间生成定时继续消息，并验证分钟向上取整；
- Weekly 仅发送 `暂停`；
- 无效 5 小时重置时间回退 `暂停`；
- 同时触发时 Weekly 优先且只发送一条；
- 多个活动主会话各收到一次，非活动、非 Codex 或子代理记录不发送。

### 10.3 去重、并发和失败

- 正向确认写入去重，拒绝与超时不写入；
- 重复刷新不重复发送；
- 账户/重置周期变化清除旧键；
- 并发刷新与在途发送不能制造重复消息；
- 在途时连续提交多个样本只保留最后一个排队样本；worker 退出后 `running` 与 `pending` 必须复位；
- 5 小时先确认、Weekly 后触发且线程仍处于同一连续活动期时，不发送第二条；
- `sent`/`pending` 达到上限时按规则清理，不能删除仍在途的键；
- 最新一次发送前观察已变空闲时跳过且不主动创建线程或启动闲置会话；
- 桥接不可用时正常额度功能仍可工作。

### 10.4 客户端兼容

- 显式环境路径始终第一；
- `codex-app-tools-*` 排在 `codex-browser-use-*` 之前；
- 旧前缀仍被接受；
- 枚举顺序变化不影响最终顺序；
- 大小写不同的同一路径只探测一次；
- 裸前缀、非法 UUID 后缀和无关管道被拒绝；
- 当前候选握手失败时可以回退旧候选；
- 只有确认成功的 `list_threads` 候选才用于发送；
- 4 字节帧长度、8 MiB 响应上限、JSON-RPC error、缺少 result 和畸形 JSON 都有失败用例；
- `pinnedThreads`/`threads` 中重复目标只发送一次，`hostId` 只在存在时传入；
- 候选管道或调用方身份数量增多时，整批仍在 5 秒总截止时间内结束。

### 10.5 前端

- 三个控件正确显示、保存、广播和恢复；
- 总开关关闭时两个阈值禁用；
- 三种语言在 960×620 和 760×500 下可访问，不遮挡既有设置；
- 状态文本不会在缺少确认时宣称成功。

### 10.6 测试文件与接线证明

- `src-tauri/src/pause_control.rs` 内部单元测试：消息、阈值、UUID、候选顺序、解析、去重、清理和任务合并；
- `src-tauri/src/commands.rs` 测试：补丁白名单、边界值、落盘重载，以及“失败/缓存路径不调度、成功路径只调度一次”；
- `src-tauri/src/config.rs` 测试：旧 JSON 缺字段时的默认值与显式值保留；
- `src/components/SettingsView.test.tsx`：开关、两个 slider、范围、禁用态和三语言可访问名称；
- 新增 Windows 协议模拟测试：建立临时命名管道，验证真实帧读写、`list_threads` 握手、消息参数、确认和超时；
- 接线测试应通过可注入的 `PauseMessageTarget` 或等价 trait 观察调用，不能只测试纯函数而不证明 `refresh_usage` 生产路径确实接上。

至少执行以下突变检查：删除 `schedule` 调用、把 `<=` 改成 `<`、移除 Weekly 优先、确认前写入 `sent`、删除当前管道前缀、移除活动状态过滤、取消总超时；每项都必须使对应测试变红。

## 11. 原生与打包验收

使用可丢弃测试会话，不使用重要工作会话。

1. 在当前 Codex 客户端上确认 `codex-app-tools-*` 握手成功并能看到活动会话；
2. 在保留旧桥接协议的客户端或可控模拟端确认 `codex-browser-use-*` 回退成功；
3. 分别输入高于 5 小时和 Weekly 阈值 1 个百分点的最新样本，确认无消息；
4. 单独达到 5 小时阈值，确认 transcript 中恰好出现一条含正确剩余时间和 `+2min` 的用户消息；
5. 单独达到 Weekly 阈值，确认恰好出现一条 `暂停`；
6. 同时达到两个阈值，确认只出现一条 `暂停`；
7. 重复刷新，确认没有重复；
8. 新建第二个可丢弃活动主会话，确认只给新目标补发；
9. 关闭开关，确认不再尝试发送；
10. 让桥接停止、超时或在提交前使会话空闲，确认不创建新回合、不虚报成功且 Overlay 其他功能正常；
11. 在一次性定时任务触发后，确认它在重置时间后 2 分钟发送且只发送 `继续`；
12. 对正式打包版本重复核心用例，并执行前端测试/构建、完整锁定 Rust 测试、打包回归、签名、安装包身份和更新清单验证。

只读 `list_threads` 握手能证明发现和可观察性，不能替代真实 `send_message_to_thread`、定时任务创建及 `继续` 回发的端到端验收。未执行的项目必须明确标为未验证。

## 12. 实施顺序

### 阶段一：建立测试基线与接口边界

1. 记录目标项目现有前端、Rust、构建和打包检查结果；
2. 为桥接发现、消息选择、去重状态和调度器定义窄接口；
3. 用纯函数测试锁定阈值、消息文本、时间取整和 Weekly 优先规则；
4. 在此阶段不连接真实会话，不改变发布配置。

本阶段同时把当前 `1.3.0` 记录为发布基线，并将实现、测试、文档和最终构建统一指向 `1.3.1`。

### 阶段二：设置与决策逻辑

1. 增加三个默认关闭的设置并完成严格校验；
2. 实现新鲜样本资格判断和消息决策矩阵；
3. 实现内存去重状态机、在途任务合并和周期清理；
4. 完成设置页、三语言文案和前后端测试。

### 阶段三：客户端桥接兼容

1. 实现显式路径、当前管道、旧管道的有序发现；
2. 实现 UUID 后缀校验、大小写无关去重和稳定排序；
3. 以 `list_threads` 确认候选可用性并只返回活动 Codex 主会话；
4. 实现 `send_message_to_thread`、确认结果解析、5 秒超时和候选回退；
5. 先使用协议模拟器完成自动化测试，再进行只读真实客户端握手。

### 阶段四：调度接入与真实验收

1. 只在新鲜成功额度样本的统一发布点调用调度器；
2. 在可丢弃活动会话中验证 5 小时、Weekly、同时触发和失败重试；
3. 验证一次性任务在重置后 2 分钟只发送 `继续`；
4. 验证桥接故障不会影响额度读取、界面、托盘或退出；
5. 记录尚未实测的客户端版本和边界，不以单次握手代替端到端结论。

### 阶段五：发布准备

1. 审阅完整 diff，确认未改变既有应用标识、更新端点、签名身份和发布通道；
2. 统一项目各处版本号并运行全部发布门禁；
3. 构建并验证正式安装包、便携包、更新清单、签名和 SHA-256；
4. 在正式安装包上重复核心真实客户端用例；
5. 更新 README、变更日志、状态文档和交接记录后再发布。

## 13. 发布与回滚要求

- 每次改变触发、消息、桥接协议或去重语义时都应增加对应回归测试；
- 正式版本必须统一 npm、前端、Cargo、Tauri 和锁文件版本；
- 标签源码 CI 与签名发布工作流必须通过；
- 项目正式支持的 NSIS、MSI、便携包、签名、校验和与更新清单必须齐全；
- 各发布资产的远端/本地大小、SHA-256、更新签名、版本身份和 Windows 升级身份必须验证；
- 旧的 `codex-browser-use-*` 回退只有在明确停止支持旧客户端并完成迁移公告后才可删除；
- 若新管道兼容路径出现回归，可关闭自动暂停开关作为立即止损，并回退到上一份已签名安装包；
- 发布动作不得自动安装、替换或重启用户当前运行的 Overlay，除非另有明确授权。

## 14. 完成标准

综合功能只有在以下条件全部满足时才可称为完成：

- 功能默认关闭，三个设置可正确保存、迁移和校验；
- 发布版本统一为 `1.3.1`，并保留 `1.3.0` 作为升级与回滚基线；
- 只使用新鲜成功样本并按各自阈值触发；
- 缺少明确账户身份的样本不触发自动暂停；
- 5 小时、Weekly、同时触发和无效时间四类消息选择完全符合决策矩阵；
- 一次性任务在正确时刻只发送 `继续`；
- 新旧桥接族均有明确发现顺序、握手门槛和回退行为；
- 只向活动 Codex 主会话发送，不为闲置会话开启新回合；
- 正向确认去重、失败重试、并发合并和周期清理均有效；
- 桥接故障不会阻塞 Overlay 核心功能；
- 自动化测试、真实客户端验收和正式包验收均通过；
- 文档清楚区分“兼容路径存在”“已完成只读握手”和“已完成端到端消息/定时继续验收”。

## 15. 实施前基线与交付物

截至 2026-10-18 对目标项目的只读检查中，现有源码未发现自动暂停、智能恢复、`list_threads`、`send_message_to_thread` 或上述两类桥接管道的实现。因此本文件定义的是待实施功能，不能据此宣称目标项目已经具备该能力。

实施开始前应再次检查最新代码，因为项目状态可能已变化。最终交付至少包含：

- 后端设置、决策、去重、桥接和调度实现；
- 前端设置控件、状态说明和三语言文案；
- 单元、集成、协议模拟和并发回归测试；
- 当前客户端只读握手记录与可丢弃会话端到端验收记录；
- 正式包构建、签名、哈希、安装/升级和更新清单验证记录；
- README、变更日志、状态与交接文档更新。

验收报告必须分别标注“代码路径已实现”“自动化测试已通过”“只读握手已通过”“真实消息发送已通过”“定时恢复已通过”和“正式安装包已通过”，不得把前一层证据外推为后一层结论。
