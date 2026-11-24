use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

use crate::codex::reader::{self, FailureKind, ReadDiagnostic, ReadEvent, ReadFailure, ReadStage};
use crate::codex::{CodexUsage, RawCodexUsage};
use crate::config::{OverlaySettings, Provider};
use crate::tray::{TrayPresentationSink, TrayReadStatus};

use super::{AppState, ProviderRuntime};

/// Inject only the usage fetch at the production refresh boundary. The guard,
/// acceptance, persistence, error and success paths remain the real pipeline.
pub(crate) trait UsageSource: TrayPresentationSink {
    fn read_usage(&self, state: &ProviderRuntime) -> impl std::future::Future<Output = Result<RawCodexUsage, String>> + Send;
}

impl UsageSource for AppHandle {
    async fn read_usage(&self, state: &ProviderRuntime) -> Result<RawCodexUsage, String> {
        let generation = state.source_generation.load(Ordering::Acquire);
        let settings = state.settings.lock().await.clone();
        match state.provider {
            Provider::Claude => return read_claude_usage(state, generation).await,
            Provider::Antigravity => return read_antigravity_usage(state, &settings, generation).await,
            Provider::Dsh => return read_dsh_usage(state, &settings, generation).await,
            Provider::Codex => {}
        }
        let mut client = state.client.lock().await;
        let started_at = reader::unix_now();
        let server_wait = state.polling.lock().unwrap_or_else(|p| p.into_inner()).server_wait(std::time::Instant::now());
        if let Some(wait) = server_wait {
            let mut failure = ReadFailure::new(ReadStage::Quota, FailureKind::Rpc);
            failure.http_status = Some(429);
            failure.retry_after_seconds = Some(wait.as_secs().saturating_add(1));
            let mut reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
            reader.failure = Some(failure.clone());
            reader.phase = "error";
            return Err(failure.to_string());
        }
        let mut observer = |event| async move {
            let finished = matches!(&event, ReadEvent::Finished(_));
            let _source_commit = state.source_commit.lock().await;
            if state.source_generation.load(Ordering::Acquire) != generation { return; }
            observe_read_event(self, state, event).await;
            if finished { queue_reader_history(self, state); }
        };
        let result = client.fetch_usage(&mut observer).await;
        let _source_commit = state.source_commit.lock().await;
        if state.source_generation.load(Ordering::Acquire) != generation { return Err("Usage source changed during the read".into()); }
        let mut status = state.reader.lock().unwrap_or_else(|p| p.into_inner());
        match result {
            Ok(value) => {
                status.observation = Some(value.timing);
                status.failure = None;
                status.phase = if status.display_only() { "stale" } else { "success" };
                Ok(value.usage)
            }
            Err(error) => {
                status.phase = "error";
                status.failure = Some(error.clone());
                if error.stage == ReadStage::Refresh {
                    let mut diagnostic = ReadDiagnostic::new(status.attempt.max(1));
                    diagnostic.started_at = started_at;
                    diagnostic.failure = Some(error.clone());
                    diagnostic.elapsed_ms = reader::REFRESH_BUDGET.as_millis() as u64;
                    status.record(diagnostic);
                    drop(status);
                    queue_reader_history(self, state);
                }
                Err(error.to_string())
            }
        }
    }
}

/// The Claude read has no attempts, retries or process diagnostics; the Codex
/// read history stays Codex-only. Without live reading it is the local file
/// the statusline bridge writes. With live reading the endpoint answers, and
/// the answer is also stored in that file so the guard hooks see it. While the
/// endpoint fails or asks to wait, that file still holds the last reading; it
/// is shown with its age and Settings reports the live failure.
async fn read_claude_usage(state: &ProviderRuntime, generation: u64) -> Result<RawCodexUsage, String> {
    let now = reader::unix_now();
    let dir = state.config_manager.runtime_dir();
    let result = if state.settings.lock().await.claude_live_polling {
        match read_claude_live(state, now).await {
            Ok(snapshot) => Ok(crate::claude::source::sample(&snapshot, now)),
            Err(failure) => crate::claude::source::read_usage(dir, now).map_err(|_| failure),
        }
    } else {
        crate::claude::source::read_usage(dir, now)
    };
    let _source_commit = state.source_commit.lock().await;
    if state.source_generation.load(Ordering::Acquire) != generation { return Err("Usage source changed during the read".into()); }
    let mut status = state.reader.lock().unwrap_or_else(|p| p.into_inner());
    status.observation = None;
    match result {
        Ok(sample) => {
            status.failure = None;
            status.phase = if sample.stale { "stale" } else { "success" };
            Ok(sample.usage)
        }
        Err(error) => {
            status.phase = "error";
            status.failure = Some(error.clone());
            Err(crate::claude::source::describe(&error))
        }
    }
}

