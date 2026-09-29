# Status

_Last updated: 2026-09-29 (v1.1.3 line — independent code review, round 3)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.3` (5 config files + `src/version.ts` unified) |
| Reviewed commit | `914d3ac` (`fix(review): resolve round 2 review findings (P2-1, P3-1)`) |
| Baseline of that review | `4d58e94` (range `4d58e94..914d3ac`, 4 commits / 20 files, +1234 −54; round-3 fix commit `914d3ac` alone, 3 files / +230 −12) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Not passed** — round 2's P3-1/CR2-2 and the "a 0×0 reading must never overwrite a valid record" half of P2-1/CR2-1 are closed and independently falsified; 1×P2 remains (CR3-1: with a minimized settings window, quitting from the tray — or closing it via the taskbar — still drops the pre-minimize move/resize, because no capture point exists between the move and the minimize) |

## Gates (reproduced locally on 2026-09-29 at `914d3ac`)

- `npm test` → 46/46 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 57/57 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass (0 warnings)

Green gates do **not** constitute review approval.

## Open defects (details in `docs/handoff/2026-09-29-v1.1.3-code-review-round3-handoff.md`)

| ID | Sev | Summary |
| --- | --- | --- |
| CR3-1 | P2 | The round-3 fix rejects unusable readings (`lib.rs:300-306` early-returns on `is_minimized` / zero size; `lib.rs:262-264` guards `Exit` on `is_visible && !is_minimized`), which correctly stops the 0×0 overwrite — but the repo has only two persist sites (`CloseRequested` `lib.rs:57`, `RunEvent::Exit` `lib.rs:266`) and `on_window_event` handles no `Moved`/`Resized` for the settings window (`lib.rs:53-83`). So in the round-2 acceptance sequence (open settings → move/resize → minimize → tray exit → restart) nothing is ever written: the file is either absent or stale, and `tray.rs:247-250`/`274-280` centres the window at the 480×660 default (or restores the pre-move record). Round 2 acceptance criteria #1/#2 ("restore the geometry as it was *before* minimizing") are therefore unmet. Not a regression versus round 2 (which wrote 0×0 and destroyed the record), but the version's headline feature still silently fails on that path. |

Closed in round 3 (verified, not re-opened): CR2-2/P3-1 (tray-menu adaptive width — per-element `scrollWidth` mock plus new widening/clamp/floor cases; mutation-tested: removing the aggregation and raising the cap to 800 fails exactly `widens` + `clamps`), the 0×0 half of CR2-1 (mutation-tested: removing both guards fails `persist_skips_zero_sized_geometry` + `persist_skips_minimized_windows`), and CR1-1/CR1-2 (kept closed).

Notes on older lines: the v1.1.0-line defect R4-1 (P2, expanded-placement idempotency) and the closed items D1–D6/R3-1 belong to `docs/handoff/2026-09-28-workbuddy-code-review-round{1..4}-handoff.md` and are **not** re-opened here; the v1.1.2 execution row in the handoff index records 42/42 frontend and 42/42 Rust tests at that time.

## Next steps

1. Fix CR3-1 by adding a capture point for the geometry as it was before minimising — either debounced persistence on the settings window's `Moved`/`Resized` events (the `v1.1.3-update-plan.md` §2 wording), or caching the last valid non-minimised geometry and writing that cache when the live reading is rejected. Do **not** relax the round-3 guards.
2. Re-review round 4 focuses only on CR3-1 plus any new regression; CR2-1/CR2-2 and CR1-1/CR1-2 must stay closed.
3. Still outstanding from earlier lines: real signed-package smoke tests (NSIS/MSI/portable upgrades), the stage-0 Windows checks, and real multi-monitor/mixed-DPI GUI verification of the settings-window geometry restore.
