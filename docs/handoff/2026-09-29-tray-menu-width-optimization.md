# 需求与技术方案：托盘宽度收窄、点击响应加固、跨屏归属回弹、小窗百分比刻度与设置界面双栏重构

日期：2026-09-29  
状态：已定稿 / 方案审查中（Finalized / Dispatched to WorkBuddy for Design Review）  
关联版本：v1.2.0 版本规划方案（共五大优化模块）  
说明：方案经用户讨论定稿确认，由 WorkBuddy 执行方案审查（上限 3 轮），收敛后进入 Mode C 执行，最终发布 v1.2.0。

---

## 模块一：系统托盘菜单自适应宽度算法收窄（紧凑回归）

### 1.1 问题背景与实机表现
- **现状**：v1.1.3 为防止在“检查更新”后追加版本号文案导致折行，引入了基于 `scrollWidth` 测量与 `layout_tray_menu` 的动态宽度自适应机制（范围 300~500px）。
- **实机表现**：在常见的桌面分辨率、系统默认字体与 DPI 缩放下，追加版本号后的完整文字（如 `检查更新 (当前版本: v1.1.3)`）在原版紧凑宽度下完全能够单行容纳。当前算法计算出的弹窗宽度比原版明显膨胀变大，产生了过多不必要的水平空白。
- **设计定调（用户意图）**：
  - **保留自适应算法**：不直接退回到写死宽度的旧方案，保留自适应骨架以防御极端大字体、系统高 DPI 缩放或长文本带来的物理遮挡风险；
  - **收窄多余预留**：核算并收窄算法中累加的多余安全余量，消除虚高膨胀，使菜单在常规状态下紧凑贴合，仅在必要时才做最小幅度微调。

### 1.2 算法优化方案
1. **消除内边距双重累加**：
   - 现存问题：`.tray-menu-item` 本身已有左右各 10px 内边距（合计 20px），测量后外层又叠加了 `Math.ceil(intrinsicWidth + 20)`，且 `content.scrollWidth` 在初始渲染时容易受容器自身宽度影响，导致水平余量被多重放大；
   - 优化：直接测量菜单项内部文本节点的真实内联宽度（Inline text width），或扣除重复计算的内边距，实现真实净文字宽度度量。
2. **紧凑基准线收紧**：
   - 设定紧凑基准下限（如紧凑贴合的 280~300px 范围）；
   - 仅在文字净宽度真实突破紧凑内容区边界时，才按实际超出量做贴合撑开，不再粗暴预留大额空白安全垫。
3. **保留单行排版契约**：
   - 继续保留 CSS 的 `.tray-menu-item { white-space: nowrap; }`，杜绝任何难看的折行；
   - 达到“常规场景精致小巧、极端长文案安全贴合”的平衡。

---

## 模块二：鼠标穿透开启后托盘图标点击偶发无反应排查与加固

### 2.1 根因分析（Root Cause）
1. **核心诱因：Windows 前台焦点锁定与瞬间失焦误杀（高发于穿透开启状态）**：
   - 未穿透时，用户频繁点击主悬浮窗，程序常处于 Windows 前台进程状态；
   - 穿透开启后，主悬浮窗完全忽略鼠标，用户操作的焦点完全属于第三方应用（浏览器、IDE、游戏等），本程序处于纯后台无焦点状态；
   - 用户点击托盘图标触发 `window.show(); window.set_focus();` 时，受 Windows Foreground Activation Lock（前台激活锁定）限制，后台进程无法随意抢夺前台焦点；
   - `lib.rs:88` 的 `tauri::WindowEvent::Focused(false)` 监听：
     ```rust
     tauri::WindowEvent::Focused(false) if window.label() == "tray-menu" => {
         let _ = window.hide();
     }
     ```
     在 Windows 点击切换的瞬间极易收到 transient killfocus 信号，导致菜单在 show 出的几毫秒内被立即 hide 隐藏，肉眼表现为“点击毫无反应”。
2. **代码级硬缺陷：`tray.rect()` 报错导致整个弹出流程被 `?` 直接中断**：
   - `src-tauri/src/tray.rs:123`：
     ```rust
     let rect = tray.rect().map_err(|e| e.to_string())?;
     ```
   - Windows 原生 API `Shell_NotifyIconGetRect` 在图标位于折叠区（`^` 向上箭头内）、任务栏重绘或快速连点时经常返回错误；
   - 虽然后续第 126-147 行写有针对未获取到位置时的屏幕右下角 fallback 降级分支，但由于第 123 行使用了 `?` 提前退出，一旦报错整个调用直接中止，**根本无法进入 fallback 分支，也从未调用 `window.show()`**。
