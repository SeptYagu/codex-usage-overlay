# 全量合规审计（Cross-Plan Compliance Audit）：v1.1.1 行为评估 → v1.1.2 / v1.1.3 计划

日期：2026-09-29
审计角色：独立合规审计员（Auditor）
审计对象仓库：`codex-usage-overlay`
审计起点：`main` @ `974f2f4`（`fix(review): resolve round 4 review findings (P3-1)`）；`git pull --ff-only` → `Already up to date`，工作区干净，未做任何 checkout/reset/rebase，未改动产品代码与测试。

对照文档（契约来源）：

1. `docs/handoff/2026-09-28-update-window-transparency-edge-review.md`（v1.1.1 行为评估 + 方案）
2. `docs/handoff/2026-09-28-v1.1.2-update-plan.md`（1.1.2 计划）
3. `docs/handoff/2026-09-28-v1.1.3-update-plan.md`（1.1.3 计划）

---

## 一、门禁复现（本机在 `974f2f4` 实跑）

| 门禁 | 结果 |
| --- | --- |
| `npm test` | 46/46 通过（3 个测试文件） |
| `npm run build`（tsc + vite） | 通过，0 错误 |
| `cargo test --locked` | **65/65** 通过（`src-tauri/`） |
| `cargo check --locked` | 通过，0 警告 |

与历史记录的对照：1.1.2 执行时的 42/42 + 42/42 已增长为 46/46 + 65/65；数量增长来自 v1.1.3 的几何恢复与捕获接线用例，无用例被删除或跳过。

---

## 二、v1.1.3 承诺项核实（100% 落地）

| # | 计划承诺 | 代码证据 | 判定 |
| --- | --- | --- | --- |
| 1.1 | 默认尺寸 380×560 → 480×660 | `src-tauri/tauri.conf.json:32-33`（settings 窗口 `width:480`/`height:660`）；`tray.rs:10-11` `SETTINGS_DEFAULT_WIDTH/HEIGHT` | 一致 |
| 1.2 | 解除宽度锁死（原 `minWidth==maxWidth==380`） | `tauri.conf.json:34-37`：`minWidth:380 / maxWidth:1600 / minHeight:400 / maxHeight:1600`；全仓已无 380 宽度锁定（仅 `:17` 的 `width:380` 属 `main` 悬浮窗，非设置窗） | 一致 |
| 1.3 | `min_inner_size(380,400)`、`max_inner_size` 宽松上限 | `tray.rs:12-15` 常量 + `:214-215` `.min_inner_size(...)/.max_inner_size(...)` | 一致 |
| 1.4 | `resizable: true`、自由拉伸四边四角 | `tauri.conf.json:42`；`tray.rs:216` `.resizable(true)`（两处创建路径均设置） | 一致 |
| 1.5 | 前端弹性布局、`overflow-x-hidden` | `SettingsView.tsx:40` 容器含 `overflow-x-hidden`；各配置块为 `flex items-center justify-between`，随宽度自然延展 | 一致 |
| 2.1 | 持久化文件 `settings-window-geometry.json` | `config.rs:146-148` `settings_geometry_path()` 即为该文件名 | 一致 |
| 2.2 | `SettingsWindowGeometry{x,y,width,height}` 结构 | `config.rs:98-110`（另含 `#[serde(default)] scale_factor`，见 §五(3)） | 一致（增强） |
| 2.3 | 关闭时保存几何 | `lib.rs:56-68`：`CloseRequested` → `api.prevent_close()` → `persist_settings_geometry(...)` → `hide()` | 一致 |
| 2.4 | 多显示器「工作区交集」校验 | `tray.rs:378-398` `select_work_area` 逐屏计算物理交集面积、取最大非零重叠（而非首个命中） | 一致（增强） |
| 2.5 | 正常情况：应用保存的尺寸/位置并 clamp 在工作区内 | `tray.rs:400-431` `place_within_work_area`（宽高 clamp 到 min/max 与工作区，x/y clamp 到工作区） | 一致 |
| 2.6 | 显示器拔出：回退主屏居中 + 默认 480×660 | `plan_settings_geometry` 返回 `None` → `tray.rs:259-262` → `center_settings_window`（`:274-280` 设 480×660 + `window.center()`） | 一致 |
| 2.7 | 首次启动：默认尺寸 + 显式居中 | 同上（`load_settings_geometry()` 为 `None` 时走同一分支，`:247-250`） | 一致 |
| 2.8 | 最小化防 0×0 覆盖 | `lib.rs:308-331` `sanitize_geometry`：`is_minimized` 与 `width==0||height==0` 直接拒绝写入 | 一致 |
| 2.9 | 缓存兜底（不可用时回写最后一次有效几何） | `lib.rs:77-87` `Moved/Resized` 捕获 → `:355-368` 写缓存；`:58`、`:285` 两处落盘点读取缓存；`:398-414` 实时读数优先、不可用才回退缓存；`:266-299` `RunEvent::Exit` 兜底（`is_visible()` 守卫 + minimized 时用缓存） | 一致 |
| 3 | 托盘「检查更新」多语言版本号后缀 | `i18n.ts:41`（en-US `Check for Updates (v{{version}})`）、`:134`（zh-CN `检查更新 (当前版本: v{{version}})`）、`:227`（zh-Hant `檢查更新 (目前版本: v{{version}})`）；`TrayMenuView.tsx:7,257` 传入 `APP_VERSION` | 一致 |
| 3b | 托盘弹出窗尺寸自适应、不折行不遮挡 | `TrayMenuView.tsx:54-92` `measureMenu()` 取 `scrollWidth` 内在宽度并 clamp 到 300–500；`tray.rs:17-18` 后端同界，`:159-163` 权威再 clamp；`TrayMenuView.test.tsx:73/86/98` 覆盖变宽/上限/下限 | 一致 |
| 4 | 设置页右上角徽章显示真实版本 | `SettingsView.tsx:45-47` `<span ...>v{APP_VERSION}</span>`，保留青色药丸样式；原 `Tauri v2` 占位符全仓已无残留 | 一致 |
| 5 | 5 处版本号统一为 1.1.3 | `package.json:4`、`package-lock.json:2` 与 `:9`、`src-tauri/Cargo.toml:4`、`src-tauri/Cargo.lock:536`、`src-tauri/tauri.conf.json:4` 均为 `1.1.3`；前端常量 `src/version.ts:1` 亦为 `1.1.3`，构建产物 `dist/assets/index-*.js` 中已固化该值 | 一致 |
| 6 | 更新文案/键位三语同步 | 逐一核对 `clickToInstallUpdate`、`updateReadyInstallNow`、`retryInstallUpdate`、`installingUpdate`、`updateProgress`、`upToDate`、`checkingUpdates`、`mousePassthrough(+Hint)`、`autoInstallUpdates(+Hint)`、`menuMousePassthrough`、`menuAutoEdgeHide`：每个键在 en-US / zh-CN / zh-Hant 各出现 3 次，无缺语言 | 一致 |

