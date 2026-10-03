# Phase 5 weighted estimator implementation and replay

Date: 2026-10-03

Baseline: `ea1cba3`

Contract: [revised v1.3.1 plan, §§8.1 and 11.6](2026-10-03-v1.3.1-display-controls-plan.md)

## Result

The exponentially weighted local crossing regression is implemented as an **offline candidate**, including conservative idle bounds, monotone idle ceilings and compatible optional-metadata normalization. The 240-parameter search found **zero candidates passing the current stability and response gates together**. Under §8.1.2, Phase 5 remains deferred from production. No half-life/window is selected for publication.

All candidate code lives in `src-tauri/src/codex/burn_rate/replay.rs`, reached only by a `#[cfg(test)]` module declaration. It calls the actual production `accept_quota()` observation/reconstruction/protection path, and compares against the actual unchanged `estimate_rate()` reference at identical observation times. It does not substitute ideal crossings for the raw replays. Production estimates, state fields/schema, account handling, refresh/save pipeline, polling and display controls remain unchanged. The metadata serialization in the prototype is not installed in the live state file.

## Search and recorded evidence

[Committed parameter summary](2026-10-03-burn-rate-parameter-grid.csv) contains every parameter combination, worst constant fixture, availability counts, first/sustained response times, six-crossing error integrals, excursions and gate results. Full per-fixture evidence is regenerated under ignored `output/burn-rate-replay/parameter-grid.json`.

- 5H half-lives: 15/30/45/60/90/120/180/240/300/360/450/600/900/1800/3600 seconds.
- Local windows: 180/240/300/450/600/900/1200/1800 seconds, relative to the newest eligible crossing; the separate hard evidence horizon is still 1800 seconds relative to the accepted observation.
- Weekly candidates multiply the same half-lives/windows by 48, retaining the 86400-second hard horizon and one-point minimum span. This includes the original 4-hour and 6-hour comparison candidates.
- Each candidate has 240 raw constant fixtures: two rates, five cadences, three fetch durations, four quantization phases and fractional/integer input. 5H uses `60/30%/h`, 15/30/60/120/300-second sleep-after-fetch cadences, 0/5/20-second successful-fetch durations and deterministic 0..5-second jitter. Weekly uses proportionally time-scaled fixtures (`1.25/0.625%/h`) for corresponding slow consumption; **its scaled cadences/durations are not a claim of the complete UI-cadence matrix**.
- Response fixtures observe every 30 seconds (Weekly: scaled by 48), use 30 minutes of `60%/h` history, then `120%/h` or `30%/h`, and continue for 20 new points. Boundaries are beyond the traces; usage remains below saturation. The integral includes the rate held at the change time, through confirmation of the sixth new crossing. Unavailability where legacy is available fails the response gate.
- The original 10-minute/6-hour max gaps remain unchanged. A separate test covers healthy 300-second sleeps plus fetch duration/jitter, and inherited 3600-second 5H segmentation.

This is an early **failure finding**, not a completed Phase 5 publication-validation claim: the expanded Weekly UI-cadence matrix, manual-refresh/phase matrix, production migration/save-error integration and independent review are still required before any future production switch. A failed 5H stability gate already prevents current publication; those missing gates cannot rescue it.

## Is the 5% criterion too strict?

Yes, the evidence suggests that **a universal pointwise 5% bound immediately after eight reconstructed crossings can reject useful accuracy improvements**. This is different from the exact linear-crossing numerical invariant, which all tested positive weights preserve within `1e-9`.

The following 5H candidates all use the 1800-second local window. Mean errors are time-weighted relative errors over matching available constant observations, pooled across the fixture matrix; they are diagnostic summaries, not acceptance decisions. Peak errors include the specified early warm-up. The response ratios are candidate/legacy six-crossing absolute-error integrals, separately by direction; lower is better.

| Half-life | Candidate mean constant error | Candidate peak | Legacy mean / peak | Acceleration ratio | Deceleration ratio |
| --- | --- | --- | --- | --- | --- |
| 15s | 21.68% | 184.52% | 14.24% / 29.41% | 0.40 | 0.71 |
| 30s | 15.97% | 106.80% | 14.24% / 29.41% | 0.72 | 0.87 |
| 60s | 8.52% | 44.57% | 14.24% / 29.41% | 1.17 | 1.32 |
| 120s | 3.47% | 24.72% | 14.24% / 29.41% | 1.49 | 2.23 |
| 300s | 1.40% | 14.95% | 14.24% / 29.41% | 1.64 | 3.19 |
| 3600s | 1.00% | 12.97% | 14.24% / 29.41% | 1.68 | 3.64 |

For example, constant `1%/min` with a 47-second fractional phase and observations at `0,125,251,378,506` seconds creates nine crossings spanning eight points through the unchanged uniform interpolation. The 3600-second candidate publishes approximately `67.78%/h` at 506 seconds, exceeding the 5% bound despite the truly constant trajectory. Longer weighting cannot remove the early reconstruction error merely by fitting an exact line to these estimated event times.

The 300-second candidate substantially improves average constant accuracy, but its acceleration/deceleration integrals are 64%/219% worse than legacy. Conversely, the 15/30-second candidates pass the chosen response comparison but create large constant-rate excursions. Thus relaxing the 5% bound alone does not establish a suitable default. Zero passing candidates does not prove that every possible parameter or cadence-aware policy must fail.

Suggested next contract revision (discussion only; **the existing plan gates have not been changed**):

1. Keep exact ideal-slope invariance, finite-positive outputs, account/reset/correction protections, conservative idle deadlines and restart preservation as hard requirements.
2. Evaluate noisy raw observations using time-weighted error, sustained bias and excursions **per cadence and quantization mode**, with the existing estimator as a matched baseline. Do not use a pooled mean to hide one failing cadence.
3. Specify a separate uncertainty/warm-up allowance for reconstructed event times instead of demanding the same 5% bound at every output after eight crossings.
4. Decide the desired stability/response tradeoff explicitly. Consider a separately specified cadence-aware half-life/window policy; do not silently replace the fixed-parameter search with an unreviewed adaptive rule.

## Reproduction and checks

From the repository root:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --locked codex::burn_rate::replay
cargo test --manifest-path src-tauri/Cargo.toml --locked parameter_grid -- --ignored --nocapture
node scripts/burn-rate-replay-report.mjs
```

The second command intentionally passes when evidence generation succeeds, **even if no algorithm passes publication gates**. Its final line and JSON/CSV `pass` fields report the algorithm outcome. The CSV generator can accept explicit input/output paths to reproduce the committed summary.

Eleven regular candidate tests cover ideal linear slopes, insufficient/expired/degenerate evidence, the `(1845,40)` manual-refresh counterexample, fractional observations and deadline equality, monotone idle through partial horizon expiry, migration/reload without new warm-up, malformed optional metadata, invalid/stale/conflicting/correction samples, clearing on gap/correction/reset, slow polling, and negative stability/response controls. Existing production tests continue covering account isolation and real persistence/refresh behavior.

Verification: frontend 86 tests passed; TypeScript/Vite build passed; Rust 136 tests passed with one explicit grid test ignored by default; the grid was run separately and generated 240 candidates / zero passing; locked Cargo check passed. Cargo test emitted the existing Windows linker informational warning about creating the import library; no compiler/check warnings were introduced.

No version bump, package publication, live overlay replacement/restart or service modification is part of this stage. Native display-release smoke checks remain pending as documented in the display implementation handoff.
