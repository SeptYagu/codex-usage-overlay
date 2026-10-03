# 5H Persistent Shadow Personalization — Implementation Plan

Date: 2026-10-03
Algorithm baseline: `662054d` (`test: validate personal evidence decay`)
Plan baseline: `f689e85` (`docs: add persistent 5h shadow personalization plan`)
Scope: production-quality shadow implementation and local validation data collection; **no user-visible estimator switch**.

Revision: reconciles the full 2026-10-03 estimator/personalization research chain and closes implementation gaps around causal shadow scoring, feedback-count semantics, polling-cadence compatibility, algorithm-version isolation, and crash-safe evidence collection.

## 1. Goal

Implement the validated 5-hour personalization algorithm in the real application so it can accumulate persistent, account-safe, causal feedback over normal use.

The implementation must produce a shadow personalized 5H rate and validation evidence while leaving the current published 5H and Weekly Burn Rate behavior unchanged.

The immediate product objective is not “ship the new number.” It is to collect trustworthy 10+, 20+, 50+, and 100+ feedback histories from the actual overlay lifecycle so a later decision can determine whether the shadow value is safe to publish.

### 1.1 Research decision ledger

This plan is the implementation endpoint of a sequence of frozen experiments. An implementer must preserve the following decisions rather than re-selecting algorithms while coding:

| Stage | Evidence | Decision carried into this plan |
| --- | --- | --- |
| Real-history estimator comparison | 5H five-minute MAE: Legacy `2.047`, Fixed Weighted `1.704`, Adaptive `2.258`; Fixed p90 `4.112` vs Legacy `4.839` | Use Fixed Weighted as the shadow base candidate. Do not include adaptive detection or fractional interpolation. |
| First personal calibration | 5H holdout MAE `2.0034 -> 1.7899`, but p90 `3.1148 -> 4.5125`; full-dataset MAE also worsened | Multiplicative personal bias is real enough to study, but full-strength early personalization is unsafe. |
| Evidence-weighted personalization | With `K=8`, holdout MAE `2.0034 -> 1.9363` while p90 remains `3.1148`; all-window MAE `1.7041 -> 1.6809` | Learn immediately, but attenuate the correction by evidence trust. Keep Weekly unpersonalized. |
| Evidence-decay validation | `rho=0.99` closely tracks a hard recent-100 window while requiring one scalar and forgetting continuously | Use exponentially decayed trust evidence as the preferred shadow design; do not implement a hard 100-item queue. |

Important interpretation boundaries:

- the real-history data were active-session observations, not complete all-day overlay polling;
- no scored 5H origin had 10+ prior trusted updates, so long-lived personalization is still unvalidated;
- the Fixed Weighted result is the preferred **shadow base candidate**, not authorization to replace the current published estimator;
- the shadow phase exists specifically to obtain the missing persistent-lifecycle evidence without changing user-visible behavior.

### 1.2 Three distinct quantities must remain separate

The implementation and later reports must not conflate:

1. `maturedFeedbacks`: every valid five-minute outcome that reaches maturation, including `q=0` low-information outcomes;
2. `informativeUpdates`: matured outcomes with `q>0`; this matches the update-count concept used by the evidence-weighted validation;
3. `evidence E`: the decayed trust scalar `E_t = 0.99E_(t-1)+q`.

The “10+/20+/50+/100+ feedback history” collection target refers primarily to **informative update count before a forecast**, with matured-feedback count also reported. It must not use `E` as a literal lifetime-count bucket: with `rho=0.99` and `q<=1`, `E` approaches 100 asymptotically and does not behave like an integer recent-100 counter.

## 2. Frozen algorithm contract

The shadow stack is:

```text
existing accepted 5H quota history
        ↓
Fixed Weighted 5H estimator
        ↓
5-minute causal outcome learner
        ↓
raw personal correction c
        ↓
decayed evidence trust g
        ↓
shadow personalized 5H rate
```

Weekly is out of scope for personalization and remains completely unchanged.
### 2.1 Fixed Weighted base estimator

Promote the already-tested Fixed Weighted candidate math from test-only replay code into production-callable internal code.

Frozen 5H parameters:

```text
hard evidence horizon = 1800 s
local window          = 1800 s
half-life             = 300 s
minimum level span    = 2 percentage points
crossing reconstruction = existing production integer crossing path
fractional interpolation = OFF
adaptive detector        = OFF
```

Retain the candidate's conservative idle behavior:

