# 独立代码审查（第 1 轮）：v1.2.0 五大模块技术方案 — 未通过

日期：2026-09-29
被审提交：`8baa2e8`（`docs: finalize v1.2.0 technical design proposal across five modules`）
基准提交：`78d7e96`（`docs: refine tray menu width optimization requirement to tighten algorithm allowances`）
实际审查范围：`git diff 78d7e96..8baa2e8`（2 个文件 / +359 −21：`docs/handoff/2026-09-29-tray-menu-width-optimization.md`、`docs/handoff/INDEX.md`）

> 命名说明：本轮为 v1.2.0 版本线的方案审查第 1 轮；`docs/handoff/` 下当前不存在同名文件，故沿用派单模板路径。历史 `2026-09-28-workbuddy-code-review-round{1..4}-handoff.md` 属 v1.1.0 版本线，互不覆盖。

---

## 一、审查基本信息与通过项简述

- 被审 HEAD `8baa2e8603fb5f4647100327de2651098b7971a0`（`8baa2e8`）`== origin/main`，工作区干净；`git pull --ff-only` → `Already up to date`；未做任何 checkout/reset/rebase，未改动产品代码或测试。仓库内不存在 `docs/review-checklist.md`，该步不适用。
- 逐文件阅读 2 个变更文件；方案引用的代码锚点抽样复核**准确**：`tray.rs:123` 的 `?` 短路、`tray.rs:126-147` 的 fallback 分支、`tray.rs:31-39` 仅匹配 `Click{Up}`、`lib.rs:88` 的 `Focused(false)` 误杀、`dock.rs:397-417` 的 16px 阈值、`dock.rs:511` 未 clamp 的裸坐标落盘、`.tray-menu-item` 的 10px 左右内边距、`.overlay-pill-bars` 的 72px 轨道 —— 全部与代码一致。
- 模块三、四、五的**接口契约主体自洽**：模块三坐标系全部为物理像素（`monitor_work_area`/`outer_position` 同源）；模块四刻度线与 `bottom:0` 起的 `overlay-pill-fill` 几何方向一致、随 `.overlay-pill-bars.overlay-pill-rotated` 自然旋转；模块五 `tauri.conf.json`（480×660/minWidth 380）与 `tray.rs:10-15` 四常数成对更新，且 `place_within_work_area`（`tray.rs:410-415`）会把旧用户 480×660 的落盘几何自动夹到新的 760 下限，方案未提但结论成立。
- 独立验证（3 项，均以真实构件反证，非照抄方案叙述）：① 按 `tray-icon 0.25.1` 源码证伪模块二根因 #3（见 P3-2）；② 以脚本复现模块一宽度算法的固定点行为（见 P3-5）；③ 以既有测试复核设置补丁白名单的"整包拒绝"不变量（见 P2-1）。
- 结论：**未通过** — 1×P2、5×P3（均为方案级缺陷：既含"按键导致功能必然不生效"的漏列改动点，也含时序/拓扑状态机与持久化契约的不完备）。五大模块的目标方向与代码锚点无误，缺陷集中在契约与状态机闭合性。

---

## 二、审查发现与缺陷清单

### P2-1 模块四的 `showPercentageGrid` 未纳入设置补丁白名单 —— 开关必然被后端整包拒绝，"开启百分比格子"永远无法生效，而方案拟定的测试仍会全绿

**文件与行号**

- 后端白名单（方案第六节改动清单**未列出**该文件）：`src-tauri/src/commands.rs:174-202`，判定分支 `:184-185`，兜底 `:191` `_ => false`，拒绝 `:194` `return Err(format!("Invalid settings field: {key}"))`
- 已被测试固化的不变量：`src-tauri/src/commands.rs:218-225`（`rejects_entire_patch_when_one_field_is_invalid`，第 224 行用 `{"futureField": true}` 断言未知键必被拒）
- 调用链：`src/App.tsx:139` `invoke('patch_settings', { patch })` → `commands.rs:166-172` → `commands.rs:259-268`（`validate_settings_patch(&patch, ...)?` 在合并落盘之前）
- 方案缺失处：`docs/handoff/2026-09-29-tray-menu-width-optimization.md:252-257`（配置契约）、`:323-325`（改动清单）、`:360-361`（测试项 5）

**触发条件**

用户在设置界面点击"开启百分比格子"（`SettingsView` → `onPatchSettings({ showPercentageGrid: true })`）。任何一次点击都触发，无需边界条件。

**实际行为与期望行为**

- 期望：白名单接受该布尔字段，`apply_settings_patch` 合并 → `save_settings_checked` 落盘 → 设置修订号递增 → 悬浮窗渲染 9 条刻度线。
- 实际（按方案字面实现）：`validate_settings_patch` 在 `:191` 的 `_ => false` 兜底判 `showPercentageGrid` 非法 → `:194` 返回 `Err("Invalid settings field: showPercentageGrid")` → `apply_settings_patch` 在**进入合并逻辑之前**返回错误 → 设置**不落盘、不生效**，面板上开关回弹。模块四的用户可见功能（需求 4.1 #2）完全不可达。

