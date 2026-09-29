# Status

_Last updated: 2026-09-29 (v1.1.3 line — independent code review, round 4)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.3` (5 config files + `src/version.ts` unified) |
| Reviewed commit | `321840b` (`fix(review): resolve round 3 review findings (CR3-1)`) |
| Baseline of that review | `4d58e94` (range `4d58e94..321840b`, 21 files, +1558 −54; round-4 fix commit `321840b` alone, 2 files / +248 −30) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Not passed** — CR3-1's functional implementation is closed and platform-source-verified (no counterexample found); 1×P3 remains (P3-1: the new pre-minimize capture wiring has no falsifiable test — disabling the capture point and both cache reads leaves 61/61 Rust tests green) |

## Gates (reproduced locally on 2026-09-29 at `321840b`)

- `npm test` → 46/46 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 61/61 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass (0 warnings)

Green gates do **not** constitute review approval.

## Open defects (details in `docs/handoff/2026-09-29-v1.1.3-code-review-round4-handoff.md`)

| ID | Sev | Summary |
| --- | --- | --- |
| P3-1 | P3 | The round-4 fix for CR3-1 adds a capture point (`lib.rs:77-87` → `remember_valid_settings_geometry` `lib.rs:336-349`) plus cache reads at both persist sites (`lib.rs:58`, `lib.rs:285`). The four new unit tests only exercise the pure functions, passing the cache in as an argument (`lib.rs:583`/`:620`/`:640`/`:664`), so the new capture and read functions have **zero** call coverage (they are referenced only from `run()`'s event wiring, hence never flagged as dead code). Mutation evidence (temp worktree, cleaned up): deleting the cache fallback fails 2 tests and removing the `is_minimized` guard fails 3 — but disabling the capture function **and** both cache reads at once leaves **61/61 green**. Round 3's §四 re-review criterion #2 ("fails when the cache **or the capture point** is broken") is therefore only half met: the fix's core mechanism has no automated evidence, and any later edit to `lib.rs:56-87` can silently restore the round-3 defect with all gates green. Fix: make the capture/read/priority decision testable with a bare `StdMutex::new(None)` and assert live-reading precedence over a stale cache. |

Closed in round 4 (verified, not re-opened): **CR3-1** — the round-4 capture point, both persist call sites, the guard ordering and the fresh-profile overwrite are correct; the three platform premises were re-verified in vendored source (`tauri-2.12.0/src/manager/window.rs:98-101` attaches `Builder::on_window_event` listeners to every window including the config-created settings window; `tao-0.37.1` emits `Moved`/`Resized` on `WM_WINDOWPOSCHANGED`/`WM_SIZE` with no visibility filter; `tao-0.37.1` `is_visible` queries `IsWindowVisible`, so a minimized window reads visible and a never-opened one does not). Also still closed: CR2-2/P3-1, the 0×0 half of CR2-1, CR1-1/CR1-2.

Notes on older lines: the v1.1.0-line defect R4-1 (P2, expanded-placement idempotency) and the closed items D1–D6/R3-1 belong to `docs/handoff/2026-09-28-workbuddy-code-review-round{1..4}-handoff.md` and are **not** re-opened here; the v1.1.2 execution row in the handoff index records 42/42 frontend and 42/42 Rust tests at that time.

## Next steps

1. Close P3-1: give the round-4 cache capture/read/priority decisions offline falsifiability (assert that a healthy live reading wins over a stale cache; make `remember_valid_settings_geometry`/`cached_settings_geometry` testable with a bare `StdMutex::new(None)`). Keep the CR3-1 functional semantics and the round-3 guards unchanged.
2. Re-review round 5 focuses only on P3-1 plus any new regression; CR3-1's implementation, CR2-1/CR2-2 and CR1-1/CR1-2 must stay closed.
3. Still outstanding from earlier lines: real signed-package smoke tests (NSIS/MSI/portable upgrades), the stage-0 Windows checks, and real multi-monitor/mixed-DPI GUI verification of the settings-window geometry restore (including the `minimize → tray exit → restart` sequence CR3-1 addresses).
