# Multi-provider monitoring implementation

Target: `v1.6.0-preview.3`. Implements the [monitoring/logo plan](2026-11-02-multi-provider-monitoring-plan.md), independent protection decision, persistent visibility request and selected-provider tray fix.

## Implementation

- `monitor.rs` owns independent provider pipelines and scheduled workers. Existing read/learner/protection logic is reused in source-specific `AppState` contexts; `AppServices` shares only window, DSH transport, audio and updater services. The displayed provider is a separate atomic selection. Source generations change for configuration invalidation, not display changes.
- Bound `ConfigManager` stores have immutable namespace values. Existing unprefixed Codex, `claude-` and DSH instance/account history filenames are preserved. Late data from an invalidated/replaced context cannot commit or repaint the current tray.
- Per-source refresh guards coalesce concurrent requests. Existing Codex read budgets, retries and Claude's live limiter remain. UI snapshot publication avoids protection commit gates, so a slow DSH pause cannot delay other providers' snapshots.
- Schema 2 stores per-provider monitoring/protection/threshold preferences. Migration is idempotent, preserves existing histories and carries protection enablement only to the previously selected provider. Legacy fields mirror the display preference when modern settings are written. Nested patches merge atomically; debounced slider patches preserve changes across providers.
- Protection and reset tracking are scoped to the provider. Claude guard configuration follows Claude's own setting. DSH rebindings preserve the process-wide per-account/cycle attempt ledger and monotonic status revisions; only explicit pairing resets attempts. Unsupported automatic resume stays disabled.
- React caches provider-scoped snapshots with generation/revision ordering. The logo button cycles the authoritative backend selection, supports keyboard activation and does not start dragging. Settings has separate display/configuration selectors and all new descriptions are translated into three languages.
- Tray updates are serialized with selection. Both icon pixels and tooltips use the displayed runtime; transient failures preserve its cached rings. Tests assert the expected pixels for Codex, Claude and DSH during switching.
- `dock-state.json` adds a backward-compatible `hidden` field. Manual visibility changes persist immediately. The main window is created invisible and shown only after restoring state; polling continues while hidden. A missing tray reveals the overlay for recovery.

## Validation

- Frontend: 161 tests; TypeScript/Vite production build.
- Rust: 422 main-library + 10 helper tests passed, 7 existing ignored. Added production-path regressions for concurrent providers, display switching during reads, independent protection/guard files, disabled-source cancellation, history/store isolation, account rebinding/status revisions, slow-action snapshot publication, authoritative cycling, tray pixels and persisted visibility.
- Companion: 15 security/transport tests and package verification. Seven release-packaging regressions.
- Browser: 634 existing display cases, 216 multi-provider interaction/settings cases and the existing 768-case percentage-grid matrix. Reports/screenshots remain under ignored `output/`.
- Native: isolated debug identifier and debug-only `CODEX_OVERLAY_TEST_RUNTIME_DIR` kept test state separate from installed settings. Protection, notifications, updates and autostart were off. Real Codex/Claude readings and both history files advanced while Codex remained displayed. A restart with saved `hidden: true` had no visible QA window and both source status files reported success. The owned QA process was stopped after recording evidence.

## Limits

Computer Use returned “Computer Use was not approved to use codex-usage-overlay” when capturing the native QA window. No native UI input was attempted after that denial. Logo mouse/keyboard interaction was checked in Edge; actual WebView2 interaction, mixed-monitor docking, installer smoke and real protected-session delivery remain unverified. No real chats were paused. DSH monitoring still requires explicit pairing/account configuration and is limited to one configured binding.
