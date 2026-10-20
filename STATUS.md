# Status

## 5H shadow personalization review pass — 2026-10-20

- The shadow implementation was reviewed by an independent read-only reviewer, which first reproduced the published behavior of the base commit `6137faa` side by side: the published `five_hour_per_hour`, `weekly_per_hour` and `state_changed` sequences were byte-identical, and all 13 committed evidence files were independently reproduced. The review returned **FAIL**.
- The critical finding: the audit slot was only guarded for the personal outcome, so a lifecycle transition (an account switch or a segment transition) could overwrite an unappended matured or rejected event and destroy it for good, while the learner state had already advanced. The event placement now requires an empty slot, so no event of any kind can take an occupied slot.
- Seven minor findings are fixed as well: the outbox is now flushed before the new sample is processed, so a transient append failure no longer drops a five-minute learning interval; a backwards observation no longer writes a negative gap duration; a quota cycle, an overnight resume and a confirmed correction have their own stable codes; the effective-factor test now uses a case where `p != y`; the "failed usage read" test really injects a failure into the production read boundary; the "visible values" test was split into a structural frontend-contract test and a golden-vector test over the full persist-and-flush cycle; and the failing-append path is covered at the production refresh boundary.
- The two nits are fixed too: a pending interval that carries another algorithm version is now discarded by the shadow normalization, and the handoff is listed in `docs/handoff/INDEX.md`.
- A 16-probe mutation audit confirms the coverage: every probe mutates one frozen production rule and the named test turns red. One probe did not turn red at first — the code compares the decrease with the previous observation, but the old test could not tell that apart from a comparison with the origin. The test now uses a reading that is below the previous observation and still above the origin.
- The published rates stay pinned: a golden vector captured from the base commit matches in all 14 samples, and the full persist-and-flush cycle produces the same values.
- **Published.** `v1.3.2` is tagged at `3600d82` and published as the latest GitHub release: the signed NSIS installer, the signed MSI, the signed portable archive, their `.sig` files, the `latest.json` updater manifest and `SHA256SUMS.txt`. The release workflow ran green on windows-latest (14m57s), including the packaging regression, the frontend and Rust suites, and the signed Tauri build. `1.3.1` stays the rollback baseline.
- The plan's packaged and native smoke validation (installing the signed package and observing the learner across a real restart) was **not performed**; the release notes keep that limitation visible.
- A release near-miss is now closed: the first `v1.3.2` tag was cancelled because `src/version.ts` still carried `1.3.1`, which would have shipped an installer whose settings badge and tray menu showed the old version. `src/version.test.ts` now fails when the six version locations drift apart, and `RELEASING.md` lists all of them.

## 5H shadow personalization implementation — 2026-10-20

- The 5-hour burn-rate estimator now carries an invisible personalization stack that collects causal feedback for a later publication decision. Nothing user-visible changed: the overlay, the pill, the tray, the notifications, the settings, the polling cadence and the frontend DTO are untouched, and the frontend suite is unchanged.
- The Fixed Weighted candidate was promoted from the offline replay code into `src-tauri/src/codex/burn_rate/weighted.rs`, and the replay suite now imports it. The regenerated real-history forecast matches the pre-refactor baseline in all 318 rows with a **maximum prediction deviation of 0**, and the 13 committed evidence files stay byte-identical.
- New `personal.rs` holds the frozen learner (`A0=B0=10`, `lambda=0.99`, `c=clamp(A/B,0.75,1.50)`, `E` decayed, `g=E/(E+8)`) and the five-minute pending interval with a complete causal origin snapshot. New `shadow.rs` holds the v2 state subtree, the algorithm-version isolation and the at-least-once audit outbox.
- The burn-rate state schema moved from v1 to v2. A v1 file keeps its 5H and Weekly crossing history and gains a neutral shadow subtree; a malformed shadow subtree keeps the recoverable quota history; an algorithm-version mismatch keeps the history and the audit identity and resets only the incompatible learner state.
- The matured outcome scores the three predictions that were stored at the origin (published, Fixed Weighted, personalized), each capped by the quota remaining at the origin. The learner always trains on the Fixed Weighted prediction. A reset-correction, a decrease, a saturation, a gap over the frozen 90 seconds, a confirmed segment transition and a backward timestamp all reject the interval without touching the long-lived `A/B/E`.
- The personal transitions are paused while an audit event is waiting for the durable log, and a retained event is flushed on the next startup. The local evidence log is bounded to 10,000 events, carries no account identifier, and a save or log failure never fails a usage refresh.
- Rust **284** regular tests passed (4 pre-existing ignored replay tests unchanged), frontend 95 passed, the production build passed, the packaging regression passed, and the locked Cargo check is clean with zero warnings.
- The packaged and native smoke validation of the plan could **not** be performed on this machine; the restart, pending resume/reject, account switch, quota reset, log retry and the 30/60/120/300-second cadence behavior are covered by the automated tests only.
- [Implementation handoff](docs/handoff/2026-10-20-5h-shadow-personalization-implementation.md) records the test counts, the migration and isolation results, two de-identified evidence records, the outbox recovery result and the thirteen implementation-level deviations.

