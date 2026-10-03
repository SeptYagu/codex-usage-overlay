# 5H Persistent Shadow Personalization — Implementation Plan

Date: 2026-10-03
Baseline: `662054d` (`test: validate personal evidence decay`)
Scope: production-quality shadow implementation and local validation data collection; **no user-visible estimator switch**.

## 1. Goal

Implement the validated 5-hour personalization algorithm in the real application so it can accumulate persistent, account-safe, causal feedback over normal use.

The implementation must produce a shadow personalized 5H rate and validation evidence while leaving the current published 5H and Weekly Burn Rate behavior unchanged.

The immediate product objective is not “ship the new number.” It is to collect trustworthy 10+, 20+, 50+, and 100+ feedback histories from the actual overlay lifecycle so a later decision can determine whether the shadow value is safe to publish.

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
  a: f64                 default 10
  b: f64                 default 10
  evidence: f64          default 0
  realizedUpdates: u64   default 0
  lastUpdateAt: Option<i64>
```

`realizedUpdates` is diagnostic lifetime count only; it does not determine trust.

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
  originAt
  originUsedPercent
  originResetAt
  originFixedRatePerHour
  lastValidAt
```

One pending forecast enforces the same non-overlapping outcome structure used in the real-history validation.

Do not keep a queue of overlapping five-minute predictions.

Persistence allows a very short application restart to continue an interval causally. A restart gap that violates the feedback gap rule invalidates the pending interval on the next valid sample.
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
3. run existing `accept_quota()` exactly once for 5H;
4. update/normalize Fixed Weighted metadata from the before/after state;
5. compute the current Fixed Weighted 5H rate;
6. process an existing pending personal forecast against the current observation;
7. if that outcome matures, update `A/B/E` before calculating the current shadow factor;
8. calculate current `c`, `g`, effective factor, and shadow rate;
9. after maturation/rejection, start a new pending forecast at this same observation when eligible;
10. process Weekly using its existing path unchanged;
11. return the unchanged production `BurnRateEstimate` plus internal shadow diagnostics for persistence/logging.

This ordering matches the offline causal replay: outcomes with `target_at <= current_at` may train the prediction made at the current time, but never an earlier prediction.
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

Record a rejection reason in local shadow evidence, but do not update `A`, `B`, `E`, or `realizedUpdates`.
## 9. Reset, gap, account, and restart semantics

These lifecycles must be intentionally different.

### Account switch

A confirmed change to a different non-empty account ID clears:

- existing 5H/Weekly histories according to current behavior;
- weighted shadow metadata;
- pending personal forecast;
- personal `A/B/E` state and diagnostic update count.

No personal calibration may cross accounts.

### Quota reset / new 5H segment

A confirmed quota-cycle transition clears:

- weighted idle metadata;
- pending forecast.

It **does not** clear personal `A/B/E`. User behavior is allowed to persist across normal 5H quota windows.
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
originAt / targetAt / elapsedSeconds
originUsed / targetUsed
originFixedRate
predictedDelta / observedDelta
q
aBefore / bBefore / evidenceBefore
aAfter / bAfter / evidenceAfter
rawFactor
trust
effectiveFactor
shadowRate
realizedUpdates
```

An optional monotonically increasing local account-generation number may be logged for separation, but never the account ID itself.

This log is evidence for future validation; it is not part of the UI contract.
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
  realizedUpdates: u64
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

Do not make a shadow persistence failure turn an otherwise successful usage refresh into an application error.
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
- `q=0` decays `A/B/E` without adding evidence;
- raw factor clamps to `[0.75,1.5]`;
- `E_t=0.99E+q`;
- trust is always finite and in `[0,1]`;
- effective factor always lies between 1 and the clamped raw factor;
- continuous `q=1` evidence approaches `E≈100`;
- after 100/200/300 updates an old contribution has the expected exponential decay;
- corrupt/non-finite learner state normalizes to neutral defaults;
- long deterministic sequences do not produce NaN/Inf or numerical underflow failure.
## 15. Required unit tests — pending outcomes

Cover causal lifecycle precisely:

- no pending forecast when Fixed Weighted is unavailable;
- first eligible origin creates exactly one pending interval;
- refreshes before 300s cannot train;
- first valid target in 300–390s matures;
- actual target elapsed time is used in `p`;
- prediction is capped by quota remaining at origin;
- matured update occurs before calculating the same-time shadow estimate;
- a mature target can immediately become the next origin;
- intervals never overlap;
- >90s valid-observation gap rejects;
- reset timestamp change rejects;
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
- account switch clears personal state before any new-account learning;
- quota reset preserves personal `A/B/E` but clears pending/weighted segment metadata;
- v1 state migrates to v2 without losing existing 5H/Weekly crossing history;
- v2 state round-trips;
- malformed shadow subtree does not destroy valid legacy state;
- short restart can resume a valid pending interval;
- long restart gap rejects pending but preserves `A/B/E`;
- save failure leaves UI refresh behavior unchanged;
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
- state v1→v2 migration is covered;
- local evidence log is bounded and contains no account ID/conversation data;
- published UI values are demonstrably unchanged;
- a packaged/native smoke run confirms state persists across restart.

Do not wait for 100 real feedbacks before merging shadow collection; collecting those feedbacks is the purpose of this phase.

## 19. Later publication gate — not part of implementation

A future plan may consider exposing the personalized 5H rate only after real shadow evidence contains meaningful samples in at least:

```text
1–4
5–9
10–19
20–49
50–99
100+
```

and confirms that MAE improves without material p90 regression as trust grows.
The later decision must also review:

- account transitions;
- normal 5H resets;
- overnight/long idle;
- application restarts;
- behavior regime changes;
- evidence-log coverage and rejection rate;
- whether time-based decay is needed in addition to feedback-count decay.

Weekly requires its own new validation before any personalization.

## 20. Implementation sequence

Execute in this order:

1. Refactor/promote Fixed Weighted shared math; prove replay parity.
2. Add v2 state structs and v1 migration; test migration before changing refresh logic.
3. Add personal learner math and pure unit tests.
4. Add pending five-minute outcome engine and causal tests.
5. Wire shadow processing into `BurnRateTracker::accept_usage()`.
6. Add atomic persistence of the new state through the existing save path.
7. Add bounded local shadow evidence logging.
8. Run lifecycle/integration tests proving UI output is unchanged.
9. Rerun every frozen replay/evidence regression.
10. Run packaged/native restart smoke validation.
11. Write an implementation handoff with exact test counts, state migration result, evidence-log sample schema, and any deviations.
12. Commit and push the implementation only after the above gates pass.

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
2026-10-03-burn-rate-real-usage-validation.md
→ 2026-10-03-personal-calibration-validation.md
→ 2026-10-03-evidence-weighted-personalization-validation.md
→ 2026-10-03-personal-evidence-decay-validation.md
→ this implementation plan
```

If implementation constraints require changing a frozen mathematical constant or causal rule, stop at the plan/review stage and document the proposed deviation before coding it.
