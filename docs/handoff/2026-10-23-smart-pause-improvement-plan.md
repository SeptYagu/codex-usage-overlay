# Smart Pause improvements 1–4 — implementation plan

- Date: 2026-10-23
- Baseline: upstream `b792bdf` (v1.3.3), plus the local fixes recorded in [the quota stopping review](2026-10-23-quota-stop-review.md).
- Status: implementation authorized and recorded in [the implementation handoff](2026-10-23-smart-pause-improvements-implementation.md). Visible status and adaptive polling are implemented; the resume store/policy is implemented with execution disabled because the current connection lacks verified interruption/activity capabilities. This plan remains the intended contract, not a claim that all gated execution is available.
- Scope: visible protection status, verified interruption where supported, overlay-managed resumption, and adaptive quota polling.

## Intended behavior

The overlay reports what it has observed: whether protection is enabled, whether its connection works, whether a pause request was accepted, and whether stopping was confirmed. Resume is a separate opt-in action, checked against fresh quota and subsequent user activity.

Keep the existing pause defaults: 5H at or below 5%, Weekly at or below 2%, with protection disabled for fresh and upgraded profiles unless already enabled. Preserve the current inclusive, rounded display-percentage trigger behavior in this work. Resume eligibility uses validated raw quota values so display rounding cannot authorize an early resume. The published burn-rate estimator remains unchanged.

Direct interruption is a capability to establish, rather than an assumed property of the existing desktop bridge. Automatic resumption depends on stronger capabilities than message delivery.

## Shared implementation contracts

1. The Rust controller owns policy and state. Settings, tray and overlay consume the same versioned status snapshot.
2. Qualify targets by owner/host, thread and active turn or observed active period. Define de-duplication and event matching using the same identity throughout.
3. Keep the existing generation checks and reservation cleanup. Disabling, an account change/loss, failed reads and superseding observations invalidate unsent work. A request already submitted can still complete.
4. Preserve accepted sends across equivalent newer low samples, while preventing an obsolete completion from restoring invalidated ownership or ledger entries.
5. Missing, invalid, stale or unattributed quota cannot authorize a new control action. Cached usage remains available for display.
6. No state lock is held across disk or transport I/O. Quota reads and control operations have separate bounded deadlines.
7. A confirmed interruption refers to a particular turn. Goal continuation, queued work and background terminals are separate capabilities. Product status must reflect the actual scope of protection.
8. No model-created continuation task is requested after the overlay scheduler replaces that path. Existing external continuation tasks are outside the overlay's ownership; migration must explain that they may still run.

## 1. Visible protection status

### Backend and UI

Extend `get_auto_pause_status` and add an `auto_pause_status_changed` event carrying a monotonically increasing `revision` and a complete bounded snapshot. Hydrate after subscribing, and reject snapshots or events older than the last applied revision, following the existing settings/cache synchronization pattern.

The snapshot should include:

| Field group | Contents |
| --- | --- |
| Configuration | Enabled, pause thresholds, resume enabled |
| Capabilities | Cooperative delivery, direct interruption, completion observation, user-activity observation, continuation control |
| Connection | Checking / available / unavailable, last successful probe, stable reason code |
| Quota health | Read outcome, last successful sample time, effective polling interval, next poll time |
| Targets | Qualified identity, phase, trigger reasons, request time, acceptance time, interruption evidence, last failure |
| Resume | Pending / waiting for quota / needs review / submitting / completed / cancelled / expired |
| Counters | Accepted requests and confirmed interruptions as separate values |

Keep `confirmedCount` temporarily as a deprecated alias of accepted requests; do not reinterpret its historical meaning. Add explicit counters for new consumers.

Use these user-facing summaries:

- **Off**: protection disabled.
- **Checking**: initial quota or connection check incomplete.
- **Armed**: enabled, quota available, connection usable, no pause pending.
- **Quota low — no active chats**: low quota without an eligible running target.
- **Pause requested — unconfirmed**: cooperative request accepted or direct interruption awaiting evidence.
- **Turn interrupted**: matching interruption confirmed; show whether further continuation is controlled.
- **Needs attention**: unavailable connection, stale/failed quota, unconfirmed stopping, or an indeterminate resume submission.

