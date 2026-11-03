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
    frames: StdMutex<Vec<Frame>>,
    reads: AtomicUsize,
}
#[derive(Clone)]
struct Frame {
    tooltip: String,
    pixels: Vec<u8>,
}
impl TrayPresentationSink for Source {
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
async fn account_rebinding_replaces_only_its_source_and_keeps_control_status_revisions_monotonic() {
    let (owner, dir) = owner();
    let registry = owner.monitors.get().unwrap();
    let codex = registry.get(Provider::Codex);
    let old = registry.get(Provider::Dsh);
    {
        let _pending_action = old.settings_patch.lock().await;
        let snapshots = tokio::time::timeout(std::time::Duration::from_secs(2), registry.snapshots()).await.expect("a pending DSH action must not block other providers' snapshots");
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
