# Quota usage history plan

Codex Usage Overlay shows where each quota stands right now, but it keeps no record of how it got there. This feature records every provider's readings for 90 days. A new History window charts them: a usage timeline, daily consumption, and a summary of each past quota window.

**Status (2026-11-23):** implemented in one change rather than in the staged order below. One deviation: the History window opens centred each time and does not remember its size and position. [Recorder](../src-tauri/src/history/mod.rs) · [Store](../src-tauri/src/history/store.rs) · [Query](../src-tauri/src/history/query.rs) · [Window](../src/components/HistoryView.tsx)

Decided scope for v1:

- A separate **History** window, opened from the tray menu and the overlay's context menu.
- Three views: **usage timeline**, **daily consumption**, **per-window summary**. No cross-provider overlay chart.
- **90-day** retention, fixed.
- No CSV export.

## What exists today

The app never stores a usage time series. The state files on disk hold:

| File | Contents | Usable as history? |
| --- | --- | --- |
| `*burn-rate-state.json` | The current 5H and weekly window only: percent-level crossings (at most 64), the last observed percent | No. It is replaced at each window reset. |
| `*burn-rate-shadow-evidence.jsonl` | Estimator training events (predicted vs observed deltas) | No. It is diagnostics for the burn-rate algorithm and only covers short feedback intervals. |
| `quota-read-history.json` | Codex read attempts: timings, failures, CLI version | No. It has no percentages. |

Every provider's successful read passes through `refresh_usage` in [usage.rs](../src-tauri/src/commands/usage.rs). There `raw` (`RawCodexUsage`) carries the 5H and weekly `QuotaSnapshot` (`used_percent`, `resets_at`, `window_duration_mins`), `account_id`, `fetched_at` and, for Antigravity, `antigravity_weekly`. That is the one place a recorder needs to hook. History therefore starts on the day the recorder ships. Nothing earlier can be rebuilt.

## 1. Recording

A new `src-tauri/src/history/` module with `store.rs` (files, retention), `recorder.rs` (what to write), and `query.rs` (what the UI asks for).

**Hook.** In `refresh_usage`'s `Ok(raw)` branch, after the burn-rate tracker accepts the sample, call `state.history.record(&raw, …)`. Recording failures are logged and never fail the read. It runs under the same `source_commit` guard, so a result from a stale source generation is never recorded.

**What a record holds.** One JSON line per kept sample:

```json
{"t":1795473636,"acct":"<account key>","pool":"gemini","fh":{"u":5.0,"r":1795480262},"wk":{"u":89.0,"r":1795543999},"ar":null}
```

- `t` is the reading's `fetched_at`, not the wall clock when it was written.
- `fh`, `wk`, `ar`: used percent and reset time for 5H, weekly and (Antigravity only) Argon weekly. A missing window is `null`.
- `pool` is Antigravity's effective pool after `resolve_pool` (so with agy 1.5.9 an Argon model records as `gemini`). Other providers omit it.
- `acct` is the same account identity the burn-rate state already stores, so an account switch starts a new series rather than joining two accounts' numbers.
- Claude records its captured window (`notification_five_hour` when present), never the display-time expiry projection. The five-hour notification fix made the same distinction.

**Change-only with a heartbeat.** Polls run about once a minute per provider, but most readings repeat. Write a line only when a used percent or a reset time changes, or when 30 minutes have passed since the last line. The heartbeat lets the charts tell "idle, nothing changed" apart from "the app was off or reads were failing." A stretch longer than two heartbeats with no line is drawn as a gap.

**Files and retention.**

- One file per provider namespace per calendar month: `%LOCALAPPDATA%\CodexUsageOverlay\history\<namespace>-YYYY-MM.jsonl`. The namespace follows `ConfigManager::provider_path`: empty for local Codex, `claude`, `dsh-<session>`, `antigravity`. Antigravity keeps one series for all pools and tags each line with `pool`.
- Append-only writes. A reader skips a truncated or unparsable last line, so a crash mid-write costs one sample.
- At startup and once a day, delete month files that end more than 90 days ago. Queries also filter to the last 90 days, so retention is exact on screen even though files are removed a month at a time.
- Expected size: a few hundred lines a day per active provider, about 120 bytes each, so roughly 3 to 5 MB per provider at 90 days.

**Setting.** A Common-tab toggle, **Record usage history** (on by default), and a **Clear history** button with a confirmation. Turning recording off stops writes but keeps existing files.

## 2. Query layer

Tauri commands, in `query.rs`:

