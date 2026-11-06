use super::*;
use crate::{
    codex::{QuotaSnapshot, RawCodexUsage},
    commands::{refresh_usage, UsageSource},
    config::ConfigManager,
    tray::{TrayPresentation, TrayPresentationSink},
};
use std::sync::{atomic::AtomicUsize, Mutex as StdMutex};

static ORDINAL: AtomicUsize = AtomicUsize::new(0);
fn owner() -> (Arc<AppState>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "overlay-monitors-{}-{}",
        std::process::id(),
        ORDINAL.fetch_add(1, Ordering::SeqCst)
    ));
    let config = ConfigManager::with_runtime_dir(dir.clone());
    let mut settings = OverlaySettings::default();
    settings.migrate();
    config.save_settings_checked(&settings).unwrap();
    let owner = Arc::new(AppState::new(config, settings.clone()));
    initialize(&owner, &settings);
    (owner, dir)
}

#[derive(Default)]
struct Source {
    display_events:StdMutex<Vec<crate::display::Snapshot>>,
    usage_events:StdMutex<Vec<crate::commands::UsageEnvelope>>,
    frames: StdMutex<Vec<Frame>>,
    reads: AtomicUsize,
}
#[derive(Clone)]
struct Frame {
    tooltip: String,
    pixels: Vec<u8>,
}
impl TrayPresentationSink for Source {
    fn publish_display(&self,snapshot:&crate::display::Snapshot){self.display_events.lock().unwrap().push(snapshot.clone());}
    fn publish_usage(&self,usage:crate::commands::UsageEnvelope){self.usage_events.lock().unwrap().push(usage);}
    fn present(&self, frame: TrayPresentation) {
        self.frames.lock().unwrap().push(Frame {
            tooltip: frame.tooltip,
            pixels: frame.icon.rgba().to_vec(),
        });
    }
    fn app_handle(&self) -> Option<&AppHandle> {
        None
    }
}
fn sample(provider: Provider) -> RawCodexUsage {
    let used = 10.0 + provider.index() as f64 * 20.0;
    RawCodexUsage {
        account_id: Some(provider.key().into()),
        five_hour: QuotaSnapshot {
            used_percent: Some(used),
            resets_at: Some(2_000_000_000),
            window_duration_mins: Some(300),
        },
        weekly: QuotaSnapshot {
            used_percent: Some(used + 5.0),
            resets_at: Some(2_000_600_000),
            window_duration_mins: Some(10080),
        },
        credits_display: "1.00".into(),
        credits_balance: Some("1.00".into()),
        has_credits: provider != Provider::Claude,
        fetched_at: crate::codex::reader::unix_now(),
    }
}
impl UsageSource for Source {
    async fn read_usage(&self, state: &AppState) -> Result<RawCodexUsage, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(sample(state.settings.lock().await.provider))
    }
}