async fn read_antigravity_usage(state: &ProviderRuntime, settings: &OverlaySettings, generation: u64) -> Result<RawCodexUsage, String> {
    let now = reader::unix_now();
    let dir = state.config_manager.runtime_dir();
    let result = if settings.antigravity_live_polling {
        match read_antigravity_live(state, settings.show_credits, now).await {
            Ok(snapshot) => Ok(crate::antigravity::source::sample(&snapshot, settings.antigravity_pool, now)),
            Err(failure) => crate::antigravity::source::read_usage(dir, settings.antigravity_pool, now).map_err(|_| failure),
        }
    } else {
        crate::antigravity::source::read_usage(dir, settings.antigravity_pool, now)
    };
    let _source_commit = state.source_commit.lock().await;
    if state.source_generation.load(Ordering::Acquire) != generation {
        return Err("Usage source changed during the read".into());
    }
    let mut status = state.reader.lock().unwrap_or_else(|p| p.into_inner());
    status.observation = None;
    match result {
        Ok(sample) => {
            status.failure = None;
            status.phase = if sample.stale { "stale" } else { "success" };
            Ok(sample.usage)
        }
        Err(error) => {
            status.phase = "error";
            status.failure = Some(error.clone());
            Err(crate::antigravity::source::describe(&error))
        }
    }
}

async fn read_antigravity_live(
    state: &ProviderRuntime,
    include_credits: bool,
    now: i64,
) -> Result<crate::antigravity::shared::Snapshot, ReadFailure> {
    use crate::antigravity::live::{self, LiveStep};
    use crate::antigravity::shared;
    let dir = state.config_manager.runtime_dir();
    let step = state
        .antigravity_live
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .step(std::time::Instant::now());
    match step {
        LiveStep::Answer(mut snapshot) => {
            // Merge active model updates written by PreInvocation hooks between 60s polls
            // so `Auto` quota pool switches immediately when changing desktop models.
            if let Some(stored) = shared::read_snapshot(dir) {
                if stored.active_model_id.is_some() || stored.active_model_name.is_some() {
                    snapshot.active_model_id = stored.active_model_id;
                    snapshot.active_model_name = stored.active_model_name;
                }
                if snapshot.cli_version.is_none() {
                    snapshot.cli_version = stored.cli_version;
                }
                if snapshot.account_id.is_none() {
                    snapshot.account_id = stored.account_id;
                }
            }
            return Ok(snapshot);
        }
        LiveStep::Failure(failure) => return Err(failure),
        LiveStep::Request => {}
    }
    let result = live::fetch(now, include_credits).await;
    state
        .antigravity_live
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .record(std::time::Instant::now(), &result);
    let mut stored = result?;
    if let Some(previous) = shared::read_snapshot(dir) {
        if stored.active_model_id.is_none() && stored.active_model_name.is_none() {
            stored.active_model_id = previous.active_model_id;
            stored.active_model_name = previous.active_model_name;
        }
        if stored.cli_version.is_none() {
            stored.cli_version = previous.cli_version;
        }
        if stored.account_id.is_none() {
            stored.account_id = previous.account_id;
        }
        if !stored.has_credits && previous.has_credits {
            stored.has_credits = true;
            stored.credits_display = previous.credits_display;
            stored.credits_balance = previous.credits_balance;
        }
    }
    if let Err(error) = shared::write_json_atomic(&dir.join(shared::SNAPSHOT_FILE), &stored) {
        eprintln!("Could not store the live Antigravity limits: {error}");
    }
    Ok(stored)
}