3. **事件捕获缺陷：快速连点或双击被静默丢弃**：
   - `src-tauri/src/tray.rs:31-39` 仅匹配了 `TrayIconEvent::Click { button_state: MouseButtonState::Up, .. }`；
   - 穿透开启后用户急于关闭穿透，常伴随快速连击或双击，500ms 内的后续点击会被操作系统识别为 `TrayIconEvent::DoubleClick`；
   - 由于未匹配 `DoubleClick`，后续点击被静默丢弃，导致用户主观感受“狂点托盘没反应”。
4. **异步测算先 `hide()` 的竞态空档**：
   - 每次点击第一步先调用 `window.hide()`，再通过 IPC 触发前端测量与反向 invoke；在 IPC 和渲染这数十毫秒的空档内，若发生连续点击，会导致前后两代 generation 冲突互斥，窗口停留在隐藏状态。

### 2.2 响应性加固方案
1. **加固 `tray.rect()` 容错降级（彻底消除致命短路）**：
   - 改为：
     ```rust
     let rect = tray.rect().ok().flatten();
     ```
   - 确保即便 Windows API 报错，也绝不中断 layout 与 show 流程，平滑降级至屏幕右下角安全展示。
2. **事件监听纳入 `DoubleClick`**：
   - 在 `on_tray_icon_event` 中，将 `TrayIconEvent::DoubleClick` 视为与 `Click` 相同的弹出操作，彻底解决快速点击与双击吞键问题。
3. **增加失焦误杀保护期（防瞬时焦点抖动）**：
   - 托盘菜单在刚刚 show 出后的前 150~200ms 内设立瞬时保护期，忽略来自操作系统的瞬时 `Focused(false)`，等待前台激活平稳后才允许因用户点击外部而自然关闭。
4. **支持已打开状态下的点击关闭（Toggle 语义）**：
   - 点击托盘时若检测到托盘菜单当前已处于可见状态，则执行关闭收起，提供符合 Windows 习惯的开关反馈。

---

## 模块三：悬浮窗跨屏拖拽归属、接缝通道与越界回弹贴边联动

### 3.1 现状与问题分析
1. **直接拖拽跨屏体验受限与归属判定缺失**：
   - 用户多显示器时，需要能**直接将浮窗从一台显示器平滑拖拽至另一台显示器**；
   - 现有机制在拖拽松手时仅依据单点 `(cx, cy)` 计算工作区，在多显示器排列（尤其是分辨率不同、DPI 混合缩放或上下错位）时，两屏接缝处极易发生宿主屏幕归属漂移或判断失效；
   - 用户明确判定标准：**在两块屏幕之间拖拽超过一半（>50% 面积）松手，必须确切归属到另一边的显示器**。
2. **拖拽允许超出屏幕，但松手未执行边界约束**：
   - 在 Windows 下拖动无边框窗口时，用户容易将悬浮窗部分或大半拖出屏幕可视边界；
   - 当前 `src-tauri/src/dock.rs::drag_ended` 在用户松手时，直接把当前物理坐标 `position` 作为锚点落盘，未对窗口调用 clamp 重新定位，导致浮窗被留在屏幕外。
3. **边缘检测算法无法处理“拖出屏幕外”的情形**：
   - 当前 `detect_edge` 算法（`dock.rs:397-417`）设定了 `threshold = 16px`。当用户把窗口向外拖拽超过 16 像素时（例如左侧拖出屏幕 30px，`distance = -30`），由于 `|-30| > 16`，算法判定**未命中边缘**；
   - 导致即使用户开启了“自动贴边隐藏”，只要拖出屏幕稍微多了几像素，不仅不会触发贴边折叠，反而被留在屏幕外。
4. **内部接缝与物理外边缘未解耦（防误贴边关键）**：
   - 若简单将“屏幕工作区四边”全部当作贴边判定边，当用户把浮窗拖动至两屏交界处时，内部接缝会被错误判定为“屏幕边缘”，导致在两块屏幕正中央的缝隙上悬空折叠成小胶囊，阻断鼠标在两屏间的自然通行。

