# 悬浮窗"自己消失"排查：隐藏、销毁、空白，还是置顶失效

> 代码依据：v1.9.4（提交 `ce1467a`，文中行号均指该提交）。现场采样：本机安装的 v1.9.2（PID 28384，2026-11-14 22:30:44 启动）。v1.9.2 → v1.9.4 之间 `dock.rs` 只是把几何函数挪到 `geometry.rs`，与可见性相关的路径两版相同。
> 排查日期：2026-11-19（文中时间均为本机时间，UTC−5）
> 结论：**没有定时器、失焦或全屏检测会自动隐藏悬浮窗**，`hide()` 只来自托盘/菜单切换和启动时沿用的隐藏状态。但"看不见"不只有隐藏、遮挡、越界三类：主窗口收到 `WM_CLOSE` 会被直接销毁，WebView2 进程退出后透明窗口不再绘制任何内容。这两种情况托盘切换都救不回来，只能重启。这两条路径已在 v1.9.5-preview.1 修复（5.1、5.2），v1.9.4 及以前仍受影响。自由态越界和尺寸自适应裁边（5.3、5.4）已在 `CHANGELOG.md` 的 `[Unreleased]` 中修复，尚未发布。

## 摘要

用户反馈"悬浮窗会自己消失"，需要判定是哪一类：窗口被 `hide()`；窗口被销毁；窗口还在但内容没了；还是窗口还在，但被遮挡、置顶失效或落到工作区外。

1. **代码层**：`main` 窗口的 `hide()` 入口见 1.1，全部由用户动作或上次持久化的状态触发。另有两条不经过 `hide()` 的消失路径（1.2）：`main` 没有拦截关闭请求；没有处理 WebView2 进程退出。
2. **运行时层**：2026-11-19 19:43 现场采样为 `SHOWN`（第 2 节）。进程自 11-14 22:30 启动后没有重启；Windows 应用程序日志近 14 天没有 `codex-usage-overlay.exe` 或 `msedgewebview2.exe` 的崩溃/挂起记录（进程被结束不会留下这类记录）。
3. **探针坐标已修正**：探针最初没有声明 DPI 感知，读到的是被 Windows 按缩放比例虚拟化的坐标（2560×1440@125% 的主屏读成 2048×1152，悬浮窗 x=771 读成 617），和 `window-position.json` 的物理坐标对不上，还会在相邻显示器之间造出不存在的缝隙。现已改为物理像素，见第 4 节。

第 4 节的探针可以直接区分以上各类。

## 1. 窗口可见性的全部控制点

`main` 窗口在 `tauri.conf.json` 中为 `label: "main"`、`transparent: true`、`alwaysOnTop: true`、`skipTaskbar: true`、`visible: false`（初始隐藏）。

### 1.1 `hide()` / `show()` 入口

| # | 触发方式 | 代码位置 | 行为 | 是否持久化 |
|---|---|---|---|---|
| 1 | **托盘图标左键单击**（双击只算一次） | `src-tauri/src/tray.rs:362` `handle_tray_left_event` → `toggle_main_window`（`tray.rs:823`）→ `dock::toggle_overlay_window`（`dock.rs:853`） | 可见则隐藏；不可见则显示并 `set_focus()` | 是，写 `dock-state.json` 的 `hidden` |
| 2 | 右键托盘图标弹出的菜单「显示 / 隐藏悬浮窗」 | `src/components/TrayMenuView.tsx:297` → 命令 `toggle_overlay_window`（`commands/mod.rs:488`） | 同 1 | 是 |
| 3 | 悬浮窗自身的右键菜单「隐藏悬浮窗」 | `commands/mod.rs:468`（`show_overlay_menu`）→ `tray.rs:519` `handle_menu_action("toggle_overlay")` | 同 1；`mousePassthrough=true` 时右键菜单打不开 | 是 |
| 4 | 再次启动 exe（单实例回调） | `src-tauri/src/lib.rs:119`–`126` | 窗口不可见才 toggle（即显示），然后无条件 `set_focus()` | 是 |
| 5 | 启动时读到 `hidden: true` | `dock.rs:635`（`restore_startup`） | 启动即不显示；托盘创建失败时 `lib.rs:218` 强制显示 | 沿用上次 |
| 6 | `toggle_main_window` 拿不到 `AppState` 时的兜底 | `tray.rs:830`–`836` | 直接 `hide()` / `show()` | 否；`AppState` 在 `setup` 之前就已注册，正常运行走不到 |