**根因**

该仓库的设置写入走"闭集白名单 + 整包原子拒绝"契约（`commands.rs:179-196`），新增设置字段必须同时登记到白名单，方案只列了 `types.ts` / `config.rs` / `i18n.ts` / `OverlayView.tsx` / `index.css` / `SettingsView.tsx`，遗漏了唯一的写入闸门 `commands.rs::validate_settings_patch`。方案第七节的验收矩阵也恰好绕开了这一层：`§7.1.4` 只断言前端回调 `onPatchSettings({ showPercentageGrid: ... })` 被触发，`§7.2.5` 只断言配置文件的反序列化默认值——两者都不会经过白名单，因此**功能实际无效时测试仍然全绿**。

**影响范围**

模块四的"设置界面开关"整条链路（唯一的功能入口）不可用；`OverlayView` 的刻度渲染只能靠手改 `config.json` 触发。模块五的双栏布局不受阻。不影响既有 v1.1.3 行为。

**复现方法或验证证据**

1. 本机执行既有不变量测试（已跑，基线绿）：`cd src-tauri && cargo test --locked rejects_entire_patch_when_one_field_is_invalid` → `test result: ok. 1 passed; 0 failed; ... 64 filtered out`，证明"未知键 ⇒ 整包 `Err`"是被测试钉住的生产语义。
2. 静态核对：`commands.rs:184-185` 的布尔分支仅列出 `showCredits | autoCheckUpdates | autoInstallUpdates | fiveHourResetNotification | weeklyResetNotification | autoEdgeHide | mousePassthrough`，不含新字段；`:191` 兜底为 `false`。
3. 反向确认测试盲区：把 `showPercentageGrid` 加入 `OverlaySettings` 但**不加**白名单时，`commands.rs:218-225` 使用的键是 `futureField`，断言不受影响 —— 故 65/65 `cargo test` 与 `npm test` 均不会报警。

**修复建议**

1. 在 `commands.rs:184-185` 的布尔分支追加 `"showPercentageGrid"`。
2. 在 `commands.rs` 的 `settings_patch_tests` 增加一条正向断言：`validate_settings_patch(&json!({"showPercentageGrid": true}), false)` 为 `Ok`，并断言经 `apply_settings_patch` 后 `settings.show_percentage_grid == true` 且 `save_settings_checked` 落盘内容含该键（覆盖"写入闸门"，而非仅前端回调）。
3. 方案第六节改动清单补入 `src-tauri/src/commands.rs`。
4. 附带（同属该模块的完整性）：`src/i18n.ts` 有 `en-US` / `zh-CN` / `zh-Hant` **三种**语言包（`:5`、`:98`、`:191`），方案只写"双语"，实现时须三份同补，否则繁中用户看到英文或裸键。

**修复后的验收标准**

- `patch_settings({showPercentageGrid: true/false})` 返回成功信封且 `revision` 递增；重启后配置文件中该键保持用户选择。
- 关闭白名单条目（变异）后至少一条测试转红，否则视为未闭环。

---

### P3-1 模块二的失焦保护期只"吞掉"事件、无到期复检 —— 保护期内点击别处会让常置顶托盘菜单永久滞留可见；且保护期的计时起点在方案中记错了位置

**文件与行号**

- 被改动点：`src-tauri/src/lib.rs:88-90`（`tauri::WindowEvent::Focused(false) if window.label() == "tray-menu" => { let _ = window.hide(); }`）
- 方案相关条目：`docs/handoff/2026-09-29-tray-menu-width-optimization.md:69-70`（"前 150~200ms 内设立瞬时保护期，忽略来自操作系统的瞬时 `Focused(false)`"）、`:317`（"`open_tray_menu_window` … 并记录打开时间戳"）、`:358-359`（测试项）

**触发条件**

保护期为 `T_show .. T_show+150~200ms`。在这段窗口内用户点击本程序之外的任意窗口/桌面（例如弹出菜单后立刻切回编辑器）。

**实际行为与期望行为**

- 期望（方案 `:70` 原文）："忽略…瞬时 `Focused(false)`，**等待前台激活平稳后才允许因用户点击外部而自然关闭**"。
- 实际：`Focused(false)` 是**一次性**边沿事件。保护期内被丢弃后没有任何到期复检机制（`TrayMenuView.tsx:186-192` 仅有 Escape 键兜底），故这一次点击**永久丢失**，常置顶（`tray.rs:85` `always_on_top(true)`）的托盘菜单会一直浮在屏幕上，直到用户再点一次托盘图标或按 Esc。

**根因**

