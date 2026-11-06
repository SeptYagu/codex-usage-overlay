# Codex 自定义暂停文案、定时继续与可选估算算法实施方案

- 日期：2026-11-06，America/Indianapolis。
- 状态：**需求与实施设计；尚未实现、发布或安装。**
- 代码基线：`v1.6.0`，提交 `ef98b6dc0faf9fc067f62759574badbbce93439d`。
- 交付目标：在现有多来源监控架构中，为本机 Codex 增加两种额度的自定义暂停文案、可主动采用的重置后定时继续预设，以及经典／加权／个性化三种 5H 消耗速率估算选项。
- 实施前若主分支有新提交，应重新核对本文件列出的入口、锁顺序和协议；本文中的新增类型、函数与文件均为设计，不代表基线已存在。

## 1. 产品行为与范围

| 功能 | 需求及验收口径 |
| --- | --- |
| 自定义暂停文案 | Codex 的 5H、Weekly 各有独立多行编辑框、保存校验与恢复默认按钮；两者同时触发时 Weekly 优先，仅投递一条消息 |
| 定时继续 | 浮窗自动计算服务器重置时间＋120 秒，发送一个含日期、时分秒和时区的指定时间；目标聊天按此时间创建、核验和停用任务，不再计算延迟 |
| 可选算法 | Codex 的 5H 可选择经典、加权、个性化（实验性）；切换立即重投影缓存，Weekly 使用现有算法 |
| 多来源兼容 | Codex 在后台监控时同样执行已开启的保护；当前浮窗显示来源不决定功能是否运行。Claude、DSH 的文案、保护和算法行为保持原定义 |

自动暂停仍默认关闭，Codex 阈值仍为 5H 剩余 5%、Weekly 剩余 2%。新安装与升级默认使用纯暂停文案和经典算法。**升级不得把已经开启的暂停保护转换为自动创建定时任务。**

定时继续是编辑器中的明确预设操作，不新增一个与文案内容可能矛盾的“已启用自动恢复”布尔值。点击“使用重置后继续模板”会替换并保存 5H 文案；按钮旁说明会覆盖当前草稿。用户可自行修改、移除定时指令；“恢复默认”恢复纯暂停文案。

本次不更换用量读取器，不更换 App Tools 消息桥接，不新增第三方更新通道，不调整安装身份，不推广算法参数自动优化。仅编写本方案时，不发送暂停消息、不创建任务、不修改用户设置或运行实例。

## 2. 基线代码与设计落点

已核对基线源码，不应从历史单来源设计直接重写当前运行模型。

| 当前位置 | 当前职责 | 本次实施要求 |
| --- | --- | --- |
| `src-tauri/src/config.rs` | `OverlaySettings`、schema 2、`ProviderPreference`、`for_provider` 与来源持久化目录 | 添加三个 Codex 专属设置，逐字段容错；保留来源迁移和投影规则 |
| `src-tauri/src/monitor.rs` | 每来源独立 `AppState`、`Registry::synchronize`、`Snapshot`、后台轮询 | 新设置同步到 Codex runtime；文案变化撤销未发送批次；算法变化仅重投影 Codex 缓存 |
| `src-tauri/src/commands.rs` | patch 校验、`refresh_usage`、缓存命令、`UsageEnvelope` | 增加严格校验、候选快照与统一显示发布；保持一次 raw 只接受一次 |
| `src-tauri/src/pause_control.rs` | `triggered_quotas`、固定 `pause_message`、`deliver_batch`、去重及桥接 | 分离文案渲染；从整批提前生成改为逐目标临发送生成；加强未知回执处理 |
| `src-tauri/src/pause_control/status.rs`、`resume.rs` | 请求／中断能力与恢复计划 | 如实展示“请求已接收”；不以文案预设伪造中断、任务创建或恢复能力 |
| `src-tauri/src/codex/burn_rate.rs`、`burn_rate/weighted.rs`、`personal.rs`、`shadow.rs` | 经典估计、后台加权和因果学习、状态及 outbox | 以现有公共类型扩展候选与连续校准；保留历史证据和失败恢复链 |
| `src-tauri/src/notify.rs`、`tray.rs` | 成功读数、缓存、通知及托盘 | 拆分真实采样副作用和纯显示发布；切算法不能伪造读取成功或重置通知 |
| `src/types.ts`、`providerSettings.ts`、`App.tsx`、`useProviderMonitors.ts` | 设置 patch、多来源视图、事件和缓存恢复 | 新字段与 envelope 元数据、逐来源版本过滤，覆盖两条显示入口 |
| `src/components/SettingsView.tsx`、`OverlayView.tsx`、`src/i18n.ts` | 设置控件、显示和翻译 | Codex 专属编辑器、预设、算法选择与实际算法说明；三语言一致 |

