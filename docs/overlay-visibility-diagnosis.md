# 悬浮窗"自己消失"排查：是隐藏还是置顶失效

> 适用版本：v1.9.4（提交 `ce1467a`）
> 排查日期：2026-11-19
> 结论：**代码里没有任何"自动隐藏悬浮窗"的路径**。真正 `hide()` 只来自主动切换；"看不见"的其余情形都是窗口还在、但被遮挡或落到工作区外。

## 摘要

用户反馈"悬浮窗会自己消失"，需要判定属于哪一类：窗口被 `hide()`，还是窗口仍在但置顶失效/被遮挡。

排查结果分两层：

1. **代码层**：`main` 窗口的全部隐藏入口只有 5 个，全部由用户动作或上次持久化状态触发，不存在定时器、失焦隐藏、全屏检测。
2. **运行时层**：2026-11-19 19:18 现场采样为 `SHOWN`——`visible=True`、`topmost=True`、`cloaked=0`、位于主屏工作区内、Z 序上方无重叠窗口；进程自 11-15 03:30 启动后未重启，Windows 事件日志无崩溃记录。

因此"消失"不是程序自己把窗口藏起来，而应从下面四类里定位：`HIDDEN` / `COVERED` / `OFFSCREEN` / `CLOAKED`。第 4 节给出零依赖判定工具。

## 1. 窗口可见性的全部控制点

`main` 窗口（`tauri.conf.json` 中 `label: "main"`，`visible: false` 初始隐藏）真正被 `hide()` 的位置只有：

| # | 触发方式 | 代码位置 | 行为 | 是否持久化 |
|---|---|---|---|---|
| 1 | **托盘图标左键单击/双击** | `src-tauri/src/tray.rs:362` → `toggle_main_window` → `dock::toggle_overlay_window` (`dock.rs:853`) | 可见则隐藏，不可见则显示 | 是，写 `dock-state.json` 的 `hidden` |
| 2 | 托盘菜单「显示/隐藏悬浮窗」 | `src-tauri/src/tray.rs:519` | 同 1 | 是 |
| 3 | 悬浮窗右键菜单同一项 | `src/components/TrayMenuView.tsx:297` | 同 1 | 是 |
| 4 | 再次启动 exe（单实例回调） | `src-tauri/src/lib.rs:119` | 窗口不可见才 `toggle`（即显示） | 是 |
| 5 | 启动时读到 `hidden: true` | `src-tauri/src/dock.rs:635`（`restore_startup`） | 启动即不显示 | 沿用上次 |

补充说明：

- `src-tauri/src/lib.rs:96` 的 `WindowEvent::Focused` 分支只处理 `tray-menu` 窗口（`label` 判断），**不会**在失焦时隐藏悬浮窗。
- `src-tauri/src/display.rs` 负责 provider 轮播与内容刷新，只读取 `is_visible()`，不写窗口可见性。
- 全仓库检索 `SW_HIDE`、`ShowWindow`、`set_visible(false)`、`.destroy()`、前端 `getCurrentWindow().hide()`：除 `TrayMenuView.tsx:237/243` 作用于托盘弹窗自身外，没有其它命中。
- `dock.rs:798` 的 `schedule_collapse` 是"贴边折叠成胶囊"，不是隐藏；且需要 `autoEdgeHide=true` 才会触发。本机 `autoEdgeHide=false`、`dock-state.json` 为 `{"hidden": false, "docked": false, "edge": null}`，不涉及。

### 关键细节：隐藏会跨重启保留

`dock::set_manual_hidden`（`dock.rs:344`）在隐藏/显示的同时调用 `config.save_dock_state(&persisted)` 落盘。因此一旦发生第 1–3 类隐藏，**下次启动也不会自动出现**，直到再切换一次。这解释了"消失后重启依然没有"的现象。

## 2. 现场实测（2026-11-19 19:18）

采样对象：`codex-usage-overlay.exe`，PID 28384，进程创建时间 2026-11-15 03:30:44（连续运行 4 天，无重启、无崩溃）。

```
SHOWN  rect=(617,0)-(1054,74)  visible=True  topmost=True  clickthrough=True
cloaked=0   on_monitor=True   inside_work=True   Z 序上方：无重叠窗口
monitor rc=(0,0,2048,1152)      work=(0,0,2048,1112)      primary=True
monitor rc=(5760,208,7680,1288) work=(5760,208,7680,1248) primary=False
monitor rc=(2560,-128,4693,1072) work=(2560,-128,4693,1032) primary=False
```

读取到的相关设置（`%LOCALAPPDATA%\CodexUsageOverlay\settings.json`）：

| 设置 | 值 | 与"消失"的关系 |
|---|---|---|
| `autoEdgeHide` | `false` | 不会自动折叠成胶囊 |
| `mousePassthrough` | `true` | 窗口 `WS_EX_TRANSPARENT`，完全不接收鼠标，只能靠托盘操作 |
| `backgroundTransparencyPercent` | `80` | 背景 alpha 仅 0.2，深色文字压在白底窗口上对比度低 |
| `scalePercent` | `110` | 内容放大，窗口尺寸随之自适应 |

`mousePassthrough=true` 与第 1 类隐藏叠加是最容易复现"自己消失"的组合：悬浮窗本身点不动，用户改用托盘图标，而托盘**左键单击**就是显示/隐藏切换。

## 3. 判定矩阵