补充说明：

- `lib.rs:96` 的 `WindowEvent::Focused` 分支只处理 `tray-menu` 窗口，**不会**在失焦时隐藏悬浮窗。
- `src-tauri/src/display.rs` 负责 provider 轮播与内容刷新，只读取 `is_visible()` / `is_minimized()`（`display.rs:268`–`276`、`display.rs:300`），不写窗口可见性。
- 全仓库检索 `hide()`、`set_visible`、`SW_HIDE`、`ShowWindow`、`destroy()`、`close()`、`minimize()`：作用于 `main` 的只有上表各项；其余 `hide()` 都作用于设置窗口（`lib.rs:73`）或托盘弹窗（`lib.rs:79`、`lib.rs:102`、`tray.rs:510`、`tray.rs:567`、`tray.rs:616`、`TrayMenuView.tsx:237`、`TrayMenuView.tsx:243`）。代码里没有任何地方主动销毁或最小化 `main`。
- `dock.rs:798` 的 `schedule_collapse` 是"贴边折叠成胶囊"，不是隐藏；只有停靠态才会触发，而进入停靠态需要 `autoEdgeHide=true`。本机 `autoEdgeHide=false`、`dock-state.json` 为 `{"hidden": false, "docked": false, "edge": null}`，不涉及。

#### 隐藏会跨重启保留

`dock::set_manual_hidden`（`dock.rs:344`）在隐藏/显示的同时调用 `config.save_dock_state` 落盘。因此一旦发生第 1–3 项隐藏，**下次启动也不会自动出现**，直到再切换一次。这解释了"消失后重启依然没有"的现象。

反过来，切换的显示分支会先经 `place_full_at_anchor` 把窗口放回保存的锚点，并 clamp 到现存显示器的工作区（`dock.rs:879`–`880`；锚点不在任何显示器上时，`geometry.rs:160` 退回当前显示器）。所以窗口漂到屏幕外时，**用托盘隐藏再显示一次就能拉回来**。

### 1.2 不经过 `hide()` 的两条消失路径

#### A. 主窗口收到关闭请求会被销毁（探针判定 `NO-WINDOW`）

`lib.rs` 的 `on_window_event` 只给设置窗口（`lib.rs:62`）和托盘弹窗（`lib.rs:75`）调用了 `api.prevent_close()`。`main` 没有对应分支，关闭请求落到 `_ => {}`。Tauri 2.12 的默认处理是：没有监听器阻止，就销毁窗口（`tauri-runtime-wry` 2.12.0 的 `on_close_requested` → `on_window_close`）。

销毁之后 `get_webview_window("main")` 返回 `None`，`toggle_overlay_window` 在 `dock.rs:854` 直接返回，**托盘左键、托盘菜单都没有任何反应**；代码里也没有重建 `main` 的地方，只能退出重开。设置窗口和托盘还在，所以进程不会退出。

会发出关闭请求的操作：

- 悬浮窗拿着键盘焦点时按 **Alt+F4**。悬浮窗没有标题栏，但每次从托盘把它显示出来，`dock.rs:886` 都会 `set_focus()`；再次启动 exe 时 `lib.rs:125` 也会。这时按 Alt+F4，关掉的就是悬浮窗本身。
- 脚本"温和地"关闭进程：不带 `/F` 的 `taskkill /IM codex-usage-overlay.exe`、PowerShell 的 `CloseMainWindow()` 等。它们只发 `WM_CLOSE`，设置窗口和托盘弹窗会拦下，`main` 不会。

以上依据本仓库代码与 Tauri 源码推断，未实测（实测会关掉正在使用的悬浮窗）。

**修复**：`main` 现在也调用 `prevent_close()`，关闭请求一律忽略，见 5.1。

#### B. WebView2 进程退出后窗口空白（探针判定 `WEBVIEW-GONE`）

悬浮窗的内容由 WebView2 渲染。现场枚举 `main` 的子窗口：`Chrome_WidgetWin_1`、`Chrome_RenderWidgetHostHWND` 属于 `msedgewebview2.exe` 的浏览器进程（PID 7788），`Intermediate D3D Window` 属于它的 GPU 进程（PID 28204）。这些进程被结束或崩溃时，属于它们的子窗口随之销毁。

