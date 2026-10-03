# Fractional interpolation and adaptive estimator validation

Date: 2026-10-03

Baseline: `0eacaef` — [fixed-parameter replay](2026-10-03-burn-rate-estimator-replay.md)

## Conclusion

The proposed direction is **partly validated**, not ready to replace production. Fractional-position interpolation reduces matched constant-rate error. Change detection with conditional recovery fixes both standard step-response directions while retaining the stable estimate. However, broader 5H slow-polling responses and some transient excursions still regress against legacy. The existing Phase 5 publication gates have not been relaxed or claimed passed.

Everything added is offline Rust test code under `replay::adaptive`. Production observation acceptance, published estimates, saved state/schema, account handling, refresh pipeline and the running overlay are unchanged. No version bump or deployment is included.

## Candidate and independent controls

All variants use the real `accept_quota()` raw acceptance/reset/correction/gap path. Legacy uses the unchanged production `estimate_rate()`. Fixed candidates use the previous weighted-regression/idle prototype. The shared test prototype separates observation acceptance from publication so an alternative crossing reconstruction can be evaluated before idle publication without duplicating the acceptance logic.

The tested combined candidate (`precision_adaptive`) does the following:

1. Once fractional precision is observed in the current segment, interpolate each newly crossed level using `(level - previous_percent) / (current_percent - previous_percent)`, rounding the timestamp to seconds. Existing crossings are retained. Integer-only segments continue using the existing estimated timestamps. The original confirming-observation bounds are retained in either mode.
2. Compare a slow fit against the slope range permitted by the newest three **distinct confirming observations** and their crossing-time bounds. Crossings reconstructed from one observation count as one group. Trigger only when the slow fit is outside that range with a 10% guard.
3. After a trigger, fit a local region beginning at the oldest of those three groups, preserving the raw history. Use a 15-second fast half-life for 5H. Recovery requires three further distinct confirming observations and fast/slow local fits agreeing within 10%; only then increase the half-life gradually to 300 seconds over 600 seconds.
4. Weekly uses the same dimensionless policy with times multiplied by 48 (slow half-life 4h, fast half-life 12m, recovery 8h), its existing 24h horizon and one-point minimum span. These remain diagnostic parameters, not approved production defaults.

Nine variants separate the effects: legacy; fixed 300s; interpolation only; adaptive only; combined conditional 15s; combined with two trigger votes; early recovery with 30s; early recovery with 15s; conditional recovery with 30s. Comparing recovery policies at the **same** fast half-life avoids attributing two simultaneous changes to one cause.

The estimator is not given fixture truth, the scheduled change time, input fractional/integer labels, cadence, or future observations. Its precision flag comes from accepted values; its evidence bounds come from accepted observation intervals. Test truth is used only for scoring. Precision switching is a separate stress case; this does not establish that all real fractional inputs have exact interpolation accuracy.

## Matched stability evidence

Mean errors below are time-weighted absolute relative errors after at least eight eligible crossings spanning seven points. Peak includes early warm-up. Comparisons are within this new matrix; its schedule and duration differ from the earlier fixed-grid report.

| Quota | Legacy mean | Fixed mean | Interpolation + adaptive mean | Legacy peak | Candidate peak |
| --- | --- | --- | --- | --- | --- |
| 5H | 14.26% | 1.48% | 0.76% | 29.27% | 12.14% |
| Weekly | 13.68% | 0.06% | 0.03% | 25.84% | 0.78% |

The combined candidate has lower mean error in every quota/cadence/precision group. All 480 constant trajectories have zero change activations and no post-warm-up unavailability. Adaptive-only matches fixed stability because no trigger fires; interpolation-only matches combined stability. Thus the additional constant accuracy is attributable to interpolation, not detector activity.

The matched fractional counterexample uses `(0,125,251,378,506)` seconds and a 47-second value phase. With the same 300s half-life, the original reconstruction gives `68.589685%/h`; interpolation-only and combined both give `60%/h` within `1e-8`, matching the underlying `1%/min`. A regular regression test checks both the old failure and new result.

The worst 5H integer-only peak remains 12.14%, so the current universal raw 5% gate is still not passed.

## Step response and counterexamples

The standard trace uses 30 minutes of `60%/h`, then `120%/h` or `30%/h`, exact 30-second polls and integer readings. The error integral starts with the rate already held at the change time and ends at the observation confirming six new points. Values are relative error integrated over seconds; ratios are meaningful within a matched trace.

| 5H standard trace | Legacy integral | Combined integral | Reduction | Candidate sustained ±10% entry |
| --- | --- | --- | --- | --- |
| Acceleration 60 → 120 | 52.65 | 29.85 | 43.3% | 60s |
| Deceleration 60 → 30 | 149.03 | 106.35 | 28.6% | 120s |

