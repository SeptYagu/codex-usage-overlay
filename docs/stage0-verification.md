# v1.1.0 阶段 0 验证记录

状态：**进行中，技术闸门尚未通过**。记录日期：2026-09-27，Windows 10 家庭版本机。验证依据：[多阶段方案](v1.1.0-multistage-plan.md)和[技术规格](v1.1.0-technical-spec.md)。本文件只记录实际观察；未测项目不视为通过。

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

预计用 Rust 侧状态机区分用户拖动锚点与程序移动。左、右边缘用竖向双条，上、下边缘将同一内容顺时针旋转 90° 并交换窗口外接宽高。实际吸附距离、尺寸与鼠标命中仍需 V2/V3/V6 实机确认。

文件选择拟用 Rust 侧 `tauri-plugin-dialog`，选择后验证扩展名、文件存在及解码能力。音频拟用 `rodio 0.22.2` 与 symphonia；通知默认模式沿用 Tauri 通知插件，自定义模式使用 WinRT 静音 toast。播放设备、队列和便携版 AUMID 仍未验证，选型尚未锁定。

## 验证结果

| 项目 | 当前结果 | 待完成 |
| --- | --- | --- |
| V1 格式解码与播放 | 用 ffmpeg 9.0.1 生成 1 秒 MP3、裸 ADTS AAC、M4A AAC、WAV 样本；`rodio 0.22.2` + 指定 symphonia features 均解码出非零样本，分别为 44100、46080、46080、44100。默认输出设备成功打开，四个样本依次播放完成，耗时分别为 0.958、1.048、1.059、1.010 秒。 | 还需实测真实来源文件及 AAC 变体；合成正弦波通过不代表所有用户文件兼容。 |
| V2 拖动结束 | 尚未修改或运行带仪表记录的拖动入口。 | 验证 `start_dragging()` 返回是否等于松手；若提前返回，改用原生结束事件。 |
| V3 工作区与 DPI | 当前只检测到一个显示器：`DISPLAY8`，Windows Forms 报告 Bounds `1707×960`、WorkingArea `1707×920`；注册表 AppliedDPI 为 144。 | 用应用进程内的 Tauri `Monitor::work_area()` 测量物理像素；验证四侧任务栏、多显示器和 100%/175%/250%。 |
| V4 十秒停止与队列 | 13 秒 WAV 经 `take_duration(10s)` 得 441014 个样本（44.1 kHz，约 10.0003 秒）；实际播放在 10.010 秒结束。 | 试听中途停止，以及双通知与试听交错时的队列顺序。 |
| V5 WinRT toast | 未测。 | NSIS、MSI、便携版的有声/静音通知、权限状态与 AUMID 快捷方式。 |
| V6 四边吸附 | 未测。 | 四边拖动、负坐标副屏、16 逻辑 px 阈值与鼠标命中。 |

音频探针源码位于 `scripts/stage0-audio-probe/`。合成样本与构建产物由 `.gitignore` 排除，可重新生成并执行：

```powershell
.\scripts\stage0-audio-probe\generate-samples.ps1
& "$env:USERPROFILE\.cargo\bin\cargo.exe" run --manifest-path scripts\stage0-audio-probe\Cargo.toml --target-dir src-tauri\target\stage0-audio-probe\target --release -- scripts\stage0-audio-probe\samples --playback
```

探针使用真实默认输出设备播放了四种格式，并验证了长 WAV 的十秒截断；尚未实现阶段 3 的停止试听、播放队列与错误恢复。阶段 0 完成前，贴边与自定义音效不得作为已验证功能发布。

本机曾间歇性拒绝运行新生成的 Rust 可执行文件（`os error 5`）。2026-09-27 重试后，`cargo test --locked` 成功运行全部 17 个 Rust 单元测试；播放探针复用已可运行的 Cargo target 目录后也成功执行。尚未确认最初的执行限制是安全软件、文件同步还是其他本机策略所致。