| 观察到的现象 | 判定 | 机制 |
|---|---|---|
| 托盘图标点一下就回来；`dock-state.json` 变为 `hidden: true` | `HIDDEN` | 第 1–5 类隐藏入口之一，非自动 |
| 切到某个应用后看不见，退出该应用又出现 | `COVERED` | `WS_EX_TOPMOST` 只保证在非置顶窗口之上；置顶组内部按最近激活排序，新的置顶窗、全屏应用、UAC 安全桌面都会压上来 |
| 插拔显示器、改分辨率、远程桌面（AnyDesk / UU远程 / GameViewer）之后不见了 | `OFFSCREEN` | 自由漂浮状态没有回界逻辑，见 5.1 |
| 按了 Win+D 或切换虚拟桌面 | `CLOAKED` | DWM 把窗口 cloaked，`IsWindowVisible` 仍为 true |
| 内容没变但窗口越来越窄/矮，最后点不到 | `ZERO-SIZE` | 尺寸自适应异常，见 5.2 |

要点：`IsWindowVisible` 无法区分 `HIDDEN` 与 `CLOAKED`（显示桌面时窗口仍报告可见），必须结合 `DwmGetWindowAttribute(DWMWA_CLOAKED)` 才能判定。

## 4. 判定工具

`scripts/diagnose-overlay-visibility.py`，仅用 Python 标准库（ctypes），无需安装依赖。

```bash
# 采样一次，直接打印结论
python scripts/diagnose-overlay-visibility.py

# 复现时持续监视，状态变化才输出一行（可重定向到文件）
python scripts/diagnose-overlay-visibility.py --watch
python scripts/diagnose-overlay-visibility.py --watch --seconds 600
```

输出为首行结论 + 明细（`hwnd`、`rect`、`visible`、`topmost`、`clickthrough`、`cloaked`、显示器工作区、Z 序上方遮挡窗口）。判定取值：

| 取值 | 含义 |
|---|---|
| `SHOWN` | 正常显示 |
| `HIDDEN` | 被 `hide()`，见第 1 节 |
| `CLOAKED` | DWM 隐藏（显示桌面 / 虚拟桌面 / 锁屏） |
| `MINIMIZED` | 被最小化 |
| `ZERO-SIZE` | 窗口宽或高为 0 |
| `OFFSCREEN` | 坐标不落在任何显示器上，或落在工作区之外 |
| `COVERED` | 可见且置顶，但 Z 序上方有窗口压着 |
| `NO-WINDOW` | 进程未运行或窗口已销毁 |

工具通过进程名 `codex-usage-overlay.exe` + `Tauri Window` 类名定位窗口，优先匹配标题含 `Usage Overlay`，回退到同进程中宽度小于 1000 的窗口，不会误判设置窗口。

## 5. 已确认的代码薄弱点（尚未修复）

### 5.1 自由漂浮状态缺少工作区回界

`dock::keep_docked_in_work_area`（`dock.rs:891`）在开头就直接返回：

```rust
if info.hidden || state.dock.is_awaiting_resize() || state.dock.is_interacting() { return; }
let settings = state.settings.lock().await.clone();
if let Some(edge) = info.edge.filter(|_| info.docked) { /* 只有停靠态才重新定位 */ }
```

`lib.rs:105` 的 `ScaleFactorChanged` 分支与 `notify.rs:169` 都会调用它，但**未停靠时函数体没有任何定位动作**。显示器拓扑、分辨率、DPI 变化后（远程桌面与串流工具属于高频场景），窗口坐标不会被 clamp 回当前工作区，可能留在已不存在的坐标上。

`dock::restore_startup` 只在启动时通过 `place_full_at_anchor`（`dock.rs:1055`）做一次 `clamp_position`；运行期没有等价保护。

### 5.2 尺寸自适应只向右下扩张

前端 `fitCapsuleSize`（`src/components/OverlayView.tsx:138`）只写尺寸、不写位置：

```ts
await getCurrentWindow().setSize(new LogicalSize(width * dprComp, height * dprComp));
```

窗口以左上角为锚点向右下扩展。当悬浮窗贴近屏幕右侧或底部时，内容变宽/变高会把窗口推出工作区，出现被裁切或整窗不可见。停靠态由 `dock_window_resized` 兜底（`dock.rs:935`），自由态没有。

## 6. 建议的后续修复

1. **自由态回界**：在 `keep_docked_in_work_area` 中为 `!docked && !hidden` 增加分支，用 `work_area_for_point` + `clamp_position` 把窗口拉回当前显示器工作区；同时处理 `place_full_at_anchor` 因显示器缺失返回 `Err` 的情况（现在是静默失败，窗口停在原位）。
2. **尺寸变化后保持可见**：`fitCapsuleSize` 写完尺寸后，若窗口越过工作区边界则调用一次后端定位命令（可复用 `dock_window_resized`，扩展成自由态也生效）。
3. **降低误触代价**：考虑把托盘左键语义从"切换"改为"唤出并聚焦"，或为隐藏增加可发现的恢复入口（例如设置页显示当前隐藏状态）。此项涉及交互变更，需产品确认。

## 附：复现与取证步骤

1. 常驻监视：
   ```bash
   python scripts/diagnose-overlay-visibility.py --watch > overlay-visibility.log
   ```
2. 等到悬浮窗"消失"，观察日志最后一行：
   - `HIDDEN` → 查 `%LOCALAPPDATA%\CodexUsageOverlay\dock-state.json` 的 `hidden` 是否变为 `true`；
   - `COVERED` → 日志会列出压在上面的窗口标题与矩形；
   - `OFFSCREEN` → 对比 `rect` 与列出的 `work` 区域；
   - `CLOAKED` → 检查是否刚用过 Win+D / 虚拟桌面。
3. `dock-state.json`、`window-position.json` 与实际 `rect` 三者不一致时，优先怀疑 5.1 的坐标漂移。
