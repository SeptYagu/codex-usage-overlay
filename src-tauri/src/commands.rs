use std::sync::{Arc, Mutex as StdMutex};
#[cfg(windows)]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

use crate::codex::{BurnRateTracker, CodexClient, CodexUsage, RawCodexUsage};
use crate::config::{ConfigManager, OverlaySettings};
use crate::notify::{PendingResetFetches, ResetStateFile};
use crate::tray::{open_settings_window as show_settings_win, TrayPresentationSink, TrayReadStatus};

pub struct AppState {
    pub client: Mutex<CodexClient>,
    pub refresh_guard: Mutex<()>,
    pub polling: StdMutex<crate::polling::PollState>,
    pub poll_wake: tokio::sync::Notify,
    pub protection_status: StdMutex<crate::pause_control::status::StatusCache>,
    pub resume: Mutex<crate::pause_control::resume::ResumeManager>,
    pub resume_update: Mutex<()>,
    pub burn_rate: Mutex<BurnRateTracker>,
    pub settings_patch: Mutex<()>,
    pub config_manager: ConfigManager,
    pub dock: crate::dock::DockManager,
    pub last_usage: Mutex<Option<CodexUsage>>,
    pub tray_read_status: Mutex<TrayReadStatus>,
    pub settings: Mutex<OverlaySettings>,
    pub settings_revision: AtomicU64,
    /// Last settings-window geometry observed while the window was on screen and
    /// not minimized. A minimized window reports an unusable 0x0 client rect, so
    /// this cache is what lets `CloseRequested` / `RunEvent::Exit` still persist
    /// the user's placement when the session ends while minimized.
    pub last_valid_settings_geometry: StdMutex<Option<crate::config::SettingsWindowGeometry>>,
    /// Focus grace period of the tray popup: the instant it was last shown plus the
    /// blur observed inside that window. Kept on `AppState` so the `on_window_event`
    /// closure and the grace timer can share it, and so an offline test can drive it
    /// through `AppState::for_test`.
    pub tray_menu_focus: StdMutex<crate::tray::TrayMenuFocusState>,
    pub autostart_update: Mutex<()>,
    pub update_check: Mutex<()>,
    pub update_install: Mutex<()>,
    pub update_installing: AtomicBool,
    pub pending_update: Mutex<Option<tauri_plugin_updater::Update>>,
    pub prepared_update: StdMutex<Option<PreparedUpdate>>,
    pub available_update: Mutex<Option<AvailableUpdate>>,
    pub update_error: Mutex<Option<String>>,
    pub last_auto_notified_version: Mutex<Option<String>>,
    pub reset_state: Mutex<ResetStateFile>,
    pub pending_reset_fetches: Mutex<PendingResetFetches>,
    /// The auto-pause ledger, the queued sample and the bridge transport.
    ///
    /// It is a standard mutex on purpose: the critical sections only exchange
    /// short-lived in-memory state, so the delivery worker never holds it
    /// across an await and never blocks the quota read.
    pub auto_pause: StdMutex<crate::pause_control::AutoPauseController>,
    #[cfg(windows)]
    pub audio: OnceLock<crate::audio::AudioHandle>,
}

/// A state for unit tests that never touch a window, the event loop, the tray or
/// the network. The caller's `config_manager` is kept so tests can assert on the
/// files it writes.
#[cfg(test)]
impl AppState {
    pub(crate) fn for_test(config_manager: ConfigManager) -> Self {
        let settings = config_manager.load_settings();
        Self {
            client: Mutex::new(CodexClient::new()),
            refresh_guard: Mutex::new(()),
            polling: StdMutex::new(crate::polling::PollState::default()),
            poll_wake: tokio::sync::Notify::new(),
            protection_status: StdMutex::new(Default::default()),
            resume: Mutex::new(crate::pause_control::resume::ResumeManager::load(&config_manager)),
            resume_update: Mutex::new(()),
            burn_rate: Mutex::new(BurnRateTracker::load(&config_manager)),
            settings_patch: Mutex::new(()),
            config_manager: config_manager.clone(),
            dock: crate::dock::DockManager::new(config_manager.clone()),
            last_usage: Mutex::new(None),
            tray_read_status: Mutex::new(TrayReadStatus::Pending),
            settings: Mutex::new(settings),
            settings_revision: AtomicU64::new(0),
            last_valid_settings_geometry: StdMutex::new(None),
            tray_menu_focus: StdMutex::new(crate::tray::TrayMenuFocusState::default()),
            autostart_update: Mutex::new(()),
            update_check: Mutex::new(()),
            update_install: Mutex::new(()),
            update_installing: AtomicBool::new(false),
            pending_update: Mutex::new(None),
            prepared_update: StdMutex::new(None),
            available_update: Mutex::new(None),
            update_error: Mutex::new(None),
            last_auto_notified_version: Mutex::new(None),
            reset_state: Mutex::new(crate::notify::load_reset_state(&config_manager)),
            pending_reset_fetches: Mutex::new(PendingResetFetches::default()),
            auto_pause: StdMutex::new(crate::pause_control::AutoPauseController::new()),
            #[cfg(windows)]
            audio: OnceLock::new(),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsEnvelope {
    pub revision: u64,
    pub settings: OverlaySettings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
}

pub struct PreparedUpdate {
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

pub fn match_supported_locale(system_locale: &str) -> &'static str {
    let normalized = system_locale.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "zh-tw" | "zh-hk" | "zh-mo" | "zh-hant" | "zh-hant-tw" | "zh-hant-hk" | "zh-hant-mo" => "zh-Hant",
        "zh" | "zh-cn" | "zh-sg" | "zh-hans" | "zh-hans-cn" | "zh-hans-sg" => "zh-CN",
        _ => "en-US",
    }
}

pub fn resolve_locale(language_setting: &str) -> &'static str {
    match language_setting {
        "zh-CN" => "zh-CN",
        "zh-Hant" => "zh-Hant",
        "en-US" => "en-US",
        _ => {
            let sys = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string());
            match_supported_locale(&sys)
        }
    }
}

/// Inject only the usage fetch at the production refresh boundary. The guard,
/// acceptance, persistence, error and success paths remain the real pipeline.
pub(crate) trait UsageSource: TrayPresentationSink {
    fn read_usage(&self, state: &AppState) -> impl std::future::Future<Output = Result<RawCodexUsage, String>> + Send;
}

impl UsageSource for AppHandle {
    async fn read_usage(&self, state: &AppState) -> Result<RawCodexUsage, String> {
        let mut client = state.client.lock().await;
        client.fetch_usage(Duration::from_secs(20)).await.map_err(|error| error.to_string())
    }
}