## v1.3.1 smart pause implementation — 2026-10-19

- The confirmed smart pause plan is implemented: a cooperative `暂停` request is sent to the running Codex session through the desktop client's own control plane, and a 5-hour trigger asks Codex to schedule a one-time task that sends `继续` two minutes after the reset. The feature is opt-in and off by default.
- New `src-tauri/src/pause_control.rs` module holds the threshold and message decisions, the in-memory de-duplication ledger, the ordered bridge-candidate discovery and the Windows named-pipe transport. `refresh_usage` hands the fresh successful sample to it; the account identity is read from the raw sample because the UI model does not carry it.
- Three points of the plan did not match the running client and the implementation follows the client: the `tools/call` tool name lives under `tool` (not `name`) in a closed parameter schema; the session transcripts are compressed `.jsonl.zst` with the thread UUID before the extension chain; and a single call carries a two-second sub-deadline inside the unchanged five-second batch deadline.
- The read-only `list_threads` handshake was exercised against the running Codex desktop client — pipe discovery, caller-identity discovery, the framed JSON-RPC call and the session-record shape. No session was active at that moment, so no message was sent. A real delivery, the one-time scheduled `继续` task and the signed package are **not** verified.
- Frontend 95 tests passed, the production build passed, Rust 194 regular tests passed and the locked Cargo check is clean. The production estimator, the polling cadence and the release channel are unchanged. No packaging or publication.
- [Implementation handoff](docs/handoff/2026-10-19-v1.3.1-smart-pause-implementation.md) records each acceptance layer, the protocol corrections and the residual risks separately.

## v1.3.1 smart pause review pass — 2026-10-19

- The first implementation was reviewed before the release gate — once by an independent reviewer that only read the code, once by a self-review that mutated the production logic to prove the tests react. Both passes found real problems; all of them are fixed, and the fixes are covered by tests that turn red under the corresponding mutation.
- The most serious one: an empty session listing was treated as a **failed handshake**, so the active periods were never released. A session that went idle and came back kept its old period identity and was paused **once per quota cycle for the life of the process**. A confirmed listing with no active session is now recorded as an observation — the bridge is reachable, every active period ends there, and a returning session starts a new period and is prompted again.
- Two of the plan's §10.6 mutation checks did not turn a test red: the five-second batch deadline (the old test cancelled the batch *before* any reservation existed, and measured the refresh instead of the batch) and the quota-cycle expiry (the old test passed because the first `prune` also switched the account, which empties the whole ledger). Both tests now fail when the rule is removed.
- The candidate discovery no longer runs while the app-wide lock is held; it runs on the blocking pool, and the session walk is bounded at 32 files and visits the newest names first.
- Smaller fixes: the bridge status follows the last handshake, a session listed with and without a `hostId` is contacted once using the record that carries the host id, a reset further than six hours falls back to the plain pause, the remaining time uses `checked_sub`, the live listing shape is pinned by a test, and the dead `present` flag was removed.
- Two reported findings did not reproduce and are recorded as such: a session listed with two host ids is sent to once, and the cycle-less key of a missing `resetsAt` is a documented residual risk (R-8) rather than a defect.
- The synthetic `turnId` was traced through the installed client: the schema only requires a non-empty string, the envelope's `turnId` is read only by the `wait_threads` completion matching and a call de-duplication key, and the send path routes by the target `threadId` and `hostId`. The read-only handshake with a synthetic turn id was accepted live, so the risk is small — but only a real delivery closes it completely.
- After the fixes: Rust **201** regular tests passed (4 pre-existing ignored replay tests unchanged), frontend 95 passed, the production build passed, and the locked Cargo check is clean with zero warnings.
- **Published.** `v1.3.1` is tagged at `6d5d0a1` and published as the latest GitHub release: the signed NSIS installer, the signed MSI, the signed portable archive, their `.sig` files, the `latest.json` updater manifest and `SHA256SUMS.txt`. The release workflow ran green on windows-latest (13m36s), including the packaging regression, the frontend and Rust suites, and the signed Tauri build. `1.3.0` stays the rollback baseline.

