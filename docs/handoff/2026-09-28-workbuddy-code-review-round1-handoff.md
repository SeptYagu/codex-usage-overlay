# WorkBuddy 代码审查交接 — 第 1 轮（codex-usage-overlay v1.1.0）

## 一、审查基本信息与通过项简述

- 被审 HEAD：`2a97264`（`2a9726464d5c5e746d33e2ac6f3100dc52214a2c`，分支 `main`，工作区干净，已与 `origin/main` 同步）
- 基准 SHA：`693a005`；实际审查范围：`git diff 693a005..2a97264`，39 个文件（Rust 后端 + React 前端 + 配置 + 文档），4961 插入 / 428 删除
- 门禁复核：`npm test` 35/35 通过、`npm run build`（含 tsc 类型检查）通过、`cargo test --locked` 30/30 通过 —— 全绿，但不构成审查通过
- 通过项简述：设置模型扩充（7 个新字段 + 旧文件缺省回退 + 字段级 `patch_settings` + revision 防竞态）、周期去重（`advance_cycle` 基线/跨越/回退语义与规格 §5.3 一致）、音频队列（通知优先于试听、10 秒截断、单线程拥有播放器）、托盘菜单按内容测量收放（generation + revision 防串号）四条主链路的实现与调用链独立核对无异常。规格/实现逐项对照未发现漏实现或表面完成。
- 独立验证：自行设计并执行 3 项负向/边界验证，其中 1 项成功证伪实现（缺陷 D1），2 项为形式化核对（尺寸换算一致性、拖拽锁缺口），详见下方缺陷清单。

## 二、审查发现与缺陷清单

### D1（P1）程序性改尺寸使用硬编码近似尺寸，且前端尺寸缓存使内容尺寸永不回写 → 浮窗被裁剪/位移

- **文件与行号**
  - `src-tauri/src/dock.rs:645-652`（`set_full_size`：`width = if show_credits {220.0} else {160.0} * scale`，`height = 50.0 * scale`）
  - 调用点：`src-tauri/src/dock.rs:456`（`mouse_enter` 悬停展开）、`:519`（`auto_hide_changed` 关闭自动贴边）、`:563`（`toggle_overlay_window` 显示窗口）、`:381`（`restore_startup` 兜底）、`:432`（`drag_ended` 失败回退）
  - `src/components/OverlayView.tsx:75-94`（`fitCapsuleSize`：`:76` 新增 `if (collapsed) return;`，`:82-84` `lastSizeRef` 相等即提前返回，`:86` 才真正 `setSize`）
- **触发条件（任一即可，均为常用操作）**
  1. 左键单击托盘图标（或托盘菜单「显示 / 隐藏悬浮窗」）隐藏后再显示；
  2. 悬浮窗已贴边收起（小浮窗），鼠标悬停展开；
  3. 关闭「自动贴边隐藏」，浮窗恢复完整窗口；
  4. 托盘菜单勾选「自动贴边隐藏」后拖动浮窗贴边、再 Hover。
- **实际行为与期望行为**
  - 期望：每次窗口被程序改尺寸后，窗口客户区与胶囊内容尺寸一致（v1.0.1 由前端 `fitCapsuleSize` 保证，也正是 README「hover to expand it」的前提）。
  - 实际：Rust 侧把窗口写成 `220×50 × scalePercent` 的逻辑尺寸，而真实内容宽度**不是**该值；前端因 `lastSizeRef` 缓存命中而**完全不发起** `setSize`，窗口停留在近似尺寸上，胶囊被窗口边界裁切、右移失效。
