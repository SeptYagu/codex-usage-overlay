# 已安装 DSH 集成接口只读核验

> 2026-11-02；目标为独立 DSH Codex 订阅来源。本文是接口调查，不是端到端验收；未修改已安装 DSH 或订阅插件，未发出 prompt/cancel、未读取凭据、未执行安装。部分 ASAR 内容只复制/解包到临时目录后读取。报告类型：普通集成/API 调查，非漏洞报告（flavor=null）。

## 核心结论

1. 现有订阅 HTTP 路由可用，但**没有 Authorization: Bearer 通用 API token 认证**；合法外部控制面为显式 launch URL → `/connect` → 独立 API Cookie → 持续 WebSocket。不要读取签名密钥、自造 Cookie、复用桌面远程桥。
2. 当前 Desktop 未找到面向用户的“复制本地 Host 连接链接”菜单或外部 CLI 取链接命令。CLI `dsh web` 打印的是该命令启动的 Host URL，不能用于取现有 Desktop 的 token。因此纯外部链接路线有实际配对入口缺口，尚未做真实 connect/usage 验证。
3. **小型独立 Host 插件可通过公开 `connection.createPeer()` 与 `createSharedFetchHandler('/api')` 同进程调用原订阅路由**，不依赖浏览器 token。可作为受限本机 IPC companion，但必须由用户通过插件管理器安装，不改已装插件。
4. 普通 GUI 会话的受支持入口是 `session/list`、`session/prompt`、`session/cancel`。`steer` 的“暂停”是消息，不是确定性停止；`cancel` 取消当前 turn 且保留 pending inbox，不是持久暂停整个系统。
5. 未发现公开稳定 profile/server/installation UUID；`$events.ready.host.home` 是 OS 用户主目录，`clientId` 是每次流的新随机 UUID。建议每次明确配对建立 overlay 自身的公开 source/instance UUID。

## 范围与证据位置

授权：本项目主代理委派的只读调查；仅本机安装源码、帮助及运行环境，排除实际暂停、凭据输出、修改安装目录和插件安装。项目中的报告是唯一交付写入。正式 case scope/timeline 不适用：本次不是攻防测试，也未创建安全 case。

临时 ASAR 解包根目录为 `C:\Users\12915\AppData\Local\Temp\dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74`。以下路径均可在本机离线复核；临时文件可能被系统清理。未运行任何解包后的 DSH 主程序。

