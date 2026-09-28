# v1.1.0 阶段 0 验证记录

状态：**进行中，技术闸门尚未通过**。记录日期：2026-09-28，Windows 10 家庭版本机。验证依据：[多阶段方案](v1.1.0-multistage-plan.md)和[技术规格](v1.1.0-technical-spec.md)。本文件只记录实际观察；未测项目不视为通过。

## 交互与实现约定

```text
用户拖动完整浮窗并松手
  ├─ 距工作区边缘 > 16 逻辑 px → 保存用户锚点，保持展开
  └─ 距工作区边缘 ≤ 16 逻辑 px → 保存锚点，收起为双条小窗
       ├─ 鼠标进入 → 展开到锚点（使小窗命中区仍被覆盖）
       └─ 鼠标离开 → 400 ms 后确认无拖动/菜单锁，再收起

Windows 通知权限关闭 → 设置页显示系统通知状态，不循环请求权限。
自定义音效无效或设备不可用 → 保留路径、显示错误，不阻断另一额度。
```

Rust 侧状态机区分用户拖动锚点与程序移动。源码核对确认 Tauri 2.12.0 所用 Tao 0.37.1 的 `start_dragging()` 通过 `PostMessageW(WM_NCLBUTTONDOWN)` 异步发起系统拖动，因此不能把命令返回视为松手；实现改用窗口原生 `WM_EXITSIZEMOVE` 通知。左、右边缘用竖向双条，上、下边缘将同一内容顺时针旋转 90° 并交换窗口外接宽高。四边几何已有单元测试；真实鼠标命中、吸附手感与不同显示器仍需 V2/V3/V6 实机确认。

文件选择拟用 Rust 侧 `tauri-plugin-dialog`，选择后验证扩展名、文件存在及解码能力。音频拟用 `rodio 0.22.2` 与 symphonia；通知默认模式沿用 Tauri 通知插件，自定义模式使用 WinRT 静音 toast。播放设备、队列和便携版 AUMID 仍未验证，选型尚未锁定。

## 验证结果

| 项目 | 当前结果 | 待完成 |
| --- | --- | --- |
| V1 格式解码与播放 | 用 ffmpeg 9.0.1 生成 1 秒 MP3、裸 ADTS AAC、M4A AAC、WAV 样本；`rodio 0.22.2` + 指定 symphonia features 均解码出非零样本，分别为 44100、46080、46080、44100。默认输出设备成功打开，四个样本依次播放完成，耗时分别为 0.958、1.048、1.059、1.010 秒。 | 还需实测真实来源文件及 AAC 变体；合成正弦波通过不代表所有用户文件兼容。 |
| V2 拖动结束 | 已检查 Tauri/Tao 源码：`start_dragging()` 异步投递 `WM_NCLBUTTONDOWN`，返回早于实际松手。阶段 4 已用 `SetWindowSubclass` 捕获 `WM_ENTERSIZEMOVE` / `WM_EXITSIZEMOVE`，只在结束消息后保存锚点与判断吸附。 | 仍需在运行窗口上人工拖动四边确认原生消息、拖动锁和用户位置写入时序；本机当前自动化会话未提供可交互的 Tauri 桌面窗口。 |
| V3 工作区与 DPI | 当前只检测到一个显示器：`DISPLAY8`，Windows Forms 报告 Bounds `1707×960`、WorkingArea `1707×920`；注册表 AppliedDPI 为 144。 | 用应用进程内的 Tauri `Monitor::work_area()` 测量物理像素；验证四侧任务栏、多显示器和 100%/175%/250%。 |
| V4 十秒停止与队列 | 13 秒 WAV 经 `take_duration(10s)` 得 441014 个样本（44.1 kHz，约 10.0003 秒）；实际播放在 10.010 秒结束。阶段 3 增加的队列单测确认正式通知优先于试听，停止试听不会清除正式通知。 | 仍需手动确认实际设备上的试听中途停止、两种通知与试听交错顺序，以及失效文件的界面错误。 |
| V5 WinRT toast | 未测。 | NSIS、MSI、便携版的有声/静音通知、权限状态与 AUMID 快捷方式。 |
| V6 四边吸附 | 未测。 | 四边拖动、负坐标副屏、16 逻辑 px 阈值与鼠标命中。 |

音频探针源码位于 `scripts/stage0-audio-probe/`。合成样本与构建产物由 `.gitignore` 排除，可重新生成并执行：

```powershell
.\scripts\stage0-audio-probe\generate-samples.ps1
& "$env:USERPROFILE\.cargo\bin\cargo.exe" run --manifest-path scripts\stage0-audio-probe\Cargo.toml --target-dir src-tauri\target\stage0-audio-probe\target --release -- scripts\stage0-audio-probe\samples --playback
```

探针使用真实默认输出设备播放了四种生成样本，并验证了长 WAV 的十秒截断。阶段 3 已实现文件预检、单线程通知/试听队列、试听停止、设备错误事件与退出清理；队列顺序和格式筛选有 Rust 单测，但实际设置页试听中止、设备恢复和真实来源文件仍需人工体验验证。阶段 4 已实现四边物理几何和原生拖动结束钩子，阶段 5 已实现双条指示与悬停状态；桌面窗口、多显示器和系统通知实测仍未完成，阶段 0 因此仍未通过。

本机曾间歇性拒绝运行新生成的 Rust 可执行文件（`os error 5`）。2026-09-27 重试后，`cargo test --locked` 成功运行全部 17 个 Rust 单元测试；播放探针复用已可运行的 Cargo target 目录后也成功执行。2026-09-28 的 v1.1.0 版本已通过 `cargo test --locked`（30 项）和 `cargo check --locked`。尚未确认最初的执行限制是安全软件、文件同步还是其他本机策略所致。
