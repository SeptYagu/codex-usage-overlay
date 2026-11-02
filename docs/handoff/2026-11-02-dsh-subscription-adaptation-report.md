# DSH Codex 订阅额度适配可行性

**结论：可以适配，但不能把 DSH 的读数直接接到现有 Codex 暂停桥。** 必须同时区分额度来源、订阅账户和被控制的 DSH 会话。

状态：仅完成 Git 更新和代码调查，**尚未实现或部署适配**。未修改已安装的 DSH/订阅插件，未发送真实“暂停/继续”，未读取登录令牌。本文区分用户所说的“发送暂停”与更强的“阻止后续模型请求”，不将二者视为同一能力。

## 1. 调查基线

- 工作仓库已从 `b792bdf`（v1.3.3）快进至 `b318d12`（v1.5.1），调查开始和结束时源代码工作树干净；本轮新增本文并更新交接索引。
- 本机安装的插件是 `dsh-codex-subscription` **3.2.3**，版本证据见 [插件清单](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/package.json#L1-L4>)。
- 依据是当前仓库及已安装插件的可读 JavaScript 源码，不是运行中的额度账户响应，也不代表其他插件版本的稳定 API 承诺。

## 2. 已核实的事实与影响

| 证据 | 发现 | 对适配的影响 |
| --- | --- | --- |
| [本机读取进程](../../src-tauri/src/codex/client.rs#L127-L173)、[额度 RPC](../../src-tauri/src/codex/client.rs#L488-L558) | 当前 Codex 来源是自己启动的本机 `codex app-server`，调用 `account/rateLimits/read` | 此路径不是 DSH 插件来源；不能以换标题或读取同一个本机 Codex 登录状态代替适配 |
| [插件额度读取](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L13760-L13837>) | 插件使用自己的可刷新 OAuth 生命周期，请求 `https://chatgpt.com/backend-api/wham/usage`；认证和账户头留在宿主侧 | 可以消费插件已经解析的额度，不必把 ChatGPT token 复制给悬浮窗，也不应重新实现 token 刷新 |
| [插件账户仓库](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L10794-L10808>)、[账户列表](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L10865-L10889>) | 插件维护独立账户记录；列表有账户 opaque ID、label 和 active 状态 | 使用插件账户 ID 绑定，不用 email、显示名称或本机 Codex 账号推断 |
| [插件额度接口](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L15629-L15677>)、[分账户读取](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L13902-L13929>) | `usage` 返回当前读数；`usage/accounts` 返回带账户 ID 的读数或错误；读数包含窗口时长、剩余比例、重置时间和 fetchedAt | 已有可消费的基础数据，但仍需给外部消费者增加原子的一致性和来源契约 |
| [本机暂停发送](../../src-tauri/src/pause_control.rs#L836-L862)、[目标筛选](../../src-tauri/src/pause_control.rs#L189-L228) | 暂停实际是通过本机 Codex 桌面命名管道给 active Codex 线程发“暂停”；筛选不校验订阅账号 | 直接套用会把 DSH 额度不足变成对本机 Codex 会话的误操作，DSH 本身却未被控制 |
| [当前能力声明](../../src-tauri/src/pause_control/status.rs#L17-L36) | 仅有 cooperative delivery；不具备确认直接中断、完成、后续用户活动或自动续发的能力 | “消息送达”不能显示成“已确认停止”；当前本机恢复路径也不能当成 DSH 自动恢复能力 |
| [配置枚举](../../src-tauri/src/config.rs#L25-L33) | Provider 仅有 Codex/Claude，未知持久化值通过 `serde(other)` 回落 Codex | 不能仅写入 `provider: dsh`；否则可能静默读成本机来源 |
| [刷新接收](../../src-tauri/src/commands.rs#L354-L429)、[来源切换](../../src-tauri/src/commands.rs#L998-L1012) | 切源不与整个读取过程串行，接收结果前没有来源 generation 校验 | 静态调用链显示：切源后，旧在途响应可能进入新源缓存/学习器。需在适配前补拒收旧响应的边界；本轮未动态复现 |

### 现有插件已经能做什么

插件有 **“额度重置后自动重试”** 设置，配置名 `autoQuotaRetry`，默认 `false`，见 [默认值](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L16863>) 和 [界面文案](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/client.js#L2144-L2145>)。

其逻辑挂在 `agent/request-error`：遇到指定 Codex 额度错误后，强制读取**插件**额度；确认 Codex 窗口已用达到 `99.9%`、窗口长度及等待时间都在六小时上限内、重置时间有效后，等待至重置加十秒并请求重试。等待期间可取消；换账户会唤醒，并等待模型目录就绪。见 [判断与等待逻辑](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L14418-L14626>)、[宿主事件注册](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L17534-L17558>)。

这可以满足一部分“触及短时额度限制后先等恢复”的需求，但：

- 是**请求失败后的等待重试**，不是轮询到低额度后主动给 DSH 会话发送“暂停”。
- 不等于在耗尽前拦截所有会话的下一次请求，也不是周额度或 workspace 限制的通用等待方案。
- 本轮未读取该设置的实际值、未替用户开启，也未触发真实限额验证。

## 3. 建议的适配边界

### 3.1 只消费 DSH 插件的无密钥快照

增加显式独立来源“本机 Codex / DSH Codex 订阅 / Claude”。DSH 模式不启动本机 `codex app-server`，插件离线、未登录或读取失败时显示对应来源的错误，**绝不 fallback 到本机 Codex**。

插件已有认证内的 POST 路由：

- `/api/codex-subscription/status`
- `/api/codex-subscription/usage`
- `/api/codex-subscription/usage/accounts`

路由要求 DSH 的认证及 RPC envelope，不是匿名 GET 接口，见 [传输注册](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L3310-L3334>)。本轮向运行中的 DSH 发过一次无凭据的只读 usage 请求，返回 `401 Unauthorized`；因此还不能宣称悬浮窗已能读取实际订阅额度。

推荐通过受认证的 DSH 插件桥发布最小快照，或为悬浮窗提供仅限当前用户访问的 IPC。只导出额度和标识，不导出 OAuth、Cookie、API 密钥或无关账户资料。现有 DSH 路由不能为方便接入而取消认证。

**以下是建议新增的数据契约，不是已经存在的响应格式：**

```text
SourceKey = (kind, dshInstanceId, pluginAccountId)
QuotaSnapshot = (SourceKey, generation, fetchedAt, codexWindows, readStatus)
ControlTarget = (SourceKey, sessionId, provider, bindingGeneration)
```

实例 ID 应稳定区分 DSH profile/实例，不能只用会变化的端口。不要仅凭“刚查过 status、随后查 usage”认定二者属于同一账户：切换可能发生在两次请求之间。快照应由桥在同一身份边界内生成，并校验读取完成后的账户/generation；分账户 API 的 active-reader 复用也需要覆盖切换竞态，不能天然假定原子一致。

### 3.2 发送暂停与请求拦截分两层

**与用户原话对应的最小版：发送协作式暂停。** 当新鲜插件读数满足条件时，只向显式绑定的 DSH 会话发送“暂停”，按来源、实例、账号、会话、额度周期去重。必须使用专用 DSH 控制适配器，禁止调用现有本机 Codex 管道。界面显示“暂停请求已送达”，不能声称已硬停止。

**如果要求确保后续请求不再发出：在 DSH 内增加发送前 gate。** 按即将执行的请求所用 provider 和实际订阅账户检查额度状态；仅限制相同身份的 Codex 订阅请求，不全局停止 DSH，不影响其他模型。外部悬浮窗定时轮询存在观测延迟，无法单独保证并发请求和最后一发不穿透。

当前看到的插件 `turn/interrupt` 位于其桌面远程桥，作用于该桥维护的 active/starting 映射，见 [桥内中断](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L9492-L9497>)。**不能将它当成已验证可控制任意 DSH GUI 会话的通用 HTTP 暂停接口。** 实现阶段仍需确认受支持的 DSH 会话发送/取消契约及发送前扩展点，再做绑定会话的真实联调。

### 3.3 隔离必须贯穿接收和控制

- 缓存、轮询退避、燃烧率学习器、通知、暂停去重和恢复状态都按 `SourceKey` 分区。仅改 UI 标签或增加 Provider 枚举不足以隔离。
- 每次读取捕获 `SourceKey + generation`，在写缓存、学习器、通知和控制队列**之前**核对；切源、切账户或解绑时递增 generation，取消旧 pending 工作。
- 同一个 account 字符串出现在本机和 DSH 时仍视为不同来源；但若两边实际登录同一 OpenAI 订阅，服务端额度本来就可能共享。来源隔离不代表能够把共享订阅额度拆成两份。
- 明确“精确耗尽”和“低额度保护”两个策略。现有本机默认 5h/周阈值是 5%/2%，且配置不接受 0；不能把沿用该设置称为“用完才暂停”。判断使用原始精度，不使用 UI 四舍五入后的 0%。
- `unknown/stale/读取失败` 不当作“已恢复”；5h 已重置但周额度仍耗尽也不能自动解锁。自动恢复不是本轮已实现或已验证的能力。

## 4. 最小实施顺序与验证

1. **来源契约与竞态防护**：补充 SourceKey/generation、来源枚举及显式错误分支，隔离缓存和持久状态。主要落点为 [额度模型](../../src-tauri/src/codex/client.rs#L31-L55)、[刷新编排](../../src-tauri/src/commands.rs#L354-L429)、[来源配置](../../src-tauri/src/config.rs#L25-L45)、[前端模型](../../src/types.ts#L1-L18)。
2. **只读接入**：桥接 DSH 插件快照，先在界面清楚显示 DSH 实例、订阅账号和新鲜度；验证插件断开时不会读成本机。
3. **绑定会话的暂停**：核验 DSH 控制契约，增加专用控制适配器和去重；若要求硬阻止请求，再增加 DSH 内发送前 gate。此阶段必须真实验证，不能以 mock 送达冒充真实停止。
4. **回归**：复用 [跨提供方隔离测试](../../src-tauri/src/commands.rs#L1904)、[读取策略测试](../../src-tauri/src/codex/reader/tests.rs)、[恢复策略测试](../../src-tauri/src/pause_control/resume.rs)，增加下表的反例。

| 场景 | 必须满足 |
| --- | --- |
| DSH=0%，本机额度充足；以及反向场景 | 不控制错来源；DSH 选择不启动本机读取器 |
| 同名账号、两个 DSH 实例、多个订阅账号 | 不合并状态；控制目标绑定可验证 |
| 切源/切账号时旧响应、旧 429 或旧暂停队列返回 | 被 generation 拒收，不污染新来源或触发旧目标 |
| 插件离线、401、缺窗口、旧缓存、重置时间已到但尚无新读数 | 显示准确状态，不回落本机，也不自行宣告恢复 |
| 多会话并发及重复轮询 | 暂停去重且只作用于绑定身份；硬 gate 的并发行为有单独证据 |
| 同一 ChatGPT 订阅同时用于本机与 DSH | 允许服务端额度相同，但来源、历史和控制目标仍隔离 |

后续可运行的离线回归入口（**本轮未执行**）：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm test -- src/components/ProtectionStatus.test.tsx src/components/SettingsView.test.tsx
```

不要启用 ignored 的真实额度读取 smoke 来代替隔离测试；它会启动本机 Codex 并查询实际登录账号。

## 5. 本轮完成范围

- [x] 安全快进最新 Git，确认版本和源代码工作树状态。
- [x] 定位独立插件额度来源、分账户接口和既有短时额度等待逻辑。
- [x] 确认不能复用本机暂停桥，记录切源竞态和最小适配边界。
- [x] 将证据、实现顺序和验收反例保存在项目交接文档。
- [ ] 功能实现、DSH 认证接入及真实暂停联调：未执行，不属于本轮完成声明。
