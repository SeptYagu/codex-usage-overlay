# Status

_Last updated: 2026-09-28 (independent code review, round 3 recheck)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.0` (preview line; preview tag form `vX.Y.Z-preview`) |
| Reviewed commit | `b718b55` (`fix(dock): ensure expand transition rectangle covers edge pill`) |
| Baseline of that review | `693a005` (round-3 recheck range `c8d646e..b718b55`) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Not passed** — round-2 D6 closed on the transition axis; 1×P2 open (R3-1: final rect after the size write-back still excludes the cursor) |

## Gates (reproduced locally on 2026-09-28 at `b718b55`)

- `npm test` → 37/37 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --offline --locked` → 35/35 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --offline --locked` → pass

Green gates do **not** constitute review approval.

## Open defects (details in `docs/handoff/`)

| ID | Sev | Summary |
| --- | --- | --- |
| R3-1 | P2 | Round-2 D6 is fixed for the *transition* rect only. `place_full_for_transition`/`transition_rect` guarantee the rect written by `mouse_enter` contains the pill (independently verified, 165,888 in-range parameter combos, 0 counterexamples). But the size write-back then runs `window_resized` → `place_full_at_anchor`, which lands the window at the saved drop anchor with the real capsule size (211 physical px tall) while the pill is 306 physical px tall; the pill's outer ~47.5 px bands (31% of its area) on the left/right edges fall outside the settled rect, so a cursor that entered there is dropped → 400 ms later the overlay collapses. Spec §3.4's "the expanded rect must cover the mouse's current pill hit area" is therefore still unmet in the settled state. |

Closed in round 2: D2 (work area resolved from the anchor's monitor, with a fallback for unplugged displays), D3 (a stale/regressed `resetsAt` no longer aborts the pending post-reset fetch; the accepted boundary is what arms the timer), D4 (`is_interacting()` stands down `keep_docked_in_work_area` while dragging or while the menu is open), D5 (`SoundMode` falls back to the default on unknown values without resetting other preferences). D1's size axis (hardcoded size + frontend size cache) is closed and independently probed. D6's transition-rect axis is closed in round 3.

## Next steps

1. Fix R3-1: make the post-write-back placement contain the cursor (union with the pill/cursor rect, or redefine §3.4's target to "cover the cursor position" and clamp to that point, updating the spec text), then add a pure-geometry Rust test asserting the *settled* rect contains the pill/cursor.
2. Let `keep_docked_in_work_area` stand down during the expand transition (see round-3 pending risk 1).
3. Record the Windows smoke checks required by rounds 1–2 (D1's four trigger paths, D2's multi-monitor restart, hover-expand on all four edges with the cursor held still) and the §3.4 size measurements in the preview notes.
4. Re-review round 4 focuses only on R3-1 plus any new regression; D2–D6 must stay closed.