可直接复用的现有能力包括：设置 patch 串行队列、`sourceKey/sourceGeneration` 隔离、App Tools 发现与目标路由、批次时限和轮转、加权估计、因果五分钟反馈、持久化 outbox，以及浏览器布局 QA。新增功能沿这些入口扩展，不另建一套全局轮询或控制器。

## 3. 配置契约与编辑器

### 3.1 新字段

三个字段放在 `OverlaySettings` 顶层，语义明确为本机 Codex 专属，不放进所有来源共用的 `ProviderPreference`。当前项目已采用该方式保存 Claude、DSH 专属配置。

| JSON 字段 | 类型 | 默认与作用域 |
| --- | --- | --- |
| `autoPauseFiveHourMessage` | string | 空间和长度校验后的 Codex 5H 纯暂停模板 |
| `autoPauseWeeklyMessage` | string | Codex Weekly 纯暂停模板 |
| `fiveHourBurnRateAlgorithm` | `legacy` / `weighted` / `personalized` | `legacy`；只影响 Codex 5H 显示 |

建议 Rust 新增 `FiveHourBurnRateAlgorithm` 枚举；前端增加同名联合类型。语言默认模板以 `pause_message_for_language` 对应的固定暂停文字初始化：首次加载缺失字段时，在现有语言设置解析完成后补齐并保存；之后切换界面语言不重写已保存文案。首次加载与“恢复默认”都调用同一模板生成入口。

配置读取与 patch 写入区分处理：旧 JSON 缺字段时补默认；单个新增字段类型错误、全空白、过长或算法名无效，仅修复该字段，保留其他偏好。实时 patch 对错误类型、非法枚举、拼错键名和非法文案整体拒绝，不部分落盘。沿用现有配置原子写入及失败回滚路径。

纯新增字段不必提升现有 settings schema 2；不得让 `migrate()` 重建 `providerPreferences` 或改变原有各来源保护开关。`for_provider` 可以携带这些字段，但执行入口必须显式检查 `Provider::Codex`，不能仅依赖 UI 隐藏。

### 3.2 编辑器交互

新增 `src/components/PauseMessageInput.tsx`，维护本地草稿；失焦后才进入现有设置队列。每个框包含关联 label、提示、错误信息和“恢复默认”。恢复按钮的鼠标按下应防止先触发失焦保存半成品，再保存默认值；键盘操作同样可用。

文案不得全部为空白，最多 **2,000 个 UTF-16 单元**。前端 `maxLength/string.length` 与 Rust `encode_utf16().count()` 对齐，覆盖 emoji 等代理对。保存原文本，不擅自 trim 用户的换行和缩进。非法输入保留草稿并报错；保存失败恢复已保存值，不留下看似已生效的编辑状态。

在“来源设置”选中 Codex 时展示编辑器，与“浮窗显示来源”无关。保护关闭或 Codex 监控关闭时，输入及预设操作禁用并保留值。算法控件可以独立保存；隐藏 5H 或消耗速率时不禁用算法选择。

## 4. 文案渲染、调度和投递

### 4.1 新的纯文案模块

新增 `src-tauri/src/pause_message.rs`，负责默认模板、预设、校验和纯渲染，不访问网络、文件、控制器或定时服务。

```rust
pub const MAX_MESSAGE_LENGTH: usize = 2000;
pub fn valid_message(message: &str) -> bool;
pub fn render_five_hour(
    template: &str,
    reset_at: Option<i64>,
    now: i64,
    time_zone: &TimeZoneContext,
) -> String;
```

渲染规则：

