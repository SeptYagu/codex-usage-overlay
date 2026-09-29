# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