pub(crate) async fn refresh_usage<S: UsageSource>(sink: &S, state: &Arc<AppState>) -> Result<CodexUsage, String> {
    let _refresh_guard = match state.refresh_guard.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            let guard = state.refresh_guard.lock().await;
            drop(guard);
            return state
                .last_usage
                .lock()
                .await
                .clone()
                .ok_or_else(|| "Usage refresh completed without a usable sample".to_string());
        }
    };

    state.polling.lock().unwrap_or_else(|p| p.into_inner()).started(std::time::Instant::now());
    state.config_manager.write_status("reading", None);
    let raw_result = sink.read_usage(state).await;

    match raw_result {
        Ok(raw) => {
            let estimate = {
                let mut tracker = state.burn_rate.lock().await;
                // A retained audit event is flushed before the new sample is
                // processed, so a transient append failure does not cost a
                // learning interval.
                if let Err(error) = tracker.flush_audit_outbox(&state.config_manager) {
                    eprintln!("Could not append the retained shadow evidence: {error}");
                }
                let estimate = tracker.accept_usage(&raw);
                if estimate.state_changed || tracker.is_dirty() {
                    if let Err(error) = tracker.save(&state.config_manager) {
                        eprintln!("Could not persist burn-rate state: {error}");
                    }
                }
                // The shadow audit event may only reach the evidence log once
                // the learner state behind it is durable.
                if let Err(error) = tracker.flush_audit_outbox(&state.config_manager) {
                    eprintln!("Could not append shadow evidence: {error}");
                }
                // The flushed slot is cleared in memory, and the cleared state
                // is persisted. Without it the persisted copy would still hold
                // the consumed event, and the next start would append it to
                // the evidence log a second time.
                if tracker.is_dirty() {
                    if let Err(error) = tracker.save(&state.config_manager) {
                        eprintln!("Could not persist burn-rate state: {error}");
                    }
                }
                estimate
            };
            let usage = raw.to_ui(estimate.five_hour_per_hour, estimate.weekly_per_hour);
            crate::pause_control::resume::update(state, Some(&raw)).await;
            {
                let settings = state.settings.lock().await;
                state.polling.lock().unwrap_or_else(|p| p.into_inner())
                    .completed(&settings, Some(&usage), std::time::Instant::now());
            }
            state.poll_wake.notify_one();
            state.config_manager.write_status("ok", None);
            crate::notify::process_usage_success(sink, state, &usage).await;
            // The account identity is read from the raw sample, because the UI
            // model does not carry it. The auto-pause path needs it to build
            // an unambiguous de-duplication key, and a sample without one
            // never triggers a delivery.
            let account_id = raw.account_id.clone();
            let settings = state.settings.lock().await.clone();
            crate::pause_control::schedule(state, settings, usage.clone(), account_id);
            Ok(usage)
        }
        Err(error) => {
            let error = error.to_string();
            crate::pause_control::resume::update(state, None).await;
            {
                let settings = state.settings.lock().await;
                state.polling.lock().unwrap_or_else(|p| p.into_inner())
                    .completed(&settings, None, std::time::Instant::now());
            }
            state.poll_wake.notify_one();
            state.config_manager.write_status("error", Some(&error));
            process_usage_error(sink, state).await;
            Err(error)
        }
    }
}

/// Failure preserves cached usage and cancels unsent pause work from an older read.
pub(crate) async fn process_usage_error<S: TrayPresentationSink>(sink: &S, state: &AppState) {
    state.auto_pause.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
        .invalidate_pending();
    let settings = state.settings.lock().await;
    let mut status = state.tray_read_status.lock().await;
    *status = TrayReadStatus::Error;
    sink.present(crate::tray::tray_presentation(&settings, None, *status));
}

pub(crate) fn overlay_geometry_changed(previous: &OverlaySettings, next: &OverlaySettings) -> bool {
    previous.scale_percent != next.scale_percent
        || previous.show_credits != next.show_credits
        || previous.show_five_hour_quota != next.show_five_hour_quota
        || previous.show_burn_rate != next.show_burn_rate
        || previous.overlay_layout != next.overlay_layout
        || previous.language != next.language
}

#[tauri::command]
pub async fn fetch_usage(state: State<'_, Arc<AppState>>, app: AppHandle) -> Result<CodexUsage, String> {
    refresh_usage(&app, state.inner()).await
}

