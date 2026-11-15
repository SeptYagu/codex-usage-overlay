# codex-usage-overlay v1.9.3-preview.1 重写版审查报告

- **审查对象**：`SeptYagu/codex-usage-overlay`，tag `v1.9.3-preview.1`（相对基线 `v1.9.2`）
- **审查日期**：2026-11-14
- **仓库路径（本机）**：`D:\OneDrive\AiPrograms\codex tools\codex-usage-overlay`
- **改动规模**：171 个文件，+10456 / −30178 行，净减约 2 万行
- **审查方式**：主审逐文件比对 + 两路独立复核（Rust 后端 / React 前端）+ 本机实跑测试

---

## 0. 质量门槛实测（本机复现，与发布说明一致）

| 项目 | 命令 | 结果 |
|---|---|---|
| Rust | `cargo test --manifest-path src-tauri/Cargo.toml --workspace --locked` | `agent_bridge` 21 通过；`codex_usage_overlay_lib` 386 通过 / 5 忽略；二进制与 doc 目标 0 失败 ✅ |
| 前端 | `npm test`（vitest） | 14 个文件 / 190 用例通过，0 失败 ✅ |
| 类型 | `npx tsc --noEmit` | exit 0，无诊断 ✅ |
| CI 门禁 | `.github/workflows/ci.yml` | 比 v1.9.2 **少了一道** `affine-parity-fixture.mjs --check`（见 §2.1）⚠️ |

---

## 1. 优化点（真实成立的改进）

### 1.1 模块化是真的，不是搬文件
- `commands.rs`（4142 行）→ `commands/{mod,providers,settings,updater,usage}.rs`
- `pause_control.rs`（2924 行）→ `pause_control/{mod,status,transport}.rs`
- **测试跟着代码走，且净增**：commands 55 → 54，pause_control 45 → **53**，桥接 21 → 21。
- 新模块之间**没有重复的同名辅助函数**（全仓扫描确认）。
- `pause_control/transport.rs` 对 `mod.rs` **零非测试依赖**，依赖方向单向 → 边界是真的。

### 1.2 `agent-bridge` 共享 crate 设计干净，还顺手修了一个 Claude 的 bug
- `crates/agent-bridge/Cargo.toml` 只依赖 `serde`/`serde_json`/`dirs` —— 没有 `tauri`/`tokio`/`windows`。
- v1.9.2 的 Windows 作业对象进程树回收**留在** `src-tauri/src/bin/claude_bridge.rs:40-62`（`CreateJobObjectW` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + `AssignProcessToJobObject`），逐行比对**除 import 位置外完全未改** → Windows 专属逻辑没有污染跨平台 crate，发布说明的声明成立。
- 共享后的 `parse_timestamp` 采用了 Antigravity 的语义（字符串 epoch-ms 也会除 1000），而 v1.9.2 的 Claude 版本是 `Ok(raw) => raw` —— 旧版 Claude 遇到字符串毫秒时间戳会被 `< 1e11` 守卫拒掉、**丢失重置时间**。统一后 Claude 也正确了。**这是修复，不是退化。**

### 1.3 1 秒无条件轮询 → 事件驱动 + 15 秒兜底
- `src-tauri/src/lib.rs:271-290`：
  ```rust
  tokio::select! {
      _ = status_state.services.status_wake.notified() => {}
      _ = tokio::time::sleep(Duration::from_secs(15)) => {}
  }
  ```
- 唤醒点覆盖全部变更路径：读数事件 `commands/usage.rs:278,434`、设置 `commands/settings.rs:808`、provider 操作 `commands/providers.rs:388,413`、pause 内部 8 处、`monitor.rs:249`。
- `tokio::sync::Notify` 的 permit 语义 = 多路合并安全，因为发布器每次推的都是**当前完整状态**。
- **浮窗倒计时不受影响**：`OverlayView.tsx:113-116` 自带 1 秒本地 tick，`formatResetCountdown` 直接用 `Date.now()`。