- track the observation time that last confirmed a new crossing;
- if valid idle observations make the fitted rate impossible, establish a monotone non-increasing idle ceiling;
- never let partial horizon expiry raise an established idle ceiling;
- clear weighted idle metadata on a new segment;
- normalize malformed/missing optional metadata conservatively.

Do not change `accept_quota()` semantics to obtain this estimator.
### 2.2 Personal calibration learner

Frozen 5H learner:

```text
A0 = 10
B0 = 10
lambda = 0.99

A_t = 0.99 * A_(t-1) + q * p * y
B_t = 0.99 * B_(t-1) + q * p^2

raw c = A_t / B_t
c = clamp(raw c, 0.75, 1.50)
```

Where:

- `p` is the Fixed Weighted prediction of quota-point increase over the matured forecast interval;
- `y` is the later observed quota-point increase over exactly that interval;
- `q` is the frozen feedback-confidence value.

This layer learns multiplicative bias only. It must not retune the weighted half-life, local window, crossing history, polling cadence, or UI settings.
### 2.3 Feedback confidence

Use the validated `ignore_zero_near` rule:

```text
q = 0  when observed_delta == 0 and predicted_delta < 0.75
q = 1  otherwise
```

Use the normal floating tolerance when testing observed zero.

A matured outcome with `q=0` still advances the decay clock:

```text
A <- 0.99 A
B <- 0.99 B
E <- 0.99 E
```

but contributes no new evidence term. This preserves the exact behavior validated offline.

Do not invent intermediate confidence weights in this implementation.
### 2.4 Evidence-weighted trust

Frozen trust memory:

```text
E0 = 0
rho = 0.99
K = 8

E_t = 0.99 * E_(t-1) + q
g_t = E_t / (E_t + 8)

effective_factor = 1 + g_t * (c_t - 1)
shadow_rate = fixed_weighted_rate * effective_factor
```

Properties intentionally preserved:

- new installations start exactly at the global Fixed Weighted estimate;
- recent personal evidence gradually gains authority;
- continuous high-quality feedback makes `E` approach about 100;
- long-old individual contributions decay smoothly rather than dropping at a hard 100/101 boundary;
- trust never reaches 100%, leaving the global estimator as a permanent anchor.

Do not implement a hard recent-100 queue.
## 3. Publication boundary

This implementation is **shadow only**.

The existing production `BurnRateEstimate.five_hour_per_hour` and `weekly_per_hour` continue to be calculated exactly as before and continue feeding `RawCodexUsage::to_ui()`.

The new Fixed Weighted rate and personalized rate must not affect:

- expanded overlay text;
- collapsed pill;
- tray tooltip;
- tray icon;
- notifications;
- settings;
- update behavior;
- polling cadence;
- public frontend DTOs.

No frontend change is required for this phase.

The new values may be exposed only to tests and local shadow evidence recording. A later reviewed plan is required before any user-visible switch.
## 4. Code organization

Recommended structure:

```text
src-tauri/src/codex/burn_rate.rs
src-tauri/src/codex/burn_rate/weighted.rs
src-tauri/src/codex/burn_rate/personal.rs
src-tauri/src/codex/burn_rate/replay.rs
```

`weighted.rs` owns production-safe weighted-fit math and idle-ceiling metadata.

`personal.rs` owns pending five-minute forecasts, maturation/rejection, A/B/E learning, trust, and shadow result structs.

`replay.rs` must import the production helpers after refactoring instead of keeping a second mathematical implementation.

Avoid copying the validated formulas into parallel production and test implementations. One shared implementation is required so future replay continues testing what the app actually runs.
## 5. Persisted state schema

Bump the burn-rate state schema from v1 to v2 and add a defaulted 5H shadow subtree while preserving the existing 5H and Weekly quota history.

Recommended logical shape:

```text
BurnRateStateFile v2
  schemaVersion
  accountId
  fiveHour              existing QuotaRateState
  weekly                existing QuotaRateState
  fiveHourShadow
    algorithmVersion
    accountGeneration
    eventSequence
    auditOutbox
    weighted
    personal
    pending
```

Do not duplicate the main 5H crossing vector inside the shadow state. The shadow Fixed Weighted estimator reads the already-accepted production 5H crossing history.
### 5.1 Weighted metadata

Persist only the additional metadata required by the validated Fixed candidate:

```text
WeightedShadowState
  lastCrossingObservationAt: Option<i64>
  idleRateCeiling: Option<f64>
```

On load, normalize it against the current 5H `QuotaRateState`:

- the crossing bound must lie between the newest eligible crossing and last observed time;
- ceiling must be finite and positive;
- otherwise repair to a conservative valid state or clear the optional metadata.

