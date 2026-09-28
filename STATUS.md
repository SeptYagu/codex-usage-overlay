# Status

_Last updated: 2026-09-28 (independent code review, round 1)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.0` (preview line; preview tag form `vX.Y.Z-preview`) |
| Reviewed commit | `2a97264` (`chore: ignore .workbuddy directory in gitignore`) |
| Baseline of that review | `693a005` |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Not passed** — 1×P1, 1×P2, 3×P3 open |

## Gates (reproduced locally on 2026-09-28 at `2a97264`)

- `npm test` → 35/35 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 30/30 pass

Green gates do **not** constitute review approval.

## Open defects (details in `docs/handoff/`)

| ID | Sev | Summary |
| --- | --- | --- |
| D1 | P1 | Rust-driven resizes write the hardcoded 220×50×scale approximation while the frontend `lastSizeRef` cache suppresses re-applying the content-fitted size → the overlay is clipped/shifted after tray show–hide, pill hover-expand, and disabling auto edge hide. |
| D2 | P2 | Startup anchor restore clamps into the work area of the monitor the window *currently* occupies, not the monitor of the saved anchor → multi-monitor/negative-coordinate positions are lost on restart. |
| D3 | P3 | A stale/regressed `resetsAt` aborts the pending post-boundary refresh for the current cycle. |
| D4 | P3 | `keep_docked_in_work_area` ignores the `dragging` lock, so a usage refresh completing mid-drag repositions the window. |
| D5 | P3 | New `SoundMode` field lacks the `#[serde(other)]` guard used by `OverlayLayout`; an unknown value resets all saved preferences. |

## Next steps

1. Fix D1 → D2 → D3/D4/D5 and re-run all gates.
2. Record the Windows smoke checks (dragging, pill hover, four edges, multi-monitor, notifications, audio) in the preview notes.
3. Re-review round 2 focuses only on whether D1–D5 are closed and whether the fixes introduce regressions.
