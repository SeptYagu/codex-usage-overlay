# 独立代码审查（第 3 轮终审）：v1.2.0 五大模块技术方案 — 未通过

日期：2026-09-29
被审提交：`3f774b1`（`docs: resolve round 2 review findings across four modules (P3-1..P3-4, R-1..R-3)`）
基准提交：`78d7e96`
实际审查范围：`git diff 78d7e96..3f774b1`（5 个文件 / +992 −25，**全部为文档**：`STATUS.md`、`docs/handoff/2026-09-29-tray-menu-width-optimization.md`、`docs/handoff/2026-09-29-workbuddy-code-review-round1-handoff.md`(新增)、`docs/handoff/2026-09-29-workbuddy-code-review-round2-handoff.md`(新增)、`docs/handoff/INDEX.md`）；本轮缺陷对应的净改动为 `git show 3f774b1 -- docs/handoff/2026-09-29-tray-menu-width-optimization.md`（1 个文件 / +62 −35）。

> 命名说明：`docs/handoff/` 下不存在 `2026-09-29-workbuddy-code-review-round3-handoff.md`，沿用派单模板路径；`2026-09-28-workbuddy-code-review-round{1..4}` 属 v1.1.0 线，`2026-09-29-v1.1.3-code-review-round{3,4}` 属 v1.1.3 线，互不覆盖。

---

## 一、审查基本信息与通过项简述

- 被审 HEAD `3f774b1e96bc656feee1b43523619e379989436d` == `origin/main`，工作区干净，上游 `origin/main` 已配置，`git fetch` 后无新提交（`git pull --ff-only` 无需执行即已一致）；未做 checkout/reset/rebase，未改动产品代码或测试。仓库无 `docs/review-checklist.md`，该步不适用。
- 需求/验收核对：第 3 轮复核的七项口径**在方案文字层面均已落地**——P3-1 唯一守卫（`:65-67`）+ 第三条断言（`:393`）、P3-2 以 `textW` 表述的 `260⇒300`/`340⇒380`（`:354-357`）与 jsdom 替身规范（`:361`）、P3-3 去抖/Toggle 三条断言与虚拟时钟（`:394-397`）、P3-4 白名单→落盘→回读（`:370-373`）、R-1 空列表返回 `None`（`:140-142`、`:380`）、R-2 宿主破平（`:157`、`:378`）、R-3 DPI 截图契约（`:292-293`）。P3-2 与 R-2/R-3 的闭环经代码/脚本独立复核**成立**（见下），P3-1/P3-3 的**逻辑**方向正确、无相反读法残留。
- 独立验证（2 项，临时脚本置于系统临时目录、已删除，未留在仓库）：① 以 Python 逐字复刻 `:130-168` 伪代码，对 2 屏/3 屏/错位屏 × 11 种窗口几何做全排列扫描（60 例）——R-2 的"2 屏 50/50 依据原宿主破平、不受枚举顺序影响"成立（0 例反例），但扫出 3 例新的判据不唯一（见 P3-3）；② 依赖与接缝核查（`Cargo.toml`、`commands.rs`、`lib.rs`、`tray.rs`）——见 P3-1/P3-2。
- 结论：**未通过** — 0×P0、0×P1、0×P2、**3×P3**。本轮 4 条 P3 的**口径文字**已补全，但三条新写的"必须转红"验收（模块四落盘断言、模块二可见性/隐藏断言、模块三归属判据唯一化）在**落地层面**仍不成立：前两条锚在离线单测无法到达的观测点上，第三条的判据在"散文/注释/伪代码"间不唯一。

---

## 二、审查发现与缺陷清单

### P3-1 模块四的「完整落盘与回读」断言锚在 `apply_settings_patch` 上，而该函数在离线单测中无法调用 —— P3-4 的闭环只停留在口径文字

**文件与行号**