This normalization should preserve valid old state rather than forcing a new warm-up.
### 5.2 Personal state

Recommended persisted fields:

```text
PersonalCalibrationState
  a: f64                    default 10
  b: f64                    default 10
  evidence: f64             default 0
  maturedFeedbacks: u64     default 0
  informativeUpdates: u64   default 0
  lastUpdateAt: Option<i64>
```

`maturedFeedbacks` increments for every matured interval, including `q=0`. `informativeUpdates` increments only when `q>0`; it is the lifetime learning-age counter used for later 10+/20+/50+/100+ analysis. Neither counter determines trust; only decayed `evidence` does. `accountGeneration` and `eventSequence` live at the shadow-audit level, not inside the learner, and must never be used as model evidence.

Normalize on load:

- `a` and `b` must be finite and positive;
- `evidence` must be finite and non-negative;
- timestamps must be internally valid;
- invalid personal state resets only the personal shadow subtree, not the existing burn-rate history.

The algorithm constants are code constants, not mutable persisted user settings.
### 5.3 Pending forecast

Persist at most one pending 5H learning interval:

```text
PendingPersonalForecast
  algorithmVersion
  originAt
  originUsedPercent
  originResetAt
  originPublishedRatePerHour
  originFixedRatePerHour
  originRawFactor
  originTrust
  originEffectiveFactor
  originShadowRatePerHour
  originEvidence
  originInformativeUpdates
  lastValidAt
```

One pending forecast enforces the same non-overlapping outcome structure used in the real-history validation.

Do not keep a queue of overlapping five-minute predictions.

Persistence allows a very short application restart to continue an interval causally. A restart gap that violates the feedback gap rule invalidates the pending interval on the next valid sample.

The origin estimator snapshot is mandatory. The learner still trains only from the frozen Fixed Weighted prediction `p`; storing the current published rate and personalized shadow rate does **not** change learning. It exists so the matured outcome can later score three causal predictions made at the same origin:

```text
published_delta = originPublishedRatePerHour * elapsed / 3600
fixed_delta     = originFixedRatePerHour     * elapsed / 3600
shadow_delta    = originShadowRatePerHour    * elapsed / 3600
```

Cap each prediction independently by quota remaining at the origin before scoring it against the same observed `y`.

Do not reconstruct the origin personalized prediction from target-time `A/B/E`. By maturation time the learner may already have changed; using the target-time factor would create look-ahead contamination and would make the shadow evidence unsuitable for publication decisions.

### 5.4 Algorithm-version isolation and audit outbox

`fiveHourShadow.algorithmVersion` is a compatibility boundary, not a decorative field.

- if the persisted version exactly matches the running algorithm, resume normally;
- if a future code version changes a frozen learner constant, confidence rule, forecast horizon, trust rule, or base-estimator semantics, it must declare an explicit migration;
- absent an explicit compatible migration, preserve legacy quota history and the audit identity (`accountGeneration/eventSequence`) but reset incompatible personal/weighted/pending shadow state to neutral defaults rather than mixing evidence generated under different algorithms.

Because the purpose of this phase is evidence collection, a crash between updating learner state and appending the diagnostic log must not silently lose a matured outcome. Keep a persisted at-least-once audit slot under `fiveHourShadow`:

```text
accountGeneration: u64
eventSequence: u64
auditOutbox: Option<ShadowEvidenceEvent>
```

On a matured/rejected/lifecycle event:

1. require the audit slot to be empty;
2. increment `eventSequence`;
3. apply the state transition and place the fully formed evidence event in `auditOutbox`;
4. atomically persist the burn-rate state;
5. append the outbox event to the JSONL evidence log;
6. clear the outbox and persist that clear opportunistically with the next state-changing save.

Before processing any new personal-learning event, attempt to flush a retained outbox. If it still cannot be appended, keep Fixed Weighted/shadow-rate computation available but **pause personal outcome maturation and creation of new pending intervals** for that refresh. Never overwrite an unflushed audit event. Once the outbox drains, normal learning may resume; an old pending interval is then subject to the ordinary late/gap rejection rules.

On startup, flush any retained outbox before processing new feedback. Duplicate appends are acceptable; every event carries `(accountGeneration,eventSequence)`, and offline analysis must de-duplicate by that key. Lost events are not acceptable when the corresponding learner state update survived.

`accountGeneration` increments on confirmed account switch and is safe to log; `eventSequence` may reset only after that generation increment. Algorithm-version resets within the same account must preserve the audit sequence. Never log the raw account ID.

## 6. v1 → v2 migration