状态机不闭合：`Focused(false)` 只有"立即 hide"一个出口，方案把它改成"按时间窗过滤"却未定义"过期后重新判定"的转移，等于在保护期内把唯一的自然关闭路径删除了。附带一处起点误差：方案把"打开时间戳"记在 `open_tray_menu_window`（`:317`），而该函数在 `tray.rs:95` 先 `window.hide()`、真正的 `window.show()` 发生在稍后的 `layout_tray_menu`（`tray.rs:181`，由前端测量后反向 invoke）。以"open"为起点虽仍能覆盖 show（同一调用，间隔仅数十毫秒），但保护期的语义基准与实现点不一致，多代并发时容易把"上一代残留"误判为"当代保护期"。

**影响范围**

仅托盘菜单窗口；但后果是留下一块不可自我关闭的常置顶浮层，用户主观体验与被修复的"点了没反应"同级。多屏/穿透场景同样受影响。

**复现方法或验证证据**

代码路径推演（不依赖运行态）：`lib.rs:88` 是唯一 hide 出口 → 保护期分支直接 `return`/忽略后，`window.is_visible()==true` 的窗口再无任何事件驱动其隐藏；`TrayMenuView.tsx` 全文件仅注册 `keydown(Escape)`（`:186-192`），无 blur/pointerleave 兜底。故"单击别处 ⇒ 菜单滞留"是该状态机的必然结果。

**修复建议**

1. 保护期改为"延迟复检"而非"丢弃"：在 `show` 后起一次性定时器（`tokio::time::sleep`，150~200ms），到期时若 `!window.is_focused()` 则 `hide()`；或记录 `FOCUS_GRACE_UNTIL`，在 `Focused(false)` 到达时若在保护期则登记"待判定"，由定时器在到期时统一裁决。
2. 计时起点改记在 `layout_tray_menu` 成功 `show()`（`tray.rs:181`）之后，与真实可见时刻对齐；保留 `open_tray_menu_window` 的 generation 递增语义（`tray.rs:69-70`）。
3. 状态矩阵补充行：`show + grace 窗口内 Focused(false)` → `grace 到期且未获得焦点 ⇒ hide`。

**修复后的验收标准**

- 菜单 show 后立即（< grace）点击别处：菜单在 grace 到期时自动隐藏，不会长期滞留。
- 菜单 show 后 grace 到期前若已获得焦点，则不得被自动隐藏（防止把本模块要修的"瞬时失焦误杀"重新引入）。
- 上述两条各有一条可失败的自动化测试或明确的实机 trace 记录。

---

### P3-2 模块二根因 #3 与依赖实现矛盾：第 2 次点击并未被"静默丢弃"，纳入 `DoubleClick` 既修不好"吞键"、又每次双击多触发一次，与新增的 Toggle 语义叠加后会出现"双击关闭反被重开"

**文件与行号**

- 现有事件匹配：`src-tauri/src/tray.rs:31-39`
- 方案主张：`docs/handoff/2026-09-29-tray-menu-width-optimization.md:53-56`（根因 #3："未匹配 `DoubleClick`，后续点击被静默丢弃"）、`:67-68`（方案 2："将 `TrayIconEvent::DoubleClick` 视为与 `Click` 相同的弹出操作，彻底解决快速点击与双击吞键问题"）、`:71-72`（方案 4：Toggle 语义）
- 依赖实现（`Cargo.lock:4872-4874` 锁定 `tray-icon 0.25.1`）：

  | 文件 | 行 | 内容 |
  | --- | --- | --- |
  | `tray-icon-0.25.1/src/platform_impl/windows/mod.rs` | 439-445 | `WM_LBUTTONUP => TrayIconEvent::Click { button_state: MouseButtonState::Up }` |
  | 同上 | 460-465 | `WM_LBUTTONDBLCLK => TrayIconEvent::DoubleClick` |
  | 同上 | 104-109 | `WNDCLASSW { lpfnWndProc, lpszClassName, hInstance, ..zeroed() }`，即 `style = 0`（**未**注册 `CS_DBLCLKS`） |

**触发条件**

用户在托盘图标上快速连击/双击（穿透开启后"急于关闭穿透"的典型操作），或双击已展开的菜单想把它收起。

**实际行为与期望行为**

- 期望（方案 `:56`、`:68`）：双击的后续点击被吞 → 纳入 `DoubleClick` 后恢复响应。
- 实际：Win32 双击的消息序列为 `BUTTONUP` → `DBLCLK` → `BUTTONUP`。`tray-icon` 对 `WM_LBUTTONUP` 是**无条件**映射为 `Click{Up}`（`:439`），与 `DBLCLK` 分支彼此独立、无去重。因此第 2 次 `BUTTONUP` 早就以 `Click{Up}` 投递，**已被 `tray.rs:32` 的现有匹配命中**（当前实现每次双击本就触发两次 `open_tray_menu_window`，不是"毫无反应"）。反之，因该类未注册 `CS_DBLCLKS`，`WM_LBUTTONDBLCLK` 是否真的投递到托盘窗口本身存疑 —— 若未投递，方案 2 是彻底的 no-op；若投递，则每次双击变成**三次**触发。两种情形下"纳入 DoubleClick"都不是"吞键"的解药。
- 叠加缺陷：与方案 4 的 Toggle 组合后，双击已有菜单时 `Click{Up}` 先 Toggle 关闭、紧接的 `DoubleClick` 再按"打开"处理 → 菜单被重开，用户无法用双击收起（且每代都会 `TRAY_MENU_GENERATION` 自增 + 触发一轮 IPC 测量，正是方案根因 #4 自己指认的竞态来源）。

