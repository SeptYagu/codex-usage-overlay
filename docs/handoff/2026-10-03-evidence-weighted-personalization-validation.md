# Evidence-Weighted Personal Calibration — Validation Result

Date: 2026-10-03
Plan baseline: `fa5ba90`
Dataset: unchanged frozen real-history artifact, 3,854 quota observations from 30 source files.

## Result

The evidence-weighted personalization hypothesis is **supported for 5H** on the current frozen dataset.

The previous full-strength personal calibration improved 5H holdout MAE but caused a large p90 regression. Scaling the correction factor by causal evidence removes that tail regression while retaining a smaller but consistent average-error improvement.

Selected 5H trust rule:

```text
evidence = confidence-weighted valid update count
K = 8
trust = evidence / (evidence + 8)
effective_factor = 1 + trust × (raw_personal_factor - 1)
```

Under the frozen `ignore_zero_near` confidence policy, confidence is binary, so `confidence_count` and `update_count` are numerically identical in this dataset. The serialized winner is `confidence_count;K=8`; `update_count;K=8` produces exactly the same scores.

Weekly does not receive the same conclusion. Evidence weighting attenuates the previous regression but does not beat Fixed on holdout.
## Primary 5H five-minute result

| Split | Fixed MAE | Full personal MAE | Evidence-weighted MAE | Fixed p90 | Full personal p90 | Evidence-weighted p90 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Earlier | 1.6315 | 1.7482 | **1.6189** | 4.1121 | 4.3201 | **3.9462** |
| Holdout | 2.0034 | **1.7899** | **1.9363** | **3.1148** | 4.5125 | **3.1148** |
| All | 1.7041 | 1.7564 | **1.6809** | 4.1121 | 4.5125 | **3.9462** |

Relative to Fixed:

- earlier MAE improves about 0.8% and p90 improves about 4.0%;
- holdout MAE improves about 3.3% while p90 is unchanged;
- all-window MAE improves about 1.4% and p90 improves about 4.0%.

Holdout equal-session MAE improves from `1.5698` to `1.5291`.

Holdout wins/losses/ties versus Fixed are `8 / 3 / 6`. This is the same directional set as full personal calibration, but the magnitude of each correction is attenuated by evidence trust.
## Why the stability improved

The selected 5H trust level remains deliberately low in this sparse dataset:

```text
holdout mean trust ≈ 0.136
holdout max trust ≈ 0.385
all-window mean trust ≈ 0.178
all-window max trust = 0.5
```

Therefore the personal learner may internally reach its raw correction bounds, but the published effective correction remains much narrower.

For 5H holdout the evidence-weighted effective factor ranges approximately:

```text
0.9722 .. 1.1667
```

rather than exposing the raw learner's full `0.75 .. 1.5` bounds.

Bias correction is correspondingly more conservative:

```text
Fixed holdout bias          -0.8324
Full personal holdout bias  -0.0122
Weighted holdout bias       -0.6387
```

The weighted version does not eliminate the systematic under-prediction as aggressively, but it avoids paying for that correction with a large tail-error increase.
## Learning-age result

The important improvement over the previous experiment is that the weighted version no longer needs to make early evidence fully authoritative.

| Prior valid updates | Windows | Fixed MAE | Full personal MAE | Weighted MAE |
| --- | ---: | ---: | ---: | ---: |
| 0 | 24 | 2.1515 | 2.1515 | 2.1515 |
| 1–2 | 32 | 1.6191 | 1.7076 | **1.5859** |
| 3–4 | 20 | 1.7036 | 1.8165 | **1.6879** |
| 5–6 | 7 | 0.8313 | 0.8866 | **0.8043** |
| 7–9 | 4 | 1.2298 | **0.9976** | 1.1157 |

This is the strongest evidence for the new design. Full personal calibration was worse than Fixed during the early 1–6 update region. Evidence weighting is better than Fixed in every populated non-cold 5H bucket.

At 7–9 updates the full personal factor becomes more effective than the weighted factor, suggesting that larger datasets may eventually justify higher trust. The present dataset contains no 10+ evidence bucket, so the asymptotic handoff to near-full personalization is still untested.
## K tradeoff

The 5H count-based family shows the expected continuous stability/response tradeoff.

Selected and nearby candidates:

```text
K=5   earlier MAE 1.6203 / p90 3.9849   holdout MAE 1.9070 / p90 3.3065
K=8   earlier MAE 1.6189 / p90 3.9462   holdout MAE 1.9363 / p90 3.1148
K=10  earlier MAE 1.6195 / p90 3.9738   holdout MAE 1.9476 / p90 3.1148
K=15  earlier MAE 1.6210 / p90 4.0145   holdout MAE 1.9641 / p90 3.1148
K=20  earlier MAE 1.6227 / p90 4.0367   holdout MAE 1.9731 / p90 3.1148
```

Smaller `K` grants personal data more authority sooner. It improves holdout MAE more strongly but begins to increase tail error. Larger `K` is progressively more conservative.

This smooth curve supports the interpretation that evidence weighting is controlling a real bias/variance tradeoff rather than exploiting an isolated parameter point.

The information-weighted `q × p²` family also becomes stable as `K` increases, but on the frozen earlier split it does not outperform the selected count-based family.
## Weekly result

The frozen earlier split selects the most aggressive eligible count rule, `K=1`.

Five-minute results:

```text
Earlier: Fixed 0.4556 → weighted 0.4480
Holdout: Fixed 0.5535 → weighted 0.5781
All:     Fixed 0.4655 → weighted 0.4611
```

Weekly holdout p90 also worsens from `0.9596` to `1.0508`.

Therefore Weekly personalization is **not validated**. Its short-horizon quota increments are much smaller and more quantized, so the current dataset gives the learner weaker signal. More conservative Weekly trust rules approach Fixed on holdout, but selecting one after inspecting holdout would violate the frozen protocol.

Weekly should remain Fixed in any next-stage shadow design unless a new confirmation experiment validates a separate policy.

## Secondary horizons

The 15-minute 5H sample is small but directionally consistent:

```text
11 windows
Fixed MAE            3.8757
Full personal MAE    4.6458
Evidence-weighted    3.7465
```

Its p90 also improves slightly, `7.4438 → 7.3895`.

Thirty-minute coverage is only two 5H windows and one Weekly window; 60-minute coverage is zero. These horizons are not decision evidence.
## Decision

The first personal-calibration experiment showed that a scalar user correction contains useful signal but was too unstable when applied at full strength immediately.

This second experiment shows that **evidence-weighted publication fixes that specific failure for 5H on the frozen dataset**.

Recommended next experimental architecture:

```text
Fixed Weighted estimator
        ↓
personal learner updates from matured outcomes
        ↓
raw personal factor c
        ↓
evidence trust g = E / (E + K)
        ↓
effective factor = 1 + g(c - 1)
        ↓
shadow personalized 5H prediction
```

For the next stage, use the validated 5H `K=8` count-equivalent trust policy as a **shadow-only** candidate while collecting persistent overlay history. Keep Weekly unpersonalized.

Do not publish this personalized value to the UI yet. The current evidence is still limited to at most eight prior trusted updates at a scored 5H origin, with no 10+, 20+, or 50+ evidence region.

The next confirmation objective is specifically to verify that:

1. the 5H benefit persists across continuous multi-day overlay history;
2. p90 remains no worse than Fixed as trust grows above 0.5;
3. larger evidence eventually justifies approaching full personal calibration;
4. account switches, long idle periods, resets, and restart persistence do not contaminate personal state.

No production estimator, UI, saved-state schema, polling behavior, version, or running overlay was modified in this validation.
