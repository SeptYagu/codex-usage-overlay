# DSH Codex usage overlay companion

独立、零依赖的 DSH Host 插件：通过公开 Connection Peer/Fetch API 提供一个当前用户受限的本机 HTTP 配对入口。它不读取 OAuth token、DSH launch token、浏览器 Cookie 或 DSH 私有服务表，也不修改订阅插件。

> **权限和边界**：桥拥有一个 DSH owner Peer，但只公开下列四个固定 RPC。持有配对 token 的当前用户进程可以查询账户/会话，并向合法会话提交固定的暂停文本。暂停是 `steer` 消息，**不保证立即停止、不等于取消当前运行**。本包不开放 `session/cancel`、任意 prompt、账户切换、退出登录、工具调用或通用 RPC 代理。请只把 token 交给可信的本机 overlay；同一 Windows 用户本身不属于隔离对手。

## 安装前检查

- DSH 已启用 `dsh-codex-subscription` 3.2.3 或兼容版本；需要公开服务 `connection.createPeer()`、`connection.createSharedFetchHandler('/api')`。
- Node.js ≥ 20.11 用于离线测试；插件运行时使用 DSH 内置 Node，不需要 npm install。
- Windows 需要系统 Windows PowerShell。发布任何含配对 token 的 JSON 前，插件建立并重新读取验证专用目录/文件的 protected DACL：仅当前用户 SID 与 SYSTEM，FullControl、无继承条目、所有者为当前用户。失败则关闭监听和 Peer，**不发布记录**，不会回退到 `chmod` 或宽松 ACL。
- POSIX 使用当前用户所有、目录 `0700`、文件 `0600`，发布路径的任意祖先符号链接/Windows junction 都拒绝。网络共享根也拒绝。
- 不需要配置 DSH API token、Host 端口或 GUI launch URL。

## 离线测试

在本目录执行，不会连接真实 DSH、安装插件或暂停任何真实会话：

```powershell
node --test test/*.test.js
```

测试使用伪 Connection 和本目录下忽略提交的 `.test-tmp`；Windows 上确实执行系统 ACL 建立/验证，验证失败时测试也会失败。测试覆盖 HTTP 鉴权、Origin/Host、严格路由与 schema、32 KiB 限制、错误脱敏、请求超时、双实例、文件安全、启动取消和卸载清理。

## 自动配置与可选手工安装

新版 overlay 内嵌本包。选择 DSH 来源后，后台寻找运行中的 Desktop，通过该安装自带的官方 CLI 执行本地、禁用安装脚本的 `plugin --profile desktop add`。若现有 profile 的其他依赖不在 pnpm 缓存中，官方安装事务可按锁文件下载缺失的包。CLI 使用 DSH 自己的 manifest writer 锁，自动将新 bundle 加入选择；默认的 Host HMR 监听 manifest 并加载纯 Host bridge。不要求用户预装本包、寻找文件或粘贴 token。

官方 CLI 返回成功**不等于**连接成功；overlay 必须等发现记录、认证握手与订阅数据校验。HMR 被关闭或重组要求重启时，界面提示由用户安全重启，不停止当前 DSH。已经安装但被用户禁用的 bundle 不会被自动重新启用。自动配置只支持 Windows 的默认 Desktop home；不猜测自定义运行环境，也不读凭据。

开发或自定义 profile 仍可在目标 DSH 插件管理器选择本机 `.tgz` 或绝对 `file:` spec；这不是默认接入的前置条件。

如 GUI 要求 tarball，先在本目录打包：

```powershell
npm pack
```

再在 GUI 中选择生成的绝对 `.tgz` 路径。按 DSH 插件管理器的 restart-required 提示操作；不要手改安装目录、manifest、writer 锁或已装订阅插件。自动配置使用的是 Desktop 随附的官方 CLI 和其自身安装事务，不是另建一个 DSH Host 或修改 ASAR。

包的 ESM 命名导出为 `name`、`inject=['connection']`、`apply(ctx,config={})`，Host bundle 入口由 [package.json](./package.json) 与 [cordis.patch.yml](./cordis.patch.yml) 声明；没有 client bundle。唯一可选配置为 `profileLabel`：1–80 字符、不含控制字符，默认不写该字段。标签只是用户提示，**不是认证身份**。

## 配对发现

启动成功后监听 `127.0.0.1` 的操作系统分配随机端口，原子发布每个活实例的记录：

```text
Windows: %LOCALAPPDATA%\CodexUsageOverlay\dsh-bridges\<instance UUID>.json
POSIX:   $HOME/.local/state/CodexUsageOverlay/dsh-bridges/<instance UUID>.json
```

记录结构：

```ts
{
  schemaVersion: 1;
  instanceId: string; // 本次插件启动生成的 UUID；不是 DSH session/profile ID
  origin: string;     // http://127.0.0.1:<随机端口>
  token: string;      // randomBytes(32).toString('hex')，桥专用 256-bit pairing token
  profileLabel?: string;
  profileId?: string;  // 公开 profile 目录规范化后的 SHA-256；跨重启稳定、不是凭据
}
```

native 自动连接器读取记录，校验 schema/UUID/严格 loopback origin，并实际握手排除陈旧记录。只有唯一合格实例或已保存的稳定 `profileId` 精确匹配时自动选择；多实例有歧义时不按排序任取。`profileId` 来自公开、纯数据的 `ctx.get('profileContext').dir` 的哈希，只用于找回同一 profile；真正的数据隔离仍包含本次 `instanceId` 和账户。不要扫描 DSH Cookie/签名密钥，不要把桥数据和 Codex CLI 来源合并。`token` 不是 OAuth token 或 Host secret，不应出现在日志、UI、错误报告、导出配置、URL、命令行或 git 中。