- 方案：`docs/handoff/2026-09-29-tray-menu-width-optimization.md:370-373`（§7.2.1 第二、三条："经 `apply_settings_patch(&json!({"showPercentageGrid": true}))` 执行后…断言内存设置 `show_percentage_grid == true`、`revision` 递增…重新通过 `load_settings()` 读取…"）、`:326-328`（§六 `commands.rs` 行）
- 对应代码：`src-tauri/src/commands.rs:259-264`（`async fn apply_settings_patch(app: &AppHandle, state: &Arc<AppState>, patch: Value, allow_auto_start: bool) -> Result<SettingsEnvelope, String>`）、`:395-405`（函数体**无条件**使用 `app`：`app.get_webview_window("settings")`、`app.emit("settings_updated", …)`）、`:378`/`:391` → `src-tauri/src/tray.rs:433-441`（`update_tray_tooltip(app, …)` 内部再调 `app.tray_by_id`）、`src-tauri/src/commands.rs:44-46,49`（`AppState::for_test` 的文档注释："never touch a window, the event loop, the tray or the network"）、`src-tauri/src/lib.rs:713-719`（v1.1.3 落盘类断言既有接缝写法：free function + `AppState::for_test`）、`src-tauri/Cargo.toml`（**无 `[dev-dependencies]` 段**，`tauri = { version = "2", features = ["tray-icon"] }`，未启用 `test` feature）

**触发条件**

实现轮次按 §7.2.1 字面编写"经 `apply_settings_patch` 落盘后回读"的断言时，必然触发。

**实际行为与期望行为**

- 期望（round 2 验收 #4，本轮验收标准亦要求"从白名单贯穿至 `apply_settings_patch` 与 `save_settings_checked` 完整落盘回读断言"）：存在一条可执行、可失败、覆盖 `patch → 合并 → save_settings_checked → load_settings` 终点的断言，键名/映射失配时转红。
- 实际：该断言的入口函数需要 `&AppHandle`，而仓库当前测试体系无法构造 `AppHandle`/`WebviewWindow`——无 `[dev-dependencies]`、未启用 `tauri` 的 `test` feature（`tauri::test::mock_builder` 不可用），65 条既有 Rust 测试中没有任何构造 `AppHandle` 的先例（`AppState::for_test` 按其自身注释与既有用法只覆盖"不碰窗口、事件循环、托盘、网络"的 free function，见 `lib.rs:713-719`）。方案 §七 也没给出替代接缝，§六 的 `commands.rs` 行只新增白名单键。方案给出的调用式 `apply_settings_patch(&json!(…))` 与真实 4 参签名（`&AppHandle, &Arc<AppState>, Value, bool`）亦不一致，说明该口径未经签名核对。

**根因**

round 2 的修复建议把"可复用 `AppState::for_test`"当作充分条件，但 `for_test` 只解决"不触窗口的 `AppState`"，不解决"`&AppHandle` 无法在 `#[test]` 中构造"；本轮照抄该建议写入方案时，未同时补一个不依赖 `AppHandle` 的接缝（或 dev-dependency + `test` feature）到 §六 改动清单，于是断言在纸面上成立、在 `cargo test --locked` 下无落点。

**影响范围**

模块四开关的唯一用户入口。实现者最现实的降级是"断言白名单通过 + 直接 `save_settings_checked` 后 `load_settings` 回读"——这条路径**不经过** `to_value(&settings) → insert(key) → from_value::<OverlaySettings>`（`commands.rs:283-288`），而 serde 对未映射字段静默忽略；因此"字段名/映射与 camelCase 键不一致导致 patch 被静默丢弃"这一 round 1/round 2 反复指认的失败形态，依旧可以在所有拟定测试全绿的情况下发生。同时，`apply_settings_patch` 的合并段对**任何**设置项都没有可离线驱动的证据（历史遗留），本方案本可在本轮一并闭合。

**复现方法或验证证据**

