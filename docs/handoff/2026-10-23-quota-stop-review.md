# Quota stopping review — 2026-10-23

## Baseline and scope

The local checkout initially pointed at `61566eb`. Fetched `origin` and fast-forwarded `main` through 18 commits to `b792bdf` (v1.3.3) before reviewing Smart Pause. The initial observation that stopping was absent applied only to the stale checkout. Local reader fixes were preserved across the update and reviewed against the updated code.

Reviewed the quota reader, refresh pipeline, Smart Pause threshold decisions, delivery worker, de-duplication and bridge transport. The complete regular test suites also cover the remaining repository behavior. This was not an independent audit of every estimator, updater or window-management implementation.

## Findings fixed

| Priority | Finding | Resolution |
| --- | --- | --- |
| P1 | An old low-quota batch retained its settings and usage snapshot across discovery and bridge awaits. Disabling, recovery, account loss or a failed read could leave its queued pause eligible for delivery. | Version each observation, clear superseded queued work, invalidate after read failures and disabling, and check the observation and current settings before discovery and each send. Obsolete completions cannot restore invalidated ledger entries. Accepted sends carry forward for the same account, active period and still-triggered cycle, preserving de-duplication. |
| P2 | Invalid listing JSON or a changed listing shape looked like an empty active-session list. The worker could claim bridge availability and clear active periods after a protocol failure. | Return a protocol error for malformed listings. Valid empty groups still count as an idle observation. |
| P2 | The quota reader pipelined initialization and the usage query, ignored initialization errors, and had no process drop guard. Cancellation and early pipe errors could bypass cleanup. | Wait for initialization success, use the package version in client metadata, propagate protocol errors, and enable cleanup on drop. Explicit cleanup has a two-second deadline. Stdio uses the default transport for compatibility with older CLIs. |
| P2 | Primary/secondary quotas were labeled 5H/Weekly regardless of duration, and null or unrelated buckets could be mistaken for usable Codex data. | Prefer the Codex bucket, validate its shape and fallback identity, and map supported durations (300/10080 minutes). Unsupported or malformed durations are unavailable. Missing/null durations retain the legacy positional mapping. |

## Stopping behavior and limits

- Smart Pause remains opt-in, with defaults of **5% remaining for 5H** and **2% for Weekly**. The comparison includes the threshold. Detection happens on successful quota polls; there is no continuous quota enforcement between polls.
- It sends a cooperative message to active Codex chats through the desktop bridge. It does not call `turn/interrupt`, terminate active Codex tasks, or verify that a turn actually became paused.
- A usable 5H reset asks Codex to create a one-time continuation task after reset plus two minutes. Weekly triggering takes precedence and requests plain pause. The overlay does not verify creation or execution of that task.
- `confirmedCount` means accepted message requests. It is not proof of halted inference or successful timer creation. The settings hint and READMEs now explain this distinction.
- A request already submitted to the desktop bridge can complete after cancellation. The fix stops subsequent requests and stale ledger updates; it cannot retract a submitted message.
- Missing account identity prevents sending. A live, read-only probe of the installed Codex executable confirmed that this installation returns `accountId` and 300/10080-minute windows. Only field names and identity presence were inspected; no credentials or account identifiers were recorded.
- Bridge availability and counters are exposed by `get_auto_pause_status` but are still not rendered in Settings. A disconnected bridge can therefore prevent pausing without a visible connection indicator.

The reader handshake follows the [official app-server initialization protocol](https://learn.chatgpt.com/docs/app-server). Official turn interruption is a separate operation; the current product intentionally uses cooperative messages.

## Verification

- Rust: **311 passed, 0 failed, 4 existing ignored offline replay tests**; locked `cargo check --all-targets` passed without warnings.
- Frontend: **97 passed**; production TypeScript/Vite build passed. Final localized wording was subsequently checked with the same commands.
- Packaging regression: **6/6 cases passed**.
- New coverage includes queued and in-flight invalidation, disabling during a multi-chat batch, preserving an accepted send across a newer low sample, malformed listings, initialization ordering/errors, real isolated child-process cleanup on success/error/timeout/cancellation, and quota bucket/window compatibility.
- No real pause or continuation messages were sent during this review. Actual model compliance, timer execution, signed-package installation and GUI smoke validation remain unverified. The private-history replay evidence gate was not rerun because its local input artifacts are absent and the estimator implementation was unchanged.

Changes remain local and uncommitted on top of upstream `b792bdf`; no release or publication was performed.