#[tauri::command]
pub async fn get_last_usage(state: State<'_, Arc<AppState>>) -> Result<Option<CodexUsage>, String> {
    Ok(state.last_usage.lock().await.clone())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<SettingsEnvelope, String> {
    let guard = state.settings.lock().await;
    let settings = guard.clone();
    let revision = state.settings_revision.load(Ordering::SeqCst);
    Ok(SettingsEnvelope {
        revision,
        settings,
    })
}

/// The auto-pause status the settings window reads.
///
/// It reports whether a bridge candidate answered the `list_threads`
/// handshake and how many pause requests were positively confirmed. It never
/// claims a delivery that the bridge did not confirm.
#[tauri::command]
pub async fn get_auto_pause_status(
    state: State<'_, Arc<AppState>>,
) -> Result<crate::pause_control::AutoPauseStatus, String> {
    Ok(crate::pause_control::status::snapshot(state.inner()).await)
}

#[tauri::command]
pub async fn cancel_resume_plans(state: State<'_, Arc<AppState>>, app: AppHandle) -> Result<(), String> {
    state.resume.lock().await.cancel_all("cancelledByUser");
    crate::pause_control::resume::update(state.inner(), None).await;
    crate::pause_control::status::publish(&app, state.inner()).await;
    let error = state.resume.lock().await.error.clone();
    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[tauri::command]
pub async fn patch_settings(
    patch: Value,
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<SettingsEnvelope, String> {
    apply_settings_patch(&app, state.inner(), patch, false).await
}

fn validate_settings_patch(patch: &Value, allow_auto_start: bool) -> Result<&Map<String, Value>, String> {
    let changes = patch.as_object().ok_or("Settings patch must be an object")?;
    if changes.is_empty() {
        return Err("Settings patch is empty".into());
    }
    for (key, value) in changes {
        let valid = match key.as_str() {
            "overlayLayout" => matches!(value.as_str(), Some("grouped" | "stacks")),
            "scalePercent" => value.as_u64().is_some_and(|n| (100..=250).contains(&n) && n % 5 == 0),
            "backgroundTransparencyPercent" => value.as_u64().is_some_and(|n| n <= 80 && n % 5 == 0),
            "showCredits" | "autoCheckUpdates" | "autoInstallUpdates" | "fiveHourResetNotification"
            | "weeklyResetNotification" | "autoEdgeHide" | "mousePassthrough" | "showPercentageGrid"
            | "showFiveHourQuota" | "showBurnRate" | "autoPauseOnLowQuota"
            | "autoResumeAfterQuotaReset" => value.is_boolean(),
            "autoStart" => allow_auto_start && value.is_boolean(),
            "autoPauseFiveHourThresholdPercent" => value.as_u64().is_some_and(|n| (1..=15).contains(&n)),
            "autoPauseWeeklyThresholdPercent" => value.as_u64().is_some_and(|n| (1..=10).contains(&n)),
            "refreshIntervalSeconds" => value.as_u64().is_some_and(|n| (15..=3600).contains(&n)),
            "language" => matches!(value.as_str(), Some("auto" | "en-US" | "zh-CN" | "zh-Hant")),
            "fiveHourSoundMode" | "weeklySoundMode" => matches!(value.as_str(), Some("windows" | "custom")),
            "fiveHourSoundPath" | "weeklySoundPath" => value.is_null() || value.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 4096),
            _ => false,
        };
        if !valid {
            return Err(format!("Invalid settings field: {key}"));
        }
    }
    if changes.get("autoCheckUpdates") == Some(&Value::Bool(false))
        && changes.get("autoInstallUpdates") == Some(&Value::Bool(true)) {
        return Err("Automatic installation requires automatic update checks".into());
    }
    Ok(changes)
}

/// Merges a settings patch into `settings` through the same serde mapping the
/// persisted file uses.
///
/// Extracted out of `apply_settings_patch` so the `patch -> merge -> save -> reload`
/// chain has an offline landing point: `apply_settings_patch` needs an `&AppHandle`,
/// which `#[test]` cannot construct, while this function is window-free. Because
/// `serde` drops a key that has no matching field *silently*, an assertion that goes
/// through here is what catches a frontend key that no longer maps onto the struct.
pub fn merge_settings_patch(settings: &mut OverlaySettings, patch: &Value) -> Result<(), String> {
    let changes = patch.as_object().ok_or("Settings patch must be an object")?;
    let mut merged = serde_json::to_value(&*settings).map_err(|e| e.to_string())?;
    let fields = merged.as_object_mut().ok_or("Settings are not an object")?;
    for (key, value) in changes {
        fields.insert(key.clone(), value.clone());
    }
    *settings = serde_json::from_value(merged).map_err(|e| e.to_string())?;
    Ok(())
}

fn reconcile_update_preferences(next: &mut OverlaySettings, changes: &Map<String, Value>) {
    if next.auto_install_updates && !next.auto_check_updates {
        if changes.get("autoInstallUpdates") == Some(&Value::Bool(true)) {
            next.auto_check_updates = true;
        } else {
            next.auto_install_updates = false;
        }
    }
}

#[cfg(test)]
mod settings_patch_tests {
    use super::*;

    #[test]
    fn rejects_entire_patch_when_one_field_is_invalid() {
        let patch = serde_json::json!({"showCredits": false, "scalePercent": 999});
        assert!(validate_settings_patch(&patch, false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"language": "unknown"}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"autoStart": false}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"futureField": true}), false).is_err());
    }

    #[test]
    fn accepts_independent_notification_fields() {
        let patch = serde_json::json!({
            "fiveHourResetNotification": false,
            "weeklySoundMode": "custom",
            "weeklySoundPath": "C:\\sound.m4a",
            "autoEdgeHide": true,
            "mousePassthrough": true
        });
        assert_eq!(validate_settings_patch(&patch, false).unwrap().len(), 5);
    }

    #[test]
    fn automatic_installation_keeps_update_checks_enabled() {
        let mut settings = OverlaySettings::default();
        settings.auto_check_updates = false;
        settings.auto_install_updates = true;
        let enable = serde_json::json!({"autoInstallUpdates": true});
        reconcile_update_preferences(&mut settings, validate_settings_patch(&enable, false).unwrap());
        assert!(settings.auto_check_updates && settings.auto_install_updates);

        settings.auto_install_updates = true;
        let disable_checks = serde_json::json!({"autoCheckUpdates": false});
        settings.auto_check_updates = false;
        reconcile_update_preferences(&mut settings, validate_settings_patch(&disable_checks, false).unwrap());
        assert!(!settings.auto_install_updates);
        assert!(validate_settings_patch(&serde_json::json!({
            "autoCheckUpdates": false, "autoInstallUpdates": true
        }), false).is_err());
    }

    /// P2-1: the closed allowlist is the toggle's only user entry point. Without the
    /// key registered here the whole patch is rejected and the switch can never
    /// persist, while a frontend-only assertion stays green.
    #[test]
    fn show_percentage_grid_is_on_the_boolean_allowlist() {
        let patch = serde_json::json!({"showPercentageGrid": true});
        assert_eq!(validate_settings_patch(&patch, false).unwrap().len(), 1);
        assert!(validate_settings_patch(&serde_json::json!({"showPercentageGrid": false}), false).is_ok());
        // The boolean branch stays strict, and unknown fields stay rejected.
        assert!(validate_settings_patch(&serde_json::json!({"showPercentageGrid": "yes"}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"showPercentageGrid": 1}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"showPercentageGrid": null}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"futureField": true}), false).is_err());
    }

    /// The offline half of P3-4: the same `validate -> merge -> save` chain the
    /// command runs, minus the `&AppHandle`-only side effects. The reload asserts the
    /// *value*, so a key that silently fails to map onto `OverlaySettings` (or a
    /// renamed field whose serde name no longer matches the camelCase wire key) makes
    /// this fail rather than pass with the toggle simply doing nothing.
    #[test]
    fn a_show_percentage_grid_patch_survives_the_persistence_chain() {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-grid-patch-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::for_test(ConfigManager::with_runtime_dir(dir.clone()));

        let patch = serde_json::json!({"showPercentageGrid": true});
        let changes = validate_settings_patch(&patch, false).expect("the allowlist must accept it");
        assert_eq!(changes.len(), 1);

        // `apply_settings_patch` (which needs an `&AppHandle`) is replaced here by its
        // window-free core plus the same `ConfigManager` it writes through.
        let mut settings = state.config_manager.load_settings();
        assert!(!settings.show_percentage_grid, "the default must be off");
        merge_settings_patch(&mut settings, &patch).expect("the patch must merge");
        assert!(settings.show_percentage_grid, "the key must map onto the field");
        state.config_manager.save_settings_checked(&settings).expect("the write must succeed");

        // Read back through a fresh manager, i.e. from the file on disk.
        let reloaded = ConfigManager::with_runtime_dir(dir.clone()).load_settings();
        assert!(reloaded.show_percentage_grid);
        let raw = std::fs::read_to_string(state.config_manager.settings_path()).unwrap();
        assert!(raw.contains("\"showPercentageGrid\": true"), "wire key missing from {raw}");

        // The command also bumps its revision on a successful patch; the state field
        // is what a frontend assertion sees, so prove the two agree here.
        state.settings_revision.fetch_add(1, Ordering::SeqCst);
        assert_eq!(state.settings_revision.load(Ordering::SeqCst), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Negative control: a patch that is not an object, or that carries a key with no
    /// matching field, must not be able to fabricate the setting.
    #[test]
    fn merge_rejects_non_objects_and_ignores_unmapped_keys() {
        let mut settings = OverlaySettings::default();
        assert!(merge_settings_patch(&mut settings, &serde_json::json!("nope")).is_err());
        assert!(merge_settings_patch(&mut settings, &serde_json::json!([1, 2])).is_err());

        // `serde` drops the unknown key silently, so the merge succeeds but the value
        // is unchanged — the reason `validate_settings_patch` has to run first.
        merge_settings_patch(&mut settings, &serde_json::json!({"showPercentageGridTypo": true})).unwrap();
        assert!(!settings.show_percentage_grid);
    }
}

pub(crate) async fn apply_settings_patch<S: TrayPresentationSink>(
    sink: &S,
    state: &Arc<AppState>,
    patch: Value,
    allow_auto_start: bool,
) -> Result<SettingsEnvelope, String> {
    // Serialize native side effects with the matching persisted preference, even
    // when Settings and the tray submit changes at the same time.
    let _patch_guard = state.settings_patch.lock().await;
    let changes = validate_settings_patch(&patch, allow_auto_start)?;
    let (
        envelope,
        auto_check_just_enabled,
        auto_install_just_enabled,
        auto_pause_just_enabled,
        auto_pause_just_disabled,
        auto_edge_hide_changed,
        previous_auto_edge_hide,
        mouse_passthrough_changed,
        previous_mouse_passthrough,
        geometry_changed,
    ) = {
        let mut settings = state.settings.lock().await;
        let was_auto_edge_hide = settings.auto_edge_hide;
        let was_mouse_passthrough = settings.mouse_passthrough;
        let previous_settings = settings.clone();
        let mut next = settings.clone();
        merge_settings_patch(&mut next, &patch)?;
        reconcile_update_preferences(&mut next, changes);
        state.config_manager.save_settings_checked(&next)?;
        let auto_check_just_enabled = !settings.auto_check_updates && next.auto_check_updates;
        let auto_install_just_enabled = !settings.auto_install_updates && next.auto_install_updates;
        let auto_pause_just_enabled =
            !settings.auto_pause_on_low_quota && next.auto_pause_on_low_quota;
        let auto_pause_just_disabled =
            settings.auto_pause_on_low_quota && !next.auto_pause_on_low_quota;
        *settings = next.clone();
        let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
        (
            SettingsEnvelope { revision, settings: next.clone() },
            auto_check_just_enabled,
            auto_install_just_enabled,
            auto_pause_just_enabled,
            auto_pause_just_disabled,
            was_auto_edge_hide != next.auto_edge_hide,
            was_auto_edge_hide,
            was_mouse_passthrough != next.mouse_passthrough,
            was_mouse_passthrough,
            overlay_geometry_changed(&previous_settings, &next),
        )
    };

    // The active period bookkeeping follows the switch. A session that comes
    // back while the switch is off must not reuse the period identity of its
    // previous low-quota period, or the de-duplication ledger would swallow
    // the next pause request.
    if auto_pause_just_disabled {
        crate::pause_control::clear_active_periods(state);
    }
    // The bridge status stays unknown until the first handshake. A read-only
    // probe answers it right after the switch is turned on, so the status is
    // not a placeholder while the quota is still comfortable.
    if auto_pause_just_enabled {
        crate::pause_control::probe_bridge_status(state);
    }
    {
        let mut polling = state.polling.lock().unwrap_or_else(|p| p.into_inner());
        polling.recalculate(&envelope.settings, std::time::Instant::now());
        if changes.keys().any(|key| matches!(key.as_str(), "autoPauseOnLowQuota"
            | "autoPauseFiveHourThresholdPercent" | "autoPauseWeeklyThresholdPercent")) {
            polling.request_now(std::time::Instant::now());
        }
    }
    state.poll_wake.notify_one();

    if let Some(app) = sink.app_handle() {
        if auto_edge_hide_changed {
            if let Err(error) = crate::dock::auto_hide_changed(
                app,
                state,
                envelope.settings.auto_edge_hide,
            )
            .await
            {
                let rollback = {
                    let mut settings = state.settings.lock().await;
                    let mut reverted = settings.clone();
                    reverted.auto_edge_hide = previous_auto_edge_hide;
                    let persistence_error = state
                        .config_manager
                        .save_settings_checked(&reverted)
                        .err();
                    *settings = reverted.clone();
                    let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
                    (SettingsEnvelope { revision, settings: reverted }, persistence_error)
                };
                let _ = app.emit("settings_updated", &rollback.0);
                return Err(match rollback.1 {
                    Some(write_error) => format!(
                        "Could not update docked overlay: {error}; could not persist rollback: {write_error}"
                    ),
                    None => format!("Could not update docked overlay: {error}"),
                });
            }
        }
        if mouse_passthrough_changed {
            if let Err(error) = set_main_mouse_passthrough(app, envelope.settings.mouse_passthrough) {
                let rollback = {
                    let mut settings = state.settings.lock().await;
                    let mut reverted = settings.clone();
                    reverted.mouse_passthrough = previous_mouse_passthrough;
                    let persistence_error = state.config_manager.save_settings_checked(&reverted).err();
                    *settings = reverted.clone();
                    let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
                    (SettingsEnvelope { revision, settings: reverted }, persistence_error)
                };
                let _ = app.emit("settings_updated", &rollback.0);
                return Err(match rollback.1 {
                    Some(write_error) => format!(
                        "Could not change mouse passthrough: {error}; could not persist rollback: {write_error}"
                    ),
                    None => format!("Could not change mouse passthrough: {error}"),
                });
            }
        }
        if geometry_changed {
            crate::dock::keep_docked_in_work_area(app, state).await;
        }
    }
    let new_settings = &envelope.settings;
    if !new_settings.auto_install_updates {
        if let Ok(mut prepared) = state.prepared_update.lock() {
            *prepared = None;
        }
    }

    if changes.keys().any(|key| matches!(key.as_str(), "autoPauseOnLowQuota"
        | "autoResumeAfterQuotaReset" | "autoPauseFiveHourThresholdPercent"
        | "autoPauseWeeklyThresholdPercent")) {
        crate::pause_control::resume::update(state, None).await;
    }
    crate::tray::repaint_tray(sink, state).await;

    if let Some(app) = sink.app_handle() {
        // Update settings window title
        crate::pause_control::status::publish(app, state).await;
        if let Some(settings_win) = app.get_webview_window("settings") {
            let title = match resolve_locale(&new_settings.language) {
                "zh-CN" => "设置",
                "zh-Hant" => "設定",
                _ => "Settings",
            };
            let _ = settings_win.set_title(title);
        }

        // Broadcast updated settings to all windows
        let _ = app.emit("settings_updated", &envelope);
        if auto_check_just_enabled || auto_install_just_enabled {
            let app_for_check = app.clone();
            let state_for_check = state.clone();
            tauri::async_runtime::spawn(async move {
                check_and_notify_auto_update(&app_for_check, &state_for_check).await;
            });
        }
    }
    Ok(envelope)
}

pub(crate) fn set_main_mouse_passthrough(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if enabled && app.tray_by_id(crate::tray::TRAY_ID).is_none() {
        return Err("System tray is unavailable; mouse passthrough needs a recovery control".into());
    }
    let window = app.get_webview_window("main").ok_or("Main overlay window unavailable")?;
    window.set_ignore_cursor_events(enabled).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) {
    show_settings_win(&app);
}

#[tauri::command]
pub fn is_installed_version() -> bool {
    crate::config::is_installed_environment()
}

#[tauri::command]
pub fn get_tray_menu_generation() -> u64 {
    crate::tray::tray_menu_generation()
}

#[tauri::command]
pub fn layout_tray_menu(
    app: AppHandle,
    generation: u64,
    revision: u64,
    height_logical: f64,
    width_logical: f64,
) -> Result<f64, String> {
    crate::tray::layout_tray_menu(&app, generation, revision, height_logical, width_logical)
}

#[tauri::command]
pub async fn pick_sound_file(kind: crate::notify::QuotaKind, app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Audio files (MP3/AAC/WAV)", &["mp3", "aac", "m4a", "wav"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|error| error.to_string())?;
    #[cfg(windows)]
    {
        crate::audio::probe_audio(&path).map_err(|error| error.to_string())?;
        let _ = kind;
        Ok(Some(path.to_string_lossy().into_owned()))
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, path);
        Err("sound_audio_busy".into())
    }
}