## Real quota history validation — 2026-10-03

- Causal forecasts over 3,854 local integer quota observations compare all nine offline variants. On 87 common 5H five-minute windows, MAE is 2.047 points (legacy), 1.704 (fixed weighted) and 2.258 (adaptive); later-period and equal-session comparisons also favor fixed over adaptive.
- Preferred next integration candidate revised to fixed weighting without interpolation/adaptation. Weekly adaptive gains are tiny and do not persist in the later period. Active-session logs, integer precision and sparse long-horizon coverage limit this evidence; current Phase 5 publication gates remain unresolved.
- [Real usage report](docs/handoff/2026-10-03-burn-rate-real-usage-validation.md), extraction/replay tools and aggregate evidence added. 145 regular Rust tests, real-history generator, locked Cargo check and data consistency checks passed. Production estimator/state and running overlay unchanged. Raw observations remain ignored locally.

## Adaptive estimator validation — 2026-10-03

- Fractional interpolation and change-detected local fitting validated offline across 664 trajectories / nine variants. Matched 5H constant mean error falls from 14.26% (legacy) to 0.76%; standard acceleration/deceleration integrals improve 43.3%/28.6%.
- Broader responses remain mixed: only 35/80 5H fixtures improve; all 80 Weekly integrals improve but two excursion regressions remain. Production estimator remains unchanged; this is partial validation, not publication approval.
- Evidence and limitations: [adaptive validation report](docs/handoff/2026-10-03-burn-rate-adaptive-validation.md). Regular Rust tests 142 passed; matrix/stress generators and locked Cargo check passed.

## Phase 5 estimator replay — 2026-10-03

- Weighted local regression, conservative idle metadata and migration are implemented as offline test candidates; production estimator/schema remain unchanged.
- Expanded search: 240 parameter combinations, zero passing the current combined gates. Legacy itself fails the universal raw 5% bound; accuracy/response tradeoffs and possible contract revisions are recorded in the [replay report](docs/handoff/2026-10-03-burn-rate-estimator-replay.md).
- Frontend 86 tests, production frontend build, Rust 136 tests plus the separately run grid, and locked Cargo check passed. Publication of Phase 5 remains deferred under the current plan.

## v1.3.1 display implementation — 2026-10-03

- Phases 2–4 of the revised `ad8dd4f` plan implemented: independent display preferences, Weekly-only overlay/pill/tray, full tray read lifecycle, and composition-aware dock sizing.
- Display-only path selected; optional estimator refinement deferred, with v1.3.0 estimator/schema preserved.
- Automated/browser verification and native release limitations: [implementation handoff](docs/handoff/2026-10-03-v1.3.1-display-controls-implementation.md).
- No version bump, packaging or publication; native WebView2/tray/hover/mixed-monitor smoke checks and independent implementation review remain pending.

## v1.2.1 patch — 2026-09-30

- Left tray single-click immediately shows/hides the overlay; a double-click toggles once. Only right-click opens/closes the menu.
- Portable updater manifest signatures now come from the generated .sig file with format validation; packaging regression tests cover trailing CLI instructions and invalid signatures.
- Local validation: frontend build passed, frontend tests 56/56 passed, locked Rust tests 102/102 passed, locked cargo check passed with incremental caching disabled, and all six packaging regression cases passed.
- Release policy for this patch: amend the previous commit and replace v1.2.1 through GitHub Actions after CI passes on the exact replacement commit, as requested by the user.
- Interactive Windows gestures and signed-package smoke tests were not performed. The historical v1.2.0 review and manual verification gaps below remain unchanged.


