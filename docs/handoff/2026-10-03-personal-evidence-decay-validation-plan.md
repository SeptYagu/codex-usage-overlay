# Personal Evidence Decay — Validation Plan

Date: 2026-10-03
Baseline: `589fbb4` (`test: validate evidence weighted personalization`)
Scope: offline validation only; no production estimator, UI, saved state, polling, service, or release change.

## 1. Question

The current validated 5H shadow candidate uses:

```text
raw personal learner: lambda = 0.99
trust: g = E / (E + 8)
E = cumulative valid feedback count
```

The raw learner already fades old calibration evidence through `lambda=0.99`, but the trust evidence `E` currently grows without bound.

This experiment tests whether trust should also have finite memory, either by exponential decay or by a hard recent-feedback window.
## 2. Important distinction

Two different memories are involved:

1. **Calibration content** — `A/B`, which determines the personal factor `c`. This already uses `lambda=0.99`, giving approximately a 100-feedback effective memory.
2. **Calibration trust** — `E`, which determines how much of `c` affects the published value.

This experiment changes only trust memory. The personal learner itself remains frozen.

Under continuous valid feedback, exponential trust evidence obeys:

```text
E_t = rho * E_(t-1) + q_t
```

and approaches `1/(1-rho)` when `q=1`. Thus `rho=0.99` caps effective trust evidence near 100 rather than allowing it to grow indefinitely.

A hard-100 control instead keeps exactly the most recent 100 valid feedback contributions.
## 3. Frozen candidates

Keep the 5H personal learner and trust scale frozen:

```text
personal lambda = 0.99
prior = 10
raw factor bounds = [0.75, 1.5]
confidence = ignore_zero_near
K = 8
```

Trust-memory candidates:

- cumulative count: current validated control;
- exponential `rho=0.95` (~20-feedback effective memory);
- exponential `rho=0.98` (~50-feedback effective memory);
- exponential `rho=0.99` (~100-feedback effective memory);
- exponential `rho=0.995` (~200-feedback effective memory);
- hard recent-100 feedback window.

All candidates use:

```text
g = E / (E + 8)
effective_factor = 1 + g * (c - 1)
```
## 4. Real-history replay

Run every candidate on the unchanged frozen real-history artifact.

Because a scored 5H origin currently has at most eight prior valid feedbacks, this replay is a **non-regression check**, not a decisive long-memory selection test.

Required five-minute metrics:

- MAE, median, p90, signed bias;
- equal-session MAE;
- win/loss/tie versus Fixed;
- mean/min/max trust;
- candidate-vs-current cumulative differences.

No candidate may be called superior solely from this sparse real replay.

## 5. Long-history stress replay

Add deterministic test sequences with at least 300 matured feedbacks so the memory policies separate materially.

The stress layer tests the trust system independently from quota-crossing reconstruction. Each step provides a frozen Fixed prediction `p` and realized outcome `y`; the existing `A/B` personal learner and candidate trust memory are updated causally.
Required scenarios:

1. **Stable bias**: Fixed continuously under-predicts by a constant factor.
2. **Bias reversal**: 150 feedbacks under-predicted, then 150 over-predicted.
3. **Bias removal**: 150 biased feedbacks, then Fixed becomes correct.
4. **Noisy stable bias**: deterministic alternating quantization noise around a constant personal factor.
5. **Sparse informative feedback**: many low-information outcomes interspersed with high-information outcomes.

For each scenario report:

- trust after 10 / 20 / 50 / 100 / 150 / 300 updates;
- effective factor;
- post-change integrated absolute prediction error;
- time/updates to return within 5% and 10% of the new target factor where applicable;
- maximum post-change overshoot;
- asymptotic trust.

The hard-100 candidate must retain only the last 100 eligible `q` contributions. No future observation may affect an earlier prediction.
## 6. Selection rule

The real-history replay is a safety gate:

- 5H p90 must not materially worsen versus the current cumulative `K=8` candidate;
- MAE must remain within 1% of the current candidate unless long-history stress shows a clear compensating stability benefit.

Among candidates that pass the real-data gate, use long-history stress to choose the most conservative finite-memory rule that:

1. preserves stable-bias convergence;
2. reduces stale-history influence after a regime change;
3. avoids discontinuities from hard-window eviction;
4. does not create oscillatory trust.

If exponential `rho=0.99` performs comparably to hard-100, prefer exponential decay because it needs only one scalar state and removes the arbitrary 100/101 discontinuity.

## 7. Interpretation boundary

Exponential count decay does **not** by itself detect a behavior regime change. Under a steady stream of valid feedback, `E` approaches a finite steady state rather than dropping when behavior changes.

Its purpose is to cap the authority granted by historical evidence and make old evidence contributions fade smoothly. Adaptation of the personal factor itself still comes from the frozen `A/B` learner's `lambda=0.99`.

If future testing shows trust should actively fall when user behavior changes, that requires a separate change detector or elapsed-time confidence decay and is outside this experiment.
