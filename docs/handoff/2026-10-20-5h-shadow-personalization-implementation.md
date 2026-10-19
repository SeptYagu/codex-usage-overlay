# 5H Shadow Personalization — Implementation Handoff

- Date: 2026-10-20
- Plan: [2026-10-03-5h-persistent-shadow-personalization-implementation-plan.md](2026-10-03-5h-persistent-shadow-personalization-implementation-plan.md)
- Base commit: `6137faa` (v1.3.1 release record)
- Scope: shadow only. No user-visible behavior changed.

## 1. What was built

The 5-hour burn-rate estimator now carries a second, invisible personalization
stack that collects causal feedback for a later publication decision. Nothing
in the overlay, the pill, the tray, the notifications, the settings, the
polling cadence or the frontend DTO changed.

```text
accepted 5H quota history
        ↓
Fixed Weighted 5H estimator      (production module, shared with the replay suite)
        ↓
5-minute causal outcome learner   (A/B/E + decayed trust)
        ↓
raw personal factor c → trust g → effective factor
        ↓
shadow personalized 5H rate      (diagnostics + local evidence log only)
```

### New production modules

| File | Lines | Owner |
| --- | --- | --- |
| `src-tauri/src/codex/burn_rate/weighted.rs` | 486 | the weighted fit and the idle ceiling |
| `src-tauri/src/codex/burn_rate/personal.rs` | 662 | the learner, the trust and the pending interval |
| `src-tauri/src/codex/burn_rate/shadow.rs` | 465 | the shadow state tree, the audit outbox, the evidence log |
| `src-tauri/src/codex/burn_rate/shadow_tests.rs` | 919 | the lifecycle and integration tests |

`weighted.rs` was promoted from the offline replay candidate. The replay
suite now imports it, so the committed evidence keeps testing the code the
application runs. The `Candidate` wrapper in `replay.rs` delegates its
normalize / observe / publish steps to `WeightedShadowState`; the previous
second mathematical implementation was deleted.

## 2. Frozen algorithm contract (unchanged)

```text
Fixed Weighted 5H: half-life 300 s, local window 1800 s, hard horizon 1800 s, span 2
Personal learner:   A0 = B0 = 10, lambda = 0.99, c = clamp(A/B, 0.75, 1.50)
Feedback quality:   q = 0 when observed_delta == 0 and predicted_delta < 0.75, else 1
Trust memory:       E0 = 0, E_t = 0.99 E + q, g = E/(E+8), factor = 1 + g(c-1)
Feedback window:    300 s target horizon, 390 s latest arrival, 90 s max valid gap
```

No constant was changed. No new dependency, no new user-visible switch, no
Weekly personalization.

## 3. Refresh ordering and causality

`BurnRateTracker::accept_usage()` runs the shadow pass in the validated
causal order:

1. the weighted metadata is normalized against the production history;
2. the account transition is applied;
3. the production 5H quota state is accepted exactly once;
4. the Fixed Weighted metadata transition is applied from the before/after state;
5. a confirmed segment transition rejects the pending interval before it can train;
6. otherwise the pending interval is observed against the current sample;
7. a matured outcome updates `A/B/E`, `maturedFeedbacks` and, when `q > 0`,
   `informativeUpdates`;
8. the current Fixed Weighted rate is published;
9. `c`, `g`, the effective factor and the shadow rate are computed from the
   post-maturation learner state;
10. a new pending interval starts at the same observation with the complete
    causal origin snapshot;
11. Weekly is processed by its unchanged path.

The origin snapshot stores the published rate, the Fixed Weighted rate and the
personalized rate **as they were at the origin**. The matured outcome scores
those stored values, so a target-time factor change can never rewrite an
earlier prediction.

## 4. State schema and migration

The burn-rate state schema moved from v1 to v2. The v2 file adds the
`fiveHourShadow` subtree:

```text
fiveHourShadow
  algorithmVersion   u32
  accountGeneration  u64
  eventSequence      u64
  auditOutbox        Option<ShadowEvidenceEvent>
  weighted           { lastCrossingObservationAt, idleRateCeiling }
  personal           { a, b, evidence, maturedFeedbacks, informativeUpdates, lastUpdateAt }
  pending            Option<PendingPersonalForecast>
```

Migration and isolation results:

| Case | Result |
| --- | --- |
| v1 file | quota history preserved unchanged, shadow subtree starts neutral, in-memory version is v2 |
| v2 file | deserialized, shadow subtree normalized against the quota history |
| v2 file with a malformed shadow subtree | the recoverable core (5H + Weekly history) is kept, the shadow is reset |
| unknown future schema | the whole state falls back to the default, without pretending compatibility |
| algorithm-version mismatch | the legacy history, `accountGeneration` and `eventSequence` survive; the personal, weighted and pending state return to neutral |

A v1 migration never erases the existing crossing history.

## 5. Audit outbox and the local evidence log