v1.1.3 未发现遗漏项，也未发现与计划语义相反的实现。

## 三、v1.1.2 承诺项核实（100% 落地且无回退）

| 承诺 | 代码证据 | 判定 |
| --- | --- | --- |
| 鼠标穿透 `mousePassthrough`（默认 false、旧设置 serde 迁移） | `config.rs:59`（`#[serde(default)]`）、`:87` 默认 false；`config.rs:321` 旧设置用例断言默认关闭 | 一致 |
| 开启前要求托盘恢复入口就绪 | `commands.rs:416-422`：托盘缺失时直接报错拒绝开启 | 一致 |
| 设置页与托盘均可切换，托盘为主窗不可点击时的恢复入口 | `SettingsView.tsx:192-201`；`TrayMenuView.tsx:319-327`；`App.test.tsx:119`、`TrayMenuView.test.tsx:129` 覆盖 | 一致 |
| 补丁串行 + 失败回滚（原生状态与持久化一致） | `commands.rs:267` 持锁、`:339-358` 失败回滚字段并重存；`set_main_mouse_passthrough` 再校验托盘 | 一致 |
| 重启恢复失败时清除穿透偏好、保持可交互 | `lib.rs:180-194`：恢复失败即写回 `mouse_passthrough=false` | 一致 |
| 托盘状态行「发现 X 版，点击安装」+ 原位重试 | `TrayMenuView.tsx:256-277`（available → `clickToInstallUpdate`；error → `retryInstallUpdate`）；`TrayMenuView.test.tsx:138` 覆盖 | 一致 |
| 安装失败不再取走待安装对象 | `commands.rs:756-762`：prepared 安装失败回填；`:764-770` `pending_update` 保持，重试无需重新检查 | 一致 |
| 可选后台下载（默认关闭的自动安装） | `config.rs:43` 默认 false；`commands.rs:638-652` 检查后按偏好触发 `prepare_update`；`:872-906` 下载→二次校验偏好与版本→存内存并广播 `update_ready` | 一致 |
| 重复检查同版本不重复下载 | `commands.rs:697-703`（版本变化才清 prepared）、`:879-883`（同版本直接返回） | 一致 |
| 关闭自动安装即清除内存中的包 | `commands.rs:363-367` | 一致 |
| 正常退出时自动安装 | `lib.rs:297` → `commands.rs:908-916`；安装版 `install_verified_bytes(..., exiting=true)` → `restart_after_install(false)`（`commands.rs:816-819`），便携版带 `--no-restart`（`:859`） | 一致 |
| 便携版同时暂存主程序与新版助手 | `commands.rs:835-851`（`CodexUsageOverlay.exe.staged` + `CodexUsageUpdater.next.exe`）；`portable_updater.rs:49-87` 备份/替换/失败还原并回写助手 | 一致 |
| 手动安装维持重启与故障回滚 | 便携版 `--restart`（`commands.rs:859`）+ `install_verified_bytes` 失败即还原（`portable_updater.rs:54-57`、`:65-79`） | 一致 |
| 托盘左右键均打开菜单 | `tray.rs:31-39`：`MouseButton::Left \| MouseButton::Right` → `open_tray_menu_window` | 一致 |
| 收紧贴边细条可见底框、保留悬停感应区 | `d0ea637` 将边框/底色从 46×100 宿主移入 `::before` 伪元素（可见框 32×80，上下边 80×32，法向偏移 5px，`index.css:95-109`）；宿主仍为 46×100/100×46 且透明（`:72-94`），`OverlayView.tsx:178-191` 的 `onMouseEnter/onMouseLeave` 仍挂在该宿主上 | 一致 |
| 1.1.2 行为未被 v1.1.3 回退 | 贴边几何与状态机未被 1.1.3 触碰：`dock.rs:12-16` 仍为 16px 触发带 / 46×100 贴边窗 / 2px 内缩 / 400ms 收起；`dock`、`notify` 用例在 65/65 中全部保留并通过 | 无回退 |

