//! Independent provider pipelines; display selection never owns their lifetime.
#[cfg(test)]
mod tests;
use crate::{
    commands::{AppState, UsageEnvelope},
    config::{OverlaySettings, Provider},
};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock,
};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

pub struct Registry {
    runtimes: RwLock<Vec<Arc<AppState>>>,
    revision: AtomicU64,
    snapshot_gate: Mutex<()>,
    tasks: std::sync::Mutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
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
    pub usage: Option<UsageEnvelope>,
}

pub fn initialize(owner: &Arc<AppState>, settings: &OverlaySettings) {
    let _ = owner.owner.set(Arc::downgrade(owner));
    let runtimes = Provider::ALL
        .into_iter()
        .map(|provider| runtime(owner, settings, provider, 0))
        .collect();
    let _ = owner.monitors.set(Registry {
        runtimes: RwLock::new(runtimes),
        revision: AtomicU64::new(0),
        snapshot_gate: Mutex::new(()),
        tasks: Default::default(),
    });
}

fn runtime(
    owner: &AppState,
    settings: &OverlaySettings,
    provider: Provider,
    generation: u64,
) -> Arc<AppState> {
    let scoped = settings.for_provider(provider);
    let state = AppState::with_services(
        owner.config_manager.for_source(&scoped),
        scoped,
        owner.services.clone(),
        Some(provider),
    );
    state.monitor_enabled.store(
        settings.preference(provider).monitor_enabled,
        Ordering::Release,
    );
    state.source_generation.store(generation, Ordering::Release);
    Arc::new(state)
}