The evidence log lives next to the burn-rate state as
`burn-rate-shadow-evidence.jsonl` in the runtime directory. It is local
diagnostic evidence, never uploaded.

Delivery rules, as implemented:

1. the audit slot must be empty before a new event is placed into it;
2. `eventSequence` is incremented and the fully formed event is placed in the slot;
3. the burn-rate state is persisted atomically (temp file + rename);
4. only then is the event appended to the JSONL log;
5. the slot is cleared in memory and the persisted copy is replaced with the
   next state-changing save.

While the slot is occupied, the Fixed Weighted estimator keeps running but the
personal outcome maturation and the creation of new pending intervals are
paused. A retained event is never overwritten, and an event whose learner
state is not durable is never appended.

The log keeps at most 10,000 events and compacts to the newest ones. A
compaction or append failure never fails a usage refresh.

### De-identified matured event

```json
{
  "schemaVersion": 1, "algorithmVersion": 1, "kind": "matured",
  "accountGeneration": 0, "eventSequence": 1, "at": 1480,
  "originAt": 1180, "targetAt": 1480, "elapsedSeconds": 300,
  "originUsed": 13.0, "targetUsed": 18.0,
  "originPublishedRate": 48.0, "originFixedRate": 60.0, "originShadowRate": 60.0,
  "publishedPredictedDelta": 4.0, "fixedPredictedDelta": 5.0, "shadowPredictedDelta": 5.0,
  "observedDelta": 5.0,
  "originRawFactor": 1.0, "originTrust": 0.0, "originEffectiveFactor": 1.0,
  "originEvidence": 0.0, "originInformativeUpdates": 0, "q": 1.0,
  "aBefore": 10.0, "bBefore": 10.0, "evidenceBefore": 0.0,
  "aAfter": 34.9, "bAfter": 34.9, "evidenceAfter": 1.0,
  "maturedFeedbacksAfter": 1, "informativeUpdatesAfter": 1,
  "targetRawFactor": 1.0, "targetTrust": 0.1111111111111111,
  "targetEffectiveFactor": 1.0, "targetShadowRate": 60.0
}
```

The three origin predictions are visibly different: the published production
rate (48.0), the Fixed Weighted rate (60.0) and the personalized rate (60.0,
still neutral because the trust was zero at the origin). The learner trained
on the Fixed Weighted prediction only.

### De-identified rejection event

```json
{
  "schemaVersion": 1, "algorithmVersion": 1, "kind": "rejected",
  "accountGeneration": 0, "eventSequence": 2, "at": 1900,
  "originAt": 1480, "targetAt": 1900,
  "originUsed": 18.0, "targetUsed": 30.0,
  "originPublishedRate": 51.42857142857143, "originFixedRate": 60.0, "originShadowRate": 60.0,
  "originRawFactor": 1.0, "originTrust": 0.1111111111111111, "originEffectiveFactor": 1.0,
  "originEvidence": 1.0, "originInformativeUpdates": 1,
  "rejectionReason": "gap", "actualGapSeconds": 420
}
```

### Outbox recovery result

- A matured event that was persisted but not appended is flushed by the next
  startup, before any new feedback is processed.
- The persisted slot still holds the event after a successful append, so a
  later start can append the same event again. The duplicate is harmless: every
  record carries `(accountGeneration, eventSequence)` and the offline analysis
  de-duplicates on that key.
- A save failure keeps the event in the slot, keeps the state dirty and pauses
  the personal transitions until the persistence recovers. The event is not
  appended to the durable log in that state.
- An append failure keeps the slot populated, so the next refresh retries it.
  The visible production behavior is unaffected.

## 6. Cadence behavior under the frozen 90-second rule

| Polling | Behavior |
| --- | --- |
| 30 s | pending intervals mature normally |
| 60 s (bounded jitter under 90 s) | pending intervals mature normally |
| any accepted gap over 90 s | the interval is rejected exactly once with the code `gap`, and the learner is not trained |
| 120 s / 300 s regular polling | no learning is fabricated and the learner state stays intact |
| returning to a fast cadence | fresh intervals start and train again; the long-lived `A/B/E` is untouched |

The 90-second rule was not stretched. The evidence log records the rejection
code and the actual gap, so a later analysis can quantify how much production
coverage is lost at the configured cadence.

## 7. Test counts and gates

| Gate | Result |
| --- | --- |
| `cargo check --all-targets` | clean, zero warnings |
| `cargo test` | **276** regular tests passed, 0 failed, 4 pre-existing ignored replay tests unchanged |
| `npm test` | **95** frontend tests passed, unchanged |
| `npm run build` | passed |
| `scripts/Test-TauriRelease.ps1` | 6/6 passed |
| `scripts/check-replay-evidence.mjs` (new) | 13/13 evidence files byte-identical with the committed evidence |

New Rust tests: 11 weighted, 25 personal, 39 shadow lifecycle.

### Replay regression gates (plan §17)