### 1.4 监控状态单一真相源
- 后端 `ProviderRegistry` / `AppServices` 持有状态；前端每 provider 只存一份快照，靠单调的 `revision`（全局原子）+ `sourceGeneration` 守卫（`useProviderMonitors.ts:19-27`）。
- 后端构造快照时做了 **TOCTOU 二次校验**（`monitor.rs:378`）。
- `source_generation` **只增不减**（全部是 `fetch_add`），所以前端「拒绝更旧代次」的规则**永远不会把某个 provider 永久卡死**（禁用→重启用也是 +1）。

### 1.5 修掉一个真实的前端 bug
v1.9.2 的 `ClaudeConnection` / `AntigravityConnection` 调的是 `useProtectionStatus(settings.provider)` —— 传的是**全局当前选中**的 provider。因此用户在 Codex 页签看 Claude 连接卡片时，卡片上显示的是 **Codex 的保护状态摘要**。
新版 `BridgeConnectionCard.tsx:76` 传入卡片自身的 provider（`useProtectionStatus.ts:17` 会按 provider 过滤）→ 修好了。

### 1.6 其他成立的改进
- i18n 契约与后端完全对齐：删掉的 4 个 key（`autoResumeAfterQuotaReset`、`protectionManualReview`、`protectionResumeUnavailable`、`resumePhases`）对应被移除的自动恢复功能；后端已不再产生 `manualReview`/`resumeUnavailable`；**无悬空引用**。
- 配置迁移单向、无损、幂等，且**按旧版本逐个测过**（`config.rs:402-516`；测试见 `:844`/`:1175`/`:1206`/`:1266`）。
- `burn-rate-state.json` 迁移逻辑**逐字节未变**（`codex/burn_rate.rs:206-279`）。
- **没有新增 panic 风险**：生产代码 `unwrap`/`expect` 数量持平（179 → 178），且全是原有；`unwrap_or_else(|p| p.into_inner())` 这套容忍锁中毒的写法全程保留。
- **没有丢锁**：`source_commit` 提交门是**新增**的；未发现锁序环。
- Antigravity 的 `.cmd` 垫片修复不再每 10 秒轮询时重跑，只在启动与显式连接时执行。
- 桥接测试整体搬迁无损（21 → 21）；`pause_control` 核心覆盖 44+8 → 22+23+7 = 52。

---

## 2. 不如的地方（退步与风险）

### 2.1 【主要】保留的 Personalized 算法被删掉了唯一的「独立实现对照」
**这是本次重构唯一真正损伤了**已上线功能**的地方。**

v1.9.2 有：
- `src-tauri/src/codex/burn_rate/affine.rs` 里的 `cross_language_tests::every_synthetic_origin_matches_the_independent_javascript_trajectory`
  —— 用 `src-tauri/tests/fixtures/affine-parity.csv`（97 行）回放 96 条合成轨迹，断言 Rust 输出与**独立编写的 JavaScript 参考实现**在 1e-9 相对误差内一致；
- `scripts/affine-parity-fixture.mjs` —— 那份 JS 参考实现；
- `.github/workflows/ci.yml:49` —— `node scripts/affine-parity-fixture.mjs --check`（CI 门禁）。

v1.9.3-preview.1 **四件全删**（测试 + 夹具 + 参考实现 + CI 门禁），`affine.rs` 测试数 3 → 2。

剩下的两个测试只校验**自洽性质**（单调、有界、衰减、serde 往返）。这意味着：只要把公式里任何一个常数改错（`+10.` 岭项、`0.99` 衰减、`n/(n+8)` 信任因子、`0.75x..1.5x` 钳位），**所有剩余测试照样全绿**。而 `AffineCalibration` 就是保留的 `Personalized` 档位，用户能直接选到。

