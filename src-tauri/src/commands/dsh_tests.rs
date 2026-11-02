//! Offline regressions for the command boundary; no Tauri runtime or real bridge.
use super::*;
use crate::codex::QuotaSnapshot;
use crate::pause_control::testing::ScriptedTransport;
use std::{
    future::{poll_fn, Future},
    path::PathBuf,
    task::Poll,
};

const ACCOUNT: &str = "123e4567-e89b-12d3-a456-426614174000";
const OTHER_ACCOUNT: &str = "223e4567-e89b-12d3-a456-426614174000";

struct Fixture {
    state: Arc<AppState>,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "overlay-dsh-command-tests-{}",
            uuid::Uuid::new_v4()
        ));
        Self {
            state: Arc::new(AppState::for_test(ConfigManager::with_runtime_dir(
                directory.clone(),
            ))),
            directory,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // This exact UUID directory was created by this fixture, never a parent.
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn sample() -> RawCodexUsage {
    let now = reader::unix_now();
    RawCodexUsage {
        account_id: Some(ACCOUNT.into()),
        five_hour: QuotaSnapshot {
            used_percent: Some(100.0),
            resets_at: Some(now + 3600),
            window_duration_mins: Some(300),
        },
        weekly: QuotaSnapshot {
            used_percent: Some(50.0),
            resets_at: Some(now + 604800),
            window_duration_mins: Some(10080),
        },
        credits_display: "—".into(),
        credits_balance: None,
        has_credits: false,
        fetched_at: now,
    }
}

async fn select_source(state: &Arc<AppState>, provider: Provider, account: &str) {
    let _commit = state.settings_patch.lock().await;
    {
        let mut settings = state.settings.lock().await;
        settings.dsh_base_url = "http://127.0.0.1:50479".into();
        settings.dsh_instance_id = "323e4567-e89b-12d3-a456-426614174000".into();
        settings.dsh_account_id = account.into();
        // No native DSH dispatch is possible in these offline command tests.
        settings.dsh_session_ids.clear();
    }
    switch_provider(state, provider).await;
}

struct Source {
    raw: RawCodexUsage,
    reads: AtomicU64,
    wait: bool,
    release: tokio::sync::Notify,
}
impl Source {
    fn new(wait: bool) -> Self {
        Self {
            raw: sample(),
            reads: AtomicU64::new(0),
            wait,
            release: Default::default(),
        }
    }
}
impl TrayPresentationSink for Source {
    fn present(&self, _: crate::tray::TrayPresentation) {}
    fn app_handle(&self) -> Option<&AppHandle> {
        None
    }
}
impl UsageSource for Source {
    async fn read_usage(&self, _: &AppState) -> Result<RawCodexUsage, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.wait {
            self.release.notified().await;
        }
        Ok(self.raw.clone())
    }
}

#[tokio::test]
async fn source_switch_advances_generation_and_clears_cached_observation() {
    let fixture = Fixture::new();
    let state = &fixture.state;
    let old = sample().to_ui(None, None);
    *state.last_usage.lock().await = Some(old.clone());
    *state.last_refresh_result.lock().unwrap() = Some(Ok(old));
    *state.tray_read_status.lock().await = TrayReadStatus::Success;
    {
        let mut reader = state.reader.lock().unwrap();
        reader.phase = "error";
        reader.failure = Some(ReadFailure::new(ReadStage::Quota, FailureKind::Rpc));
        reader.writer_running = true;
    }
    let generation = state.source_generation.load(Ordering::Acquire);
    select_source(state, Provider::Dsh, ACCOUNT).await;
    assert_eq!(state.source_generation.load(Ordering::Acquire), generation + 1);
    assert!(state.last_usage.lock().await.is_none());
    assert!(state.last_refresh_result.lock().unwrap().is_none());
    assert_eq!(*state.tray_read_status.lock().await, TrayReadStatus::Pending);
    let reader = state.reader.lock().unwrap();
    assert_eq!(reader.phase, "idle");
    assert!(reader.failure.is_none());
    assert!(reader.observation.is_none());
    assert!(!reader.writer_running);
}

#[tokio::test]
async fn dsh_account_namespaces_are_distinct_from_local_and_each_other() {
    let fixture = Fixture::new();
    let state = &fixture.state;
    let config_clone = state.config_manager.clone();
    let local = state.config_manager.status_path();
    state.config_manager.write_status("local-only", None);
    let original = std::fs::read(&local).unwrap();

    select_source(state, Provider::Dsh, ACCOUNT).await;
    let first = state.config_manager.status_path();
    let first_learner = state.config_manager.burn_rate_state_path();
    assert_ne!(first, local);
    assert_eq!(config_clone.status_path(), first);
    state.config_manager.write_status("dsh-only", None);
    assert_eq!(std::fs::read(&local).unwrap(), original);

    select_source(state, Provider::Dsh, OTHER_ACCOUNT).await;
    assert_ne!(state.config_manager.status_path(), first);
    assert_ne!(state.config_manager.burn_rate_state_path(), first_learner);
    assert_ne!(state.config_manager.status_path(), local);
    select_source(state, Provider::Codex, ACCOUNT).await;
    assert_eq!(state.config_manager.status_path(), local);
    assert_eq!(std::fs::read(&local).unwrap(), original);
}