1. Weekly 触发时发送 Weekly 文案；不替换其占位符，不拼接 5H 文案。
2. 只有 5H 触发时使用 5H 模板；没有 `{resume_at}` 就原样发送固定文字。
3. 存在变量时，浮窗从该次有效额度样本的服务器重置时间计算 `resume_at = reset_at.checked_add(120)`。要求重置时间仍晚于实际发送时刻，目标时间也仍在未来；不按发送时刻重新累加剩余分钟，不做整分钟上浮。将目标绝对时刻转换为用户本地日期和时分秒，附目标时刻的 UTC 偏移及明确时区标识，替换所有 `{resume_at}`。例如：`2026-11-06 18:30:00 -05:00（America/Indianapolis）`。
4. `TimeZoneContext` 为调用方传入的显式时区上下文，默认取发送时 Windows 当前系统时区；纯渲染函数不自行读取系统时区。同批次使用一致时区快照；转换按目标时刻的规则计算偏移，不能复用当前偏移跨越夏令时。重置时间缺失、过期、加法溢出、时区无法确定或转换失败时回退到固定本地化纯暂停文字，不发送错误时间或残留变量。
5. 不做任意表达式解释、模板代码执行或自动扩展其他变量。

`deliver_batch` 目前在发现目标前统一生成固定 prompt。实施时移除这一位置，改为每个目标发送前复核来源、账号、最新成功样本、settings revision、保护开关与阈值，再渲染最终文案。同一重置时间和时区应始终得到同一个指定时间，不因目标数量或发送耗时漂移；发送时刻仅用于有效性校验。不能只修改旧 `pause_message()` 的返回值而继续使用批次启动时的有效性判断。

`Registry::synchronize` 应把两个文案字段变化加入 Codex 的取消条件与紧急唤醒条件。编辑文案使未发送工作失效；等待下一份有效成功样本按新文案排队。对等价新低额度样本，已经写出的旧请求若迟到成功，仍要登记其原始周期去重，不能因 settings revision 变化而丢掉回执。

### 4.2 复用桥接与防止重复

保留 App Tools 命名管道、`list_threads`、`send_message_to_thread`、目标 thread 路由、host 归属、五秒整批预算、两秒单调用上限、目标轮转及三十秒动作新鲜度。显示缓存和算法重投影不能成为发送依据。

基线需要一并修正的两点：

- 当前 ledger 的 `thread_id` 实际使用活动期 ID。增加显式真实 `threadId` 与活动期字段，按账号／真实聊天／额度种类／重置周期维护已确认集合；同周期成功后，切换文案、切换算法、切换浮窗来源、聊天产生新回合均不能再次发送。两项额度同时触发只发送 Weekly，并登记本次已覆盖的两个周期；连续活动中的后续跨阈值合并保持明确测试。
- 当前 `send_pause(...).await.unwrap_or(false)` 会把超时、拒绝和写出后断连混在一起，`ReservationGuard` 又会释放所有失败预留。新增结构化结果：`Accepted`、`Rejected`、`UnavailableBeforeWrite`、`CancelledBeforeWrite`、`SubmissionUnknown`。只有确认未写出的失败可重新排队；部分写出、写出后断连／超时或批次取消进入未知隔离，不能自动换请求 ID 重发。写出状态必须在可取消 future 外保留。

去重范围按现有产品约定保持进程内。重启清空 ledger，外部任务却可能仍存在，因此不能承诺跨重启恰好一次，也不能自动取消未知的旧任务；定时预设要求目标聊天先核对本聊天已有的相应用途任务，存在时核验并复用，不再创建。若未来要求浮窗跨重启可靠防重，需要另行设计持久化操作账本和可验证任务读回，不能把它描述成本次已具备的能力。

当前列表请求 `limit=50` 和调用方候选最多 32 个文件仍是发现边界。列表之后聊天转空闲的竞态也没有原子目标回合保证。本次不能写成已覆盖全部运行会话、绝不唤醒空闲会话，或已验证模型停止；真实投递验收应单列这些限制。

## 5. 重置后定时继续预设

### 5.1 产品语义

预设只适用于 Codex 5H 单独触发。它是一段可查看、可修改的发送内容，通过现有桥接投递给聊天；浮窗不直接调用定时工具，不写定时数据库，不在后台自行创建“继续”任务。

预设应用后仍使用原自动暂停开关。Weekly 优先，Weekly 模板默认纯暂停；即便 5H 已重置，只要 Weekly 同时触发就不通过默认路径请求定时继续。用户自行写入其他指令时按自定义文本处理，不宣称产品能识别任意自然语言的全部意图。

现有 `autoResumeAfterQuotaReset` 与 `resume.rs` 是基于能力验证的恢复计划路径；它们的能力门槛继续保留，不能借本预设将 `directInterrupt`、`completionObservation` 或 `continuationControl` 标成可用。设置说明应明确区分“请求聊天安排继续”和“浮窗已确认可恢复”。

