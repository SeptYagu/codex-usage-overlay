# Real quota history forecast validation

Date: 2026-10-03. Baseline: `db68e74`.

## Selection

The real-history forecast experiment **does not support the earlier choice of adaptive-only as the default candidate**. Fixed weighted regression is the preferred next integration candidate, with no fractional interpolation or adaptive detector. This is a candidate selection, not a production rollout or proof that the existing Phase 5 publication gates passed. The live overlay, production estimator/schema and saved state remain unchanged.

On matched five-minute forecasts, fixed weighting reduces 5H mean absolute increment error from 2.047 to 1.704 percentage points (16.8% versus legacy). Adaptive-only raises it to 2.258 (32.5% versus fixed). Weekly's adaptive improvement over fixed is only 0.004 percentage points overall and reverses in the later period and with equal session weighting. Synthetic standard-step improvements did not transfer to this forecast task.

## Data and interpretation

The frozen extraction contains 3,854 valid quota observations from 30 local session files dated September 27 through October 3. All 5H and Weekly observations are integers. No invalid quota fields, conflicting observations in the same second or duplicate-second observations were found among matching parsed events. Snapshot last timestamp: `2026-10-03T15:58:32Z`. The prior exploratory count of 3,848 came from an earlier read of growing logs; it is not this frozen dataset.

Only `event_msg`-style `token_count` payloads for the `codex` limit, 300-minute primary and 10080-minute secondary windows are extracted. The artifact contains anonymous session ordinals, observation timestamps, percentages and reset timestamps. Conversation text, source filenames, account identifiers, credit information and authentication data are not copied. Raw input and individual forecasts remain in ignored `output/`; only aggregate evidence is committed.

These are **active-session notification observations**, not a complete overlay polling history. The saved overlay setting is 30 seconds, but log event timing is irregular and often faster. The experiment cannot measure full-day idle frequency or establish that all records belong to one account. Each file starts a fresh tracker and is never joined to another file, avoiding invented cross-session continuity but losing the live overlay's persisted warm history. Concurrent sessions and windows at different horizons are not statistically independent.

The quantity scored is future **observed quota increment**, not ground-truth instantaneous burn rate. An unchanged integer reading does not prove zero consumption. Integer endpoint quantization introduces uncertainty of roughly one percentage point in an increment; `excessBeyondOnePoint` is a diagnostic allowance, not an API precision guarantee. Short-horizon Weekly results are particularly limited by that resolution.

## Causal forecast protocol

All nine previously specified variants are frozen without tuning on these scores. Their real observation acceptance/reset/correction path is reused. For each file/quota, feed each snapshot once, in increasing integer-second order. An estimate at origin time uses only that origin and earlier observations. Regular tests check prefix estimates remain identical after future data is appended.

Predict `rate * actual_elapsed_seconds / 3600`, capped by remaining quota, and compare with the later observed percentage minus the origin percentage. A zero-increment predictor is included as a sanity baseline. Target horizons are 5, 15, 30 and 60 minutes. Select the first future observation reaching the horizon, at most 90 seconds late.

Discard an outcome path with any nonpositive gap, gap over 90 seconds, changed reset timestamp, percentage decrease or saturation at 100%. Reset-timestamp changes are excluded conservatively even when they might only be metadata shifts. Forecast intervals do not overlap within each file/quota/horizon. Do not bridge missing readings with synthetic samples or carry forward unseen future quotas.

Score all variants on the **same common availability windows**. Also publish each variant's availability over all eligible windows, including warm-up failures. Do not substitute zero for unavailable estimates. The fixed chronological check starts October 2 at 00:00 UTC; earlier forecasts whose outcomes cross that boundary are excluded. The later period is not used to tune parameters, but the earlier scene-frequency discussion already inspected these dates; it is not an independent unseen deployment trial.

## Five-minute results

Mean absolute error in **percentage points of quota**, lower is better:

| Variant | 5H all (87 windows) | 5H later (17) | Weekly all (89) | Weekly later (9) |
| --- | ---: | ---: | ---: | ---: |
| Legacy | 2.047 | 2.083 | 0.505 | 0.601 |
| Fixed weighted | **1.704** | 2.003 | 0.466 | **0.554** |
| Precision only | **1.704** | 2.003 | 0.466 | **0.554** |
| Adaptive only | 2.258 | 2.163 | **0.461** | 0.572 |
| Precision + adaptive, conditional 15s | 2.258 | 2.163 | **0.461** | 0.572 |
| Combined, two trigger votes | 2.106 | **1.879** | 0.478 | 0.575 |
| Combined, early recovery 30s | 2.113 | 2.066 | 0.462 | 0.572 |
| Combined, early recovery 15s | 2.199 | 2.063 | 0.462 | 0.572 |
| Combined, conditional recovery 30s | 2.111 | 2.068 | 0.462 | 0.572 |
| Zero increment | 4.149 | 4.176 | 0.551 | 0.556 |

