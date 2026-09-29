# Status

_Last updated: 2026-09-28 (v1.1.3 line — independent code review, round 2)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.3` (5 config files + `src/version.ts` unified) |
| Reviewed commit | `ba81595` (`fix(review): resolve round 1 review findings (P2-1, P3-1, P3-2)`) |
| Baseline of that review | `4d58e94` (range `4d58e94..ba81595`, 3 commits / 19 files, +893 −54; round-2 fix commit `1bb685b..ba81595`) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Not passed** — CR1-1 (P2) and CR1-2 (P3) from round 1 are closed and independently verified; the round-1 CR1-3 fix is implemented but introduces 1×P2 (CR2-1: exit-save of a minimized settings window writes a 0×0 geometry record and destroys the previously valid one) + 1×P3 (CR2-2: tray-menu adaptive-width test can never fail) |

## Gates (reproduced locally on 2026-09-28 at `ba81595`)

- `npm test` → 43/43 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 53/53 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass (0 warnings)

Green gates do **not** constitute review approval.

## Open defects (details in `docs/handoff/2026-09-28-v1.1.3-code-review-round2-handoff.md`)

| ID | Sev | Summary |
| --- | --- | --- |
| CR2-1 | P2 | The new `RunEvent::Exit` fallback (`lib.rs:253-266`) guards only on `window.is_visible()`. `IsWindowVisible` is `TRUE` for a minimized window, while `inner_size()` (= `GetClientRect`, tao `window.rs:219-225`) returns `0×0`. `persist_settings_geometry` (`lib.rs:286-297`) validates only the scale, so quitting from the tray with the settings window minimized overwrites `settings-window-geometry.json` with `"width":0.0,"height":0.0`; `load_settings_geometry` (`config.rs:212-218`) then rejects it and the window degrades to centred 480×660 — the saved geometry is destroyed. Regression relative to round 1, which had no exit-path save. Verified: native probe on a minimized window reports `visible=true iconic=true client=0x0`. |
| CR2-2 | P3 | `TrayMenuView.test.tsx:30-32` mocks `scrollWidth` to a constant `280`, so `intrinsicWidth` is always 280 and `width` always 300 = `TRAY_MENU_MIN_WIDTH`; all three assertions expect `widthLogical: 300`. The adaptive-width aggregation and clamp in `TrayMenuView.tsx:55-59` can be broken entirely without failing the test (test-effectiveness gap only — the production path was independently confirmed working in a real Chromium). |

Closed in round 2 (verified, not re-opened): CR1-1 (mixed-DPI monitor selection — scale-factor anchor + largest-overlap pick + pure-geometry tests; re-verified order-invariant, negative-coordinate, detached-display and clamp behaviour with an independent replica), CR1-2 (tray-menu adaptive width — `widthLogical` plumbing + `nowrap` + clamp; `scrollWidth` overflow detection confirmed in real Chromium).

Notes on older lines: the v1.1.0-line defect R4-1 (P2, expanded-placement idempotency) and the closed items D1–D6/R3-1 belong to `docs/handoff/2026-09-28-workbuddy-code-review-round{1..4}-handoff.md` and are **not** re-opened here; the v1.1.2 execution row in the handoff index records 42/42 frontend and 42/42 Rust tests at that time.

## Next steps

1. Fix CR2-1 by rejecting zero-sized geometry inside `persist_settings_geometry` (covers both the `CloseRequested` and `RunEvent::Exit` call sites); optionally also skip when `is_minimized()`.
2. Fix CR2-2 by adding a case whose mocked `scrollWidth` exceeds the 300 floor (and one above the 500 clamp) so the adaptive-width logic is falsifiable.
3. Re-review round 3 focuses only on CR2-1/CR2-2 plus any new regression; CR1-1/CR1-2 must stay closed.
4. Still outstanding from earlier lines: real signed-package smoke tests (NSIS/MSI/portable upgrades), the stage-0 Windows checks, and real multi-monitor/mixed-DPI GUI verification of the settings-window geometry restore.