pub(crate) async fn read_dsh_usage(state: &ProviderRuntime, settings: &OverlaySettings, generation: u64) -> Result<RawCodexUsage, String> {
    let result = async {
        let connection = state.dsh.status().await;
        if !connection.connected || connection.instance_id.as_deref() != Some(settings.dsh_instance_id.as_str())
            || state.dsh.origin().await.as_deref() != Some(settings.dsh_base_url.as_str()) {
            return Err("DSH is disconnected or its instance binding changed; reconnect the selected bridge".into());
        }
        state.dsh.read_usage(&settings.dsh_account_id).await
    }.await;
    let _source_commit = state.source_commit.lock().await;
    if state.source_generation.load(Ordering::Acquire) != generation { return Err("Usage source changed during the read".into()); }
    let mut reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
    reader.observation = None;
    reader.failure = None;
    reader.phase = if result.is_ok() { "success" } else { "error" };
    result
}

async fn read_claude_live(state: &ProviderRuntime, now: i64) -> Result<crate::claude::shared::Snapshot, ReadFailure> {
    use crate::claude::live::{self, LiveStep};
    use crate::claude::shared;
    let step = state.claude_live.lock().unwrap_or_else(|p| p.into_inner()).step(std::time::Instant::now());
    match step {
        LiveStep::Answer(snapshot) => return Ok(snapshot),
        LiveStep::Failure(failure) => return Err(failure),
        LiveStep::Request => {}
    }
    let mut result = live::fetch(now).await;
    if let Err(failure) = &result {
        let renew = state.claude_live.lock().unwrap_or_else(|p| p.into_inner())
            .take_renewal(std::time::Instant::now(), failure);
        if renew && live::renew_login().await {
            result = live::fetch(reader::unix_now()).await;
        }
    }
    state.claude_live.lock().unwrap_or_else(|p| p.into_inner()).record(std::time::Instant::now(), &result);
    let snapshot = result?;
    let dir = state.config_manager.runtime_dir();
    let mut stored = snapshot.clone();
    // The bridge's capture keeps its Claude Code version.
    if let Some(previous) = shared::read_snapshot(dir) {
        stored.claude_version = previous.claude_version;
    }
    if let Err(error) = shared::write_json_atomic(&dir.join(shared::SNAPSHOT_FILE), &stored) {
        eprintln!("Could not store the live Claude limits: {error}");
    }
    Ok(snapshot)
}

pub(crate) async fn observe_read_event<S: TrayPresentationSink>(sink: &S, state: &ProviderRuntime, event: ReadEvent) {
    match event {
        ReadEvent::Failed(error) => {
            {
                let mut reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
                reader.phase = "error";
                reader.failure = Some(error);
            }
            // Invalidate immediately, before any cleanup or recovery wait.
            process_usage_error(sink, state).await;
            state.status_wake.notify_one();
        }
        event => {
            let mut reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
            match event {
                ReadEvent::Started(attempt) => { reader.attempt = attempt; reader.phase = "initializing"; }
                ReadEvent::Initializing => reader.phase = "initializing",
                ReadEvent::Quota => reader.phase = "reading",
                ReadEvent::Retrying => reader.phase = "retrying",
                ReadEvent::Finished(report) => reader.record(report),
                ReadEvent::Failed(_) => unreachable!(),
            }
            drop(reader);
            state.status_wake.notify_one();
        }
    }
}