The current loader rejects any schema version other than v1. Replace that all-or-nothing behavior with an explicit migration path.

Required behavior:

1. v2: deserialize current state and normalize the shadow subtree.
2. v1: deserialize the existing `accountId`, `fiveHour`, and `weekly` fields unchanged; initialize `fiveHourShadow` to defaults; return v2 in memory.
3. unknown future schema: fall back safely without pretending compatibility.
4. malformed new shadow fields: retain valid legacy quota history whenever the v1/v2 core fields can still be recovered.

A v1 migration must not erase the user's existing crossing history merely because personalization is new.

The next successful state-changing refresh may persist the migrated v2 representation.
## 7. Refresh ordering and causality

Integrate the shadow path inside `BurnRateTracker::accept_usage()` so the tracker remains the single owner of usage-history state.

For every successful raw usage sample, use this order:

1. apply existing account transition handling;
2. snapshot the pre-accept 5H quota state needed for weighted metadata transitions;
3. run existing `accept_quota()` exactly once for 5H and retain both its production estimate and transition classification;
4. update/normalize Fixed Weighted metadata from the before/after state;
5. if this observation caused a confirmed segment/reset/correction transition, invalidate any pending interval before it can train;
6. otherwise process an existing pending personal forecast against the current observation using only the origin snapshot plus the current accepted observation;
7. if that outcome matures, update `A/B/E`, `maturedFeedbacks`, and when `q>0`, `informativeUpdates`;
8. compute the current Fixed Weighted 5H rate;
9. calculate current `c`, `g`, effective factor, and shadow rate from the post-maturation learner state;
10. after maturation/rejection, start a new pending forecast at this same observation when eligible, storing the complete causal origin snapshot including production, Fixed Weighted, and personalized rates;
11. process Weekly using its existing path unchanged;
12. return the unchanged production `BurnRateEstimate` plus internal shadow diagnostics for persistence/logging.

This ordering matches the offline causal replay: outcomes with `target_at <= current_at` may train the prediction made at the current time, but never an earlier prediction. A transition detected from the current observation can reject old pending evidence, but that observation may then become the origin of a fresh interval under the new segment.
## 8. Five-minute feedback lifecycle

A pending interval begins only when all required origin data exist:

- current 5H sample is valid and below saturation;
- Fixed Weighted rate is finite and positive;
- origin timestamp is valid;
- reset metadata is usable under the existing snapshot contract.

Store the rate at the origin. Do not continuously revise the pending prediction as later rates arrive.

At target time, compute:

```text
elapsed = target_at - origin_at
p = origin_fixed_rate * elapsed / 3600
p = min(p, 100 - origin_used_percent)
y = target_used_percent - origin_used_percent
```

The actual elapsed duration is intentionally used, matching the real-history forecast benchmark.
### 8.1 Maturation window

Target horizon is 300 seconds.

Use the first eligible valid observation reaching the target. It may mature only when:

```text
300 <= elapsed <= 390 seconds
```

The extra 90 seconds matches the frozen real-history protocol.

Once a pending interval matures or is rejected, it is finished. A new interval may start at that same valid observation.

This produces non-overlapping personal-learning outcomes by construction.
### 8.2 Rejection rules

Reject the pending outcome without learning if any of the following occurs before maturation:

- a valid-observation gap exceeds 90 seconds;
- reset timestamp differs from the origin reset timestamp;
- used percentage decreases;
- used percentage reaches saturation at 100%;
- target arrives too late;
- timestamps move backwards;
- required data become non-finite or invalid;
- account context changes.

Missing/invalid samples do not fabricate progress. They also must not silently bridge a gap; the next valid observation is compared with `lastValidAt`.

Same-second duplicate refreshes should be ignored for pending progression rather than counted as new evidence.

Record a rejection reason in local shadow evidence, but do not update `A`, `B`, `E`, `maturedFeedbacks`, or `informativeUpdates`.

A raw `resetsAt` mismatch is intentionally conservative for **that pending five-minute outcome** because the frozen real-history protocol rejected such paths. It does not by itself mean an actual quota-cycle transition occurred. Preserve the existing production distinction between a future reset-boundary correction and a completed cycle: a mere metadata shift may reject the pending training interval, but must not clear long-lived `A/B/E`.

### 8.3 Polling-cadence compatibility

The frozen feedback protocol has a stricter continuity rule than the production burn-history tracker:

```text
personal feedback max valid-observation gap = 90 s
5H burn-history max interpolation gap       = 10 min
```