**根因**：夹具的期望值由 JS 参考实现生成，而 Rust 测试的输入结构 `MaturedOutcome` 带 `published_predicted_delta` / `shadow_predicted_delta` 两个来自被删 shadow 模块的字段 → 编译不过 → 连坐删除。

**建议**：把夹具和参考实现找回来（git 历史里有），输入结构改用本地最小定义（不需要 shadow 字段），并**恢复 CI 里那道 `--check`**。这是唯一一处我建议动手修的。

### 2.2 【主要】发布说明对 15 秒兜底的描述与实现不符
- 实现：`lib.rs:285-288` 的 15 秒 tick 是**无条件**的；只有 *bridge 探测* 被 Codex 保护开关门控（`lib.rs:276-279`）。
- 发布说明写「plus a 15-second fallback probe when Codex protection is enabled」／「在开启 Codex 保护时按 15 秒兜底探测」→ 读起来像兜底只在开启保护时生效。**实际不是。**
- 后果：v1.9.2 每秒重发 `auto_pause_status_changed`，现在对**未携带唤醒**的变更（保护徽章、已暂停线程列表）最坏延迟 15 秒；读数本身不受影响（读数走独立发布路径）。

### 2.3 【重要，但是「修 bug」性质】Claude / Antigravity 守卫的武装范围变了
| | 武装条件 |
|---|---|
| v1.9.2 `claude/install.rs` | `settings.provider == Claude && settings.auto_pause_on_low_quota`（注释原文：「The guard is only armed while Claude is the active provider.」） |
| v1.9.3-preview.1 `claude/install.rs:112` | `claude.monitor_enabled && claude.protection_enabled`（Antigravity 同理，`antigravity/install.rs:206`） |

**效果**：现在即使浮窗显示的是 Codex，只要 Claude 的偏好是「监控 + 保护」打开，Claude Code 的 `PreInvocation` 钩子就会**拦截新请求**。

这与 1.6.0-preview.3 写明的「每 provider 独立保护，与显示选择分离」的设计一致 —— 所以**更像是 v1.9.2 才是那个不一致的实现**，v1.9.3 在补齐。但发布说明**一个字都没提**，而这是用户可感知的（Claude Code 开始拒活）。**建议在发布说明里写明。**

### 2.4 【次要】「Resume chat」的形态变了，`protectionManualReview` 警告在三种语言里消失
- 契约从 `resumePlans: Array<{id, threadId, phase, eligibleAfter, reason}>` 变成 `pausedThreads: string[]`（`types.ts:206`、`pause_control/status.rs:97-98`）。
- v1.9.2 的恢复按钮挂在 `resumePlan` 上，条件 `['ready','waitingForQuota','needsReview']`，并对 `needsReview`/`submissionUnknown` 额外显示 `protectionManualReview` 警告；新版按钮对 `pausedThreads` 里**每一个** id 都显示，且无 phase 标注。

**但这不构成安全洞**（这点我专门复核过）：`paused_threads` 是**新引入**的内存账本，注释为「Thread IDs **confirmed paused** during the current session」（`pause_control/mod.rs:169-170`），只在确认暂停处写入（`mod.rs:907`）。而 `submissionUnknown` 属于**自动恢复提交**流程的 phase，那套流程已整体移除。
净效果其实是**改进**：v1.9.2 里只要没开自动恢复，`resumePlans` 就是空的，**根本没有恢复按钮**；现在对所有已确认暂停的线程都有。丢的只是那条警告文案。

### 2.5 【次要】身份短暂不一致时浮窗会闪成 `--%`
- `App.tsx:216-224` 用 `monitor.sourceKey/sourceGeneration` 与 `display.sourceKey/sourceGeneration` 比对才显示读数；两者来自**两条独立流**（`display.rs:336-337` vs `monitor.rs:381-385`）。DSH 重绑定或 Antigravity 换池后，两者会短暂不一致。
- 该窗口内 `isLoading={monitor?.health === 'checking'}` 为 false → **没有加载提示**；而且 `monitor.error` / `monitor.available` 在整个 UI 里**从未被展示过**。
- v1.9.2 的 `usageSourceScope` + `bufferUsage` inbox 对多身份样本处理得更细。