### 3.2 优化与联动方案

#### 1. 拖拽过程与归属仲裁（>50% 面积跨屏归属法则）
- **拖拽过程全局自由漫游**：
  - 拖拽中不设置任何人造屏障，悬浮窗随鼠标在所有显示器组成的 Windows 联合虚拟桌面上自由平滑移动；
  - 允许用户直接将浮窗从当前显示器拖移到任意其他显示器。
- **松手时多屏相交面积仲裁（`drag_ended`）**：
  - 遍历所有可用显示器工作区 `work_area`，计算悬浮窗物理矩形与各工作区的相交面积（`intersection_area`）；
  - **情形 A（完全拖入目标屏）**：重叠面积占比 100%，宿主屏幕无缝切换至目标显示器；
  - **情形 B（跨在两屏接缝处）**：
    - 若进入目标屏幕 B 的面积 **> 50%**：归属判定为**目标显示器 B**，浮窗宿主切换为 B，并平滑吸入 B 的工作区边缘内（避免浮窗跨在物理黑边缝隙上被割裂撕扯），更新 B 对应的 DPI 缩放与坐标；
    - 若进入目标屏幕 B 的面积 **<= 50%**（即大半仍留在原屏幕 A）：判定归属保留在**原显示器 A**，浮窗平滑弹回吸入 A 的工作区边缘内。

#### 2. 内部接缝（Inter-Monitor Seam）与物理外边界（Outer Bezel）解耦
- **边缘属性自动识别**：
  - 对目标显示器的 4 个边缘（左、右、上、下）分别进行空间拓扑探测：
    - 若某个边缘在空间上紧邻另一台可用显示器的工作区，则该边缘定性为 **“内部跨屏通道（Internal Seam）”**；
    - 若某个边缘外部为空白桌面虚空，则定性为 **“物理外边界（Physical Outer Bezel）”**。
- **防误折叠契约**：
  - **内部接缝严禁触发贴边折叠**：跨屏接缝仅用于屏幕切换与吸入，即便开启了 `auto_edge_hide`，也绝不在内部接缝处收缩为 pill，确保多屏视觉连贯与光标通行无阻；
  - **物理外边界正常响应贴边**：只有接触或拖出物理外边界时，才根据 `auto_edge_hide` 设置执行贴边折叠或外框回弹。

#### 3. 松手时行为状态矩阵（State Matrix）

| 拖拽松手落点 | 自动贴边开启 (`auto_edge_hide: true`) | 自动贴边关闭 (`auto_edge_hide: false`) |
| :--- | :--- | :--- |
| **完全位于单屏内部** | 保持当前位置，更新保存锚点 | 保持当前位置，更新保存锚点 |
| **接触或拖出物理外边界** (`dist <= 16px` 或 `越界`) | 命中对应外边缘，立即收缩为贴边细条（`apply_pill`） | 平滑弹回工作区内边界紧贴（`clamp_position`），保持展开状态 |
| **两屏接缝处（>50% 进入目标屏 B）** | 归属于目标屏 B，完整滑入屏 B 接缝内紧贴，**不折叠**（保持展开） | 归属于目标屏 B，完整滑入屏 B 接缝内紧贴，**不折叠**（保持展开） |
| **两屏接缝处（<=50% 留在原屏 A）** | 归属于原屏 A，完整滑回屏 A 接缝内紧贴，**不折叠**（保持展开） | 归属于原屏 A，完整滑回屏 A 接缝内紧贴，**不折叠**（保持展开） |


### 3.3 核心算法设计与伪代码规范

#### 1. 矩形交集与跨屏归属判定 (`select_monitor_by_overlap`)
```rust
/// 计算窗口物理矩形与各显示器工作区的交集面积，按最大面积（>50% 多数原则）确定宿主显示器
pub fn select_monitor_by_overlap(
    win: PhysicalRect,
    monitors: &[(PhysicalRect, f64)],
    fallback: usize,
) -> (PhysicalRect, f64) {
    let mut best_monitor = None;
    let mut max_area = 0i64;

    for (work, scale) in monitors {
        let ix1 = win.x.max(work.x) as i64;
        let iy1 = win.y.max(work.y) as i64;
        let ix2 = (win.x + win.width as i32).min(work.x + work.width as i32) as i64;
        let iy2 = (win.y + win.height as i32).min(work.y + work.height as i32) as i64;
        let overlap_w = (ix2 - ix1).max(0);
        let overlap_h = (iy2 - iy1).max(0);
        let area = overlap_w * overlap_h;

        if area > max_area {
            max_area = area;
            best_monitor = Some((*work, *scale));
        }
    }

    best_monitor.unwrap_or_else(|| {
        monitors.get(fallback).copied().unwrap_or((win, 1.0))
    })
}
```