_Last updated: 2026-10-20 (v1.3.2 published: the invisible 5H shadow personalization stack is in, it passed an independent review round whose findings are all fixed, and `v1.3.2` is tagged and released as the latest release. Nothing user-visible changed. Four gates fully green: npm test 95/95, npm run build, cargo test 284/284, cargo check 0 warnings, plus the 13 byte-identical replay evidence files and a 318-row forecast with a maximum prediction deviation of 0. The packaged and native smoke validation of the plan is not verified.)_

## Current state

| Item | Value |
| --- | --- |
| Version | `1.3.2` (package metadata, lockfiles, Tauri config, and `src/version.ts` unified); `1.3.1` stays the upgrade and rollback baseline |
| Reviewed commit | `1a5314d` (`fix(review): resolve round 2 findings (P3-1 call-site adapters, P3-2 compaction sentinels)`) — v1.2.0 line, the last line with an independent review |
| Baseline of that review | `6eeed35` (range `6eeed35..1a5314d`, 25 files / +2606 −390) |
| v1.3.2 line | **5H shadow personalization: reviewed and fixed** — an independent read-only review reproduced the published behavior of the base commit side by side and returned FAIL with 1 critical, 1 major, 7 minor and 2 nit findings; every finding is fixed and covered by a test that turns red under the corresponding mutation (see the review-pass section above). Shadow only: no user-visible behavior changed |
| v1.3.1 line | **Smart pause: reviewed and fixed** — an independent read-only review and a mutation-based self review both ran over the smart pause implementation; every finding is fixed and covered by a test that turns red under the corresponding mutation (see the review-pass section above). The display-controls line still has **no** independent review round |
| Historical manual verification gate | **Unfinished** — stage 0 Windows checks (see `docs/stage0-verification.md`); v1.2.1 uses the automated release policy above |
| Code review gate (v1.1.3) | **Passed** — v1.1.3 round 5 (Mode C) closed the line with **0 defects**. Round 4's P3-1 is closed: `AppState::for_test` (`src-tauri/src/commands.rs:53-81`) plus the cache-read wiring tests make the capture/read/priority decisions falsifiable, and three acceptance mutations were each verified to fail tests. Rust tests 61 → 65 |
| Code review gate (v1.2.0) | **Passed** — v1.2.0 round 3 (Mode C) closed the line with **0 defects**. Round 2's P3-1 (M1~M4, M9 call-site adapter traits) and P3-2 (SoundPicker compaction layout budget sentinels) confirmed closed. Dead helper code cleaned up. Rust tests 94 → 98 |
| Design review gate (v1.2.0) | **Superseded by implementation** — the design round-3 residuals (P3-1 landing seam, P3-2 §7.2.6 observation points, P3-3 unique attribution rule) were addressed in `998a558` and doc sync R-5 completed. See `docs/handoff/2026-09-29-workbuddy-code-review-round3-handoff.md` |
| Compliance audit | **Compliant** — the v1.1.2 and v1.1.3 plans and the v1.1.1 behavior review were audited line-by-line against the code: every in-scope deliverable is implemented, with no omissions, reversals, or v1.1.2 regressions (`docs/handoff/2026-09-29-cross-plan-compliance-audit.md`) |

## Gates (reproduced locally on 2026-10-19 with the v1.3.1 line, after the review pass)

- `npm test` → 95/95 pass
- `npm run build` (tsc + vite) → pass
- `cargo test --manifest-path src-tauri/Cargo.toml --locked` → 201/201 pass (4 pre-existing ignored replay tests)
- `cargo check --manifest-path src-tauri/Cargo.toml --locked --all-targets` → pass (0 warnings)

Green gates do **not** constitute review approval, and none of them exercise a real GUI.

## Open defects

None in the v1.1.3 review line.
None in the v1.2.0 review line (all 0×P0, 0×P1, 0×P2, 0×P3). Closed by WorkBuddy review round 3.
The v1.3.1 **smart pause** line had a review round on 2026-10-19: 2×P1, 2×P2 and 6×P3 were reported, all are fixed and covered by regression tests, and 2 reported findings did not reproduce. The v1.3.1 **display controls** line has had no independent review round yet, so it still has no defect count — that is a gap, not a clean bill of health.

## Residual risks (not defects — they need a packaged, multi-monitor GUI to close)

