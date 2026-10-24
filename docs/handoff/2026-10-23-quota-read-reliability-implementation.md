# Quota read reliability — v1.4.1 implementation

Date: October 23, 2026. Baseline: `36c09f0` (v1.4.0).

Authorization: the user requested implementation of the [reliability plan](2026-10-23-quota-read-reliability-plan.md) and release as v1.4.1.

## Result

The quota reader accepts the observed 24.110/27.474-second responses instead of terminating them at 20 seconds. One monotonic 45-second deadline covers discovery, initialization, request attempts, recovery delay, and cleanup; request handling reserves two seconds for process cleanup. Initialization is capped at ten seconds. An early transient failure can retry once after one second if ten request seconds plus cleanup remain. Unknown/permanent RPC errors and invalid data do not retry inline.

After exhausted transient recovery, polling waits 15 seconds from completion, then doubles through 30/60/120/240/300 seconds at the default setting. Structured server retry delays also constrain manual/reset-triggered requests. Normal and near-threshold successful polling retain their existing cadence and minimum post-read gap. A concurrent refresh now returns the shared final result, including failure or cancellation, rather than substituting a cached successful sample.

The reader emits the first attempt failure before cleanup/retry. That event invalidates unsent pause work and clears resume recovery. Each valid observation carries quota-request start and receipt times independently of the existing receipt-based `fetched_at`. Pause eligibility and every queued send share the original 30-second action window. A 27.474-second response has approximately 2.526 seconds remaining; a 35-second response can update display but cannot arm protection or establish resume recovery. Display-only reads request another check after 15 seconds. Account changes still clear prior-account state. Health staleness uses healthy polling cadence rather than the inflated failure-backoff interval.

Manual resume obtains a fresh shared quota read, validates both quota windows against the latest settings, and checks observation identity/freshness and settings revision again before each bridge candidate. A plan is marked complete only after delivery acceptance.

## Diagnostics and compatibility

`quota-read-history.json` is local to the existing runtime directory. It keeps independent last-success/last-failure summaries plus at most 200 events, 256 KiB, and seven days of history. Atomic file replacement runs outside application state locks through one coalescing writer. A corrupt/unwritable history cannot fail a quota read.

Records contain stage timings, attempt/outcome, typed failure/RPC/HTTP codes, executable kind and modification time, optional validated CLI version, cleanup status, and counts of known model/plugin stderr warnings. Concurrent stderr draining uses fixed-size reads and bounded lines, so noisy helpers cannot fill an unconsumed pipe or create an unbounded diagnostic buffer. Unknown stderr is counted and dropped. No credentials, account/chat identifiers, arbitrary error bodies, full RPC results, filesystem paths, or raw stderr are persisted.

CLI version comes from a separately owned `--version` probe, bounded to two seconds and 256 stdout bytes, cached by executable path/mtime. It runs alongside the quota request; quota completion never waits for it, and an unfinished probe is cancelled. The initialization `userAgent` is deliberately not interpreted as authoritative CLI-version metadata.

The public usage shape and settings/burn-rate persistence schemas are unchanged. Protection status has additive optional reader metadata, and the frontend tolerates its absence. Reading, retrying, recovery countdown, and retained failure/timing details are localized in English, Simplified Chinese, and Traditional Chinese. Persistent app-server reuse and global plugin configuration changes remain deferred.

## Verification

- Rust: **355 passed, 0 failed**. Six ignored entries are four existing private estimator replays, the existing child fixture invoked by active lifecycle tests, and the new explicitly selected live authenticated-reader smoke test. The live smoke was run separately.
- Locked all-target Cargo check passed without compiler warnings with local incremental caching disabled. The sandbox prevented Cargo/Vite/Git path canonicalization, so build/test checks used normal workspace access. This was an environment restriction, not a code failure.
- Frontend: **113 passed**; production TypeScript/Vite build passed; version assertions confirm all six release locations agree on 1.4.1.
- Packaging regression: **6/6 fixtures passed**.
- Edge rendering: **616 cases passed**, covering all three languages, two scale factors, both Settings sizes, expanded/collapsed details, tray views, and retry/quota-error/stale fixtures. The 760×500 English retry screenshot was visually inspected.
- Native read-only smoke: three real production-reader samples at 60-second start intervals succeeded in **412 / 445 / 5,118 ms**, with valid 300/10080-minute windows, usable action age, and successful owned-reader cleanup. A further **14,351 ms** sample verified the final nonblocking version probe (`0.163.0-alpha.11.2`), a summarized plugin warning, valid quotas, and successful cleanup; detailed records remain in ignored `output/`.

New regressions exercise real protocol delays with virtual time, initialization/request caps, shared deadline exhaustion, retry classification and server delays, owned-process retry/cancellation, oversized/private stderr handling, coalesced success/failure/cancellation, completion-based polling, pending-action invalidation, remaining action lifetime, display-only observations, account switching, resume recovery, manual dispatch freshness, bounded diagnostics, and localized status recovery.

No real pause/resume messages were sent during validation. Native WebView2/OS tray interaction, mixed-monitor movement, and signed-installer installation have not been manually smoke-tested in this change; browser rendering and native reader lifecycle are verified separately. GitHub Actions builds and signs published packages from the release tag.

The intermittent delay below the quota RPC remains unattributed to a particular CLI/network/service component. These measurements demonstrate the fix and its limits, not a long-term service failure-rate estimate.