静态核对：`commands.rs:259-264` 签名首参为 `&AppHandle`；`:395-405` 表明 `app` 在成功路径上被无条件使用（`last_usage` 为空时走 `:391` 分支同样调用 `update_tray_tooltip(app, …)`），故不存在"传个空 AppHandle 也能跑通"的路径；`Cargo.toml` 全文无 `[dev-dependencies]`；全仓 `grep -rn "mock_builder\|tauri::test"` 无命中。故按 §7.2.1 字面实现该断言在离线门禁下不可执行。

**修复建议**

1. 在 §六 `commands.rs` 行补一个可测接缝，并把 §7.2.1 的断言改到该接缝上，例如抽出
   `fn merge_settings_patch(current: &OverlaySettings, changes: &Map<String, Value>) -> Result<OverlaySettings, String>`
   （内部完成 `to_value → insert → from_value`，`apply_settings_patch` 改为调用它），§7.2.1 断言改为"经 `merge_settings_patch` → `ConfigManager::save_settings_checked`（`config.rs:177`）→ `load_settings()`（`config.rs:162`）回读"；或
2. 显式声明新增 `[dev-dependencies] tauri = { version = "2", features = ["test"] }`，以 `tauri::test::mock_builder` 构造 `AppHandle`，并注明需同步提交 `Cargo.lock`、§8.3 的 `--locked` 门禁才能通过。

**修复后的验收标准**

- §7.2.1 的落盘断言在 `cargo test --manifest-path src-tauri/Cargo.toml --locked`、无 GUI 环境下**真实可执行**（实现轮次回报用例名与执行结果）。
- 变异：保留白名单键但让 `config.rs` 的 `show_percentage_grid` 字段缺失/改名（不改 `rename_all` 规则）⇒ 该断言转红；把 §六 声明的接缝函数删掉 ⇒ 编译或断言失败（而非静默跳过）。

---

### P3-2 模块二新写的隐藏/可见性断言全部锚在只有真实 `AppHandle` 才能到达的观测点上，方案未给出可离线驱动的接缝（§7.2.6 ⑥ 只覆盖去抖时钟）

**文件与行号**

- 方案：`:61-67`（§2.2.3 唯一守卫契约，未指明 `pending_blur` 与计时起点由谁持有）、`:332-333`（§六 `tray.rs` 状态机行、`lib.rs` 行，均只描述行为、未标接缝）、`:388-397`（§7.2.6：第一项"`tray.rect()` 返回 `None` ⇒ 断言窗口最终可见"；① ② ③ 保护期三向断言；④ ⑤ 去抖/Toggle 断言；⑥ 时钟可注入口径——写在"去抖与 Toggle 状态机断言组"之下）
- 对应代码：`src-tauri/src/lib.rs:88-90`（唯一 hide 出口，位于 `run()` 内传给 `on_window_event` 的**闭包**中，离线测试无法驱动）、`src-tauri/src/commands.rs:20-42`（`AppState` 字段清单：**无**任何保护期/`pending_blur` 字段）、`src-tauri/src/tray.rs:67-70`（`TRAY_MENU_GENERATION` 自增在 `open_tray_menu_window(app)` 内）、`:95-98`（`window.hide()` + emit 同在该函数内）、`:179-182`（`set_size/set_position/show/set_focus` 在 `layout_tray_menu(app, …)` 内，首参 `&AppHandle`）、`:100-102`（`tray_menu_generation()` 只读，无写入接缝）

**触发条件**

实现轮次按 §7.2.6 字面编写模块二断言时（③④⑤ 及第一项的验收官话均以"必须转红"表述）。

**实际行为与期望行为**