#### 2. 接缝通道与物理外边界识别 (`is_external_boundary`)
```rust
/// 检查目标显示器指定边缘是否为物理外边界（若与邻近屏幕相接则为 Internal Seam，不触发贴边）
pub fn is_external_boundary(
    work: PhysicalRect,
    edge: Edge,
    all_monitors: &[(PhysicalRect, f64)],
) -> bool {
    let tolerance = 4; // 允许少量对齐容差像素
    for (other, _) in all_monitors {
        if *other == work {
            continue;
        }
        match edge {
            Edge::Left => {
                // 若左侧紧邻另一显示器的右边缘，且在 Y 轴上有重叠通道
                let adjacent_x = (work.x - (other.x + other.width as i32)).abs() <= tolerance;
                let overlap_y = work.y < other.y + other.height as i32 && work.y + work.height as i32 > other.y;
                if adjacent_x && overlap_y { return false; }
            }
            Edge::Right => {
                let adjacent_x = ((work.x + work.width as i32) - other.x).abs() <= tolerance;
                let overlap_y = work.y < other.y + other.height as i32 && work.y + work.height as i32 > other.y;
                if adjacent_x && overlap_y { return false; }
            }
            Edge::Top => {
                let adjacent_y = (work.y - (other.y + other.height as i32)).abs() <= tolerance;
                let overlap_x = work.x < other.x + other.width as i32 && work.x + work.width as i32 > other.x;
                if adjacent_y && overlap_x { return false; }
            }
            Edge::Bottom => {
                let adjacent_y = ((work.y + work.height as i32) - other.y).abs() <= tolerance;
                let overlap_x = work.x < other.x + other.width as i32 && work.x + work.width as i32 > other.x;
                if adjacent_y && overlap_x { return false; }
            }
        }
    }
    true
}
```

#### 3. 越界增强边缘探测与安全回弹 (`detect_edge_multi_monitor` & `drag_ended`)
```rust
/// 结合外边界识别的边缘命中判断：支持 distance <= 0（拖拽出界）仍精准吸附到对应外边缘
pub fn detect_edge_multi_monitor(
    win: PhysicalRect,
    work: PhysicalRect,
    scale_factor: f64,
    all_monitors: &[(PhysicalRect, f64)],
) -> Option<Edge> {
    let threshold = (SNAP_MARGIN_LOGICAL * scale_factor.max(0.1)).round() as i64;
    let wx = work.x as i64;
    let wy = work.y as i64;
    let wr = wx + work.width as i64;
    let wb = wy + work.height as i64;
    let x = win.x as i64;
    let y = win.y as i64;
    let right = x + win.width as i64;
    let bottom = y + win.height as i64;

    [
        (x - wx, Edge::Left),
        (wr - right, Edge::Right),
        (y - wy, Edge::Top),
        (wb - bottom, Edge::Bottom),
    ]
    .into_iter()
    // 只有物理外边界才参与贴边折叠判定
    .filter(|(_, edge)| is_external_boundary(work, *edge, all_monitors))
    // 距离在贴边阈值内，或者已经越界拖出屏幕外（distance <= 0）
    .filter(|(distance, _)| *distance <= threshold)
    .min_by_key(|(distance, _)| distance.abs())
    .map(|(_, edge)| edge)
}
```

---

## 模块四：贴边小窗百分比刻度格子（推算百分比）

### 4.1 需求背景与用户痛点
- **现状**：贴边隐藏收缩为细条小窗（pill 胶囊）时，内部两条细进度条（5小时与每周）仅通过纯色柱体展示余量。在无数字读数的紧凑小窗状态下，用户难以一眼准确推算出当前额度百分比（如究竟是 30%、40% 还是 50%）。
- **用户需求**：
  1. 在贴边小窗的两个进度条上，均匀加上黑色分割细线，将进度条平均分成十个等分格子（每个格子代表 10% 额度），方便用户仅凭格数即可精准推算百分比；
  2. 在设置界面中加入**“开启百分比格子”**（`showPercentageGrid`）选项开关；
  3. 排版顺序明确：放置在**“开启贴边隐藏”选项下方**。