// Coalesce persistence work: one writer per app, snapshots taken after acquiring
// ownership, and at most one latest revision waiting behind a slow disk write.
fn queue_reader_history(app: &AppHandle, context: &ProviderRuntime) {
    let state = app.state::<Arc<AppState>>().monitors.get(context.provider);
    if !std::ptr::eq(state.as_ref(), context) || !state.is_current() {
        return;
    }
    let generation = state.source_generation.load(Ordering::Acquire);
    {
        let mut reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
        if reader.writer_running { return; }
        reader.writer_running = true;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            // Keep the commit gate through the bounded disk write. Otherwise
            // A -> B -> A may reload A and start a second writer while A's old
            // spawn_blocking still writes the same path and temporary file.
            let _source_commit = state.source_commit.lock().await;
            if state.source_generation.load(Ordering::Acquire) != generation || !state.is_current() { return; }
            let (history, revision, path) = {
                let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
                (reader.history.clone(), reader.revision, state.config_manager.quota_history_path())
            };
            let saved = tokio::task::spawn_blocking(move || history.save(&path)).await;
            if state.source_generation.load(Ordering::Acquire) != generation || !state.is_current() { return; }
            let mut reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
            if !matches!(saved, Ok(Ok(()))) || reader.revision == revision {
                reader.writer_running = false;
                break;
            }
        }
    });
}

pub(crate) struct RefreshCompletion<'a> { pub(crate) state: &'a ProviderRuntime, pub(crate) generation: u64, pub(crate) completed: bool }
impl Drop for RefreshCompletion<'_> {
    fn drop(&mut self) {
        if !self.completed && self.state.source_generation.load(Ordering::Acquire) == self.generation {
            let failure = ReadFailure::new(ReadStage::Refresh, FailureKind::Cancelled);
            *self.state.last_refresh_result.lock().unwrap_or_else(|p| p.into_inner()) = Some(Err(failure.to_string()));
            let mut reader = self.state.reader.lock().unwrap_or_else(|p| p.into_inner());
            reader.phase = "error"; reader.failure = Some(failure);
            drop(reader);
            self.state.auto_pause.lock().unwrap_or_else(|p| p.into_inner()).invalidate_pending();
            self.state.dsh_control.lock().unwrap_or_else(|p| p.into_inner()).invalidate();
        }
    }
}

