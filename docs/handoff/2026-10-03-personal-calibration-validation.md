# Personal Burn-Rate Calibration — First Real-History Validation

Date: 2026-10-03
Plan baseline: `0b6fe7e`
Dataset: frozen real-history forecast artifact from 3,854 quota observations / 30 source files, September 27–October 3, last observation timestamp `1791043112`.

## Result

The first replay finds a **real calibration signal but does not pass the stability gates for shadow integration**.

For 5H five-minute holdout forecasts, the selected personal calibration reduces MAE from `2.0034` to `1.7899` quota points, a 10.7% improvement, and reduces equal-session MAE from `1.5698` to `1.4220`. Signed bias moves from `-0.8324` to `-0.0122`, nearly eliminating the systematic under-prediction in this small holdout.

However, p90 error rises from `3.1148` to `4.5125` (+44.9%), and 6 of 17 holdout forecasts hit a calibration bound. Across all 87 five-minute 5H windows, calibrated MAE is `1.7564` versus Fixed `1.7041` (+3.1%). Therefore the current learner is not safe enough to publish or advance unchanged.

Weekly is also not robust: its earlier split improves slightly, but holdout MAE worsens from `0.5535` to `0.5979` (+8.0%).
## Frozen experiment

The replay used the exact parameter grid frozen in the plan before scores were read:

- `λ = 0.80 / 0.90 / 0.95 / 0.98 / 0.99`;
- prior strength `0.5 / 2 / 5 / 10`;
- calibration bounds `[0.5,2.0]`, `[0.75,1.5]`, `[0.8,1.25]`;
- confidence policies `all`, `downweight_low`, and `ignore_zero_near`.

The learner uses only matured five-minute outcomes. At every forecast origin it first incorporates only learning intervals whose `target_at <= origin_at`, then computes the factor. No forecast trains on its own outcome or later data.

Each source file starts from `c=1`; files are not merged into an invented long-lived account. One factor is shared across horizons within a source session/quota. This makes the test conservative relative to the intended persistent per-user learner.

5H and Weekly choose parameters independently from the earlier split only. Holdout parameters are frozen while causal learning continues from newly matured outcomes.

## Selected parameters

5H selected:

```text
lambda = 0.99
prior strength = 10
bounds = [0.75, 1.5]
confidence = ignore_zero_near
nominal effective update memory ≈ 100 updates
```

Its earlier-split MAE is `1.7482`, versus Fixed `1.6315`; p90 is `4.3201` versus Fixed `4.1121`. It remained eligible under the predeclared p90/bound criteria, but it did not beat Fixed on the tuning period.

Weekly selected:

```text
lambda = 0.80
prior strength = 0.5
bounds = [0.5, 2.0]
confidence = ignore_zero_near
nominal effective update memory ≈ 5 updates
```
## Five-minute comparison

| Quota / split | Fixed MAE | Calibrated MAE | Change | Fixed p90 | Calibrated p90 | Fixed bias | Calibrated bias |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 5H earlier | 1.6315 | 1.7482 | +7.2% | 4.1121 | 4.3201 | -1.0410 | -0.6284 |
| 5H holdout | 2.0034 | 1.7899 | -10.7% | 3.1148 | 4.5125 | -0.8324 | -0.0122 |
| 5H all | 1.7041 | 1.7564 | +3.1% | 4.1121 | 4.5125 | -1.0003 | -0.5080 |
| WK earlier | 0.4556 | 0.4430 | -2.8% | 0.8555 | 0.9249 | -0.0951 | -0.0115 |
| WK holdout | 0.5535 | 0.5979 | +8.0% | 0.9596 | 1.1421 | 0.0716 | 0.2468 |
| WK all | 0.4655 | 0.4587 | -1.5% | 0.8698 | 1.0092 | -0.0782 | 0.0146 |

For 5H holdout, the calibrated estimator wins 8 windows, loses 3 and ties 6 versus Fixed. The mean improvement therefore is not caused by dropping difficult forecasts; availability is identical.

The problem is tail behavior. The same 5H holdout has six bound-hit forecasts, and its p90 degradation violates the intended stability requirement.
## Learning-age evidence

The current source-file segmentation permits only short learning histories:

| 5H updates before forecast | Windows | Fixed MAE | Calibrated MAE |
| --- | ---: | ---: | ---: |
| cold start | 24 | 2.1515 | 2.1515 |
| 1–4 | 52 | 1.6516 | 1.7495 |
| 5–9 | 11 | 0.9762 | 0.9269 |
| 10+ | 0 | — | — |

This is consistent with the product hypothesis in one limited sense: after 5–9 realized updates, calibrated error is about 5% lower than Fixed. But the much larger 1–4 bucket is worse, and there is no evidence at all for longer-lived learning because source sessions are intentionally reset.

Weekly shows a similar but weak pattern: `1–4` updates improve from `0.4732` to `0.4640`; `5–9` improve from `0.2945` to `0.2838`. Holdout does not confirm the aggregate improvement.

## Post-primary activation diagnostic

After the frozen primary replay, a descriptive diagnostic tested delaying publication of the learned factor while still learning in the background. This was **not** part of parameter selection and is not confirmation evidence.

For 5H holdout:

- activate immediately: MAE `1.7899`, p90 `4.5125`, 17 active;
- require 3 updates: MAE `1.9040`, p90 `4.5125`, 4 active;
- require 5 updates: MAE `1.9610`, p90 `3.1148`, only 1 active;
- require 8 updates: identical to Fixed; no forecast had enough evidence.

The five-update gate removes the observed p90 regression, but only one holdout forecast actually uses personalization. It is therefore a useful next hypothesis, not a validated solution.
## Interpretation

The strongest positive finding is bias correction. The selected 5H learner sees that Fixed systematically under-predicts future observed consumption and raises the user factor; in holdout this nearly removes mean signed bias.

The strongest negative finding is early-sample variance. With only a few integer-quantized outcomes, the factor can move to the edge of its allowed range. Average error may improve while a small number of forecasts become much worse.

This means the basic architecture remains plausible:

```text
Fixed Weighted × personal correction factor
```

but the current update policy should not be activated immediately after the first few outcomes.

The present dataset also cannot test the main long-term promise — “the estimator becomes increasingly personalized over days of use” — because no source session contributes 10 or more usable updates and cross-file account continuity was deliberately not assumed.

## Decision

**Do not integrate the current personal calibration into the published Burn Rate or production state yet.**

The first replay fails the p90/stability requirement and does not show aggregate 5H improvement across the full frozen dataset.

The next validation should preserve the learner as a shadow-only candidate and collect actual overlay observations with persistent account-safe state. The highest-priority follow-up hypotheses are:

1. learn immediately but delay applying the factor until a minimum evidence threshold;
2. use a stronger/longer cold-start prior and/or an explicit maximum factor-change per update;
3. evaluate time-based forgetting once multi-day continuous overlay history exists;
4. repeat the frozen evaluation on a confirmation dataset with substantially more 10+, 20+ and 50+ update forecasts.

No production estimator, UI, saved-state schema, polling behavior, version, or running overlay was changed by this experiment.