- **根因**：两处新增逻辑叠加。① 新增的 `set_full_size` 用 v1.0.x 遗留的硬编码近似值（`220/160×50`）当作「完整浮窗尺寸」，它既不含布局（`grouped/stacks`）、语言、数字位数、credit 文本长度，也不等于实测内容尺寸；② 前端新增的 `if (collapsed) return;` 让 `lastSizeRef` 在小浮窗期间保留旧值，`collapse→expand` 后实测宽度与旧值相同 → `OverlayView.tsx:83` 提前返回，`setSize` 永不执行，`dock_window_resized` 也随之不触发（位置二次夹紧同样丢失）。规格 `docs/v1.1.0-technical-spec.md:407` 明确要求「展开后的矩形须覆盖鼠标当前所在的小窗命中区域，避免刚展开就收到 mouseleave。前端恢复完整胶囊渲染后由 fitCapsuleSize 接管尺寸，尺寸变化完成后再夹紧一次」——该契约未落地。
- **影响范围**：主悬浮窗（核心展示）在 4 条常用路径上被裁切：右侧 credit 区整体不可见、倒计时下半行被切；贴边场景下展开矩形小于小浮窗命中区，可能刚展开即触发 `mouseleave`，400 ms 后被 `dock.rs:482-505` 收起（悬停展开表现为「一闪即回」）。`dock-state.json` 与用户锚点不受影响（无数据损坏）。
- **复现方法与验证证据**（两项独立证据）
  1. *生产 CSS 实测*（无头 Edge + `dist/assets/index-D8WqzVo-.css`，脚本置于系统临时目录，未入库）：`Segoe UI` 可用，`grouped` 布局 + credit、175% 缩放时 `.overlay-capsule` 的 `getBoundingClientRect` = **497.5 × 120.75** CSS px（未变换盒 284.31 × 69）；而 Rust 写入的窗口客户区 = `220×1.75 = 385 × 87.5` → 水平溢出 **112.5 px**、垂直溢出 **33.25 px**。`stacks` 布局实测 457×170（与 `set_full_size` 无关布局参数，偏差 72×82.5）；`showCredits=false` 时实测 324×121 对 280×87.5。
  2. *前端状态机探针*（临时 vitest 用例 `src/zz-review-probe.test.tsx`，用仓库自身 mock + 会在 `observe()` 时回调的 `ResizeObserver`，运行后已删除，`git status` 干净）：把 `getBoundingClientRect` 固定为实测值 498×121，`dockState` 依次为 `expanded → collapsed → expanded`。输出：`setSize calls: first-expand=1, after-re-expand=1, last-applied={"width":498,"height":121}, rust-hardcoded-logical={"width":385,"height":87.5}` —— 第二次展开**零次** `setSize`，窗口保持 385×87.5。
  3. 现有测试未覆盖该路径：`OverlayView.test.tsx` 对 `dockState` 只断言 `dock_mouse_enter/leave` 被调用与 `start_dragging` 未被调用，从不断言折叠/展开循环后的 `setSize`；`App.test.tsx` 的 mock 也不含 `set_full_size` 语义，因此全绿测试无法发现。
- **修复建议**（供参考，审查轮不落码）
  1. 消除「Rust 猜测完整浮窗尺寸」：`set_full_size` 只应提供临时安全尺寸，且必须让前端在每次程序性改尺寸后强制重新测量——例如 `dock` 侧在 `set_full_size`/`apply_pill` 后 emit `overlay_size_invalidated`（或复用 `dock_state_changed` 增加字段），前端收到后置 `lastSizeRef.current = {0,0}` 并立即 `fitCapsuleSize()`；
  2. 或让 `fitCapsuleSize` 记录「实际写入窗口的尺寸」而非「上次测量的内容尺寸」，并在 `set_full_size` 之后清空该记录；
  3. 保持 `if (collapsed) return;` 的守卫语义（收起时确实不应按内容尺寸撑窗口），但必须在展开的同一帧恢复一次内容尺寸并随后 `dock_window_resized` 二次夹紧，以满足规格 §3.4/§3.5。
- **修复后验收标准**：对上述 4 条触发路径逐条实测：`window.inner_size()` 与胶囊 `getBoundingClientRect` 换算后一致（±1 px），credit 区完整可见；贴边展开后展开矩形完全覆盖小浮窗原命中区，鼠标不动 400 ms 内不收起；并为 `dockState` 折叠/展开循环新增一条断言 `setSize` 被重新调用的前端用例。

### D2（P2）启动恢复把锚点夹紧到「窗口当前所在显示器」，副屏位置被丢弃