| # | Item |
| --- | --- |
| R-1 | The plans' own real-machine checks are unexecuted: settings-window drag-resize → close → reopen exact restore, tray label truncation, the settings version badge rendering (v1.1.3 plan §三-2), and the signed-package NSIS/MSI/portable upgrade, exit-install, elevation and failure-recovery smoke tests (v1.1.2 plan, "发布前验证"). |
| R-2 | `apply_settings_geometry` / `center_settings_window` (`src-tauri/src/tray.rs:514`, `:546`) have no test or mutation probe — the geometry restore chain is automated only up to the pure planner (`select_work_area` / `place_within_work_area`) and the config read/write path. This is the `tray.rs` counterpart of the round-4/5 `lib.rs` wiring concern. |
| R-3 | **Closed** — the settings-window version badge is now asserted in `src/components/SettingsView.test.tsx` (the header badge read `v1.3.0` and follows the unified version, `v1.3.2` now). |
| R-4 | `README.md` / `README.en.md` do not describe the v1.1.3 user-visible behaviour (free settings-window resize, cross-process geometry memory). The v1.1.3 plan did not list README changes, so this is a coverage suggestion rather than a contract gap. |
| R-5 (v1.2.0) | **Closed** (round 2). The design proposal §3.2 case B and the §3.3.1 pseudocode comment now state the same totality contract as the shipped `select_monitor_by_overlap` (`src-tauri/src/dock.rs:561-599`): strictly largest intersection area wins, the host screen (`fallback_index`) breaks a tie that includes it, a non-host tie is broken by the smallest `(x, y)`, and zero overlap falls back to the host. §3.1's user-visible rule (>50 % ⇒ the target screen) holds either way. |
| R-6 (v1.2.0) | If Windows foreground lock keeps `set_focus()` failing *and* a transient `Focused(false)` arrived inside the grace window, the popup still hides at expiry (`pending_blur && !is_focused`, which is the specified contract). This is the extreme-focus variant of the symptom module 2 fixes; it can only be settled on a real machine with passthrough enabled. |
| R-7 (v1.3.1) | The auto-pause bridge is verified read-only against the live Codex client, but a real `send_message_to_thread` delivery, the one-time `继续` task and the signed package are not. The client has no `turnId` in its thread records, so the active period falls back to the synthetic counter, which deliberately under-sends rather than double-interrupting. See the [handoff](docs/handoff/2026-10-19-v1.3.1-smart-pause-implementation.md) §5 and §7. |
| R-8 (v1.3.1) | A 5-hour window reported **without** `resetsAt` has no cycle identity, so its de-duplication key never expires and the session is paused only once per active period. The plan is split here on purpose (§3.1 excludes data without a cycle identity, §4.1 requires the plain `暂停` anyway); the implementation follows §4.1. The impact is bounded, because a missing reset time also means a plain pause with no resume timer — nothing scheduled is missed, and a session that leaves the active list starts a new period and is prompted again. A regression test pins the behaviour. |

## Next steps

0. v1.3.1: run an independent code review round over the **display controls** line. The smart pause implementation has had one (findings fixed); the display controls have not, and the green gates do not substitute for it.
1. v1.3.1: run the end-to-end auto-pause acceptance on a *disposable* Codex session — the 5-hour case, the Weekly case, the simultaneous case, the repeated refresh, the bridge-failure path and the one-time `继续` task two minutes after the reset. Do not use a live working session.
2. Run the stage-0 Windows checks and the real signed-package smoke tests (NSIS / MSI / portable): upgrade, exit-install, elevation, failure recovery.
3. Verify on a real multi-monitor / mixed-DPI desktop: the v1.2.0 cross-screen drag attribution and seam防误折叠, the pill percentage grid at 100/150/200 % DPI, settings-window resize → close → reopen exact restore, display detach fallback, and the tray click/toggle timing with passthrough enabled (R-6).
4. Optional hardening: give `apply_settings_geometry` an offline falsifiability path (or an integration probe), and update `README.md` / `README.en.md` for the v1.3.1 user-visible behaviour.
5. Only then consider lifting the release gate; note that the v1.1.1 review's future-version items (transparency real-machine comparison, border-alpha coordination, the click-to-expand / narrow-label edge-hiding state machine) are explicitly **not** part of v1.1.2/v1.1.3 and remain unimplemented by design.
