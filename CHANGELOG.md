# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — v1.3.1 display controls

### Added
- Independent, persisted 5-hour quota and Burn Rate visibility switches, enabled by default for fresh and upgraded profiles, with English/Simplified Chinese/Traditional Chinese labels.
- Weekly-only expanded overlay, centered single-bar edge pill, and large single-ring tray presentation when 5H is hidden.
- Offline production-entry tray lifecycle tests and a repeatable Edge layout QA runner.

### Fixed
- Apply the saved tray mode during cold startup, read failures and settings changes without cached usage; preserve pending/error tooltips and cached data until recovery.
- Repaint display preferences immediately without fetching usage; invalidate dock geometry for either switch and size temporary expansion from the visible composition.
- Increase temporary grouped height and credit width budgets to cover measured content, and keep all default settings controls visible at 960x620.
- Preserve the collapsed pill's session anchor across display switches and background refreshes, clamping the actual pill to the selected work area.
- Reserve 120px per quota when Burn Rate is hidden so 100% readings fit during expansion; include full-quota fixtures in browser QA.

### Verification
- Frontend: 86 tests passed; TypeScript/Vite production build passed.
- Rust: 125 tests passed; locked Cargo check passed.
- Browser: 406 offline Edge layout cases passed, including normal and 100% quota fixtures; all four tray lifecycle regression mutations were detected during initial implementation.

### Scope
- Selected path: display-only. The optional Phase 5 weighted estimator, idle metadata and migration are deferred to a later change pending their complete replay gates. The v1.3.0 production estimator and state schema remain unchanged.
- Version bump, packaging and publication await native WebView2/tray/hover/mixed-monitor smoke checks. Browser DPI emulation and offline effects do not close that gate.

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
