# Personal Burn-Rate Calibration Learning — Validation Plan

Date: 2026-10-03  
Baseline: `98f9bb4` (`test: compare burn rate candidates on real quota history`)  
Scope: validation only; no production estimator, schema, UI, release, or service change.

## 1. Goal

Test whether a lightweight per-user calibration layer can improve the selected fixed-weighted burn-rate estimator as real usage evidence accumulates.

The proposed production shape, if validated, is:

```text
quota history
    ↓
fixed weighted estimator
    ↓
raw rate (%/h)
    ↓
personal calibration factor c
    ↓
published rate = raw rate × c
```

This experiment does **not** learn the regression half-life itself. The fixed weighted estimator remains the global default model; personalization learns only a multiplicative correction factor.

## 2. Hypothesis

A single global estimator can have persistent user-specific bias because usage rhythm, session structure, model mix, and burstiness differ between users.

If the base forecast repeatedly under-predicts or over-predicts one user's future observed quota increment, a causal online calibration factor should reduce future error without materially increasing variance, warm-up failures, or transient excursions.

The factor must start at `1.0`, learn only from outcomes already observed, forget stale behavior gradually, and remain bounded against integer-quota quantization noise.
## 3. Online calibration model

For each eligible forecast origin, let:

- `p_t` = base estimator's predicted quota increment over the validation horizon.
- `y_t` = later observed quota increment over the same interval.
- `c_t` = personal calibration factor available at the forecast origin.
- calibrated prediction = `c_t × p_t`.

After the outcome becomes observable, update two decayed sufficient statistics:

```text
A_t = λ A_(t-1) + q_t p_t y_t
B_t = λ B_(t-1) + q_t p_t²
c_t = A_t / B_t
```

This is an exponentially decayed one-parameter least-squares fit for `y ≈ c p`. It avoids maintaining multiple estimator instances and requires only constant-time arithmetic and a few persisted scalars.

Cold start uses a prior centered on `c = 1.0`. The replay must test prior strengths rather than silently choosing one after seeing final scores.

Candidate forgetting factors should include values corresponding roughly to short, medium, and long behavioral memory. The experiment should report effective memory in observations and elapsed time, not only raw `λ`.

## 4. Feedback confidence

Not every realized interval carries equal information. Codex quota observations in the current real-history dataset are integer percentages, so a small observed change can be dominated by endpoint quantization.

Use a confidence term `q_t ∈ [0,1]` in the update, but treat its policy as an explicit experimental parameter.

Required candidate policies:

1. `q=1` for every otherwise valid interval — control.
2. Down-weight low-information intervals where both predicted and observed increments are near the integer-resolution floor.
3. Ignore only clearly uninformative zero/near-zero pairs as a separate sensitivity test.

Do not use future regime labels, future rate changes, model identity, conversation contents, or any other information unavailable to the live estimator.

The calibration factor must be bounded during validation. Test conservative bounds such as `[0.5, 2.0]` and at least one tighter range. A bound hit must be counted and reported rather than hidden.
## 5. Causal replay protocol

Extend the existing real-history forecast benchmark rather than inventing a different scoring task.

Primary comparison:

```text
Legacy
Fixed weighted
Fixed weighted + personal calibration
Zero-increment sanity baseline
```

The personalized candidate must use exactly the same fixed-weighted raw estimate as the current preferred candidate.

At forecast time `t`, the calibration state may contain only outcomes whose target observation time is already ≤ `t`. A forecast must never train its own calibration factor or consume any later observation before producing its prediction.

Keep the existing validity rules for gaps, reset changes, corrections, saturation, common-availability matching, and horizon selection. Preserve the current 5-minute task as the primary endpoint; 15/30/60-minute results remain secondary where coverage exists.

Because the frozen real dataset cannot establish account continuity across source session files, **do not merge calibration state across files**. Each source file starts at `c=1.0`. This is deliberately conservative and models repeated cold starts rather than pretending the files are one user history.

A second phase must use actual overlay observations with persisted account-safe state to test the intended long-lived learning behavior.

## 6. Leakage and evaluation discipline

Parameter tuning and evaluation must be separated chronologically.

Use an early interval only to select:

- forgetting factor `λ`;
- prior strength;
- confidence policy;
- calibration bounds.

Freeze those choices before scoring a later chronological interval.

Do not choose parameters from aggregate all-period MAE and then report the same period as validation.

If the current frozen dataset is too small for a defensible train/validation split, report the experiment as exploratory and do not authorize production integration. A later overlay-collected dataset should then become the confirmation set.
## 7. Metrics

