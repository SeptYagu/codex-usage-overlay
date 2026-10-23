# Smart Pause improvements 1–4 — implementation record

- Date: 2026-10-23
- Baseline: upstream `b792bdf` (v1.3.3), plus the [quota stopping review fixes](2026-10-23-quota-stop-review.md).
- Authorization: the user approved the [implementation plan](2026-10-23-smart-pause-improvement-plan.md) with “Proceed”.
- Delivery: local working-tree changes. No version bump, commit, release, installed-app replacement or live chat-control operation was performed.

## Outcome by plan item

| Item | Outcome | Limit |
| --- | --- | --- |
| 1. Visible protection status | Implemented in Rust, Settings, tray menu and native tray tooltip, with versioned hydration/events and three languages. | Cooperative message acceptance is explicitly unconfirmed stopping. No interruption counter is fabricated. |
| 2. Direct interruption and confirmation | Read-only capability investigation completed for the installed connection. Adapter capabilities are explicit and default to cooperative delivery only. | A supported connection to the desktop's owning turn was not established. No direct-interrupt adapter is enabled. |
| 3. Overlay-managed resume | Implemented the default-off preference, durable eligibility policy/store, restart/unknown-submission handling, review guidance and cancellation UI. Removed model-requested external timers. | **Automatic resumption is not implemented as an executable integration.** No production caller creates a confirmed-pause plan or submits a continuation. The current connection cannot supply required pause ownership and later-user-activity evidence, so enabling resume is unavailable in the UI. |
| 4. Adaptive polling | Implemented monotonic deadlines, near-threshold 15-second reads, recovery hysteresis, failure backoff and cancellable waits. | Optimizations based on confirmed pauses, owning-turn activity events and executable resume deadlines remain gated with items 2–3. |

Items 1 and 4 are usable with cooperative mode. The resume store and offline policy are infrastructure, not evidence that automatic continuation works on the current desktop connection.

## Status and delivery behavior

`pause_control/status.rs` owns the complete status snapshot. It carries a monotonic revision, configuration flags, capabilities, connection health, accepted-request and verified-interruption counters, last successful sample, effective/next poll timing, bounded per-period targets, saved-plan summaries and persistence errors. `confirmedCount` remains a deprecated compatibility alias for accepted requests.

The getter does not consume the event revision. Frontend consumers subscribe before hydration and reject older snapshots; a late failed hydration cannot hide a newer event. Settings also rehydrates on focus/visibility. Disabling protection overrides the displayed summary immediately. The backend publishes changed snapshots and retries read-only health checks while enabled; successful probes are normally 30 seconds apart, with backoff to 300 seconds after failures. Discovery is coalesced with an active probe or delivery.

Targets are qualified by owner, chat and observed active turn/period. Historical records remain visible for up to 24 hours, capped at 512, and carry an active-period flag. Historical acceptance or failure cannot imply a current active request. An owner change gets a new period identity. With no stable turn identifier, the controller still cannot detect two turns that exchange places between observations without an observed idle period.

Delivery retains the prior review's generation/settings/account checks and reservation cleanup. It also refuses a queued sample more than 30 seconds after local receipt. The two-second call and five-second batch limits remain. A rotating delivery cursor prevents a slow early target from permanently starving later targets. Accepted sends deduplicate across equivalent newer low samples; obsolete completions do not restore invalidated ledger ownership. A batch timeout leaves stopping unconfirmed; it does not establish either interruption or successful continuation control.

Product summaries cover off, checking, armed, low quota without current requests, pause requested/unconfirmed and needs attention. Failed, stale, missing or unattributed quota cannot display as armed. A large future timestamp is also stale. The tooltip is localized. Rendering or opening Settings never sends a control message.

## Capability investigation

Installed executable: `C:\Users\Justin\AppData\Local\OpenAI\Codex\bin\896a0166b774037a\codex.exe`.

Reported version: `codex-cli 0.163.0-alpha.10.2`.

The installed CLI advertises `app-server proxy --sock` and daemon commands. Read-only `app-server daemon version` could not reach `C:\Users\Justin\.codex\app-server-control\app-server-control.sock`; it failed with OS error 10050 (“A socket operation encountered a dead network”). This was also reproduced outside the sandbox. No daemon was started/restarted and no Codex process was terminated as a workaround.

| Required capability | Evidence for this connection | Enabled |
| --- | --- | --- |
| Cooperative delivery | Existing desktop `tools/call` bridge supports `list_threads` and `send_message_to_thread`; scripted protocol and delivery tests pass. Live read-only discovery was established in the preceding review. | Yes, subject to bridge health and user opt-in. |
| Exact owner/chat/active-turn identity | Listings identify chat and optional host/turn. There is no demonstrated authoritative owning-server subscription. | No direct-control claim. |
| Interrupt that exact active turn | Official protocol documents `turn/interrupt`, but the current tools bridge does not demonstrate access to that method on the owning server. The advertised proxy route was unavailable in this investigation. | No. |
| Confirm matching interrupted completion | No verified subscription/readback route to the owning server. Listing disappearance or generic idle is insufficient. | No. |
| Observe later manual pause/stop, new turns, archive or deletion | No reliable user-activity revision or authoritative event stream was established. | No. |
| Control goal continuation, queued work or background execution | Not demonstrated by the current bridge. | No. |
| Safe automatic continuation | Depends on confirmed ownership and activity observation above. | No. |