With integer-only input, precision-only equals fixed; combined conditional 15s equals adaptive-only. The two-vote candidate wins the small later 5H sample, but changes only two predictions versus fixed and is worse overall. It does not establish a reliable default advantage.

All nine estimators share five-minute availability: 87/125 eligible 5H windows across 24 source files and 89/149 Weekly windows across 19 source files. Later-period availability is 17/25 and 9/25 respectively. Zero increment is available on every eligible window but evaluated in the table on those same common windows.

5H p90 error: legacy 4.839, fixed 4.112, adaptive-only 5.046 percentage points. Later p90: 3.443, 3.115 and 4.760. The fixed improvement is not obtained by dropping additional forecasts.

For 5H, fixed beats legacy in 50/87 windows, loses in 37; adaptive beats fixed in 19, loses in 32, ties in 36. With equal weight per source session, mean error is legacy 2.397, fixed 1.928, adaptive 2.620. Fixed beats legacy in 17/24 session means; adaptive beats fixed in four, loses in 16 and ties in four. No identical `(origin, target, observed_delta)` five-minute common windows were found across files in either quota.

For Weekly, interval-weighted means give adaptive a tiny advantage. Equal session weighting reverses it: fixed 0.600, adaptive 0.602. Later-period means likewise favor fixed. These differences do not justify the added state machine.

## Other horizons and observed regimes

| Horizon | 5H common windows | Legacy / fixed / adaptive MAE | Weekly common windows | Legacy / fixed / adaptive MAE |
| --- | ---: | --- | ---: | --- |
| 15 minutes | 11 | 4.514 / **3.876** / 5.557 | 11 | 0.921 / **0.768** / 0.920 |
| 30 minutes | 2 | 3.060 / 4.661 / 4.661 | 1 | 1.443 / 2.343 / 0.920 |
| 60 minutes | 0 | unavailable | 0 | unavailable |

The 30/60-minute evidence is insufficient for selection. Later-period 15-minute common coverage is only one 5H window and zero Weekly windows.

Within five-minute common windows, 5H observed increment is zero in seven, one point in ten and two or more in 70. Their fixed/adaptive MAEs are respectively 0.877/0.877, 0.427/0.308 and 1.969/2.675. Adaptive's low-change improvement does not offset the larger errors in the frequent two-plus-point group. This is classification by **observed future increment for scoring only**, never a feature supplied to the estimator. It does not identify an underlying stable or changing rate.

Weekly regimes: zero in 44, one point in 41, two-plus in four. This explains why five-minute Weekly MAE alone gives weak discrimination between rate estimators. Complete regime/variant summaries include empty groups explicitly.

## Artifacts, checks and next step

- [Forecast aggregates](burn-rate-real-evidence/forecast.csv): all variants, quotas, horizons and chronological periods, including availability and equal session weighting.
- [Observed-increment regimes](burn-rate-real-evidence/scenarios.csv): regime-specific error for every estimator.

Reproduce locally (the source logs and raw outputs are private to this machine):

```powershell
node scripts/burn-rate-real-extract.mjs
cargo test --manifest-path src-tauri/Cargo.toml --locked real_usage_forecast_replay -- --ignored --nocapture
node scripts/burn-rate-real-report.mjs
```

The extractor optionally accepts source session root, output path, start date and end date. `BURN_RATE_REAL_INPUT` overrides the Rust runner's input. This benchmark's chronological split remains fixed to October 2; a later dataset requires an explicitly specified new protocol, not post-score split selection.

Regular checks cover future-data isolation, non-overlapping forecasts, chronological boundary isolation and rejection of gaps/resets/corrections/saturation. Verification: 145 regular Rust tests passed; the real-history generator passed separately; locked Cargo check and both JavaScript syntax checks passed. No frontend files changed, so frontend/layout checks were not repeated.

Next: validate fixed weighting against actual overlay observations including idle, restarts and account transitions before production integration. The existing synthetic worst-error and response gates remain unresolved; forecasting evidence does not silently replace the display estimator contract. No production update, version bump, overlay restart or service change is part of this milestone.