### 4.2 视觉与渲染方案
1. **刻度等分几何规范**：
   - 胶囊小窗进度条轨道（`.overlay-pill-track`）高度为 72px；
   - 将 0%~100% 均分成 10 个格子，内部设置 9 条均匀水平刻度分割线，分别位于 `10%, 20%, 30%, 40%, 50%, 60%, 70%, 80%, 90%` 的垂直百分比高度；
   - 刻度线外观：高度为 1px 的细黑线（`rgba(0, 0, 0, 0.45)` 在浅色高饱和的绿/黄/红填充条以及底槽上具有极佳辨识度，深色模式下清晰自然）；
   - 刻度线层级：位于进度条填充（`.overlay-pill-fill`）的上层覆盖，指针/填充色上涨时直接穿过格线，用户一眼即可数出剩余“满格数 + 半格”；
   - 旋转自适应：在 Top / Bottom 边缘贴边时，小窗整体旋转 90deg（`.overlay-pill-rotated`），刻度线随 DOM 树自然跟随旋转，无需额外计算。
2. **配置契约与持久化**：
   - `OverlaySettings` 新增字段 `showPercentageGrid: boolean`（默认 `false`，由用户按需开启）；
   - 前端 `src/types.ts`、后端 `src-tauri/src/config.rs` 同步增加定义与 serde 默认值。
   - `src/i18n.ts` 同步增加双语翻译：
     - 中文：`showPercentageGrid: '开启百分比格子'`
     - 英文：`showPercentageGrid: 'Show Percentage Grid'`

---

## 模块五：设置界面双栏布局重构（宽度乘二、一屏尽览）

### 5.1 现状痛点与重设定调
- **现状痛点**：当前设置界面为单列垂直排布，默认尺寸为 480×660。随着功能增加（透明度、缩放、刷新间隔、自动更新、贴边隐藏、鼠标穿透、5小时重置提示、每周重置提示、自定义声音等），纵向内容过长，导致必须滚动窗口才能浏览下半部分的通知和音频选项，操作繁琐且缺乏整体掌控感。
- **设计定调（用户意图）**：
  - **宽度乘二（480px → 960px）**，重构为专业、工整的**双栏并排设计（Two-Column Layout）**；
  - **默认一屏幕展示所有选项**：彻底消除默认尺寸下的垂直滚动条，所有核心配置项在打开设置窗口的瞬间尽收眼底。

### 5.2 窗口尺寸与约束规范
- **默认尺寸**：
  - 宽度从 480px 乘二增加至 **960px**（`SETTINGS_DEFAULT_WIDTH = 960.0`）；
  - 默认高度设定为 **600px ~ 620px**（`SETTINGS_DEFAULT_HEIGHT = 600.0`）；
- **窗口约束调整**：
  - `minWidth`: 从 380px 调整为 **760px**（保证双栏并排时不发生空间挤压换行）；
  - `minHeight`: 从 400px 调整为 **500px**；
  - `maxWidth`: 保持 **1600px**，`maxHeight`: 保持 **1600px**；
  - `src-tauri/tauri.conf.json` 与 `src-tauri/src/tray.rs` 同步更新默认与极值约束。

### 5.3 双栏功能分区架构
- **顶部 Header（全宽跨栏）**：
  - 窗口设置标题 (`t('windowSettings')`) 与版本标识徽章 (`v1.1.3`)，下方贯穿式分割线；
