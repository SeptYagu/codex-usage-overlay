# Antigravity Provider Support Plan

- **Date**: 2026-11-13
- **Baseline**: `main` (`76ed0b1`, `v1.8.5`)
- **Status**: Implementation complete; running final verification

## 1. Goals

Add first-class `antigravity` (`Provider::Antigravity`) provider support to `codex-usage-overlay` alongside `codex`, `claude`, and `dsh`, covering:
- Zero-credential Antigravity CLI statusline bridge (`antigravity-usage-bridge.exe statusline`)
- All Antigravity quota pools (`auto`, `gemini`, `argon`, `3p`) plus optional AI Credits extraction
- Low-quota protection via Antigravity CLI `PreInvocation` hook (`antigravity-usage-bridge.exe hook invocation`) without affecting tool permission prompts
- Independent monitor runtime, settings column, tray/overlay/carousel integration, localization (`en-US`, `zh-CN`, `zh-Hant`), and release packaging (`antigravity-usage-bridge.exe`)

## 2. Architecture & Design Decisions

### 2.1 Quota Capture (`statusline` Bridge)
- `antigravity-usage-bridge.exe statusline` reads JSON from stdin emitted by Antigravity CLI (`~/.gemini/antigravity-cli/settings.json` -> `statusLine`), persists raw telemetry + parsed rate limits to `%LOCALAPPDATA%\CodexUsageOverlay\antigravity-rate-limits.json`, chains any previously configured user custom `statusLine` command (5s timeout), and exits `0` on all paths so Antigravity CLI never disables the hook/statusline.
- First-time install backs up user `~/.gemini/antigravity-cli/settings.json` to `settings.json.codex-overlay.bak` and saves any prior non-overlay `statusLine` into `antigravity-bridge.json` for chaining.
- When no previous custom statusline exists, configures `statusLine` with `"stack_with_default": true` so Antigravity's built-in statusline remains visible.

### 2.2 Multi-Pool & Argon Support
- Captures all buckets from Antigravity statusline JSON:
  - **Gemini**: `gemini-5h` (`five_hour`), `gemini-weekly` (`weekly`)
  - **Argon**: `gemini-5h` (fallback shared 5H or `gemini-argon-5h` / `argon-5h`), `gemini-argon-wtu-weekly` / aliases (`gemini_argon_wtu_weekly`, `gemini-argon-weekly`, `gemini_argon_weekly`, `argon-weekly`, `argon_weekly`) (`weekly`)
  - **Third-Party (`3p`)**: `3p-5h` (`five_hour`), `3p-weekly` (`weekly`)
  - **AI Credits**: extracted when present (`credits` / `ai_credits` / `aiCredits`); otherwise `has_credits = false`, `credits_display = "—"`.
- `antigravityPool` setting (`auto` | `gemini` | `argon` | `3p`, default `auto`):
  - `auto` inspects `active_model_id` / `active_model_name` (`argon` / `orchestrator` -> Argon; `claude` / `gpt` / `sonnet` / `opus` / `haiku` -> 3P; else Gemini).
  - Changing `antigravityPool` re-projects immediately from cached `antigravity-rate-limits.json` and isolates history `source_key` as `antigravity-<pool>` (filename-safe on Windows).

### 2.3 Low-Quota Protection (`PreInvocation` Hook)
- `antigravity-usage-bridge.exe hook invocation` is registered under `~/.gemini/config/hooks.json` top-level key `"codex-usage-overlay"` on `PreInvocation` only (backed up once to `hooks.json.codex-overlay.bak`).
- Never emit `PreToolUse` `"decision": "allow"`, because that bypasses Antigravity user tool permission prompts.
- On allow: output `{}`.
- On block (when guard enabled and effective quota pool meets `pause_threshold_percent` with non-expired reset window): output `{"injectSteps":[{"errorMessage":{"userMessage":"<localized or custom message>"}}],"terminationBehavior":"terminate"}`.

### 2.4 Integration & Packaging
- Backend: `src-tauri/src/antigravity/{shared,mod,install,source}.rs`, `src-tauri/src/bin/antigravity_bridge.rs`, `config.rs`, `monitor.rs`, `commands.rs`, `lib.rs`, `pause_control/status.rs`, `tray.rs`, `notify.rs`.
- Frontend: `src/types.ts`, `src/usageSource.ts`, `src/App.tsx`, `src/hooks/{useProviderMonitors,useDisplayState}.ts`, `src/components/{ProviderLogo,AntigravityConnection,SettingsView,OverlayView,TrayMenuView}.tsx`, `src/i18n.ts`.
- Packaging: `src-tauri/Cargo.toml` (`[[bin]] name = "antigravity-usage-bridge"`), `scripts/Build-TauriRelease.ps1`, `scripts/Test-TauriRelease.ps1`, `.github/workflows/release.yml`, and `commands.rs::install_verified_bytes`.

## 3. Verification Plan
- `pwsh -NoProfile -File ./scripts/Test-TauriRelease.ps1`
- `npm run build`
- `npm test`
- `node scripts/affine-parity-fixture.mjs --check`
- `cargo test --manifest-path src-tauri/Cargo.toml --locked`
- `cargo build --manifest-path src-tauri/Cargo.toml --locked --bin antigravity-usage-bridge`