Display a compact status and sample age beside the Smart Pause controls, a matching tray-menu row, and a short tray-tooltip summary. Put per-chat outcomes and failure details behind expandable Settings details. Use text as well as color, with English, Simplified Chinese and Traditional Chinese strings.

Refresh connection health while enabled, including retrying a previously unavailable connection. Start with a 30-second read-only health interval, reconnect backoff, and immediate invalidation on a detected disconnect. Coalesce probes with in-flight control discovery. UI rendering must never trigger a control message.

### Acceptance criteria

- Accepted messages never display as confirmed stopping.
- Empty target lists, partial success and transport errors have distinct summaries.
- A closed/reopened Settings window hydrates correctly; late snapshots cannot replace newer events.
- Disabling updates every visible surface immediately, including during discovery or a send.
- Status collections are bounded at 512 target records and prune completed history after 24 hours.
- Verify collapsed and expanded details at 960×620 and 760×500, in all three languages. Existing controls remain reachable; expanded details can scroll.

## 2. Supported direct interruption and confirmation

### First deliverable: capability investigation

Inspect the installed client's exposed control interface and relevant generated schemas. Establish whether a supported connection can reach the server that owns an already-running desktop turn, identify its current turn and subscribe to its completion/activity events.

The official app-server protocol documents `turn/interrupt` and a resulting interrupted completion. That establishes protocol semantics, but does not establish that this overlay's existing `tools/call` bridge exposes that operation. A fresh usage-reader process does not prove access to the desktop's running turn.

Record a capability matrix, with evidence and client version, for:

1. Reading the owning host/thread/turn identity.
2. Interrupting that exact active turn.
3. Observing the matching `turn/completed` result or authoritative readback.
4. Observing later user activity, including manual stop/pause, archive and new turns.
5. Suspending/restoring automatic goal continuation, where applicable.
6. Discovering queued work or other execution that can restart after interruption.

Read-only discovery is sufficient for the investigation stage. Actual control validation belongs to an explicit disposable-chat smoke test during implementation, with its result recorded separately.

### Adapter and controller behavior

Introduce a control adapter with explicit capability reporting, using the existing cooperative bridge as one implementation. A direct adapter is enabled only after the owning-server route is established. Unsupported clients retain cooperative mode with visible limitations; no speculative endpoint or process-wide termination fallback is introduced.

For direct control:

1. Revalidate quota, account, settings generation and the target's current active turn.
2. Record the action identity and current user-activity revision before submission.
3. Request interruption for that exact turn. Preserve the existing two-second call/five-second batch bounds for request delivery.
4. Observe confirmation independently of message delivery. Start with a ten-second confirmation window, without blocking quota refresh or other targets.
5. Mark the turn interrupted only on a matching authoritative result. Acceptance, disappearance from a listing, `notLoaded`, or generic idle status is insufficient evidence of an overlay-owned pause.
6. On timeout, report unconfirmed and attempt bounded readback. Do not blindly repeat an operation with an unknown outcome. A late completion is applied only to its original action/turn.

Goal continuation and newly started turns require separate handling. Advertise continuous protection only when a supported event stream/control path can observe new activity and manage automatic continuation. Otherwise report protection as limited to observed turns. Background commands are not implicitly classified as stopped by a turn interruption.

### Acceptance criteria

- A completion for another host, thread, turn or generation cannot confirm the action.
- One failing target does not corrupt other targets' outcomes or permanently starve later targets.
- Reconnects reconcile current state before retrying. Confirmation readback does not load a second writer for the desktop transcript.
- A user restart/new turn invalidates the old pause identity; it is not mistaken for continuation of the interrupted turn.
- If the owning-server route is unavailable, the capability investigation is complete with a documented unsupported result; direct stopping remains unavailable and visible as such.

## 3. Overlay-managed resume plans

### Eligibility and settings

Add `autoResumeAfterQuotaReset`, default **false** for fresh and upgraded profiles. The existing pause switch remains independent. Existing profiles keep their pause preference, and the release notes explain the replacement of model-requested scheduling with this new explicit setting.