The application can be configured for 30/60/120/300-second polling in the UI and accepts a wider 15–3600-second backend range. Therefore a 90-second feedback-gap rule cannot be silently treated as universally compatible.

For this shadow implementation:

- preserve the validated 90-second rule exactly;
- 30-second and normal 60-second polling can accumulate personal feedback when fetch latency/jitter keeps consecutive accepted observations within 90 seconds;
- 120/300-second polling, or any actual accepted-observation gap above 90 seconds, must reject the pending interval without learning;
- do not stretch the 90-second rule based on configured cadence in this implementation, because that would create a new algorithm that has not been replay-validated;
- the existing published Burn Rate and Fixed Weighted shadow estimator continue operating under their own gap semantics; only personal five-minute learning becomes unavailable;
- evidence logging must make this visible through rejection reason and actual gap duration, so later analysis can quantify how much production coverage is lost.

Before any future public personalization is expected to support polling above 90 seconds, run a separate frozen cadence-aware validation. That experiment may test a rule such as `configured cadence + bounded fetch/jitter allowance`, but it is not part of the present frozen algorithm.

Required cadence tests:

- 30-second stable polling: pending outcomes mature normally;
- 60-second polling with bounded jitter under 90 seconds: matures normally;
- any gap just above 90 seconds: rejects exactly once and does not train;
- 120-second and 300-second regular polling: does not fabricate learning or corrupt state;
- returning from a slow/gapped cadence to <=90-second accepted gaps can start fresh pending intervals without resetting long-lived personal state.

## 9. Reset, gap, account, and restart semantics

These lifecycles must be intentionally different.

### Account switch

A confirmed change to a different non-empty account ID:

- increments `accountGeneration` before any new-account evidence can be recorded;
- resets per-generation `eventSequence` to zero;
- clears existing 5H/Weekly histories according to current behavior;
- clears weighted shadow metadata and pending personal forecast;
- resets personal `A/B/E`, `maturedFeedbacks`, and `informativeUpdates`.

No personal calibration or audit identity may cross accounts. The raw account ID remains available only to the existing backend isolation logic and must never be copied into the evidence log.

### Quota reset / new 5H segment

A confirmed quota-cycle transition clears:

- weighted idle metadata;
- pending forecast.

It **does not** clear personal `A/B/E`, `maturedFeedbacks`, or `informativeUpdates`. User behavior is allowed to persist across normal 5H quota windows. `accountGeneration` also remains unchanged because a quota reset is not an account change.
### Long gap

A gap beyond the existing 5H history max gap starts a new quota segment as today and clears weighted segment metadata/pending.

A feedback gap above 90 seconds rejects a pending five-minute outcome even when it is shorter than the 10-minute burn-history max gap.

Do not apply wall-clock decay to `A/B/E` merely because the app was idle. The validated decay is per matured feedback, not elapsed time.

### Restart

Persist `A/B/E`, weighted metadata, and pending state.

On restart:

- valid state resumes without a new personal warm-up;
- a pending interval may continue only if the subsequent valid observation still satisfies all timing/continuity rules;
- otherwise reject it safely and start fresh;
- corrupt shadow state must never prevent the application from loading the legacy estimator state.
## 10. Local shadow evidence log

Create a local-only validation log separate from the persisted learner state.

Recommended path: a runtime file such as `burn-rate-shadow-evidence.jsonl` exposed through a dedicated `ConfigManager` path helper.

This is diagnostic evidence, **not network telemetry**. Do not upload or transmit it.

Record one compact event for:

- matured feedback;
- rejected pending feedback;
- account/reset lifecycle clear when diagnostically useful.

Do not log conversation text, source filenames, authentication material, credits, or raw account identifiers.
### 10.1 Matured record fields

A matured outcome should contain enough information to replay the learner:

```text
schemaVersion
algorithmVersion
accountGeneration / eventSequence
originAt / targetAt / elapsedSeconds
originUsed / targetUsed
originPublishedRate / originFixedRate / originShadowRate
publishedPredictedDelta / fixedPredictedDelta / shadowPredictedDelta
observedDelta
originRawFactor / originTrust / originEffectiveFactor
originEvidence / originInformativeUpdates
q
aBefore / bBefore / evidenceBefore
aAfter / bAfter / evidenceAfter
maturedFeedbacksAfter / informativeUpdatesAfter
targetRawFactor / targetTrust / targetEffectiveFactor / targetShadowRate
```

`accountGeneration` and `eventSequence` are mandatory audit keys; the raw account ID is never logged. Rejected events additionally record a stable rejection code plus relevant timing metadata such as `actualGapSeconds` when applicable. Lifecycle events record only the minimum state transition needed to interpret later evidence.