代码里没有处理 WebView2 进程失败的逻辑（检索 `ProcessFailed` 无结果），`main` 又是透明窗口，于是只剩一个可见、置顶、但什么也不画的空壳。它的窗口属性和正常显示完全一样，托盘隐藏再显示也不会重建 WebView2，只能重启。

上级目录 `AGENTS.md` 第 3 条专门要求清理进程时排除悬浮窗的 `msedgewebview2.exe`，可见这在本机的自动化清理中是现实风险。以上同样是推断，未实测（实测需要结束悬浮窗的 WebView2 进程，正是该条规则禁止的操作）。

**修复**：每个窗口的 WebView2 都注册了 `ProcessFailed`，渲染进程出问题时原地重载，浏览器进程退出时整个应用自动重启，见 5.2。

## 2. 现场实测（2026-11-19 19:43）

采样对象：`codex-usage-overlay.exe` v1.9.2，PID 28384，2026-11-14 22:30:44 启动，连续运行近 5 天未重启。以下为修正后的探针输出，坐标为物理像素：

```
SHOWN: 正常显示
  pid=28384 hwnd=0x909DA rect=(771, 0, 1152, 92) size=(381, 92) in_work=100%
  visible=True minimized=False cloaked=0 topmost=True clickthrough=True webview_windows=3
  monitor rc=(0, 0, 2560, 1440) work=(0, 0, 2560, 1390) primary=True
  monitor rc=(5760, 208, 7680, 1288) work=(5760, 208, 7680, 1248) primary=False
  monitor rc=(2560, -128, 5760, 1672) work=(2560, -128, 5760, 1612) primary=False
  above: 无
```

窗口左上角 `(771, 0)` 与 `window-position.json` 的 `{"left":771.0,"top":0.0}` 一致。修正前的探针在同一台机器上读到 x=617、主屏 `(0,0,2048,1152)`、副屏 `(2560,-128,4693,1072)`：主屏按 125%、副屏按 150% 被缩小，主屏与副屏之间、副屏与第三块屏之间都出现了实际不存在的缝隙。

读取到的相关设置（`%LOCALAPPDATA%\CodexUsageOverlay\settings.json`）：

| 设置 | 值 | 与"消失"的关系 |
|---|---|---|
| `autoEdgeHide` | `false` | 不会自动折叠成胶囊 |
| `mousePassthrough` | `true` | 窗口带 `WS_EX_TRANSPARENT`，完全不接收鼠标，只能通过托盘操作 |
| `backgroundTransparencyPercent` | `80` | 背景 alpha 只有 0.2，压在颜色相近的窗口上时不容易看清 |
| `scalePercent` | `110` | 内容放大，窗口尺寸随之自适应 |
| `quotaCarouselEnabled` | `true`（间隔 5 秒） | 轮播切换 provider 时窗口宽度跟着变，见 5.4 |

`mousePassthrough=true` 时悬浮窗本身点不动，用户只能用托盘图标，而托盘**左键单击**就是显示/隐藏切换，最容易误触第 1 项隐藏。把它点出来的同时它还会拿到焦点，这时按 Alt+F4 就会触发 1.2 A。

## 3. 判定矩阵

