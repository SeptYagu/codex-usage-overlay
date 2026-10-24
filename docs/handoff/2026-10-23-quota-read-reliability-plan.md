# Quota read reliability fix — implementation plan

Date: October 23, 2026. Baseline: `36c09f0` (v1.4.0).

Status: implemented for v1.4.1; see the [implementation and verification record](2026-10-23-quota-read-reliability-implementation.md). This document preserves the agreed design.

## Problem and intended result

Read-only investigation reproduced valid quota responses arriving in 24.110 and 27.474 seconds, after initialization completed in 176 and 313 ms. The production reader expires after 20 seconds and terminates its helper. At the user's 60-second polling setting, one failed refresh moves the next attempt to 120 seconds after the previous attempt began.

The fix should accept these delayed valid responses, recover promptly from transient failures, and preserve enough local evidence to explain recurrence. Longer waits must not make old observations eligible for automatic pause or resume.

The underlying latency increase remains unattributed: local network/service behavior and a CLI regression are still possible. Plugin-disabled probes also had a latency spike. The fix therefore addresses demonstrated reader behavior without depending on a plugin-specific explanation.

## 1. Introduce structured reader outcomes and a shared deadline

Primary files: `src-tauri/src/codex/client.rs`, `src-tauri/src/commands.rs`; bounded executable rediscovery may also require `src-tauri/src/codex/finder.rs`.

- Replace the untyped internal error path with a reader error carrying stage, kind, RPC code/structured HTTP status when present, and retry eligibility. Stages distinguish executable discovery/spawn, initialization, quota request, and payload validation. Cleanup problems are diagnostics and do not erase a valid quota response.
- Give one physical refresh a **45-second overall budget**, measured with a monotonic deadline. It covers executable rediscovery if needed, attempts, retry delay, and cleanup. Reserve the existing two-second cleanup allowance before each request deadline. Do not assign a fresh 45 seconds to each attempt.
- Cap initialization at **10 seconds**, within the shared budget. Once initialized, the quota request can use the remaining request budget. This accommodates the observed 24–28-second quota responses.
- Keep cached executable recovery. Run blocking discovery outside the async executor with a bounded lifetime; an expired discovery must not launch a helper later. Any discovery subprocess must also be owned and cleaned up.
- Keep `kill_on_drop`, explicit bounded termination, default stdio transport, and correct initialization ordering. Cancellation must stop only the helper belonging to that refresh.
- Preserve the public Tauri usage response and error-string contract; structured metadata is internal or additive status data.

## 2. Retry early transient failures once; recover sooner after exhaustion

Primary files: `client.rs`, `commands.rs`, `polling.rs`.

Allow at most **two attempts per refresh**. Retry only after an early transient failure, after cleaning up the first helper and waiting **one second**, and only if at least **10 seconds of usable request time plus cleanup** remain. A timeout that consumes the request budget gets no additional inline attempt.

Retry candidates are initialization timeout, unexpected pipe closure, and specifically recoverable transport I/O errors. RPC failures qualify only when structured data explicitly identifies a transient server/transport failure. Missing executable, permission denial, authentication rejection, invalid request, unsupported protocol, malformed result, and invalid quota payload do not get an inline retry. An unknown RPC error is not presumed transient. Honor a structured server retry delay within the remaining budget; otherwise defer to polling.

After a refresh exhausts transient recovery, schedule the next background attempt **15 seconds after completion**, then 30, 60, 120, 240, and 300 seconds after consecutive failures at the user's current setting. In general use `15 × 2^(consecutive failures − 1)`, capped at the greater of the configured normal interval and 300 seconds. A longer explicit server retry delay takes precedence. Non-transient errors retain normal/backoff polling rather than entering this fast recovery sequence.

A successful fresh response resets the failure streak and restores the existing normal/near-threshold cadence. Successful reads retain the existing start-based cadence and minimum five-second gap after a long read. Settings changes do not erase the failure streak. Manual, reset-triggered, and background requests continue to share one refresh; concurrent callers observe the same outcome, including a shared failure, rather than receiving cached data disguised as a successful refresh.

## 3. Carry observation timing into protection checks

Primary files: `commands.rs`, `pause_control.rs`, `pause_control/status.rs`, `pause_control/resume.rs`.

Carry the successful attempt's quota-request start and receipt times alongside the parsed sample. Keep existing `fetched_at` receipt semantics and persisted estimator schemas for compatibility. The separate timing metadata provides a conservative freshness bound because no authoritative server snapshot timestamp was demonstrated.

- For automatic actions, measure observation age from **quota-request start**, not only from local receipt. Keep the existing **30-second maximum action age**; the longer transport deadline must not extend it.
- A valid response already older than that bound is usable for display but cannot schedule a pause, establish resume recovery, or display protection as armed. Show stale/protection-needs-attention and schedule another read after 15 seconds. A failed attempt or display-only observation clears resume recovery and invalidates unsent pause work.
- The pause queue carries the successful request's monotonic start time, so discovery and delivery consume the remaining portion of the same 30-second freshness window. Recheck before every send. Do not give a slow response a new 30-second allowance at receipt.
- At the first failed attempt, invalidate old pending actions before waiting or retrying. A later fresh success can rearm protection. Preserve accepted-request de-duplication, account/settings/generation invalidation, and existing resume ownership requirements.
- Slow in-flight reads continue to age the previous cached observation. Mark an old observation stale independently of retry/backoff timing; increasing a recovery interval must never make cached evidence younger or actionable again.
- Apply a successful refresh to notifications, estimator state, and protection only once, even if it required two helper processes. A display-only result must still process account changes and invalidate the old account's action state.