**根因**

根因 #3 未在生产/依赖层取证即被写为"硬性事实"，导致对策与真实机理错位；同时"把 DoubleClick 等同 Click"与新增的 Toggle 语义在同一个模块内相互冲突，方案未做取舍。

**影响范围**

模块二的响应性收益不成立；双击路径的 generation 抖动加剧（`tray.rs:69-70`、`TrayMenuView.tsx:72-92` 的 `lastLayoutKey`/`revision` 去重会在同 key 时短路，但仍多一轮 hide→show 往返）。

**复现方法或验证证据**

1. 依赖源码级核对（上表行号；`Cargo.lock` 已锁定该版本，非推测）。
2. 实机待定项：确认 shell 是否把 `WM_LBUTTONDBLCLK` 投递到 `tray_icon_app` 消息窗口（该窗口类无 `CS_DBLCLKS`，见 `mod.rs:104-109`），以及一次双击实际触发几次 `open_tray_menu_window`（可临时在 `tray.rs:31` 打印事件枚举观察，不得留在产品代码里）。
3. 反向推演：即使 `DoubleClick` 确实到达，`Click{Up}` 的两条分支（`:439`）也已覆盖两次点击，故"静默丢弃"的表述与依赖实现不符。

**修复建议**

1. 先取证再下结论：以实机 trace（一次性调试打印）确认双击的完整事件序列与当前 `Click{Up}` 命中次数，用事实改写根因 #3，并给出实测数字。
2. 若目标是"双击不产生额外副作用"，正确做法是**去抖/去重**（例如 250~500ms 内合并同一按键的 `Click`/`DoubleClick` 到一次 toggle，或在 `open_tray_menu_window` 内对同 generation 的重复调用做幂等短路），而不是新增第三条"打开"入口。
3. 明确 Toggle 与双击的取舍：推荐"双击 = 与单击同义的一次 toggle"，并在状态矩阵中写明"双击已展开菜单 ⇒ 关闭且不再打开"，避免 `Click`+`DoubleClick` 两次相反动作。
4. 相应地把 `§7.2.4` 的验收从"不 panic"升级为可证伪断言：`N` 次快速点击只产生 1 次有效开关动作（给出 N 与期望动作数）。

**修复后的验收标准**

- 双击托盘图标：菜单的最终可见状态与"单击一次"一致，且 `TRAY_MENU_GENERATION` 的自增次数有明确上界并被测试/日志断言。
- 保留"菜单已展开时再次点击托盘可收起"的 Toggle 行为。

---

### P3-3 模块三 `select_monitor_by_overlap` 的 `fallback` 语义未定义，状态矩阵漏掉"零重叠"情形（错位双屏的空洞区、完全拖出桌面）；退化默认值 `(win, 1.0)` 会让窗口自任宿主工作区并以比例 1.0 折叠

**文件与行号**

- 方案伪代码：`docs/handoff/2026-09-29-tray-menu-width-optimization.md:127-155`（`select_monitor_by_overlap`，`fallback: usize` 参数与 `:152-154` 的 `unwrap_or_else`）、`:114-121`（状态矩阵，共 4 行）、`:199-231`（`detect_edge_multi_monitor`）
- 现有同源函数作对照：`src-tauri/src/dock.rs:1132-1149`（`select_work_area` 对"点不在任何屏幕内"的情况有明确的 `fallback` → `monitors.first()` → `None` 语义）、`:1164-1170`（`fallback` = 窗口当前所在屏的下标，逐级退化）

**触发条件**

1. 显示器错位排布（如 A 1920×1080@(0,0)、B 1920×1080@(1920,400)，即副屏下移 400px），在 `x∈[1920,3840], y∈[0,400)` 的**空洞区**松手；或
2. 把浮窗**整体**拖出所有工作区之外（例如最左屏左侧 `x = -400`，窗口 380 宽已完全离开 `[0,1920]`）后松手。

此时对每个 `work` 的 `overlap_w/overlap_h` 均为 0，`area = 0`，`max_area` 保持 0 → `best_monitor = None`。

**实际行为与期望行为**