### 5.2 预设内容规范

提供简中、英文、繁中语义一致的完整模板，以下为简中候选正文。代码中的默认与前端预览应来自同一权威模板或有全文一致性校验，避免两个副本逐渐不同。

```text
暂停当前工作。请在当前聊天安排一次性继续唤醒，指定时间为 {resume_at}。直接使用此时间，不再加任何延迟。

先检查本聊天是否已有同一用途、同一目标时间的继续任务；有则核验并复用，不重复创建。
请使用当前可用的 automation_update 工具，创建 heartbeat 类型、目标为当前 thread 的任务，名称为“继续”，状态为 ACTIVE。

保存的任务提示词必须包含：
“这是一次性继续唤醒。先从心跳上下文取得本任务 automation_id，读取并暂停该任务，保留其他配置；确认已停用且没有下次执行时间后，再继续原工作。停用失败则报告错误并停止，不重新创建任务。”

核对指定时间仍在未来，并确认调度器时区能够准确表示该绝对时刻。按当前工具 schema 为此时间生成单次时间规则；若使用每日频率与单次计数，必须指定相应的本地小时、分钟和秒，直接创建不传 DTSTART。不得重新计算剩余时长、再次加两分钟或自行顺延到次日。
创建后保存工具返回的真实任务 ID，并核对目标聊天、日期、时区和未来执行时间。无法核对、工具不可用或创建失败时，报告具体原因并停止，不反复创建。
成功后简短确认并结束当前回复，等待唤醒。
```

`automation_update` 为能力名，命名空间前缀以接收端实际可用工具为准；集成测试需核对当前 schema，不把某个客户端版本的工具参数当成永久协议。提示中不加入目标生命周期命令，不要求创建独立周期任务代替当前聊天跟进。

### 5.3 时间与任务生命周期

- 浮窗以服务器重置时间精确加 120 秒生成唯一目标时刻，不把“剩余时间＋缓冲”的算术交给聊天。实际消息处理与任务调度仍可能延迟；不能承诺准点送达。若目标到达接收端时已经过期，报告并停止，不从接收时间重新加两分钟，也不自动改排次日。
- 单纯 `COUNT=1` 不足以作为一次性停止保证。**自停用要求必须写进保存的任务 prompt**，不能只写在创建任务的当前对话中；停用失败不得继续工作。
- 5H 预设只按短于 24 小时的目标使用。跨午夜时浮窗直接提供正确日期；跨夏令时时按目标时刻给出正确偏移。聊天需核对调度器时区与指定时刻等价；偏移与时区不一致、时钟重叠无法表达、时区改变、长于一天、休眠与重启均不能仅凭“每日某钟点”推断正确，无法确认时停止创建并提示。
- 暂停消息回执只证明发送工具接收，不能证明任务已创建、任务会准时唤醒或推理已停止。状态文案只能显示“已发送定时继续请求”，除非后续有真实、可归属的结果读回。
- 更改文案、关闭保护或退出浮窗不会取消已由聊天创建的外部任务。界面应提供简短说明；任务的停用／删除由相应聊天或任务管理入口完成。
- 本次方案不创建任何验收任务。真正实施时，只有获授权的隔离接收聊天才执行端到端定时测试，测试后按真实任务 ID 停用或删除。

## 6. 可选 5H 消耗速率估算

### 6.1 选择、候选与回退

设置仅对 Codex 5H 生效，默认 `legacy`。三种候选使用同一真实 raw、同一 fetchedAt、同一 tracker 接受结果，候选选择不参与暂停阈值判断。

| 选择 | 展示候选 | 不可用时 |
| --- | --- | --- |
| `legacy` | 现有经典值 | `—` |
| `weighted` | 固定加权值 | 经典 → `—` |
| `personalized` | 连续仿射校准值 | 加权 → 经典 → `—` |

有效候选须存在、有限且非负；0 是有效值。`requestedAlgorithm` 和 `effectiveAlgorithm` 分开，回退不修改用户选择，恢复有效数据后自动恢复。无有效候选时 effective 为 null。

加权复用现有生产辅助函数：半衰期 300 秒、局部窗口及硬证据范围 1,800 秒、最小跨越 2 个百分点，保留已有 idle ceiling 处理。不要只复制回归公式而丢掉闲置和历史有效性规则。