- **文件与行号**：`src-tauri/src/dock.rs:654-685`（`place_full_at_anchor` → `work_area(window)` + `clamp_position`）、`src-tauri/src/dock.rs:748-770`（`work_area` 取窗口当前中心点所在显示器）、`src-tauri/src/dock.rs:365-386`（`restore_startup` 先 `set_full_size`/`place_full_at_anchor`，再 `startup_state` + `apply_pill`）、`:731-746`（`clamp_position`）
- **触发条件**：多显示器环境（含负坐标副屏），用户把悬浮窗（或贴边小浮窗）放在非主显示器上 → 退出 → 重新启动（或开机自启）。
- **实际行为与期望行为**：期望恢复上次所在显示器与锚点（计划 §5 验收矩阵「位置恢复合理，不丢失屏幕外」）。实际：进程启动时主窗口位于 Tauri 的默认创建位置（主显示器），`work_area(window)` 因此返回**主显示器**工作区，`clamp_position` 把副屏锚点（例如 `x = -1500`）夹紧到主显示器范围内（`x = work.x`），浮窗跳到主显示器边缘；贴边状态同理：`startup_state` → `apply_pill` 使用同一已被改写的窗口位置，小浮窗被恢复到主显示器边缘。v1.0.1 的实现（基准 diff 中被删除的 `lib.rs` 片段）是 `set_position(saved)` 直接恢复、无夹紧，因此这是本次改动引入的回归。
- **根因**：夹紧所用的工作区来自「窗口改尺寸前的位置」，而非「被恢复锚点所在显示器」。恢复顺序（先 `set_full_size` + 夹紧定位，后按锚点落位）让窗口从未真正移动到目标显示器。
- **影响范围**：多显示器用户每次启动位置丢失，需手动拖回；`dock-state.json` 内容正确（不损坏），但呈现位置错误；负坐标副屏完全无法恢复。
- **复现方法或验证证据**：代码路径推演 + 定值演算：`work = (0,0,2560,1440)`（主屏），`anchor = (-1500, 300)`，`size = 385×87.5` → `clamp_position` 得 `x = 0`（`left = 0`，`right - size.width = 2175`），副屏坐标被抹掉。手工复现：把浮窗放到副屏 → 退出 → 启动，观察浮窗出现在主屏边缘。本机有双显示器环境（2 个屏、150%/125% DPI），但当前自动化会话无可见 Tauri 桌面窗口，未能截图留证。
- **修复建议**：`place_full_at_anchor`（及 `restore_startup`/`apply_pill` 路径）应先按**锚点坐标**取显示器：`window.monitor_from_point(anchor.x, anchor.y)`（Tauri 已有 `monitor_from_point`），失败再回退 `current_monitor()`/`primary_monitor()`，然后用该显示器的工作区做夹紧与 `pill_geometry` 落位；`ensure` 只在锚点完全落在所有显示器之外时回退主屏。
- **修复后验收标准**：副屏（含负坐标）锚点重启后位置误差 ≤ 8 px 且仍在同一显示器工作区内；断掉副屏后重启仍能把窗口夹回可用工作区（不出现屏幕外窗口）；贴边小浮窗在副屏左/右边缘重启后仍落在副屏。

### D3（P3）过期/回退的 `resetsAt` 会取消当前周期已排定的到期补读

- **文件与行号**：`src-tauri/src/notify.rs:217`（无条件用原始 `new_resets_at` 调 `schedule_post_reset_fetch`）、`src-tauri/src/notify.rs:391-406`（`deadline <= now` → `slot.take()` + `task.abort()`）
- **触发条件**：某次成功读取返回的 `resets_at` 比已记录值更早（规格 §5.3 明确按「时间戳回退」处理，`advance_cycle` 在 `notify.rs:117-122` 直接返回 `changed=false`），且该值 + 30 s 宽限已过（服务器缓存/时钟回拨的陈旧响应）。
- **实际行为与期望行为**：期望回退响应只被忽略、不影响当前周期已排定的 `resetsAt+30s` 补读定时器。实际：`schedule_post_reset_fetch` 收到的是这个**陈旧**时间戳，判定 `deadline <= now` 后 `abort()` 掉当前周期挂起的补读任务并返回，于是该额度失去到期唤醒读取，只能等常规轮询（默认 60 s）兜底。
- **根因**：`schedule_post_reset_fetch` 的入参应是「已被 `advance_cycle` 接受并写入周期状态的边界（`last_seen_resets_at`）」，而实现传入了未被校验的原始响应值；`changed=false` 分支同样会走到该调用。
- **影响范围**：重置通知的最坏情况延迟从 ~30 s 变为 ≤1 个刷新间隔；下一次成功读取会重新排定定时器（自愈）。不影响去重正确性（不会重复通知）。
- **复现方法或验证证据**：代码路径推演；`advance_cycle` 已有单测 `missing_future_and_regressed_timestamps_do_not_notify` 覆盖回退分支，但没有任何用例覆盖「回退响应走到 `schedule_post_reset_fetch` 并 abort 挂起任务」。
- **修复建议**：`schedule_post_reset_fetch` 改传 `transition.changed` 后周期状态中的 `last_seen_resets_at`（或在 `changed == false` 时直接不调度/仅做幂等校验）。
- **修复后验收标准**：新增 Rust 单测：先对未来边界 T 排定补读任务 → 再喂入已过期的回退值 → 断言原挂起任务未被 abort（`PendingResetFetches` 槽位仍为该 boundary）。