These are limits of the connection investigated here, not a claim that the protocol can never support them. [Official app-server documentation](https://learn.chatgpt.com/docs/app-server) and [developer commands](https://learn.chatgpt.com/docs/developer-commands) describe protocol operations; they do not prove this desktop bridge exposes them. A disposable-chat interruption/resume smoke test was not performed because no supported owning-server route was available.

## Resume state and migration

`autoResumeAfterQuotaReset` defaults to false for both fresh and legacy profiles. The independent pause preference retains its existing migration behavior. The UI cannot enable resume when the control capabilities are missing; an existing true value can still be switched off.

The schema-1 `pause-resume-state.json` lives in the normal runtime directory. It stores action/account/host/chat/turn/activity identifiers, eligibility/expiry and state; it contains no credentials or conversation content. Reads are limited to 1 MiB and 512 plans, validate identities and expiry, and reject duplicates or unsupported schemas. Corrupt source files are reported and preserved. Writes use a temporary file and atomic replacement outside state locks, with serialized updates and visible write failures.

The future supported adapter must supply confirmed pause evidence. Cooperative acceptance cannot create a plan. The implemented policy allows only a 5H-only pause with a future reset within six hours. Eligibility starts at reset plus 120 seconds. Both validated raw remaining percentages must reach their pause threshold plus two points on two distinct post-eligibility observations at least 15 seconds apart. The final observation must be at most 30 seconds old. Missing/failed/stale/future quota clears recovery; an account loss or switch cancels. Disabling either preference cancels pending plans. Plans expire 24 hours after eligibility.

The offline submission gate requires matching owner/chat/turn/activity identity and fresh recovery. It transitions to `submitting`, which a future adapter must persist before issuing any continuation. No production sender is connected to this gate. Current-account/settings/ownership revalidation, authoritative activity observation, completion reconciliation and due-plan scheduling remain required integration work before enabling it.

After restart, pending plans become **Needs review**. A saved submitting or unknown plan becomes **Submission unknown**, never an automatic retry. Settings shows the full saved chat identifier and asks the user to review it in Codex before manually continuing. Pending reviewable plans can be cancelled; cancellation reports persistence errors. There is no manual continuation button against this unverified connection.

The cooperative prompt is now only `暂停`. It no longer asks the model to create an external “Continue” timer. Older Codex-created tasks are external: this change neither finds nor cancels them, and switching off overlay protection does not imply they were cancelled.

## Polling and reader cost

The normal cadence remains the user's configured refresh interval. With protection enabled, either known displayed remaining percentage at or below its threshold plus five points enters 15-second mode. Two consecutive successful samples with both windows strictly above their thresholds plus seven points leave it. An unknown window or failed read cannot establish recovery. The existing inclusive, rounded display-percentage pause trigger is unchanged; raw values are used only by the gated resume policy. Burn-rate estimator code and persisted schema are unchanged.

Failures double the current base interval up to the greater of normal cadence and 300 seconds. Deadlines are measured between read starts with a monotonic clock, with at least five seconds after a long read completes. The shared single-flight guard coalesces background, manual and reset-triggered reads. Settings changes wake/recompute the old wait; protection/threshold changes request an immediate read. Sleep does not replay a backlog of missed ticks. Health becomes stale after the greater of 60 seconds or twice the effective interval, and failed reads degrade it immediately.

A local read-only measurement used three isolated, owned usage-reader processes. It sent initialize and `account/rateLimits/read`, then shut down only its owned reader. No chat messages were sent.

| Measurement | Samples |
| --- | --- |
| Initialize plus quota read | 427 / 396 / 404 ms |
| Process CPU consumed by the read | 0.328125 / 0.343750 / 0.328125 seconds |
| Working set after read | 125.6 / 127.9 / 128.6 MiB |
| Handle count after read | 458 / 458 / 458 |
| CPU increase over roughly 0.5 seconds idle | 0 / 0 / 0 seconds |
| Owned processes after cleanup | All three exited. |

At 15 seconds this architecture launches four readers per minute. This short local sample is not a long-running CPU/memory soak or a provider reporting-latency guarantee. A persistent reader remains a separate design decision. The temporary benchmark script and browser artifacts are under ignored `output/`.

## Verification

| Gate | Result |
| --- | --- |
| `npm test` | 107 passed. |
| `npm run build` | TypeScript and production Vite build passed. |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked` | 330 passed, 0 failed; four existing ignored estimator replays unchanged. |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked --all-targets` | Passed without warnings. |
| `pwsh -NoProfile -File scripts/Test-TauriRelease.ps1` | All six packaging fixtures passed. |
| Edge display-controls QA | 526 cases passed: all three languages, Settings at 960×620 and 760×500, collapsed/expanded details, tray menus, device scale factors 1 and 1.75, plus existing overlay cases. |
| `git diff --check` | Passed. |

Focused regressions cover accepted-versus-interrupted status, snapshot/event ordering, late hydration errors and listener cleanup, disabled UI capability guards, explicit cancellation, bounded history, inactive target summaries, health retry coalescing, poll boundaries/wakeups/backoff/long reads, raw recovery margins, failed observations, account/activity changes, invalid resets, expiry, corrupt/oversized/duplicate persistence, migration defaults and restart submission ambiguity. The earlier reader lifecycle and stale-delivery regressions remain green.

Browser QA uses offline Tauri effects and real Edge rendering. A screenshot of the expanded English Settings view was visually inspected. Native WebView2/tray behavior, mixed-monitor transitions, an installed package, real interruption and real automatic resumption have not been smoke-tested in this change. No verified-stop or automatic-resume claim should ship until the required connection and end-to-end evidence exist.