impl Registry {
    pub fn get(&self, provider: Provider) -> Arc<AppState> {
        self.runtimes.read().unwrap_or_else(|p| p.into_inner())[provider.index() as usize].clone()
    }
    pub fn all(&self) -> Vec<Arc<AppState>> {
        self.runtimes
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub async fn synchronize(&self, owner: &Arc<AppState>, settings: &OverlaySettings) {
        {
            let _tray = owner.tray_gate.lock().unwrap_or_else(|p| p.into_inner());
            owner
                .displayed_provider
                .store(settings.provider.index(), Ordering::Release);
        }
        for provider in Provider::ALL {
            let previous = self.get(provider);
            let _commit = previous.settings_patch.lock().await;
            let next = settings.for_provider(provider);
            let old = previous.settings.lock().await.clone();
            let enabled = settings.preference(provider).monitor_enabled;
            let changed = serde_json::to_value(&old).ok() != serde_json::to_value(&next).ok()
                || previous.monitor_enabled.load(Ordering::Acquire) != enabled;
            if !changed {
                continue;
            }
            let binding_changed =
                crate::config::source_key(&old) != crate::config::source_key(&next);
            let urgent = old.refresh_interval_seconds != next.refresh_interval_seconds || binding_changed
                || previous.monitor_enabled.load(Ordering::Acquire) != enabled
                || old.claude_live_polling != next.claude_live_polling
                || old.auto_pause_on_low_quota != next.auto_pause_on_low_quota
                || old.auto_pause_five_hour_threshold_percent
                    != next.auto_pause_five_hour_threshold_percent
                || old.auto_pause_weekly_threshold_percent
                    != next.auto_pause_weekly_threshold_percent;
            if binding_changed {
                let generation = previous.source_generation.fetch_add(1, Ordering::AcqRel) + 1;
                revoke(&previous).await;
                let replacement = runtime(owner, settings, provider, generation);
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
                    || old.claude_live_polling != next.claude_live_polling;
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
                if read_changed
                    || old.auto_pause_on_low_quota != next.auto_pause_on_low_quota
                    || old.dsh_session_ids != next.dsh_session_ids
                    || old.auto_pause_five_hour_threshold_percent
                        != next.auto_pause_five_hour_threshold_percent
                    || old.auto_pause_weekly_threshold_percent
                        != next.auto_pause_weekly_threshold_percent
                {
                    revoke(&previous).await;
                }
                if old.claude_live_polling != next.claude_live_polling {
                    *previous
                        .claude_live
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = Default::default();
                }
                if old.auto_pause_on_low_quota != next.auto_pause_on_low_quota
                    && provider == Provider::Codex
                    && next.auto_pause_on_low_quota
                {
                    crate::pause_control::probe_bridge_status(&previous);
                }
                previous
                    .polling
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .recalculate(&next, std::time::Instant::now());
            }
            // Wake the old worker even when its runtime was replaced.
            if urgent {
                previous
                    .polling
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .request_now(std::time::Instant::now());
            }
            previous.poll_wake.notify_one();
        }
        let claude = settings.for_provider(Provider::Claude);
        if let Err(error) =
            crate::claude::install::write_guard_config(owner.config_manager.runtime_dir(), &claude)
        {
            eprintln!("Could not update Claude guard preferences: {error}");
        }
    }

    pub fn next(&self, current: Provider) -> Option<Provider> {
        let all = self.all();
        (1..=3)
            .map(|offset| Provider::ALL[(current.index() as usize + offset) % 3])
            .find(|p| {
                *p != current
                    && all[p.index() as usize]
                        .monitor_enabled
                        .load(Ordering::Acquire)
                    && all[p.index() as usize].has_reading.load(Ordering::Acquire)
            })
    }

    pub async fn snapshots(&self) -> Vec<Snapshot> {
        let _snapshot = self.snapshot_gate.lock().await;
        let mut output = Vec::new();
        for state in self.all() {
            // A protection request may hold its commit gate through network I/O.
            // Immutable runtime identities let UI snapshots avoid that gate, so
            // one provider cannot hold up publication of every other provider.
            let generation = state.source_generation.load(Ordering::Acquire);
            if !state.is_current() {
                continue;
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
                    > (settings.refresh_interval_seconds as i64 * 2).max(120)
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
            if !state.is_current() || state.source_generation.load(Ordering::Acquire) != generation { continue; }
            let key = crate::config::source_key(&settings);
            output.push(Snapshot {
                provider: settings.provider,
                source_key: key.clone(),
                source_generation: generation,
                revision: self.revision.fetch_add(1, Ordering::AcqRel) + 1,
                monitor_enabled: enabled,
                available,
                health: health.into(),
                error: error.or(last_error),
                usage: usage.map(|usage| UsageEnvelope { usage, source_key: key, source_generation: generation }),
            });
        }
        output
    }
    pub async fn publish(&self, app: &AppHandle) {
        for snapshot in self.snapshots().await {
            let _ = app.emit("monitor_updated", snapshot);
        }
    }
    pub fn shutdown(&self) {
        for state in self.all() {
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

async fn revoke(state: &Arc<AppState>) {
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

pub fn selected(owner: &Arc<AppState>, provider: Provider) -> Arc<AppState> {
    owner
        .monitors
        .get()
        .map(|r| r.get(provider))
        .unwrap_or_else(|| owner.clone())
}

pub fn start(app: &AppHandle, owner: &Arc<AppState>) {
    let Some(registry) = owner.monitors.get() else {
        return;
    };
    for provider in Provider::ALL {
        let app = app.clone();
        let weak = Arc::downgrade(owner);
        let task = tauri::async_runtime::spawn(async move {
            loop {
                let Some(owner) = weak.upgrade() else {
                    return;
                };
                let registry = owner.monitors.get().unwrap();
                let state = registry.get(provider);
                crate::polling::wait_until_due(&state).await;
                if !Arc::ptr_eq(&state, &registry.get(provider)) {
                    continue;
                }
                if state.monitor_enabled.load(Ordering::Acquire) {
                    crate::refresh_provider_background(&app, &state).await;
                } else {
                    let settings = state.settings.lock().await.clone();
                    let mut polling = state.polling.lock().unwrap_or_else(|p| p.into_inner());
                    polling.started(std::time::Instant::now());
                    polling.recalculate(&settings, std::time::Instant::now());
                }
                registry.publish(&app).await;
            }
        });
        registry
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(task);
    }
}

#[tauri::command]
pub async fn get_monitor_snapshots(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<Snapshot>, String> {
    Ok(match state.monitors.get() {
        Some(registry) => registry.snapshots().await,
        None => Vec::new(),
    })
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