| 观察到的现象 | 判定 | 机制与恢复 |
|---|---|---|
| 托盘左键点一下就回来；`dock-state.json` 为 `hidden: true` | `HIDDEN` | 1.1 的入口之一，非自动；再切换一次 |
| 托盘图标还在，但左键、托盘菜单都没反应 | `NO-WINDOW` | 1.2 A，主窗口已被销毁（v1.9.4 及以前）；只能退出重开 |
| 托盘切换没报错、探针也说窗口在，屏幕上却什么都没有 | `WEBVIEW-GONE` | 1.2 B，WebView2 进程已退出。v1.9.4 及以前只能退出重开；修复后会自动重载或重启，持续出现说明恢复没有生效（例如启动不到一分钟又退出） |
| 切到某个应用后看不见，退出该应用又出现 | `COVERED` | `WS_EX_TOPMOST` 只保证在非置顶窗口之上；置顶窗口之间按最近激活排序，别的置顶窗口和全屏独占应用都能压上来。探针会列出压在上面的窗口及其进程 |
| 同上，且探针显示 `topmost=False` | `NOT-TOPMOST` / `COVERED` | 置顶标志丢失，普通窗口也能盖住它 |
| 插拔显示器、改分辨率或缩放、远程桌面/串流之后不见了 | `OFFSCREEN` | 自由漂浮状态运行期不回界（5.3）；托盘隐藏再显示一次可拉回。修复后显示器或任务栏一变就自动拉回 |
| 放在屏幕右侧或底部附近，轮播切换后被裁掉一截 | `OFFSCREEN`（部分，探针给出落在工作区内的百分比） | 尺寸自适应只向右下扩张（5.4）。修复后变大时自动左移或上移，变小后回到锚点 |
| 切换虚拟桌面后不见了 | `CLOAKED` | DWM cloak 了窗口，`IsWindowVisible` 仍为 true |
| 按 Win+D 后不见，再按一次又出现 | `MINIMIZED` 或 `CLOAKED` | 以探针结果为准；不是程序隐藏 |
| 宽或高为 0 | `ZERO-SIZE` | 前端 `fitCapsuleSize` 拒绝 ≤0 的尺寸（`OverlayView.tsx:145`），目前没有已知路径；出现请保留日志 |

要点：被 cloak 的窗口 `IsWindowVisible` 仍为 true，只看可见性会把 `CLOAKED` 误判为正常，必须结合 `DwmGetWindowAttribute(DWMWA_CLOAKED)`。`WEBVIEW-GONE` 更隐蔽：窗口的所有属性都和正常显示一样，只能看它的 WebView2 子窗口还在不在。

## 4. 判定工具

`scripts/diagnose-overlay-visibility.py`，只用 Python 标准库（ctypes），无需安装依赖。在仓库根目录执行：

```powershell
# 采样一次，打印结论和明细
python scripts/diagnose-overlay-visibility.py

# 复现时持续监视，状态变化才输出一行；--log 以 UTF-8 追加写入文件
python scripts/diagnose-overlay-visibility.py --watch --log overlay-visibility.log
python scripts/diagnose-overlay-visibility.py --seconds 600 --log overlay-visibility.log
```

保存日志请用 `--log`，不要用 PowerShell 的 `>` 重定向：中文系统上 PowerShell 默认按 GBK（代码页 936）解码外部程序的输出，脚本输出的 UTF-8 中文会变成乱码。

单次采样先输出一行结论，再列明细：`pid`、`hwnd`、`rect`、`in_work`、`visible`、`minimized`、`cloaked`、`topmost`、`clickthrough`、`webview_windows`、各显示器矩形与工作区，以及 Z 序上方压着它的窗口和所属进程。监视模式每次状态变化输出一行，显示器布局变化时另起一行；状态里不含窗口矩形本身，轮播导致的宽度变化不会刷屏。

判定取值（按优先级从高到低）：

| 取值 | 含义 |
|---|---|
| `NO-PROCESS` | 进程没在运行（退出、崩溃、被结束，或正在自动更新重启） |
| `NO-WINDOW` | 进程还在，悬浮窗已被销毁，见 1.2 A |
| `HIDDEN` | 被 `hide()`，见 1.1 |
| `CLOAKED` | 被 DWM cloak（例如属于另一个虚拟桌面） |
| `MINIMIZED` | 被最小化 |
| `ZERO-SIZE` | 窗口宽或高为 0 |
| `OFFSCREEN` | 完全不在任何显示器上，或部分落在工作区之外（给出落在工作区内的百分比） |
| `WEBVIEW-GONE` | 没有属于 `msedgewebview2.exe` 的内容子窗口，见 1.2 B |
| `COVERED` | Z 序上方有可见窗口压着它（跳过被 cloak 的窗口和整体 alpha 为 0 的分层窗口） |
| `NOT-TOPMOST` | 没被压住，但已失去置顶 |
| `SHOWN` | 正常显示 |

实现要点：

- 启动时切换到 Per-Monitor DPI Aware V2，所有坐标为物理像素，可直接与 `window-position.json` 对比。
- 只认 `codex-usage-overlay.exe` 进程中类名为 `Tauri Window`、标题为 `Codex Usage Overlay` 的窗口。设置窗口和托盘弹窗同进程、同类名，不靠尺寸猜。
- 跨在两块相邻显示器上的窗口，按它与各工作区相交面积之和判断，不会被误判为越界。