## 四、`2026-09-28-update-window-transparency-edge-review.md` 的定位与交付核实

该文档是 **v1.1.1 截止的现状核对 + 下一步方案**，不是发布承诺清单。按三类分拣：

**A. 已在 1.1.2 / 1.1.3 实现（本版范围）**

| 文档条目 | 落地位置 |
| --- | --- |
| §5 全窗口鼠标穿透（含托盘恢复入口、重启恢复失败保护、三语与 README 更新） | `commands.rs:416-422`、`lib.rs:180-194`、`SettingsView.tsx:192-201`、`TrayMenuView.tsx:319-327` |
| §1 更新入口/失败重试、可选后台下载、正常退出自动安装、README 说明 | 见 §三各行；`README.md:28`、`README.en.md:28` |
| §6 设置面板默认尺寸放大、首次显式居中、用户移动/缩放后保存位置与尺寸、坏值回默认 | `tray.rs:10-15,274-280,398-414`、`config.rs:207-230`（v1.1.3 把文档建议的「高度+位置」扩展为宽高全量，符合 §6「按工作区夹紧」的意图） |
| §2 结论「设置窗口可保存逻辑像素位置与尺寸、按当前工作区夹紧」 | 同上 |

**B. 明确属现状分析、不构成本版承诺**

- §2 对主窗口尺寸为何不做持久化的分析（结论：主窗保持内容驱动尺寸、只持久化 `scalePercent`）—— 现状分析，v1.1.2/1.1.3 均未改动该语义。
- §3 透明度观感核对：数学映射与字段语义结论 + 建议先做实机色差比对、再决定是否引入「整体不透明度」或协调边框 alpha —— 现状分析 + 待验证建议，文档自身在「下一会话顺序」第 2 项列为待办。
- §4 贴边现状与 MeowpinPet 对照表 —— 现状核对；1.1.2 只兑现了「收紧可见底框」，其余为方案建议。

**C. 明确标记为未来版本演进规划（本版不承诺）**

- §4「推荐的悬浮窗方案」1–5：拖拽结束用可见内容边界 + 短边约 25%/24–48px 触发带、窄标签形态、**默认点击展开 + 明确收起**（替代当前 400ms 悬停收放）、位置/状态分离与贴边偏好独立化、分阶段验收。当前代码仍是 §4 表格「当前悬浮窗」一列的行为（`dock.rs:12-16`、`mouse_enter/mouse_leave`），即：**未实现，且文档未承诺在本版实现**。
- §3「新增整体不透明度语义 / 设置页即时预览 / 边框 alpha 协调」：未来方向。
- §1「后端持久保存发现版本与通知投递结果」：仍为内存态 `last_auto_notified_version`（`commands.rs:713-721`），文档将其列在「后续改进建议」，1.1.2 计划亦未纳入 —— 不构成遗漏。

**核查结论：不存在「文档写明当前版本要实现、但代码遗漏或反向实现」的条目。** 唯一需登记的偏离是实现细节层面的，见 §五(3)。

## 五、偏离与残余风险（不计为代码缺陷，但必须登记）

