use std::sync::atomic::Ordering;
use std::sync::Arc;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::codex::CodexUsage;
use crate::config::{OverlaySettings, Provider};
use crate::tray::TrayPresentationSink;

use super::{
    refresh_usage, validate_settings_patch, AppState, ProviderRuntime, SettingsEnvelope,
};

#[derive(Serialize)]
pub struct DshBridgeInstance { pub id: String, pub label: String, pub origin: String }

#[tauri::command]
pub async fn dsh_discover(state: State<'_, Arc<AppState>>) -> Result<Vec<DshBridgeInstance>, String> {
    let root = state.config_manager.runtime_dir().join("dsh-bridges");
    if !root.exists() { return Ok(Vec::new()); }
    let canonical = std::fs::canonicalize(&root).map_err(|_| "Could not read DSH bridge discovery")?;
    let entries = std::fs::read_dir(&root).map_err(|_| "Could not read DSH bridge discovery")?;
    let mut instances = Vec::new();
    for entry in entries.take(64).flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue; };
        if !crate::pause_control::looks_like_thread_id(id) { continue; }
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink())
            || !std::fs::canonicalize(&path).is_ok_and(|p| p.parent() == Some(canonical.as_path())) { continue; }
        let Ok(metadata) = std::fs::metadata(&path) else { continue; };
        if !metadata.is_file() || metadata.len() > 16384 { continue; }
        let Ok(bytes) = std::fs::read(&path) else { continue; };
        let Ok(record) = serde_json::from_slice::<Value>(&bytes) else { continue; };
        if record["schemaVersion"] != 1 || record["instanceId"].as_str() != Some(id) { continue; }
        let Some(origin) = record["origin"].as_str() else { continue; };
        let Ok(url) = reqwest::Url::parse(origin) else { continue; };
        if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") || url.port().is_none()
            || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() || url.path() != "/" { continue; }
        let label = record["profileLabel"].as_str().filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)).unwrap_or("DSH companion");
        instances.push(DshBridgeInstance { id: id.into(), label: label.into(), origin: url.origin().ascii_serialization() });
    }
    instances.sort_by(|a,b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    Ok(instances)
}

#[tauri::command]
pub async fn dsh_connection_status(
    state: State<'_, Arc<AppState>>,
) -> Result<crate::dsh::ConnectionStatus, String> {
    Ok(state.dsh_auto.status(&state.dsh))
}

#[tauri::command]
pub async fn dsh_auto_retry(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let _commit = state.settings_patch.lock().await;
    let settings = state.settings.lock().await.clone();
    if !dsh_monitor_enabled(&settings) { return Err("Enable DSH monitoring before retrying".into()); }
    let runtime = state.monitors.get(Provider::Dsh);
    let frozen = freeze_dsh_connection(&runtime).await;
    state.dsh_auto.retry(&state.dsh);
    runtime
        .dsh_control
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .invalidate();
    drop(frozen);
    synchronize_dsh_runtime(state.inner(), &settings).await;
    Ok(())
}

#[tauri::command]
pub async fn dsh_auto_setup(state: State<'_, Arc<AppState>>, app: AppHandle) -> Result<(), String> {
    crate::dsh::auto::setup(&app, state.inner(), true).await;
    Ok(())
}