个性化使用连续比例与偏差校准，放入新增 `burn_rate/affine.rs`，接入现有 `personal::MaturedOutcome` 和 `shadow` 的原子提交链。现有乘法模型继续作为历史审计基线，不能将其状态直接解释成新模型。

### 6.2 连续校准的冻结数学契约

基础速率单位为额度百分点／小时。令 `x = weightedRate / 12`，即基础五分钟预计消耗；成熟反馈 `y = observedDelta * 300 / elapsedSeconds`。维护五个带 0.99 衰减的矩：`S0、Sx、Sy、Sxx、Sxy`。

```text
A = Sxx + 10
B = S0 + 10
D = A * B - Sx * Sx
slope  = clamp(((Sxy + 10) * B - Sy * Sx) / D, 0.5, 1.5)
offset = clamp((A * Sy - Sx * (Sxy + 10)) / D, -1.5, 1.5)
trust  = S0 / (S0 + 8)
fiveMinute = clamp(x + trust * (slope * x + offset - x), 0.75*x, 1.5*x)
ratePerHour = fiveMinute * 12
```

无有效证据时恰好等于加权值。非有限基础值、无效行列式或损坏模型返回不可用，由上述回退规则处理。输出连续、非负、随基础值非递减；参数固定，不向用户开放自定义。

每个合格成熟反馈先衰减五个矩；仅在 `q > 0` 且基础区间预测没有因剩余额度被截断时增加 `1、x、y、x*x、x*y`。信息不足或截断反馈只推进衰减／反馈记账，不拟合截断造成的假偏差。未来增量展示最后才受剩余额度限制。

沿用 300–390 秒成熟窗口、最大 90 秒连续有效观察间隔，以及缺失、修正、重置、饱和和账号切换处理。120／300 秒轮询可能暂停新学习，但已有模型仍可提供有效估计；不为学习强制改变用户刷新间隔。30／60 秒也需按实际采样间隔判断，不能只看设置值。

保留经典默认与“个性化（实验性）”标签。验收关注实现与数学一致性，不以某一历史回测的优势宣传必然更准。

## 7. 候选缓存与多来源发布契约

### 7.1 数据结构

新增 `src-tauri/src/presentation.rs`，包含 `BurnRateCandidates`、`UsageCandidateSnapshot`、纯 `project()` 和同步发布抽象。候选快照是运行时缓存，不是另一份学习历史。

沿用现有 `UsageEnvelope` 的**扁平** JSON 结构和 `sourceKey/sourceGeneration`，扩展可空元数据，而不是把 `usage` 突然改成嵌套对象：

| 字段 | 含义 |
| --- | --- |
| `presentationRevision` | 同来源代次内递增的显示序号，采样发布和纯重投影都递增 |
| `settingsRevision` | 本次实际采用的已提交设置版本；与根设置广播版本关联，不能冒用乐观 UI 版本 |
| `requestedAlgorithm`、`effectiveAlgorithm` | Codex 的保存选择及实际候选；其他来源为 null |
| `fallbackReason` | 稳定可翻译原因码，无回退为 null |
| `personalLearningStatus` | 积累中、已运行、观察间隔不足、观察缺失或持久化暂停 |

`MonitorSnapshot.revision` 保持监控快照排序用途，不能与内层 `presentationRevision` 混用。`Registry::synchronize` 需把根设置已提交版本传到 runtime，并在发布时核对来源代次；若采用独立 runtime 版本，必须另命名和明确映射，不混称根设置版本。

覆盖所有入口：`refresh_usage`、刷新合并结果、`last_refresh_result`、`fetch_usage`、`get_last_usage`、`usage_updated`、`Registry::snapshots/monitor_updated`、托盘和前端缓存恢复。不得让新增快照与原 `last_usage` 分别成为两个可以独立写入的权威缓存；应统一缓存结构，并只提供一致的派生视图。

### 7.2 真实采样与纯重投影

真实采样路径：读取 raw → 复核 source generation → flush 旧 outbox → 接受一次 raw → 保存／flush／保存清空状态 → 读取最终候选及学习状态 → 投影发布 → 按原流程处理真实额度、通知和保护。

算法设置保存后：从 Codex runtime 最近成功候选快照纯投影 → 增加显示版本 → 发布相同 fetchedAt 的新结果。无快照时只保存设置，等待下一次成功采样。

