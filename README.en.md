# Codex Usage Overlay

[![简体中文](https://img.shields.io/badge/Language-%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-lightgrey?style=for-the-badge)](README.md) [![English](https://img.shields.io/badge/Language-English-2F6FEB?style=for-the-badge)](README.en.md)

A standalone usage overlay for Windows. It does not read Codex Pets settings, pet position, or Codex UI state, and it does not modify Codex installation files. The overlay gets usage data from the local Codex `app-server` through `account/rateLimits/read`. It does not read or save `auth.json` or call web APIs.

The window stays on top, is hidden from the taskbar, and can be dragged with the left mouse button. Its position is saved to `%LOCALAPPDATA%\CodexUsageOverlay\window-position.json` for the current Windows user. The interface is about 1.75 times the size of the original mini bar and refreshes every 60 seconds by default. The second row shows the 5H and WK reset countdowns without period labels. Each countdown uses cumulative hours and minutes, such as `167H 05min`; the two values are separated by two spaces and use larger, bold text. The groups on the first row are separated by one space. The 5H/WK value fields reserve room for three digits and a percent sign to reduce width changes as values change. Both rows share a background, are left aligned, and determine the window width together based on whichever row is longer.

The app has a bright, dashboard-style system tray icon. Double-click it to show or hide the window. Its right-click menu lets you show or hide the window, refresh usage immediately, open overlay settings, enable or disable launch at sign-in, or exit. In **Overlay Settings**, sliders adjust the window size (100%–250%) and background opacity (0%–80%, default 23%). You can also toggle **Show Credit balance**. This setting controls whether the balance appears in both the overlay and the tray tooltip. Choices are saved when the settings window closes. Opacity affects only the background; text remains clear. Settings are saved to `%LOCALAPPDATA%\CodexUsageOverlay\settings.json`. The bottom of the settings window shows the feedback email `septwind@agent.qq.com`. On first launch, the app creates a sign-in startup entry for the current Windows user; administrator rights are not required. Turn off **Launch at sign-in** in the tray menu to disable it. The choice is saved.

## Launch

Double-click `Start-CodexUsageOverlay.cmd` to start the app. You can also open PowerShell in the extracted folder and run:

```powershell
.\Start-CodexPetUsage.ps1
```

The overlay appears in the upper-right corner of the screen on first launch. After it starts, you can close the PowerShell window; the overlay will keep running.

## Stop

Run this in PowerShell:

```powershell
.\Stop-CodexPetUsage.ps1
```

You can also right-click the overlay and choose **Exit Overlay**.

## Compatibility

- Windows 10 and 11.
- Uses Windows PowerShell 5.1 by default, which is included with Windows. PowerShell 7 and administrator rights are not required. Set `CODEX_USAGE_POWERSHELL_PATH` to use a different PowerShell executable.
- Requires the Codex desktop app to be installed and signed in. The app automatically looks for Codex on `PATH`, in the user's Codex CLI directory, or in the Microsoft Store installation.
- If Codex is installed in a custom location, set `CODEX_CLI_PATH` before launch to the `codex.exe` in that installation.
- To choose a PowerShell executable, set `CODEX_USAGE_POWERSHELL_PATH` to the path of `pwsh.exe` or `powershell.exe`.
- If you use a custom `CODEX_HOME`, set that environment variable before launching the overlay. The usage-reading child process inherits it.
- If startup fails, the app displays an error and writes details to `%LOCALAPPDATA%\CodexUsageOverlay\startup-error.log`.
- Usage-reading status is recorded in `%LOCALAPPDATA%\CodexUsageOverlay\usage-status.json`. It contains the last update time and any error details, but not usage values.
- If you move the app folder, launch it manually once to update the path in the sign-in startup entry.

Codex Pets does not need to be open, and administrator rights are not required. This tool supports Windows only and currently requires PowerShell and the Codex desktop app to be available.
