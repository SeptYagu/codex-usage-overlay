# Status

_Last updated: 2026-09-28 (independent code review, round 4 recheck)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.1.0` (preview line; preview tag form `vX.Y.Z-preview`) |
| Reviewed commit | `86951ae` (`fix(dock): guarantee final resized window covers mouse cursor on edge pill`) |
| Baseline of that review | `693a005` (round-4 recheck range `1ab82ad..86951ae`) |
| Release gate | **Blocked** — stage 0 Windows checks unfinished (see `docs/stage0-verification.md`) |
| Code review gate | **Not passed** — R3-1 closed and independently verified; 1×P2 open (R4-1: expanded placement is not idempotent — background repositions follow the live cursor and can drop it outside the window) |

## Gates (reproduced locally on 2026-09-28 at `86951ae`)

- `npm test` → 37/37 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 40/40 pass
- `cargo check --manifest-path src-tauri/Cargo.toml --locked` → pass

Green gates do **not** constitute review approval.

## Open defects (details in `docs/handoff/`)

| ID | Sev | Summary |
| --- | --- | --- |
| R4-1 | P2 | `place_docked_expanded` derives the expanded window's free axis from the **live cursor** and applies the "cover the cursor" clamp only while the cursor is still inside the pill hit rect. That hit rect (141 physical px wide on left/right edges) is far smaller than the expanded capsule (871 px), so `keep_docked_in_work_area` (every usage refresh, default 60 s; also settings/DPI changes) and `window_resized` (any content-size write-back) re-place the window from wherever the cursor happens to be: the free axis travels 96 physical px (~55 logical px, ~45% of the capsule height) within one session, and when the cursor leaves the hit rect the window snaps back to the anchor-centred position, which can be 46–48 px away from a cursor still resting on the capsule — the geometric precondition for `mouseleave` → 400 ms collapse. Spec §3.4's claim that both reposition paths "always yield the same rectangle and never fight each other" only holds for a frozen cursor. |

Closed in round 2: D2 (work area resolved from the anchor's monitor, with a fallback for unplugged displays), D3 (a stale/regressed `resetsAt` no longer aborts the pending post-reset fetch; the accepted boundary is what arms the timer), D4 (`is_interacting()` stands down `keep_docked_in_work_area` while dragging or while the menu is open), D5 (`SoundMode` falls back to the default on unknown values without resetting other preferences). D1's size axis (hardcoded size + frontend size cache) is closed and independently probed. D6's transition-rect axis is closed in round 3; R3-1's settled-rect axis is closed in round 4 and independently verified (94,150,080 cursor-coverage cases, 0 counterexamples).

## Next steps

1. Fix R4-1: make the expanded-session placement idempotent — record the settled rectangle when the authoritative size write-back lands and reuse it for the rest of the session (or, at minimum, never re-place the window so that a cursor point that was inside the previous rectangle ends up outside). Add the two pure-geometry tests from the round-4 acceptance criteria, and remove the over-strong "never fight each other" wording from spec §3.4 (`docs/v1.1.0-technical-spec.md:414`).
2. Close the settle gap between the frontend `setSize` and the backend `set_position` (round-4 pending risk 1): during that gap the window's free axis is 95 physical px shorter than the transition rectangle, so a cursor parked on the far outer band is transiently excluded.
3. Record the Windows smoke checks required by rounds 1–3 (D1's four trigger paths, D2's multi-monitor restart, hover-expand on all four edges with the cursor held still, and now: hold the cursor inside the capsule for ≥2 minutes across ≥2 usage refreshes without the window moving or collapsing) plus the §3.4 size measurements in the preview notes.
4. Re-review round 5 focuses only on R4-1 plus any new regression; D2–D6 and R3-1 must stay closed.
