# Status

_Last updated: 2026-09-29 (v1.1.3 line — independent code review round 5 passed; cross-plan compliance audit completed)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.3` (5 config files + `src/version.ts` unified) |
| Reviewed commit | `974f2f4` (`fix(review): resolve round 4 review findings (P3-1)`) |
| Baseline of that review | `4d58e94` (range `4d58e94..974f2f4`; round-5 fix commit `974f2f4` alone) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Passed** — v1.1.3 round 5 (Mode C) closed the line with **0 defects**. Round 4's P3-1 is closed: `AppState::for_test` (`src-tauri/src/commands.rs:47-75`) plus the cache-read wiring tests make the capture/read/priority decisions falsifiable, and three acceptance mutations were each verified to fail tests. Rust tests 61 → 65 |
| Compliance audit | **Compliant** — the v1.1.2 and v1.1.3 plans and the v1.1.1 behavior review were audited line-by-line against the code: every in-scope deliverable is implemented, with no omissions, reversals, or v1.1.2 regressions (`docs/handoff/2026-09-29-cross-plan-compliance-audit.md`) |

## Gates (reproduced locally on 2026-09-29 at `974f2f4`)

- `npm test` → 46/46 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 65/65 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass (0 warnings)

Green gates do **not** constitute review approval, and none of them exercise a real GUI.

## Open defects

None in the v1.1.3 review line. P3-1 (round 4) is closed and verified in round 5; CR1-1/CR1-2/CR1-3, CR2-1 (0×0 half), CR2-2 and CR3-1 remain closed and are not re-opened here.

## Residual risks (not defects — they need a packaged, multi-monitor GUI to close)

| # | Item |
| --- | --- |
| R-1 | The plans' own real-machine checks are unexecuted: settings-window drag-resize → close → reopen exact restore, tray label truncation, the settings version badge rendering (v1.1.3 plan §三-2), and the signed-package NSIS/MSI/portable upgrade, exit-install, elevation and failure-recovery smoke tests (v1.1.2 plan, "发布前验证"). |
| R-2 | `apply_settings_geometry` / `center_settings_window` (`src-tauri/src/tray.rs:232`, `:274`) have no test or mutation probe — the geometry restore chain is automated only up to the pure planner (`select_work_area` / `place_within_work_area`) and the config read/write path. This is the `tray.rs` counterpart of the round-4/5 `lib.rs` wiring concern. |
| R-3 | The settings-window version badge has no frontend assertion; only the tray menu label is covered (`src/components/TrayMenuView.test.tsx:111,117`). |
| R-4 | `README.md` / `README.en.md` do not describe the v1.1.3 user-visible behaviour (free settings-window resize, cross-process geometry memory). The v1.1.3 plan did not list README changes, so this is a coverage suggestion rather than a contract gap. |

## Next steps

1. Run the stage-0 Windows checks and the real signed-package smoke tests (NSIS / MSI / portable): upgrade, exit-install, elevation, failure recovery.
2. Verify on a real multi-monitor / mixed-DPI desktop: settings-window resize → close → reopen exact restore, display detach fallback (centered 480×660), the `resize → minimize → tray exit → restart` sequence, and the tray label / version badge rendering at 100/150/200% DPI.
3. Optional hardening: give `apply_settings_geometry` an offline falsifiability path (or an integration probe) and add a `SettingsView` assertion for `v{APP_VERSION}`.
4. Only then consider lifting the release gate; note that the v1.1.1 review's future-version items (transparency real-machine comparison, border-alpha coordination, the click-to-expand / narrow-label edge-hiding state machine) are explicitly **not** part of v1.1.2/v1.1.3 and remain unimplemented by design.