1. **1.1.3 §三-2「多场景实机核验」未执行**：拖拽拉伸后关闭重开是否精确复原、托盘文案是否截断、设置页徽章实机呈现，均依赖打包 GUI 与多显示器环境，本机不具备。代码层已逐项核对，但**运行时行为未经证实**。
2. **1.1.2 §「发布前验证」的真实签名包烟雾测试未执行**：NSIS / MSI / 便携版的升级、退出安装、提权、失败恢复仍待实机。文档已如实标注。
3. **实现细节偏离（改进型）**：1.1.3 计划 §2 写「读取当前逻辑坐标与逻辑尺寸并**异步**写入 JSON」，实现为在事件回调内**同步**写盘（`config.rs:232-240` 的 tmp+rename），并额外增加 `RunEvent::Exit` 兜底落盘（`lib.rs:266-299`，含 `is_visible()` 守卫）。同步写在退出路径上更可靠；兜底是 round 1 CR1-3 要求的产物。属对计划的正向偏离。
4. **几何恢复整链缺少自动化证据**：`tray::plan_settings_geometry / select_work_area / place_within_work_area` 等纯函数有 9 个单测，但生产接线 `apply_settings_geometry`（`tray.rs:232` 唯一调用点）与 `center_settings_window`（`tray.rs:274`）**无任何测试或变异探针触及**——把 `open_settings_window` 里的 `apply_settings_geometry` 调用改掉不会有测试失败。这是 round 4/5 对 `lib.rs` 捕获接线的同类问题在 `tray.rs` 上的残余，需实机核验才能覆盖。
5. **设置页版本徽章无前端断言**：1.1.3 §三-1 要求测试「覆盖版本号渲染及新文案」，现有断言只覆盖托盘一侧（`TrayMenuView.test.tsx:111,117`）；`SettingsView` 无测试文件，徽章的正确性目前仅由代码与构建产物核对支撑。
6. **README 未记述 1.1.3 的新用户可见行为**（设置窗口可自由缩放、几何跨进程记忆）：`README.md` / `README.en.md` 仍只覆盖穿透与更新。1.1.3 计划未把 README 列为交付项，故不计为契约遗漏，仅登记为文档覆盖建议。
7. **几何记录未带显式版本字段**：edge-review §2 曾建议「写入带版本的窗口布局记录，损坏/旧版时回默认值」；1.1.3 计划 §2 的结构体未含版本号，实现以 `#[serde(default)] scale_factor` + 加载期校验（`config.rs:207-230`）实现向后兼容与坏值回默认。建议未采纳进方案，不构成偏离。

## 六、审查线状态同步（本次一并登记）

- v1.1.3 第 5 轮独立复审（被审 `974f2f4`，基准 `4d58e94`）判定**通过、0 缺陷**：round 4 的 P3-1（捕获/读取接线不可证伪）已由新增 `AppState::for_test`（`commands.rs:47-75`）与读接线用例闭环，`lib.rs` 的 `capture_fills_the_cache_and_read_back_matches` / `capture_never_poisons_the_cache_with_an_unusable_reading` / `persist_falls_back_to_the_cache_through_the_read_wiring` / `persist_prefers_the_live_reading_over_a_non_empty_cache` 直接驱动生产接线；Rust 用例数 61 → 65，四道门禁全绿。
- 该轮按「无缺陷即不改文档、不提交」的约定未产出 handoff，因此此前 `INDEX.md` 无 round 5 行、`STATUS.md` 仍停留在 round 4 的 `Not passed — 1×P3` 陈旧结论。本次审计一并更正。

---

## 七、审计结论

- **计划契约合规：通过。** v1.1.3 计划的全部承诺项（宽度解禁与弹性布局、几何持久化与多屏安全恢复、最小化防覆盖与缓存兜底、托盘版本号、设置页徽章、全仓版本号 1.1.3）与 v1.1.2 计划的全部承诺项（穿透与托盘恢复、更新入口/重试/后台下载/退出安装、托盘左右键、细条底框收紧）**逐条在代码中落地，无遗漏、无反向实现、无对 1.1.2 行为的回退**。
- **行为评估文档定位：清晰。** 已实现项（穿透、设置窗几何、更新重试与退出安装）与「现状分析 / 待验证建议 / 未来演进规划」三类界线分明，未出现把未来规划误记为已交付、或把现版本承诺漏记的情况。
- **发布就绪：未达成（不属本次合规结论的判定范围）。** 计划自设的实机核验（1.1.3 §三-2、1.1.2 §发布前验证）与 `docs/stage0-verification.md` 的 stage 0 项仍未完成；`apply_settings_geometry` 生产接线与设置页徽章缺少自动化证据。上述均已如实登记于 §五，不做掩盖。
- 四道门禁在本机 `974f2f4` 全绿（46/46、构建通过、65/65、0 警告）；门禁全绿不构成实机通过。

审计员：独立合规审计员（Auditor）
审计方式：文档契约逐条 → 代码 `file:line` 取证 → 门禁本机复现 → 分类定性（已实现 / 现状分析 / 未来规划）