pub(crate) async fn refresh_usage<S: UsageSource>(sink: &S, state: &Arc<ProviderRuntime>) -> Result<CodexUsage, String> {
    if !state.monitor_enabled.load(Ordering::Acquire) { return Err("Monitoring is disabled for this provider".into()); }
    let _refresh_guard = match state.refresh_guard.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            let guard = state.refresh_guard.lock().await;
            let result = state.last_refresh_result.lock().unwrap_or_else(|p| p.into_inner()).clone()
                .unwrap_or_else(|| Err("Usage refresh completed without a result".into()));
            drop(guard);
            return result;
        }
    };

    let generation = {
        let _source_commit = state.source_commit.lock().await;
        if !state.monitor_enabled.load(Ordering::Acquire) || !state.is_current() {
            return Err("Usage source is disabled or no longer current".into());
        }
        state.source_generation.load(Ordering::Acquire)
    };
    *state.last_refresh_result.lock().unwrap_or_else(|p| p.into_inner()) = None;
    let mut completion = RefreshCompletion { state, generation, completed: false };

    state.polling.lock().unwrap_or_else(|p| p.into_inner()).started(std::time::Instant::now());
    state.config_manager.write_status("reading", None);
    let raw_result = sink.read_usage(state).await;

    // Source changes may proceed while network I/O is in flight, but no old
    // result may commit to a new namespace, train its learner or send control.
    let _source_commit = state.source_commit.lock().await;
    if state.source_generation.load(Ordering::Acquire) != generation
        || !state.monitor_enabled.load(Ordering::Acquire) || !state.is_current() {
        completion.completed = true;
        return Err("Usage source changed during the read; discarded the previous source result".into());
    }
    let source_settings = state.settings.lock().await.clone();
    let result = match raw_result {
        Ok(raw) => {
            let estimate = {
                let mut tracker = state.burn_rate.lock().await;
                let estimate = tracker.accept_usage(&raw);
                if estimate.state_changed || tracker.is_dirty() {
                    if let Err(error) = tracker.save(&state.config_manager) {
                        eprintln!("Could not persist burn-rate state: {error}");
                    }
                }
                let candidates=tracker.candidates(&estimate);
                (estimate,candidates)
            };
            crate::history::record_reading(&state.history, state.provider, &source_settings, &raw, reader::unix_now());
            let (estimate,candidates)=estimate;
            let mut usage = raw.to_ui(estimate.five_hour_per_hour, estimate.weekly_per_hour);
            // Smart Pause acts on Codex sessions only; the Claude
            // guard runs inside Claude Code's hooks instead.
            let pref = source_settings.preference(state.provider);
            {
                let display_only = state.reader.lock().unwrap_or_else(|p| p.into_inner()).display_only();
                state.polling.lock().unwrap_or_else(|p| p.into_inner())
                    .completed_with_recovery(&pref, Some(&usage), std::time::Instant::now(), crate::polling::ReadRecovery { display_only, ..Default::default() });
            }
            state.poll_wake.notify_one();
            {
                let mut cache=state.presentation.lock().unwrap_or_else(|p|p.into_inner());
                cache.source=Some(crate::presentation::Sample{usage:usage.clone(),candidates});
                usage=cache.project(pref.five_hour_burn_rate_algorithm,state.applied_settings_revision.load(Ordering::Acquire),state.polling.lock().unwrap_or_else(|p|p.into_inner()).healthy_interval_seconds(&pref)>90).expect("sample was just stored");
            }
            state.config_manager.write_status("ok", None);
            *state.last_refresh_result.lock().unwrap_or_else(|p| p.into_inner()) = Some(Ok(usage.clone()));
            crate::notify::process_usage_success(sink, state, &usage, &raw).await;
            // The account identity is read from the raw sample, because the UI
            // model does not carry it. The auto-pause path needs it to build
            // an unambiguous de-duplication key, and a sample without one
            // never triggers a delivery.
            let account_id = raw.account_id.clone();
            let settings = state.settings.lock().await.clone();
            if state.provider == Provider::Codex {
                crate::pause_control::schedule(state, settings, usage.clone(), account_id);
            } else if state.provider == Provider::Dsh {
                crate::dsh_control::schedule(state, source_settings.clone(), raw.clone(), generation);
            }
            Ok(usage)
        }
        Err(error) => {
            let error = error.to_string();
            {
                let pref = source_settings.preference(state.provider);
                let failure = state.reader.lock().unwrap_or_else(|p| p.into_inner()).failure.clone();
                state.polling.lock().unwrap_or_else(|p| p.into_inner())
                    .completed_with_recovery(&pref, None, std::time::Instant::now(), crate::polling::ReadRecovery {
                        transient: failure.as_ref().is_some_and(ReadFailure::transient),
                        retry_after_seconds: failure.as_ref().and_then(|f| f.retry_after_seconds),
                        display_only: false,
                    });
            }
            state.poll_wake.notify_one();
            state.config_manager.write_status("error", Some(&error));
            *state.last_refresh_result.lock().unwrap_or_else(|p| p.into_inner()) = Some(Err(error.clone()));
            process_usage_error(sink, state).await;
            Err(error)
        }
    };
    completion.completed = true;
    state.status_wake.notify_one();
    result
}

/// Failure preserves cached usage and cancels unsent pause work from an older read.
pub(crate) async fn process_usage_error<S: TrayPresentationSink>(sink: &S, state: &ProviderRuntime) {
    state.auto_pause.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
        .invalidate_pending();
    state.dsh_control.lock().unwrap_or_else(|p| p.into_inner()).invalidate();
    let settings = state.settings.lock().await;
    let mut status = state.tray_read_status.lock().await;
    *status = TrayReadStatus::Error;
    let cached = state.last_usage.lock().await.clone();
    {
        let _tray = state.tray_gate.lock().unwrap_or_else(|p| p.into_inner());
        if state.is_displayed() { sink.present(crate::tray::runtime_presentation(state,&settings, cached.as_ref(), *status)); }
    }
    drop(status);
    drop(settings);
    crate::monitor::publish_runtime(sink, state).await;
}


