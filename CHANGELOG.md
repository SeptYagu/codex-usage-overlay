# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.3.1] - 2026-10-19

### Added
- Independent, persisted 5-hour quota and Burn Rate visibility switches, enabled by default for fresh and upgraded profiles, with English/Simplified Chinese/Traditional Chinese labels.
- Weekly-only expanded overlay, centered single-bar edge pill, and large single-ring tray presentation when 5H is hidden.
- Optional low-quota auto pause: a cooperative `暂停` request is sent to the running Codex session through the desktop client's own control plane, and a 5-hour trigger asks Codex to schedule a one-time task that sends `继续` two minutes after the reset. Off by default; `autoPauseOnLowQuota`, `autoPauseFiveHourThresholdPercent` (5 %) and `autoPauseWeeklyThresholdPercent` (2 %) are persisted settings with English/Simplified Chinese/Traditional Chinese labels.
- New `src-tauri/src/pause_control.rs` module: the threshold and message decisions, the in-memory de-duplication ledger, the ordered bridge-candidate discovery (`codex-app-tools-*` first, `codex-browser-use-*` as the legacy fallback) and the Windows named-pipe transport.
- A read-only bridge status command that reports whether a candidate answered the `list_threads` handshake and how many pause requests were positively confirmed; it never claims a delivery the client did not confirm.
- Offline production-entry tray lifecycle tests and a repeatable Edge layout QA runner.
- Offline weighted crossing-regression/idle/migration prototypes and a repeatable 240-parameter replay search with matched legacy accuracy/response evidence.
- Offline fractional-interpolation and adaptive-window ablations with actual Weekly polling cadences, independent-observation change detection, conditional recovery and committed matched stability/response/stress summaries; [validation remains partial](docs/handoff/2026-10-03-burn-rate-adaptive-validation.md).
- Private-log quota extraction and causal real-history forecast comparison across nine candidates, with availability, chronological checks and committed aggregate evidence; [real usage favors fixed weighting as the next candidate](docs/handoff/2026-10-03-burn-rate-real-usage-validation.md), without production integration.

### Fixed
- Apply the saved tray mode during cold startup, read failures and settings changes without cached usage; preserve pending/error tooltips and cached data until recovery.
- Repaint display preferences immediately without fetching usage; invalidate dock geometry for either switch and size temporary expansion from the visible composition.
- Increase temporary grouped height and credit width budgets to cover measured content, and keep all default settings controls visible at 960x620.
- Preserve the collapsed pill's session anchor across display switches and background refreshes, clamping the actual pill to the selected work area.
- Reserve 120px per quota when Burn Rate is hidden so 100% readings fit during expansion; include full-quota fixtures in browser QA.
- The caller-identity discovery accepts the compressed `.jsonl.zst` session transcripts and reads the thread UUID from before the extension chain, which is where the current Codex client puts it.
- The auto-pause active periods are released again when the client reports no active session. An empty listing used to be treated as a failed handshake, so the periods were never refreshed and a session that went idle and came back was paused only once per quota cycle for the life of the process.
- The bridge status follows the last handshake instead of staying `available` after the bridge stopped answering; a confirmed listing with no active session counts as an answered bridge, not a missing one.
- The bridge candidate discovery no longer runs while the app-wide lock is held. It runs on the blocking pool, and the session-directory walk is bounded at 32 files and visits the newest names first.
- A session that the listing reports twice, once with and once without a `hostId`, is contacted once, using the record that carries the host id.
- A 5-hour reset further than six hours away is no longer turned into a scheduled instruction to wait for days; it falls back to the plain `暂停`. The remaining time is computed with an overflow check, so a malformed reset value cannot produce a plausible looking duration.
- The session listing parser tolerates one extra layer of JSON encoding instead of silently emptying the target list.

### Verification
- Frontend: 95 tests passed; TypeScript/Vite production build passed.
- Rust: 201 regular tests passed, including a real Windows named-pipe protocol simulation and an offline run of the production refresh path through an injectable transport; locked Cargo check passed with no warnings.
- Review: the smart pause implementation was reviewed before the release gate, once read-only and once by mutating the production logic. The active-period release, the five-second batch deadline and the quota-cycle expiry each have a regression test that turns red when the rule is removed.
- Browser: 406 offline Edge layout cases passed, including normal and 100% quota fixtures; all four tray lifecycle regression mutations were detected during initial implementation.
- Bridge: the read-only `list_threads` handshake was exercised against the running Codex desktop client — pipe discovery, caller-identity discovery, the framed JSON-RPC `tools/call` and the session-record shape. No session was active at that moment, so no message was sent. The client's handling of the synthetic `turnId` was traced in the installed package: the schema only requires a non-empty string, and the send path routes by the target thread id.
- Estimator: the separately executed fixed estimator grid still generates 240 candidates with zero passing its current combined gates; the adaptive ablations still produce 5676 result rows and preserve the broader response/excursion regressions explicitly.

