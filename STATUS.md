# Status

_Last updated: 2026-09-29 (v1.2.0 round-1 review findings resolved: P2-1 closed via column-level `overflow-y-auto pr-1` and SoundPicker compaction, P3-1 closed via seam functions and falsifiable action assertions across M1, M2, M3, M4, M9, and R-5 design §3.2 doc sync complete. Four gates green: npm test 56/56, build, cargo test 94/94, cargo check 0 warnings)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.2.0` (5 config files + `src/version.ts` unified; feature commit `998a558`) |
| Reviewed commit | `998a558` (`feat: implement v1.2.0 five modules across tray, dock, pill grid, and settings`) — v1.2.0 line |
| Baseline of that review | `6eeed35` (range `6eeed35..998a558`, 19 files / +1731 −343) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate (v1.1.3) | **Passed** — v1.1.3 round 5 (Mode C) closed the line with **0 defects**. Round 4's P3-1 is closed: `AppState::for_test` (`src-tauri/src/commands.rs:53-81`) plus the cache-read wiring tests make the capture/read/priority decisions falsifiable, and three acceptance mutations were each verified to fail tests. Rust tests 61 → 65 |
| Code review gate (v1.2.0) | **Resolving (round 2 dispatched)** — round 1 findings P2-1 (settings column scrollability) and P3-1 (wiring falsifiability) addressed in code and tests. Rust tests 88 → 94, frontend tests 55 → 56 |
| Design review gate (v1.2.0) | **Superseded by implementation** — the design round-3 residuals (P3-1 landing seam, P3-2 §7.2.6 observation points, P3-3 unique attribution rule) were addressed in `998a558` and doc sync R-5 completed. See `docs/handoff/2026-09-29-workbuddy-code-review-round3-handoff.md` |
| Compliance audit | **Compliant** — the v1.1.2 and v1.1.3 plans and the v1.1.1 behavior review were audited line-by-line against the code: every in-scope deliverable is implemented, with no omissions, reversals, or v1.1.2 regressions (`docs/handoff/2026-09-29-cross-plan-compliance-audit.md`) |

## Gates (reproduced locally on 2026-09-29 after round 1 fixes)

- `npm test` → 56/56 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 94/94 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass (0 warnings)

Green gates do **not** constitute review approval, and none of them exercise a real GUI.

## Open defects

None in the v1.1.3 review line.

In the **v1.2.0 implementation**:
- **P2-1 (settings-window clipping)** — **Resolved**: both `<section>` columns now have `min-h-0 overflow-y-auto pr-1`; `SoundPicker` path shortened to single-line `truncate font-mono` with tooltip title and `py-0.5` button footprint. Under default settings, height is <486px so no scrollbar appears (default zero-scroll). Under custom sounds with long paths, column scrolls internally, all buttons remain fully reachable, and root `clientHeight == scrollHeight` is strictly maintained. Covered by `SettingsView.test.tsx`.
- **P3-1 (module 2 and dock wiring falsifiability)** — **Resolved**: seam functions with pure action sinks and assertions extracted:
  - M1: `plan_tray_click_effect` (`TrayMenuWindowEffect`), tested by `tray_click_effect_dispatches_hide_and_never_reopens`.
  - M2: `resolve_tray_icon_anchor_box`, tested by `tray_icon_anchor_box_degrades_when_tray_rect_fails`.
  - M3: `should_hide_on_tray_menu_blur` and `handle_tray_menu_focus_event`, tested by `blur_decision_inside_grace_period_defers_and_outside_hides` and `tray_menu_focus_event_hides_only_when_blur_evaluator_approves`.
  - M4: `drive_tray_menu_show_sequence` (`TrayMenuShowAction`), tested by `show_sequence_emits_note_shown_and_grace_check_in_exact_order`.
  - M9: `apply_drag_outcome_to_session`, tested by `drag_outcome_persists_settled_anchor_and_never_constants`.

Details, evidence and per-item acceptance criteria: `docs/handoff/2026-09-29-v1.2.0-code-review-round1-handoff.md`.

## Residual risks (not defects — they need a packaged, multi-monitor GUI to close)

| # | Item |
| --- | --- |
| R-1 | The plans' own real-machine checks are unexecuted: settings-window drag-resize → close → reopen exact restore, tray label truncation, the settings version badge rendering (v1.1.3 plan §三-2), and the signed-package NSIS/MSI/portable upgrade, exit-install, elevation and failure-recovery smoke tests (v1.1.2 plan, "发布前验证"). |
| R-2 | `apply_settings_geometry` / `center_settings_window` (`src-tauri/src/tray.rs:514`, `:546`) have no test or mutation probe — the geometry restore chain is automated only up to the pure planner (`select_work_area` / `place_within_work_area`) and the config read/write path. This is the `tray.rs` counterpart of the round-4/5 `lib.rs` wiring concern. |
| R-3 | The settings-window version badge has no frontend assertion; only the tray menu label is covered (`src/components/TrayMenuView.test.tsx`). |
| R-4 | `README.md` / `README.en.md` do not describe the v1.1.3 user-visible behaviour (free settings-window resize, cross-process geometry memory). The v1.1.3 plan did not list README changes, so this is a coverage suggestion rather than a contract gap. |
| R-5 (v1.2.0) | The v1.2.0 design proposal §3.2 still describes the ≤50 % case as "keep the original screen", while the shipped `select_monitor_by_overlap` (`src-tauri/src/dock.rs:560-599`) uses strict-max-area with a host tie-break — the rule the design round-3 review recommended. They differ only when ≥3 screens are attached and no screen holds >50 %; §3.1's user-visible rule (>50 % ⇒ the target screen) holds either way. Needs a one-line doc sync, not a code change. |
| R-6 (v1.2.0) | If Windows foreground lock keeps `set_focus()` failing *and* a transient `Focused(false)` arrived inside the grace window, the popup still hides at expiry (`pending_blur && !is_focused`, which is the specified contract). This is the extreme-focus variant of the symptom module 2 fixes; it can only be settled on a real machine with passthrough enabled. |

## Next steps

0. v1.2.0: close **P2-1** first (make both settings columns scrollable as the safety net and/or compact the right column; R-3's screenshot step must then include the "both alerts = custom sound + long path" configuration), then **P3-1** (extract the tray-menu show/dispatch/blur paths into seam functions with an injectable action sink, and verify M1–M4/M9 each turn a test red). The doc sync R-5 can ride along.
1. Run the stage-0 Windows checks and the real signed-package smoke tests (NSIS / MSI / portable): upgrade, exit-install, elevation, failure recovery.
2. Verify on a real multi-monitor / mixed-DPI desktop: the v1.2.0 cross-screen drag attribution and seam防误折叠, the pill percentage grid at 100/150/200 % DPI, settings-window resize → close → reopen exact restore, display detach fallback, and the tray click/toggle timing with passthrough enabled (R-6).
3. Optional hardening: give `apply_settings_geometry` an offline falsifiability path (or an integration probe) and add a `SettingsView` assertion for `v{APP_VERSION}`.
4. Only then consider lifting the release gate; note that the v1.1.1 review's future-version items (transparency real-machine comparison, border-alpha coordination, the click-to-expand / narrow-label edge-hiding state machine) are explicitly **not** part of v1.1.2/v1.1.3 and remain unimplemented by design.