Offline readers must de-duplicate at-least-once log delivery by `(accountGeneration,eventSequence)` before computing metrics. This log is evidence for future validation; it is not part of the UI contract.
### 10.2 Bounded retention

The evidence log must not grow indefinitely.

Use a deterministic local retention rule sufficient for long-run validation, for example:

- retain at most 10,000 shadow events;
- compact on startup and/or after crossing a size threshold;
- preserve the newest events;
- log/compaction failure must not fail usage refresh.

The exact file-maintenance helper may vary, but unbounded append-only growth is not acceptable.

Do not store every 30-second raw poll. Store only mature/reject/lifecycle evidence needed to audit learning.
## 11. Internal result type

Do not add shadow fields to the frontend-facing `CodexUsage` yet.

Introduce an internal diagnostic result, for example:

```text
FiveHourShadowEstimate
  fixedWeightedPerHour: Option<f64>
  personalizedPerHour: Option<f64>
  rawFactor: Option<f64>
  trust: f64
  effectiveFactor: f64
  evidence: f64
  maturedFeedbacks: u64
  informativeUpdates: u64
  outcomeEvent: Option<...>
```

`BurnRateTracker::accept_usage()` may return this alongside the current production estimate through an internal aggregate type, or expose it through an internal accessor.

The design must make it mechanically difficult to accidentally substitute the shadow value into `RawCodexUsage::to_ui()`.
## 12. Save behavior

Shadow state changes count as burn-rate state changes and should use the existing atomic temp-file + rename persistence path.

Examples that require persistence:

- weighted idle metadata changes;
- pending forecast starts/updates/rejects/matures;
- `A/B/E` update;
- account/reset clearing;
- schema migration once another state-changing event causes save.

A state-save failure should retain the updated in-memory shadow state for the running process and log the persistence error exactly as current burn-rate save failures do.

For audit consistency, do not append a newly generated outbox event to the durable JSONL file until the corresponding learner/outbox state save has succeeded. If that save fails, keep the event and learner state dirty in memory, retry persistence on subsequent refreshes, and pause additional personal outcome transitions until that state/outbox pair is durably saved. If state persistence succeeds but JSONL append fails, leave the persisted outbox populated so startup or a later refresh can retry it. This yields at-least-once evidence logging without allowing a durable log event to claim a learner update that was never durably saved or allowing multiple unpersisted learning events to accumulate behind one audit slot.

Do not make a shadow persistence or diagnostic-log failure turn an otherwise successful usage refresh into an application error.
## 13. Required unit tests — weighted core

Promoting replay code to production-safe helpers must preserve prior evidence.

Required tests include:

- exact linear weighted slope invariance;
- frozen 300s/1800s 5H parameters;
- insufficient span/evidence returns unavailable;
- invalid parameters rejected;
- idle ceiling only decreases while idle;
- partial horizon expiry never raises the ceiling;
- a new crossing clears the idle ceiling;
- segment transition clears weighted metadata;
- metadata normalization after JSON round-trip;
- malformed optional metadata is repaired/cleared;
- weighted helper reproduces existing replay candidate outputs within the existing numerical tolerance.

After refactor, the replay suite must call this shared weighted implementation rather than a copy.
## 14. Required unit tests — personal learner

Test the frozen formulas directly:

- defaults are `A=B=10`, `E=0`, factor/trust neutral;
- one known `p/y` update produces the expected `A/B`;
- `q=0` decays `A/B/E`, increments `maturedFeedbacks`, and does **not** increment `informativeUpdates`;
- `q>0` increments both counters exactly once;
- raw factor clamps to `[0.75,1.5]`;
- `E_t=0.99E+q`;
- trust is always finite and in `[0,1]`;
- effective factor always lies between 1 and the clamped raw factor;
- continuous `q=1` evidence approaches `E≈100` from below while `informativeUpdates` continues past 100;
- after 100/200/300 updates an old contribution has the expected exponential decay;
- corrupt/non-finite learner state normalizes to neutral defaults;
- algorithm-version mismatch resets only the incompatible shadow subtree unless an explicit migration exists;
- long deterministic sequences do not produce NaN/Inf or numerical underflow failure.
## 15. Required unit tests — pending outcomes

Cover causal lifecycle precisely:

- no pending forecast when Fixed Weighted is unavailable;
- first eligible origin creates exactly one pending interval;
- refreshes before 300s cannot train;
- first valid target in 300–390s matures;
- actual target elapsed time is used in `p`;
- published, Fixed Weighted, and personalized predictions are all captured from the **origin** snapshot and capped independently by quota remaining at origin;
- maturation scores the stored origin shadow prediction; target-time factor changes cannot rewrite the origin prediction;
- the learner's `p` remains the stored origin Fixed Weighted prediction, never the personalized prediction;
- matured update occurs before calculating the same-time shadow estimate;
- a mature target can immediately become the next origin using the post-update factor;
- intervals never overlap;
- >90s valid-observation gap rejects;
- 30s and bounded-jitter 60s cadence can mature; 120s/300s cadence cannot silently train under the frozen rule;
- reset timestamp change rejects the interval without automatically clearing long-lived personal state;
- confirmed segment/correction transition rejects before training;
- decrease/correction rejects;
- saturation rejects;
- too-late target rejects;
- missing samples never fabricate outcomes;
- same-second duplicate does not create evidence;
- stale/backward timestamps cannot train;
- future observations cannot change an earlier shadow prediction.
## 16. Required lifecycle/integration tests

Add integration coverage around the real `AppState` refresh path:

- existing visible `CodexUsage` values remain byte/field equivalent with shadow enabled;
- successful refresh persists shadow state when it changes;
- failed usage read does not fabricate feedback or erase learner state;
- account switch increments `accountGeneration` and clears personal state/counters before any new-account learning;
- quota reset preserves personal `A/B/E` and lifetime counters but clears pending/weighted segment metadata;
- a future `resetsAt` metadata correction can reject pending feedback without being misclassified as an account/quota reset;
- v1 state migrates to v2 without losing existing 5H/Weekly crossing history;
- v2 state round-trips;
- malformed shadow subtree does not destroy valid legacy state;
- algorithm-version mismatch preserves legacy quota history and audit sequence while isolating incompatible algorithm state;
- short restart can resume a valid pending interval with the exact origin estimator snapshot intact;
- long restart gap rejects pending but preserves `A/B/E`;
- persisted audit outbox is retried after restart;
- duplicate outbox append is harmless after de-duplication by `(accountGeneration,eventSequence)`;
- an unflushed outbox is never overwritten by a second personal outcome;
- state-save failure does not append an unpersisted learner event to durable evidence and pauses further personal transitions until persistence recovers;
- evidence-log append failure leaves the persisted outbox retryable and pauses new personal outcomes while leaving visible production behavior unaffected;
- save/log failure leaves UI refresh behavior unchanged;
- tray/settings presentation tests remain unchanged.

Frontend tests should not require modification unless an accidental public contract change occurs; such a change is a failure for this phase.
## 17. Replay regression gates

Before merging implementation, rerun the existing offline evidence scripts and add a production-helper parity replay.

Minimum gates:

1. Fixed Weighted real-history aggregate remains consistent with the committed `98f9bb4` evidence within numerical tolerance.
2. Personal calibration replay remains consistent with `8cbf153`.
3. Evidence-weighted replay remains consistent with `589fbb4`.
4. Evidence-decay replay remains consistent with `662054d`.
5. No replay obtains access to future observations through the production refactor.

If moving shared math changes historical scores materially, stop and explain the cause rather than updating golden evidence silently.
## 18. Shadow deployment acceptance

This implementation may be merged/deployed as shadow-only when:

- Rust unit/integration tests pass;
- existing frontend tests/build pass;
- all frozen replay regressions pass;
- state v1→v2 migration and algorithm-version isolation are covered;
- causal origin snapshots prove that published/fixed/personalized predictions can be scored later without target-time reconstruction;
- 30/60/120/300-second cadence behavior is explicit and tested under the frozen 90-second feedback rule;
- local evidence log is bounded, de-duplicable, crash-recoverable through the audit outbox, and contains no account ID/conversation data;
- published UI values are demonstrably unchanged;
- a packaged/native smoke run confirms learner, pending-origin snapshot, account generation, and audit outbox behavior across restart.

Do not wait for 100 real feedbacks before merging shadow collection; collecting those feedbacks is the purpose of this phase.

## 19. Later publication gate — not part of implementation

A future publication decision must be based on the causal origin snapshots collected by this implementation, not on target-time recomputation.

### 19.1 Primary matched comparison

For every matured interval with all three origin predictions available, score the same observed `y` against:

```text
current published estimator
Fixed Weighted shadow base
personalized shadow
```

Report at minimum:

- MAE;
- p90 absolute error;
- signed bias;
- wins / losses / ties on matched windows;
- availability and rejection rate;
- equal-day and, where a stable session grouping exists, equal-session aggregates.