- 期望：③"删除 `pending_blur` 守卫时该断言必须转红"（`:393`）、④"移除去抖逐次触发则转红"（`:395`）、⑤"双击 ⇒ 最终不可见"（`:396`）、第一项"`rect()` 为 `None` ⇒ 仍调用 `show()` 且窗口可见"（`:389`）都是可失败断言；⑥ 保证"完全确定性执行，不依赖真实线程 sleep"。
- 实际：这些断言要观测的是 `window.hide()` / `window.is_visible()` / `TRAY_MENU_GENERATION` 自增——三者分别位于 `lib.rs` 的事件闭包、`open_tray_menu_window(app)` 与 `layout_tray_menu(app, …)` 内部，离线测试既拿不到 `AppHandle`，也无法触发 `on_window_event` 闭包；而 `pending_blur` 与 150~200ms 计时起点的持有者（`AppState` 字段？闭包局部状态？）在 §2.2.3 与 §六 中都没有定义，裁决单元也未命名。⑥ 的"可注入时钟"明确挂在"去抖与 Toggle 状态机断言组"下，只覆盖点击路径，保护期路径依旧没有确定性驱动方式。

**根因**

修复把断言写到了"行为层"（hide / 可见性 / generation），但没有把该行为抽成可离线调用的判定单元，也没有在 §六 的 `lib.rs` 行、`tray.rs` 状态机行标注接缝；与 v1.1.3 round 4 P3-1 同类（"接线无可证伪性"），而 v1.1.3 线当时被接受的闭环形态正是"抽出 free function + `AppState::for_test`"（`lib.rs:713-719` 的 `test_state/capture_healthy` 模式）。

**影响范围**

模块二（本轮唯一以"修 bug"为目标的模块）的核心修复——保护期守卫与去抖合流——在 Mode C 轮次很可能只得到"状态机内部单测"，而没有任何断言覆盖最终可见状态；③④⑤ 的"必须转红"前提落空后，`pending_blur` 写错（例如写成 `blur_seen_ever`）或去抖窗口吞掉首个点击都仍可全绿，即 round 1 P3-3 与 round 2 P3-1/P3-3 的病根会在实现阶段原样复发。

**复现方法或验证证据**

静态核对：`lib.rs:88-90` 的 `Focused(false) => { window.hide() }` 是 `Builder::on_window_event` 的闭包分支，测试无法调用；`AppState`（`commands.rs:20-42`）与 `AppState::for_test`（`:49-74`）字段一一对应且**不含**保护期状态，说明 §2.2.3 的状态尚无落点；`tray.rs:67-70`、`:95-98`、`:179-182` 表明 generation 自增、hide、show 全在首参为 `&AppHandle` 的函数内，故 ④ 的"generation 自增恰好为 1"与第一项的"窗口最终可见"在离线单测中同样不可观测。补充反证：方案 §7.1 前端侧给出了明确的 jsdom 替身规范（`:361`），而后端侧 §7.2.6 只给了时钟口径、未给观测接缝，两者粒度不一致。

**修复建议**

1. §六 `lib.rs` 行（`:333`）注明接缝：把保护期状态（`pending_blur` + 起始时刻，时刻取可注入类型）与裁决抽成独立单元，例如 `struct TrayMenuFocusGuard { … }` 或 `fn grace_expiry_action(pending_blur: bool, is_focused: bool) -> Action`，并把 `pending_blur` 放到 `AppState`（`for_test` 同步补齐）或该结构体内；§7.2.6 ①②③ 改为"由该单元断言，时钟注入，不依赖真实 150~200ms sleep"。
2. 模块二把"点击/去抖/Toggle"的输出定义成可观测返回值（如 `enum TrayMenuAction { Show, Hide, Ignore }`），§7.2.6 ④⑤ 改为断言该返回值序列；若 `TRAY_MENU_GENERATION` 自增继续留在 `open_tray_menu_window` 内，则须显式标注 ④ 是"集成断言"并给出驱动方式，或把自增移入可测单元。
3. `tray.rect()==None` 一项（`:389`）同法处理：把 `layout_tray_menu` 的"取 rect 失败 ⇒ 走 fallback 并 `show()`"抽成不依赖窗体的计算 + 一个可注入的窗口动作记录器，或在方案中明确该项只能由集成/实机验证覆盖（不得写成"断言窗口可见"却无落点）。

