# Multi-provider monitoring and logo switching

Status: implemented for v1.6.0-preview.3; see [implementation and validation](2026-11-03-multi-provider-implementation.md). This document retains the original design and acceptance targets.

Baseline: `c341f50` / `v1.6.0-preview.2`.

## Intended behavior

Monitor every available, enabled provider in the background while displaying one provider at a time. Clicking the provider logo changes the displayed provider immediately, using its latest cached reading. Display selection must not interrupt polling, discard history, reset a learner, reconnect a provider, or change protection.

Confirmed decision: **protection is independent for each provider**, including providers not currently displayed. On upgrade, preserve the existing protection setting for the previously selected provider; other providers start with protection off.

Initial scope is the existing three source types: local Codex, Claude, and one explicitly configured DSH instance/account. This does not add monitoring of every DSH instance, subscription account, or local login simultaneously.

## Availability and provider controls

Keep monitoring enablement, connection/configuration, data freshness, and display selection separate.

| Provider | When it can be monitored | Existing limits to preserve |
| --- | --- | --- |
| Local Codex | Its installed app-server/CLI can provide quota for its signed-in account | Existing executable discovery, read budget, retry and account validation; no direct reading of Codex credentials |
| Claude | An existing statusline bridge supplies readings, or live reading has already been explicitly enabled | Live reading stays opt-in and at most once a minute; bridge-only mode obtains new data only when Claude Code emits it |
| DSH | An explicitly paired instance and selected subscription account provide a valid read path | No automatic installation, pairing, account selection, session selection, or fallback to local Codex |

- Add a monitor-enabled preference per provider, initially enabled so existing usable sources are monitored automatically. An enabled provider without a configured read path reports “Needs setup”; it does not start setup or authentication itself.
- Give each source a health state: checking, current, stale, failed, or needs setup. Retain the last successful value with its age when a read fails.
- Separate connection discovery from repeated quota reads. Use bounded retries/backoff for unavailable sources instead of repeatedly spawning failing readers.
- An intermittent error must not erase history or repeatedly add/remove a source from the logo cycle. Explicitly disabled or never-configured sources are excluded. A displayed provider that loses its connection stays visible with its stale/error state until the user switches it.
- Disabling a provider's monitor also disables its protection and cancels unsent actions for that provider; require fresh valid data before protection can operate again.

## Architecture changes

### 1. Replace selected-source runtime state with per-source runtimes

Today `AppState` has one refresh guard, polling policy, reader status, latest sample, burn-rate tracker, reset tracker, and source generation. `switch_provider` replaces these when the displayed source changes. Those responsibilities must move into a registry of independent provider runtimes.

Each runtime owns or references:

- Explicit source identity and a configuration generation.
- Its immutable persistence store.
- Its read adapter/client, in-flight refresh guard, and shared result for concurrent refresh callers.
- Its polling deadline, wake signal, retry state, and server retry-after deadline.
- Latest accepted raw sample, formatted usage, freshness, and reader diagnostics.
- Burn-rate tracker and reset notification state.
- Provider-specific protection configuration, status and action bookkeeping.

Keep window geometry, appearance, language, updater, audio output, and display selection at application scope. The DSH connection manager and Claude live-request limiter remain shared within their respective provider, so a foreground request cannot duplicate background transport.

Reads receive a provider runtime and explicit settings snapshot. They must not choose their source by rereading the currently displayed provider.

### 2. Make persistence namespaces immutable before adding concurrency

`ConfigManager` currently stores its namespace in an `Arc<Mutex<String>>`; clones share it. Calling `set_source` around concurrent reads would let one provider write another provider's history.

Introduce an immutable source-specific store, constructed from the shared runtime directory and an explicit source key. Global settings and window files remain separate from source-specific files.

Preserve existing filenames: unprefixed local Codex state, `claude-` state, and the existing DSH instance/account namespace. Do not reset histories, duplicate evidence logs, move credentials, or change the burn-rate algorithm. A fresh install and an upgrade must both load independent stores correctly.

### 3. Run independent monitoring workers

Use one scheduled worker per enabled/configured provider, with at most one in-flight quota read per provider. A slow Codex read must not hold up Claude or DSH.

- Keep the configured normal refresh interval initially; each provider independently applies existing backoff, protection acceleration and server-requested waiting.
- Preserve Claude's once-per-minute live limit across background, manual, and reset-triggered reads.
- Manual Refresh refreshes the displayed provider and joins its existing in-flight operation. Changing the display itself does not force a network read or bypass a deadline.
- Record a sample in the learner only when it is genuinely new. Re-reading a cached Claude statusline snapshot must not manufacture extra observations.
- Disabling/reconfiguring one provider invalidates only that runtime's generation and pending commits. Display changes invalidate no monitoring generations.
- Account/DSH binding changes retain identity validation and reject late results for the previous binding. Never commit samples or actions to a different runtime.
- Stop workers and release owned reader processes/connections during application shutdown.

### 4. Separate display events from source events

Expose a snapshot of all provider states to the frontend and emit provider-scoped updates carrying source key, configuration generation and update revision.

The frontend maintains a cache for every provider. It renders the cache entry named by the display preference, while accepting valid updates for background providers. Register listeners before loading initial snapshots and reconcile revisions so startup events cannot be overwritten by older bootstrap results.

Use a separate revision for display/settings changes. A delayed provider response must not change selection, put old-source values beside a new logo, or replace another provider's error/status.