### 2.6 【次要】去重只落在 bridge，没落到 app
- app 内仍有 6 处固定名 `json.tmp` 写法（`config.rs:740,760,795,810`、`codex/burn_rate.rs:377`、`codex/reader.rs:370`、`notify.rs:95`），而 `agent-bridge/src/lib.rs:160-169` 提供了按 PID 的原子写。
- `unix_now` 定义了 **4 份**（`codex/reader.rs:423`、`notify.rs:502`、`pause_control/mod.rs:980`、`agent-bridge/src/lib.rs:117`）。
- `contains_point` 在 `geometry.rs:22` 与 `dock.rs:1896` **各有一份，且边界语义相反**：前者 `>= left && < left+width`（右/下开区间），后者 `>= left && <= left+width`（闭区间）。号称「统一几何」的模块存在，dock 却留着语义不一致的副本。

### 2.7 【次要】测试覆盖净变化
| | v1.9.2 | v1.9.3-preview.1 |
|---|---|---|
| Rust `#[test]`/`#[tokio::test]` | 521 | 413 |
| 前端 vitest 用例 | 192 | 190 |

- Rust 掉的约 100 个**基本可归因于真被删的模块**（`personal.rs` −26、`replay*` −24、`shadow_tests.rs` −41、`resume.rs` −9）。**唯一被保留功能因此失去有效覆盖的是 `AffineCalibration`（§2.1）。**
- `pause_control/status.rs` 另丢了一条安全不变量测试 `cooperative_capabilities_never_allow_automatic_resume`。
- tray / dock / lib 丢的 6 条**可以接受** —— 我逐条确认了底层行为仍被纯规划器测试覆盖：v1.8.5 的「已关闭托盘菜单不再自己弹开」仍在 `tray.rs:1720`，失焦宽限期在 `tray.rs:1733`，贴边锚点持久化在 `dock.rs:1765`。
- 前端：`usageSource.test.ts`（−5）由 `providerSettings.test.ts`（+3）+ `useProviderMonitors.test.ts`（+2）接手，主体逻辑有覆盖。
- **但 `BridgeConnectionCard` —— 现在同时承载 Claude 与 Antigravity 两个 provider 行为 —— 零测试。**
- `src/App.test.tsx:449-462`（"holds early identified monitor snapshot until matching settings arrive"）对身份门禁而言是**空测**：69% 之所以不显示，只是因为 `App.tsx:212` 在 settings 加载中直接返回 null。

### 2.8 【次要】研究证据被整体删除
`docs/handoff/` 整棵目录（75 个文件，含全部 burn-rate 实证 CSV）与 9 个 burn-rate 研究脚本（1227 行）被删。**保留的三档预测算法因此无法再在仓库内复现验证**（git 历史里还在，但仓库变薄了，与发布说明的表述有落差）。

### 2.9 【琐碎】发布说明与测试门禁的小瑕疵
- `docs/releases/v1.9.3-preview.1.md:12` 把 `useProviderMonitors` 称为「新」，实际它 **v1.6.0-preview.3 就有了**（`git log --diff-filter=A` → `af3ca8c`），真正的前端改动是删掉 `usage_updated` 监听与 `get_last_usage`。
- `src/version.test.ts:35-38` 文案写「six places」，实际断言了七处。
- `version.ts` / `package.json` / `package-lock.json`（两处）/ `Cargo.toml` / `Cargo.lock` / `tauri.conf.json` 的 `1.9.3-preview.1` **全部一致**，MSI 的 `wix.version` = `1.9.3.1` 符合 preview 规则 ✅。

---

## 3. 结论与建议

