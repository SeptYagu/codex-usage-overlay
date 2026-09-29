# Status

_Last updated: 2026-09-29 (v1.2.0 design proposal — independent review round 3, the terminal design-review round, did not pass: 0×P0/P1/P2, 3×P3; round 2's four P3s are closed as wording, but three of the new "must fail" acceptance gates still have no offline landing point; the v1.1.3 review line remains passed at 0 defects)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.3` (5 config files + `src/version.ts` unified); v1.2.0 is a design proposal only |
| Reviewed commit | `974f2f4` (`fix(review): resolve round 4 review findings (P3-1)`) — v1.1.3 line |
| Baseline of that review | `4d58e94` (range `4d58e94..974f2f4`; round-5 fix commit `974f2f4` alone) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate (v1.1.3) | **Passed** — v1.1.3 round 5 (Mode C) closed the line with **0 defects**. Round 4's P3-1 is closed: `AppState::for_test` (`src-tauri/src/commands.rs:47-75`) plus the cache-read wiring tests make the capture/read/priority decisions falsifiable, and three acceptance mutations were each verified to fail tests. Rust tests 61 → 65 |
| Design review gate (v1.2.0) | **Not passed (round 3, terminal design round)** — the round-3 recheck of the post-fix proposal at `3f774b1` (range `78d7e96..3f774b1`, docs only) found **0×P0/P1/P2 + 3×P3**. Round 2's P3-1..P3-4 are verified as landed *as wording* (unique `pending_blur` guard + third assertion, `textW` 260⇒300 / 340⇒380 + jsdom double spec, debounce/Toggle assertions + injectable clock, allowlist→write→read-back chain), and R-1/R-2/R-3 are closed at the function level (the 2-screen 50/50 tie-break is order-independent — 60-case permutation sweep, 0 counterexamples; empty monitor list returns `None`). The three residuals are *landing* gaps on the new text: P3-1 the §7.2.1 write assertion is pinned to `apply_settings_patch`, which cannot be called offline (it needs `&AppHandle`, the repo has no `[dev-dependencies]` and no `tauri/test`), P3-2 §7.2.6's hide/visibility assertions are pinned to live-window observables with no named seam (⑥ covers only the debounce clock), P3-3 the `select_monitor_by_overlap` rule is not unique across prose/comment/pseudocode (≥3 screens: order-sensitive 50/50 between two non-host screens, and the "≤50% keeps the host" reading differs from the max-area pseudocode; all §7.2.2 assertions are 2-screen). See `docs/handoff/2026-09-29-workbuddy-code-review-round3-handoff.md` |
| Compliance audit | **Compliant** — the v1.1.2 and v1.1.3 plans and the v1.1.1 behavior review were audited line-by-line against the code: every in-scope deliverable is implemented, with no omissions, reversals, or v1.1.2 regressions (`docs/handoff/2026-09-29-cross-plan-compliance-audit.md`) |

## Gates (reproduced locally on 2026-09-29 at `974f2f4`)

- `npm test` → 46/46 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 65/65 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass (0 warnings)

Green gates do **not** constitute review approval, and none of them exercise a real GUI.

## Open defects

None in the v1.1.3 review line. P3-1 (round 4) is closed and verified in round 5; CR1-1/CR1-2/CR1-3, CR2-1 (0×0 half), CR2-2 and CR3-1 remain closed and are not re-opened here.

Open in the **v1.2.0 design line** (proposal at `3f774b1`, not yet implemented): round 1's P2-1 + P3-1…P3-5 and round 2's P3-1…P3-4 are closed *as specified*, but round 3 (the terminal design round) keeps 3×P3 open on the *landing* of the new acceptance gates — P3-1 (§7.2.1 pins the write assertion to `apply_settings_patch`, unreachable from an offline `#[test]`), P3-2 (§7.2.6's hide/visibility assertions have no named offline seam; ⑥ covers only the debounce clock), P3-3 (the monitor-attribution rule is not unique across prose/comment/pseudocode; R-2's order-independence holds only when the host is in the tie) — details and per-item acceptance criteria in `docs/handoff/2026-09-29-workbuddy-code-review-round3-handoff.md`. R-1's new `Option` return still has no caller-side contract in `drag_ended` (recorded there as a risk, not a defect).

## Residual risks (not defects — they need a packaged, multi-monitor GUI to close)

| # | Item |
| --- | --- |
| R-1 | The plans' own real-machine checks are unexecuted: settings-window drag-resize → close → reopen exact restore, tray label truncation, the settings version badge rendering (v1.1.3 plan §三-2), and the signed-package NSIS/MSI/portable upgrade, exit-install, elevation and failure-recovery smoke tests (v1.1.2 plan, "发布前验证"). |
| R-2 | `apply_settings_geometry` / `center_settings_window` (`src-tauri/src/tray.rs:232`, `:274`) have no test or mutation probe — the geometry restore chain is automated only up to the pure planner (`select_work_area` / `place_within_work_area`) and the config read/write path. This is the `tray.rs` counterpart of the round-4/5 `lib.rs` wiring concern. |
| R-3 | The settings-window version badge has no frontend assertion; only the tray menu label is covered (`src/components/TrayMenuView.test.tsx:111,117`). |
| R-4 | `README.md` / `README.en.md` do not describe the v1.1.3 user-visible behaviour (free settings-window resize, cross-process geometry memory). The v1.1.3 plan did not list README changes, so this is a coverage suggestion rather than a contract gap. |

## Next steps

0. v1.2.0: close the round-3 residuals **before** any implementation work — P3-3 (unique attribution rule + ≥3-screen / order-independence / zero-overlap assertions), then P3-2 (name the offline seam for §7.2.6's grace and visibility assertions), then P3-1 (move the write assertion onto a seam that runs under `cargo test --locked`); the review's §四 lists the per-item re-review criteria. R-1's `None`-branch contract in `drag_ended` and T-1's DPI screenshots can ride along in the same round.
1. Run the stage-0 Windows checks and the real signed-package smoke tests (NSIS / MSI / portable): upgrade, exit-install, elevation, failure recovery.
2. Verify on a real multi-monitor / mixed-DPI desktop: settings-window resize → close → reopen exact restore, display detach fallback (centered 480×660), the `resize → minimize → tray exit → restart` sequence, and the tray label / version badge rendering at 100/150/200% DPI.
3. Optional hardening: give `apply_settings_geometry` an offline falsifiability path (or an integration probe) and add a `SettingsView` assertion for `v{APP_VERSION}`.
4. Only then consider lifting the release gate; note that the v1.1.1 review's future-version items (transparency real-machine comparison, border-alpha coordination, the click-to-expand / narrow-label edge-hiding state machine) are explicitly **not** part of v1.1.2/v1.1.3 and remain unimplemented by design.