#[tauri::command]
pub async fn fetch_usage(state: State<'_, Arc<AppState>>, app: AppHandle) -> Result<Option<crate::monitor::Snapshot>, String> {
    let provider = crate::display::effective(&state);
    let runtime = state.monitors.get(provider);
    if !runtime.monitor_enabled.load(Ordering::Acquire) { return Err("Monitoring is disabled for this provider".into()); }
    let generation = runtime.source_generation.load(Ordering::Acquire);
    refresh_usage(&app, &runtime).await?;
    let _source_commit = runtime.source_commit.lock().await;
    if runtime.source_generation.load(Ordering::Acquire) != generation { return Err("Usage source changed".into()); }
    Ok(state.monitors.snapshot(provider).await)
}


#[cfg(test)]
mod refresh_coalescing_tests {
    use super::*;
    use crate::config::ConfigManager;
    use std::sync::atomic::AtomicU64;
    use std::{future::{poll_fn, Future}, task::Poll};
    struct WaitingSource { release: tokio::sync::Notify, reads: AtomicU64, result: Result<RawCodexUsage, String> }
    impl TrayPresentationSink for WaitingSource {
        fn present(&self, _: crate::tray::TrayPresentation) {}
        fn app_handle(&self) -> Option<&AppHandle> { None }
    }
    impl UsageSource for WaitingSource {
        async fn read_usage(&self, _: &ProviderRuntime) -> Result<RawCodexUsage, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.release.notified().await;
            self.result.clone()
        }
    }
    fn sample() -> RawCodexUsage {
        RawCodexUsage { account_id: Some("test".into()), five_hour: Default::default(), weekly: Default::default(), credits_display: "—".into(), credits_balance: None, has_credits: false, fetched_at: reader::unix_now(), antigravity_weekly: None, quota_pool: None, notification_five_hour: None }
    }
    #[tokio::test]
    async fn concurrent_callers_share_success_or_failure_even_with_cached_usage() {
        for failed in [false, true] {
            let dir = std::env::temp_dir().join(format!("quota-coalescing-{}-{failed}", std::process::id()));
            let owner = AppState::for_test(ConfigManager::with_runtime_dir(dir.clone()));
            let state = owner.monitors.get(Provider::Codex);
            *state.last_usage.lock().await = Some(sample().to_ui(None, None));
            let source = WaitingSource { release: Default::default(), reads: AtomicU64::new(0), result: if failed { Err("shared failure".into()) } else { Ok(sample()) } };
            let mut leader = Box::pin(refresh_usage(&source, &state));
            poll_fn(|cx| { assert!(leader.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
            let mut follower = Box::pin(refresh_usage(&source, &state));
            poll_fn(|cx| { assert!(follower.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
            source.release.notify_one();
            let (a, b) = tokio::join!(leader, follower);
            assert_eq!(a, b);
            assert_eq!(a.is_err(), failed);
            assert_eq!(source.reads.load(Ordering::SeqCst), 1);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    #[tokio::test]
    async fn cancelling_the_leader_does_not_return_old_cached_success_to_a_waiter() {
        let dir = std::env::temp_dir().join(format!("quota-cancel-leader-{}", std::process::id()));
        let owner = AppState::for_test(ConfigManager::with_runtime_dir(dir.clone()));
        let state = owner.monitors.get(Provider::Codex);
        *state.last_usage.lock().await = Some(sample().to_ui(None, None));
        let source = WaitingSource { release: Default::default(), reads: AtomicU64::new(0), result: Ok(sample()) };
        let mut leader = Box::pin(refresh_usage(&source, &state));
        poll_fn(|cx| { assert!(leader.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
        let mut follower = Box::pin(refresh_usage(&source, &state));
        poll_fn(|cx| { assert!(follower.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
        drop(leader);
        assert!(follower.await.unwrap_err().contains("Cancelled"));
        assert_eq!(state.reader.lock().unwrap().phase, "error");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The shadow refresh tests.
///
/// They drive the *production* refresh path — `refresh_usage` — with a
/// scripted usage source, so the order of the flush, the acceptance and the
/// persistence is the order the application performs. The assertions read the
/// persisted state file and the local evidence log, because the durable copy
/// is the contract between two runs.
#[cfg(test)]
mod shadow_refresh_tests {
    use super::*;
    use crate::codex::{QuotaSnapshot, RawCodexUsage};
    use crate::config::ConfigManager;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};

    const RESET_AT: i64 = 1_000_000;

    /// A usage source that replays a scripted sequence of outcomes.
    struct ScriptedSource {
        outcomes: StdMutex<VecDeque<Result<RawCodexUsage, String>>>,
    }

    impl ScriptedSource {
        fn new(outcomes: Vec<Result<RawCodexUsage, String>>) -> Self {
            Self {
                outcomes: StdMutex::new(outcomes.into()),
            }
        }
    }

    impl TrayPresentationSink for ScriptedSource {
        fn present(&self, _presentation: crate::tray::TrayPresentation) {}
        fn app_handle(&self) -> Option<&AppHandle> {
            None
        }
    }

    impl UsageSource for ScriptedSource {
        async fn read_usage(&self, _: &ProviderRuntime) -> Result<RawCodexUsage, String> {
            let mut outcomes = self.outcomes.lock().unwrap();
            outcomes
                .pop_front()
                .expect("the scripted outcome must be present")
        }
    }

    fn sample(at: i64, five: f64, week: f64) -> RawCodexUsage {
        RawCodexUsage {
            account_id: Some("acct-a".into()),
            five_hour: QuotaSnapshot {
                used_percent: Some(five),
                resets_at: Some(RESET_AT),
                window_duration_mins: None,
            },
            weekly: QuotaSnapshot {
                used_percent: Some(week),
                resets_at: Some(RESET_AT),
                window_duration_mins: None,
            },
            credits_display: "-".into(),
            credits_balance: None,
            has_credits: false,
            fetched_at: at,
            antigravity_weekly: None,
            quota_pool: None, notification_five_hour: None,
        }
    }

    /// A series that burns exactly one quota point per minute, so the Fixed
    /// Weighted estimator settles at sixty points per hour and a five-minute
    /// interval matures.
    fn burning_series(count: usize) -> Vec<Result<RawCodexUsage, String>> {
        (0..count)
            .map(|minute| Ok(sample(1000 + minute as i64 * 60, 10.0 + minute as f64, 10.0)))
            .collect()
    }

    fn temp_state(tag: &str) -> (Arc<ProviderRuntime>, ConfigManager, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-shadow-refresh-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let config = ConfigManager::with_runtime_dir(dir.clone());
        let owner = Arc::new(AppState::for_test(config.clone()));
        (
            owner.monitors.get(Provider::Codex),
            config,
            dir,
        )
    }

    fn shadow_state(config: &ConfigManager) -> serde_json::Value {
        let raw = std::fs::read_to_string(config.burn_rate_state_path()).unwrap();
        let state: serde_json::Value =
            serde_json::from_str(&raw).expect("the state must be JSON");
        state
            .get("fiveHourShadow")
            .cloned()
            .expect("the state must carry the fiveHourShadow subtree")
    }

    #[tokio::test]
    async fn a_failed_usage_read_does_not_fabricate_feedback_or_erase_the_learner_state() {
        let (state, config, dir) = temp_state("failed-read");
        let mut outcomes = burning_series(9);
        outcomes.push(Err("usage read failed".to_string()));
        let sink = ScriptedSource::new(outcomes);

        for _ in 0..9 {
            refresh_usage(&sink, &state)
                .await
                .expect("the refresh must succeed");
        }

        let before = shadow_state(&config);
        assert_eq!(before["affine"]["feedbacks"], 1);
        let before_raw = std::fs::read_to_string(config.burn_rate_state_path()).unwrap();

        let error = refresh_usage(&sink, &state)
            .await
            .expect_err("the read must fail");
        assert_eq!(error, "usage read failed");

        let after_raw = std::fs::read_to_string(config.burn_rate_state_path()).unwrap();
        assert_eq!(before_raw, after_raw, "a failed read must not change the state");
        let after = shadow_state(&config);
        assert_eq!(after["affine"], before["affine"]);
        assert_eq!(after["pending"], before["pending"]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

