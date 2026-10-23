# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.4.0-preview] - 2026-10-23

### Added
- Versioned Smart Pause status in Settings, the tray menu and tooltip, with sample age, effective polling cadence, per-active-period outcomes and separate accepted-request/verified-interruption counters. Connection health retries while protection is enabled.
- An explicit automatic-resume preference (default off) and schema-versioned resume eligibility store. Raw quota recovery, freshness, account/activity identity, expiry, restart review and unknown-submission safeguards are covered offline. Automatic execution stays disabled on the current cooperative connection.

### Fixed
- Cancel unsent Smart Pause requests when newer quota data, read failures or settings changes supersede a delivery batch. Preserve accepted requests for de-duplication when a newer low sample describes the same account, active period and quota cycle.
- Reject malformed bridge listings instead of treating them as a successful observation of no active sessions.
- Await Codex initialization before requesting usage, report initialization errors immediately, and clean up the owned reader process on cancellation and early errors with a bounded cleanup deadline.
- Match quota windows by their reported duration and reject malformed or unrelated buckets. Retain positional compatibility when older responses omit the duration.
- Rotate delivery attempts after bounded batches so a slow first target cannot permanently starve later chats. Keep inactive-period status history from appearing as a current pause request.

### Changed
- Poll every 15 seconds near a pause threshold while protection is enabled, recover to the user's normal interval with two-sample hysteresis, and back off after failures. Settings changes interrupt the old polling sleep; long reads do not queue catch-up bursts.
- Send pause-only cooperative requests. Do not ask Codex to create external continuation timers. Existing external tasks are not cancelled by this change.
- Clarify in all three settings languages and both READMEs that Smart Pause is cooperative; acceptance does not verify actual stopping. Direct interruption and automatic continuation require capabilities that have not been established on this desktop connection.

## [1.3.3] - 2026-10-20

### Fixed
- The shadow audit slot is now persisted in its cleared state. An event that reached the local evidence log is no longer left behind in `burn-rate-state.json`, so a restart can no longer append the same evidence a second time and push real history out of the bounded log.
- A pending five-minute interval is no longer blind while the audit slot is occupied. A quota cycle, an overnight resume, a confirmed correction, a decrease or a saturation in that window latches the rejection on the interval, and the rejection is reported as soon as the slot is free — with the observation that caused it, not with the tick that finally reported it. An interval can no longer survive a quota cycle and train the learner across it.
- A restored pending interval is validated against the production quota state before it is resumed: the percentage range, the reset timestamp, the published and personalized rates, the learner snapshot, the segment identity and the observation timestamps.
- The smart pause handshake no longer repeats the same read-only call for every caller identity of a pipe that already answered without an active session.
- The active periods are released when the auto-pause switch is off or the quota is back above the threshold, so a session that returns in that window is paused again instead of being swallowed by the de-duplication ledger of its previous period.
- The bridge connection status is no longer stuck at `Unknown` while the quota is comfortable. A read-only handshake probe answers it, and the probe never sends a message to a session.

### Changed
- While the audit slot is occupied, the pending interval keeps advancing (`lastValidAt`) instead of being frozen, but it is still not settled and the learner does not move. A busy audit slot no longer costs a learning interval that the quota really produced.

### Verification
- Frontend: 97 tests passed; TypeScript/Vite production build passed.
- Rust: 293 regular tests passed, 0 failed, 4 pre-existing ignored replay tests unchanged; locked Cargo check with `--all-targets` passed with no warnings.
- Mutation gates: 10 of 10 single-mutation probes (the plan's M1–M6) turned a named test red. The first run exposed one probe that no test covered, and the missing regression test was added.
- Estimator parity: the 13 committed replay evidence files stay byte-identical.
- The version is `1.3.3` in all six release locations.

### Scope
- The shadow stack stays a shadow: **no user-visible behavior changed.** The personalized rate is still internal evidence and is never published.
- The packaged and native smoke validation could **not** be performed on this machine. The restart without a duplicate append, the paused-again session after the switch was toggled and the real bridge status are covered by the automated tests only.

## [1.3.2] - 2026-10-20

### Added
- An invisible 5-hour personalization stack that collects causal feedback for a later publication decision: the Fixed Weighted candidate promoted to production code, a frozen multiplicative learner (`A0=B0=10`, `lambda=0.99`, `c=clamp(A/B,0.75,1.50)`, decayed evidence `E`, trust `g=E/(E+8)`), a five-minute pending interval with a complete causal origin snapshot, a v2 state subtree with an algorithm-version boundary and an at-least-once audit outbox, and a bounded, de-identified local evidence log.
- The burn-rate state schema moved from v1 to v2. A v1 file keeps its 5H and Weekly crossing history and gains a neutral shadow subtree; a malformed shadow subtree keeps the recoverable quota history; an algorithm-version mismatch keeps the history and the audit identity and resets only the incompatible learner state.
- Offline gates that keep the committed replay evidence honest: the replay suite now imports the production weighted helper, and `scripts/check-replay-evidence.mjs` compares the regenerated evidence with the committed files byte by byte.

### Fixed
- The audit slot is now guarded for every event, not only for the personal outcome. A lifecycle transition during an occupied slot could overwrite an unappended matured or rejected event and destroy it for good, while the learner state had already advanced.
- A retained audit event is flushed before the next sample is processed, so a transient append failure no longer drops one five-minute learning interval.
- A backwards observation no longer records a negative gap duration.
- A quota cycle, an overnight resume and a confirmed correction are recorded with their own stable codes instead of one shared code.
- A pending interval that carries another algorithm version is discarded when the shadow state is normalized.

### Verification
- Frontend: 95 tests passed; TypeScript/Vite production build passed.
- Rust: 284 regular tests passed, 4 pre-existing ignored replay tests unchanged; locked Cargo check passed with no warnings.
- Estimator parity: the regenerated real-history forecast matches the pre-refactor baseline in all 318 rows with a **maximum prediction deviation of 0**, and the 13 committed evidence files stay byte-identical.
- Review: the shadow implementation was reviewed before the release gate by an independent read-only reviewer, which first reproduced the published behavior of the base commit side by side. The review found one critical, one major, seven minor and two nit defects; all of them are fixed and each fix has a regression test that turns red when the rule is removed.

### Scope
- Shadow only: **no user-visible behavior changed.** The overlay, the pill, the tray, the notifications, the settings, the polling cadence and the frontend DTO are untouched, and the frontend suite is unchanged. The personalized rate is internal evidence and is never published.
- The packaged and native smoke validation of the plan could **not** be performed on this machine. The restart, pending resume/reject, account switch, quota reset, log retry and the 30/60/120/300-second cadence behavior are covered by the automated tests only.

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
