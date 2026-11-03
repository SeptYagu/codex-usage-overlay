# DSH 主动连接改造与验收记录

**状态：已恢复审查并整合到多来源架构，准备 v1.6.0 稳定版；真实 Host 热加载、订阅读数和暂停尚未验收。** 本记录不把 `v1.6.0-preview.1` 的手工接入称为自动连接完成。

## 目标与选择

原要求是读取 DSH 订阅插件额度，耗尽后向明确绑定的会话发送暂停，不混入本机 Codex。首个预览版要求先手工安装 companion，用户明确拒绝，要求主动连接。因此改为：浮窗后台主动发现 Desktop，自动配置必要适配层，等待认证，再读取插件活动账户；用户不必预装、找文件、粘贴链接。

不使用私有签名凭据、浏览器 Cookie、OAuth 或本机 Codex 认证。DSH 的 remote-access 是已认证方法授权，不是外部应用的免认证 bootstrap；直接向现有 HTTP RPC 请求得到 401 不是可以绕过的接入方案。

## Evidence → Finding → Path

| 证据 | 发现 | 实现路径 |
| --- | --- | --- |
| 已安装 Desktop 官方 CLI 载体、`runDesktopCli` 与插件事务公开源码；隔离 home 实际安装 exit 0 | 官方 CLI 使用 manifest writer 锁；新本地 bundle 自动加入选择，不必让用户手工装包 | 原生后台调用同一安装中的官方 Electron CLI，嵌入本地适配包，`--offline`、`--ignore-scripts`，不手改 manifest |
| 默认 HMR 监听 manifest 中 bundle 列表，重组 profile | 不能把“必须退出 DSH”当成固定前置，也不能认为 CLI 成功就代表热加载成功 | CLI 后继续等待发现记录与认证；不能热加载时提示安全重启，绝不杀 DSH |
| JS companion 新增 profile 哈希；16 个协议/安全测试通过 | 运行实例 UUID 随重启变化，不能用它直接识别持久 profile | 公开 profile 目录 SHA-256 只作稳定匹配；认证仍用专用桥令牌，暂停目标不跨重启沿用 |
| Rust 后台与 UI DTO 分离，设置界面只轮询缓存 | 不应靠用户打开设置或多 WebView 各自触发安装 | 原生单例执行发现/安装/连接；UI 展示阶段，显式重试，不自动绑定会话 |

## 修改范围

- [原生安装事务](../../src-tauri/src/dsh/setup.rs)：探测运行中 Desktop 的 ExecutablePath；验证同安装中的 ASAR、官方 CLI 和 pnpm 布局；仅默认 Desktop home；精确校验内嵌 5 文件，保护暂存目录 ACL，并保持 deny-write/delete 读句柄到安装子进程结束。
- [后台连接状态机](../../src-tauri/src/dsh/auto.rs)：唯一实例或稳定 profile 匹配、健康检查、退避、来源代次和取消；无进程等待与真正安装尝试区分；断开挂起。
- [命令与原子绑定](../../src-tauri/src/commands.rs)：状态读取不执行网络；目标绑定核验实例和连接代次，拒绝迟到旧 UI；无自动 steer、prompt 或会话绑定。
- [连接界面](../../src/components/DshConnection.tsx)：主动状态为默认入口；兼容手工链接折叠；后端错误原文不回显；绑定不回放宽泛设置补丁。
- [适配层](../../integrations/dsh-bridge/index.js)：新增公开 profile 哈希，继续只允许四类 RPC；[包元数据](../../integrations/dsh-bridge/package.json)更新至 `0.1.1`，与 overlay 同包分发。
- [用户说明](../dsh-subscription.md)和 [CLI 隔离验证脚本](../../scripts/Test-DshDesktopSetup.ps1)记录默认流程、受限环境及真实验收边界。

## 已完成的可复现验证

