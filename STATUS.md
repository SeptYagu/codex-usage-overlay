# Status

_Last updated: 2026-09-28 (v1.1.3 line — independent code review, round 1)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.3` (5 config files + `src/version.ts` unified) |
| Reviewed commit | `1e363b6` (`feat: settings window geometry persistence, version displays, and bump to 1.1.3`) |
| Baseline of that review | `4d58e94` (range `4d58e94..1e363b6`, 1 commit / 15 files, +326 −32) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Not passed** — 1×P2 (CR1-1: mixed-DPI monitor selection in settings-window geometry restore) + 2×P3 (CR1-2 tray menu width is a hardcoded constant, "adaptive width" acceptance item has no code path; CR1-3 geometry is only persisted on `CloseRequested`, the tray-exit path loses the adjustment) |

## Gates (reproduced locally on 2026-09-28 at `1e363b6`)

- `npm test` → 43/43 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 43/43 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass

Green gates do **not** constitute review approval.

## Open defects (details in `docs/handoff/2026-09-28-v1.1.3-code-review-round1-handoff.md`)

| ID | Sev | Summary |
| --- | --- | --- |
| CR1-1 | P2 | Settings-window geometry is saved as a *logical* rect whose scale is bound to the monitor the window was on (`lib.rs:57-66`), but restore re-projects that rect into physical space using **each candidate monitor's own** `scale_factor()` (`tray.rs:287-304`) and then takes the **first** intersecting monitor (`tray.rs:240-249`). In mixed-DPI layouts this yields a false positive on a monitor the window was never on, so the window is restored to the wrong screen (independent replica: primary-first enumeration restores to `PRIMARY-100% pos=(1333,133)` instead of `SECONDARY-150% pos=(2000,200)`; same-DPI dual-monitor is correct). |
| CR1-2 | P3 | The tray popup width is the constant `300 * scale` (`tray.rs:148`); `layout_tray_menu` accepts only `height_logical` (`tray.rs:101`) and `measureMenu()` measures only height (`TrayMenuView.tsx:47-66`). Acceptance item 3's "adaptive width" therefore has no code path, and the plan's claim that `measureMenu` recomputes the popup size to keep the text unwrapped is not backed by code. No visible truncation today (three-language labels ≈165–205 px vs 260 px available). |
| CR1-3 | P3 | Geometry is persisted only on the settings window's `CloseRequested` (`lib.rs:55-70`, the single `save_settings_geometry` call site); `RunEvent::Exit` (`lib.rs:253-258`) has no fallback. Resizing the window and then exiting straight from the tray loses the adjustment. |

Notes on older lines: the v1.1.0-line defect R4-1 (P2, expanded-placement idempotency) and the closed items D1–D6/R3-1 belong to `docs/handoff/2026-09-28-workbuddy-code-review-round{1..4}-handoff.md` and are **not** re-opened here; the v1.1.2 execution row in the handoff index records 42/42 frontend and 42/42 Rust tests at that time.

## Next steps

1. Fix CR1-1: persist the save-time monitor scale (or the physical rect plus the monitor identity) and select the target monitor by largest overlap instead of first hit; add pure-geometry tests for mixed-DPI layouts in both enumeration orders.
2. Fix CR1-3 in the same edit: add a `RunEvent::Exit` fallback (or a debounced `Resized`/`Moved` save).
3. Fix CR1-2: report the measured width to `layout_tray_menu`, or align the acceptance wording/plan with "no truncation" and assert it in a test.
4. Re-review round 2 focuses only on CR1-1..CR1-3 plus any new regression; the items above must all be closed.
5. Still outstanding from earlier lines: real signed-package smoke tests (NSIS/MSI/portable upgrades) and the stage-0 Windows checks.