纯重投影不得请求额度、调用 `accept_usage`、重新按墙钟计算闲置、训练、取消 pending、写证据、触发重置通知、清除读数错误、重算动作新鲜度、安排暂停／恢复或唤醒轮询读取。失败后可投影上次成功读数，健康状态仍为失败。

后台 Codex 即使未显示也更新自己的候选快照。切换到 Claude、DSH 不丢弃 Codex 缓存；切回直接展示最新投影。非 Codex pipeline 不启用本次个性化模型、不写入 Codex 学习文件，Weekly 三种选择下均保持原值。

### 7.3 并发与前端排序

每 runtime 增加短生命周期 `presentation_guard`，序列化“读取已提交设置 → 投影 → 缓存／版本更新 → 同步发布”。保留现有 source commit 的 `settings_patch` 门控：先 runtime `settings_patch`，释放 tracker 锁之后再取 `presentation_guard`，内部顺序为 `settings → tray_read_status → presentation cache → tray_gate`；内部锁释放后同步 emit，异步 dock／通知在显示 guard 外执行。

根 patch 仍先完成配置落盘，再在根 settings 锁释放后调用 Registry 同步；不要持有根 settings 锁等待 runtime。不得持有显示 guard 等待 refresh guard、tracker、Registry snapshot gate 或网络。监控快照只能短时读取已提交缓存，不能在持 snapshot gate 时反向等待会持它的显示发布；必要时先复制不可变快照再 emit。

并发请求完成时必须使用**当前已保存算法**，不能使用请求开始时的旧选项。`App.tsx` 与 `useProviderMonitors.ts` 都按来源身份／代次及各自显示序号过滤；同 fetchedAt 的新投影必须可接受。晚到缓存、旧 refresh 返回和旧 monitor 事件不能覆盖新选择。每来源保留自己的版本水位，不能把一个来源的高序号用于拒绝另一个来源的新读数。

## 8. 模型持久化与证据兼容

在当前 burn-rate schema 2 的 Codex shadow 子树中新增有独立 `version=2` 的 `affine` 状态；保留原 quota crossings、乘法模型、pending、account generation、event sequence 和 outbox 身份。不能整体替换 `burn_rate.rs/personal.rs/shadow.rs`，否则会丢掉现有 busy-outbox、锁存拒绝原因及重启不重复审计等修复。

模型加载校验有限性、矩的合法范围、时间顺序、版本、反馈计数和矩的可行关系；错误只使新模型冷启动，不能清空有效额度历史。账号切换按现有隔离逻辑重置新模型；普通算法切换与浮窗来源切换不清空学习。

首次升级默认采用中性冷启动，不回放未经校验或可能截断的旧日志。后续真实成熟反馈逐步训练。若另行实现历史回放，必须验证当前账号代次、完整链、事件去重、outbox 和末状态一致性，再决定启用；不是本次交付前提。

正常重启从已保存 affine 状态恢复，不重复训练。审计 outbox 未提交或持久化失败时继续沿当前链暂停新学习；已有有效模型可以显示，状态如实标记暂停。

现有 `originPublishedRatePerHour/publishedPredictedDelta` 永远指经典基线，旧 personal 字段维持原历史模型含义。增加可空的 `originAffineRatePerHour/affinePredictedDelta`，冻结 origin 时刻的新预测；旧 pending 缺字段不能用 target 模型伪造过去预测。离线 validator 接受旧事件并校验新字段；显示选择绝不回写审计基线。

## 9. 文件级实施清单

