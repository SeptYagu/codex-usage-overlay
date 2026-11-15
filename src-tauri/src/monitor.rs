//! Independent provider pipelines; display selection never owns their lifetime.
#[cfg(test)]
mod tests;
use crate::{
    codex::CodexUsage,
    commands::{AppServices, AppState, ProviderRuntime},
    config::{ConfigManager, OverlaySettings, Provider},
};
use serde::Serialize;
use std::sync::{
    atomic::Ordering,
    Arc, RwLock,
};
use tauri::AppHandle;

pub struct Registry {
    runtimes: RwLock<Vec<Arc<ProviderRuntime>>>,
    tasks: std::sync::Mutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotUsage {
    #[serde(flatten)]
    pub usage: CodexUsage,
    pub source_key: String,
    pub source_generation: u64,
    #[serde(flatten)]
    pub presentation: crate::presentation::Metadata,
}

pub(crate) fn snapshot_usage(
    state: &ProviderRuntime,
    settings: &OverlaySettings,
    usage: CodexUsage,
) -> SnapshotUsage {
    let cache = state.presentation.lock().unwrap_or_else(|p| p.into_inner());
    SnapshotUsage {
        usage: cache.current.clone().unwrap_or(usage),
        source_key: crate::config::source_key_for(state.provider, settings),
        source_generation: state.source_generation.load(Ordering::Acquire),
        presentation: cache.metadata.clone(),
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub provider: Provider,
    pub source_key: String,
    pub source_generation: u64,
    pub revision: u64,
    pub monitor_enabled: bool,
    pub available: bool,
    pub health: String,
    pub error: Option<String>,
    pub usage: Option<SnapshotUsage>,
}

fn runtime(
    services: &Arc<AppServices>,
    config_manager: &ConfigManager,
    settings: &OverlaySettings,
    provider: Provider,
    generation: u64,
    applied_settings_revision: u64,
) -> Arc<ProviderRuntime> {
    Arc::new(ProviderRuntime::new(
        services.clone(),
        config_manager.clone(),
        settings.clone(),
        provider,
        generation,
        applied_settings_revision,
    ))
}

impl Registry {
    pub fn new(
        services: &Arc<AppServices>,
        config_manager: &ConfigManager,
        settings: &OverlaySettings,
        applied_settings_revision: u64,
    ) -> Self {
        let runtimes = Provider::ALL
            .into_iter()
            .map(|provider| {
                runtime(
                    services,
                    config_manager,
                    settings,
                    provider,
                    0,
                    applied_settings_revision,
                )
            })
            .collect();
        Self {
            runtimes: RwLock::new(runtimes),
            tasks: Default::default(),
        }
    }

    pub fn get(&self, provider: Provider) -> Arc<ProviderRuntime> {
        self.runtimes.read().unwrap_or_else(|p| p.into_inner())[provider.index() as usize].clone()
    }
    pub fn all(&self) -> Vec<Arc<ProviderRuntime>> {
        self.runtimes
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub async fn synchronize(&self, owner: &Arc<AppState>, settings: &OverlaySettings) -> Vec<Provider> {
        let mut changed_providers = Vec::new();
        for provider in Provider::ALL {
            let previous = self.get(provider);
            let _commit = previous.source_commit.lock().await;
            let mut next = settings.clone();
            next.provider = provider;
            let old = previous.settings.lock().await.clone();
            let old_pref = old.preference(provider);
            let next_pref = next.preference(provider);
            let enabled = next_pref.monitor_enabled;
            let global_changed = {
                let mut g_old = serde_json::to_value(&old).ok();
                let mut g_next = serde_json::to_value(&next).ok();
                if let (Some(o), Some(n)) = (
                    g_old.as_mut().and_then(|v| v.as_object_mut()),
                    g_next.as_mut().and_then(|v| v.as_object_mut()),
                ) {
                    o.remove("providerPreferences");
                    n.remove("providerPreferences");
                }
                g_old != g_next
            };
            let changed = global_changed
                || old_pref != next_pref
                || previous.monitor_enabled.load(Ordering::Acquire) != enabled;
            if !changed {
                continue;
            }
            changed_providers.push(provider);
            let binding_changed =
                crate::config::source_key_for(provider, &old)
                    != crate::config::source_key_for(provider, &next);
            let message_changed = provider == Provider::Codex && (old.auto_pause_five_hour_message != next.auto_pause_five_hour_message || old.auto_pause_weekly_message != next.auto_pause_weekly_message);
            let urgent = message_changed || old_pref.refresh_interval_seconds != next_pref.refresh_interval_seconds || binding_changed
                || previous.monitor_enabled.load(Ordering::Acquire) != enabled
                || old.claude_live_polling != next.claude_live_polling
                || old.antigravity_live_polling != next.antigravity_live_polling
                || old_pref.protection_enabled != next_pref.protection_enabled
                || old_pref.five_hour_threshold_percent
                    != next_pref.five_hour_threshold_percent
                || old_pref.weekly_threshold_percent
                    != next_pref.weekly_threshold_percent;
            if binding_changed {
                let generation = previous.source_generation.fetch_add(1, Ordering::AcqRel) + 1;
                revoke(&previous).await;
                previous.revoked.store(true, Ordering::Release);
                let replacement = runtime(
                    &owner.services,
                    &owner.config_manager,
                    settings,
                    provider,
                    generation,
                    owner.settings_revision.load(Ordering::Acquire),
                );
                *replacement
                    .protection_status
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = previous
                    .protection_status
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                // Retain the process-wide per-account/cycle attempt ledger across
                // A -> B -> A. Only explicit re-pairing is allowed to reset it.
                if provider == Provider::Dsh {
                    *replacement
                        .dsh_control
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = previous
                        .dsh_control
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                }
                self.runtimes.write().unwrap_or_else(|p| p.into_inner())
                    [provider.index() as usize] = replacement;
            } else {
                let read_changed = previous.monitor_enabled.load(Ordering::Acquire) != enabled
                    || old.claude_live_polling != next.claude_live_polling
                    || old.antigravity_live_polling != next.antigravity_live_polling;
                if read_changed {
                    previous.source_generation.fetch_add(1, Ordering::AcqRel);
                    previous
                        .reader
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .writer_running = false;
                }
                *previous.settings.lock().await = next.clone();
                previous.monitor_enabled.store(enabled, Ordering::Release);
                previous.settings_revision.fetch_add(1, Ordering::AcqRel);
                previous.applied_settings_revision.store(owner.settings_revision.load(Ordering::Acquire), Ordering::Release);
                if message_changed || read_changed
                    || old_pref.protection_enabled != next_pref.protection_enabled
                    || old.dsh_session_ids != next.dsh_session_ids
                    || old_pref.five_hour_threshold_percent
                        != next_pref.five_hour_threshold_percent
                    || old_pref.weekly_threshold_percent
                        != next_pref.weekly_threshold_percent
                {
                    revoke(&previous).await;
                }
                if old.claude_live_polling != next.claude_live_polling {
                    *previous
                        .claude_live
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = Default::default();
                }
                if old.antigravity_live_polling != next.antigravity_live_polling
                    || (provider == Provider::Antigravity && old.show_credits != next.show_credits && next.show_credits)
                {
                    *previous
                        .antigravity_live
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = Default::default();
                }
                if old_pref.protection_enabled != next_pref.protection_enabled
                    && provider == Provider::Codex
                    && next_pref.protection_enabled
                {
                    crate::pause_control::probe_bridge_status(&previous);
                }
                if urgent {previous.polling.lock().unwrap_or_else(|p|p.into_inner()).recalculate(&next_pref,std::time::Instant::now());}
            }
            // Wake the old worker even when its runtime was replaced.
            if urgent {
                previous
                    .polling
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .request_now(std::time::Instant::now());
            }
            if urgent {previous.poll_wake.notify_one();}
        }
        owner.services.status_wake.notify_one();
        let claude = settings;
        if let Err(error) =
            crate::claude::install::write_guard_config(owner.config_manager.runtime_dir(), claude)
        {
            eprintln!("Could not update Claude guard preferences: {error}");
        }
        let antigravity = settings;
        if let Err(error) = crate::antigravity::install::write_guard_config(
            owner.config_manager.runtime_dir(),
            antigravity,
        ) {
            eprintln!("Could not update Antigravity guard preferences: {error}");
        }
        changed_providers
    }

    pub fn next(&self, current: Provider) -> Option<Provider> {
        let all = self.all();
        let count = Provider::ALL.len();
        (1..count)
            .map(|offset| Provider::ALL[(current.index() as usize + offset) % count])
            .find(|p| {
                *p != current
                    && all[p.index() as usize]
                        .monitor_enabled
                        .load(Ordering::Acquire)
                    && all[p.index() as usize].has_reading.load(Ordering::Acquire)
            })
    }

    pub async fn snapshot(&self, provider: Provider) -> Option<Snapshot> {
        let state = self.get(provider);
        snapshot_runtime(&state).await
    }
    pub async fn snapshots(&self) -> Vec<Snapshot> {
        let states = self.all();
        let Some(first) = states.first() else {
            return Vec::new();
        };
        let _snapshot = first.services.snapshot_gate.lock().await;
        let mut output = Vec::new();
        for state in &states {
            if let Some(snapshot) = build_snapshot_locked(state).await {
                output.push(snapshot);
            }
        }
        output
    }
    pub async fn publish_one<S: crate::tray::TrayPresentationSink>(&self, sink: &S, provider: Provider) {
        if let Some(snapshot) = self.snapshot(provider).await {
            sink.publish_snapshot(&snapshot);
        }
    }
    pub async fn publish<S: crate::tray::TrayPresentationSink>(&self, sink: &S) {
        for snapshot in self.snapshots().await {
            sink.publish_snapshot(&snapshot);
        }
    }
    pub fn track_task(&self, task:tauri::async_runtime::JoinHandle<()>) {self.tasks.lock().unwrap_or_else(|p|p.into_inner()).push(task);}
    pub fn shutdown(&self) {
        for state in self.all() {
            state.revoked.store(true, Ordering::Release);
            state.monitor_enabled.store(false, Ordering::Release);
            state.source_generation.fetch_add(1, Ordering::AcqRel);
            state
                .auto_pause
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .invalidate_pending();
            state
                .dsh_control
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .invalidate();
        }
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
        {
            task.abort();
        }
    }
}

async fn build_snapshot_locked(state: &ProviderRuntime) -> Option<Snapshot> {
    // A protection request may hold its commit gate through network I/O.
    // Immutable runtime identities let UI snapshots avoid that gate, so
    // one provider cannot hold up publication of every other provider.
    let generation = state.source_generation.load(Ordering::Acquire);
    if !state.is_current() {
        return None;
    }
    let settings = state.settings.lock().await.clone();
    let usage = state.last_usage.lock().await.clone();
    let enabled = state.monitor_enabled.load(Ordering::Acquire);
    let available = enabled && state.has_reading.load(Ordering::Acquire);
    let (phase, error) = {
        let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
        (reader.phase, reader.failure.as_ref().map(|e| e.to_string()))
    };
    let last_error = state
        .last_refresh_result
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|r| r.as_ref().err().cloned());
    let now = crate::codex::reader::unix_now();
    let stale = usage.as_ref().is_some_and(|u| {
        now.saturating_sub(u.fetched_at)
            > (settings.preference(state.provider).refresh_interval_seconds as i64 * 2).max(120)
    });
    let health = if !enabled {
        "disabled"
    } else if !available && (phase == "error" || phase == "idle") {
        "needsSetup"
    } else if phase == "error" {
        "failed"
    } else if stale || phase == "stale" {
        "stale"
    } else if matches!(phase, "initializing" | "reading" | "retrying") {
        "checking"
    } else if usage.is_some() {
        "current"
    } else {
        "checking"
    };
    if !state.is_current() || state.source_generation.load(Ordering::Acquire) != generation {
        return None;
    }
    let key = crate::config::source_key_for(state.provider, &settings);
    Some(Snapshot {
        provider: state.provider,
        source_key: key,
        source_generation: generation,
        revision: state.services.snapshot_revision.fetch_add(1, Ordering::AcqRel) + 1,
        monitor_enabled: enabled,
        available,
        health: health.into(),
        error: error.or(last_error),
        usage: usage.map(|usage| snapshot_usage(state, &settings, usage)),
    })
}

async fn snapshot_runtime(state: &ProviderRuntime) -> Option<Snapshot> {
    let _snapshot = state.services.snapshot_gate.lock().await;
    build_snapshot_locked(state).await
}

async fn revoke(state: &Arc<ProviderRuntime>) {
    state.pending_reset_fetches.lock().await.cancel_all();
    state
        .auto_pause
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .invalidate_pending();
    state
        .dsh_control
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .invalidate();
    crate::pause_control::clear_active_periods(state);
}

pub async fn publish_runtime<S: crate::tray::TrayPresentationSink>(sink: &S, state: &ProviderRuntime) {
    if let Some(snapshot) = snapshot_runtime(state).await {
        sink.publish_snapshot(&snapshot);
    }
}

pub fn start(app: &AppHandle, owner: &Arc<AppState>) {
    for provider in Provider::ALL {
        let app = app.clone();
        let weak = Arc::downgrade(owner);
        let task = tauri::async_runtime::spawn(async move {
            loop {
                let Some(owner) = weak.upgrade() else {
                    return;
                };
                let state = owner.monitors.get(provider);
                crate::polling::wait_until_due(&state).await;
                if !Arc::ptr_eq(&state, &owner.monitors.get(provider)) {
                    continue;
                }
                if state.monitor_enabled.load(Ordering::Acquire) {
                    crate::refresh_provider_background(&app, &state).await;
                } else {
                    let settings = state.settings.lock().await.clone();
                    let pref = settings.preference(state.provider);
                    {
                        let mut polling = state.polling.lock().unwrap_or_else(|p| p.into_inner());
                        polling.started(std::time::Instant::now());
                        polling.recalculate(&pref, std::time::Instant::now());
                    }
                    owner.monitors.publish_one(&app, provider).await;
                }
            }
        });
        owner.monitors.track_task(task);
    }
}

#[tauri::command]
pub async fn get_monitor_snapshots(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<Snapshot>, String> {
    Ok(state.monitors.snapshots().await)
}

#[tauri::command]
pub async fn cycle_display_provider(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<crate::commands::SettingsEnvelope, String> {
    crate::commands::apply_settings_patch(
        &app,
        state.inner(),
        serde_json::json!({"cycleProvider": true}),
        false,
    )
    .await
}
