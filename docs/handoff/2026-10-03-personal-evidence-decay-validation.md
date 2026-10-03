# Personal Evidence Decay — Validation Result

Date: 2026-10-03
Plan baseline: `cf539fc`
Dataset: unchanged frozen real-history artifact plus deterministic 300-feedback stress replays.

## Result

The finite-memory trust hypothesis is supported, but the existing real dataset is too short to choose a decay policy by itself.

The preferred 5H trust-memory candidate is:

```text
E_t = 0.99 * E_(t-1) + q_t
g_t = E_t / (E_t + 8)
```

with the previously validated personal learner unchanged.

This gives trust evidence an approximately 100-feedback effective memory. Old evidence contributions fade smoothly instead of remaining permanently counted, while the total evidence approaches about 100 under continuous high-quality feedback.

A hard recent-100 window produces very similar long-run behavior, but `rho=0.99` is preferred because it is continuous and requires only one persisted scalar.
## Real-history non-regression check

The real dataset still contains at most eight prior valid 5H feedbacks at any scored origin, so all finite-memory policies remain close to the current cumulative-count behavior.

5H holdout:

| Trust memory | MAE | p90 | Mean trust | Max effective evidence |
| --- | ---: | ---: | ---: | ---: |
| Cumulative | 1.936325 | 3.114754 | 0.1048 | 5.000 |
| rho 0.95 | 1.937509 | 3.114754 | 0.1017 | 4.524 |
| rho 0.98 | 1.936798 | 3.114754 | 0.1035 | 4.804 |
| rho 0.99 | 1.936561 | 3.114754 | 0.1041 | 4.901 |
| rho 0.995 | 1.936443 | 3.114754 | 0.1045 | 4.950 |
| Hard 100 | 1.936325 | 3.114754 | 0.1048 | 5.000 |

All candidates preserve the previously validated p90 behavior. The largest MAE difference versus cumulative is only about 0.0012 quota points.

As expected, hard-100 is exactly identical to cumulative in the real replay because no scored origin has anywhere near 100 previous feedbacks.
## Long-run trust behavior

Under a stable stream of informative feedback, trust evidence separates strongly.

At 300 feedbacks:

| Policy | Effective evidence E | Trust g | Effective factor for true 1.20 bias |
| --- | ---: | ---: | ---: |
| Cumulative | 299.0 | 0.974 | 1.195 |
| rho 0.95 | 20.0 | 0.714 | 1.143 |
| rho 0.98 | 49.9 | 0.862 | 1.172 |
| rho 0.99 | 95.0 | 0.922 | 1.184 |
| rho 0.995 | 155.3 | 0.951 | 1.190 |
| Hard 100 | 100.0 | 0.926 | 1.185 |

This confirms the expected soft-window interpretation:

- `rho=0.95` behaves like roughly 20 feedbacks of memory;
- `rho=0.98` like roughly 50;
- `rho=0.99` like roughly 100;
- `rho=0.995` like roughly 200.

The cumulative control continues toward full trust indefinitely.
## Stable-bias tradeoff

For a persistent true correction factor of 1.20, lower decay preserves more of the personalization benefit.

Integrated absolute prediction error over 300 feedbacks:

```text
cumulative  14.03
rho 0.995   16.19
hard 100    16.58
rho 0.99    18.70
rho 0.98    24.15
rho 0.95    39.03
```

The `rho=0.95` policy is too conservative: even after 300 valid feedbacks its effective factor reaches only about 1.143 for a true 1.20 bias.

`rho=0.99` reaches within 5% of the target after about 28 feedbacks, compared with 26 for hard-100 and 26 for cumulative. The practical convergence penalty relative to hard-100 is therefore small.
## Regime-change behavior

The trust-memory policy does not itself detect a regime change. The raw personal factor still adapts according to the frozen `A/B` learner with `lambda=0.99`.

When a prior 1.20 bias disappears and Fixed becomes correct, finite trust helps by keeping the published result closer to the global estimator while the raw personal factor relearns:

| Policy | Post-change integrated error | Within 10% | Within 5% |
| --- | ---: | ---: | ---: |
| Cumulative | 26.94 | 54 | 117 |
| rho 0.95 | 20.03 | 29 | 89 |
| rho 0.98 | 24.10 | 44 | 106 |
| rho 0.99 | 25.65 | 50 | 113 |
| rho 0.995 | 26.35 | 52 | 115 |
| Hard 100 | 25.96 | 51 | 113 |

Thus `rho=0.99` gives a modest improvement over cumulative and a small improvement over hard-100 when the old personal bias disappears.

For a full bias reversal from 1.20 to 0.80, no finite-trust rule materially improves adaptation. The bottleneck is the raw personal learner's own `lambda=0.99`, not trust memory. This experiment therefore does not justify changing the learner decay.
## Hard-100 comparison

Hard-100 and `rho=0.99` are deliberately close in steady state:

```text
hard-100 steady E = 100
hard-100 steady trust = 100 / 108 = 0.9259

rho=0.99 steady E -> 100
rho=0.99 steady trust -> 100 / 108 = 0.9259
```

Their stress scores are correspondingly close.

Hard-100 has two disadvantages:

1. it requires retaining a queue or equivalent 100-feedback history for exact behavior;
2. an individual contribution is fully present until eviction and then disappears at the window boundary.

Exponential decay needs only `E` and reduces each old contribution continuously by another factor of `0.99` per new feedback.

For one old contribution:

```text
after 50 newer feedbacks:  ~60.5% remains
after 100:                 ~36.6%
after 200:                 ~13.4%
after 300:                  ~4.9%
```

This is the intended “soft recent-100” behavior.
## Sparse-information stress

The confidence-aware test also included many low-information outcomes with only periodic informative feedback.

Final effective evidence after 300 steps:

```text
cumulative 59.0
rho 0.95    3.60
rho 0.98    9.58
rho 0.99   18.59
rho 0.995  30.57
hard 100   20.0
```

Here `rho=0.99` again tracks hard-100 closely without needing the queue.

This matters because the trust system should measure usable recent evidence, not merely elapsed observations.

## Decision

Adopt `rho=0.99` as the preferred **shadow validation design** for 5H trust evidence:

```text
A_t = 0.99 A_(t-1) + q p y
B_t = 0.99 B_(t-1) + q p^2
c_t = clamp(A_t / B_t)

E_t = 0.99 E_(t-1) + q
g_t = E_t / (E_t + 8)

effective_factor = 1 + g_t (c_t - 1)
personalized_rate = fixed_rate * effective_factor
```

The matching `0.99` values are not assumed to be universally optimal; they are simply supported by the current validation as a coherent approximately-100-feedback memory for both calibration content and trust.

Do not implement a hard recent-100 queue unless later production evidence shows a measurable advantage.

This result still does not authorize publishing personalized Burn Rate in the UI. The next useful step is persistent overlay shadow collection so real users can accumulate 10+, 20+, 50+, and 100+ feedback histories.

No production estimator, UI, saved-state schema, polling behavior, version, or running overlay was modified.