Legacy never sustains three consecutive readings in the target band in these traces; its deceleration transient first enters at 210s. This is partly legacy's constant-rate bias, not a claim that it never responds. Candidate standard excursions are effectively zero and below legacy. Both quotas' scaled standard traces pass regular assertions for at least 10% lower integral in each direction, equal coverage, no worse excursions and sustained-band entry.

For standard deceleration, early recovery with 30s gives integral 147.58, conditional recovery with 30s gives 118.61, early recovery with 15s gives 141.75, and conditional recovery with 15s gives 106.35. Conditional recovery helps at either fast half-life. Requiring two trigger votes gives 195.12, showing the latency cost of extra confirmation.

Broader tests invalidate a universal superiority claim:

- 5H: combined improves the integral in only **35/80** response fixtures. Six have slower sustained entry where legacy has one; 20 have greater excursions. The worst integral ratio is **3.27×** legacy.
- Weekly: **80/80** response fixtures have lower integral, with no slower sustained entry where legacy has one. Nevertheless, two 300-second integer acceleration fixtures have larger excursions (group maxima: candidate `0.20849%/h`, legacy `0.15210%/h`), so its excursion gate is not universally passed either.
- Example 5H integer deceleration group ratios (candidate/legacy): 30s cadence `1.30`, 60s `1.76`, 120s `2.02`, 300s `2.20`. Each group includes two fetch durations and two phase offsets; these are not cherry-picked single observations.
- In exploratory 5H stress scoring, 30s ramp error ratio is about `0.58`, but rapid oscillation is `1.09`. At 300s cadence, idle/resume is `1.25`. These confirm the stability/response tradeoff outside ideal steps.

A likely next improvement is a cadence-aware detector that can recognize a strongly supported rate change within a long raw observation interval, using the interval's percentage uncertainty rather than requiring extra crossing-confirmation groups. Shortening a half-life alone cannot recover information delayed by 300-second polling. That is a new candidate to specify and validate; no such rule has been silently installed here.

## Coverage, artifacts and reproduction

664 underlying synthetic trajectories are compared across nine variants, producing 5676 result rows:

- 480 constant trajectories / 4320 rows: both quotas; rates `60/30%/h` for 5H and `1.25/0.625%/h` for Weekly; **actual** 15/30/60/120/300-second cadences for both; 0/5/20-second fetch durations; four observation phase offsets; deterministic 0..5-second jitter; integer/fractional input. Duration is 32 underlying crossing periods, with reset boundaries safely beyond all traces.
- 160 broader response trajectories / 1440 rows: two directions per quota; the same actual cadence choices; 0/20-second fetch durations; two different observation phases; integer/fractional input and a different jitter seed. Rates stay below saturation; traces continue for 20 new points to observe settling/excursions.
- Four exact standard step trajectories / 36 rows. These are the tuning/positive-control fixtures; broader phase/cadence results are reported separately rather than asserting generalization from the standard traces.
- 20 stress trajectories / 180 rows: manual refreshes, ramp, oscillation, idle/resume and precision switching, at 30/300-second cadences on both quotas. Stress uses exploratory Simpson quadrature of changing truth; discontinuities and absolute-error sign changes can add quadrature error. These scores are supporting evidence, not publication gates. Coverage/unavailability is included explicitly.

Committed summaries:

- [Constant groups](burn-rate-adaptive-evidence/constant.csv)
- [Response groups and worst fixtures](burn-rate-adaptive-evidence/response.csv)
- [Every standard trace/variant](burn-rate-adaptive-evidence/canonical.csv)
- [Every stress trace/variant](burn-rate-adaptive-evidence/stress.csv)

Full per-fixture results remain reproducible under ignored `output/burn-rate-replay/`. CSV empty response times mean unavailable, never a fabricated zero. Regular tests additionally cover one-observation multi-crossing independence, active idle ceilings through evidence expiry, JSON restart of the detector/filter/clock, and invalid/stale/conflicting/correction samples plus reset/max-gap clearing. JSON ceiling checks allow `1e-12` relative serialization tolerance; production account and file-save tests remain unchanged. Production integration of the added detector metadata, migration, account switches and save failures is not claimed.

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --locked codex::burn_rate::replay::adaptive
cargo test --manifest-path src-tauri/Cargo.toml --locked adaptive_comparison_matrix -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --locked adaptive_stress_replays -- --ignored --nocapture
node scripts/burn-rate-adaptive-report.mjs
```

Matrix/stress commands pass when evidence generation succeeds, including negative algorithm findings. They do not mean every candidate passes a publication gate.

Verification: 142 regular Rust tests passed, three evidence generators ignored by default; the adaptive matrix and stress generators were explicitly executed; locked Cargo check, JavaScript syntax and diff checks passed. The earlier fixed-grid generator is unchanged in behavior and was not repeated. Frontend files were unchanged, so previous frontend gates were not rerun. No overlay restart or service changes were performed.