**总体判断**：这是一次**质量明显提升**的重构，不是「为了重构而重构」。
- 净删 2 万行、模块边界真实、测试跟着代码走且总体不减、配置迁移逐版本验证、无新增 panic 风险、还顺手修掉了「Claude 卡片显示 Codex 保护状态」这个前端 bug。
- 但**有一处真损伤**：§2.1 删掉了保留算法 `AffineCalibration` 的跨语言一致性校验与 CI 门禁。**建议优先修回这一条。**
- 另有三条**发布说明没说清的行为变化**需要补文档：§2.2（15 秒兜底无条件）、§2.3（守卫按 provider 武装，Claude Code 可能开始被拦截）、§2.4（手动恢复按钮可用范围扩大、`protectionManualReview` 文案消失）。

**建议动作清单**

| 优先级 | 动作 | 位置 |
|---|---|---|
| P0 | 恢复 affine 跨语言一致性测试 + JS 参考实现 + CI 门禁（输入结构改用本地最小定义） | `codex/burn_rate/affine.rs`、`tests/fixtures/affine-parity.csv`、`scripts/affine-parity-fixture.mjs`、`.github/workflows/ci.yml` |
| P1 | 发布说明补记三条行为变化（§2.2 / §2.3 / §2.4） | `docs/releases/v1.9.3-preview.1.md`、CHANGELOG |
| P1 | 给 `BridgeConnectionCard` 补测试（Claude vs Antigravity：标签、live 失败映射、保护摘要、拦截计数、错误展示） | `src/components/BridgeConnectionCard.test.tsx`（新建） |
| P2 | 身份门禁补一条真实用例（settings 已加载、monitor 身份不匹配） | `src/App.test.tsx` |
| P2 | 统一 `contains_point` 边界语义，统一 `unix_now` 与原子写辅助 | `geometry.rs` / `dock.rs`、`reader.rs` / `notify.rs` / `pause_control/mod.rs` |
| P3 | 考虑恢复 burn-rate 实证数据集到 `docs/evidence/`（可从 git 历史取回） | `docs/` |

---

## 附录：关键证据索引

| 结论 | 证据 |
|---|---|
| 模块拆分真实、测试随迁 | `commands/*`、`pause_control/*` 逐文件 `#[test]` 计数（55→54、45→53、21→21） |
| Windows 作业对象未被污染 | `src-tauri/src/bin/claude_bridge.rs:40-62` 与 v1.9.2 逐行一致；`crates/agent-bridge/Cargo.toml` 无 windows 依赖 |
| 1s → 事件驱动 | `src-tauri/src/lib.rs:271-290` vs v1.9.2 `lib.rs` 同段 |
| 倒计时不依赖后端节奏 | `src/components/OverlayView.tsx:113-116`、`:20-26` |
| 来源代次单调 | `src-tauri/src/monitor.rs:158,196,313`（全为 `fetch_add`）；`:378` TOCTOU 复检 |
| 前端身份门禁 | `src/App.tsx:216-224`、`src/hooks/useProviderMonitors.ts:19-27` |
| Claude 卡片状态 bug 已修 | v1.9.2 `ClaudeConnection.tsx` 用 `settings.provider`；新版 `BridgeConnectionCard.tsx:76` 用卡片自身 provider；`useProtectionStatus.ts:17` 过滤 |
| 守卫武装范围变化 | v1.9.2 `claude/install.rs`（`provider == Claude && auto_pause_on_low_quota`）vs 新版 `claude/install.rs:112` |
| 手动恢复集合语义 | `src-tauri/src/pause_control/mod.rs:169-170,239-242,907`（仅确认暂停才记入） |
| affine 对照被删 | v1.9.2 `affine.rs` `cross_language_tests` + `tests/fixtures/affine-parity.csv` + `scripts/affine-parity-fixture.mjs` + `.github/workflows/ci.yml:49` |
| i18n 契约对齐 | `src/i18n.ts` 删 4 键；后端 `pause_control/status.rs:266-272,336-367` 产出的 summary 值与前端键一一对应 |
