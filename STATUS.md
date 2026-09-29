# Status

_Last updated: 2026-09-29 (v1.2.0 round-2 fixes: P3-1 call-site adapters implemented and verified with 5 dedicated unit tests across tray click, fallback anchor, show sequence, focus blur, and dock persistence; P3-2 SoundPicker compaction layout budget sentinels added to SettingsView.test.tsx. Four gates fully green: npm test 56/56, npm run build, cargo test 99/99, cargo check 0 warnings)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.2.0` (5 config files + `src/version.ts` unified; feature commit `998a558`) |
| Reviewed commit | `998a558` (`feat: implement v1.2.0 five modules across tray, dock, pill grid, and settings`) — v1.2.0 line |
| Baseline of that review | `6eeed35` (range `6eeed35..998a558`, 19 files / +1731 −343) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate (v1.1.3) | **Passed** — v1.1.3 round 5 (Mode C) closed the line with **0 defects**. Round 4's P3-1 is closed: `AppState::for_test` (`src-tauri/src/commands.rs:53-81`) plus the cache-read wiring tests make the capture/read/priority decisions falsifiable, and three acceptance mutations were each verified to fail tests. Rust tests 61 → 65 |
| Code review gate (v1.2.0) | **In review (round 3)** — round 2's P3-1 (M1~M4, M9 call-site adapter traits) and P3-2 (SoundPicker compaction layout budget sentinels) resolved. Ready for Round 3 verification. |
| Design review gate (v1.2.0) | **Superseded by implementation** — the design round-3 residuals (P3-1 landing seam, P3-2 §7.2.6 observation points, P3-3 unique attribution rule) were addressed in `998a558` and doc sync R-5 completed. See `docs/handoff/2026-09-29-workbuddy-code-review-round3-handoff.md` |
| Compliance audit | **Compliant** — the v1.1.2 and v1.1.3 plans and the v1.1.1 behavior review were audited line-by-line against the code: every in-scope deliverable is implemented, with no omissions, reversals, or v1.1.2 regressions (`docs/handoff/2026-09-29-cross-plan-compliance-audit.md`) |

## Gates (reproduced locally on 2026-09-29 after round 2 fixes)

- `npm test` → 56/56 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 99/99 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass (0 warnings)

Green gates do **not** constitute review approval, and none of them exercise a real GUI.

## Open defects

None in the v1.1.3 review line.

In the **v1.2.0 implementation**:
- **P2-1 (settings-window clipping)** — **Resolved (behaviourally)**: both `<section>` columns have `min-h-0 overflow-y-auto pr-1`; `SoundPicker`'s path is a single-line `truncate font-mono` row with a tooltip and `py-0.5` buttons. Verified in a real layout engine (Chromium build of `dist/` + Tauri bridge stub, 960×620): root `clientHeight == scrollHeight` = 620 for all six configurations (3 languages × {安装版, 便携版} × {both/only-weekly custom sounds + long paths}), every control is fully scrollable into view (`requiredScroll <= maxScroll`, `unreachable: []`), and the default configuration shows no scrollbar. The same holds at the 938×522 viewport and at `SETTINGS_MIN_HEIGHT = 500`.
- **P3-1 (module 2 and dock wiring falsifiability)** — **Resolved**: Injected call-site adapter traits across all five critical call sites (`TrayClickWindowOps` + `execute_tray_click_effect`, `compute_tray_anchor_with_fallback`, `TrayMenuShowOps` + `execute_tray_menu_show_action`, `TrayMenuFocusWindowOps` + `apply_tray_menu_focus_event`, and `DragOutcomeSessionOps` + `commit_drag_outcome_to_session`). Each adapter is verified with offline unit tests with recording test doubles. Rust tests expanded from 94 to 99 tests.
- **P3-2 (settings test effectiveness)** — **Resolved**: `SettingsView.test.tsx` updated with explicit layout budget sentinels asserting that `SoundPicker` action buttons preserve `py-0.5`, path elements retain `truncate` and `font-mono`, and button containers maintain compact `gap-1.5` spacing. FE-M2 compaction mutation verified to turn test red.

## Residual risks (not defects — they need a packaged, multi-monitor GUI to close)

| # | Item |
| --- | --- |
| R-1 | The plans' own real-machine checks are unexecuted: settings-window drag-resize → close → reopen exact restore, tray label truncation, the settings version badge rendering (v1.1.3 plan §三-2), and the signed-package NSIS/MSI/portable upgrade, exit-install, elevation and failure-recovery smoke tests (v1.1.2 plan, "发布前验证"). |
| R-2 | `apply_settings_geometry` / `center_settings_window` (`src-tauri/src/tray.rs:514`, `:546`) have no test or mutation probe — the geometry restore chain is automated only up to the pure planner (`select_work_area` / `place_within_work_area`) and the config read/write path. This is the `tray.rs` counterpart of the round-4/5 `lib.rs` wiring concern. |
| R-3 | The settings-window version badge has no frontend assertion; only the tray menu label is covered (`src/components/TrayMenuView.test.tsx`). |
| R-4 | `README.md` / `README.en.md` do not describe the v1.1.3 user-visible behaviour (free settings-window resize, cross-process geometry memory). The v1.1.3 plan did not list README changes, so this is a coverage suggestion rather than a contract gap. |
| R-5 (v1.2.0) | **Closed** (round 2). The design proposal §3.2 case B and the §3.3.1 pseudocode comment now state the same totality contract as the shipped `select_monitor_by_overlap` (`src-tauri/src/dock.rs:561-599`): strictly largest intersection area wins, the host screen (`fallback_index`) breaks a tie that includes it, a non-host tie is broken by the smallest `(x, y)`, and zero overlap falls back to the host. §3.1's user-visible rule (>50 % ⇒ the target screen) holds either way. |
| R-6 (v1.2.0) | If Windows foreground lock keeps `set_focus()` failing *and* a transient `Focused(false)` arrived inside the grace window, the popup still hides at expiry (`pending_blur && !is_focused`, which is the specified contract). This is the extreme-focus variant of the symptom module 2 fixes; it can only be settled on a real machine with passthrough enabled. |

## Next steps

0. v1.2.0: close **P3-1** first — either make the action→window adapters injectable so the five call-site mutations (M1/M2/M3/M4/M9) each turn a test red, or declare them real-machine-only in this file and in the plan. Then **P3-2** — replace the tautological `SettingsView` "reachable" test with a real-layout assertion (headless Chromium against the built bundle) covering the "both alerts = custom sound + long path" configuration in 3 languages at 960×620 and 760×500, and make the `SoundPicker` compaction mutation fail. R-5 is closed.
1. Run the stage-0 Windows checks and the real signed-package smoke tests (NSIS / MSI / portable): upgrade, exit-install, elevation, failure recovery.
2. Verify on a real multi-monitor / mixed-DPI desktop: the v1.2.0 cross-screen drag attribution and seam防误折叠, the pill percentage grid at 100/150/200 % DPI, settings-window resize → close → reopen exact restore, display detach fallback, and the tray click/toggle timing with passthrough enabled (R-6).
3. Optional hardening: give `apply_settings_geometry` an offline falsifiability path (or an integration probe) and add a `SettingsView` assertion for `v{APP_VERSION}`.
4. Only then consider lifting the release gate; note that the v1.1.1 review's future-version items (transparency real-machine comparison, border-alpha coordination, the click-to-expand / narrow-label edge-hiding state machine) are explicitly **not** part of v1.1.2/v1.1.3 and remain unimplemented by design.