- **核心内容区（2-Column Grid 并排）**：
  - **左栏（第一栏：浮窗外观、尺寸与交互控制）**：
    1. 布局方式选择（`overlayLayout`：分组胶囊 / 紧凑堆叠）
    2. 缩放比例滑块（`scalePercent`：100% ~ 250%）
    3. 背景透明度滑块（`backgroundTransparencyPercent`：0% ~ 80%）
    4. 显示额度余额开关（`showCredits`）
    5. 开启贴边隐藏开关（`autoEdgeHide`）
    6. **开启百分比格子开关（`showPercentageGrid`）** —— 严格放置在贴边隐藏正下方！
    7. 鼠标穿透模式开关（`mousePassthrough`）+ 穿透解除快捷键提示
    8. 底部操作说明提示文案（`t('hint')`）
  - **右栏（第二栏：系统集成、更新协同与重置通知警报）**：
    1. 数据刷新频率下拉菜单（`refreshIntervalSeconds`：30s / 60s / 2m / 5m）
    2. 开机自启动开关（`autoStart`，仅安装版显示）
    3. 自动检查更新开关（`autoCheckUpdates`）
    4. 自动下载并安装更新开关（`autoInstallUpdates`）+ 更新说明
    5. 重置提醒设置区（`notificationSection`）：
       - 5小时额度重置提醒（开关 + 声音模式 + 自定义音频文件选择器）
       - 每周额度重置提醒（开关 + 声音模式 + 自定义音频文件选择器）
- **底部 Footer（全宽跨栏）**：
  - 贯穿式分割线与居中反馈邮箱 (`septwind@agent.qq.com`)。
- **一屏尽览效果核算**：
  - 左栏高度约 460px，右栏高度约 470px；
  - 在 600px 窗口高度下，两栏完全无缝容纳在一屏之内，无需任何上下滚动！

---

## 六、具体改动文件与技术点清单

| 模块 | 改动文件 | 涉及函数 / 组件 / 配置 | 改动具体内容与目标 |
| :--- | :--- | :--- | :--- |
| **模块一** | `src/components/TrayMenuView.tsx` | `measureMenu` | 1. 消除外层重复叠加的 `+ 20` 内边距。<br>2. 测量净文本元素真实宽度，紧凑基准下限收紧至 280px。<br>3. 保持 `white-space: nowrap` 单行不折行。 |
| **模块一** | `src/components/TrayMenuView.test.tsx` | 单元测试 | 更新测试期望值与 mock，保证 280~500px 范围有效性与可证伪性。 |
| **模块一** | `src-tauri/src/tray.rs` | `TRAY_MENU_MIN_WIDTH` | 下限常数从 300.0 微调至 280.0，与前端保持契约统一。 |
| **模块二** | `src-tauri/src/tray.rs` | `layout_tray_menu` | 将 `tray.rect().map_err(...)?` 改为 `tray.rect().ok().flatten()`，报错时平滑进入屏幕右下角 fallback 分支。 |
| **模块二** | `src-tauri/src/tray.rs` | `setup_tray` | 监听纳入 `TrayIconEvent::DoubleClick`，防止快速连击被操作系统吞键。 |
| **模块二** | `src-tauri/src/tray.rs` | `open_tray_menu_window` | 增加 Toggle 支持（若菜单已可见则直接隐藏），并记录打开时间戳。 |
| **模块二** | `src-tauri/src/lib.rs` | `WindowEvent::Focused(false)` | 设立 150~200ms 保护期，过滤鼠标穿透启动时的瞬时 killfocus 误杀。 |
| **模块三** | `src-tauri/src/dock.rs` | `select_monitor_by_overlap` | 新增基于重叠相交面积的宿主屏幕选择算法，实现 >50% 面积跨屏归属。 |
| **模块三** | `src-tauri/src/dock.rs` | `is_external_boundary` | 新增屏幕接缝通道 vs 物理外边界拓扑判定，内部接缝严禁触发贴边折叠。 |
| **模块三** | `src-tauri/src/dock.rs` | `detect_edge` | 支持 `distance <= 0` 越界吸附，仅对物理外边界生效。 |
| **模块三** | `src-tauri/src/dock.rs` | `drag_ended` | 整合跨屏归属 + clamp 安全回弹 + 锚点持久化，未贴边时确保窗口完整吸在屏内。 |
| **模块四** | `src/types.ts` / `config.rs` | `OverlaySettings` | 新增 `showPercentageGrid: boolean` 配置字段及默认值。 |
| **模块四** | `src/i18n.ts` | 国际化语言包 | 增加 `showPercentageGrid` 双语对照文案。 |
| **模块四** | `src/components/OverlayView.tsx` | 胶囊渲染 | 在 `settings.showPercentageGrid` 开启时，在小窗进度条上渲染 9 条等分刻度黑细线（10 等分格子）。 |
| **模块四** | `src/index.css` | `.overlay-pill-tick` | 添加刻度细线绝对定位样式（`bottom: 10% ~ 90%`）。 |
| **模块五** | `src-tauri/tauri.conf.json` | `settings` window | 设置窗口默认尺寸从 480×660 升级为 960×600，minWidth 改为 760，minHeight 改为 500。 |
| **模块五** | `src-tauri/src/tray.rs` | 常数定义 | `SETTINGS_DEFAULT_WIDTH` 更新为 960.0，`SETTINGS_DEFAULT_HEIGHT` 更新为 600.0，`SETTINGS_MIN_WIDTH` 更新为 760.0。 |
| **模块五** | `src/components/SettingsView.tsx` | 整体布局 | 重构为双栏 Grid 布局，将所有设置项划分为“外观与交互”和“系统协同与通知”两栏，放置“开启百分比格子”于贴边隐藏正下方，消除垂直滚动。 |