**修复后的验收标准**

- §7.2.6 ① ② ③ ④ ⑤ 与 `rect()==None` 项各有一行"驱动方式 + 观测对象"（指向具体函数/单元），且这些断言能在 `cargo test --locked`、无 GUI 环境下运行。
- 变异全部转红：删 `pending_blur` 守卫 ⇒ ③ 红；删去抖合流 ⇒ ④ 红；恢复"`Focused(false)` 直接 hide" ⇒ ①/③ 至少一条红；把 `tray.rect().ok().flatten()` 改回 `?` ⇒ `rect()==None` 项红。

---

### P3-3 `select_monitor_by_overlap` 的归属判据在「散文 / 注释 / 伪代码」三者间不唯一：≥3 屏时三种读法给出不同归属，R-2 的"不受枚举顺序影响"只在宿主参与并列时成立，零重叠回退的断言口径也与伪代码不一致

**文件与行号**

- 方案：`:99-100`（§3.2 情形 B："若进入目标屏幕 B 的面积 **<= 50%**（即大半仍留在原屏幕 A，或恰好各 50%）：判定归属保留在**原显示器 A**（以原宿主屏 `fallback_index` 破平）"）、`:131`（§3.3.1 注释："按最大面积（**>50% 多数原则**）确定宿主显示器"）、`:156-160`（并列破平条件 `area > max_area || (area == max_area && area > 0 && idx == fallback_index)`）、`:163-166`（零重叠回退 `monitors.get(fallback_index).or_else(|| monitors.first())`）、`:376-380`（§7.2.2 五条断言，全部为 2 屏场景）
- 对应代码：`src-tauri/src/dock.rs:1132-1149`（`select_work_area` 既有退化链，供对照）、`:1151-1172`（`work_area_for_point`，`fallback` 由当前屏定位、缺省 0）

**触发条件**

≥3 台显示器（或错位/上下排布）的桌面上，浮窗松手时没有任何单屏面积占比 >50% 或两个**非宿主**屏恰好各占 50%。任何一次这样的拖拽松手都会走到该判据。

**实际行为与期望行为**

- 期望（round 2 R-2 的闭合口径 + §7.2.2 自述）：归属判据唯一、与 `fallback_index` 一致、不受 `available_monitors()` 枚举顺序影响，且 §7.2.2 能唯一区分各种读法。
- 实际（独立脚本，逐字复刻 `:130-168` 伪代码后扫描）：
  - **反例 A（枚举顺序敏感，R-2 未普遍闭合）**：宿主 A（`fallback_index=0`），窗口 `(x=3740, y=0, w=200, h=200)` 跨 B|C 接缝各半（面积 A=0、B=20000、C=20000）⇒ `monitors=[A,B,C]` 判 **B**，`monitors=[A,C,B]` 判 **C**。并列条件要求 `idx == fallback_index`，宿主不在并列集合中时该条件永不成立，于是"不受枚举遍历顺序翻转影响"（§7.2.2 原话）不成立。
  - **反例 B（散文与伪代码读法相反）**：宿主 D（屏 D 在 A 下方，`fallback_index=2`），窗口 `(1700, 900, 600, 400)` 压在 A/B/D 错位角上，面积占比 A=16.5%、B=28.5%、D=20.2%（**无任何屏 >50%**）⇒ 伪代码取严格最大面积判 **B**；按 §3.2 散文"<=50% ⇒ 保留原屏 D"应判 **D**；而 §3.3.1 注释自述的判据是">50% 多数原则"，与伪代码的"严格最大面积"在无多数场景下并不等价。三种读法在 §7.2.2 全部断言（2 屏场景）下**都通过**。
  - **反例 C（零重叠口径不一致）**：零重叠时伪代码返回 `monitors.get(fallback_index).or_else(first)`（宿主为屏 B 时返回 **B**），而 §7.2.2 断言"必须退化返回**首屏**真实工作区"。实现者若按断言把回退改成 `monitors.first()`，反而会破坏 §3.3.1 注释声明的"回退至窗口当前所在屏"契约。

