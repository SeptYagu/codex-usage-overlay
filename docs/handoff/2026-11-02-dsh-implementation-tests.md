# DSH 独立订阅源：实现与测试记录

> 制作已提交并推送 `f9c4974`。以下是离线自动化验证，不是当前 DSH 真实账户/暂停验收。不修改已安装插件、活动 profile 或本机 Codex 登录，不向真实会话发送消息。

## 已实现

- 来源独立为 Codex / Claude / DSH。DSH 只读选中插件实例的订阅额度；无连接或请求失败时不回退到本机 Codex。
- 实例、账户、速率历史、重置状态、读取诊断分区；generation 保护迟到请求、合并刷新等待者和前端事件。
- Desktop 使用 companion 的专用 loopback Bearer，不读取、复制或刷新 Host/OAuth 密钥。凭据留在 native 内存中。
- 账户及普通会话必须手选。DSH 保护默认关闭，5H/WK 阈值各自默认为 0%。
- 暂停只提交固定本地化文本的 `steer`；提交前验证账户、时间、来源、额度及运行会话。请求被接收不代表模型已经停下。
- 取消刷新/切换来源/关闭保护撤销尚未提交的动作。已经开始传输的结果不明请求不重试，切换来源不清除已尝试账本。
- DSH 不自动恢复，不启用本机 Codex 的 named-pipe 暂停或恢复链。

## 测试结果

2026-11-02，在 Windows 本机执行：

| 命令 | 结果 |
| --- | --- |
| `npm test` | 8 个文件、153 项通过 |
| `npm run build` | TypeScript 与 Vite 生产构建通过 |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked` | 主库 415 项通过、7 项原有忽略；Claude helper 10 项通过；其他 bin/doctest 通过 |
| `node --test integrations/dsh-bridge/test/*.test.js` | Node 24 下 15 项通过 |
| `npx --yes --package=node@22 node --test integrations/dsh-bridge/test/*.test.js` | 与 Actions 相同的 Node 22 下 15 项通过 |
| `./scripts/Test-TauriRelease.ps1` | 7 个原有包装/签名格式/标签用例通过 |

7 项 ignored 包含需要真实登录/历史输入的 smoke 与手工回放、子进程夹具入口；没有将它们计入通过数。Windows MSVC 链接器有一条创建 import library 的提示；没有未使用的 `set_provider` 告警。

## 证据 → 结论 → 后续路径

| 证据 | 确认的结论 | 对应路径 |
| --- | --- | --- |
| `commands::dsh_tests` 6 项调用生产刷新/配置链 | 切源清缓存并推进 generation；旧 fetch/合并等待者不写新源；低 DSH 额度不调用本机暂停桥；取消刷新撤销队列；账户路径不混用 | [生产链回归](../../src-tauri/src/commands/dsh_tests.rs) |
| native parser、URL 与 loopback transport tests | RPC 标识、账户前后复核、窗口时长、原始精度、时间、重定向、超时和绑定轮换均有拒绝路径 | [DSH native 模块](../../src-tauri/src/dsh/mod.rs)、[协议回归](../../src-tauri/src/dsh/transport_tests.rs) |
| DSH control 与 polling 单测 | `usedPercent=99.6` 不能因 UI 舍入触发 0% 阈值；取消不消耗未提交次数；源切换保留提交记录；DSH 轮询使用自身阈值 | [暂停状态](../../src-tauri/src/dsh_control.rs)、[轮询](../../src-tauri/src/polling.rs) |
| companion 15 项在 Node 22/24 运行 | Host/Origin/Bearer、重复头、固定 allowlist/schema、body/response 限制、超时、卸载取消、ACL、替换记录与目录链接防护通过 | [桥回归](../../integrations/dsh-bridge/test/bridge.test.js) |
| 前端 provider/generation 与 DSH 设置回归 | 不自动选择实例/账户/目标；旧发现/配对/刷新结果不能覆盖新选择；DSH 不显示本机恢复动作 | [连接回归](../../src/components/DshConnection.test.tsx)、[来源回归](../../src/usageSource.test.ts)、[App 回归](../../src/App.test.tsx) |

CI 已加入 Node 22 的 companion 测试，见 [CI 配置](../../.github/workflows/ci.yml)。制作提交 `f9c4974` 的远端 CI 通过；测试提交 `e132dbb` 首次加入 companion gate 后，远端 10 项在安全发现初始化阶段失败，本机 Node 22/24 仍全部通过。已补充对提权 Windows 创建者默认 `BUILTIN\\Administrators` 所有权的兼容：仅提权进程可将此组所有权转为自身用户，正常用户不请求多余 WRITE_OWNER，其他用户所有权仍拒绝；DACL 仍限定当前用户和 SYSTEM 并回读验证。修正后本机 Node 22 的 15 项再次通过。修正的远端 CI 与发布以对应 GitHub run 为准，不将本机通过当成远端通过。

## 尚未验证与发布策略

- 没有在当前 DSH 中安装 companion、没有重启 DSH，也没有读取真实账户后触发真实暂停。
- 没有对新 UI 做实际 WebView2/Edge 截图审查；React/jsdom 交互测试不能替代视觉布局验收。
- 没有证明额度真的为 0 时模型仍能处理 `steer`；DSH 会话查询与发送之间也没有原子运行期校验。这个版本是协作式提示，不是请求层强制拦截器。
- 新适配先以 `v1.6.0-preview.1` 发布，保留既有稳定版更新渠道；不把离线通过描述为真实环境验收。

首次使用按 [安装与边界](../dsh-subscription.md) 操作：手工安装匹配的 companion，选实例及账户，先只读核对额度，再选专用普通测试会话开启暂停。真实验收应由使用者在可中断的会话进行，不能拿当前开发会话作试验。
