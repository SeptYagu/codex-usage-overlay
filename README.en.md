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
- Drag with the left mouse button. Enable **Auto hide at screen edge**, then drag the overlay to a work-area edge to collapse it into two quota bars; hover to expand it. Supports multi-monitor free dragging with majority intersection area attribution (>50%), internal seam traversal without false folding, and physical outer boundary safe snapping. **Show 10% grid on edge bars** adds ten equal divisions to the collapsed quota bars. This setting appears only with **Auto hide at screen edge** enabled; drag to an edge and move the pointer away to see the grid. Left-click the tray icon to immediately show or hide the overlay, including when docked. A left double-click toggles it once. Right-click opens or closes the tray menu.
- Settings defaults to 1320×800 with shared appearance/system controls and independent Codex, Claude and DSH columns. Each provider has its own refresh interval, reset notifications and sounds. Narrow windows wrap the columns; window geometry is retained.
- Enable **Mouse click-through** in Settings or the tray menu to interact with windows behind the overlay. Use the tray menu to turn it off at any time; click-through cannot be enabled without a working tray icon.
- Autostart controls are available only for installed copies. Launching a portable or development copy leaves the installed copy's startup entry untouched.
- Use the tray menu to change languages and check for updates. When a new version is found, click the status line below Check for Updates to install it; a failed installation can be retried there. Automatic installation is off by default. When enabled, it also enables automatic checks, then downloads and verifies new versions in the background and installs when the app normally exits. The installed build uses the new version on the next launch; the portable helper replaces its executable after exit. Windows installer progress or elevation prompts may still appear. A prepared update can also be installed immediately from the tray status line. Turn on **Include preview releases** to also be offered preview builds (off by default). Previews carry numbered versions (`1.5.0-preview.1` < `1.5.0-preview.2` < `1.5.0`), so each one updates to the next preview or the stable release.
- Codex executable discovery follows the existing override/PATH/CLI/Store order. If the executable moves or disappears, later polls can rediscover it. Set `CODEX_CLI_PATH` for a custom location.