| Gate | Result |
| --- | --- |
| 1. Fixed Weighted real history vs. the committed `98f9bb4` evidence | byte-identical (`forecast.csv`, `scenarios.csv`) |
| 2. Personal calibration vs. `8cbf153` | byte-identical (3 files) |
| 3. Evidence weighted vs. `589fbb4` | byte-identical (5 files) |
| 4. Evidence decay vs. `662054d` | byte-identical (3 files) |
| 5. No future observation leaks into a prediction | the regenerated `real-forecasts.json` matches the pre-refactor baseline in all 318 rows, 0 null mismatches, **maximum prediction deviation 0** |

The regeneration uses the production helper: `cargo test --lib -- --ignored
real_usage_forecast_replay` writes `output/burn-rate-replay/real-forecasts.json`
through `WeightedShadowState`, and the four offline scripts then produce the
evidence files. `scripts/check-replay-evidence.mjs` compares the result with
the committed evidence byte by byte.

## 8. Deviations from the plan

The frozen constants and the causal rules were not changed. The following
implementation-level deviations are recorded:

1. **One extra persisted field.** `PendingPersonalForecast` stores
   `lastValidUsedPercent` in addition to the fields the plan lists. The
   frozen protocol compares the used percentage with the **previous**
   observation, not with the origin, so the last valid percentage has to
   survive a restart. The plan calls its field list "recommended".
2. **Same-second duplicate refresh.** The plan says a same-second refresh is
   "ignored for pending progression". The implementation ignores it: the
   interval neither advances nor produces evidence. The frozen reference
   discards the whole interval on a non-positive gap. The frozen data has
   `duplicateSeconds: 0`, so the parity gates are unaffected.
3. **The `too_late` rejection code is unreachable in practice.** With a
   90-second maximum gap and a 90-second wide maturation window, the first
   observation that reaches the horizon always lands inside it. The branch is
   kept as a guard and is covered by a direct unit test.
4. **No `audit_paused` rejection code.** While the audit slot is occupied the
   personal transitions are paused and no outcome is produced, so there is no
   rejection to record. The code was removed instead of being emitted.
5. **At most one evidence event per observation.** The outbox holds a single
   slot, so an observation records either a matured outcome, a rejected
   outcome or a lifecycle transition, in that priority order.
6. **`state_changed` kept its original meaning.** A separate `is_dirty()`
   accessor drives the persistence retry, so a failed write is retried on the
   next refresh without changing the production contract.
7. **The shadow normalization takes the quota state.** The plan requires the
   weighted metadata to be normalized against the current 5H `QuotaRateState`,
   so `FiveHourShadowState::normalize` receives it.
8. **A `lifecycle` field was added** to the evidence record, because the plan
   asks for lifecycle records but does not list their field.
9. **`shadow_estimate()` carries `#[allow(dead_code)]`.** The shadow phase
   deliberately publishes nothing, so the accessor is only called by the
   offline tests. The crate is a binary, where `pub` does not imply a
   reachable symbol.

## 9. Not verified

- **Packaged and native smoke validation (plan §20 step 12) was not
  performed.** The machine used for this implementation cannot run the signed
  installer and observe the learner across a real restart. The restart,
  pending resume/reject, account switch, quota reset, log retry and the
  30/60/120/300-second cadence behavior are covered by the automated tests
  only. A packaged run remains open.
- The live overlay, the tray and the notification behavior were not observed
  interactively; the automated gate is that the visible `CodexUsage` values
  and the frontend DTO are unchanged and that the frontend suite is untouched.

## 10. How to reproduce the gates

```text
cd src-tauri
cargo check --all-targets
cargo test
cargo test --lib -- --ignored real_usage_forecast_replay

cd ..
npm ci
npm run build
npm test
pwsh ./scripts/Test-TauriRelease.ps1

node scripts/burn-rate-real-report.mjs
node scripts/burn-rate-evidence-weighted-report.mjs
node scripts/burn-rate-personal-calibration-report.mjs
node scripts/burn-rate-personal-evidence-decay-report.mjs
node scripts/check-replay-evidence.mjs
```

The real-history artifacts under `output/` are local and ignored by git; the
committed evidence under `docs/handoff/*-evidence/` is the golden baseline.

## 11. Files changed

```text
src-tauri/src/codex/burn_rate.rs            the tracker, the shadow pass, the migration
src-tauri/src/codex/burn_rate/weighted.rs   new: the production weighted estimator
src-tauri/src/codex/burn_rate/personal.rs   new: the learner and the pending interval
src-tauri/src/codex/burn_rate/shadow.rs     new: the shadow state, the outbox, the log
src-tauri/src/codex/burn_rate/shadow_tests.rs new: the lifecycle tests
src-tauri/src/codex/burn_rate/replay.rs     now imports the production helpers
src-tauri/src/config.rs                     the evidence log path
src-tauri/src/commands.rs                   the outbox flush after the save
scripts/check-replay-evidence.mjs           new: the byte-identical replay gate
```

No frontend file, no Cargo dependency and no npm dependency changed.