For the first implementation, only a 5H-only pause with a usable reset time can create a resume plan. Weekly-triggered and combined-trigger pauses remain manual-resume cases. Both quota windows must still pass the resume checks for a 5H plan.

Automatic resume is available only when the control adapter can establish an overlay-owned confirmed pause and reliably detect subsequent user actions. A cooperative message acceptance alone cannot create an executable automatic resume plan. Where those capabilities are absent, show why automatic resume is unavailable and provide a reviewable manual-resume action.

### Persistence and execution

Add a schema-versioned `pause-resume-state.json` under the existing runtime directory. Use `ConfigManager` and atomic replacement. Persist only the account/owner/thread identity, interrupted turn, original activity revision, action/job identity, triggering reset boundary, eligibility time, expiry, and execution state. Do not store credentials or conversation content.

Proposed first-version policy:

| Rule | Value |
| --- | --- |
| Earliest resume eligibility | Recorded 5H reset timestamp + 120 seconds |
| Recovery margin | Both raw remaining percentages ≥ their pause threshold + 2 percentage points |
| Required observations | Two successful post-eligibility observations, at least 15 seconds apart |
| Final sample freshness | At most 30 seconds old, with no intervening failed/unknown observation |
| Plan expiry | 24 hours after earliest eligibility |
| Active-plan cap | 512; refuse new plans visibly when full |

The timer wakes the scheduler to validate eligibility; elapsed time alone never starts work. A delayed server reset or still-low Weekly quota leaves the plan waiting for quota. A changed account invalidates it. Disabling either pause protection or automatic resume cancels its pending plans.

Immediately before submission, revalidate the current account, both quotas, pause ownership, thread/turn state, settings generation and user-activity revision. A later manual pause/stop, new user turn, archive, deletion or ownership change cancels the plan. If the adapter cannot distinguish such activity reliably, hold for manual review.

After restart, reload plans as awaiting validation. Automatic execution requires durable evidence that no intervening user action superseded the plan; without that evidence, show **Needs review**. Scheduler operation requires the overlay to be running. On return from sleep, validate once rather than replaying missed timer firings.

Persist `submitting` before issuing a continuation. Prefer a supported idempotent action when available. If the client has no idempotency/readback facility and the process crashes or the response is lost after submission, restore the plan as indeterminate and require review. Do not promise exactly-once delivery across that boundary or retry automatically.

### Acceptance criteria

- A 5H reset cannot resume while Weekly is below its recovery threshold or unknown.
- A displayed rounded percentage cannot authorize resume when the raw value misses the margin.
- An accepted pause message, an idle observation alone, or a missing reset cannot arm automatic resume.
- Manual activity, disabled settings, account switches and stale quota win over a due timer.
- Restart, suspend/resume, clock changes, corrupt records and unknown submission outcomes are covered without duplicate automatic continuation.
- Legacy Codex-created continuation tasks are documented as external tasks; the UI does not claim that disabling the overlay cancelled them.

## 4. Adaptive quota polling

### Shared policy and scheduling

Extract a pure polling policy and replace the fixed fetch-then-sleep loop in `lib.rs` with one cancellable scheduler. Keep `refreshIntervalSeconds` as the user's normal cadence. Enabling protection also enables the adaptive policy; protection off retains the normal cadence.

Proposed policy:

| Situation | Effective target |
| --- | --- |
| Comfortable quota | User's normal interval |
| Either known window ≤ pause threshold + 5 percentage points | 15 seconds |
| Low quota with active or unconfirmed targets | 15 seconds |
| Every targeted turn confirmed interrupted, no active work | Normal interval capped at 60 seconds, plus activity/reset wakeups |
| Resume plan due | Immediate fresh read; second qualifying sample at least 15 seconds later |
| Transient failures | Exponential backoff from the effective interval, capped at the greater of normal cadence and 300 seconds |

Leave fast mode only after two successful observations put both windows above their corresponding pause threshold + 7 points. This entry/exit gap avoids cadence oscillation. Unknown windows cannot establish recovery. Do not use the shadow estimator to authorize polling or control decisions in this scope.

Measure target intervals between read starts with a monotonic clock. If a read outlasts its interval, do not queue catch-up reads: wait at least five seconds after completion before another background read. Retain the existing single-flight guard; manual refreshes, reset events, control events and resume validation coalesce with it.