#[tokio::test]
async fn late_fetch_and_its_coalesced_waiter_cannot_populate_the_new_source() {
    let fixture = Fixture::new();
    let state = &fixture.state;
    let source = Source::new(true);
    let mut leader = Box::pin(refresh_usage(&source, state));
    poll_fn(|cx| {
        assert!(leader.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    }).await;
    let mut follower = Box::pin(refresh_usage(&source, state));
    poll_fn(|cx| {
        assert!(follower.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    }).await;

    select_source(state, Provider::Dsh, ACCOUNT).await;
    source.release.notify_one();
    let (leader, follower) = tokio::join!(leader, follower);
    assert!(leader.unwrap_err().contains("source changed"));
    assert!(follower.is_err(), "a waiter must not receive the discarded old success");
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    assert!(state.last_usage.lock().await.is_none());
    assert!(!state.config_manager.burn_rate_state_path().exists());
    assert_eq!(state.reader.lock().unwrap().phase, "idle");
}

#[tokio::test]
async fn cancelled_refresh_completion_revokes_dsh_pending_work() {
    let fixture = Fixture::new();
    let state = &fixture.state;
    select_source(state, Provider::Dsh, ACCOUNT).await;
    state.dsh_control.lock().unwrap().running = true;
    let completion = RefreshCompletion {
        state,
        generation: state.source_generation.load(Ordering::Acquire),
        completed: false,
    };
    drop(completion);
    assert!(!state.dsh_control.lock().unwrap().running);
    assert_eq!(state.reader.lock().unwrap().phase, "error");
    let result = state.last_refresh_result.lock().unwrap().clone().unwrap();
    assert!(result.unwrap_err().contains("Cancelled"));
}

#[tokio::test]
async fn low_dsh_usage_never_schedules_the_local_codex_bridge() {
    // Positive control: the same sample really schedules a local worker for
    // Codex. The counted spawners never launch tasks or open actual pipes.
    for provider in [Provider::Dsh, Provider::Codex] {
        let fixture = Fixture::new();
        let state = &fixture.state;
        select_source(state, provider, ACCOUNT).await;
        state.settings.lock().await.auto_pause_on_low_quota = true;
        let transport = Arc::new(ScriptedTransport::new());
        let workers = Arc::new(AtomicU64::new(0));
        let probes = Arc::new(AtomicU64::new(0));
        {
            let mut controller = state.auto_pause.lock().unwrap();
            controller.set_transport(transport.clone());
            let workers = workers.clone();
            controller.set_worker_spawner(Arc::new(move |_| {
                workers.fetch_add(1, Ordering::SeqCst);
            }));
            let probes = probes.clone();
            controller.set_probe_spawner(Arc::new(move |_| {
                probes.fetch_add(1, Ordering::SeqCst);
            }));
        }
        let source = Source::new(false);
        let usage = refresh_usage(&source, state).await.unwrap();
        assert_eq!(usage.five_hour_remaining_percent, Some(0));
        assert_eq!(workers.load(Ordering::SeqCst), u64::from(provider == Provider::Codex));
        assert_eq!(probes.load(Ordering::SeqCst), 0);
        assert!(transport.calls().is_empty());
        assert!(state.last_usage.lock().await.is_some());
    }
}

#[tokio::test]
async fn dsh_usage_and_settings_envelopes_carry_the_current_source_identity() {
    let fixture = Fixture::new();
    let state = &fixture.state;
    select_source(state, Provider::Dsh, ACCOUNT).await;
    let settings = state.settings.lock().await.clone();
    let generation = state.source_generation.load(Ordering::Acquire);
    let expected_key = crate::config::source_key(&settings);
    let usage = sample().to_ui(None, None);
    let json = serde_json::to_value(usage_envelope(state, &settings, usage)).unwrap();
    assert_eq!(json["sourceKey"], expected_key);
    assert_eq!(json["sourceGeneration"], generation);
    assert_eq!(json["fiveHourRemainingPercent"], 0);
    assert!(json.get("usage").is_none(), "usage remains flattened on the wire");
    let json = serde_json::to_value(SettingsEnvelope::new(state, 7, settings)).unwrap();
    assert_eq!(json["revision"], 7);
    assert_eq!(json["sourceKey"], expected_key);
    assert_eq!(json["sourceGeneration"], generation);
    assert_eq!(json["settings"]["provider"], "dsh");
    assert!(json.get("source_key").is_none());
}
