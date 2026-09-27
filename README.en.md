# Codex Usage Overlay (Tauri v2)

[简体中文](README.md) · English

A Windows desktop overlay that displays Codex quota and credit balances. It reads usage through the local Codex `app-server` command `account/rateLimits/read`; it does not read or save `auth.json` or modify Codex installation files.

## Download and launch

Get the latest release from the [releases page](https://github.com/SeptYagu/codex-usage-overlay/releases). Choose the NSIS setup executable, MSI installer, or Windows x64 portable ZIP. `SHA256SUMS.txt` includes checksums for all three packages.

Install a setup package, or extract the portable ZIP and run `CodexUsageOverlay.exe`. Windows 10/11, WebView2, and an installed, signed-in Codex app are required. No extra PowerShell launcher is needed; Microsoft Store executable discovery uses the built-in Windows PowerShell.

## Overlay and settings

- Displays 5-hour and weekly **remaining** percentages, reset countdowns, and optional credits. Numeric balances use two decimals; unlimited credits appear as `∞`.
- **Grouped capsule** is the default layout: readings with reset times underneath and inline credits. **Metric stacks** puts labels above values and captions below. Switch layouts in **Overlay Settings**.
- Both layouts follow the Windows light/dark theme. Settings provide scale (100%–250%), background transparency (0%–80%), credits visibility, refresh interval (30/60/120/300 seconds), and English/Simplified Chinese.
- Settings apply immediately and synchronize between windows. Existing settings files retain their preferences and default to Grouped capsule.
- Drag with the left mouse button. Right-click the overlay for Refresh, Settings, Hide, and Exit. A single left click on the tray icon shows or hides the overlay.
- Autostart controls are available only for installed copies. Launching a portable or development copy leaves the installed copy's startup entry untouched.
- Codex executable discovery follows the existing override/PATH/CLI/Store order. If the executable moves or disappears, later polls can rediscover it. Set `CODEX_CLI_PATH` for a custom location.

Preferences, window position, and usage status are stored under `%LOCALAPPDATA%\CodexUsageOverlay\` in `settings.json`, `window-position.json`, and `usage-status.json`.

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
.\scripts\Build-TauriRelease.ps1 -Tag v1.0.0
```

The tag workflow tests and builds the Tauri app, then publishes NSIS, MSI, portable ZIP, and checksums. Packaging verifies the portable archive against its source files.

Feedback: septwind@agent.qq.com