- 期望：文档应规定归属（例如"取距窗口中心最近的工作区"或"保留原宿主"），并保证 `apply_pill`/`clamp_position` 拿到**真实**工作区与**真实** DPI 比例。
- 实际（按伪代码）：落入 `:152` 的 `unwrap_or_else` → `monitors.get(fallback)`。`fallback` 由谁计算、含义为何，方案通篇未定义；若该下标越界（或调用方遗忘传入），则返回 `(win, 1.0)` —— 把**窗口自身矩形**当作宿主工作区、并把缩放比例硬编码为 `1.0`。随后 `detect_edge_multi_monitor`（`:218-231`）对 `work == win` 求四个距离，结果全为 `0`，均 `<= threshold`，`min_by_key` 取数组首个 → 恒返回 `Edge::Left`；再经 `is_external_boundary(win, Left, …)`（`:161-196`，与自身矩形比较必然"外部"）判真，于是**误命中一条并不存在的左边缘**，`apply_pill` 用 `scale_percent` × `1.0` 计算胶囊几何：在 150%/200% DPI 显示器上胶囊物理尺寸偏小、贴边位置按错误工作区计算。

**根因**

新 API 的参数契约（`fallback` 的语义与取值来源）未定义；状态矩阵只覆盖"完全在屏内/接触外边界/接缝 >50%/接缝 ≤50%"四种，遗漏"与任何工作区零重叠"这一可由方案自身"拖拽过程全局自由漫游"（`:95-97`）直接产生的第 5 种落点；退化分支返回了一个语义非法（非工作区、比例 1.0）的值而非 `None`。

**影响范围**

错位多屏与 HiDPI 混合场景下的"松手后"表现：误折叠为胶囊 + 胶囊尺寸/贴边位置错误；不会 panic，也不会丢配置（`drag_ended` 仍会落盘），但用户会看到胶囊出现在非预期的边上。单屏或等高并排双屏不触发。

**复现方法或验证证据**

纯函数可离线复现（伪代码本身即为给定输入）：构造 `win = {x:2000, y:50, w:380, h:90}`、`monitors = [({0,0,1920,1080},1.5), ({1920,400,1920,1080},1.0)]`，四个 `area` 依次为 0、0 → `best_monitor=None` → 走 `:152`；再令 `fallback` 越界即得 `(win, 1.0)`，`detect_edge_multi_monitor` 四距离全 0 → 返回 `Edge::Left`。这一路径与 `dock.rs:1132-1149` 现有的"点不在任何屏内"处理方式（有明确退化链）形成对比，说明新函数缺的是同一类契约。

**修复建议**

1. 明确 `fallback` 契约与现有 `select_work_area`（`dock.rs:1132-1149`）对齐：由调用方以"窗口当前所在屏下标 → `0`"计算，并在函数内用 `monitors.get(fallback).or_else(|| monitors.first())`。
2. 退化分支不得返回 `(win, 1.0)`：改为返回 `None`（由 `drag_ended` 走"保留当前锚点 + clamp 回最近工作区"的兜底），或返回"距窗口中心最近的工作区及其真实比例"。
3. 状态矩阵补第 5 行："与所有工作区零重叠（错位空洞区 / 完全出屏）" → 归属最近屏（或保留原宿主），按 `auto_edge_hide` 决定折叠或 clamp，**锚点必须更新为最终位置**（见 P3-4）。
4. 顺带在文档中写清 `is_external_boundary` 的取舍：整条边只要任一段与邻屏相接即判为内部接缝，故错位排布下该边"虚空段"不再触发贴边（见 §三 T-2）。

**修复后的验收标准**

- 新增 Rust 单测：错位双屏空洞区松手 → 归属为最近屏、比例取该屏真实 `scale_factor`、不折叠出错误边缘。
- 新增 Rust 单测：完全出屏松手 → 结果 `Ok`（不 panic）、窗口被夹回某屏工作区内、`scale_factor != 1.0`（HiDPI 用例）。

---

### P3-4 模块三状态矩阵只给"屏内"一行写了"更新保存锚点" —— 被重定位的两类落点（接缝吸入 / clamp 回弹）不更新锚点，导致落盘锚点停在位移前坐标，重启后的屏归属可与会话内归属不一致

**文件与行号**

- 现有落盘顺序：`src-tauri/src/dock.rs:507-520`（`:511` `save_position(&anchor)` 用的是**未 clamp** 的裸 `outer_position`，`:520` `set_anchor_center` 同理）
- 现有重启恢复：`src-tauri/src/dock.rs:837-873`（`:845-849` 用锚点**点**解析工作区 `work_area_for_point` → `select_work_area` 的 `monitor_contains`；`:861` clamp 后才 `set_anchor_center`）
- 方案矩阵：`docs/handoff/2026-09-29-tray-menu-width-optimization.md:114-121`（仅第 1 行写"更新保存锚点"，第 2/3/4 行的重定位结果均未提）
- 方案改动清单：`:322`（`drag_ended`："整合跨屏归属 + clamp 安全回弹 + 锚点持久化"，未说明持久化的是哪一时刻的坐标）

**触发条件**

按方案实现后，把浮窗拖到两屏接缝并松手（无论 >50% 还是 ≤50%），或拖出物理外边界后松手。这两类落点在方案中都会**主动重定位**窗口（"完整滑入/滑回屏 X 接缝内紧贴" / "平滑弹回工作区内边界紧贴"）。

**实际行为与期望行为**