## 4. Retain bounded diagnostics and explain recovery in Settings

Primary files: `client.rs`, `config.rs`, `commands.rs`, `pause_control/status.rs`, `src/types.ts`, `src/components/ProtectionStatus.tsx`, `src/i18n.ts`.

Drain helper stderr concurrently to avoid pipe blockage. Parse bounded chunks/lines; retain only allowlisted diagnostic categories and error codes. Drop unknown content while counting it. Do not persist raw stderr, full RPC bodies, credentials, account/chat identifiers, arbitrary error messages, headers, or URLs with query strings.

For each attempt, record start time, stage durations, outcome, retry reason, attempt count, sanitized executable identity, and CLI version when available. Cache version metadata per executable; obtaining it must not block quota refresh. Keep the latest success and latest failure independently, plus a local history capped at **200 events, 256 KiB, and seven days**, whichever limit is reached first. Serialize writes and replace atomically outside application state locks. Disk/diagnostic failures must not fail a quota read. A later success must not erase the last failure evidence.

Add current read phase, retry state, last failure category/stage/time, and next recovery attempt to the existing status snapshot. Preserve revision/event hydration ordering. Settings uses concise localized text such as “Reading quota”, “Retrying quota read”, and “Quota read timed out; retrying in …”. Keep protocol details in expanded diagnostics. Update English, Simplified Chinese, and Traditional Chinese. Preserve the visible distinction between cached display values and actionable protection data.

## Implementation order

1. Add typed outcomes, observation timing, and the bounded diagnostic model without changing the timeout policy.
2. Implement the shared deadline, bounded early retry, and subprocess lifecycle handling.
3. Wire retry invalidation, conservative action freshness, shared refresh outcomes, and recovery polling together.
4. Expose localized status and retained diagnostics, then run the complete validation gates.

Each step must leave the relevant existing regression tests green. No release/install step is needed to review the implementation.

## Verification and acceptance criteria

Use deterministic transports and Tokio's paused clock for deadline/backoff tests, rather than sleeping through real 45-second cases. Extend the existing owned-child fixture for actual Windows process and pipe lifecycle checks.

Required regressions:

- The observed 24.110/27.474-second responses succeed; a stalled read respects the 45-second overall budget including cleanup.
- An early transient failure followed by success uses two attempts; authentication, invalid payload, and unknown RPC failures do not retry. No attempt, sleep, discovery, or cleanup restarts the overall deadline.
- Timeout, cancellation during either attempt or retry delay, pipe errors, noisy/oversized stderr, and helper exit leave no owned helper running and cannot block refresh indefinitely.
- Exhausted transient reads produce the 15/30/60/120/240/300-second completion-based sequence at a 60-second setting. Server delays, manual/background coalescing, settings wakeups, long successful reads, and restoration of normal/15-second near-threshold cadence behave as specified.
- Every coalesced caller observes the same final result. Success is accepted once; shared failure does not masquerade as success with cached values.
- Retry invalidates old unsent work immediately. A response or queued send over the 30-second conservative action bound cannot pause/resume or claim armed protection. A 27.5-second response has only about 2.5 seconds of action eligibility remaining. Account changes, recovery, disabling, and failure clear previous eligibility.
- Restart-safe diagnostics preserve the last failure after success, respect all size/age limits, exclude sensitive fixture values, and tolerate corrupt history or unwritable storage.
- Frontend retry/countdown/stale states, status hydration ordering, and all three locales work with older/missing additive status fields.

Run `cargo test --manifest-path src-tauri/Cargo.toml --locked`, `cargo check --manifest-path src-tauri/Cargo.toml --locked --all-targets`, `npm test`, `npm run build`, and `pwsh -NoProfile -File scripts/Test-TauriRelease.ps1`.

Finish with a bounded read-only smoke run against the installed helper using the same recorded fields and normal request cadence. Report latency distribution, attempts, failures, and cleanup; do not infer a long-term failure rate from a small sample. Use controlled helper stalls/disconnects for failure-path smoke checks. Real chat pause/resume messages are unnecessary for this reader fix.

Acceptance: delayed valid responses no longer fail at 20 seconds; transient exhaustion gets a recovery attempt in 15 seconds rather than roughly 100; one refresh remains bounded; stale observations never gain automatic-action eligibility; and the next failure remains diagnosable after recovery.

## Deferred follow-up

Evaluate a persistent owned app-server only after measuring this fix. It introduces restart/reconnect, update detection, idle-resource, request routing, and unsolicited-event handling concerns and is not necessary to correct the demonstrated timeout mismatch. Plugin disabling is not a proven fix. No global Codex configuration change, estimator retuning, automatic-resume expansion, installed-app replacement, commit, or publication is part of this planning task.