Preferences, expanded position, dock state, and usage status are stored under `%LOCALAPPDATA%\CodexUsageOverlay\` in `settings.json`, `window-position.json`, `dock-state.json`, and `usage-status.json`. Custom sound files stay at the location you selected.

Smart Pause requests cooperation from Codex; accepted delivery does not verify stopped inference, queued work or background commands. Disabling protection, a failed read or newer ineligible quota cancels unsent requests. An already submitted request may still complete.

**Automatic resume after reset** is a separate preference, off by default. It is unavailable on the current connection because exact-turn interruption, completion and later user activity cannot be verified. The guarded resume store uses `pause-resume-state.json` in the same runtime directory; cooperative deliveries cannot create executable plans. Restored pending plans require review, and unknown submission outcomes are never retried automatically. This build sends only pause requests and creates no external "Continue" timers. Older Codex-created continuation tasks remain external and may still run; disabling the overlay does not cancel them.

## Monitor multiple providers

Available enabled local Codex, Claude and DSH sources keep monitoring in the background, with separate readings and burn-rate histories. Click the overlay's provider logo to cycle Codex → Claude → DSH; only sources with an available reading and enabled monitoring are included. Switching displays cached data immediately and leaves monitoring/protection running independently.

Settings separates **Displayed provider** from **Provider settings**. Select a source to configure its monitor and protection without changing the overlay. Disabling its monitor also disables its protection. On upgrade, only the previously selected source inherits an enabled protection switch; other sources start with protection off. Claude live reading remains opt-in. Enabled DSH monitoring automatically connects to a supported running Desktop and reads its active subscription account; pause sessions still require explicit selection.

Show/hide is remembered across restarts. A hidden overlay launches hidden while monitoring and the tray continue running. The tray icon follows the displayed source and retains its cached quota during a temporary read failure; the tooltip reports the failure. If the tray is unavailable, the overlay is shown for recovery.

## DSH subscription usage

With DSH monitoring enabled, the overlay looks for a running DSH Desktop in the background, even while another provider is displayed. It uses Desktop's official CLI to configure its bundled companion when needed, waits for discovery and authentication, then reads the subscription plugin's active account. No manual plugin installation or pairing is required for the supported Windows default Desktop profile.

Multiple uncertain instances are not selected arbitrarily. A profile identifier allows reconnection after DSH restarts, but protected sessions must be selected again. The overlay never restarts DSH. Disable DSH monitoring to stop background attachment; an explicit Disconnect stays disconnected until Retry. A compatibility connection-link form remains available for other configurations.

History is isolated by instance/account. DSH thresholds default to 0% remaining. Protection sends a cooperative pause message only to explicitly selected ordinary running sessions; acceptance does not prove interruption, and automatic resume is unavailable. Local Codex and Claude are never used as a fallback. See the [setup and limitations guide](docs/dsh-subscription.md) and [companion protocol](integrations/dsh-bridge/README.md).

## Claude usage limits

Choose **Claude** under **Provider settings** to configure its connection and protection. Choose **Claude** under **Displayed provider**, or click the overlay logo, to show its 5-hour and weekly limits. Other available enabled sources continue monitoring.

- Click **Connect Claude Code**. The overlay registers the bundled `claude-usage-bridge.exe` as Claude Code's statusline command and adds two hooks (`UserPromptSubmit`, `PreToolUse`) to `~/.claude/settings.json`, honoring `CLAUDE_CONFIG_DIR`. It backs up the original file once as `settings.json.codex-overlay.bak`. Your existing statusline keeps rendering through the helper, and **Disconnect** restores it and removes only the entries the overlay added. Claude Code applies hook changes to new sessions.
- By default, limits come from the `rate_limits` data Claude Code passes to its statusline. This needs Claude Code 2.1.80 or later, signed in with a Pro or Max plan; API-key sessions report none. In this mode the overlay reads no Claude credentials and makes no network requests. Values update only while the **terminal `claude` CLI** is in use: the desktop app and IDE extensions never run a statusline.
- **Read limits live with your Claude login** (off by default) covers the desktop app. Once a minute at most, the overlay asks the usage endpoint behind Claude Code's `/usage`, using the login the terminal CLI stores in `~/.claude/.credentials.json`. The token is read for each request and is never stored, logged or sent anywhere else. The endpoint is undocumented and may change. The desktop app does not share its own login, so sign in once with `claude` → `/login`. CLI logins expire after a few hours; the overlay then runs `claude plugin list`, then `claude doctor` if needed, (at most every ten minutes), which makes Claude Code renew its own login without using quota. The overlay never writes the credentials. Live readings are written to the same local file, so the guard hooks use them too. While a live read fails or the endpoint asks to wait, the overlay keeps showing that file's last reading with its age, and Settings shows the failure. A window whose reset time has passed is shown as reset, and the reset notification fires with the next sample from the new cycle.
- With the protection switch on, the hooks refuse new prompts and tool calls while the 5-hour or weekly quota is at or below its threshold, and Claude Code shows the reason. Codex Smart Pause is not used for Claude. An old sample still counts until its reset time, because used quota only grows within a window. A missing or unreadable sample, a passed reset or a turned-off switch always lets requests through.
- The capsule starts with the active provider's logo (OpenAI for Codex, Claude for Claude); **Show provider logo** hides it. The marks come from Simple Icons (Claude, CC0) and LobeHub Icons (OpenAI, MIT) and are trademarks of their owners. Credits are not shown for Claude. Burn rate and reset state are kept in separate `claude-` prefixed files, so Claude samples never mix with the Codex history.
- The helper writes `claude-rate-limits.json` and `claude-guard-state.json`, and the overlay writes `claude-guard.json` and `claude-bridge.json`, all in the runtime directory above. Disconnect before uninstalling; otherwise Claude Code keeps calling the removed helper.

## New main-branch features (not released yet)

The Codex column has separate 5-hour and Weekly pause messages, saved on blur with a 2,000 UTF-16-unit limit. Weekly takes precedence. Confirmed or uncertain submissions are not automatically repeated for the same account/chat/quota cycle; editing text does not reset this ledger.

Explicitly applying the continuation preset inserts `{resume_at}` as the server reset time plus 120 seconds, including date, seconds, offset and zone. The receiving chat creates and verifies its own task, whose saved prompt requires self-pausing before continuing. The overlay does not create tasks directly, acceptance proves neither stopping nor scheduling, and disabling protection does not cancel existing tasks. Invalid time or zone falls back to plain pause.

Codex 5H burn-rate display offers Classic (default), Weighted and experimental Personalized estimation, with reported fallback. Changing it only reprojects a cached successful sample: no extra reads, training, notification or protection actions. Weekly remains unchanged. Cold personalization starts at Weighted and is not a promise of better future accuracy.

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
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin claude-usage-bridge
npm run tauri -- build --ci --bundles nsis,msi
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin codex-usage-updater
.\scripts\Build-TauriRelease.ps1 -Tag v1.5.1
```

Set the Tauri updater signing key in the build environment before packaging a release. The tag workflow publishes NSIS, MSI, portable ZIP, signatures, and `latest.json`; the portable updater helper is included in the ZIP. The installers include every Cargo binary present at bundle time, so build the Claude bridge before `tauri build`.
See the [preview notes](docs/v1.1.0-preview-notes.md) for automated results and Windows checks still to be completed.

## License

This project is licensed under the [MIT License](LICENSE).

Feedback: septwind@agent.qq.com
