# Evidence-Weighted Personal Calibration — Validation Plan

Date: 2026-10-03
Baseline: `8cbf153` (`test: validate personal burn rate calibration`)
Scope: offline validation only; no production estimator, UI, saved-state schema, polling, service, or release change.

## 1. Question

The first personal-calibration replay found useful bias correction but excessive early-sample variance. This experiment tests one isolated change:

> Keep learning the personal correction factor from the first valid outcome, but scale how much of that factor is allowed to affect the published prediction according to the amount of causal evidence accumulated so far.

The underlying personal learner is frozen from the previous experiment. This run must not retune its `lambda`, prior, factor bounds, or confidence policy.

## 2. Frozen personal learners

5H remains:

```text
lambda = 0.99
prior = 10
factor bounds = [0.75, 1.5]
confidence policy = ignore_zero_near
```

Weekly remains:

```text
lambda = 0.80
prior = 0.5
factor bounds = [0.5, 2.0]
confidence policy = ignore_zero_near
```

The raw learned correction is still `c = clamp(A/B)`.
## 3. Evidence-weighted publication

Instead of publishing `fixed × c` immediately, define a trust value `g ∈ [0,1]` and use:

```text
effective_factor = 1 + g × (c - 1)
prediction = fixed_prediction × effective_factor
```

At zero evidence, `g=0`, so the output is exactly Fixed. As evidence increases, `g` approaches 1 continuously.

The primary trust family is:

```text
g = E / (E + K)
```

where `E` is evidence accumulated only from matured five-minute learning outcomes whose target time is no later than the current forecast origin.

This avoids a discontinuous “enable after N samples” switch.

## 4. Evidence definitions

Test three evidence definitions independently.

1. `update_count`: add 1 for every matured learning outcome whose confidence `q > 0`.
2. `confidence_count`: add the same feedback confidence `q` used by the frozen learner.
3. `information`: add `q × p²`, where `p` is the Fixed five-minute predicted quota increment for that matured outcome.

The third form approximates the observation's contribution to the calibration denominator and therefore gives more trust to intervals that contain more measurable quota movement.

No future observation, observed regime label, model identity, conversation content, or target outcome may contribute before it matures.
## 5. Frozen trust parameter grid

Freeze the following grid before scores are inspected:

- `update_count`: `K ∈ {1,2,3,5,8,10,15,20}`;
- `confidence_count`: `K ∈ {1,2,3,5,8,10,15,20}`;
- `information`: `K ∈ {1,2,5,10,20,50,100}`.

Controls:

- Fixed Weighted: `g=0`;
- full personal calibration: `g=1`;
- the post-primary hard activation diagnostic from the previous run remains descriptive only and is not a selection candidate.

5H and Weekly select trust parameters independently using only the existing `earlier` split.

A trust candidate is eligible for selection only if, on the earlier five-minute split:

1. availability is identical to Fixed;
2. p90 error is no more than 2% worse than Fixed;
3. MAE is no more than 2% worse than Fixed.

Among eligible candidates select lowest MAE, then lowest equal-session MAE, then lowest p90, then serialized candidate key. If none are eligible, select Fixed (`g=0`) and record selection failure.

The selected trust rule is then frozen before holdout scoring.
## 6. Required evidence

For each quota and split report:

- MAE, median, p90 and signed bias;
- equal-session MAE;
- win/loss/tie counts versus Fixed;
- mean/min/max trust `g`;
- mean/min/max effective factor;
- evidence distribution at forecast time;
- results by evidence-age bucket.

For 5H, explicitly compare:

```text
Fixed
full personal calibration
selected evidence-weighted calibration
```

The key success criterion is whether evidence weighting retains meaningful bias/MAE improvement while removing the p90 regression seen in the full-calibration holdout.

Also report all candidates, not only the selected winner, so the shape of the `K` tradeoff remains auditable.

## 7. Interpretation boundary

This frozen dataset contains 3,854 raw quota observations but only 87 valid 5H and 89 valid Weekly non-overlapping five-minute learning outcomes under the current conservative protocol. Per-session evidence remains short.

Therefore this experiment can test whether gradual trust is superior to immediate trust in sparse data. It cannot establish the asymptotic behavior after dozens or hundreds of continuous personal updates.

A positive result supports implementing evidence-weighted calibration as a shadow candidate for actual overlay history collection. It does not authorize production publication.

A negative result means the current scalar personal calibration is not sufficiently stabilized by evidence weighting alone.
