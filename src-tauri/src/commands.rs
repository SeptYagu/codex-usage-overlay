use std::sync::{Arc, Mutex as StdMutex};
#[cfg(windows)]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

use crate::codex::{CodexClient, CodexUsage};
use crate::config::{ConfigManager, OverlaySettings};
use crate::notify::{PendingResetFetches, ResetStateFile};
use crate::tray::{open_settings_window as show_settings_win, update_tray_icon, update_tray_tooltip};

pub struct AppState {
    pub client: Mutex<CodexClient>,
    pub settings_patch: Mutex<()>,
    pub config_manager: ConfigManager,
    pub dock: crate::dock::DockManager,
    pub last_usage: Mutex<Option<CodexUsage>>,
    pub settings: Mutex<OverlaySettings>,
    pub settings_revision: AtomicU64,
    /// Last settings-window geometry observed while the window was on screen and
    /// not minimized. A minimized window reports an unusable 0x0 client rect, so
    /// this cache is what lets `CloseRequested` / `RunEvent::Exit` still persist
    /// the user's placement when the session ends while minimized.
    pub last_valid_settings_geometry: StdMutex<Option<crate::config::SettingsWindowGeometry>>,
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
    #[cfg(windows)]
    pub audio: OnceLock<crate::audio::AudioHandle>,
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

#[tauri::command]
pub async fn fetch_usage(state: State<'_, Arc<AppState>>, app: AppHandle) -> Result<CodexUsage, String> {
    let mut client = state.client.lock().await;
    state.config_manager.write_status("reading", None);

    match client.fetch_usage(Duration::from_secs(20)).await {
        Ok(usage) => {
            state.config_manager.write_status("ok", None);
            crate::notify::process_usage_success(&app, state.inner(), &usage).await;
            Ok(usage)
        }
        Err(e) => {
            let err_msg = e.to_string();
            state.config_manager.write_status("error", Some(&err_msg));
            
            let current_settings = state.settings.lock().await.clone();
            let error_text = match resolve_locale(&current_settings.language) {
                "zh-CN" => "Codex 用量读取失败",
                "zh-Hant" => "Codex 用量讀取失敗",
                _ => "Failed to read Codex usage",
            };
            update_tray_tooltip(&app, error_text);
            update_tray_icon(&app, None, None);
            
            Err(err_msg)
        }
    }
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
            | "weeklyResetNotification" | "autoEdgeHide" | "mousePassthrough" => value.is_boolean(),
            "autoStart" => allow_auto_start && value.is_boolean(),
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
}

async fn apply_settings_patch(
    app: &AppHandle,
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
        let mut merged = serde_json::to_value(&*settings).map_err(|e| e.to_string())?;
        let fields = merged.as_object_mut().ok_or("Settings are not an object")?;
        for (key, value) in changes {
            fields.insert(key.clone(), value.clone());
        }
        let mut next: OverlaySettings = serde_json::from_value(merged).map_err(|e| e.to_string())?;
        reconcile_update_preferences(&mut next, changes);
        state.config_manager.save_settings_checked(&next)?;
        let auto_check_just_enabled = !settings.auto_check_updates && next.auto_check_updates;
        let auto_install_just_enabled = !settings.auto_install_updates && next.auto_install_updates;
        *settings = next.clone();
        let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
        (
            SettingsEnvelope { revision, settings: next.clone() },
            auto_check_just_enabled,
            auto_install_just_enabled,
            was_auto_edge_hide != next.auto_edge_hide,
            was_auto_edge_hide,
            was_mouse_passthrough != next.mouse_passthrough,
            was_mouse_passthrough,
            previous_settings.scale_percent != next.scale_percent
                || previous_settings.show_credits != next.show_credits
                || previous_settings.overlay_layout != next.overlay_layout
                || previous_settings.language != next.language,
        )
    };

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
    let new_settings = &envelope.settings;
    if !new_settings.auto_install_updates {
        if let Ok(mut prepared) = state.prepared_update.lock() {
            *prepared = None;
        }
    }

    // Update tray tooltip if we have last usage
    if let Some(usage) = state.last_usage.lock().await.as_ref() {
        let five = usage.five_hour_remaining_percent.map(|p| p.to_string()).unwrap_or_else(|| "--".to_string());
        let week = usage.week_remaining_percent.map(|p| p.to_string()).unwrap_or_else(|| "--".to_string());
        let tooltip = if new_settings.show_credits {
            format!("5H {}% | WK {}% | CR {}", five, week, usage.credits_display)
        } else {
            format!("5H {}% | WK {}%", five, week)
        };
        update_tray_tooltip(app, &tooltip);
        update_tray_icon(
            app,
            usage.five_hour_remaining_percent.map(|p| p as f64),
            usage.week_remaining_percent.map(|p| p as f64),
        );
    } else {
        // Update error text if in error state
        let error_text = match resolve_locale(&new_settings.language) {
            "zh-CN" => "Codex 用量读取失败",
            "zh-Hant" => "Codex 用量讀取失敗",
            _ => "Failed to read Codex usage",
        };
        update_tray_tooltip(app, error_text);
    }

    // Update settings window title
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