---

## 七、自动化测试与可证伪性验证方案

### 7.1 前端 Vitest 测试矩阵
1. **紧凑基础宽度回归**：
   - 当菜单项文本在常规紧凑范围（如 220px 净宽）时，计算出的 `widthLogical` 落在 280px 基准下限，不再膨胀为 320~350px。
2. **极端长文案自适应撑开**：
   - 当菜单项出现超长版本号文案（如 `scrollWidth = 360px`）时，自适应撑开至贴合净宽，不折行且不超过 500px 上限。
3. **刻度格子开关渲染验证**：
   - 验证 `showPercentageGrid: true` 时，pill 内部每个进度条渲染出 9 条等分刻度线；
   - 验证 `showPercentageGrid: false` 时，不渲染刻度线节点。
4. **设置界面双栏与交互验证**：
   - 验证设置界面中包含双栏结构容器；
   - 验证“开启百分比格子”复选框位于“开启贴边隐藏”之后，且点击后正确触发 `onPatchSettings({ showPercentageGrid: ... })`。

### 7.2 后端 Rust 单元测试矩阵 (`dock.rs`, `tray.rs` & `config.rs`)
1. **多屏 >50% 跨屏归属判定测试**：
   - 模拟两台并排显示器（屏 A: 0..1920, 屏 B: 1920..3840）。
   - 浮窗横跨两屏（如 70% 面积在屏 B，30% 在屏 A）-> 必须精准判定归属为屏 B。
   - 浮窗横跨两屏（如 40% 面积在屏 B，60% 在屏 A）-> 必须精准判定归属为屏 A。
2. **内部接缝防误折叠测试**：
   - 浮窗在屏 A 和屏 B 交界处（屏 A 的右边界 / 屏 B 的左边界）松手，即便开启 `auto_edge_hide: true`，`detect_edge` 必须返回 `None`，严禁触发贴边胶囊折叠。
3. **物理外边界越界回弹与贴边测试**：
   - 开启自动贴边时，向最左屏左侧物理外框拖出 50px（越界 `x = -50`）-> 必须命中 `Edge::Left` 并折叠为贴边细条。
   - 关闭自动贴边时，向最左屏左侧物理外框拖出 50px（越界 `x = -50`）-> 必须平滑弹回屏内 `clamp_position(x = 0)`，绝不留在屏幕外。
4. **托盘点击响应性测试**：
   - 测试 `tray.rect()` 返回 `Err` 时，确保不 panic 且顺利进入 fallback 布局分支。
   - 测试菜单在 `show` 后 100ms 内触发 `Focused(false)` 时，不被误触发 `hide`。
5. **配置项序列化与反序列化测试**：
   - 测试包含/缺失 `showPercentageGrid` 的 json 配置文件的兼容加载与默认值注入。

---

## 八、执行与交付流程（定稿后执行）

1. **定稿确认**：待用户审阅确认本技术设计文档全部五大模块无误后，启动实现。
2. **代码修改与实现**：按照第六节清单严格顺序修改前端与后端核心逻辑。
3. **全量门禁检验**：
   - `npm test`（确保前端全部测试通过并覆盖新功能）；
   - `npm run build`（确保前端无类型错误编译通过）；
   - `cargo test --manifest-path src-tauri/Cargo.toml --locked`（确保 Rust 全部测试通过）；
   - `cargo check --manifest-path src-tauri/Cargo.toml --locked`（确保后端 0 warnings 通过）。
4. **Git 提交与远端推送**：测试全部通过后，创建符合规范的 Git 提交并推送到 GitHub 远端仓库。