Wake or recompute the scheduler on relevant settings changes, new active-turn events, reset deadlines, plan eligibility, manual refresh, reconnect and return from system sleep. Drop obsolete timer generations. A delayed old timer cannot restore a slower cadence or fire a superseded job.

Expose sample age and actual effective cadence in status. Mark quota health stale after the greater of 60 seconds or twice the current effective interval; a failed read reports degraded health immediately. Control/resume actions use the stricter 30-second freshness requirement. Polling remains subject to provider reporting delay and request latency, so the UI must not promise a precise quota ceiling.

The temporary-reader architecture makes 15-second polling four process launches per minute. Measure launch time, idle CPU and handle/process cleanup on Windows. A persistent reader is a separate follow-up if measurements justify it, rather than an automatic expansion of this plan.

### Acceptance criteria

- A fake clock proves exact entry/exit boundaries, backoff, deadline recomputation and sample freshness.
- Long reads and bursts of triggers produce no overlapping readers, queued catch-up burst or busy loop.
- Disabling protection or changing thresholds interrupts a pending sleep and recomputes policy.
- Reset/resume wakeups do not run obsolete work after cancellation or restart.
- Polling and display cadence changes do not alter the burn-rate estimator or its persisted schema.

## Code changes and delivery order

| Area | Expected changes |
| --- | --- |
| `src-tauri/src/pause_control.rs` | Canonical status, capability adapter boundary, target/action state and generation integration |
| New pause-control helper modules | Pure policy, confirmation handling and resume-plan scheduler/store; split focused helpers as the implementation grows |
| `src-tauri/src/commands.rs` | Status hydration/events, settings validation, cancellation and refresh integration |
| `src-tauri/src/config.rs` | Resume preference, schema-versioned plan persistence and migration defaults |
| `src-tauri/src/lib.rs` | Scheduler lifecycle, wakeups and shutdown |
| `src-tauri/src/tray.rs` | Protection summary in the existing tooltip/presentation lifecycle |
| `src/types.ts`, Settings and tray views | Shared status DTO, compact summaries and resume controls |
| `src/i18n.ts` | All new labels, reasons and capability limitations in three languages |

Implementation order:

1. Deliver item 1's status contract and UI against existing cooperative behavior.
2. Complete item 2's capability investigation and record the supported/unsupported result.
3. Deliver item 4's polling policy and shared cancellable scheduler.
4. Complete item 2's verified adapter where the capability investigation permits it.
5. Deliver item 3's persistence and opt-in scheduler, with automatic execution gated by verified capabilities.

Items 1 and 4 can ship with cooperative mode. Unsupported direct control must not block those improvements. Item 3 may expose plan/review UI while automatic execution remains unavailable, but that is not completion of automatic resumption. No version bump, commit, release or live control action is part of writing this plan.

## Verification and completion

Keep the existing quota-reader, stale-batch, bridge-shape and de-duplication regressions. Add focused tests for each acceptance contract above, using scripted adapters, an injected clock, temporary persistence and controlled await barriers.

Run `npm test`, `npm run build`, locked Rust tests, locked `cargo check --all-targets`, and the six packaging regression cases. Add real layout checks for the changed Settings/tray views. A shipped claim of verified interruption or automatic resumption also needs recorded end-to-end evidence on the supported desktop connection; unit tests alone cannot establish it.

At each stage update the handoff with implemented behavior, test results, supported client versions and any capability still unavailable. Completion requires truthful UI and tested fallback behavior, with direct-stop and auto-resume claims limited to the capabilities actually demonstrated.

## Sources

- [Official Codex app-server protocol](https://learn.chatgpt.com/docs/app-server): exact turn interruption, completion events, runtime thread status and goal APIs. Checked 2026-10-23. This documents protocol behavior, not automatic availability through the overlay's desktop bridge.
- [Local quota stopping review](2026-10-23-quota-stop-review.md): current behavior, confirmed fixes and remaining verification limits.
- [Existing Smart Pause implementation](2026-10-19-v1.3.1-smart-pause-implementation.md): desktop bridge integration and compatibility history.