For every quota and horizon, report the base Fixed and calibrated Fixed on identical windows.

Required metrics:

- mean absolute increment error (MAE), percentage points;
- median absolute error;
- p90 absolute error;
- signed mean error/bias;
- equal-session-weighted MAE;
- win / loss / tie counts against uncalibrated Fixed;
- availability;
- calibration-factor mean, median, range, and bound-hit count;
- number of realized learning updates before each forecast;
- error as a function of learning age / evidence count.

For 5H, additionally publish a learning curve such as:

```text
cold start
1–4 realized updates
5–9
10–19
20–49
50+
```

The central product claim under test is not merely “final MAE is lower.” It is whether error decreases as valid personal evidence accumulates without unacceptable tail regressions.

## 8. Validation gates

A personalized candidate may advance to shadow integration only if all of the following hold on frozen evaluation data:

1. 5H calibrated MAE is lower than uncalibrated Fixed on matched windows.
2. Improvement is not explained solely by one source session; equal-session weighting must also improve or be statistically indistinguishable while aggregate error improves materially.
3. p90 error does not materially worsen.
4. Signed bias moves toward zero or remains negligible.
5. Availability is identical to Fixed; calibration must never create a new unavailable estimate.
6. Calibration does not produce frequent bound hits or unstable factor oscillation.
7. Later evidence-count buckets are no worse than cold-start buckets in a pattern inconsistent with learning.
8. Weekly is evaluated separately; no 5H result is used to justify Weekly calibration.

No percentage improvement threshold should be retrofitted after seeing the result. If a minimum materiality threshold is desired, define it before the confirmation replay.
## 9. State and lifecycle contract for a future shadow prototype

If replay passes, implement calibration first in shadow mode. The displayed Burn Rate remains the uncalibrated production value until a separate review authorizes publication.

Per quota, shadow state should require only:

```text
A
B
realized_update_count
last_update_at
selected/frozen learner parameter version
```

Derived `c=A/B` does not need to be persisted separately unless useful for diagnostics.

Account switching must isolate or clear calibration exactly as raw burn-rate history is isolated. A confirmed quota reset does not necessarily imply the user's behavioral calibration should be forgotten; replay/shadow evidence must decide this explicitly rather than inheriting crossing-reset semantics by accident.

Long inactivity should decay confidence/history according to the chosen forgetting model. Corrupt or incompatible calibration state must fall back safely to `c=1.0`.

Missing/invalid quota observations, regression corrections, reset-boundary intervals, excessive gaps, and saturated outcomes must not update the learner.

## 10. Important failure modes to test

The validation suite must include:

- base prediction close to zero, avoiding `actual/predicted` ratio explosions;
- integer observations that alternate between zero and one-point increments;
- sudden high-burn bursts followed by idle periods;
- sustained constant consumption;
- acceleration and deceleration;
- long idle then resume;
- restart with preserved calibration state;
- account transition;
- quota reset;
- malformed persisted state;
- repeated low-information feedback;
- behavior regime change after the learner has accumulated strong old evidence.

The direct ratio `y/p` may be reported diagnostically but must not be used as the production update rule.
## 11. Experiment artifacts

Add a dedicated offline replay module or extension under the existing burn-rate replay test code. Keep all learning logic test-only for this phase.

Recommended committed outputs:

```text
docs/handoff/2026-10-03-personal-calibration-validation-plan.md
docs/handoff/burn-rate-personal-calibration-evidence/summary.csv
docs/handoff/burn-rate-personal-calibration-evidence/learning-curve.csv
docs/handoff/2026-10-03-personal-calibration-validation.md
```

Large per-forecast traces remain ignored under `output/`.

The final validation handoff must record:

- exact frozen dataset identity and timestamp range;
- all candidate parameter values;
- chronological tuning/evaluation boundary;
- Fixed baseline metrics;
- calibrated metrics;
- learning curves;
- bound-hit and stability diagnostics;
- whether the gates passed;
- explicit decision: reject, continue shadow validation, or propose production design.

## 12. Decision boundary

This plan intentionally does **not** assume personalization will work.

A positive result means only that a one-parameter user calibration layer is worth validating in real overlay shadow operation. It does not authorize replacing Fixed Weighted, changing the UI, or claiming that instantaneous burn rate is ground truth.

A negative result is also useful: it would show that most forecast error comes from temporal-shape/quantization effects that a scalar correction cannot repair, and that further work should return to estimator structure rather than user calibration.

The preferred sequence is therefore:

```text
existing real-history replay
→ frozen personal-calibration replay
→ review
→ actual overlay shadow learning
→ confirmation dataset
→ production decision
```