- 期望：持久化锚点与窗口的最终位置一致，重启后归属屏 == 会话内归属屏。
- 实际：若沿用现有 `drag_ended` 的顺序（先落盘、后重定位），落盘锚点仍是"位移前"的坐标（可能落在原屏 A 内或恰好压在接缝上），而窗口实际已滑入屏 B。重启时 `place_full_at_anchor`（`dock.rs:839-849`）用**锚点点位**挑工作区 → 归属回屏 A；`keep_docked_in_work_area` 在非 dock 状态不参与（`dock.rs:705-738` 仅 `info.docked` 时生效），故没有任何机制纠正这次归属漂移。锚点还会同时流入落盘坐标与 `anchor_center`（`dock.rs:512-520`、`:862-869`），影响后续 `place_full_for_transition`（`:893-897`）与 `place_docked_expanded`（`:955-959`）的工作区解析。

**根因**

持久化契约未定义"锚点 = 重定位后的最终位置还是拖拽松手时的原始位置"；矩阵缺少"重定位后回写锚点"这一状态转移。

**影响范围**

跨屏拖拽与越界回弹这两条**本次新增的主路径**在跨会话维度不自洽：用户下次启动可能发现浮窗回到另一块屏/另一个位置。不会丢配置、不 panic。

**复现方法或验证证据**

代码路径推演：`dock.rs:511` 在方案要求的"吸入/回弹"之前执行 → 之后代码（`:538-550`）只改窗口位置与 dock 状态，不再回写 `config_manager`；`load_position` 的唯一写入点即 `:511`（`grep save_position` 全仓库仅此一处，来自 drag 结束路径）。与 `place_full_at_anchor` 的"锚点定屏"逻辑（`:845-849`）组合即可推出跨会话归属漂移。

**修复建议**

1. 矩阵第 2/3/4 行补"更新保存锚点"，并把持久化动作移到**最终位置确定之后**（先算 `select_monitor_by_overlap` → clamp/吸入 → `set_position` → 用 `outer_position()` 或最终 clamped 坐标落盘 → 再 `set_anchor_center`）。
2. 在 `place_full_at_anchor` 侧对齐语义：当锚点点位与窗口实际矩形归属不一致时，优先采用窗口矩形重叠（新算法）而非点位（即复用 `select_monitor_by_overlap`），使两条路径永不互相矛盾。
3. 若选择"先落盘后重定位"，则必须在重定位后补一次 `save_position`，并明确哪一次是权威写入。

**修复后的验收标准**

- 新增 Rust 单测：接缝吸入后 `load_position()` 与最终窗口矩形归属同一屏；下次 `place_full_at_anchor` 得到相同工作区（同屏、同 scale）。
- 越界回弹用例同样断言"落盘坐标已在工作区内"。

---

### P3-5 模块一的宽度契约未唯一化，且根因诊断与代码/测试自带契约相矛盾 —— "+20"是外层 `p-2.5` 内边距而非 item 内边距的重复；按"扣除重复内边距"实现会让弹窗**少 20px**，而拟更新的测试会固化这一错误假设

**文件与行号**

- 现有实现：`src/components/TrayMenuView.tsx:58-69`（`:59` `Math.ceil(getBoundingClientRect().height + 20)`，`:62-65` `scrollWidth` 聚合，`:68` `Math.ceil(intrinsicWidth + 20)`）、`:29-30`（前端 `TRAY_MENU_MIN_WIDTH = 300`/`MAX = 500`）、`:25-28`（注释：前端常数镜像 `tray.rs`，后端权威再夹一次）
- 现有 CSS/结构：`src/index.css:141-156`（`.tray-menu-item { width: 100%; padding: 7px 10px; white-space: nowrap }`）、`src/components/TrayMenuView.tsx:244`（外层 `p-2.5` = 各 10px）
- 现有测试**自带的**契约说明：`src/components/TrayMenuView.test.tsx:19-22`（"item width is what drives the adaptive sizing"）、`:71-80`（`itemScrollWidth = 420` ⇒ 断言 `widthLogical = 440`，注释原文："then **20px of padding** yields 440 logical px"）
- 方案主张：`docs/handoff/2026-09-29-tray-menu-width-optimization.md:20-22`（"`.tray-menu-item` 已有左右各 10px 内边距…测量后外层又叠加了 `Math.ceil(intrinsicWidth + 20)`…导致水平余量被多重放大" → "扣除重复计算的内边距"）、`:312`（改动清单）、`:336-339`（测试项 1/2）、`:314`（后端下限 300→280）

**触发条件**

任何一次托盘菜单测量（每次打开/语言子菜单展开/字号或 DPI 变化都会触发 `measureMenu`）。

**实际行为与期望行为**