### D4（P3）拖动期间不检查 `dragging` 锁，用量刷新会程序性移动窗口

- **文件与行号**：`src-tauri/src/dock.rs:575-608`（`keep_docked_in_work_area`：`info.expanded` 分支调 `place_full_at_anchor`，`!expanded` 分支调 `apply_pill`，均未检查 `dragging`），调用点 `src-tauri/src/notify.rs:162`（每次用量读取成功都调用）
- **触发条件**：`auto_edge_hide` 开启且窗口处于 `Docked`，用户 Hover 展开后开始拖动；恰有一次后台/手动用量读取在拖动期间完成（`fetch_usage` 最长 20 s，刷新间隔最小 30 s，覆盖窗口可观）。
- **实际行为与期望行为**：期望拖动期间状态机锁（规格 §3.1 T6、§3.7「拖动、右键菜单打开…取消收起」）同样保护程序性移动。实际：`keep_docked_in_work_area` 会把窗口重新 `set_position` 到锚点/小浮窗位，与用户拖动争夺位置——窗口被拉回，且 `drag_ended`（`WM_EXITSIZEMOVE`）随后读到的 `outer_position()` 可能是程序写入的位置，导致本次拖动结果未按用户意图保存。
- **根因**：`dragging` 只在收起定时器（`collapse_if_current`）与 `leave_window` 中被检查，程序性重定位路径遗漏了同一把锁。
- **影响范围**：拖动期间浮窗跳动/落点错误；不造成数据损坏（锚点仍为写盘一次的物理坐标）。
- **复现方法或验证证据**：代码路径核对；`dock.rs` 测试模块只有 `dock_timer_cannot_collapse_during_drag_or_after_pointer_reenters` 覆盖收起路径，未覆盖 `keep_docked_in_work_area`。真实触发需可见桌面窗口（本机不可用）。
- **修复建议**：`keep_docked_in_work_area` 开头（`info()` 之后）增加 `dragging || menu_open` 的提前返回，或把「程序性移动」统一收口到一个检查 `dragging/menu_open` 的函数。
- **修复后验收标准**：拖动进行中调用 `keep_docked_in_work_area`，断言未发生任何 `set_position`/`set_size`（可用 Rust 单测覆盖状态机判定，或实机拖动 + 60 s 观察无跳动）。

### D5（P3）新增枚举字段缺少「未知值不重置全部偏好」防护（与同文件既有约定不一致）

- **文件与行号**：`src-tauri/src/config.rs:14-20`（`SoundMode` 只有 `#[serde(default)]`，无 `#[serde(other)]`/`#[serde(untagged)]` 兜底），对照 `src-tauri/src/config.rs:5-12`（`OverlayLayout` 用 `#[serde(other)]` 保护）与 `src-tauri/src/config.rs:131-141`（`load_settings` 解析失败即整体回落 `OverlaySettings::default()`）
- **触发条件**：`settings.json` 中 `fiveHourSoundMode`/`weeklySoundMode` 出现未知取值（例如从后续版本降级、手工编辑、其它工具写入 `"alarm"`）。
- **实际行为与期望行为**：期望与 `overlayLayout` 一致——未知值仅该字段回退默认，其余偏好（缩放、透明度、语言、自启动、通知开关、音效路径）保留（计划 §1「新功能需兼容…已有的 `settings.json`」、既有测试 `unknown_layout_defaults_without_resetting_preferences` 亦确立此标准）。实际：整文件 `serde_json::from_str::<OverlaySettings>` 失败，`load_settings` 直接返回 `Default`，**所有**用户偏好被静默重置。
- **根因**：`OverlayLayout` 专项加了 `#[serde(other)]`，但新增的两个 `SoundMode` 字段未沿用同一防护。
- **影响范围**：一次性偏好丢失（可重新设置，无数据文件损坏）；仅在出现未知取值时发生。
- **复现方法或验证证据**：在临时副本上把 `weeklySoundMode` 改为 `"unknown-mode"`，`load_settings` 得到 `OverlaySettings::default()`（可用现有 `config.rs` 测试模块的同一手法复现，例如构造 `json!({"scalePercent":220,"weeklySoundMode":"unknown-mode"})` 后断言 `scale_percent == 220`）。
- **修复建议**：为 `SoundMode` 增加未知值兜底（`#[serde(other)]` + 默认分支，或改用 `#[serde(default)] String` + 显式映射），与 `OverlayLayout` 保持一致。
- **修复后验收标准**：新增单测——未知 `soundMode` 值下 `scale_percent/background_transparency_percent/language/auto_start` 等既有偏好全部保持不变，`sound_mode == Windows`。