1. **官方 CLI 隔离安装通过**：实际已安装 Desktop CLI；临时独立 home 的 desktop manifest 初始 bundles 为空；使用内嵌包副本及 `--offline --ignore-scripts` 安装；新依赖和 bundle 选择均出现，Host 入口存在。pnpm 仅处理本地包，未启动 Host，未读取或修改真实 profile。测试目录已安全清理。
2. **Companion 16/16 通过**：包含真实 Windows ACL、专用令牌认证、重复头拒绝、参数重建、超时/取消、卸载清理，以及稳定 profile 标识跨重启相同、原路径不泄漏。
3. **安装模块独立测试 9/9 通过**：元数据边界、错误脱敏、默认 home 约束、嵌入包精确字节与额外文件拒绝、实际 Windows ACL 重复应用、持有文件锁时允许复制且拒绝写入/删除。
4. **包内容 dry-run 通过**：仅包含必要 5 文件，无测试目录、日志或凭据。

主树完整 Rust、前端最终回归、原生锁定包调用官方 CLI、真实 Host 接入与发布产物仍须后续记录；未发生的验收不计入通过。

## 行为与剩余边界

- 自动读取插件活动账户的额度，但不自动授权暂停任何会话；首次连接和重连均不继承旧运行实例的目标。
- 不向本机 Codex、其他 DSH 实例、团队或后台任务发送暂停；不自动恢复，不承诺 hard stop。
- 只支持 Windows 默认 Desktop home 的自动配置。多个安装、显式禁用、异常 manifest、自定义继承 home 和不安全路径均拒绝猜测处理。
- 安装超时只取消本程序启动的 CLI 子进程；不杀 DSH 或清理外部锁。pnpm 可能仍在收尾，界面必须提示等待。
- 已安装且选择了旧版本时不擅自重装；健康检查和多实例判断仍不可省略。
- 测试不对当前会话发送真实暂停，不用模拟数据冒充真实插件额度。

## 当前交接点：独立分支保存后暂停

用户要求避免与协作者冲突：只把现有工作提交并推送独立分支 `feat/dsh-auto-connect`，不直接更新 `main`；推送后停止，等待明确通知再完成后续工作。没有创建发布 tag 或发新 release。

暂停前已取得的结果：前端 178/178 通过且 TypeScript/Vite 构建通过；Rust 全库基线 434 通过、8 忽略；Companion 16/16 通过；发布打包回归七种情形通过，companion 0.1.1 包验证通过。

最后整合补了崩溃遗留发现文件的只读认证筛选及回归用例；这之后启动的定向 Rust/原生隔离 CLI 验证未完成，已按暂停要求取消，不计为通过。下一次继续时须先重新运行完整 Rust 回归及原生锁定包的官方 CLI 隔离测试，再做真实 Host 热加载、订阅读数与发布验收。尚未对当前会话发送真实暂停，也没有完成真实 DSH 接入验收。

## 2026-11-03 恢复审查与多来源整合

- 用户明确要求审查、合并并发布，版本经确认采用 `v1.6.0` 稳定版。`tauri-rewrite` 已从本机和远程删除。
- 修复旧分支依赖「当前显示 DSH」的问题：后台接入改用 DSH 独立监控偏好，切换浮窗图标不撤销连接。关闭 DSH 监控会取消在途握手，重新开启仅恢复读取，不继承旧暂停目标。
- DSH 控制器与传输位于共享服务；绑定、安装新连接和断开操作同步当前 DSH runtime，不能重置 Codex/Claude 历史。连接安装持有 DSH 动作提交锁，冻结旧来源并撤销未发送动作后才替换传输，保留 preview.4 修复的取消边界。
- 合并提交 `47db6e7`。新增真实 loopback RPC 整合回归：在 Codex/Claude 显示时自动连接、读取和绑定 DSH；切换显示保持连接和 generation；关闭监控取消握手；重新开启清空旧授权目标。18 项 DSH transport 回归通过。
- 当前机器没有运行可识别的 DSH Desktop。主动执行的官方 CLI 隔离测试在发现阶段失败，未安装或修改任何真实 profile；不沿用原机器的成功记录作为本次通过证据。真实 Host 热加载、订阅读数和暂停仍未实机验收。