Do not let one estimator gain an apparent advantage by being scored on a different outcome set. Unavailable predictions remain unavailable rather than becoming zero.

The personalized candidate should not be considered for publication unless it improves matched average error over its Fixed Weighted base **without material p90 degradation** and does not regress materially against the current published estimator. The exact statistical acceptance threshold must be pre-registered in the future publication plan before reading the final confirmation slice; do not choose a threshold post hoc from the collected scores.

### 19.2 Learning-age reporting

Bucket forecasts by `originInformativeUpdates`, not by decayed `E`:

```text
0
1–4
5–9
10–19
20–49
50–99
100+
```

Also report `originEvidence` and trust distributions separately. This preserves the distinction between lifetime learning age and the approximately-100-feedback soft memory.

The key missing evidence from the current research chain is the 10+, 20+, 50+, and 100+ region. Collection should continue until those regions are genuinely represented; do not infer long-run safety from the current <=9-update replay.

### 19.3 Dependence and lifecycle review

Because adjacent five-minute windows from one user/day are not statistically independent, future confidence analysis should use day/session blocking or an equivalent grouped resampling method rather than treating every interval as iid evidence.

The later decision must separately review:

- account transitions and `accountGeneration` separation;
- normal 5H resets and reset-metadata corrections;
- overnight/long idle;
- application restarts and outbox recovery;
- polling-cadence coverage, especially the share rejected by the frozen 90-second rule;
- behavior regime changes;
- evidence-log completeness after de-duplication;
- rejection reasons and rejection rate;
- whether time-based decay is needed in addition to feedback-count decay.

Weekly requires its own new validation before any personalization.

## 20. Implementation sequence

Execute in this order:

1. Freeze the revised plan and record the exact implementation baseline before touching production code.
2. Refactor/promote Fixed Weighted shared math; prove parity with the committed `98f9bb4` real-history evidence before continuing.
3. Add v2 state structs, explicit v1 migration, `algorithmVersion`, `accountGeneration`, counters, and audit-outbox fields; test migration/version isolation before changing refresh logic.
4. Add personal learner math and pure unit tests, including separate matured/informative counter semantics.
5. Add the pending five-minute engine with complete origin snapshots for published, Fixed Weighted, and personalized predictions.
6. Add cadence-boundary tests and keep the validated 90-second personal-feedback rule unchanged; document expected 120/300-second learning unavailability rather than silently broadening it.
7. Wire transition-aware shadow processing into `BurnRateTracker::accept_usage()`, ensuring reset/correction/account transitions reject stale pending evidence before training.
8. Add atomic persistence and at-least-once audit-outbox delivery through the existing save path.
9. Add bounded JSONL evidence logging and a small offline validator that de-duplicates events and verifies the three origin predictions can be causally scored.
10. Run lifecycle/integration tests proving visible `CodexUsage`, tray, settings, notifications, and polling behavior are unchanged.
11. Rerun every frozen replay/evidence regression using the promoted production helpers; any material score drift is a stop condition, not a new golden baseline.
12. Run packaged/native smoke validation covering restart, pending resume/reject, account switch, quota reset, log retry, and 30/60/120/300-second cadence behavior.
13. Write an implementation handoff with exact test counts, migration/version results, one de-identified matured-event example, one rejection example, outbox recovery result, and any deviations.
14. Commit and push the implementation only after all gates above pass.

No version bump or public release is required merely to complete the shadow implementation.
## 21. Non-goals

Do not add any of the following during this implementation:

- adaptive rate-change detector;
- fractional crossing interpolation;
- automatic half-life tuning;
- multi-estimator competition;
- Weekly personal calibration;
- hard recent-100 feedback queue;
- elapsed-time trust decay;
- behavior-change detector;
- user-facing personalization setting;
- personalized value in UI/tray;
- remote analytics upload.

Those are separate research or product decisions.

## 22. Handoff contract

A new implementer should be able to execute this document without reopening algorithm selection.

The relevant research chain is:

```text
2026-10-03-v1.3.1-display-controls-plan.md §8.1 weighted refinement
→ 2026-10-03-burn-rate-adaptive-validation.md
→ 2026-10-03-burn-rate-real-usage-validation.md
→ 2026-10-03-personal-calibration-validation.md
→ 2026-10-03-evidence-weighted-personalization-validation.md
→ 2026-10-03-personal-evidence-decay-validation.md
→ this implementation plan
```

If implementation constraints require changing a frozen mathematical constant or causal rule, stop at the plan/review stage and document the proposed deviation before coding it.