每次启动都会产生新的 instanceId/token/端口。卸载删除自己的完整匹配记录，绝不删除其他实例、替换记录或整个发现目录。进程崩溃可能留下过期记录，native 应把连接失败标记为不可用，不使用旧用量触发自动动作；本插件不清扫其他/历史记录。

## native HTTP 协议

请求必须来自原生 loopback 客户端，**不发送任何 Origin**。不支持 CORS、浏览器 fetch、Cookie 或 query token。请求要求：

- `POST` 到下表的**精确路径**；不接受 query、尾斜线、编码别名或其他方法。
- `Host` 恰为记录 origin 的 `127.0.0.1:<port>`，且必须唯一。
- 唯一 `Authorization: Bearer <记录中的 token>`，恒时比较。
- 唯一 `Content-Type: application/json`，可带 charset；正文最多 **32768 bytes**，包括 chunked 实际长度。
- `rpcId` 是 1–128 字符的 `[A-Za-z0-9_.:-]` 字符串；每次请求使用新值。

| 路径 | envelope.method | envelope.payload |
|---|---|---|
| `/api/codex-subscription/status` | `codex-subscription/status` | `{}` 或 `{force:boolean}` |
| `/api/codex-subscription/usage/accounts` | `codex-subscription/usage/accounts` | `{}` 或 `{force:boolean}` |
| `/api/session/list` | `session/list` | `{args:{request:{}}}` |
| `/api/session/prompt` | `session/prompt` | 见下文，仅固定 steer 文本 |

订阅请求示例（token 仅在 native 内存中组装 header；不要粘贴真实值到终端）：

```json
{
  "type": "client-request",
  "rpcId": "caller-generated-id",
  "method": "codex-subscription/usage/accounts",
  "payload": { "force": true }
}
```

桥逐字段验证后重造独立内部 `rpcId`、固定 method、固定 payload，并调用同路径的 `sharedFetch.fetch(Request, ownerPeer)`。外部 rpcId 只用于对应原客户端响应，不向 owner dispatch 透传。错误细节/异常文本/内部 headers 不透传，成功的 `result.value` 保留 DSH 原数据结构。

```json
{
  "type": "server-response",
  "rpcId": "caller-generated-id",
  "result": { "ok": true, "value": { "accounts": [] } }
}
```

所有响应 `Cache-Control: no-store`、不含 CORS 或 Set-Cookie。native 必须同时检查 HTTP 状态、type、rpcId、result.ok 和 value schema；上游 RPC 错误可能是 HTTP 200 + `ok:false`。

### 提交固定暂停文本

请求 payload 的准确类型如下，**所有未列字段均拒绝**：

```ts
{
  args: {
    request: {
      requestId: string; // 标准 UUID；用于 DSH 幂等
      sessionId: string; // 正常 GUI 的 session-UUID，或兼容旧纯 UUID
      mode: 'steer';
      content: [{ type: 'text'; text: 'Pause' | '暂停' | '暫停' }];
    };
  };
}
```

不要将这个类型当成自动授权。overlay 必须让用户明确启用暂停功能、从 `session/list` 选择普通会话、排除 `origin==='subagent'`、校验运行状态及当前来源，并且只消费新鲜、账户身份一致的 usage 样本。固定 `steer` 可能唤醒刚完成的冷会话；accepted 只说明消息被接受。桥不会替 overlay 判断阈值、账户切换竞态或用户选定会话。

### 错误与生命周期

| HTTP 状态 | 意义 |
|---|---|
| 400 | 无效 JSON、envelope/schema 或不允许的字段 |
| 401 | 缺失/错误/重复的桥专用鉴权 |
| 403 | 不匹配的本机 Host，或任何 Origin |
| 404 / 405 / 415 | 路由、方法或 Content-Type 不允许 |
| 408 / 413 / 429 | 请求中断、超过正文上限、超过 16 个在途请求 |
| 502 / 504 | DSH 响应异常或超时；错误已脱敏 |
| 503 | 桥尚未发布或正在关闭 |

每个请求默认 15 秒超时，上游成功响应最多 2 MiB；监听最多 32 个连接。ctx.effect 同步注册 cleanup，再异步启动。卸载会中止启动/在途请求、关闭 listener/connections、dispose owner Peer，再只删除属于本 instance+origin+token 的记录。ACL/启动失败同样关闭并回滚，不产生未处理的 promise rejection。

## 文件与核验依据

- [index.js](./index.js)：纯 Node HTTP、安全 allowlist、公开 Peer/Fetch、Cordis 生命周期。
- [file-security.js](./file-security.js)：路径祖先检查、Windows DACL/POSIX 权限、私密临时文件与原子发布、所有权清理。
- [离线测试](./test/bridge.test.js)：伪 DSH Connection，不访问当前 DSH。
- [已安装 DSH 接口核验报告](../../docs/handoff/2026-11-02-dsh-supported-api-audit.md)：公开 API 签名与本机源码证据。本包和 native 的真实路径严格使用上表的 simple API 路径。

这只是受限 companion；离线测试通过不等于真实安装验收。首次安装后应先验证只读 status/usage/session list；实际暂停必须由用户另行明确授权和选择会话。