#[tokio::test]
async fn background_reads_have_separate_caches_files_and_do_not_repaint_the_selected_tray() {
    let (owner, dir) = owner();
    let registry = owner.monitors.get().unwrap();
    let source = Source::default();
    let runtimes = registry.all();
    let (a, b, c) = tokio::join!(
        refresh_usage(&source, &runtimes[0]),
        refresh_usage(&source, &runtimes[1]),
        refresh_usage(&source, &runtimes[2])
    );
    assert_eq!(
        [
            a.unwrap().five_hour_remaining_percent,
            b.unwrap().five_hour_remaining_percent,
            c.unwrap().five_hour_remaining_percent
        ],
        [Some(90), Some(70), Some(50)]
    );
    assert_eq!(source.reads.load(Ordering::SeqCst), 3);
    assert_eq!(
        source.frames.lock().unwrap().len(),
        1,
        "background providers cannot repaint the selected tray"
    );
    let paths: std::collections::HashSet<_> = runtimes
        .iter()
        .map(|r| r.config_manager.burn_rate_state_path())
        .collect();
    assert_eq!(paths.len(), 3);
    assert!(paths.iter().all(|p| p.is_file()));
    assert_eq!(registry.next(Provider::Codex), Some(Provider::Claude));
    let before: Vec<_> = runtimes
        .iter()
        .map(|r| {
            (
                r.source_generation.load(Ordering::Acquire),
                std::fs::read(r.config_manager.burn_rate_state_path()).unwrap(),
            )
        })
        .collect();
    for provider in [Provider::Claude, Provider::Dsh, Provider::Codex] {
        crate::commands::apply_settings_patch(
            &source,
            &owner,
            serde_json::json!({"provider": provider}),
            false,
        )
        .await
        .unwrap();
        let frame = source.frames.lock().unwrap().last().cloned().unwrap();
        assert!(frame.tooltip.contains(match provider {
            Provider::Codex => "90%",
            Provider::Claude => "70%",
            Provider::Dsh => "50%",
        }));
        let remaining = 90.0 - provider.index() as f64 * 20.0;
        assert_eq!(
            frame.pixels,
            crate::tray::generate_quota_icon(
                crate::tray::TrayQuotaMode::FiveHourAndWeekly,
                Some(remaining),
                Some(remaining - 5.0)
            )
            .rgba()
        );
        for (runtime, (generation, bytes)) in runtimes.iter().zip(&before) {
            assert_eq!(
                runtime.source_generation.load(Ordering::Acquire),
                *generation
            );
            assert_eq!(
                std::fs::read(runtime.config_manager.burn_rate_state_path()).unwrap(),
                *bytes
            );
        }
    }
    assert_eq!(
        source.reads.load(Ordering::SeqCst),
        3,
        "display selection must not fetch"
    );
    let (one, two, three) = tokio::join!(
        crate::commands::apply_settings_patch(
            &source,
            &owner,
            serde_json::json!({"cycleProvider":true}),
            false
        ),
        crate::commands::apply_settings_patch(
            &source,
            &owner,
            serde_json::json!({"cycleProvider":true}),
            false
        ),
        crate::commands::apply_settings_patch(
            &source,
            &owner,
            serde_json::json!({"cycleProvider":true}),
            false
        )
    );
    assert_eq!(
        [
            one.unwrap().settings.provider,
            two.unwrap().settings.provider,
            three.unwrap().settings.provider
        ],
        [Provider::Claude, Provider::Dsh, Provider::Codex]
    );
    assert_eq!(source.reads.load(Ordering::SeqCst), 3);
    registry.shutdown();
    drop(runtimes);
    drop(owner);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn protection_preferences_are_independent_and_disabling_a_monitor_revokes_only_its_runtime() {
    let (owner, dir) = owner();
    let source = Source::default();
    let registry = owner.monitors.get().unwrap();
    crate::commands::apply_settings_patch(
        &source,
        &owner,
        serde_json::json!({"providerPreferences":{"claude":{"protectionEnabled":true}}}),
        false,
    )
    .await
    .unwrap();
    assert!(
        registry
            .get(Provider::Claude)
            .settings
            .lock()
            .await
            .auto_pause_on_low_quota
    );
    assert!(
        !registry
            .get(Provider::Codex)
            .settings
            .lock()
            .await
            .auto_pause_on_low_quota
    );
    crate::commands::apply_settings_patch(
        &source,
        &owner,
        serde_json::json!({"provider":"dsh"}),
        false,
    )
    .await
    .unwrap();
    assert!(
        registry
            .get(Provider::Claude)
            .settings
            .lock()
            .await
            .auto_pause_on_low_quota
    );
    let guard: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("claude-guard.json")).unwrap()).unwrap();
    assert_eq!(
        guard["enabled"], true,
        "Claude protection must stay armed when DSH is displayed"
    );
    let codex_generation = registry
        .get(Provider::Codex)
        .source_generation
        .load(Ordering::Acquire);
    crate::commands::apply_settings_patch(
        &source,
        &owner,
        serde_json::json!({"providerPreferences":{"claude":{"monitorEnabled":false}}}),
        false,
    )
    .await
    .unwrap();
    let claude = registry.get(Provider::Claude);
    assert!(!claude.settings.lock().await.auto_pause_on_low_quota);
    assert!(refresh_usage(&source, &claude).await.is_err());
    assert_eq!(
        registry
            .get(Provider::Codex)
            .source_generation
            .load(Ordering::Acquire),
        codex_generation
    );
    assert!(
        !owner
            .config_manager
            .load_settings()
            .preference(Provider::Claude)
            .protection_enabled
    );
    let guard: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("claude-guard.json")).unwrap()).unwrap();
    assert_eq!(guard["enabled"], false);
    registry.shutdown();
    drop(claude);
    drop(owner);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn reconnect_freezes_old_context_before_transport_changes_and_cancels_queued_reads() {
    use std::{
        future::{poll_fn, Future},
        task::Poll,
    };
    let (owner, dir) = owner();
    let registry = owner.monitors.get().unwrap();
    let dsh = registry.get(Provider::Dsh);
    let source = Source::default();
    let generation = dsh.source_generation.load(Ordering::Acquire);
    dsh.dsh_control.lock().unwrap().running = true;
    // An already dispatched action owns this gate until its transport finishes.
    let active_action = dsh.settings_patch.lock().await;
    let mut freeze = Box::pin(crate::commands::freeze_dsh_connection(&dsh));
    poll_fn(|cx| {
        assert!(freeze.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(dsh.monitor_enabled.load(Ordering::Acquire));
    // This read passed the entry check before the reconnect obtained its gate.
    let mut queued_read = Box::pin(refresh_usage(&source, &dsh));
    poll_fn(|cx| {
        assert!(queued_read.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(active_action);
    let frozen = freeze.await;
    assert!(!dsh.monitor_enabled.load(Ordering::Acquire));
    assert!(dsh.source_generation.load(Ordering::Acquire) > generation);
    assert!(!dsh.dsh_control.lock().unwrap().running);
    // The transport swap completes before synchronization installs the binding.
    drop(frozen);
    assert!(queued_read.await.is_err());
    assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    assert!(dsh.last_usage.lock().await.is_none());
    assert!(!dsh.config_manager.burn_rate_state_path().exists());
    // Even reconnecting to the same instance restores the user's preference.
    let settings = owner.settings.lock().await.clone();
    registry.synchronize(&owner, &settings).await;
    assert!(dsh.monitor_enabled.load(Ordering::Acquire));
    refresh_usage(&source, &dsh).await.unwrap();
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    registry.shutdown();
    drop(dsh);
    drop(owner);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn account_rebinding_replaces_only_its_source_and_keeps_control_status_revisions_monotonic() {
    let (owner, dir) = owner();
    let registry = owner.monitors.get().unwrap();
    let codex = registry.get(Provider::Codex);
    let old = registry.get(Provider::Dsh);
    {
        let _pending_action = old.settings_patch.lock().await;
        let snapshots =
            tokio::time::timeout(std::time::Duration::from_secs(2), registry.snapshots())
                .await
                .expect("a pending DSH action must not block other providers' snapshots");
        assert_eq!(snapshots.len(), 3);
    }
    old.dsh_control.lock().unwrap().accepted = 9;
    let before = crate::pause_control::status::snapshot(&old).await;
    let mut settings = owner.settings.lock().await.clone();
    settings.dsh_base_url = "http://127.0.0.1:12345".into();
    settings.dsh_instance_id = "instance-one".into();
    settings.dsh_account_id = "account-one".into();
    *owner.settings.lock().await = settings.clone();
    registry.synchronize(&owner, &settings).await;
    let replacement = registry.get(Provider::Dsh);
    assert!(!Arc::ptr_eq(&old, &replacement));
    assert!(Arc::ptr_eq(&codex, &registry.get(Provider::Codex)));
    assert!(!old.is_current());
    assert!(!old.is_displayed());
    assert_eq!(replacement.source_generation.load(Ordering::Acquire), 1);
    assert_eq!(replacement.dsh_control.lock().unwrap().accepted, 9);
    assert!(
        crate::pause_control::status::snapshot(&replacement)
            .await
            .revision
            >= before.revision
    );
    assert_ne!(
        old.config_manager.burn_rate_state_path(),
        replacement.config_manager.burn_rate_state_path()
    );
    registry.shutdown();
    drop(codex);
    drop(old);
    drop(replacement);
    drop(owner);
    std::fs::remove_dir_all(dir).unwrap();
}

struct SlowSource {
    started: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}
impl TrayPresentationSink for SlowSource {
    fn present(&self, _: TrayPresentation) {}
    fn app_handle(&self) -> Option<&AppHandle> {
        None
    }
}
impl UsageSource for SlowSource {
    async fn read_usage(&self, state: &AppState) -> Result<RawCodexUsage, String> {
        let provider = state.settings.lock().await.provider;
        if provider == Provider::Codex {
            self.started.add_permits(1);
            self.release.acquire().await.unwrap().forget();
        }
        Ok(sample(provider))
    }
}

#[tokio::test]
async fn slow_reads_do_not_block_other_providers_and_display_changes_do_not_cancel_them() {
    let (owner, dir) = owner();
    let registry = owner.monitors.get().unwrap();
    let source = Arc::new(SlowSource {
        started: tokio::sync::Semaphore::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let codex = registry.get(Provider::Codex);
    let pending = {
        let source = source.clone();
        let codex = codex.clone();
        tokio::spawn(async move { refresh_usage(source.as_ref(), &codex).await })
    };
    source.started.acquire().await.unwrap().forget();
    let claude = registry.get(Provider::Claude);
    assert_eq!(
        refresh_usage(source.as_ref(), &claude)
            .await
            .unwrap()
            .five_hour_remaining_percent,
        Some(70)
    );
    crate::commands::apply_settings_patch(
        source.as_ref(),
        &owner,
        serde_json::json!({"provider":"claude"}),
        false,
    )
    .await
    .unwrap();
    source.release.add_permits(1);
    assert_eq!(
        pending.await.unwrap().unwrap().five_hour_remaining_percent,
        Some(90)
    );
    assert_eq!(owner.settings.lock().await.provider, Provider::Claude);
    registry.shutdown();
    drop(codex);
    drop(claude);
    drop(owner);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn disabling_during_a_read_discards_the_result_and_does_not_train_or_repaint() {
    let (owner, dir) = owner();
    let registry = owner.monitors.get().unwrap();
    let codex = registry.get(Provider::Codex);
    let source = Arc::new(SlowSource {
        started: tokio::sync::Semaphore::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let pending = {
        let source = source.clone();
        let codex = codex.clone();
        tokio::spawn(async move { refresh_usage(source.as_ref(), &codex).await })
    };
    source.started.acquire().await.unwrap().forget();
    crate::commands::apply_settings_patch(
        source.as_ref(),
        &owner,
        serde_json::json!({"providerPreferences":{"codex":{"monitorEnabled":false}}}),
        false,
    )
    .await
    .unwrap();
    source.release.add_permits(1);
    assert!(pending.await.unwrap().is_err());
    assert!(codex.last_usage.lock().await.is_none());
    assert!(!codex.config_manager.burn_rate_state_path().exists());
    registry.shutdown();
    drop(codex);
    drop(owner);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn migration_preserves_only_the_previously_enabled_protection_and_source_store_paths() {
    for provider in Provider::ALL {
        let mut legacy: OverlaySettings = serde_json::from_value(serde_json::json!({
            "provider": provider, "scalePercent": 175, "backgroundTransparencyPercent":23,
            "showCredits":true,"refreshIntervalSeconds":60,"language":"en-US","autoStart":false,
            "autoCheckUpdates":true,"fiveHourResetNotification":true,"weeklyResetNotification":true,
            "autoEdgeHide":false,"mousePassthrough":false,"autoPauseOnLowQuota":true
        }))
        .unwrap();
        legacy.migrate();
        let first = serde_json::to_value(&legacy).unwrap();
        legacy.migrate();
        assert_eq!(serde_json::to_value(&legacy).unwrap(), first);
        for target in Provider::ALL {
            assert_eq!(
                legacy.preference(target).protection_enabled,
                provider == target
            );
        }
    }
    let (owner, dir) = owner();
    let weak = Arc::downgrade(&owner);
    let registry = owner.monitors.get().unwrap();
    assert_eq!(
        registry
            .get(Provider::Codex)
            .config_manager
            .burn_rate_state_path(),
        dir.join("burn-rate-state.json")
    );
    assert_eq!(
        registry
            .get(Provider::Claude)
            .config_manager
            .burn_rate_state_path(),
        dir.join("claude-burn-rate-state.json")
    );
    drop(owner);
    assert!(
        weak.upgrade().is_none(),
        "provider runtimes must not retain the application owner"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn provider_preference_patch_is_persisted_and_only_reconfigures_its_runtime() {
    let (owner, dir)=owner(); let registry=owner.monitors.get().unwrap(); let source=Source::default();
    let codex=registry.get(Provider::Codex); let claude=registry.get(Provider::Claude); let dsh=registry.get(Provider::Dsh);
    let codex_revision=codex.settings_revision.load(Ordering::Acquire);
    let dsh_revision=dsh.settings_revision.load(Ordering::Acquire);
    crate::commands::apply_settings_patch(&source,&owner,serde_json::json!({"providerPreferences":{"claude":{
        "refreshIntervalSeconds":120,"weeklyResetNotification":false,"fiveHourSoundPath":null
    }}}),false).await.unwrap();
    assert_eq!(claude.settings.lock().await.refresh_interval_seconds,120);
    assert!(!claude.settings.lock().await.weekly_reset_notification);
    assert_eq!(codex.settings_revision.load(Ordering::Acquire),codex_revision);
    assert_eq!(dsh.settings_revision.load(Ordering::Acquire),dsh_revision);
    assert_eq!(source.reads.load(Ordering::SeqCst),0);
    assert_eq!(owner.config_manager.load_settings().for_provider(Provider::Claude).refresh_interval_seconds,120);
    assert!(owner.config_manager.load_settings().for_provider(Provider::Codex).weekly_reset_notification);
    registry.shutdown();drop(codex);drop(claude);drop(dsh);drop(owner);std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn algorithm_selection_reprojects_cache_without_reading_training_or_clearing_an_error() {
    use crate::config::FiveHourBurnRateAlgorithm as Algorithm;
    let (owner,dir)=owner();let registry=owner.monitors.get().unwrap();let codex=registry.get(Provider::Codex);let source=Source::default();
    refresh_usage(&source,&codex).await.unwrap();
    crate::commands::process_usage_error(&source,&codex).await;
    let memory=codex.burn_rate.lock().await.test_snapshot();
    let file=std::fs::read(codex.config_manager.burn_rate_state_path()).unwrap();
    let fetched=codex.last_usage.lock().await.as_ref().unwrap().fetched_at;
    let revision=codex.presentation.lock().unwrap().metadata.presentation_revision;
    let count=source.usage_events.lock().unwrap().len();
    crate::commands::apply_settings_patch(&source,&owner,serde_json::json!({"fiveHourBurnRateAlgorithm":"weighted"}),false).await.unwrap();
    assert_eq!(source.reads.load(Ordering::SeqCst),1);
    assert_eq!(codex.burn_rate.lock().await.test_snapshot(),memory);
    assert_eq!(std::fs::read(codex.config_manager.burn_rate_state_path()).unwrap(),file);
    assert_eq!(codex.last_usage.lock().await.as_ref().unwrap().fetched_at,fetched);
    assert!(matches!(*codex.tray_read_status.lock().await,crate::tray::TrayReadStatus::Error));
    let events=source.usage_events.lock().unwrap();assert_eq!(events.len(),count+1);
    let event=events.last().unwrap();assert_eq!(event.presentation.presentation_revision,revision+1);
    assert_eq!(event.presentation.requested_algorithm,Some(Algorithm::Weighted));
    assert_eq!(event.presentation.settings_revision,owner.settings_revision.load(Ordering::Acquire));drop(events);
    registry.shutdown();drop(codex);drop(owner);std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn fast_protection_cadence_does_not_report_learning_paused_from_the_normal_interval() {
    let (owner,dir)=owner();let registry=owner.monitors.get().unwrap();let codex=registry.get(Provider::Codex);let source=Source::default();
    refresh_usage(&source,&codex).await.unwrap();
    {let _commit=codex.settings_patch.lock().await;
        let settings={let mut s=codex.settings.lock().await;s.refresh_interval_seconds=120;s.auto_pause_on_low_quota=true;s.five_hour_burn_rate_algorithm=crate::config::FiveHourBurnRateAlgorithm::Personalized;s.clone()};
        let mut usage=codex.last_usage.lock().await.clone().unwrap();usage.five_hour_remaining_percent=Some(5);
        codex.polling.lock().unwrap().completed(&settings,Some(&usage),std::time::Instant::now());
        codex.presentation.lock().unwrap().source.as_mut().unwrap().candidates.learning="active";
        crate::presentation::reproject_locked(&source,&codex).await;
        assert_eq!(codex.presentation.lock().unwrap().metadata.personal_learning_status,Some("active"));
        codex.presentation.lock().unwrap().source.as_mut().unwrap().candidates.learning="observationGap";
        crate::presentation::reproject_locked(&source,&codex).await;
        assert_eq!(codex.presentation.lock().unwrap().metadata.personal_learning_status,Some("observationGap"));
    }
    assert_eq!(source.reads.load(Ordering::SeqCst),1);
    registry.shutdown();drop(codex);drop(owner);std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn carousel_ticks_publish_effective_cache_selection_without_writes_reads_or_runtime_changes() {
    let (owner,dir)=owner();let source=Source::default();let registry=owner.monitors.get().unwrap();
    for state in registry.all(){refresh_usage(&source,&state).await.unwrap();}
    crate::commands::apply_settings_patch(&source,&owner,serde_json::json!({"quotaCarouselEnabled":true}),false).await.unwrap();
    let saved=std::fs::read(owner.config_manager.runtime_dir().join("settings.json")).unwrap();
    let baseline:Vec<_>=registry.all().into_iter().map(|s|(s.settings_revision.load(Ordering::Acquire),s.source_generation.load(Ordering::Acquire),std::fs::read(s.config_manager.burn_rate_state_path()).unwrap())).collect();
    let now=std::time::Instant::now();let revision=owner.settings_revision.load(Ordering::Acquire);
    for (offset,provider) in [(9,Provider::Claude),(18,Provider::Dsh),(27,Provider::Codex)] {
        let snapshot=crate::display::tick(&source,&owner,now+std::time::Duration::from_secs(offset),false).await;
        assert_eq!(snapshot.effective_provider,provider);assert_eq!(crate::display::effective(&owner),provider);
        assert_eq!(snapshot.manual_provider,Provider::Codex);assert_eq!(snapshot.candidates.len(),3);
        assert!(registry.get(provider).is_displayed());
        assert!(source.frames.lock().unwrap().last().unwrap().tooltip.contains(match provider{Provider::Claude=>"Claude",Provider::Dsh=>"DSH",_=>"Codex"}));
    }
    assert_eq!(owner.settings_revision.load(Ordering::Acquire),revision);assert_eq!(source.reads.load(Ordering::Acquire),3);
    assert_eq!(std::fs::read(owner.config_manager.runtime_dir().join("settings.json")).unwrap(),saved);
    for (s,before) in registry.all().iter().zip(baseline){assert_eq!(s.settings_revision.load(Ordering::Acquire),before.0);assert_eq!(s.source_generation.load(Ordering::Acquire),before.1);assert_eq!(std::fs::read(s.config_manager.burn_rate_state_path()).unwrap(),before.2);}
    assert_eq!(source.display_events.lock().unwrap().last().unwrap().effective_provider,Provider::Codex);
    let events=source.display_events.lock().unwrap().len();crate::display::tick(&source,&owner,now+std::time::Duration::from_secs(28),false).await;assert_eq!(source.display_events.lock().unwrap().len(),events);
    let _=std::fs::remove_dir_all(dir);
}
#[tokio::test]
async fn carousel_filters_selected_fresh_identity_bound_sources_and_retains_transient_cached_readings() {
    let (owner,dir)=owner();let source=Source::default();let registry=owner.monitors.get().unwrap();
    for state in registry.all(){refresh_usage(&source,&state).await.unwrap();}
    crate::commands::apply_settings_patch(&source,&owner,serde_json::json!({"quotaCarouselEnabled":true,"providerPreferences":{"dsh":{"carouselIncluded":false}}}),false).await.unwrap();
    let claude=registry.get(Provider::Claude);claude.reader.lock().unwrap().failure=Some(crate::codex::reader::ReadFailure::timeout(crate::codex::reader::ReadStage::Quota));
    let now=std::time::Instant::now();let snapshot=crate::display::tick(&source,&owner,now,false).await;assert_eq!(snapshot.candidates.len(),2);
    claude.reader.lock().unwrap().failure=Some(crate::codex::reader::ReadFailure::new(crate::codex::reader::ReadStage::Quota,crate::codex::reader::FailureKind::Authentication));
    assert_eq!(crate::display::tick(&source,&owner,now,false).await.candidates.len(),1);
    claude.reader.lock().unwrap().failure=None;claude.last_usage.lock().await.as_mut().unwrap().fetched_at-=121;
    assert_eq!(crate::display::tick(&source,&owner,now,false).await.candidates.len(),1);
    let codex=registry.get(Provider::Codex);codex.last_usage.lock().await.as_mut().unwrap().fetched_at-=121;
    let snapshot=crate::display::tick(&source,&owner,now,false).await;assert!(snapshot.candidates.is_empty());assert_eq!(snapshot.effective_provider,Provider::Codex);assert_eq!(snapshot.status,"waiting");
    assert_eq!(source.reads.load(Ordering::Acquire),3);let _=std::fs::remove_dir_all(dir);
}
#[tokio::test]
async fn manual_next_starts_from_effective_page_and_disabling_freezes_it_in_one_save() {
    let (owner,dir)=owner();let source=Source::default();let registry=owner.monitors.get().unwrap();
    for state in registry.all(){refresh_usage(&source,&state).await.unwrap();}
    crate::commands::apply_settings_patch(&source,&owner,serde_json::json!({"quotaCarouselEnabled":true}),false).await.unwrap();
    let now=std::time::Instant::now();crate::display::tick(&source,&owner,now+std::time::Duration::from_secs(9),false).await;
    assert_eq!(crate::display::effective(&owner),Provider::Claude);
    let saved=crate::commands::apply_settings_patch(&source,&owner,serde_json::json!({"cycleProvider":true}),false).await.unwrap();assert_eq!(saved.settings.provider,Provider::Dsh);
    let held=crate::display::tick(&source,&owner,now+std::time::Duration::from_secs(29),false).await;assert_eq!(held.effective_provider,Provider::Dsh);assert_eq!(held.status,"manualHold");
    crate::display::tick(&source,&owner,now+std::time::Duration::from_secs(31),false).await;assert_eq!(crate::display::effective(&owner),Provider::Codex);
    let before=owner.settings_revision.load(Ordering::Acquire);
    let frozen=crate::commands::apply_settings_patch(&source,&owner,serde_json::json!({"quotaCarouselEnabled":false}),false).await.unwrap();assert_eq!(frozen.settings.provider,Provider::Codex);assert_eq!(frozen.revision,before+1);
    assert_eq!(owner.config_manager.load_settings().provider,Provider::Codex);
    assert_eq!(crate::display::tick(&source,&owner,now+std::time::Duration::from_secs(60),false).await.effective_provider,Provider::Codex);
    let unchanged=std::fs::read(owner.config_manager.runtime_dir().join("settings.json")).unwrap();
    assert!(crate::commands::apply_settings_patch(&source,&owner,serde_json::json!({"quotaCarouselIntervalSeconds":2}),false).await.is_err());assert_eq!(std::fs::read(owner.config_manager.runtime_dir().join("settings.json")).unwrap(),unchanged);
    let _=std::fs::remove_dir_all(dir);
}