| 文件 | 具体新增／调整 |
| --- | --- |
| `src-tauri/src/pause_message.rs`（新增） | 模板、UTF-16 校验、`{resume_at}` 绝对时间计算／格式化、显式时区上下文、定时预设及纯函数测试 |
| `src/components/PauseMessageInput.tsx`（新增） | 失焦保存、错误草稿、恢复默认、预设应用和可访问性 |
| `src-tauri/src/config.rs`、`src/types.ts` | 三个字段、枚举、逐字段恢复、默认与配置往返 |
| `src-tauri/src/commands.rs` | patch allowlist、模板初始化、快照和设置保存后重投影；迁移缓存／refresh 合并返回 |
| `src-tauri/src/monitor.rs`、`src/providerSettings.ts` | Codex 专属投影、文案取消、后台缓存、设置版本和 monitor 发布 |
| `src-tauri/src/pause_control.rs`、`pause_control/status.rs` | 逐目标渲染、真实会话周期去重、未知回执隔离、准确状态 |
| `src-tauri/src/codex/burn_rate/affine.rs`（新增） | 连续校准模型、状态校验、成熟反馈更新与数学边界 |
| `src-tauri/src/codex/burn_rate.rs`、`personal.rs`、`shadow.rs` | 候选读取、affine 子状态、origin 预测及 outbox 原子链接线 |
| `src-tauri/src/presentation.rs`（新增） | 纯 selector、候选快照、显示缓存、版本与发布抽象 |
| `src-tauri/src/notify.rs`、`tray.rs`、`lib.rs` | 拆分成功采样与纯显示、初始化新模块、全部缓存调用点 |
| `src/App.tsx`、`src/hooks/useProviderMonitors.ts`、`src/usageSource.ts` | 新显示元数据、逐来源排序、缓存恢复与来源隔离 |
| `src/components/SettingsView.tsx`、`OverlayView.tsx`、`src/i18n.ts` | Codex 控件、实际算法 tooltip、学习状态和三语言说明 |
| `scripts/check-replay-evidence.mjs` 及相关 burn-rate reader／validator | 保持旧基线、可空新字段和确定性新模型对照 |
| `scripts/display-controls-qa/`、相关 Rust／React 测试 | 更新 stub 契约、编辑器、算法、多来源和布局验证 |
| `README.md`、`README.en.md`、`CHANGELOG.md`、`STATUS.md` | 实现完成后再描述实际交付、使用方法与剩余边界 |

## 10. 实施阶段与交付门槛

| 阶段 | 内容 | 完成条件 |
| --- | --- | --- |
| 1：配置与文案编辑 | 三字段、模板模块、编辑器、三语言、旧配置兼容 | 真实 patch 入口、保存失败及 Unicode 边界通过；纯暂停默认不变 |
| 2：投递接线 | 浮窗计算唯一指定时间、逐目标有效性校验、同步取消、稳定周期去重、未知结果类型 | 生产调度夹具证明不重复、不发送过期文案，同一 reset 的目标时间不随发送耗时漂移；桥接协议及边界保留 |
| 3：定时预设 | 完整可见模板、任务复用提示、唤醒自停用与状态文案 | 离线模板契约通过；获授权的隔离聊天通过实际创建／唤醒／清理后才声明端到端可用 |
| 4：模型与状态 | 连续校准、候选、origin 预测、状态恢复与隔离 | 数学边界、因果性、异常状态、重启及 outbox 测试通过 |
| 5：显示选择与多来源 | 缓存投影、前端顺序、后台 Codex 和托盘 | 真实入口并发验证无重复训练、无旧显示覆盖、无跨来源污染 |
| 6：集成与发布准备 | 全量检查、浏览器及 Windows 实机、使用说明 | 下面矩阵完成，失败与未验证项如实登记；另行发布和安装 |

每阶段检查与审阅 diff 后仅提交相应改动并正常推送，不 force-push。若源码基线变化，重审接线后再继续。文档阶段本身不提升版本号、不生成安装包。

## 11. 验收矩阵

| 类别 | 必须覆盖的行为 |
| --- | --- |
| 配置 | 缺失、null、错误类型、空白、超长、未知算法只修复对应字段；非法混合 patch 整体拒绝；失败保存不提交设置／显示事件 |
| 文案 UI | 2,000／2,001 UTF-16 边界、emoji、多行、失焦、指针／键盘恢复、切来源草稿、跨窗口更新和保存回滚 |
| 时间渲染 | 无变量固定文字、重复 `{resume_at}`、reset 精确＋120 秒且保留秒数、跨日／月／年、时区及目标偏移、过期／缺失／checked 加法溢出、同一 reset 多目标／不同发送耗时得到相同指定时间 |
| 触发 | 5H 单独、Weekly 单独、同时 Weekly 优先；已关闭保护／监控或只读旧缓存不发；编辑期间取消未发送批次 |
| 去重 | 同周期新回合、反复开关、文案／算法改变、等价新低样本迟到回执不重复；写出后超时进未知，未写出失败才可重试 |
| 定时流程 | 正确接收聊天与任务 ID、按唯一指定时间创建且不二次加延迟、同日／跨午夜、接收时目标已过期则停止、工具不可用、创建成功但时间不明、未知回执、已有同用途任务、实际唤醒先停用、停用失败停止 |
| 时间边界 | 跨 DST 转换使用目标偏移、系统时区与调度器时区不同须核对等价、无法确定时区／表达歧义／目标超 24 小时拒绝误排；忙碌／休眠／应用关闭列为实机项目，不以离线测试代替 |
| 数学 | 无证据恰等于加权、0 有效、系数／输出界限、连续／单调、实际 elapsed 归一化、q=0／截断只衰减、负值／非有限保护 |
| 因果学习 | origin 冻结，未来 target 不参与旧预测；一个成熟反馈只更新一次，旧 pending 不补造 affine 预测 |
| 状态 | 缺／坏 affine 仅新模型冷启动、正常恢复、账号隔离、慢采样、忙 outbox、写入失败与重启不重复审计 |
| 纯显示 | 三选择／逐级回退／全部为空，Weekly 不变；切换不改变 fetchedAt、采样次数、tracker、pending、审计、通知、保护状态或失败健康状态 |
| 并发 | 读网期间切换、快照先完成再切换、快速连续保存、相同 fetchedAt、晚到 cache／command／monitor、source generation 改变，均保持最后成功设置 |
| 多来源 | 浮窗显示 Claude 时设置并运行 Codex；切回即时显示；Claude／DSH 原算法和保护不变；关闭一个来源不影响其他来源 |
| UI 与实机 | 三语言、浅／深色、960×620 与 760×500、DPI 100%／175%、键盘可达与长模板滚动，无横向裁切；Windows WebView2、托盘和多屏另测 |