### Scope
- Selected path: display-only. The optional Phase 5 weighted estimator, idle metadata and migration remain deferred from production after the offline parameter search failed the current combined gates. Raw accuracy standards and stability/response tradeoffs are documented in the [replay report](docs/handoff/2026-10-03-burn-rate-estimator-replay.md). The v1.3.0 production estimator and state schema remain unchanged.
- The auto-pause feature is opt-in and ships dark: the code path, the automated tests and the read-only handshake are verified, but a real `send_message_to_thread` delivery, the one-time scheduled `继续` task and the signed package are **not**. The [handoff](docs/handoff/2026-10-19-v1.3.1-smart-pause-implementation.md) records each acceptance layer separately.
- Version metadata, the frontend, the Rust crate and the Tauri config now read 1.3.1, with 1.3.0 kept as the upgrade and rollback baseline. Packaging, signing and publication await the native WebView2/tray/hover/mixed-monitor smoke checks, which are still outstanding.

## [1.3.0] - 2026-10-02

### Added
- Account-level 5-hour and weekly Burn Rate estimates derived from observed quota crossings, with idle decay, long-gap segmentation, reset/correction handling, and account-switch isolation.
- Persistent Burn Rate estimator state in `burn-rate-state.json`, including accepted observation timing and bounded crossing history.
- Expanded-overlay Burn Rate display on the second row beside each reset countdown, with bounded `—`, `<0.1%/h`, and `>999%/h` presentation states.

### Changed
- Split raw Codex rate-limit snapshots from the UI-facing usage model so raw `usedPercent` precision remains `f64`.
- Unified background, manual, and post-reset usage refreshes behind one backend processing path and limited the Codex client lock to app-server I/O.
- Replaced the duplicate React startup fetch with listener-first, cache-only `get_last_usage` startup hydration.
- Increased provisional dock expansion geometry for the wider quota metadata row while keeping final sizing content-driven.

### Fixed
- Suppress Burn Rate output when the current quota sample is unavailable or invalid while preserving valid historical estimator state for recovery.
- Prevent delayed startup cache hydration from overwriting newer live usage events, including multiple updates within the same second and component cleanup races.

### Verification
- Frontend test suite: 63 tests passed.
- Rust test suite: 119 tests passed.
- Production frontend build completed successfully.
- Interactive Windows DPI/multi-monitor layout checks remain a release smoke-test item.

## [1.2.1] - 2026-09-30

### Changed
- Left-click on the tray icon immediately shows or hides the overlay using its existing dock-aware behavior. Left double-click toggles once, without a delay.
- Right-click exclusively opens or closes the tray menu, retaining its burst debounce.

### Fixed
- Portable updater manifests read the generated signature file instead of mistakenly using Tauri CLI instructions as the signature. Missing, empty, and malformed signatures fail packaging.

### Verification
- Interactive Windows gesture and signed-package smoke checks have not been performed for this patch; historical verification gaps remain documented in STATUS.md.

## [1.2.0] - 2026-09-29

### Added
- **Multi-Monitor Free Dragging & Screen Attribution**:
  - Full-desktop roaming allowing free movement across multiple displays.
  - Majority intersection area arbitration (>50%) determining screen ownership on release, with deterministic 50/50 host tie-breaking.
  - Decoupling of internal display seams from physical outer borders: releases across monitor seams stay expanded without false docking.
  - Safe out-of-bounds snapping and clamping at outer physical screen edges, with authoritative post-drag anchor persistence.
- **Docked Percentage Grid**:
  - Optional 10-equal-segment dividing grid (9 subtle divider lines) on docked capsule progress bars for rapid visual percentage estimation.
  - "Show percentage grid" setting toggle positioned beneath "Auto hide at screen edge", with persistent configuration sync.
- **Two-Column Settings Window Layout**:
  - Refactored settings window to a 960×620 two-column layout (Appearance & Overlay on the left, Notifications & System on the right).
  - Displays all configuration options in a single view with zero vertical scrolling under default settings.
  - Compact sound picker component with single-line path truncation, monospace styling, and compact buttons.

### Improved
- **Tray Menu Compact Sizing & Click Robustness**:
  - Adaptive tray menu width tightening formula (`clamp(280, 500, ceil(textW) + 40)`), eliminating excessive horizontal padding while guaranteeing single-line text in all supported languages.
  - Robust click debounce (280ms) and idempotent Toggle state machine for tray icon clicks.
  - 180ms focus grace period preventing premature collapse when mouse click-through is active.
  - Graceful fallback for Windows notification area flyout rect errors.

### Fixed
- Fixed settings window vertical clipping when both reset notifications have custom sound paths configured.
- Cleaned up internal adapter traits and test doubles for offline falsifiability across tray, focus, and drag session workflows.

---

## [1.1.3] - 2026-09-29

### Added
- Settings window free resize and cross-process geometry persistence.
- Adaptive tray menu popup width calculation based on content width.
- Version badge display in settings window and tray menu.

---

## [1.1.2] - 2026-09-28

### Added
- Mouse click-through mode support via settings toggle and tray menu.
- Automatic update background download, verification, and exit installation.

---

## [1.1.1] - 2026-09-28

### Added
- Grouped capsule and metric stack layouts.
- Auto-docking and edge-hide capabilities.
- 5-hour and weekly reset notifications with custom sound preview.
