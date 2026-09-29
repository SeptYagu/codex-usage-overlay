# Handoff index

| Date | Round | Reviewed commit | Verdict | Document |
| --- | --- | --- | --- | --- |
| 2026-09-28 | 1 (recheck) | `2a97264` (base `693a005`) | Not passed — 1×P1, 1×P2, 3×P3 | [2026-09-28-workbuddy-code-review-round1-handoff.md](2026-09-28-workbuddy-code-review-round1-handoff.md) |
| 2026-09-28 | 2 (recheck) | `c8d646e` (range `2a97264..c8d646e`) | Not passed — 1×P2 (D2/D3/D4/D5 closed; D1 size axis closed) | [2026-09-28-workbuddy-code-review-round2-handoff.md](2026-09-28-workbuddy-code-review-round2-handoff.md) |
| 2026-09-28 | 3 (recheck) | `b718b55` (range `c8d646e..b718b55`) | Not passed — 1×P2 (R3-1: settled rect still excludes cursor; D6 transition axis closed) | [2026-09-28-workbuddy-code-review-round3-handoff.md](2026-09-28-workbuddy-code-review-round3-handoff.md) |
| 2026-09-28 | 4 (recheck) | `86951ae` (range `1ab82ad..86951ae`) | Not passed — 1×P2 (R4-1: expanded placement not idempotent — background repositions follow the live cursor and can drop it outside the window; R3-1 closed and independently verified) | [2026-09-28-workbuddy-code-review-round4-handoff.md](2026-09-28-workbuddy-code-review-round4-handoff.md) |
| 2026-09-28 | v1.1.1 behavior review + click-through | `435dc57` base | Current behavior and proposed changes for updates, window sizing, transparency, edge hiding, settings window; mouse click-through implemented | [2026-09-28-update-window-transparency-edge-review.md](2026-09-28-update-window-transparency-edge-review.md) |
| 2026-09-28 | v1.1.2 update plan | `v1.1.1` release base | Candidate changes and release checks for click-through, update installation, tray menu, and collapsed frame | [2026-09-28-v1.1.2-update-plan.md](2026-09-28-v1.1.2-update-plan.md) |
| 2026-09-28 | v1.1.2 execution | `7593e6d` base (`d0ea637` tip of candidate range) | Executed — versions bumped to `1.1.2` in 5 files; candidates `3fd166a`/`57f2ce9`/`d0ea637` retained and verified as ancestors; frontend 42/42 tests, frontend build, Rust 42/42 tests and `cargo check --locked` all green. Real signed-package smoke tests for NSIS/MSI/portable upgrades still outstanding | [2026-09-28-v1.1.2-update-plan.md](2026-09-28-v1.1.2-update-plan.md) |