## 三、待确认风险与未验证项

1. **自定义音效模式的 WinRT toast 可达性（未验证）**：`notify.rs:314-345` 的 `show_toast` 用 `app.config().identifier`（`com.codex.usage.overlay`）作 AUMID 直连 `tauri-winrt-notification`，而规格 §5.4 要求的 `ensure_aumid_shortcut()`（便携版开始菜单快捷方式兜底）在本次实现中不存在（`Cargo.toml` 也未启用规格列出的 `Win32_System_Com`/`Win32_UI_Shell_PropertiesSystem`）。`docs/stage0-verification.md` V5 亦记录「未测」。若该 AUMID 在便携版未注册，自定义模式可能只有声音、没有通知，而 `show_toast` 仍返回 `Ok`（`notify.rs:269` 已先入队音频）。缺少证据：无可见桌面会话，无法触发真实 toast。建议在 NSIS/MSI/便携三种形态各发一次通知确认；若要闭环，需按规格补 `ensure_aumid_shortcut` 或明确降级文案。
2. **托盘菜单被关闭后可能被异步重排重新弹出（待确认）**：`layout_tray_menu`（`tray.rs:164-165`）在每次测量后都执行 `window.show()+set_focus()`；若菜单在「检查更新」等异步内容变化期间被用户关闭，而后台 `ResizeObserver`（`TrayMenuView.tsx:109-114`）仍被触发，则菜单会自行重现。取决于隐藏窗口在 WebView2 中是否继续投递 ResizeObserver，本机无法验证。建议关闭菜单时置一个「已关闭」标志，或在 Rust 侧对 `hidden` 窗口拒绝 `show`（另见 D1 的同类「程序性改窗口状态」风险）。
3. **未覆盖的实机项（沿用阶段 0 记录）**：原生拖动结束消息与吸附手感、四边命中区、任务栏四边、多显示器 DPI 切换与显示器拔插（V2/V3/V6）、真实用户音频文件与设备恢复、真实 AAC 变体、试听中途停止。残余风险：D1/D2 的实机表现细节（裁切幅度、跳屏幅度）未在可见桌面上截屏确认。
4. **残余验证局限**：本机自动化会话无可见 Tauri 桌面窗口；`cargo test` 以 `--offline` 复用本机缓存运行（等价于 `--locked` 的门禁结果）。D2/D4 的结论基于代码路径推演与定值演算，未取得运行时截图。

## 四、推荐修复顺序与复审验收标准

1. **D1（P1）先修**：它影响主悬浮窗在所有常用路径上的呈现，且是 `dock` 与前端尺寸契约断裂的根；修完必须补一条「折叠→展开后重新 `setSize`」的前端用例。
2. **D2（P2）**：修 `work_area`/`place_full_at_anchor` 的显示器归属判定，避免多显示器位置丢失回归。
3. **D3、D4、D5（P3）**：D3/D5 各补一条单测即可闭环；D4 统一程序性移动入口的锁检查。
4. 复审验收标准：`npm test`、`npm run build`、`cargo test --locked` 全绿；D1–D5 各自「修复后验收标准」逐条通过；D1/D2 需给出实机（含双显示器）测量或截图证据；本轮列出的待确认风险 1/2 需给出结论或明确降级为已知限制并写入预览说明。

---

审查结论：**未通过**。存在 1 项 P1、1 项 P2、3 项 P3 缺陷，须全部修复闭环后复审。
