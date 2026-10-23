# Codex Usage Overlay (Tauri v2)

[![GitHub Release](https://img.shields.io/github/v/release/SeptYagu/codex-usage-overlay?color=3b82f6&logo=github)](https://github.com/SeptYagu/codex-usage-overlay/releases/latest)
[![CI](https://github.com/SeptYagu/codex-usage-overlay/actions/workflows/ci.yml/badge.svg)](https://github.com/SeptYagu/codex-usage-overlay/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-10b981.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows-0078D6?logo=windows)](https://github.com/SeptYagu/codex-usage-overlay/releases/latest)

[简体中文](README.md) · English

A Windows desktop overlay that displays Codex quota and credit balances. It reads usage through the local Codex `app-server` command `account/rateLimits/read`; it does not read or save `auth.json` or modify Codex installation files.

## Download and launch

Get the latest release from the [releases page](https://github.com/SeptYagu/codex-usage-overlay/releases). Choose the NSIS setup executable, MSI installer, or Windows x64 portable ZIP. `SHA256SUMS.txt` includes checksums for all three packages.

Install a setup package, or extract the portable ZIP and run `CodexUsageOverlay.exe`. Windows 10/11, WebView2, and an installed, signed-in Codex app are required. No extra PowerShell launcher is needed; Microsoft Store executable discovery uses the built-in Windows PowerShell.

## Overlay and settings

- Displays 5-hour and weekly **remaining** percentages, reset countdowns, and optional credits. Numeric balances use two decimals; unlimited credits appear as `∞`.
- **Grouped capsule** is the default layout: readings with reset times underneath and inline credits. **Metric stacks** puts labels above values and captions below. Switch layouts in **Settings**.
- Both layouts follow the Windows light/dark theme. Settings provide scale (100%–250%), background transparency (0%–80%), credits visibility, refresh interval (30/60/120/300 seconds), and English, Simplified Chinese, or Traditional Chinese.
- Settings apply immediately and synchronize between windows. Existing settings files retain their preferences and default to Grouped capsule.
- Each quota can send its own reset notification, once per confirmed new cycle. Use the Windows default sound or choose and preview an MP3, AAC, M4A, or WAV file; custom playback is capped at 10 seconds and the app stores only the original file path.
- Optionally enable **Pause Codex when quota runs low**: when either remaining percentage reaches or falls below its threshold (5-hour: 5%, weekly: 2% by default), the app sends a cooperative "pause" request through the Codex desktop client's control plane. Settings, the tray menu and the tray tooltip report protection health. Settings details distinguish accepted requests from verified interruptions and show per-chat outcomes. Protection is off by default; request de-duplication is kept in memory.
- While protection is enabled, quota polling accelerates to 15 seconds when either window is within 5 percentage points of its threshold. It returns to your normal refresh interval after two successful readings put both windows more than 7 points above their thresholds. Failed reads back off. Settings show the sample age and effective interval.
- Drag with the left mouse button. Enable **Auto hide at screen edge**, then drag the overlay to a work-area edge to collapse it into two quota bars; hover to expand it. Supports multi-monitor free dragging with majority intersection area attribution (>50%), internal seam traversal without false folding, and physical outer boundary safe snapping. Docked quota bars support optional **Percentage Grid** (10-segment dividing ticks) for direct visual estimation. Left-click the tray icon to immediately show or hide the overlay, including when docked. A left double-click toggles it once. Right-click opens or closes the tray menu.
- Settings uses a two-column layout (960×620), with resizing and window geometry persistence. Expanded protection details and smaller windows can scroll.
- Enable **Mouse click-through** in Settings or the tray menu to interact with windows behind the overlay. Use the tray menu to turn it off at any time; click-through cannot be enabled without a working tray icon.
- Autostart controls are available only for installed copies. Launching a portable or development copy leaves the installed copy's startup entry untouched.
- Use the tray menu to change languages and check for updates. When a new version is found, click the status line below Check for Updates to install it; a failed installation can be retried there. Automatic installation is off by default. When enabled, it also enables automatic checks, then downloads and verifies new versions in the background and installs when the app normally exits. The installed build uses the new version on the next launch; the portable helper replaces its executable after exit. Windows installer progress or elevation prompts may still appear. A prepared update can also be installed immediately from the tray status line.
- Codex executable discovery follows the existing override/PATH/CLI/Store order. If the executable moves or disappears, later polls can rediscover it. Set `CODEX_CLI_PATH` for a custom location.

Preferences, expanded position, dock state, and usage status are stored under `%LOCALAPPDATA%\CodexUsageOverlay\` in `settings.json`, `window-position.json`, `dock-state.json`, and `usage-status.json`. Custom sound files stay at the location you selected.

Smart Pause requests cooperation from Codex; accepted delivery does not verify stopped inference, queued work or background commands. Disabling protection, a failed read or newer ineligible quota cancels unsent requests. An already submitted request may still complete.

**Automatic resume after reset** is a separate preference, off by default. It is unavailable on the current connection because exact-turn interruption, completion and later user activity cannot be verified. The guarded resume store uses `pause-resume-state.json` in the same runtime directory; cooperative deliveries cannot create executable plans. Restored pending plans require review, and unknown submission outcomes are never retried automatically. This build sends only pause requests and creates no external "Continue" timers. Older Codex-created continuation tasks remain external and may still run; disabling the overlay does not cancel them.

## Development

Install Node.js 22 and a current stable Rust toolchain, plus the Windows Tauri build prerequisites.

```powershell
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run tauri dev
```

Build and package a release:

```powershell
npm run tauri -- build --ci --bundles nsis,msi
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin codex-usage-updater
.\scripts\Build-TauriRelease.ps1 -Tag v1.1.0-preview
```

Set the Tauri updater signing key in the build environment before packaging a release. The tag workflow publishes NSIS, MSI, portable ZIP, signatures, and `latest.json`; the portable updater helper is included in the ZIP.
See the [preview notes](docs/v1.1.0-preview-notes.md) for automated results and Windows checks still to be completed.

## License

This project is licensed under the [MIT License](LICENSE).

Feedback: septwind@agent.qq.com