- 期望：方案给出**唯一**的宽度公式，使 `widthLogical` 恰好等于"净文本宽 + item 左右内边距(20) + 外层 `p-2.5` 左右内边距(20)"，从而单行不折行、无多余空白。
- 实际（按方案两种并列做法之一实现）：
  - 分支 A（"直接测量文本节点内联宽度"）+ 沿用 `+20`：得到 `textW + 20`，**缺 20px**；
  - 分支 B（"扣除重复计算的内边距"）：同样 `textW + 20`，**缺 20px**。
  两种做法都建立在"当前 `+20` 是 item 内边距的重复叠加"这一诊断上；但该诊断与代码自带注释（`TrayMenuView.tsx:58` "20px covers the outer `p-2.5` padding"）及既有测试断言（`420 → 440`）矛盾。真实根因是**自引用固定点**：因为 item 是 `width:100%`，`scrollWidth ≥ clientWidth` 恒成立，故 `intrinsicWidth = max(content.clientWidth, 20+textW) = max(弹窗宽−20, 20+textW)`，于是 `width_out = clamp(max(width_in, 40+textW))` —— **宽度只增不减**，一旦某次因长文案被撑到 350，就永远停在 350，这正是方案 §1.1 观察到的"膨胀"。

**根因**

把"自引用固定点导致的不收缩"误诊为"内边距重复累加"，并在文档里并列了两种做法而未钉住公式；同时 `§7.1` 的验收项（"220px 净宽 → 落在 280 下限"、"`scrollWidth = 360` 时贴合净宽"）都不足以区分 `textW+20` 与 `textW+40`，拟更新的 `TrayMenuView.test.tsx` 又会由实现者按自己的理解重写期望值 —— 测试与实现容易共享同一错误假设。

**影响范围**

托盘菜单的核心排版契约。若按字面实现，长文案（`检查更新 (当前版本: v1.2.0)`、语言子菜单标签）在部分字号/DPI 下会横向溢出被裁切（外层为 `overflow-y-auto`，不换行）；这是本模块最核心的可感知回归。模块一其余部分（后端下限 300→280 与前端常数同步）方向正确且无问题。

**复现方法或验证证据**

独立复现脚本（本机已执行，临时文件已删除，未留在仓库）：

```js
const MIN=300,MAX=500,PAD=20;
const clamp=v=>Math.min(MAX,Math.max(MIN,Math.ceil(v)));
const step=(popup,textW)=>clamp(Math.max(popup-PAD,PAD+textW)+PAD); // 复刻现有测量
let a=300,t=[];for(let i=0;i<5;i++){a=step(a,196);t.push(a);}        // textW=196
console.log(t);              // [300,300,300,300,300]
let b=500,u=[];for(let i=0;i<5;i++){b=step(b,196);u.push(b);}
console.log(u);              // [500,500,500,500,500]
console.log(196+40, 20+196+20, 196+20); // 236 236 216
```

结论：① 任意初值都是固定点（`300`、`500` 均不动）⇒ 现有算法**只会增不会减**，真实根因是自引用固定点，不是重复累加；② 当前溢出分支 `(20+textW)+20 = 236` 与"应得值 `textW+40 = 236`"相等 ⇒ `+20` 不是 item 内边距的重复；③ 按方案"扣除一次内边距"得 `216`，比应得值少 20px。

**修复建议**

1. 在方案中**唯一化**宽度公式并写明推导：`widthLogical = clamp(280, 500, ceil(textW) + 40)`，其中 `textW` 为最长菜单项文本节点的内联宽度（`Range`/`getBoundingClientRect` 取文本宽度，或"item `scrollWidth` 减去 item 左右内边距 20"），`+40 = item(20) + 外层 p-2.5(20)`。
2. 同时修掉固定点根因：测量必须**与当前窗口宽度无关**（测文本而非 `width:100%` 容器的 `scrollWidth`），使宽度可双向收敛（能收窄、也能撑开）。方案 §1.2 的方向正确，需把这一点写成显式约束而非"或"选项。
3. 更新 `TrayMenuView.test.tsx` 时保留**可反证**的量化断言（沿用现有 `420 → 440` 风格：给定 `textW`，断言精确数值；并补一条"从 500 收窄回 300"的收敛用例）。
4. `tray.rs:17` 的 `TRAY_MENU_MIN_WIDTH` 300→280 与前端 `TrayMenuView.tsx:29` 必须同改（方案 `:314` 已列，实现时勿漏其一，否则前后端下限不一致会让 `§7.1.1` 断言失真）。

**修复后的验收标准**

- 单测：`textW = 220` → `widthLogical = 280`（下限生效）；`textW = 340` → `widthLogical = 380`（精确值，不折行）；连续测量序列收敛且**可下降**（例如先 420 后 220 的长文案，宽度须回到 280）。
- 变异检查：把 `+40` 改成 `+20` 时至少一条测试转红。

---

## 三、待确认风险与未验证项

**待确认风险（非已确认缺陷，不阻断，但建议在实现前落一条结论）**