## 5. 已确认的代码薄弱点

### 5.1 `main` 没有拦截关闭请求（已修复，v1.9.5-preview.1）

见 1.2 A。`ce1467a` 中 `lib.rs:60`–`115` 的 `on_window_event` 没有 `CloseRequested if window.label() == "main"` 分支，Alt+F4 或 `WM_CLOSE` 会永久销毁悬浮窗，直到重启。

修复：`on_window_event` 为 `main` 增加 `CloseRequested` 分支，只调用 `prevent_close()`。Alt+F4 和 `WM_CLOSE` 不再有任何效果，隐藏和退出仍走托盘与悬浮窗菜单。退出、自动更新和 portable updater 都不经过关闭请求，不受影响：`app.exit()` 走 `RunEvent::ExitRequested`，Tauri 更新插件用 `process::exit`，portable updater 只等待进程退出。

### 5.2 没有处理 WebView2 进程退出（已修复，v1.9.5-preview.1）

见 1.2 B。`ce1467a` 没有注册 `ProcessFailed`，WebView2 浏览器进程一旦退出，悬浮窗就一直空白，直到重启。

修复：新增 `src-tauri/src/webview_recovery.rs`，悬浮窗、设置窗口和托盘弹窗创建后都注册 `ProcessFailed`：

- **渲染进程退出或无响应**：原地 `Reload()`。无响应事件会每隔几秒重复一次，所以每个窗口 30 秒内最多重载一次。
- **浏览器进程退出**：三个窗口共用的 WebView2 全部失效，调用 `request_restart()` 重启整个应用，从落盘状态重建所有窗口。以下情况不重启：
  - 启动不到 60 秒就发生，避免 WebView2 运行时损坏时反复重启；
  - 应用正在退出，此时的进程结束属于正常关闭。
- **其它进程**（GPU、utility 等）WebView2 会自己拉起，不处理。

重启过程中跳过 `install_prepared_update_on_exit`，避免更新安装与重新启动的进程争抢 exe；开启自动检查时，重启后的应用会在启动时重新准备更新。重启会丢失只存在内存里的状态（例如待恢复的已暂停对话列表），与手动重启相同。

### 5.3 自由漂浮状态运行期不回界（已修复，未发布）

`ce1467a` 中的 `dock::keep_docked_in_work_area`（`dock.rs:891`）：

```rust
if info.hidden || state.dock.is_awaiting_resize() || state.dock.is_interacting() {
    return;
}
let settings = state.settings.lock().await.clone();
if let Some(edge) = info.edge.filter(|_| info.docked) {
    // 只有停靠态才重新定位
}
```

调用方有 `lib.rs:105`（`ScaleFactorChanged`）、`notify.rs:169`（每次读到用量之后）和 `commands/settings.rs:797`（几何相关设置变更之后），但**未停靠时函数体没有任何定位动作**。显示器拓扑、分辨率或 DPI 变化后（远程桌面与串流工具是高频场景），应用不会把窗口拉回当前工作区，窗口能否回来取决于 Windows 自己是否重排。

运行期只有两处会 clamp 自由态窗口：拖拽结束（`dock.rs:671` `drag_ended`）和托盘重新显示（`dock.rs:879`–`880`）。`mousePassthrough=true` 时无法拖拽，只剩后者。另外，规格 §3.1 T10 要求的"显示器/工作区变化"触发也没有实现：只有 DPI 变化（`ScaleFactorChanged`）和用量刷新会调用它，而用量刷新只在读取成功后才调用，断网时一直不会触发。

修复：

- 函数改名为 `keep_in_work_area`，自由态也处理：把窗口放回保存的锚点（`window-position.json`，即最后一次拖拽松手的位置；没有锚点时用默认的右上角位置），按当前尺寸 clamp 进锚点所在显示器的工作区。锚点所在显示器不在了就落到当前显示器，显示器接回后再回到原处。只移动窗口，不改写锚点（规格 §3.6）。位置没变时不调用 `set_position`。
- 主窗口的原生子类过程新增两条消息：`WM_DISPLAYCHANGE`（显示器增减或改分辨率）和带 `SPI_SETWORKAREA` 的 `WM_SETTINGCHANGE`（任务栏移动、改变大小或自动隐藏）。收到后等 500 ms 让布局稳定，再调用 `keep_in_work_area`。停靠态的小浮窗也因此能在这些变化后立即重新落位，补上规格 T10。