并发测试用可控制屏障和有界超时；应通过真实 patch、refresh、Registry 和发布 sink 入口验证，不能仅测纯 selector 就宣称接线完成。对新关键接线做最少量反向检查：移除 patch allowlist、移除同步取消／重投影、错误传入所选显示值作为 published 基线时，相关行为测试必须失败。

基础复验命令（在仓库根目录；按影响范围分阶段执行）：

```powershell
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo check --manifest-path src-tauri/Cargo.toml --locked --all-targets
node scripts/check-replay-evidence.mjs
node scripts/display-controls-qa/check.cjs
node scripts/display-controls-qa/multi-provider.cjs
powershell -NoProfile -File scripts/Test-TauriRelease.ps1
```

新增 editor／算法 QA runner 在实施时加入同一套离线 harness。生产校准公式可用独立 JavaScript 对照及固定合成轨迹验证，不依赖任何未随项目分发的个人历史文件；实机或导出数据依赖的 ignored 用例需显式执行并记录，未执行不能计为通过。经典与加权旧 replay 证据应保持字节一致，新 affine 证据单列。

## 12. 完成定义与回退

完成须同时具备：可保存的两种文案、实际发送时正确渲染、可用且解释准确的定时预设、三算法与回退、无副作用的缓存切换、持久化和来源隔离、完整的设置／显示契约，以及对应自动化和实机验收记录。只增加输入框、只修改默认提示词或只开放下拉框均不能视为完成。

用户回退路径为：恢复两种纯暂停默认文案、选择经典算法；不会删除历史模型，也不会取消已经创建的外部任务。代码回退应保持新字段可被旧程序忽略、新模型为独立可空子树，并通过回退再升级测试确认 quota 历史仍可恢复；不假定旧程序能保留它不认识的字段。

实施后的交接记录必须逐项区分“源码完成”“自动检查通过”“真实投递通过”“定时实际唤醒通过”“发布”“本机安装”。当前文档仅完成方案，不构成这些后续步骤的执行结果。

## 13. 项目内参考

- [当前多来源设计](2026-11-02-multi-provider-monitoring-plan.md)与[实施记录](2026-11-03-multi-provider-implementation.md)：来源 runtime、后台生命周期和显示隔离。
- [智能暂停改进方案](2026-10-23-smart-pause-improvement-plan.md)与[实施记录](2026-10-23-smart-pause-improvements-implementation.md)：取消、动作新鲜度、能力与恢复边界。
- [读取可靠性记录](2026-10-23-quota-read-reliability-implementation.md)：读数诊断、读取合并和过期样本。
- [既有个性化后台设计](2026-10-03-5h-persistent-shadow-personalization-implementation-plan.md)、[实施记录](2026-10-20-5h-shadow-personalization-implementation.md)及[后续修复](2026-10-20-v1.3.3-shadow-and-smart-pause-remediation-implementation.md)：因果反馈、outbox 和证据基线。
- [显示 QA 使用说明](../../scripts/display-controls-qa/README.md)：离线渲染与实机检查的边界。
