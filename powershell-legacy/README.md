# powershell-legacy — 旧版 PowerShell 实现存档

本文件夹保存 Tauri v2 重写（v0.3.x-preview）之前的完整 PowerShell 方案，
对应 tag `v0.1.1`（commit 9eeec6d / 9e8bbb8）。仅作历史存档，不再维护。

| 文件 | 说明 |
|---|---|
| `CodexPetUsageOverlay.ps1` | overlay 主体实现（WinForms 置顶悬浮窗） |
| `Get-CodexUsage.ps1` | Codex 用量数据抓取 |
| `Localization.ps1` | 中英文本地化 |
| `Run-CodexUsageOverlay.ps1` | 一键启动入口 |
| `Start-CodexPetUsage.ps1` / `Stop-CodexPetUsage.ps1` | 启动 / 停止 |
| `Start-CodexUsageOverlay.cmd` | 批处理启动器（CRLF） |
| `Build-Portable.ps1` / `Test-Portable.ps1` | 便携包构建与测试（原位于 scripts/） |

如需运行旧版，可参考当时 README 的用法，把脚本放回任意目录直接执行即可。