Update tray icon, tooltip, credits visibility and protection summary from the displayed provider. Background events update their own cache and notifications without changing the tray's displayed source.

## Logo interaction

- Replace the expanded overlay's decorative logo span with an accessible button using the same visual footprint.
- Cycle in a stable order: **Local Codex → Claude → DSH → Local Codex**, skipping sources outside the configured monitoring set.
- The backend computes the next provider from authoritative selection and availability under a selection lock. Rapid requests must not all compute the same next source from an old frontend value.
- Apply the new identity and cached reading together, then synchronize the selection across overlay, Settings and tray. Persist it across restart.
- Tooltip and accessible name identify both the current provider and next provider. Support Enter/Space and a visible keyboard focus indicator. When there is no other eligible source, activation is unavailable and the tooltip explains why.
- Restore pointer events specifically for the logo button: `.overlay-capsule > *` currently disables them. Prevent its left-button events from starting window dragging. Preserve dragging elsewhere and the existing right-click menu.
- With the logo hidden, Settings retains a display-provider selector. With click-through enabled, use Settings/the tray to turn it off before clicking the logo. The collapsed bars retain their current design: hover to expand, then use the logo.
- Re-run measured sizing and docked hover-position checks when switching between providers, including Claude's absent credits and the DSH mark.

## Independent protection and notifications

Store protection enablement and thresholds per provider. Settings needs a provider configuration selector distinct from “Displayed provider”, so reviewing or changing a background provider's protection does not change the overlay.

- Local Codex retains its cooperative pause flow and its own deduplication/status state.
- Claude's guard file follows Claude's protection setting, rather than checking whether Claude is displayed. Its statusline/hooks continue to operate independently.
- DSH acts only on the explicitly paired instance, matching account, and explicitly bound sessions. Keep cancellation, freshness, generation, target validation and uncertain-delivery rules.
- A display change must neither cancel valid protection for the previous provider nor arm protection for the next one.
- Existing default-off behavior and unsupported automatic-resume behavior remain. This feature does not introduce automatic continuation.
- Reset notifications operate on monitored providers, using the existing notification preferences. Include the provider name and namespace deduplication by source/quota/cycle, so simultaneous resets remain distinguishable. Do not notify again merely because the display changed.

## Settings migration and localization

Introduce a versioned settings schema with a display-provider preference and a per-provider monitoring/protection section; retain global appearance and system settings.

Migration rules:

1. The old `provider` becomes the displayed provider.
2. Preserve existing appearance, update preferences, refresh interval, reset notification preferences, and saved source-specific histories.
3. Copy the legacy protection enablement only to the formerly selected provider; all other providers start with protection off. Preserve its threshold values, including separate DSH thresholds. Seed other providers' thresholds from their appropriate saved/default values without enabling them.
4. Preserve Claude live-reading consent and DSH connection/account/session preferences. Do not save or reconstruct memory-only pairing secrets.
5. Save migration atomically and make it idempotent. Opening Settings or changing the displayed source must not run it again.

Add clear English, Simplified Chinese and Traditional Chinese copy for monitoring, displayed provider, provider configuration, health/freshness, logo switching, protection scope and unavailable states. Replace the current claim that only one usage source is active at a time. Explain the additional background reads and the per-provider monitor switches.

## Implementation order and acceptance gates

1. **Settings and immutable stores.** Add the versioned schema/migration and provider stores. Verify compatibility with existing Codex, Claude and DSH files and the old enabled-protection setting before changing polling.
2. **Runtime registry and polling.** Extract provider runtimes/read adapters; run concurrent independent workers. Use deterministic fake clocks and readers to prove isolation, cancellation and bounded request rates.
3. **Protection and notifications.** Move their configuration/state to the proper runtime and remove display-based gates. Test background protection explicitly before making display switching available.
4. **Frontend state and logo button.** Add provider snapshots/events, authoritative display switching, provider configuration controls, and all three translations.
5. **Regression and Windows acceptance.** Run frontend tests/build, locked Rust tests, companion/security and packaging checks, then extend Edge and native smoke checks for the new behavior.

Required regression scenarios:

- Two/three available providers keep gathering independent samples while only one is displayed; hiding the window or 5-hour reading does not stop monitoring.
- A blocked/failed provider does not delay others; concurrent manual/background reads share one request per source; retries and Claude's limiter remain effective.
- Switching repeatedly during in-flight reads never mixes samples, histories, errors or protection targets, and never resets burn-rate state.
- An account/binding change discards its old response without cancelling other providers. Shutdown cancels outstanding readers cleanly.
- Independent protection works while hidden; disabled providers never send actions; switching a logo never changes enablement or bypasses target/freshness checks.
- Reset notifications identify providers and fire once per new cycle, independent of display changes.
- Upgrade migration preserves histories and enables protection only where previously enabled; legacy and migrated settings survive restart.
- Logo clicks and keyboard activation cycle correctly; dragging elsewhere, right-click, hidden-logo and click-through paths remain usable.
- Empty, partial, stale and failed availability sets have deterministic selection behavior. Failure does not cause automatic visual hopping.
- Both layouts, both themes, three languages, one/two quota rows, provider-specific credits, 100–250% scale and all dock edges fit correctly at standard/minimum Settings sizes.

Native Windows acceptance should use at least two real available providers to confirm background histories advance, logo switching remains instant, and dock/drag/click-through behavior survives mixed DPI. Record any unavailable-provider or real-protection scenarios not exercised. Implementation, commits and any further prerelease are a subsequent step; this document only plans the change.