#[tauri::command]
pub async fn preview_sound(kind: crate::notify::QuotaKind, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let settings = state.settings.lock().await;
        let path = match kind {
            crate::notify::QuotaKind::FiveHour => settings.five_hour_sound_path.clone(),
            crate::notify::QuotaKind::Week => settings.weekly_sound_path.clone(),
        }
        .ok_or_else(|| "sound_path_missing".to_string())?;
        drop(settings);
        let audio = state.audio.get().ok_or_else(|| "sound_audio_busy".to_string())?.clone();
        tauri::async_runtime::spawn_blocking(move || audio.preview(path.into(), kind))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, state);
        Err("sound_audio_busy".into())
    }
}

#[tauri::command]
pub fn stop_preview_sound(state: State<'_, Arc<AppState>>) {
    #[cfg(windows)]
    if let Some(audio) = state.audio.get() {
        audio.stop_preview();
    }
    #[cfg(not(windows))]
    let _ = state;
}

pub fn shutdown_audio(app: &AppHandle) {
    #[cfg(windows)]
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        if let Some(audio) = state.audio.get() {
            audio.shutdown();
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[tauri::command]
pub async fn set_autostart(
    enable: bool,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if !crate::config::is_installed_environment() {
        return Err("Autostart is only supported in installer versions".to_string());
    }
    let _autostart_guard = state.autostart_update.lock().await;
    use tauri_plugin_autostart::ManagerExt;
    let autostart_mgr = app.autolaunch();
    let previous = state.settings.lock().await.auto_start;
    if enable {
        autostart_mgr.enable().map_err(|e| e.to_string())?;
    } else {
        autostart_mgr.disable().map_err(|e| e.to_string())?;
    }

    if let Err(error) = apply_settings_patch(&app, state.inner(), serde_json::json!({ "autoStart": enable }), true).await {
        let rollback = if previous { autostart_mgr.enable() } else { autostart_mgr.disable() };
        if let Err(rollback_error) = rollback {
            return Err(format!("{error}; autostart rollback failed: {rollback_error}"));
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub fn start_dragging(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    state.dock.begin_drag();
    if let Err(error) = window.start_dragging() {
        state.dock.cancel_drag();
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn get_dock_state(state: State<'_, Arc<AppState>>) -> crate::dock::DockStateInfo {
    state.dock.info()
}

#[tauri::command]
pub fn dock_mouse_enter(app: AppHandle, state: State<'_, Arc<AppState>>) {
    crate::dock::mouse_enter(&app, state.inner());
}

#[tauri::command]
pub fn dock_mouse_leave(app: AppHandle, state: State<'_, Arc<AppState>>) {
    crate::dock::mouse_leave(&app, state.inner());
}

#[tauri::command]
pub fn dock_window_resized(app: AppHandle, state: State<'_, Arc<AppState>>) {
    crate::dock::window_resized(&app, state.inner());
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    shutdown_audio(&app);
    app.exit(0);
}

#[tauri::command]
pub async fn show_overlay_menu(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<(), String> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let current_settings = state.settings.lock().await.clone();
    let locale = resolve_locale(&current_settings.language);

    let (refresh_str, settings_str, hide_str, exit_str) = match locale {
        "zh-CN" => ("立即刷新用量", "设置…", "隐藏悬浮窗", "退出悬浮窗"),
        "zh-Hant" => ("立即重新整理用量", "設定…", "隱藏懸浮窗", "結束懸浮窗"),
        _ => ("Refresh Now", "Settings…", "Hide", "Exit"),
    };

    let refresh_i = MenuItem::with_id(&app, "refresh_usage", refresh_str, true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let settings_i = MenuItem::with_id(&app, "open_settings", settings_str, true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep1 = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
    let hide_i = MenuItem::with_id(&app, "toggle_overlay", hide_str, true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep2 = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
    let exit_i = MenuItem::with_id(&app, "exit_app", exit_str, true, None::<&str>)
        .map_err(|e| e.to_string())?;

    let menu = Menu::with_items(
        &app,
        &[&refresh_i, &settings_i, &sep1, &hide_i, &sep2, &exit_i],
    )
    .map_err(|e| e.to_string())?;

    state.dock.begin_menu();
    let result = window.popup_menu(&menu).map_err(|e: tauri::Error| e.to_string());
    crate::dock::menu_changed(&app, state.inner(), false);
    result?;
    Ok(())
}

#[tauri::command]
pub fn toggle_overlay_window(app: AppHandle, state: State<'_, Arc<AppState>>) {
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        crate::dock::toggle_overlay_window(&app, &state).await;
    });
}

#[tauri::command]
pub async fn check_for_updates(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<AvailableUpdate>, String> {
    let found = check_and_store_update(&app, &state).await?;
    if found.is_some() && state.settings.lock().await.auto_install_updates {
        let app_for_prepare = app.clone();
        let state_for_prepare = state.inner().clone();
        tauri::async_runtime::spawn(async move {
            let _ = prepare_update(&app_for_prepare, &state_for_prepare).await;
        });
    }
    Ok(found)
}

#[tauri::command]
pub async fn get_available_update(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<AvailableUpdate>, String> {
    Ok(state.available_update.lock().await.clone())
}

#[tauri::command]
pub async fn get_update_error(state: State<'_, Arc<AppState>>) -> Result<Option<String>, String> {
    Ok(state.update_error.lock().await.clone())
}

#[tauri::command]
pub fn get_update_installing(state: State<'_, Arc<AppState>>) -> bool {
    state.update_installing.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn get_update_ready(state: State<'_, Arc<AppState>>) -> bool {
    state.prepared_update.lock().is_ok_and(|prepared| prepared.is_some())
}

pub(crate) async fn check_and_store_update(
    app: &AppHandle,
    state: &AppState,
) -> Result<Option<AvailableUpdate>, String> {
    use tauri_plugin_updater::UpdaterExt;

    let _check_guard = state.update_check.lock().await;
    let updater = app
        .updater_builder()
        .target(crate::config::updater_target())
        .build()
        .map_err(|error| error.to_string())?;
    let update = updater.check().await.map_err(|error| error.to_string())?;
    let info = update.as_ref().map(|update| AvailableUpdate {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: update.body.clone(),
    });
    *state.pending_update.lock().await = update;
    *state.available_update.lock().await = info.clone();
    *state.update_error.lock().await = None;
    if let Ok(mut prepared) = state.prepared_update.lock() {
        if prepared.as_ref().map(|package| package.update.version.as_str())
            != info.as_ref().map(|available| available.version.as_str())
        {
            *prepared = None;
        }
    }
    Ok(info)
}

pub(crate) async fn check_and_notify_auto_update(app: &AppHandle, state: &AppState) {
    if let Ok(Some(update)) = check_and_store_update(app, state).await {
        if state.settings.lock().await.auto_install_updates {
            let _ = prepare_update(app, state).await;
            return;
        }
        let should_notify = {
            let mut last = state.last_auto_notified_version.lock().await;
            if last.as_deref() == Some(update.version.as_str()) {
                false
            } else {
                *last = Some(update.version.clone());
                true
            }
        };
        if should_notify {
            let _ = app.emit("update_available", &update);
        }
    }
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    attempt_install_update(&app, state.inner()).await
}

async fn attempt_install_update(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if state.update_installing.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Ok(());
    }
    *state.update_error.lock().await = None;
    let _ = app.emit("update_install_started", ());
    let result = install_pending_update(app, state).await;
    state.update_installing.store(false, Ordering::SeqCst);
    if let Err(error) = &result {
        *state.update_error.lock().await = Some(error.clone());
        if let Some(update) = state.available_update.lock().await.clone() {
            let _ = app.emit("update_install_failed", update);
        }
        eprintln!("Update installation failed: {error}");
    }
    result
}

async fn install_pending_update(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let _install_guard = state.update_install.lock().await;
    let prepared = state.prepared_update.lock().map_err(|error| error.to_string())?.take();
    if let Some(package) = prepared {
        let result = install_verified_bytes(app, &package.update, &package.bytes, false);
        if result.is_err() {
            *state.prepared_update.lock().map_err(|error| error.to_string())? = Some(package);
        }
        return result;
    }
    let update = state
        .pending_update
        .lock()
        .await
        .as_ref()
        .cloned()
        .ok_or_else(|| "No checked update is available".to_string())?;

    if crate::config::is_installed_environment() {
        let progress_app = app.clone();
        let mut downloaded = 0_u64;
        update
            .download_and_install(
                move |chunk, total| {
                    downloaded += chunk as u64;
                    let _ = progress_app.emit(
                        "update_progress",
                        UpdateProgress { downloaded, total },
                    );
                },
                || {},
            )
            .await
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let progress_app = app.clone();
    let mut downloaded = 0_u64;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit(
                    "update_progress",
                    UpdateProgress { downloaded, total },
                );
            },
            || {},
        )
        .await
        .map_err(|error| error.to_string())?;

    install_verified_bytes(app, &update, &bytes, false)
}

fn install_verified_bytes(
    app: &AppHandle,
    update: &tauri_plugin_updater::Update,
    bytes: &[u8],
    exiting: bool,
) -> Result<(), String> {
    if crate::config::is_installed_environment() {
        let update = update.clone().restart_after_install(!exiting);
        return update.install(bytes).map_err(|error| error.to_string());
    }

    use std::io::Cursor;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Invalid portable update archive: {error}"))?;
    let mut executable = archive
        .by_name("CodexUsageOverlay.exe")
        .map_err(|error| format!("Portable update is missing the application: {error}"))?;
    if executable.size() > 256 * 1024 * 1024 {
        return Err("Portable update executable is too large".to_string());
    }

    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = current_exe
        .parent()
        .ok_or_else(|| "Could not locate the portable application directory".to_string())?;
    let staged_exe = directory.join("CodexUsageOverlay.exe.staged");
    let helper = directory.join("CodexUsageUpdater.exe");
    let mut staged_file = std::fs::File::create(&staged_exe).map_err(|error| error.to_string())?;
    std::io::copy(&mut executable, &mut staged_file).map_err(|error| error.to_string())?;
    drop(executable);
    drop(staged_file);

    let staged_helper = directory.join("CodexUsageUpdater.next.exe");
    let mut helper_archive = archive.by_name("CodexUsageUpdater.exe")
        .map_err(|error| format!("Portable update is missing the updater helper: {error}"))?;
    if helper_archive.size() > 64 * 1024 * 1024 {
        let _ = std::fs::remove_file(&staged_exe);
        return Err("Portable update helper is too large".to_string());
    }
    let mut helper_file = std::fs::File::create(&staged_helper).map_err(|error| error.to_string())?;
    std::io::copy(&mut helper_archive, &mut helper_file).map_err(|error| error.to_string())?;
    drop(helper_file);

    let mut command = std::process::Command::new(&staged_helper);
    command
        .arg(std::process::id().to_string())
        .arg(&current_exe)
        .arg(&staged_exe)
        .arg(&helper)
        .arg(if exiting { "--no-restart" } else { "--restart" })
        .args(std::env::args_os().skip(1));
    command.spawn().map_err(|error| {
        let _ = std::fs::remove_file(&staged_exe);
        let _ = std::fs::remove_file(&staged_helper);
        error.to_string()
    })?;
    if !exiting {
        app.exit(0);
    }
    Ok(())
}

async fn prepare_update(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let _install_guard = state.update_install.lock().await;
    if !state.settings.lock().await.auto_install_updates {
        return Ok(());
    }
    let update = state.pending_update.lock().await.as_ref().cloned()
        .ok_or_else(|| "No checked update is available".to_string())?;
    if state.prepared_update.lock().map_err(|error| error.to_string())?
        .as_ref().is_some_and(|package| package.update.version == update.version)
    {
        return Ok(());
    }
    let bytes = match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => bytes,
        Err(error) => {
            *state.update_error.lock().await = Some(error.to_string());
            if let Some(info) = state.available_update.lock().await.clone() {
                let _ = app.emit("update_install_failed", info);
            }
            eprintln!("Automatic update download failed: {error}");
            return Err(error.to_string());
        }
    };
    if !state.settings.lock().await.auto_install_updates
        || state.pending_update.lock().await.as_ref().is_none_or(|pending| pending.version != update.version)
    {
        return Ok(());
    }
    *state.prepared_update.lock().map_err(|error| error.to_string())? = Some(PreparedUpdate { update, bytes });
    *state.update_error.lock().await = None;
    if let Some(info) = state.available_update.lock().await.clone() {
        let _ = app.emit("update_ready", info);
    }
    Ok(())
}

pub(crate) fn install_prepared_update_on_exit(app: &AppHandle, state: &AppState) {
    if let Ok(mut prepared) = state.prepared_update.lock() {
        if let Some(package) = prepared.take() {
            if let Err(error) = install_verified_bytes(app, &package.update, &package.bytes, true) {
                eprintln!("Could not install prepared update on exit: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_patches_require_booleans_and_invalidate_geometry() {
        for key in ["showFiveHourQuota", "showBurnRate"] {
            for value in [serde_json::json!(true), serde_json::json!(false)] {
                assert!(validate_settings_patch(&serde_json::json!({key: value}), false).is_ok());
            }
            for value in [serde_json::json!("false"), serde_json::json!(0), Value::Null, serde_json::json!([])] {
                assert!(validate_settings_patch(&serde_json::json!({key: value}), false).is_err());
            }
            let previous = OverlaySettings::default();
            let mut next = previous.clone();
            merge_settings_patch(&mut next, &serde_json::json!({key: false})).unwrap();
            assert!(overlay_geometry_changed(&previous, &next));
            assert!(!overlay_geometry_changed(&next, &next));
        }
    }

    #[test]
    fn test_match_supported_locale() {
        // Traditional Chinese literals
        assert_eq!(match_supported_locale("zh-TW"), "zh-Hant");
        assert_eq!(match_supported_locale("zh_TW"), "zh-Hant");
        assert_eq!(match_supported_locale("zh-HK"), "zh-Hant");
        assert_eq!(match_supported_locale("zh-MO"), "zh-Hant");
        assert_eq!(match_supported_locale("zh-Hant"), "zh-Hant");
        assert_eq!(match_supported_locale("zh-Hant-TW"), "zh-Hant");
        assert_eq!(match_supported_locale("zh-Hant-HK"), "zh-Hant");
        assert_eq!(match_supported_locale("zh-Hant-MO"), "zh-Hant");

        // Simplified Chinese literals
        assert_eq!(match_supported_locale("zh"), "zh-CN");
        assert_eq!(match_supported_locale("zh-CN"), "zh-CN");
        assert_eq!(match_supported_locale("zh_CN"), "zh-CN");
        assert_eq!(match_supported_locale("zh-SG"), "zh-CN");
        assert_eq!(match_supported_locale("zh-Hans"), "zh-CN");
        assert_eq!(match_supported_locale("zh-Hans-CN"), "zh-CN");
        assert_eq!(match_supported_locale("zh-Hans-SG"), "zh-CN");

        // Fallbacks
        assert_eq!(match_supported_locale("en-US"), "en-US");
        assert_eq!(match_supported_locale("en-GB"), "en-US");
        assert_eq!(match_supported_locale("ja-JP"), "en-US");
        assert_eq!(match_supported_locale("fr-FR"), "en-US");
        assert_eq!(match_supported_locale(""), "en-US");
    }

    #[test]
    fn test_resolve_locale_explicit() {
        assert_eq!(resolve_locale("zh-CN"), "zh-CN");
        assert_eq!(resolve_locale("zh-Hant"), "zh-Hant");
        assert_eq!(resolve_locale("en-US"), "en-US");
    }
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
    use crate::codex::{QuotaSnapshot, RawCodexUsage};
    use crate::config::ConfigManager;
    use crate::pause_control::testing::ScriptedTransport;
    use crate::pause_control::BridgeConnection;
    use crate::pause_control::BridgeDiscovery;
    use std::sync::Arc;

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
        async fn read_usage(&self, _: &AppState) -> Result<RawCodexUsage, String> {
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
    async fn armed_state(tag: &str) -> (Arc<AppState>, Arc<ScriptedTransport>, std::path::PathBuf) {
        let (state, dir) = temp_state(tag);
        {
            let mut settings = state.settings.lock().await;
            settings.auto_pause_on_low_quota = true;
            settings.auto_pause_five_hour_threshold_percent = 5;
            settings.auto_pause_weekly_threshold_percent = 2;
        }

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
            settings.auto_pause_on_low_quota = false;
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
        assert_eq!(prompts, vec![crate::pause_control::PAUSE_MESSAGE; 2]);

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
        assert_eq!(prompts[0], crate::pause_control::PAUSE_MESSAGE);

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
            vec![crate::pause_control::PAUSE_MESSAGE.to_string()],
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
    /// active period, so it may be prompted again even though the quota
    /// cycle never changed.
    #[tokio::test]
    async fn a_session_that_returns_after_going_idle_is_prompted_again() {
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
        wait_for_prompts(&transport, 2).await;

        assert_eq!(
            transport.sent_prompts().len(),
            2,
            "a returning session starts a new active period and may be prompted again"
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
    /// reservation, so a later refresh may try again.
    ///
    /// The handshake answers at once and only the send hangs. That way a
    /// reservation really exists when the deadline cuts the batch off, and
    /// the measured clock is the batch rather than the refresh.
    #[tokio::test]
    async fn a_batch_that_outlives_the_deadline_releases_its_reservation() {
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

        // The next refresh tries again.
        transport.set_send_delay_ms(0);
        refresh_usage(&sink, &state)
            .await
            .expect("the second refresh must succeed");
        wait_for_prompts(&transport, 2).await;
        assert_eq!(transport.sent_prompts().len(), 2);
        let status = state.auto_pause.lock().unwrap().status(true);
        assert_eq!(status.confirmed_count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The closed allowlist is the only way into the three new settings.
    #[test]
    fn the_auto_pause_settings_are_on_the_closed_allowlist() {
        let switch = serde_json::json!({"autoPauseOnLowQuota": true});
        assert_eq!(validate_settings_patch(&switch, false).unwrap().len(), 1);
        assert!(validate_settings_patch(&serde_json::json!({"autoPauseOnLowQuota": false}), false).is_ok());
        assert!(validate_settings_patch(&serde_json::json!({"autoPauseOnLowQuota": "yes"}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"autoPauseOnLowQuota": 1}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"autoPauseOnLowQuota": null}), false).is_err());

        for value in [1u64, 5, 15] {
            let patch = serde_json::json!({ "autoPauseFiveHourThresholdPercent": value });
            assert!(validate_settings_patch(&patch, false).is_ok(), "{value}");
        }
        for value in [1u64, 2, 10] {
            let patch = serde_json::json!({ "autoPauseWeeklyThresholdPercent": value });
            assert!(validate_settings_patch(&patch, false).is_ok(), "{value}");
        }
        for value in [0u64, 16, 100] {
            let patch = serde_json::json!({ "autoPauseFiveHourThresholdPercent": value });
            assert!(validate_settings_patch(&patch, false).is_err(), "{value}");
        }
        for value in [0u64, 11, 100] {
            let patch = serde_json::json!({ "autoPauseWeeklyThresholdPercent": value });
            assert!(validate_settings_patch(&patch, false).is_err(), "{value}");
        }
        // Fractions, strings and nulls are rejected.
        assert!(validate_settings_patch(
            &serde_json::json!({ "autoPauseFiveHourThresholdPercent": 5.5 }),
            false
        )
        .is_err());
        assert!(validate_settings_patch(
            &serde_json::json!({ "autoPauseWeeklyThresholdPercent": "2" }),
            false
        )
        .is_err());
        assert!(validate_settings_patch(
            &serde_json::json!({ "autoPauseWeeklyThresholdPercent": null }),
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
            "autoPauseOnLowQuota": true,
            "autoPauseFiveHourThresholdPercent": 9,
            "autoPauseWeeklyThresholdPercent": 4,
        });
        let changes = validate_settings_patch(&patch, false).expect("the allowlist must accept it");
        assert_eq!(changes.len(), 3);

        let mut settings = state.config_manager.load_settings();
        assert!(!settings.auto_pause_on_low_quota, "the default must be off");
        merge_settings_patch(&mut settings, &patch).expect("the patch must merge");
        assert!(settings.auto_pause_on_low_quota);
        assert_eq!(settings.auto_pause_five_hour_threshold_percent, 9);
        assert_eq!(settings.auto_pause_weekly_threshold_percent, 4);
        state
            .config_manager
            .save_settings_checked(&settings)
            .expect("the write must succeed");

        let reloaded = ConfigManager::with_runtime_dir(dir.clone()).load_settings();
        assert!(reloaded.auto_pause_on_low_quota);
        assert_eq!(reloaded.auto_pause_five_hour_threshold_percent, 9);
        assert_eq!(reloaded.auto_pause_weekly_threshold_percent, 4);

        let raw = std::fs::read_to_string(state.config_manager.settings_path()).unwrap();
        assert!(raw.contains("\"autoPauseOnLowQuota\": true"), "wire key missing");
        assert!(
            raw.contains("\"autoPauseFiveHourThresholdPercent\": 9"),
            "wire key missing"
        );
        assert!(
            raw.contains("\"autoPauseWeeklyThresholdPercent\": 4"),
            "wire key missing"
        );

        // One invalid field rejects the whole patch.
        let rejected = serde_json::json!({
            "autoPauseOnLowQuota": true,
            "autoPauseWeeklyThresholdPercent": 99,
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
    async fn toggling_auto_pause_off_and_on_resets_active_periods_so_restarted_turn_is_paused() {
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
            settings.auto_pause_on_low_quota = false;
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
            settings.auto_pause_on_low_quota = true;
        }
        refresh_usage(&sink, &state)
            .await
            .expect("the third refresh must succeed");
        wait_for_prompts(&transport, 2).await;

        assert_eq!(
            transport.sent_prompts().len(),
            2,
            "a session that returns after the switch was off is paused again"
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
        let (state, transport, dir) = armed_state("probe-enable").await;
        // The switch starts off.
        {
            let mut settings = state.settings.lock().await;
            settings.auto_pause_on_low_quota = false;
        }
        let sink = OfflineSink::with_usage(comfortable_quota_usage(Some("acct-1")));

        apply_settings_patch(
            &sink,
            &state,
            serde_json::json!({ "autoPauseOnLowQuota": true }),
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

    async fn wait_for_pause_worker(state: &Arc<AppState>) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while state.auto_pause.lock().unwrap().running_flag() {
                tokio::task::yield_now().await;
            }
        }).await.expect("the pause worker must finish");
    }

    #[tokio::test]
    async fn ineligible_updates_cancel_a_low_quota_batch_waiting_for_the_bridge() {
        for change in ["recovered", "missing-account", "failed-read", "disabled", "threshold"] {
            let (state, transport, dir) = armed_state(&format!("cancel-{change}")).await;
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
                    apply_settings_patch(&low, &state,
                        serde_json::json!({ "autoPauseOnLowQuota": false }), false).await.unwrap();
                }
                "threshold" => {
                    apply_settings_patch(&low, &state,
                        serde_json::json!({ "autoPauseFiveHourThresholdPercent": 1 }), false).await.unwrap();
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
        let (state, transport, dir) = armed_state("cancel-remaining").await;
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
        apply_settings_patch(&sink, &state,
            serde_json::json!({ "autoPauseOnLowQuota": false }), false).await.unwrap();
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
    fn resume_preference_is_explicit_and_validated() {
        let mut settings = OverlaySettings::default();
        assert!(!settings.auto_resume_after_quota_reset);
        let patch = serde_json::json!({ "autoResumeAfterQuotaReset": true });
        validate_settings_patch(&patch, false).unwrap();
        merge_settings_patch(&mut settings, &patch).unwrap();
        assert!(settings.auto_resume_after_quota_reset);
        assert!(validate_settings_patch(&serde_json::json!({ "autoResumeAfterQuotaReset": "true" }), false).is_err());
        let legacy: OverlaySettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!legacy.auto_resume_after_quota_reset);
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
    use std::sync::Arc;

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
        async fn read_usage(&self, _: &AppState) -> Result<RawCodexUsage, String> {
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

    /// A series of explicit samples.
    fn series(points: &[(i64, f64)]) -> Vec<Result<RawCodexUsage, String>> {
        points
            .iter()
            .map(|(at, used)| Ok(sample(*at, *used, 10.0)))
            .collect()
    }

    fn temp_state(tag: &str) -> (Arc<AppState>, ConfigManager, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-shadow-refresh-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let config = ConfigManager::with_runtime_dir(dir.clone());
        (
            Arc::new(AppState::for_test(config.clone())),
            config,
            dir,
        )
    }

    /// The persisted shadow subtree of the burn-rate state.
    fn shadow_state(config: &ConfigManager) -> serde_json::Value {
        let raw = std::fs::read_to_string(config.burn_rate_state_path()).unwrap();
        let state: serde_json::Value =
            serde_json::from_str(&raw).expect("the state must be JSON");
        state
            .get("fiveHourShadow")
            .cloned()
            .expect("the state must carry the shadow subtree")
    }

    /// The records of the local evidence log.
    fn evidence(config: &ConfigManager) -> Vec<serde_json::Value> {
        let Ok(content) = std::fs::read_to_string(config.shadow_evidence_path()) else {
            return Vec::new();
        };
        content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("the evidence record must be JSON"))
            .collect()
    }

    /// The refresh that flushes the audit event also persists the cleared
    /// slot.
    ///
    /// The persisted copy is the contract between two runs: a retained event
    /// on disk would be appended to the evidence log again on the next start.
    #[tokio::test]
    async fn a_refresh_persists_the_cleared_audit_slot() {
        let (state, config, dir) = temp_state("cleared-slot");
        let sink = ScriptedSource::new(burning_series(9));

        for _ in 0..9 {
            refresh_usage(&sink, &state)
                .await
                .expect("the refresh must succeed");
        }

        // The interval matured, so the event reached the log.
        let records = evidence(&config);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["kind"], "matured");
        // The persisted slot is empty, so the next start appends nothing.
        let persisted = shadow_state(&config);
        assert!(
            persisted["auditOutbox"].is_null(),
            "the cleared slot must be persisted: {persisted}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed usage read must not fabricate a feedback and must not erase
    /// the learner state.
    ///
    /// The failure is injected into the production read boundary, so the
    /// sample really never reaches the tracker.
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

        // One interval matured, so the learner holds real evidence and the
        // durable state really carries the shadow subtree.
        let before = shadow_state(&config);
        assert_eq!(before["personal"]["maturedFeedbacks"], 1);
        let before_raw = std::fs::read_to_string(config.burn_rate_state_path()).unwrap();
        let before_evidence = evidence(&config).len();

        // The read fails now.
        let error = refresh_usage(&sink, &state)
            .await
            .expect_err("the read must fail");
        assert_eq!(error, "usage read failed");

        // The durable state is untouched: no fabricated feedback, no erased
        // learner state, no lost interval.
        let after_raw = std::fs::read_to_string(config.burn_rate_state_path()).unwrap();
        assert_eq!(before_raw, after_raw, "a failed read must not change the state");
        let after = shadow_state(&config);
        assert_eq!(after["personal"], before["personal"]);
        assert_eq!(after["pending"], before["pending"]);
        assert_eq!(after["auditOutbox"], before["auditOutbox"]);
        assert_eq!(evidence(&config).len(), before_evidence);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A retained audit event is flushed before the next sample is processed.
    ///
    /// A transient append failure must cost no learning interval: the
    /// retained event is appended by the next refresh, and the interval that
    /// was waiting in the meantime is learned from, not dropped.
    #[tokio::test]
    async fn a_retained_audit_event_is_flushed_before_the_next_sample_is_processed() {
        let (state, config, dir) = temp_state("retained-flush");
        // The evidence log cannot be appended to while it is a directory.
        std::fs::create_dir_all(config.shadow_evidence_path()).unwrap();
        let sink = ScriptedSource::new(series(&[
            (1000, 10.0),
            (1060, 11.0),
            (1120, 12.0),
            (1180, 13.0),
            (1240, 14.0),
            (1300, 15.0),
            (1360, 16.0),
            (1420, 17.0),
            (1480, 18.0),
            // The paused sample. The learner skips it, so the next one has to
            // stay inside the frozen 90 second gap rule.
            (1520, 18.5),
            (1560, 19.0),
        ]));

        // Nine samples: the fourth opens the interval, the ninth matures it.
        for _ in 0..9 {
            refresh_usage(&sink, &state)
                .await
                .expect("the refresh must succeed");
        }

        // The matured event is retained, because the append failed.
        let retained = shadow_state(&config);
        assert_eq!(retained["auditOutbox"]["kind"], "matured");
        assert_eq!(retained["auditOutbox"]["targetAt"], 1480);
        assert_eq!(retained["personal"]["maturedFeedbacks"], 1);
        // The next interval starts at the maturation point.
        assert_eq!(retained["pending"]["originAt"], 1480);

        // The tenth sample arrives while the event is still retained.
        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        let blocked = shadow_state(&config);
        assert_eq!(
            blocked["auditOutbox"]["kind"], "matured",
            "the retained event must survive"
        );
        // The learner is paused, so the interval is not settled. It stays
        // alive, because a busy audit outbox must not cost a learning
        // interval that the quota really produced.
        assert_eq!(blocked["pending"]["lastValidAt"], 1520);
        assert_eq!(blocked["personal"]["maturedFeedbacks"], 1);

        // The evidence log becomes writable again.
        std::fs::remove_dir_all(config.shadow_evidence_path()).unwrap();

        // The eleventh sample arrives. The retained event is flushed first, so
        // the learner is no longer paused and the interval advances.
        refresh_usage(&sink, &state)
            .await
            .expect("the refresh must succeed");
        let flushed = shadow_state(&config);
        assert!(
            flushed["auditOutbox"].is_null(),
            "the retained event must be flushed"
        );
        assert_eq!(
            flushed["pending"]["lastValidAt"], 1560,
            "the waiting interval must not be lost"
        );

        let records = evidence(&config);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["kind"], "matured");
        assert_eq!(records[0]["targetAt"], 1480);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