- **T-1（模块五，"一屏尽览"高度预算偏紧）** 方案 `:302-304` 自估"左栏 ≈460px、右栏 ≈470px，600px 高度下无缝容纳"。按其自身布局核算：容器 `p-5`(上下 40) + Header(≈45，`SettingsView.tsx:43-48`) + Footer(≈60，`:259-263`) ≈ 145px 外框，600 − 145 ≈ **455px 可用**，小于自估的 470px；右栏还要叠两个通知卡片（`:216-249`）。高度依赖实际字号/DPI，静态推导无法定论，故不判缺陷。**验证方法**：实现后 100%/150%/200% DPI 各截一张 960×600 默认窗口图，确认无垂直滚动条；若超出则采用方案已给出的 620 上限（`§5.2` 原文即写"600px ~ 620px"，但与 `SETTINGS_DEFAULT_HEIGHT = 600.0` 不一致，建议同时钉死一个值）。
- **T-2（模块三，整边判定的取舍未记录）** `is_external_boundary`（方案 `:161-196`）只要某条边**任一段**与邻屏相接即判整条边为内部接缝，故错位排布下该边其余"虚空段"不再触发自动贴边/越界吸附。这可能是刻意为之（优先杜绝"接缝上悬空折叠"），属可辩护的工程简化，方案未写明取舍。**验证方法**：错位双屏实机把浮窗拖至该边虚空段，确认期望行为并在文档中记一条决策。

**未验证项（受环境限制）**

- 无 GUI 实机验证：本机无第二台显示器，无法执行多屏/混合 DPI 的拖拽归属、接缝防误折叠、越界回弹、托盘按钮点击与双击时序等端到端检查；方案 `§7` 的实机部分与 `§8` 的门禁若被实现，需在真实多屏 Windows 上复核。
- 未运行前端/Rust 全量门禁（本轮为**纯文档**审查，无产品代码变更）。仅按需运行了 1 项既有 Rust 测试以钉住 P2-1 的白名单不变量（`cargo test --locked rejects_entire_patch_when_one_field_is_invalid` → `1 passed; 64 filtered out`），基线为绿。
- P3-2 的实机尾项：shell 是否把 `WM_LBUTTONDBLCLK` 投递到 `tray-icon` 的消息窗口（该窗口类 `style = 0`，见 `tray-icon-0.25.1/src/platform_impl/windows/mod.rs:104-109`）无法离线定论；但无论投递与否，`WM_LBUTTONUP => Click{Up}`（`:439`）已证明"后续点击被静默丢弃"不成立。
- 残余风险：模块三的 `select_monitor_by_overlap`/`is_external_boundary` 为纯函数、可离线单测；但 `drag_ended` 的落盘时序与 `apply_pill` 的真实几何仍只能靠实机确认（P3-4 的修复验收为此保留了单测口径）。

---

## 四、推荐修复顺序与复审验收标准

**修复顺序**（先让功能可达，再闭合状态机与持久化契约）

1. **P2-1**（1 行白名单 + 1 条写入闸门测试 + 改动清单与三语言包补齐）—— 不修则模块四整体不可用，优先级最高。
2. **P3-5**（钉死宽度公式 `textW + 40` 且测量与窗口宽度无关；同步 `tray.rs:17` 与 `TrayMenuView.tsx:29`）—— 交付的唯一排版契约。
3. **P3-1**（保护期改为"到期复检"，计时起点移到 `layout_tray_menu` 的 `show()` 之后）。
4. **P3-3 + P3-4**（`fallback` 契约对齐 `select_work_area`；矩阵补"零重叠"与"重定位后回写锚点"两行；退化分支不返回 `(win, 1.0)`）。
5. **P3-2**（先用实机 trace 取证改写根因 #3，再以去抖/幂等替代新增 DoubleClick 入口，并明确双击与 Toggle 的取舍）。
6. 落地 T-1 / T-2 的两条决策记录（可与实现同轮完成）。

**Round 2 复审验收标准**

1. 方案文档中 P2-1 涉及的 `commands.rs::validate_settings_patch` 已列入改动清单，且新增一条断言"该键被白名单接受并随 `save_settings_checked` 落盘"；变异（移出白名单）时该测试转红。
2. 模块一给出**唯一**宽度公式与精确数值断言，并含一条"宽度可下降"的收敛用例；把 `+40` 改为 `+20` 时测试转红。
3. 模块二给出保护期的到期复检机制与计时起点，并含"grace 内点击别处 ⇒ 到期自动隐藏"与"已获焦 ⇒ 不隐藏"两条可失败断言；双击行为有实机事件序列证据，且"最终可见状态 == 单击一次"。
4. 模块三 `fallback` 有明确契约（与 `dock.rs:1132-1149` 语义一致），状态矩阵含"零重叠"与"重定位后回写锚点"两行；新增错位双屏空洞区与完全出屏两条离线单测，并断言 HiDPI 下 `scale_factor != 1.0`。
5. T-1 的 DPI 截图证据或明确改用的默认高度值；T-2 的取舍以一句决策写入文档。
6. 全量门禁（`npm test`、`npm run build`、`cargo test --locked`、`cargo check --locked`）在实现轮次复现全绿 —— 但如 P2-1 所示，门禁全绿**不构成**通过依据。