| 证据 | 来源与位置 | 观察 |
|---|---|---|
| E-001 | [Connection 文档](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-client-connection/README.md#L28-L82>)；[Connection 实现](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-client-connection/lib/index.js#L505-L538>) | token 只在 root/connect 交换，API Cookie 与 Peer 生命周期关联；无 Authorization-header token |
| E-002 | [订阅路由及 envelope](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L3310-L3349>) | 注册准确 POST 路径、校验完整 method 命名空间，消费 payload |
| E-003 | [多账户公开字段](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L10865-L10875>)；[status](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L11152-L11166>)；[账户用量](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L13902-L13929>) | opaque vault UUID，usage/accounts 按 UUID 返回，不含凭据 |
| E-004 | [缓存](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L13801-L13836>)；[force 透传](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/lib/index.js#L15629-L15673>) | force 绕成功缓存，但不绕失败退避或在途合并；时间为毫秒 |
| E-005 | [session prompt](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-api-session-controller/lib/index.js#L925-L965>)；[cancel](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-api-session-controller/lib/index.js#L1056-L1066>)；[列表](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-api-session-controller/lib/index.js#L1940-L2002>) | steer/followup/cancel 语义、running、cold-safe list |
| E-006 | [公开 Peer/Fetch](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-client-connection/lib/index.js#L658-L683>)；[createPeer](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-client-connection/lib/index.js#L751-L754>)；[shared Fetch](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-client-connection/lib/index.js#L815-L838>) | in-process owner Peer 与 exact-route 分派，不需 HTTP Cookie |
| E-007 | [Mux 服务](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/node_modules/@deepseek-ai/dsh-api-gateway/src/stream-server.ts#L42-L179>)；[Host facts](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/node_modules/@deepseek-ai/dsh-api-gateway/src/stream-protocol.ts#L27-L38>) | 无逻辑流也可维持物理 WS；协议 Ping/Pong，最多两次漏回应后断开；host 只有 home |
| E-008 | [现插件 manifest](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/package.json#L5-L44>)；[bundle patch](<C:/Users/12915/.dsh/profiles/desktop/node_modules/dsh-codex-subscription/cordis.patch.yml#L4-L6>)；[安装支持](<C:/Users/12915/AppData/Local/Temp/dsh-api-evidence-a6916a6f626d442c93e59815b13eeb74/dsh/node_modules/@deepseek-ai/dsh-plugin-manager/README.md#L44-L54>) | ESM 命名导出、dsh.bundle.patch、绝对本地包路径 |

来源 SHA-256（2026-11-02 只读取得）：

- E-002/E-003/E-004 的订阅实现：`BECEA0CB25789D0E3CBC5B47F65E168E16EF39BF9C76C5BA14B93D89902FDAF3`
- E-001/E-006 的 Connection 实现：`0EBCCA7583E082716F79B0B55BC1749D48269CC8DB2989B043B321B57ABD3684`
- E-005 的 Session Controller 实现：`A9801ACB577B03053C5D5A3191FF555C5F1D791D0ACDDEC113BEA4511E33B386`
- E-007 的 Mux 服务源码：`CEC92F5946F3A7C417E7041882BAD14D6EFF23B23D057ADFCEBACDBB31E56D2A`

复核方法：按链接用只读文本工具检查所列行；使用 PowerShell `Get-FileHash -Algorithm SHA256 -LiteralPath <所列源路径>` 比较哈希。HTTP wire 示例仅为静态协议样例，未发送；不可将本文当作已通过实测。

## 外部配对与 wire

### 合法认证

用户显式提供当前 Host 的完整 launch URL 后，保留 authority/mount，将 root 交换改为同一 Host 的 `/connect?token=...&interactions=none`。请求禁止自动跨 authority 重定向；接收 302 的 `Set-Cookie` 和相对 `Location: ./api/remote.mux`，把 Cookie 保持在独立 API cookie jar，不写进日志、前端状态或公开配置。

连接该 Mux 的 WebSocket，携带同一 Cookie，保持读循环并响应 WebSocket Ping 控制帧的 Pong。**不要发送 JSON heartbeat**：协议只接受 open/item/end/cancel。`interactions=none` 不需要 `$events` 逻辑流或任何事件应答，仅持有 WS 即可做 HTTP unary RPC；重复使用 Cookie 连接第二个 socket 返回 409。WS 关闭后 Peer 被移除，旧 Cookie 后续 HTTP 返回 401，应重新 connect，而不是仅重开原 Cookie 的 WS。

CLI `dsh web` 默认打印完整带 token URL 并打开浏览器；`--no-open` 仅关闭自动打开。它启动新 Host，不是现有 Desktop Host 的查询命令。Desktop “复制登录链接”是平台 OAuth 登录入口，不是本地 Host launch URL。现有 clean GUI origin 无法独立完成配对。

### 通用 envelope

```json
{
  "type": "client-request",
  "rpcId": "caller-generated-unique-id",
  "method": "codex-subscription/usage/accounts",
  "payload": { "force": true }
}
```

POST 到 `/api/codex-subscription/usage/accounts`，`Content-Type: application/json`。响应：

```json
{
  "type": "server-response",
  "rpcId": "caller-generated-unique-id",
  "result": { "ok": true, "value": { "accounts": [] } }
}
```

必须校验 rpcId、type、HTTP 状态、result.ok 与具体 schema；失败有 `result.error`。订阅 payload 不包 args；标准 Typert Session payload 必须恰为 `{args:{...命名参数}}`。wire 无 `params` 字段。

## 账户、用量与一致性

`codex-subscription/status` 的 value：

```ts
{
  authenticated: boolean;
  provider: 'openai-codex';
  type?: 'oauth';
  expiresAt?: number;
  accounts?: Array<{
    id: string; label: string; active: boolean; expiresAt: number; email?: string;
  }>;
}
```

id 是 vault 生成的 opaque UUID，**不是 ChatGPT account_id，也不是邮箱**。active id 由 `accounts.find(a => a.active)?.id` 得到。token 续期及切换不重建 UUID，删除后重新添加会重建。

`codex-subscription/usage/accounts` 的 value 是 `{accounts:[{id,usage}|{id,error}]}`，每项通过 id join status；usage 不自带账户标识。读全部账户不切换 active 账号，最多并行四项。单项错误不影响其他项；整体 ok 不代表每项都有 usage。

usage 主要结构：`{fetchedAt:number,rateLimits:[{id,name?,windows:[{usedPercent,remainingPercent,windowSeconds,resetsAt?}]}],planType?,...}`。`fetchedAt` 是 epoch **毫秒**；window `resetsAt` 是 epoch **秒**。应选 `rateLimits.id==='codex'`，再按 `windowSeconds` 区分窗口，不能混用 code-review/其他 buckets。

一致性建议：先 status，再 force usage/accounts，再 status；active id 或账户集合变化则丢弃样本并重试。单次 status 也不是凭据与列表的事务快照。账户切换通过正常插件 handler 会清 active reader cache，但外部仍应防在途结果与重复时钟。

force 只绕成功 TTL；失败在 retryAt 前仍直接失败（默认 5 秒，上游 Retry-After 有界最大 5 分钟），在途 `requests.run('read')` 仍会合并，因此 force 不保证“一次独立新网络请求”。不要按整秒去重；最多按同一 source/account/fetchedAt 计一次。自动动作只消费足够新的、无错误的样本；比较请求起始时间时允许同毫秒相等，无法证明新鲜则稍后 force 重读，不用缓存样本累计确认次数。

## 普通会话列表和控制

| Endpoint / method | payload | 语义 |
|---|---|---|
| `/api/session/list` / `session/list` | `{args:{request:{}}}` | cold-safe；返回 `{items:[...]}`，不激活 Agent |
| `/api/session/prompt` / `session/prompt` | `{args:{request:{requestId,sessionId,mode:'steer',content:[{type:'text',text:'暂停'}]}}}` | 幂等 requestId；可能恢复冷会话；返回 accepted 只代表消息被接收 |
| `/api/session/cancel` / `session/cancel` | `{args:{request:{sessionId}}}` | 已存活普通 Agent 的 `cancel({kind:'user'},{keepInbox:true})`；不清 inbox |

列表 item：`{sessionId,updatedAt,agentAvailable,formatStatus?,running,blank,parentSessionId?,origin?:'subagent',cwd?,projections?:{kind:'cached'|'sequenced',asOfSeq,values}}`。不存在 busy/processing 字段。模型路由在 `projections.values.modelSelection.{lastUsed,next}`，各为 `{provider,model,reasoningEffort?}|null`；运行中的 lastUsed 比 next 更贴近当前请求。title、agentPreset 也在 values 中，字段可缺失。

没有从 Session 模型路由反推出它当前使用哪个 subscription vault account 的公开绑定。必须让用户明确选普通 sessionId，不控制 `origin==='subagent'` 的会话；subagent 另有父地址协议。不要把 Codex 手机 Remote Control 的 thread/turn/interrupt 当普通 DSH API。

## Companion 最小可靠方案

公开签名：

```ts
createPeer(options: {
  readonly principal: Principal;
  readonly interactions?: 'answer' | 'none';
}): PeerScope;
createSharedFetchHandler(channel: '/api'): ConnectionFetchHandler;
peer.dispose(): Promise<void>;
```

使用读取服务的同一插件 Context 创建 owner Peer，并通过 shared handler 的 `fetch(request,peer)` 处理固定 envelope。只调用已知订阅/Session endpoints，不访问 app.services、私有 fetchRoutes 或订阅闭包。Peer 绑定创建者 fiber；卸载会释放，手动提前关闭用幂等 `dispose()`。

包采用 ESM 命名导出 `name`、`inject=['connection']`、`apply(ctx,config={})`；`inject` 是服务 key，不是 npm 包名。只声明 Host bundle，不声明 dsh.client。

```json
{
  "name": "dsh-codex-usage-overlay-bridge",
  "version": "0.1.0",
  "type": "module",
  "main": "./index.js",
  "exports": {
    ".": "./index.js",
    "./cordis.patch.yml": "./cordis.patch.yml",
    "./package.json": "./package.json"
  },
  "files": ["index.js", "cordis.patch.yml"],
  "dsh": { "bundle": { "patch": "./cordis.patch.yml" } }
}
```

bundle patch：

```yaml
- insert:
    - id: codex-usage-overlay-bridge
      name: 'dsh-codex-usage-overlay-bridge'
```

GUI 官方安装路径：插件 → 添加插件 → 包名或地址，选择绝对本地包路径或 `file:`/`link:` 绝对路径。默认安装启用；遵循 restart-required。CLI 路径为 `dsh plugin --profile desktop add` 加绝对本地 spec，但须完全退出 Desktop 后进行，不能由另一个运行中 CLI 写活动 profile。上述安装均未执行。

IPC 必须独立鉴权：仅绑定 `127.0.0.1` 随机端口、随机专用 Bearer、严格 allowlist、请求体上限、schema 校验及超时；拒绝不匹配 Origin/跨站访问，不开放 CORS。Bearer 文件不等于公开 discovery：当前用户 LocalAppData 下发布并验证 Windows ACL；`mode:0o600` 不能替代 Windows ACL。临时文件原子发布，清理时核对自身 instanceId，避免旧 fiber 删除新记录。卸载时先停止接收/中止在途，再关闭 server、dispose Peer 并删自身 discovery。

最小 allowlist：status、usage/accounts、session/list；取消与固定 steer 暂停文本必须显式授权并限制用户选定 sessionId。不开放任意 prompt、任意 RPC、账户选择、logout、reset-credit 或凭据读取。

## Findings → paths

- **F-001（validated/high，design）**：E-001/E-007 证明外部 API 使用 launch-token bootstrap + Cookie + WS-owned Peer，不支持配置型通用 Bearer。调用路径 P-001：用户显式交付当前 Host launch URL → connect →独立 Cookie/WS → HTTP RPC。剩余风险：当前 Desktop 未找到公开取链接入口，尚无真实配对验收。
- **F-002（validated/high，other）**：E-002/E-003/E-004 证明账户与用量可独立消费，但 active 状态/用量非原子、force 不等于独立网络请求。P-002：status → usage/accounts(force) → status →按 vault UUID join/校验freshness。剩余风险：切换竞争、时钟倒退、在途合并。
- **F-003（validated/high，other）**：E-005 证明普通会话有独立受支持控制面。P-003：用户选普通 sessionId → list 核验 →明确授权后调用 prompt 或 cancel →重新观测状态。剩余风险：steer 非确定暂停；cancel 保留 inbox，后续输入可能继续。
- **F-004（validated/high，design）**：E-006/E-008 证明可安装小型 Host companion 通过公开同进程分派避免 launch-link 缺口。P-004：用户插件安装 → owner Peer → fixed shared Fetch →本机受限 IPC。剩余风险：owner 权限很广，安全边界由 companion allowlist/ACL/auth 承担；本次没有实现或加载该插件。

## 交付状态

源码接口及签名已核验并逐项给主代理；真实网络配对/用量尚未验证，因为没有通过受支持控制面得到当前 Host launch URL，且本次不读取 secret/cookie。不需要扩大权限或修改安装目录。下一步可由主代理实现 companion、做离线 schema/协议测试，再由用户安装并验收只读用量，最后另行明确授权控制动作。

附：docs-generator 打包中声明的 `skills/tool-index.md` 不存在；实际使用本会话原生 read/grep/glob/pwsh 工具，未猜测外部工具或安装补齐。