- `history_providers()` lists the providers and namespaces that have history, with their first and last timestamps.
- `history_query(provider, namespace, range)` takes a range of 24h, 7d, 30d or 90d. It returns:
  - **Timeline series** for 5H, weekly and AR, downsampled to at most about 1,000 points per series. Each bucket keeps its min and max, and points next to a reset or a gap are always kept, so peaks and drops survive downsampling.
  - **Reset markers** and **gap ranges**.
  - **Daily consumption**: per local calendar day, the sum of positive changes in used percent within the same window, kept separately for 5H and weekly. A drop at a reset counts as zero, not as negative use. A change that spans midnight goes to the day of the later reading. A change that spans a gap is counted but flagged approximate.
  - **Window summaries**: one row per past 5H or weekly window, with start (`resets_at − duration`), reset time, peak used percent, whether it reached 100%, time from window start to limit, and percent left unused at reset.

**Window identity.** Group readings into windows by `resets_at`. Providers jitter `resets_at` by a few seconds between reads, so treat reset times within a tolerance (for example 120 s for 5H and 10 min for weekly) as the same window. Reuse the burn-rate tracker's segment logic if it can be shared, rather than writing a second rule.

**Caching.** Parse each provider's files once and keep them in memory. Append each new record to that cache when the recorder writes it. A 90-day query then needs no disk reads.

Local days use the machine's current time zone (`chrono::Local`, already a dependency). A day can be 23 or 25 hours long at DST changes.

## 3. History window

**Window.** Add a `history` window in [tauri.conf.json](../src-tauri/tauri.conf.json) (`index.html#history`, about 1000×680, hidden at start). It works like `settings`: closing hides it, it remembers its size and position, and an `open_history_window` command shows and focuses it.

**Entry points.** A **Usage history** item in [TrayMenuView.tsx](../src/components/TrayMenuView.tsx) next to Settings, and the same item in the overlay's context menu ([OverlayView.tsx](../src/components/OverlayView.tsx)).

**Layout.**

- Provider tabs across the top, showing only providers with history, and a range selector (24h / 7d / 30d / 90d).
- **Usage timeline**: used percent over time, one line each for 5H and weekly (plus AR for Antigravity). Each reset is a vertical marker and gaps are shaded. The provider's protection thresholds are dashed lines. Hovering shows the exact reading and its time.
- **Daily consumption**: one bar per day of weekly percent consumed, with 5H consumption as a second series. The 5H total can exceed 100% in a day, since several 5H windows fit in one day. The axis label says so.
- **Window summary**: a table of past windows, newest first, with peak, limit hit, time to limit and unused at reset. It can be filtered to 5H or weekly.
- **Empty state**: before any history exists, say that recording started on a given date and show how much has been collected.

**Charts.** Hand-written SVG components (`TimelineChart`, `DailyBars`, `WindowTable`), with no new chart library. The frontend dependencies are deliberately small, and these three chart shapes are simple. The existing Tailwind light and dark themes apply. Follow the `dataviz` skill for colours, axes and tooltips when building them.

**Live updates.** The window listens to the existing usage update event and appends the newest point without re-querying.

**Localization.** All new strings in `en-US`, `zh-CN`, `zh-Hant` and `ru-RU`. The existing key-parity test enforces this.

## 4. Delivery order

1. **Recorder and store** (sections 1, retention, and the setting). Ship this first as its own patch release. It changes nothing visible, but every day it is out is a day of history the charts will have at launch.
2. **Query layer** with full Rust tests against recorded fixtures.
3. **History window**: window plumbing, entry points, the three views, localization.
4. Docs (README in both languages, CHANGELOG, release notes) and a preview release, then stable.

## Tests

- **Recorder**: change-only writes, the 30-minute heartbeat, account switch, Antigravity pool tagging, Claude's raw window used instead of the projection, no write for a stale-generation result.
- **Store**: month rollover, 90-day pruning, a truncated last line, a missing or unwritable history folder.
- **Query**:
  - window grouping with `resets_at` jitter;
  - daily deltas across a reset, across midnight, across a gap, and on a DST day;
  - downsampling keeps extrema, reset points and gap edges;
  - window summary on a window that hit 100% and one that did not.
- **Frontend**: each chart renders fixture data, gaps and the empty state; range and provider switching; the tray and context menu items open the window; locale key parity.
- **Manual, on the installed build**: run a day with Codex and Claude active, and check the timeline against what the overlay showed. Check one window that hits the limit. Confirm the History window survives a display change like the overlay does.

## Risks and open points

- **Reset-time jitter** is the main correctness risk for window grouping and daily deltas. Test the tolerance against a few days of real recorded data before relying on the summary table.
- **DSH** can bind different sessions over time. Each session namespace is its own series. The UI shows them under one DSH tab with a session picker only if more than one has history.
- **Antigravity pool changes** inside a window mix pools in one series. Lines are tagged, so the timeline can break the line where the pool changes instead of joining unrelated percentages.
- **Clock changes** (manual time changes, sleep) can produce out-of-order `t`. The recorder drops a sample older than the last written one for the same series.
- **Privacy**: history stays local and is never uploaded. It stores the same account identifier the burn-rate state already keeps. Clear history removes it.