**根因**

round 2 把问题收窄到"2 屏恰好 50/50、由枚举顺序决定"，修复据此只加了 `idx == fallback_index` 的并列分支，既没有把"≤50%（无多数）"写成一条 totality 规则，也没有为"非宿主屏之间并列"定义稳定破平键；同时 §3.2/§3.3.1/§7.2.2 三处对同一判据的表述各自演化，没有任何一处被指定为权威；§7.2.2 的断言集合只覆盖 2 屏，天然无法区分三种读法。

**影响范围**

模块三唯一的功能性判据（宿主屏归属）。在三屏/错位布局下，用户松手后的归属屏幕、吸入方向与 DPI 缩放选择可能与文档契约不同；同一几何在不同 `available_monitors()` 顺序下落到不同屏，且该分支在方案中没有一条能失败的断言。属"功能性判据未唯一化"，是本轮三条 P3 中影响面最大的一条。

**复现方法或验证证据**

独立脚本（本机 Python，临时文件已删除，逐字复刻 `:130-168` 的伪代码与 `PhysicalRect` 语义）：

```
CASE 1 host=A, window=(3740,0,200,200), areas=[A0,B20000,C20000]
  monitors=[A,B,C] fb=A(idx0) -> B
  monitors=[A,C,B] fb=A(idx0) -> C          # 枚举顺序敏感
CASE 2 host=D, window=(1700,900,600,400), shares=[A16.5%,B28.5%,D20.2%]（无屏 >50%）
  order=ABD fb=D -> B                       # §3.2 散文读法应判 D
  全排列扫描（2 屏/3 屏 × 11 种几何 = 60 例）：宿主参与并列的 50/50 情形 0 反例（R-2 的 2 屏口径成立）
CASE 3 zero overlap, host=B
  fb=0 -> A ; fb=1 -> B                     # 伪代码返回宿主屏，"返回首屏"的断言口径不符
CASE 4 monitors=[] -> None                  # R-1 成立
```

**修复建议**

1. 把 `select_monitor_by_overlap` 的判据写成一条**全覆盖**契约并让三处表述统一，例如："存在严格最大面积屏 ⇒ 该屏；否则（并列或无多数）⇒ 若宿主在候选集合内取宿主，否则取 `(x, y)` 字典序最小的并列屏"；§3.3.1 注释里的">50% 多数原则"改为"最大面积优先 + 宿主破平"，§3.2 情形 B 补一句"该法则对 ≥3 屏同样成立，宿主不参与并列时按上述稳定键破平"。
2. §7.2.2 增两条可失败断言：① ≥3 屏、无任何屏 >50% ⇒ 归属符合第 1 条的 totality 规则（本条必须能区分"取最大面积"与"一律保留原屏"两种读法）；② 两个非宿主屏恰好 50/50 ⇒ 打乱 `monitors` 顺序断言结果同值。
3. 零重叠断言（`:379`）改为"返回值等于 `fallback_index` 指向的工作区；`fallback_index` 越界时等于首屏"，与 §3.3.1 对齐。

**修复后的验收标准**

- 上述三条断言齐备且可失败：删掉并列破平那一行（退回 `area > max_area`）⇒ 顺序无关断言转红；把 `monitors.get(fallback_index)` 改成 `monitors.first()` ⇒ 零重叠断言转红；把判据改成"无多数一律保留原屏"⇒ 新增的 ≥3 屏断言转红。
- §3.2（散文）、§3.3.1（注释 + 伪代码）、§7.2.2（断言）三处对同一几何给出同一结果，且方案中只有一处被标注为权威定义。

---

## 三、待确认风险与未验证项

**待确认风险（非已确认缺陷，但建议在进入 Mode C 前落结论）**

