# Status

_Last updated: 2026-09-28 (independent code review, round 2 recheck)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.0` (preview line; preview tag form `vX.Y.Z-preview`) |
| Reviewed commit | `c8d646e` (`fix(review): resolve round 1 review findings`) |
| Baseline of that review | `693a005` (round-1 recheck range `2a97264..c8d646e`) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Not passed** — round 1 D2/D3/D4/D5 closed, D1 closed on the size axis only; 1×P2 open (D6) |

## Gates (reproduced locally on 2026-09-28 at `c8d646e`)

- `npm test` → 37/37 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --offline --locked` → 34/34 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --offline --locked` → pass

Green gates do **not** constitute review approval.

## Open defects (details in `docs/handoff/`)

| ID | Sev | Summary |
| --- | --- | --- |
| D6 | P2 | Round-1 D1 is only closed on the size axis. `set_full_size` still writes the hardcoded estimate (220×50×scale) and `place_full_at_anchor` restores the saved drop anchor, while the collapsed pill is pinned to the work-area edge by `pill_geometry`. The rect written during the hover-expand transition therefore does not cover the pill hit area required by spec §3.4: 0% for a right-edge dock (every legal drop offset), 42–62% for bottom, 70–84% for left, 83–100% for top. |

Closed in round 2: D2 (work area resolved from the anchor's monitor, with a fallback for unplugged displays), D3 (a stale/regressed `resetsAt` no longer aborts the pending post-reset fetch; the accepted boundary is what arms the timer), D4 (`is_interacting()` stands down `keep_docked_in_work_area` while dragging or while the menu is open), D5 (`SoundMode` falls back to the default on unknown values without resetting other preferences). D1's size axis (hardcoded size + frontend size cache) is closed and independently probed.

## Next steps

1. Fix D6: make the hover-expand transition rect cover the pill hit area (union of the clamped anchor rect and the current pill rect, or split "restore user anchor" from "transition placement"), then add a pure-geometry Rust test for all four edges.
2. Record the Windows smoke checks required by round 1 (D1's four trigger paths, D2's multi-monitor restart) in the preview notes, and state round-1 risks 1/2 explicitly as preview known limitations.
3. Re-review round 3 focuses only on D6 plus any new regression; D2–D5 must stay closed.