### 5.4 尺寸自适应只向右下扩张（已修复，未发布）

前端 `fitCapsuleSize`（`src/components/OverlayView.tsx:138`）只写尺寸、不写位置：

```ts
await getCurrentWindow().setSize(new LogicalSize(width * dprComp, height * dprComp));
```

窗口以左上角为锚点向右下扩展。左上角始终留在原处，所以这条路径不会让整个窗口消失；但窗口靠近屏幕右侧或底部时，内容变宽/变高会把右下部分推出工作区裁掉。开启轮播后，切换到内容更宽的 provider 就会触发。停靠态由 `dock_window_resized` → `dock::window_resized`（`dock.rs:935`）兜底重新定位，自由态在 `dock.rs:939` 直接返回。

修复：`fitCapsuleSize` 每次写完尺寸都调用 `dock_window_resized`（收起的小浮窗不会走到这里）。后端在自由态用 5.3 的同一套落位：放回锚点，按新尺寸 clamp 进工作区。所以贴着右边缘的胶囊变宽时右边缘保持贴边、整体左移，变窄后回到拖拽时的位置；变高时同理上移，不压到任务栏下面。拖拽中或右键菜单打开时不移动窗口，与停靠态的规则一致。

## 6. 后续修复

已随 v1.9.5-preview.1 发布：拦截 `main` 的关闭请求（5.1），WebView2 进程退出后自动恢复（5.2）。

已修复、尚未发布：自由态运行期回界（5.3），尺寸变化后保持在工作区内（5.4）。

以上四项都有单元测试和编译验证，但都还没有在真实窗口上验收：Alt+F4、结束 WebView2 进程、插拔显示器或移动任务栏、贴着右边缘时轮播变宽。

仍待处理：

1. **降低误触代价**：考虑把托盘左键语义从"切换"改为"唤出"，或在设置页显示当前隐藏状态。此项涉及交互变更，需产品确认。

## 附：复现与取证步骤

1. 常驻监视（在仓库根目录执行）：
   ```powershell
   python scripts/diagnose-overlay-visibility.py --watch --log overlay-visibility.log
   ```
2. 等悬浮窗"消失"后，看日志最后一行：
   - `HIDDEN` → 查 `%LOCALAPPDATA%\CodexUsageOverlay\dock-state.json` 的 `hidden` 是否为 `true`；托盘切换一次即可恢复。
   - `NO-WINDOW` → v1.9.4 及以前：回想消失前是否按过 Alt+F4，或有脚本关闭过进程；退出重开。修复后的版本不应再出现，出现请保留日志。
   - `WEBVIEW-GONE` → 查当时是否有脚本结束过 `msedgewebview2.exe`。修复后的版本会在几秒内自动重载或重启；一直停在 `WEBVIEW-GONE` 说明恢复没有生效，退出重开并保留日志。
   - `COVERED` → 日志会列出压在上面的窗口标题和所属进程。
   - `OFFSCREEN` → 对比日志里的 `rect`、显示器布局与 `window-position.json`（三者都是物理像素）。v1.9.5-preview.1 及以前用托盘隐藏再显示拉回；修复后的版本在显示器、任务栏变化或下一次用量刷新时自动拉回，持续 `OFFSCREEN` 请保留日志。
   - `CLOAKED` / `MINIMIZED` → 检查是否刚切换过虚拟桌面或按过 Win+D。
   - `NO-PROCESS` → 进程没了：看是否正在自动更新（`autoInstallUpdates=true`），或被结束、崩溃。
3. `window-position.json` 只在拖拽结束时更新（`dock.rs:696`、`dock.rs:734`）。v1.9.5-preview.1 及以前，窗口左上角与它不一致，说明窗口在运行期被挪动过（显示器变化、尺寸自适应或系统重排），优先看 5.3、5.4。修复后，自由态窗口的左上角应等于锚点按当前尺寸 clamp 进工作区后的位置（贴边时会偏离锚点），偏离这个位置才说明有别的东西挪动了它。