- **R-1 新签名 `Option` 的消费方契约缺失**：`select_monitor_by_overlap` 改为返回 `Option`（`:139`）后，§3.2 状态矩阵、§六 `drag_ended` 行（`:337`）与 §7.2.2 都只规定"函数自身返回 `None`"，未规定 `drag_ended` 拿到 `None` 时的行为。**怀疑依据**：`drag_ended` 正是本轮改动点，"归属 → clamp → 落盘"三步都依赖该返回值；实现者若写 `.ok_or(…)?` 或 `.expect(…)`，round 2 R-1 要消除的 panic 只是从被调函数搬到了调用方（用户可见路径仍是拖拽松手）。**缺少的证据**：`available_monitors()` 返回空 vec 的可达性（需要 0 显示器会话，本机无法构造）。**建议**：在 §六 `drag_ended` 行与 §7.2.2 各补一句"`None` ⇒ 保留当前锚点、跳过重定位与落盘，不 panic"，并加一条以空列表驱动松手路径的断言（或明确声明该分支仅由函数级单测覆盖）。

**未验证项（受环境限制）**

- 无 GUI / 多屏实机：本机无第二台显示器，多屏拖拽归属、接缝防误折叠、越界回弹、托盘点击/双击时序、620px 高度下是否出现垂直滚动条（T-1）与 100/150/200% DPI 截图均无法执行。
- 未运行任何门禁（本轮为**纯文档**审查，diff 中无产品代码或测试变更）：`npm test`、`npm run build`、`cargo test --locked`、`cargo check --locked` 未在本轮复现（§一/STATUS 引用的 65/46 基线来自 v1.1.3 round 5，方案 §8.3 要求在实现轮次复现）。
- 模块二的保护期与去抖只有代码路径推演与接缝核查，无实机 trace；模块三的归属判据只有离线脚本，未在真实混合 DPI 桌面上核对。

**残余风险**

- `apply_pill` / `clamp_position` 的真实几何、拖拽落盘时序、T-1 的 620px 高度余量（按其自估仅余约 5px）与 R-3 的 DPI 截图，仍只能由实现后的实机验证确认（§5.2 已把截图列入实现后步骤）。

---

## 四、推荐修复顺序与复审验收标准

**修复顺序**（均为方案文字/接缝补齐，不含产品代码改动；按"影响功能正确性 → 影响可落地性"排序）

1. **P3-3**（模块三判据唯一化 + 两条新断言 + 零重叠口径对齐）——唯一一条会直接改变用户可感知归属结果的项。
2. **P3-2**（模块二断言标注接缝/观测对象）——不补则 round 1 P3-3 与 round 2 P3-1/P3-3 的病根在实现阶段复发。
3. **P3-1**（模块四落盘断言改到可离线驱动的接缝上）——P3-4 的闭环前提。
4. R-1 的调用方契约澄清（建议与上述同一轮写入方案，非阻断）。

**复审验收标准（收敛后即可进入 Mode C）**

1. 模块三给出唯一权威判据，覆盖"严格最大面积 / 并列 / 无多数 / 零重叠 / 空列表"五种情形，且 §3.2、§3.3.1、§7.2.2 三处一致；新增的 ≥3 屏无多数断言与"枚举顺序无关"断言可失败。
2. 模块二 §7.2.6 的 ① ② ③ ④ ⑤ 与 `rect()==None` 项各注明驱动方式与观测对象，且删除 `pending_blur` 守卫 / 删除去抖时对应断言转红。
3. 模块四 §7.2.1 的落盘断言指明可在 `cargo test --locked` 离线执行的入口（抽出的合并函数，或新增 dev-dependency + `tauri/test` 并同步 `Cargo.lock`）；映射失配时转红。
4. R-1 的 `None` 分支在 `drag_ended` 侧有明确行为（不 panic）并写入方案。
5. 本轮 3 条 P3 全部闭环后方可进入 Mode C 实现；实现轮次仍需复现四条门禁全绿，但门禁全绿**不构成**通过依据（见 P3-1/P3-2）。
