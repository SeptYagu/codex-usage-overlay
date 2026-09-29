# Status

_Last updated: 2026-09-29 (v1.2.0 is now **implemented** at `998a558` and reviewed as code — independent code-review round 1 did not pass: 0×P0/P1, 1×P2, 1×P3. All five modules are implemented with correct call-site wiring and four gates are green, but the settings window clips its right column when both reset alerts use a custom sound, and module 2's fix has no falsifiable wiring. The v1.1.3 review line remains passed at 0 defects)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.2.0` (5 config files + `src/version.ts` unified; feature commit `998a558`) |
| Reviewed commit | `998a558` (`feat: implement v1.2.0 five modules across tray, dock, pill grid, and settings`) — v1.2.0 line |
| Baseline of that review | `6eeed35` (range `6eeed35..998a558`, 19 files / +1731 −343) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate (v1.1.3) | **Passed** — v1.1.3 round 5 (Mode C) closed the line with **0 defects**. Round 4's P3-1 is closed: `AppState::for_test` (`src-tauri/src/commands.rs:53-81`) plus the cache-read wiring tests make the capture/read/priority decisions falsifiable, and three acceptance mutations were each verified to fail tests. Rust tests 61 → 65 |
| Code review gate (v1.2.0) | **Not passed (round 1)** — the implementation review at `998a558` found **0×P0/P1 + 1×P2 + 1×P3**. All five modules are implemented with no omissions and the call-site wiring is correct: module 1 verified in a real layout engine (no row overflow at the 280 floor in en-US/zh-CN/zh-Hant; the long update row widens 280 ⇒ 284), module 4 verified end-to-end (9 ticks/track at exactly 10 % spacing, above the fill, 0 ticks when off, rotation preserved), module 3's attribution and module 4's allowlist→merge→save→reload chain verified falsifiable by mutation. Residuals: **P2-1** the settings window's right column needs 602–634 px in a fixed 486 px slot when both reset alerts use a custom sound, so the root (`h-screen overflow-hidden`, with `html`/`body` also `overflow: hidden`) clips it at y = 600 with no scrollbar — R-3's own `clientHeight == scrollHeight` is 620 vs 691 (en) / 620 vs 659 (zh, zh-Hant) and the weekly alert's sound controls are unreachable by mouse at the default size (a regression from the old `overflow-y-auto` root); **P3-1** module 2's wiring has no falsifiable assertion (M1 Hide-dispatch, M2 `rect()` `?`, M3 unconditional blur-hide, M4 missing grace anchor/timer, M9 anchor write all leave 88/88 Rust tests green). See `docs/handoff/2026-09-29-v1.2.0-code-review-round1-handoff.md` |
| Design review gate (v1.2.0) | **Superseded by implementation** — the design round-3 residuals (P3-1 landing seam, P3-2 §7.2.6 observation points, P3-3 unique attribution rule) were addressed in `998a558`: `merge_settings_patch` + `AppState::for_test` drive the write chain offline, `TrayMenuClickGate`/`TrayMenuFocusState` take an injected clock, and `select_monitor_by_overlap` now carries an explicit 5-branch totality contract with ≥3-screen / non-host-tie / zero-overlap / empty-list assertions. What remains is P3-1 above (the seams stop at the units, not at the call sites) plus one documentation divergence (design §3.2 prose vs the shipped strict-max rule). See `docs/handoff/2026-09-29-workbuddy-code-review-round3-handoff.md` |
| Compliance audit | **Compliant** — the v1.1.2 and v1.1.3 plans and the v1.1.1 behavior review were audited line-by-line against the code: every in-scope deliverable is implemented, with no omissions, reversals, or v1.1.2 regressions (`docs/handoff/2026-09-29-cross-plan-compliance-audit.md`) |

## Gates (reproduced locally on 2026-09-29 at `998a558`)

- `npm test` → 55/55 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 88/88 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass (0 warnings)

Green gates do **not** constitute review approval, and none of them exercise a real GUI. The v1.2.0 review additionally reproduced the gates at `998a558`, ran a real-layout-engine probe (headless Chromium on the built `dist/`, viewport pinned to exactly 960×620), and ran nine mutations in an isolated `git archive` sandbox sharing the existing `target/` directory.

## Open defects

None in the v1.1.3 review line. P3-1 (round 4) is closed and verified in round 5; CR1-1/CR1-2/CR1-3, CR2-1 (0×0 half), CR2-2 and CR3-1 remain closed and are not re-opened here.

Open in the **v1.2.0 implementation** reviewed at `998a558`:

- **P2-1 (settings-window clipping)** — with both reset alerts set to a custom sound, the right column is 602–634 px tall in a 486 px slot; the root is `h-screen overflow-hidden` and `html`/`body` are also `overflow: hidden`, so the overflow is clipped at y = 600 with no scrollbar and the weekly alert's sound `<select>` / path / `SoundPicker` buttons are unreachable by mouse at the default 960×620 size (keyboard focus does auto-scroll the clipped root; the window can also be resized taller). R-3's own acceptance `clientHeight == scrollHeight` reads 620 vs 691 (en-US) and 620 vs 659 (zh-CN / zh-Hant). Regression: the previous single-column root had `overflow-y-auto`, so every option was reachable.
- **P3-1 (module 2 falsifiability)** — the debounce/Toggle and grace-period *state machines* are covered and mutation-sensitive (verified: dropping the `pending_blur` half of the guard fails one test, deleting the debounce fails two), but the four call sites that make them effective — `handle_tray_click`'s dispatch, `layout_tray_menu`'s `rect()` degradation, `lib.rs`'s `Focused(false)` branch, and the `show()` → anchor → grace-timer sequence — have no failing assertion: reverting each to its pre-fix form (M1/M2/M3/M4, plus M9 for `drag_ended`'s anchor write) leaves 88/88 Rust tests green. The commit's wiring is correct; the gap is that nothing keeps it correct.

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