#[tauri::command]
pub async fn dsh_bind_targets(
    account_id: String,
    session_ids: Vec<String>,
    expected_instance_id: String,
    expected_connection_revision: u64,
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<SettingsEnvelope, String> {
    bind_dsh_targets(
        &app,
        state.inner(),
        account_id,
        session_ids,
        expected_instance_id,
        expected_connection_revision,
    )
    .await
}

pub(crate) async fn bind_dsh_targets<S: TrayPresentationSink>(
    sink: &S,
    state: &Arc<AppState>,
    account_id: String,
    session_ids: Vec<String>,
    expected_instance_id: String,
    expected_connection_revision: u64,
) -> Result<SettingsEnvelope, String> {
    // Serialize the UI's connection expectation with the real source patch.
    let _commit = state.settings_patch.lock().await;
    let cached = state.dsh.cached_status();
    if !cached.connected
        || cached.instance_id.as_deref() != Some(expected_instance_id.as_str())
        || cached.connection_revision != expected_connection_revision
    {
        return Err("DSH connection changed; refresh before selecting targets".into());
    }
    let patch = serde_json::json!({"dshAccountId":account_id,"dshSessionIds":session_ids});
    let changes = validate_settings_patch(&patch, false)?;
    let status = state.dsh.status().await;
    let mut settings = state.settings.lock().await;
    if !dsh_monitor_enabled(&settings)
        || settings.dsh_instance_id != expected_instance_id
        || !status.connected
        || status.connection_revision != expected_connection_revision
        || (!account_id.is_empty()
            && !status
                .accounts
                .iter()
                .any(|a| a.active && a.id == account_id))
        || (account_id.is_empty() && !session_ids.is_empty())
        || session_ids.iter().any(|id| {
            !status
                .sessions
                .iter()
                .any(|s| s.id == *id && s.provider == "openai-codex")
        })
    {
        return Err("Select the current DSH account and existing Codex sessions".into());
    }
    let unique: std::collections::HashSet<_> = session_ids.iter().collect();
    if unique.len() != session_ids.len() {
        return Err("Duplicate DSH target binding".into());
    }
    let mut next = settings.clone();
    changes.apply_to(&mut next);
    state.config_manager.save_settings_checked(&next)?;
    let source_changed = !crate::config::same_source_for(Provider::Dsh, &settings, &next);
    *settings = next.clone();
    drop(settings);
    state.dsh_auto.source_changed();
    if source_changed { synchronize_dsh_runtime(state, &next).await; }
    let runtime = state.monitors.get(Provider::Dsh);
    runtime.dsh_control.lock().unwrap_or_else(|p| p.into_inner()).invalidate();
    let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
    let envelope = SettingsEnvelope::new(state, revision, next);
    if let Some(app) = sink.app_handle() {
        let _ = app.emit("settings_updated", &envelope);
    }
    runtime
        .polling
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .request_now(std::time::Instant::now());
    runtime.poll_wake.notify_one();
    crate::tray::repaint_tray(sink, state).await;
    Ok(envelope)
}

#[tauri::command]
pub async fn dsh_connect(
    bridge_id: Option<String>,
    connection_url: Option<String>,
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<crate::dsh::ConnectionStatus, String> {
    if bridge_id.is_some() == connection_url.is_some() {
        return Err("Select exactly one DSH connection method".into());
    }
    if bridge_id.as_deref().is_some_and(|id| !crate::pause_control::looks_like_thread_id(id)) { return Err("Invalid DSH bridge identity".into()); }
    let epoch = state.dsh_auto.suspend(&state.dsh);
    let runtime = state.monitors.get(Provider::Dsh);
    runtime
        .dsh_control
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .invalidate();
    let generation = runtime.source_generation.load(Ordering::Acquire);
    let prepared = if let Some(id) = bridge_id {
        if !crate::pause_control::looks_like_thread_id(&id) {
            return Err("Invalid DSH bridge identity".into());
        }
        let directory = state.config_manager.runtime_dir().join("dsh-bridges");
        state
            .dsh
            .prepare_bridge_in_directory(&directory.join(format!("{id}.json")), &directory)
            .await?
    } else {
        state
            .dsh
            .preparation(connection_url.as_deref().unwrap_or_default())
            .await?
    };
    let _commit = state.settings_patch.lock().await;
    let source_commit = runtime.source_commit.lock().await;
    let mut settings = state.settings.lock().await;
    let mut installation = state.dsh.installation().await;
    let status = prepared.status().clone();
    let mut next = settings.clone();
    next.dsh_base_url = status.origin.clone().unwrap_or_default();
    next.dsh_instance_id = status.instance_id.clone().unwrap_or_default();
    next.dsh_profile_id.clear();
    next.dsh_account_id.clear();
    next.dsh_session_ids.clear();
    state.dsh_auto.with_operation(epoch, || {
        if runtime.source_generation.load(Ordering::Acquire) != generation || !runtime.is_current()
            || !installation.accepts(&prepared)
        {
            return Err("DSH connection changed during pairing".into());
        }
        state.config_manager.save_settings_checked(&next)?;
        invalidate_dsh_runtime(&runtime);
        installation.install(prepared);
        *settings = next.clone();
        Ok(())
    })?;
    drop(installation);
    drop(settings);
    runtime.dsh_control.lock().unwrap_or_else(|p| p.into_inner()).reset();
    drop(source_commit);
    publish_dsh_binding(&app, &state, next).await;
    state.dsh_auto.manual_phase(epoch, "connected-unbound");
    Ok(state.dsh_auto.status(&state.dsh))
}

#[tauri::command]
pub async fn dsh_disconnect(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<crate::dsh::ConnectionStatus, String> {
    disconnect_dsh(&app, state.inner()).await
}

pub(crate) async fn disconnect_dsh<S: TrayPresentationSink>(sink: &S, state: &Arc<AppState>) -> Result<crate::dsh::ConnectionStatus, String> {
    // Revoke before waiting on any source/network lock. Old handshakes cannot
    // install after this point and the background worker stays suspended.
    state.dsh_auto.suspend(&state.dsh);
    let _commit = state.settings_patch.lock().await;
    let runtime = state.monitors.get(Provider::Dsh);
    let frozen = freeze_dsh_connection(&runtime).await;
    let next = {
        let mut settings = state.settings.lock().await;
        let mut next = settings.clone();
        next.dsh_base_url.clear();
        next.dsh_instance_id.clear();
        next.dsh_account_id.clear();
        next.dsh_session_ids.clear();
        let changed = !crate::config::same_source_for(Provider::Dsh, &settings, &next);
        if changed {
            state.config_manager.save_settings_checked(&next)?;
            *settings = next.clone();
        }
        next
    };
    drop(frozen);
    publish_dsh_binding(sink, state, next).await;
    let manager = state.dsh.clone();
    tauri::async_runtime::spawn(async move {
        manager.clear_revoked().await;
    });
    Ok(state.dsh_auto.status(&state.dsh))
}

// Caller has durably saved settings and holds settings_patch. Only explicit
// re-pair clears the at-most-once ledger, never automatic retries/disconnect.
async fn publish_dsh_binding<S: TrayPresentationSink>(
    sink: &S,
    state: &Arc<AppState>,
    next: OverlaySettings,
) {
    synchronize_dsh_runtime(state, &next).await;
    let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(app) = sink.app_handle() {
        let _ = app.emit("settings_updated", SettingsEnvelope::new(state, revision, next));
    }
    crate::tray::repaint_tray(sink, state).await;
}

pub(crate) fn dsh_monitor_enabled(settings: &OverlaySettings) -> bool {
    settings.preference(Provider::Dsh).monitor_enabled
}

pub(crate) fn invalidate_dsh_runtime(runtime: &ProviderRuntime) {
    runtime.monitor_enabled.store(false, Ordering::Release);
    runtime.source_generation.fetch_add(1, Ordering::AcqRel);
    runtime.dsh_control.lock().unwrap_or_else(|p| p.into_inner()).invalidate();
}

pub(crate) async fn synchronize_dsh_runtime(state: &Arc<AppState>, settings: &OverlaySettings) {
    state.monitors.synchronize(state, settings).await;
    let runtime = state.monitors.get(Provider::Dsh);
    runtime.polling.lock().unwrap_or_else(|p| p.into_inner()).request_now(std::time::Instant::now());
    runtime.poll_wake.notify_one();
}

// The shared transport must not change while an old provider action can dispatch.
// Leave the runtime frozen after releasing the gate; synchronize restores the
// configured monitoring preference only after the new binding is installed.
pub(crate) async fn freeze_dsh_connection(runtime: &Arc<ProviderRuntime>) -> tokio::sync::MutexGuard<'_, ()> {
    let guard = runtime.source_commit.lock().await;
    invalidate_dsh_runtime(runtime);
    guard
}


/// The auto-pause status the settings window reads.
///
/// It reports whether a bridge candidate answered the `list_threads`
/// handshake and how many pause requests were positively confirmed. It never
/// claims a delivery that the bridge did not confirm.
#[tauri::command]
pub async fn get_auto_pause_status(
    provider: Option<Provider>,
    state: State<'_, Arc<AppState>>,
) -> Result<crate::pause_control::AutoPauseStatus, String> {
    let provider = provider.unwrap_or_else(||crate::display::effective(&state));
    let runtime = state.monitors.get(provider);
    Ok(crate::pause_control::status::snapshot(&runtime).await)
}

#[tauri::command]
pub async fn resume_chat(
    thread_id: Option<String>,
    provider: Option<Provider>,
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<String, String> {
    let provider = provider.unwrap_or_else(||crate::display::effective(&state));
    let runtime = state.monitors.get(provider);
    let state = &runtime;
    if state.provider != Provider::Codex { return Err("Resume is only supported for the local Codex source".into()); }
    refresh_usage(&app, state).await?;
    // A newer refresh may finish while this caller is being resumed. Capture
    // matching cached values/timing under the refresh guard, then release it
    // before bridge I/O. The dispatch guard rejects subsequent observations.
    let (usage, settings, settings_revision, observation) = {
        let _refresh = state.refresh_guard.lock().await;
        let usage = state.last_usage.lock().await.clone().ok_or_else(|| "quotaStale".to_string())?;
        let settings = state.settings.lock().await;
        let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
        (usage, settings.clone(), state.settings_revision.load(Ordering::Acquire),
            reader.observation.filter(|o| reader.failure.is_none() && o.action_eligible()))
    };
    resume_quota_gate(&settings, &usage, observation.is_some())?;
    let target_thread_id = if let Some(tid) = thread_id {
        tid
    } else {
        let controller = state.auto_pause.lock().unwrap_or_else(|p| p.into_inner());
        controller
            .paused_threads()
            .into_iter()
            .next()
            .ok_or_else(|| "noResumePlanAvailable".to_string())?
    };
    let prompt = crate::pause_control::resume_message_for_language(&settings.language);
    let transport = {
        let controller = state.auto_pause.lock().unwrap();
        controller.transport()
    };
    let outcome = crate::pause_control::send_resume_to_thread(
        transport.as_ref(),
        &target_thread_id,
        prompt,
        || {
            let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
            state.settings_revision.load(Ordering::Acquire) == settings_revision
                && reader.failure.is_none()
                && reader.observation.is_some_and(|o| o.action_eligible() && observation.is_some_and(|original| original.quota_started == o.quota_started))
        },
    )
    .await;
    if outcome.is_ok() {
        state
            .auto_pause
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove_paused_thread(&target_thread_id);
    }
    crate::pause_control::status::publish(&app, state).await;
    state.status_wake.notify_one();
    outcome
}

fn resume_quota_gate(settings: &OverlaySettings, usage: &CodexUsage, fresh: bool) -> Result<(), String> {
    if settings.provider != Provider::Codex { return Err("Resume is only supported for the local Codex source".into()); }
    let (Some(five), Some(week)) = (usage.five_hour_remaining_percent, usage.week_remaining_percent) else { return Err("quotaStale".into()); };
    if !fresh { return Err("quotaStale".into()); }
    let codex = settings.preference(Provider::Codex);
    if five <= codex.five_hour_threshold_percent || week <= codex.weekly_threshold_percent { return Err("quotaStillLow".into()); }
    Ok(())
}

#[tauri::command]
pub async fn cancel_resume_plans(provider: Option<Provider>, state: State<'_, Arc<AppState>>, app: AppHandle) -> Result<(), String> {
    let provider = provider.unwrap_or_else(||crate::display::effective(&state));
    let runtime = state.monitors.get(provider);
    let state = &runtime;
    if state.provider != Provider::Codex { return Err("Resume is only supported for the local Codex source".into()); }
    state
        .auto_pause
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear_paused_threads();
    crate::pause_control::status::publish(&app, state).await;
    state.status_wake.notify_one();
    Ok(())
}


#[tauri::command]
pub async fn claude_connection_status(state: State<'_, Arc<AppState>>) -> Result<crate::claude::install::ClaudeConnectionStatus, String> {
    let settings_path = crate::claude::install::claude_settings_path();
    let bridge = crate::claude::install::bridge_path();
    let mut status = crate::claude::install::status(settings_path.as_deref(), state.config_manager.runtime_dir(), bridge.as_deref());
    let runtime = state.monitors.get(Provider::Claude);
    let settings = runtime.settings.lock().await.clone();
    // A failed live read falls back to the stored reading, which clears
    // the read failure; the live failure is what needs attention then.
    let live_failure = settings.claude_live_polling
        .then(|| runtime.claude_live.lock().unwrap_or_else(|p| p.into_inner()).failure.clone())
        .flatten();
    let failure = live_failure.or_else(|| runtime.reader.lock().unwrap_or_else(|p| p.into_inner()).failure.clone());
    status.read_failure = failure.as_ref().map(|f| f.kind);
    status.read_http_status = failure.and_then(|f| f.http_status);
    Ok(status)
}

#[tauri::command]
pub async fn claude_connect(state: State<'_, Arc<AppState>>) -> Result<crate::claude::install::ClaudeConnectionStatus, String> {
    let settings_path = crate::claude::install::claude_settings_path().ok_or("Could not locate the Claude Code settings folder")?;
    let bridge = crate::claude::install::bridge_path().ok_or("Could not locate the overlay executable")?;
    crate::claude::install::connect(&settings_path, state.config_manager.runtime_dir(), &bridge)?;
    let runtime = state.monitors.get(Provider::Claude);
    runtime.polling.lock().unwrap_or_else(|p| p.into_inner()).request_now(std::time::Instant::now());
    runtime.poll_wake.notify_one();
    claude_connection_status(state).await
}

#[tauri::command]
pub async fn claude_disconnect(state: State<'_, Arc<AppState>>) -> Result<crate::claude::install::ClaudeConnectionStatus, String> {
    let settings_path = crate::claude::install::claude_settings_path().ok_or("Could not locate the Claude Code settings folder")?;
    crate::claude::install::disconnect(&settings_path, state.config_manager.runtime_dir())?;
    claude_connection_status(state).await
}

#[tauri::command]
pub async fn antigravity_connection_status(state: State<'_, Arc<AppState>>) -> Result<crate::antigravity::install::AntigravityConnectionStatus, String> {
    let settings_path = crate::antigravity::install::cli_settings_path();
    let hooks_path = crate::antigravity::install::hooks_path();
    let bridge = crate::antigravity::install::bridge_path();
    let runtime = state.monitors.get(Provider::Antigravity);
    let settings = runtime.settings.lock().await.clone();
    let mut status = crate::antigravity::install::status(
        settings_path.as_deref(),
        hooks_path.as_deref(),
        state.config_manager.runtime_dir(),
        bridge.as_deref(),
        settings.antigravity_pool,
    );
    let live_failure = settings
        .antigravity_live_polling
        .then(|| {
            runtime
                .antigravity_live
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .failure
                .clone()
        })
        .flatten();
    let failure = live_failure.or_else(|| {
        runtime
            .reader
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .failure
            .clone()
    });
    status.read_failure = failure.as_ref().map(|f| f.kind);
    status.read_http_status = failure.and_then(|f| f.http_status);
    Ok(status)
}

#[tauri::command]
pub async fn antigravity_connect(state: State<'_, Arc<AppState>>) -> Result<crate::antigravity::install::AntigravityConnectionStatus, String> {
    let settings_path = crate::antigravity::install::cli_settings_path().ok_or("Could not locate the Antigravity CLI settings folder")?;
    let hooks_path = crate::antigravity::install::hooks_path().ok_or("Could not locate the Antigravity hooks folder")?;
    let bridge = crate::antigravity::install::bridge_path().ok_or("Could not locate the overlay executable")?;
    crate::antigravity::install::connect(&settings_path, &hooks_path, state.config_manager.runtime_dir(), &bridge)?;
    let runtime = state.monitors.get(Provider::Antigravity);
    {
        let mut live = runtime.antigravity_live.lock().unwrap_or_else(|p| p.into_inner());
        if live.failure.is_some() || live.answer.is_none() {
            *live = Default::default();
        }
    }
    runtime.polling.lock().unwrap_or_else(|p| p.into_inner()).request_now(std::time::Instant::now());
    runtime.poll_wake.notify_one();
    antigravity_connection_status(state).await
}

#[tauri::command]
pub async fn antigravity_disconnect(state: State<'_, Arc<AppState>>) -> Result<crate::antigravity::install::AntigravityConnectionStatus, String> {
    let settings_path = crate::antigravity::install::cli_settings_path().ok_or("Could not locate the Antigravity CLI settings folder")?;
    let hooks_path = crate::antigravity::install::hooks_path().ok_or("Could not locate the Antigravity hooks folder")?;
    crate::antigravity::install::disconnect(&settings_path, &hooks_path, state.config_manager.runtime_dir())?;
    antigravity_connection_status(state).await
}


/// The auto-pause wiring tests.
///
/// They drive the *production* refresh path — `refresh_usage` — and observe the
/// bridge through the injectable transport, so they prove that the successful
/// sample really reaches the scheduler and not just the pure decision
/// functions.
#[cfg(test)]
mod auto_pause_tests {
    use super::*;
    use crate::codex::reader::{self, FailureKind, ReadEvent, ReadFailure, ReadStage};
    use crate::codex::{QuotaSnapshot, RawCodexUsage};
    use crate::commands::{
        apply_settings_patch, merge_settings_patch, observe_read_event, UsageSource,
    };
    use crate::config::ConfigManager;
    use crate::pause_control::testing::ScriptedTransport;
    use crate::pause_control::BridgeConnection;
    use crate::pause_control::BridgeDiscovery;
    use std::sync::{atomic::AtomicU64, Arc, Mutex as StdMutex};
    use std::time::Duration;

    const FETCHED_AT: i64 = 1_700_000_000;
    const CALLER_THREAD: &str = "123e4567-e89b-12d3-a456-426614174000";
    const PIPE: &str = "codex-app-tools-123e4567-e89b-12d3-a456-426614174000";
    const SECOND_PIPE: &str = "codex-app-tools-223e4567-e89b-12d3-a456-426614174000";

    /// A tray sink that records nothing and needs no event loop.
    struct OfflineSink {
        outcome: StdMutex<Option<Result<RawCodexUsage, String>>>,
        reads: AtomicU64,
    }

    impl OfflineSink {
        fn with_usage(usage: RawCodexUsage) -> Self {
            Self {
                outcome: StdMutex::new(Some(Ok(usage))),
                reads: AtomicU64::new(0),
            }
        }

        fn failing(message: &str) -> Self {
            Self {
                outcome: StdMutex::new(Some(Err(message.to_string()))),
                reads: AtomicU64::new(0),
            }
        }
    }

    impl TrayPresentationSink for OfflineSink {
        fn present(&self, _presentation: crate::tray::TrayPresentation) {}
        fn app_handle(&self) -> Option<&AppHandle> {
            None
        }
    }

    impl UsageSource for OfflineSink {
        async fn read_usage(&self, _: &ProviderRuntime) -> Result<RawCodexUsage, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.outcome
                .lock()
                .unwrap()
                .clone()
                .expect("the scripted outcome must be present")
        }
    }

    /// A sample whose 5-hour window is below the threshold.
    fn low_quota_usage(account_id: Option<&str>) -> RawCodexUsage {
        quota_usage_at(account_id, FETCHED_AT, FETCHED_AT + 3_600)
    }

    /// A sample whose 5-hour window is below the threshold, taken at an
    /// explicit moment and belonging to an explicit reset cycle.
    ///
    /// The weekly window stays far above its threshold, so only the 5-hour
    /// quota triggers.
    fn quota_usage_at(
        account_id: Option<&str>,
        fetched_at: i64,
        five_hour_resets_at: i64,
    ) -> RawCodexUsage {
        RawCodexUsage {
            account_id: account_id.map(ToOwned::to_owned),
            five_hour: QuotaSnapshot {
                used_percent: Some(97.0),
                resets_at: Some(five_hour_resets_at),
                window_duration_mins: Some(300),
            },
            weekly: QuotaSnapshot {
                used_percent: Some(50.0),
                resets_at: Some(fetched_at + 86_400),
                window_duration_mins: Some(10_080),
            },
            credits_display: "—".to_string(),
            credits_balance: None,
            has_credits: false,
            fetched_at,
            antigravity_weekly: None,
            notification_five_hour: None,
        }
    }

    fn temp_state(tag: &str) -> (Arc<AppState>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-auto-pause-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (
            Arc::new(AppState::for_test(ConfigManager::with_runtime_dir(dir.clone()))),
            dir,
        )
    }

    /// Builds a state whose auto-pause path is switched on and wired to a
    /// scripted bridge.
    async fn armed_state_with_owner(
        tag: &str,
    ) -> (
        Arc<AppState>,
        Arc<ProviderRuntime>,
        Arc<ScriptedTransport>,
        std::path::PathBuf,
    ) {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-auto-pause-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let config = ConfigManager::with_runtime_dir(dir.clone());
        let mut settings = config.load_settings();
        let codex = settings.preference_mut(Provider::Codex);
        codex.protection_enabled = true;
        codex.five_hour_threshold_percent = 5;
        codex.weekly_threshold_percent = 2;
        settings.language = "zh-CN".to_string();
        // Loading filled the messages from the default "auto" language (the
        // OS locale); reseed them as a first load under zh-CN would, so the
        // tests do not depend on the machine's locale.
        settings.auto_pause_five_hour_message = crate::pause_message::default_pause("zh-CN");
        settings.auto_pause_weekly_message = crate::pause_message::default_pause("zh-CN");
        let owner = Arc::new(AppState::new(config, settings));
        let state = owner.monitors.get(Provider::Codex);

        let transport = Arc::new(ScriptedTransport::new());
        {
            let mut controller = state.auto_pause.lock().unwrap();
            controller.set_transport(transport.clone());
            controller.set_discovery(BridgeDiscovery {
                explicit_pipe_path: None,
                pipe_names: vec![PIPE.to_string()],
                caller_thread_ids: vec![CALLER_THREAD.to_string()],
            });
            controller.set_worker_spawner(Arc::new(|state| {
                tokio::spawn(async move {
                    crate::pause_control::run_delivery_worker(state).await;
                });
            }));
            controller.set_probe_spawner(Arc::new(|state| {
                tokio::spawn(async move {
                    crate::pause_control::probe_bridge(&state).await;
                });
            }));
        }
        (owner, state, transport, dir)
    }

    async fn armed_state(
        tag: &str,
    ) -> (
        Arc<ProviderRuntime>,
        Arc<ScriptedTransport>,
        std::path::PathBuf,
    ) {
        let (_owner, state, transport, dir) = armed_state_with_owner(tag).await;
        (state, transport, dir)
    }

    /// Waits until the scripted bridge recorded at least one call.
    async fn wait_for_calls(transport: &ScriptedTransport) {
        for _ in 0..200 {
            if !transport.calls().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits until the scripted bridge recorded the expected number of
    /// delivered prompts.
    async fn wait_for_prompts(transport: &ScriptedTransport, expected: usize) {
        for _ in 0..200 {
            if transport.sent_prompts().len() >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits until the scripted bridge recorded the expected number of
    /// handshakes.
    async fn wait_for_handshakes(transport: &ScriptedTransport, expected: usize) {
        for _ in 0..200 {
            let count = transport
                .calls()
                .iter()
                .filter(|call| call.tool_name == "list_threads")
                .count();
            if count >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// A sample whose 5-hour window is far above the threshold.
    ///
    /// The weekly window stays far above its threshold as well, so the sample
    /// never schedules a delivery.
    fn comfortable_quota_usage(account_id: Option<&str>) -> RawCodexUsage {
        let mut usage = quota_usage_at(account_id, FETCHED_AT, FETCHED_AT + 3_600);
        usage.five_hour.used_percent = Some(20.0);
        usage
    }

    /// Cooperative delivery must never ask for an external continuation timer.
    const EXPECTED_PAUSE_MESSAGE: &str = "\u{6682}\u{505C}";

    #[tokio::test]
    async fn custom_weekly_message_wins_and_editing_does_not_clear_cycle_dedup() {
        let (state,transport,_dir)=armed_state("custom-weekly").await;
        {let mut settings=state.settings.lock().await;settings.auto_pause_five_hour_message="FIVE CUSTOM".into();settings.auto_pause_weekly_message="WEEKLY CUSTOM".into();}
        let mut raw=low_quota_usage(Some("acct-1"));raw.weekly.used_percent=Some(100.);
        let sink=OfflineSink::with_usage(raw);refresh_usage(&sink,&state).await.unwrap();wait_for_prompts(&transport,1).await;
        assert_eq!(transport.sent_prompts(),vec!["WEEKLY CUSTOM"]);
        state.settings.lock().await.auto_pause_weekly_message="EDITED".into();
        refresh_usage(&sink,&state).await.unwrap();wait_for_pause_worker(&state).await;
        assert_eq!(transport.sent_prompts(),vec!["WEEKLY CUSTOM"]);
    }
    #[tokio::test]
    async fn fixed_custom_five_hour_text_survives_interface_language_changes() {
        let (state,transport,_dir)=armed_state("custom-fixed").await;
        {let mut settings=state.settings.lock().await;settings.language="en-US".into();settings.auto_pause_five_hour_message="保留我的固定文字".into();}
        let sink=OfflineSink::with_usage(low_quota_usage(Some("acct-1")));
        refresh_usage(&sink,&state).await.unwrap();wait_for_prompts(&transport,1).await;
        assert_eq!(transport.sent_prompts(),vec!["保留我的固定文字"]);
    }


    #[tokio::test]
    async fn pause_message_uses_english_when_selected() {
        let (state, transport, _dir) = armed_state("wiring-locale-en").await;
        {
            let mut settings = state.settings.lock().await;
            settings.language = "en-US".to_string();
        }
        {let mut settings=state.settings.lock().await;settings.auto_pause_five_hour_message="Pause".into();settings.auto_pause_weekly_message="Pause".into();}
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));
        refresh_usage(&sink, &state).await.expect("the refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        let prompts = transport.sent_prompts();
        assert_eq!(prompts, vec!["Pause"]);
    }

    #[tokio::test]
    async fn pause_message_uses_traditional_chinese_when_selected() {
        let (state, transport, _dir) = armed_state("wiring-locale-hant").await;
        {
            let mut settings = state.settings.lock().await;
            settings.language = "zh-Hant".to_string();
        }
        {let mut settings=state.settings.lock().await;settings.auto_pause_five_hour_message="暫停".into();settings.auto_pause_weekly_message="暫停".into();}
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));
        refresh_usage(&sink, &state).await.expect("the refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        let prompts = transport.sent_prompts();
        assert_eq!(prompts, vec!["\u{66AB}\u{505C}"]);
    }

    #[tokio::test]
    async fn a_low_claude_quota_never_reaches_the_codex_bridge_or_the_codex_learner() {
        let (owner, codex, transport, dir) = armed_state_with_owner("wiring-claude").await;
        let claude = owner.monitors.get(Provider::Claude);
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        let usage = refresh_usage(&sink, &claude).await.expect("the refresh must succeed");
        assert_eq!(usage.five_hour_remaining_percent, Some(3));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(transport.calls().is_empty(), "no handshake and no pause request");
        assert!(dir.join("claude-burn-rate-state.json").is_file());
        assert!(!dir.join("burn-rate-state.json").exists());

        assert!(codex.last_usage.lock().await.is_none());
        assert_eq!(codex.config_manager.burn_rate_state_path(), dir.join("burn-rate-state.json"));
        refresh_usage(&sink, &codex).await.expect("the refresh must succeed");
        wait_for_prompts(&transport, 1).await;

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_successful_refresh_requests_pause_without_an_external_resume_timer() {
        let (state, transport, dir) = armed_state("wiring-success").await;
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        let usage = refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        assert_eq!(usage.five_hour_remaining_percent, Some(3));
        assert_eq!(sink.reads.load(Ordering::SeqCst), 1);

        wait_for_prompts(&transport, 1).await;

        // The handshake ran against the discovered pipe with the caller
        // identity, and the pause request carried the fixed message.
        let calls = transport.calls();
        let handshake = calls
            .iter()
            .find(|call| call.tool_name == "list_threads")
            .expect("the handshake must run");
        assert_eq!(handshake.pipe, PIPE);
        assert_eq!(handshake.caller_thread_id, CALLER_THREAD);
        assert_eq!(handshake.arguments["limit"], 50);

        let sent = calls
            .iter()
            .find(|call| call.tool_name == "send_message_to_thread")
            .expect("the pause request must be sent");
        assert_eq!(sent.arguments["threadId"], "session-1");
        assert_eq!(sent.arguments["hostId"], "host-1");
        assert_eq!(sent.arguments["prompt"], EXPECTED_PAUSE_MESSAGE);

        // A confirmed delivery is reported, not guessed.
        let status = state
            .auto_pause
            .lock()
            .unwrap()
            .status(true);
        assert_eq!(status.confirmed_count, 1);
        assert!(status.last_confirmed_at.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_failed_refresh_never_reaches_the_bridge() {
        let (state, transport, dir) = armed_state("wiring-error").await;
        let sink = OfflineSink::failing("usage read failed");

        let result = refresh_usage(&sink, &state).await;
        assert!(result.is_err());
        assert!(transport.calls().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_switched_off_feature_never_reaches_the_bridge() {
        let (state, transport, dir) = armed_state("wiring-disabled").await;

        // Control: with the feature on, the bridge is reached. Without it the
        // assertion below would also hold on a build where the whole feature
        // is dead, so the control is what makes the negative case meaningful.
        // It uses another account, so the de-duplication ledger cannot mask
        // the effect that is being tested.
        {
            let control = OfflineSink::with_usage(low_quota_usage(Some("acct-control")));
            refresh_usage(&control, &state)
                .await
                .expect("the control refresh must succeed");
            wait_for_prompts(&transport, 1).await;
            assert_eq!(
                transport.sent_prompts().len(),
                1,
                "the control must reach the bridge"
            );
        }

        {
            let mut settings = state.settings.lock().await;
            settings.preference_mut(Provider::Codex).protection_enabled = false;
        }
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-2")));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "the switched-off feature must stay silent"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_sample_without_an_account_never_reaches_the_bridge() {
        let (state, transport, dir) = armed_state("wiring-no-account").await;

        // Control: a sample that carries an account identity does reach the
        // bridge, so the negative case below is about the missing identity and
        // not about a dead feature.
        {
            let control = OfflineSink::with_usage(low_quota_usage(Some("acct-control")));
            refresh_usage(&control, &state)
                .await
                .expect("the control refresh must succeed");
            wait_for_prompts(&transport, 1).await;
            assert_eq!(
                transport.sent_prompts().len(),
                1,
                "the control must reach the bridge"
            );
        }

        let sink = OfflineSink::with_usage(low_quota_usage(None));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "a sample without an account identity must not be delivered"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_repeated_refresh_does_not_send_a_second_message() {
        let (state, transport, dir) = armed_state("wiring-repeat").await;
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        assert_eq!(transport.sent_prompts().len(), 1);

        refresh_usage(&sink, &state)
            .await
            .expect("the second refresh must succeed");
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "a confirmed delivery must not be repeated"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_new_quota_cycle_can_prompt_the_same_active_session_again() {
        let (state, transport, dir) = armed_state("wiring-new-cycle").await;

        // The first cycle: the 5-hour window is nearly spent and resets in one
        // hour.
        let first = OfflineSink::with_usage(quota_usage_at(
            Some("acct-1"),
            FETCHED_AT,
            FETCHED_AT + 3_600,
        ));
        refresh_usage(&first, &state)
            .await
            .expect("the first refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "the first cycle must be announced"
        );

        // The second cycle: the window already reset, so the record of the
        // finished cycle is stale. The session never left the active list, but
        // a new reset cycle is a new quota identity and must be announced
        // again.
        let second = OfflineSink::with_usage(quota_usage_at(
            Some("acct-1"),
            FETCHED_AT + 3_601,
            FETCHED_AT + 3_601 + 5_400,
        ));
        refresh_usage(&second, &state)
            .await
            .expect("the second refresh must succeed");
        wait_for_prompts(&transport, 2).await;

        assert_eq!(
            transport.sent_prompts().len(),
            2,
            "a new reset cycle must be announced again"
        );

        // Cycle identity belongs to the ledger; both requests are pause-only.
        let prompts = transport.sent_prompts();
        assert_eq!(prompts, vec![crate::pause_control::PAUSE_MESSAGE_ZH_CN; 2]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_unconfirmed_delivery_is_retried_on_the_next_refresh() {
        let (state, transport, dir) = armed_state("wiring-retry").await;
        transport.set_send_success(false);
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        assert_eq!(transport.sent_prompts().len(), 1);

        // A refused request is never recorded as a confirmed delivery.
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.confirmed_count, 0);
        assert_eq!(status.last_confirmed_at, None);

        // The next refresh tries again.
        transport.set_send_success(true);
        refresh_usage(&sink, &state)
            .await
            .expect("the second refresh must succeed");
        for _ in 0..200 {
            if transport.sent_prompts().len() > 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(transport.sent_prompts().len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn two_triggered_windows_produce_a_single_plain_pause() {
        let (state, transport, dir) = armed_state("wiring-both").await;
        let mut usage = low_quota_usage(Some("acct-1"));
        usage.weekly.used_percent = Some(99.0);
        let sink = OfflineSink::with_usage(usage);

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        wait_for_prompts(&transport, 1).await;

        let prompts = transport.sent_prompts();
        assert_eq!(prompts.len(), 1, "both windows share one message");
        assert_eq!(prompts[0], crate::pause_control::PAUSE_MESSAGE_ZH_CN);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An unconfirmed handshake never reaches the sending step: the pipe name
    /// alone is not evidence that a bridge is usable.
    #[tokio::test]
    async fn an_unconfirmed_handshake_never_sends_anything() {
        let (state, transport, dir) = armed_state("wiring-handshake").await;
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: false,
            text: None,
        });
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        wait_for_calls(&transport).await;

        let calls = transport.calls();
        assert!(calls.iter().any(|call| call.tool_name == "list_threads"));
        assert!(
            !calls.iter().any(|call| call.tool_name == "send_message_to_thread"),
            "an unconfirmed handshake must not send"
        );
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.confirmed_count, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The bridge status follows reality. A bridge that answered once and
    /// then stopped must not keep claiming to be available, because the
    /// status is the only place the user can see that the feature is
    /// connected.
    #[tokio::test]
    async fn the_bridge_status_follows_the_last_handshake() {
        let (state, transport, dir) = armed_state("wiring-connection").await;
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the first refresh must succeed");
        wait_for_handshakes(&transport, 1).await;
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.connection, BridgeConnection::Available);

        // The bridge stops answering.
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: false,
            text: None,
        });
        refresh_usage(&sink, &state)
            .await
            .expect("the second refresh must succeed");
        wait_for_handshakes(&transport, 2).await;

        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(
            status.connection,
            BridgeConnection::Unavailable,
            "a bridge that stopped answering must not report itself available"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session that the listing reports twice, once with a host and once
    /// without, is one session. It must receive one pause request, not two.
    #[tokio::test]
    async fn a_session_reported_with_two_host_ids_is_targeted_once() {
        let (state, transport, dir) = armed_state("wiring-host-id").await;
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(
                serde_json::json!({
                    "pinnedThreads": [],
                    "threads": [
                        {
                            "id": "session-1",
                            "kind": "codex",
                            "status": "active",
                        },
                        {
                            "id": "session-1",
                            "kind": "codex",
                            "status": "active",
                            "hostId": "host-1",
                        },
                    ],
                })
                .to_string(),
            ),
        });
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        let sends = transport
            .calls()
            .into_iter()
            .filter(|call| call.tool_name == "send_message_to_thread")
            .collect::<Vec<_>>();
        assert_eq!(
            sends.len(),
            1,
            "one session must receive one pause request, not one per host"
        );
        assert_eq!(
            sends[0].arguments["hostId"], "host-1",
            "the record that carries the host id must be used"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A 5-hour trigger whose reset time is missing still pauses, because the
    /// plain pause needs no cycle identity. The missing identity also means
    /// the request cannot be scoped to a cycle, so the session is not asked
    /// to pause a second time while it stays in the same active period.
    #[tokio::test]
    async fn a_missing_reset_time_pauses_once_per_active_period() {
        let (state, transport, dir) = armed_state("wiring-no-reset").await;
        let mut usage = low_quota_usage(Some("acct-1"));
        usage.five_hour.resets_at = None;
        let sink = OfflineSink::with_usage(usage);

        refresh_usage(&sink, &state)
            .await
            .expect("the first refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        assert_eq!(
            transport.sent_prompts(),
            vec![crate::pause_control::PAUSE_MESSAGE_ZH_CN.to_string()],
            "a missing reset time falls back to the plain pause"
        );

        refresh_usage(&sink, &state)
            .await
            .expect("the second refresh must succeed");
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "the same active period must not be paused twice"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session that leaves the active list and comes back starts a new
    /// active period; confirmed quota-cycle messages must nevertheless stay deduplicated.
    #[tokio::test]
    async fn a_returning_session_is_not_prompted_twice_in_the_same_quota_cycle() {
        let (state, transport, dir) = armed_state("wiring-return").await;
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the first refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        assert_eq!(transport.sent_prompts().len(), 1);

        // The session is no longer active.
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(serde_json::json!({ "pinnedThreads": [], "threads": [] }).to_string()),
        });
        refresh_usage(&sink, &state)
            .await
            .expect("the second refresh must succeed");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "an idle session is not a target"
        );

        // The session comes back and is active again.
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(
                serde_json::json!({
                    "pinnedThreads": [],
                    "threads": [{
                        "id": "session-1",
                        "kind": "codex",
                        "status": "active",
                        "hostId": "host-1",
                    }],
                })
                .to_string(),
            ),
        });
        refresh_usage(&sink, &state)
            .await
            .expect("the third refresh must succeed");
        wait_for_pause_worker(&state).await;

        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "a new active period must not duplicate a confirmed quota-cycle message"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Without a reset time there is no quota cycle to deduplicate against, so
    /// the ledger falls back to active periods: a returning session is paused
    /// again instead of being skipped until the app restarts.
    #[tokio::test]
    async fn a_returning_session_without_a_reset_time_is_paused_again() {
        let (state, transport, dir) = armed_state("wiring-return-no-reset").await;
        let mut usage = low_quota_usage(Some("acct-1"));
        usage.five_hour.resets_at = None;
        let sink = OfflineSink::with_usage(usage);
        let active = || crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(serde_json::json!({
                "pinnedThreads": [],
                "threads": [{ "id": "session-1", "kind": "codex", "status": "active", "hostId": "host-1" }],
            }).to_string()),
        };

        refresh_usage(&sink, &state).await.expect("the first refresh must succeed");
        wait_for_prompts(&transport, 1).await;

        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(serde_json::json!({ "pinnedThreads": [], "threads": [] }).to_string()),
        });
        refresh_usage(&sink, &state).await.expect("the second refresh must succeed");
        wait_for_pause_worker(&state).await;
        assert_eq!(transport.sent_prompts().len(), 1, "an idle session is not a target");

        transport.set_list_threads(active());
        refresh_usage(&sink, &state).await.expect("the third refresh must succeed");
        wait_for_prompts(&transport, 2).await;
        assert_eq!(
            transport.sent_prompts().len(),
            2,
            "a new active period without a reset time must be paused again"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A transport failure is an unconfirmed delivery, not a success.
    #[tokio::test]
    async fn a_transport_failure_is_not_a_confirmed_delivery() {
        let (state, transport, dir) = armed_state("wiring-transport-error").await;
        transport.set_send_error("the pipe vanished");
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        wait_for_prompts(&transport, 1).await;

        assert_eq!(transport.sent_prompts().len(), 1, "the attempt was made");
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.confirmed_count, 0);
        assert_eq!(status.last_confirmed_at, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A batch that outlives the deadline is cancelled and releases its
    /// reservation; after submission, an unknown result must suppress blind retries.
    ///
    /// The handshake answers at once and only the send hangs. That way a
    /// reservation really exists when the deadline cuts the batch off, and
    /// the measured clock is the batch rather than the refresh.
    #[tokio::test]
    async fn a_timed_out_submitted_batch_releases_reservations_without_blind_resend() {
        let (state, transport, dir) = armed_state("wiring-deadline").await;
        transport.set_send_delay_ms(3_000);
        {
            let mut controller = state.auto_pause.lock().unwrap();
            controller.set_delivery_deadline(Duration::from_millis(150));
        }
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");

        // The batch is cut off at the deadline.
        let started = std::time::Instant::now();
        for _ in 0..300 {
            if !state.auto_pause.lock().unwrap().running_flag() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "the whole batch must end at the deadline, not after the call"
        );
        assert!(
            !state.auto_pause.lock().unwrap().running_flag(),
            "the worker must not stay alive after the deadline"
        );

        // The request was really attempted, so a reservation existed.
        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "the send must have been attempted"
        );

        // The cancelled batch released its reservation and confirmed nothing.
        {
            let controller = state.auto_pause.lock().unwrap();
            assert_eq!(
                controller.pending_len(),
                0,
                "the cancelled batch must release its reservation"
            );
            assert_eq!(
                controller.sent_len(),
                0,
                "a cancelled batch is not a confirmed delivery"
            );
        }
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.confirmed_count, 0);
        assert!(status.targets.iter().any(|target|target.phase=="unknown" && target.reason.as_deref()==Some("submissionUnknown")));

        // A submitted request with no acknowledgement must not be blindly resent.
        transport.set_send_delay_ms(0);
        refresh_usage(&sink, &state)
            .await
            .expect("the second refresh must succeed");
        wait_for_pause_worker(&state).await;
        assert_eq!(transport.sent_prompts().len(), 1);
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.confirmed_count, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The typed `SettingsPatch` is the only way into provider protection settings; legacy flat keys are rejected.
    #[test]
    fn the_auto_pause_settings_are_on_the_closed_allowlist() {
        let switch = serde_json::json!({"providerPreferences": {"codex": {"protectionEnabled": true}}});
        assert!(validate_settings_patch(&switch, false).is_ok());
        assert!(validate_settings_patch(&serde_json::json!({"providerPreferences": {"codex": {"protectionEnabled": false}}}), false).is_ok());
        assert!(validate_settings_patch(&serde_json::json!({"providerPreferences": {"codex": {"protectionEnabled": "yes"}}}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"providerPreferences": {"codex": {"protectionEnabled": 1}}}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"providerPreferences": {"codex": {"protectionEnabled": null}}}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"autoPauseOnLowQuota": true}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"autoPauseFiveHourThresholdPercent": 5}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"autoPauseWeeklyThresholdPercent": 2}), false).is_err());

        for value in [1u64, 5, 15] {
            let patch = serde_json::json!({ "providerPreferences": { "codex": { "fiveHourThresholdPercent": value } } });
            assert!(validate_settings_patch(&patch, false).is_ok(), "{value}");
        }
        for value in [1u64, 2, 10] {
            let patch = serde_json::json!({ "providerPreferences": { "codex": { "weeklyThresholdPercent": value } } });
            assert!(validate_settings_patch(&patch, false).is_ok(), "{value}");
        }
        for value in [0u64, 16, 100] {
            let patch = serde_json::json!({ "providerPreferences": { "codex": { "fiveHourThresholdPercent": value } } });
            assert!(validate_settings_patch(&patch, false).is_err(), "{value}");
        }
        for value in [0u64, 11, 100] {
            let patch = serde_json::json!({ "providerPreferences": { "codex": { "weeklyThresholdPercent": value } } });
            assert!(validate_settings_patch(&patch, false).is_err(), "{value}");
        }
        // Fractions, strings and nulls are rejected.
        assert!(validate_settings_patch(
            &serde_json::json!({ "providerPreferences": { "codex": { "fiveHourThresholdPercent": 5.5 } } }),
            false
        )
        .is_err());
        assert!(validate_settings_patch(
            &serde_json::json!({ "providerPreferences": { "codex": { "weeklyThresholdPercent": "2" } } }),
            false
        )
        .is_err());
        assert!(validate_settings_patch(
            &serde_json::json!({ "providerPreferences": { "codex": { "weeklyThresholdPercent": null } } }),
            false
        )
        .is_err());
    }

    /// The whole patch is rejected when one field is out of range, and the
    /// three values really reach the persisted settings.
    #[test]
    fn an_auto_pause_patch_survives_the_persistence_chain() {
        let (state, dir) = temp_state("patch-persistence");

        let patch = serde_json::json!({
            "providerPreferences": {
                "codex": {
                    "protectionEnabled": true,
                    "fiveHourThresholdPercent": 9,
                    "weeklyThresholdPercent": 4,
                }
            }
        });
        validate_settings_patch(&patch, false).expect("the allowlist must accept it");

        let mut settings = state.config_manager.load_settings();
        assert!(!settings.preference(Provider::Codex).protection_enabled, "the default must be off");
        merge_settings_patch(&mut settings, &patch).expect("the patch must merge");
        assert!(settings.preference(Provider::Codex).protection_enabled);
        assert_eq!(settings.preference(Provider::Codex).five_hour_threshold_percent, 9);
        assert_eq!(settings.preference(Provider::Codex).weekly_threshold_percent, 4);
        state
            .config_manager
            .save_settings_checked(&settings)
            .expect("the write must succeed");

        let reloaded = ConfigManager::with_runtime_dir(dir.clone()).load_settings();
        assert!(reloaded.preference(Provider::Codex).protection_enabled);
        assert_eq!(reloaded.preference(Provider::Codex).five_hour_threshold_percent, 9);
        assert_eq!(reloaded.preference(Provider::Codex).weekly_threshold_percent, 4);

        let raw = std::fs::read_to_string(state.config_manager.settings_path()).unwrap();
        assert!(raw.contains("\"protectionEnabled\": true"), "wire key missing");
        assert!(
            raw.contains("\"fiveHourThresholdPercent\": 9"),
            "wire key missing"
        );
        assert!(
            raw.contains("\"weeklyThresholdPercent\": 4"),
            "wire key missing"
        );

        // One invalid field rejects the whole patch.
        let rejected = serde_json::json!({
            "providerPreferences": { "codex": { "protectionEnabled": true, "weeklyThresholdPercent": 99 } },
        });
        assert!(validate_settings_patch(&rejected, false).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pipe that answered without an active session is not probed again
    /// for the remaining caller identities.
    ///
    /// The candidate list crosses every pipe with every caller identity, so
    /// the same pipe is offered up to 32 times. The pipe answered and
    /// reported no active session, so the remaining identities would repeat
    /// the same read-only call.
    #[tokio::test]
    async fn no_active_session_short_circuits_remaining_caller_threads_for_same_pipe() {
        let (state, transport, dir) = armed_state("short-circuit").await;
        // The bridge answers, but no session is active.
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(serde_json::json!({ "pinnedThreads": [], "threads": [] }).to_string()),
        });
        // Two pipes are offered three caller identities each.
        {
            let mut controller = state.auto_pause.lock().unwrap();
            controller.set_discovery(BridgeDiscovery {
                explicit_pipe_path: None,
                pipe_names: vec![PIPE.to_string(), SECOND_PIPE.to_string()],
                caller_thread_ids: vec![
                    CALLER_THREAD.to_string(),
                    "223e4567-e89b-12d3-a456-426614174000".to_string(),
                    "323e4567-e89b-12d3-a456-426614174000".to_string(),
                ],
            });
        }

        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-short")));
        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        wait_for_handshakes(&transport, 1).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        let handshakes = transport
            .calls()
            .iter()
            .filter(|call| call.tool_name == "list_threads")
            .count();
        assert_eq!(
            handshakes, 2,
            "an exhausted pipe is probed once, not once per caller identity"
        );
        // The bridge answered, so the connection is available, and an idle
        // bridge never receives a pause request.
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.connection, BridgeConnection::Available);
        assert!(transport.sent_prompts().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session that comes back while the feature was off starts a new
    /// active period, so it is paused again.
    ///
    /// The quota cycle never changed, so only a fresh period identity can
    /// lift the de-duplication of the earlier pause request.
    #[tokio::test]
    async fn toggling_auto_pause_preserves_confirmed_quota_cycle_dedup() {
        let (state, transport, dir) = armed_state("period-reset").await;
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the first refresh must succeed");
        wait_for_prompts(&transport, 1).await;
        assert_eq!(transport.sent_prompts().len(), 1);

        // The switch is turned off and the session leaves the active list.
        {
            let mut settings = state.settings.lock().await;
            settings.preference_mut(Provider::Codex).protection_enabled = false;
        }
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(serde_json::json!({ "pinnedThreads": [], "threads": [] }).to_string()),
        });
        refresh_usage(&sink, &state)
            .await
            .expect("the second refresh must succeed");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(transport.sent_prompts().len(), 1);

        // The session is active again and the switch is on. The quota never
        // left its cycle, so only a fresh active period can be paused again.
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(
                serde_json::json!({
                    "pinnedThreads": [],
                    "threads": [{
                        "id": "session-1",
                        "kind": "codex",
                        "status": "active",
                        "hostId": "host-1",
                    }],
                })
                .to_string(),
            ),
        });
        {
            let mut settings = state.settings.lock().await;
            settings.preference_mut(Provider::Codex).protection_enabled = true;
        }
        refresh_usage(&sink, &state)
            .await
            .expect("the third refresh must succeed");
        wait_for_pause_worker(&state).await;

        assert_eq!(
            transport.sent_prompts().len(),
            1,
            "toggling protection must not duplicate a confirmed quota-cycle message"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The bridge status is answered by a read-only probe while the quota is
    /// comfortable.
    ///
    /// Without the probe the status stays unknown, because the handshake only
    /// happens when a delivery is scheduled.
    #[tokio::test]
    async fn enabling_auto_pause_above_threshold_probes_bridge_status_when_unknown() {
        let (state, transport, dir) = armed_state("probe-status").await;
        // The quota is comfortable, so no delivery is scheduled.
        let sink = OfflineSink::with_usage(comfortable_quota_usage(Some("acct-1")));

        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        wait_for_handshakes(&transport, 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(
            status.connection,
            BridgeConnection::Available,
            "the probe replaces the unknown status"
        );
        // The probe is read-only.
        assert!(transport.sent_prompts().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Turning the switch on answers the bridge status right away.
    #[tokio::test]
    async fn turning_the_switch_on_probes_the_bridge_status() {
        let (owner, state, transport, dir) = armed_state_with_owner("probe-enable").await;
        // The switch starts off.
        {
            let mut settings = owner.settings.lock().await;
            settings.preference_mut(Provider::Codex).protection_enabled = false;
            state.settings.lock().await.preference_mut(Provider::Codex).protection_enabled = false;
        }
        let sink = OfflineSink::with_usage(comfortable_quota_usage(Some("acct-1")));

        apply_settings_patch(
            &sink,
            &owner,
            serde_json::json!({ "providerPreferences": { "codex": { "protectionEnabled": true } } }),
            false,
        )
        .await
        .expect("the patch must apply");

        wait_for_handshakes(&transport, 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.connection, BridgeConnection::Available);
        assert!(transport.sent_prompts().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn wait_for_pause_worker(state: &Arc<ProviderRuntime>) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while state.auto_pause.lock().unwrap().running_flag() {
                tokio::task::yield_now().await;
            }
        }).await.expect("the pause worker must finish");
    }

    #[tokio::test]
    async fn ineligible_updates_cancel_a_low_quota_batch_waiting_for_the_bridge() {
        for change in ["recovered", "missing-account", "failed-read", "disabled", "threshold"] {
            let (owner, state, transport, dir) = armed_state_with_owner(&format!("cancel-{change}")).await;
            let gate = transport.hold_handshakes();
            let low = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));
            refresh_usage(&low, &state).await.unwrap();
            wait_for_handshakes(&transport, 1).await;
            assert_eq!(transport.calls().len(), 1, "the handshake must be waiting");

            match change {
                "recovered" => {
                    refresh_usage(&OfflineSink::with_usage(comfortable_quota_usage(Some("acct-1"))), &state)
                        .await.unwrap();
                }
                "missing-account" => {
                    refresh_usage(&OfflineSink::with_usage(low_quota_usage(None)), &state)
                        .await.unwrap();
                }
                "failed-read" => {
                    assert!(refresh_usage(&OfflineSink::failing("offline"), &state).await.is_err());
                }
                "disabled" => {
                    apply_settings_patch(&low, &owner,
                        serde_json::json!({ "providerPreferences": { "codex": { "protectionEnabled": false } } }), false).await.unwrap();
                }
                "threshold" => {
                    apply_settings_patch(&low, &owner,
                        serde_json::json!({ "providerPreferences": { "codex": { "fiveHourThresholdPercent": 1 } } }), false).await.unwrap();
                }
                _ => unreachable!(),
            }
            gate.notify_one();
            wait_for_pause_worker(&state).await;
            assert!(transport.sent_prompts().is_empty(), "stale pause sent after {change}");
            assert_eq!(state.auto_pause.lock().unwrap().pending_len(), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[tokio::test]
    async fn an_attempt_failure_invalidates_delivery_before_the_inline_retry() {
        let (state, transport, dir) = armed_state("retry-invalidation").await;
        let gate = transport.hold_handshakes();
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));
        refresh_usage(&sink, &state).await.unwrap();
        wait_for_handshakes(&transport, 1).await;
        observe_read_event(&sink, &state, ReadEvent::Failed(ReadFailure::new(ReadStage::Quota, FailureKind::Transport))).await;
        observe_read_event(&sink, &state, ReadEvent::Retrying).await;
        gate.notify_one();
        wait_for_pause_worker(&state).await;
        assert!(transport.sent_prompts().is_empty());
        assert_eq!(crate::pause_control::status::snapshot(&state).await.reason.as_deref(), Some("quotaReadRetrying"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_samples_keep_only_the_remaining_action_lifetime() {
        let (state, transport, dir) = armed_state("slow-action-age").await;
        state.auto_pause.lock().unwrap().set_worker_spawner(Arc::new(|_| {}));
        let now = tokio::time::Instant::now();
        state.reader.lock().unwrap().observation = Some(reader::ObservationTiming { quota_started: now - Duration::from_millis(27_474), received: now });
        refresh_usage(&OfflineSink::with_usage(low_quota_usage(Some("acct-1"))), &state).await.unwrap();
        tokio::time::advance(Duration::from_millis(2_527)).await;
        crate::pause_control::run_delivery_worker(state.clone()).await;
        assert!(transport.calls().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn display_only_samples_update_the_account_but_cannot_schedule_actions() {
        let (state, transport, dir) = armed_state("display-only").await;
        let now = tokio::time::Instant::now();
        state.reader.lock().unwrap().observation = Some(reader::ObservationTiming { quota_started: now - Duration::from_secs(35), received: now });
        let sample = refresh_usage(&OfflineSink::with_usage(low_quota_usage(Some("new-account"))), &state).await.unwrap();
        assert_eq!(sample.five_hour_remaining_percent, Some(3));
        assert!(transport.calls().is_empty());
        assert!(!state.auto_pause.lock().unwrap().running_flag());
        assert_eq!(crate::pause_control::status::snapshot(&state).await.quota_health, "stale");
        assert_eq!(state.polling.lock().unwrap().effective_interval_seconds, 15);
        assert_eq!(resume_quota_gate(&OverlaySettings::default(), &sample, false), Err("quotaStale".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn recovery_removes_an_unsent_queued_pause_before_the_worker_starts() {
        let (state, transport, dir) = armed_state("cancel-queued").await;
        state.auto_pause.lock().unwrap().set_worker_spawner(Arc::new(|_| {}));
        refresh_usage(&OfflineSink::with_usage(low_quota_usage(Some("acct-1"))), &state)
            .await.unwrap();
        refresh_usage(&OfflineSink::with_usage(comfortable_quota_usage(Some("acct-1"))), &state)
            .await.unwrap();
        crate::pause_control::run_delivery_worker(state.clone()).await;
        assert!(transport.sent_prompts().is_empty());
        assert_eq!(state.auto_pause.lock().unwrap().sent_len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn disabling_during_one_send_prevents_sends_to_the_remaining_threads() {
        let (owner, state, transport, dir) = armed_state_with_owner("cancel-remaining").await;
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(serde_json::json!({ "threads": [
                { "id": "session-1", "kind": "codex", "status": "active" },
                { "id": "session-2", "kind": "codex", "status": "active" }
            ] }).to_string()),
        });
        let gate = transport.hold_sends();
        let sink = OfflineSink::with_usage(low_quota_usage(Some("acct-1")));
        refresh_usage(&sink, &state).await.unwrap();
        wait_for_prompts(&transport, 1).await;
        apply_settings_patch(&sink, &owner,
            serde_json::json!({ "providerPreferences": { "codex": { "protectionEnabled": false } } }), false).await.unwrap();
        gate.notify_one();
        wait_for_pause_worker(&state).await;
        assert_eq!(transport.sent_prompts().len(), 1, "only the already submitted request can complete");
        assert_eq!(state.auto_pause.lock().unwrap().sent_len(), 0,
            "an obsolete confirmation must not restore invalidated ledger entries");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_new_low_sample_preserves_an_already_submitted_send_confirmation() {
        let (state, transport, dir) = armed_state("confirm-newer-low").await;
        let gate = transport.hold_sends();
        refresh_usage(&OfflineSink::with_usage(low_quota_usage(Some("acct-1"))), &state)
            .await.unwrap();
        wait_for_prompts(&transport, 1).await;
        let next = quota_usage_at(Some("acct-1"), FETCHED_AT + 1, FETCHED_AT + 3_600);
        refresh_usage(&OfflineSink::with_usage(next), &state).await.unwrap();
        gate.notify_one();
        wait_for_pause_worker(&state).await;
        assert_eq!(transport.sent_prompts().len(), 1, "the accepted send must still de-duplicate");
        assert_eq!(state.auto_pause.lock().unwrap().sent_len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_slow_target_does_not_starve_later_chats_after_a_deadline() {
        let (state, transport, dir) = armed_state("fair-batches").await;
        transport.set_list_threads(crate::pause_control::ToolCallOutcome {
            success: true,
            text: Some(serde_json::json!({ "threads": [
                { "id": "session-1", "kind": "codex", "status": "active" },
                { "id": "session-2", "kind": "codex", "status": "active" }
            ] }).to_string()),
        });
        transport.set_send_delay_ms(1000);
        state.auto_pause.lock().unwrap().set_delivery_deadline(Duration::from_millis(50));
        refresh_usage(&OfflineSink::with_usage(low_quota_usage(Some("acct-1"))), &state).await.unwrap();
        wait_for_pause_worker(&state).await;
        transport.set_send_delay_ms(0);
        refresh_usage(&OfflineSink::with_usage(low_quota_usage(Some("acct-1"))), &state).await.unwrap();
        wait_for_pause_worker(&state).await;
        let sends: Vec<_> = transport.calls().into_iter()
            .filter(|call| call.tool_name == "send_message_to_thread").collect();
        assert_eq!(sends[0].arguments["threadId"], "session-1");
        assert_eq!(sends[1].arguments["threadId"], "session-2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_auto_resume_field_is_ignored_on_load_and_rejected_in_patches() {
        let legacy: OverlaySettings = serde_json::from_value(serde_json::json!({
            "autoResumeAfterQuotaReset": true
        }))
        .unwrap();
        assert!(!legacy.preference(Provider::Codex).protection_enabled);
        assert!(validate_settings_patch(
            &serde_json::json!({ "autoResumeAfterQuotaReset": true }),
            false
        )
        .is_err());
    }

    #[tokio::test]
    async fn paused_threads_stay_in_session_across_recovery_until_cleared() {
        let (state, _transport, dir) = armed_state("paused-threads-session").await;
        refresh_usage(&OfflineSink::with_usage(low_quota_usage(Some("acct-1"))), &state)
            .await
            .unwrap();
        wait_for_pause_worker(&state).await;

        let paused = state.auto_pause.lock().unwrap().paused_threads();
        assert_eq!(paused, vec!["session-1".to_string()]);
        let status = crate::pause_control::status::snapshot(&state).await;
        assert_eq!(status.paused_threads, vec!["session-1".to_string()]);

        // Quota recovery clears active periods while keeping in-session paused threads for manual resume.
        refresh_usage(&OfflineSink::with_usage(comfortable_quota_usage(Some("acct-1"))), &state)
            .await
            .unwrap();
        assert_eq!(
            state.auto_pause.lock().unwrap().paused_threads(),
            vec!["session-1".to_string()]
        );

        state.auto_pause.lock().unwrap().clear_paused_threads();
        assert!(crate::pause_control::status::snapshot(&state)
            .await
            .paused_threads
            .is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

