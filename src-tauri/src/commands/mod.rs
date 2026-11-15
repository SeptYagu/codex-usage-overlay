use std::sync::{Arc, Mutex as StdMutex};
#[cfg(windows)]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

use crate::codex::{BurnRateTracker, CodexClient, CodexUsage};
use crate::codex::reader::ReaderState;
use crate::config::{ConfigManager, OverlaySettings, Provider};
use crate::notify::{PendingResetFetches, ResetStateFile};
use crate::tray::{open_settings_window as show_settings_win, TrayReadStatus};

#[cfg(test)]
mod dsh_tests;
mod providers;
mod settings;
mod updater;
mod usage;

pub(crate) use providers::*;
pub(crate) use settings::*;
pub(crate) use updater::*;
pub(crate) use usage::*;

/// Window/updater services are shared by every provider pipeline.
pub struct AppServices {
    pub tray_gate: StdMutex<()>,
    pub dsh: crate::dsh::Manager,
    pub dsh_auto: crate::dsh::auto::Controller,
    pub dock: crate::dock::DockManager,
    pub last_valid_settings_geometry: StdMutex<Option<crate::config::SettingsWindowGeometry>>,
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
    #[cfg(windows)]
    pub audio: OnceLock<crate::audio::AudioHandle>,
    pub displayed_provider: std::sync::atomic::AtomicU8,
    pub display: StdMutex<crate::display::Controller>,
    pub display_wake: tokio::sync::Notify,
    pub status_wake: tokio::sync::Notify,
    pub carousel_enabled: AtomicBool,
    pub snapshot_revision: AtomicU64,
    pub snapshot_gate: Mutex<()>,
}

/// One independent quota/protection pipeline owned by `monitor::Registry`.
pub struct ProviderRuntime {
    pub services: Arc<AppServices>,
    pub provider: Provider,
    pub revoked: AtomicBool,
    pub monitor_enabled: AtomicBool,
    pub has_reading: AtomicBool,
    pub client: Mutex<CodexClient>,
    pub refresh_guard: Mutex<()>,
    pub source_generation: AtomicU64,
    pub dsh_control: StdMutex<crate::dsh_control::ControlState>,
    pub last_refresh_result: StdMutex<Option<Result<CodexUsage, String>>>,
    pub reader: StdMutex<ReaderState>,
    pub polling: StdMutex<crate::polling::PollState>,
    pub poll_wake: tokio::sync::Notify,
    pub protection_status: StdMutex<crate::pause_control::status::StatusCache>,
    pub burn_rate: Mutex<BurnRateTracker>,
    pub source_commit: Mutex<()>,
    pub config_manager: ConfigManager,
    pub last_usage: Mutex<Option<CodexUsage>>,
    pub presentation: StdMutex<crate::presentation::Cache>,
    pub applied_settings_revision: AtomicU64,
    pub claude_live: StdMutex<crate::claude::live::LiveState>,
    pub antigravity_live: StdMutex<crate::antigravity::live::LiveState>,
    pub tray_read_status: Mutex<TrayReadStatus>,
    pub settings: Mutex<OverlaySettings>,
    pub settings_revision: AtomicU64,
    pub reset_state: Mutex<ResetStateFile>,
    pub pending_reset_fetches: Mutex<PendingResetFetches>,
    pub auto_pause: StdMutex<crate::pause_control::AutoPauseController>,
}

impl std::ops::Deref for ProviderRuntime {
    type Target = AppServices;
    fn deref(&self) -> &Self::Target { &self.services }
}

impl ProviderRuntime {
    pub(crate) fn new(
        services: Arc<AppServices>,
        config_manager: ConfigManager,
        mut settings: OverlaySettings,
        provider: Provider,
        generation: u64,
        applied_settings_revision: u64,
    ) -> Self {
        settings.provider = provider;
        let scoped_config = config_manager.for_source(&settings);
        let monitor_enabled = settings.preference(provider).monitor_enabled;
        Self {
            services,
            provider,
            revoked: AtomicBool::new(false),
            monitor_enabled: AtomicBool::new(monitor_enabled),
            has_reading: AtomicBool::new(false),
            client: Mutex::new(CodexClient::new()),
            refresh_guard: Mutex::new(()),
            source_generation: AtomicU64::new(generation),
            dsh_control: StdMutex::new(Default::default()),
            last_refresh_result: StdMutex::new(None),
            reader: StdMutex::new(ReaderState::load(&scoped_config)),
            polling: StdMutex::new(crate::polling::PollState::default()),
            poll_wake: tokio::sync::Notify::new(),
            protection_status: StdMutex::new(Default::default()),
            burn_rate: Mutex::new(BurnRateTracker::load(&scoped_config)),
            source_commit: Mutex::new(()),
            config_manager: scoped_config.clone(),
            last_usage: Mutex::new(None),
            presentation: StdMutex::new(Default::default()),
            applied_settings_revision: AtomicU64::new(applied_settings_revision),
            claude_live: StdMutex::new(Default::default()),
            antigravity_live: StdMutex::new(Default::default()),
            tray_read_status: Mutex::new(TrayReadStatus::Pending),
            settings: Mutex::new(settings),
            settings_revision: AtomicU64::new(0),
            reset_state: Mutex::new(crate::notify::load_reset_state(&scoped_config)),
            pending_reset_fetches: Mutex::new(PendingResetFetches::default()),
            auto_pause: StdMutex::new(crate::pause_control::AutoPauseController::new()),
        }
    }

    pub(crate) fn is_displayed(&self) -> bool {
        self.is_current() && self.displayed_provider.load(Ordering::Acquire) == self.provider.index()
    }

    pub(crate) fn is_current(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
    }
}

/// Application state holding persisted display settings, shared window/updater services,
/// and the registry of independent provider pipelines.
pub struct AppState {
    pub services: Arc<AppServices>,
    pub monitors: crate::monitor::Registry,
    pub settings_patch: Mutex<()>,
    pub config_manager: ConfigManager,
    pub settings: Mutex<OverlaySettings>,
    pub settings_revision: AtomicU64,
}

impl std::ops::Deref for AppState {
    type Target = AppServices;
    fn deref(&self) -> &Self::Target { &self.services }
}

impl AppState {
    pub fn new(config_manager: ConfigManager, settings: OverlaySettings) -> Self {
        let services = Arc::new(AppServices {
            tray_gate: StdMutex::new(()),
            dsh: Default::default(),
            dsh_auto: Default::default(),
            dock: crate::dock::DockManager::new(config_manager.clone()),
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
            #[cfg(windows)]
            audio: OnceLock::new(),
            displayed_provider: std::sync::atomic::AtomicU8::new(settings.provider.index()),
            display: StdMutex::new(crate::display::Controller::new(&settings)),
            display_wake: Default::default(),
            status_wake: Default::default(),
            carousel_enabled: AtomicBool::new(settings.quota_carousel_enabled),
            snapshot_revision: AtomicU64::new(0),
            snapshot_gate: Mutex::new(()),
        });
        let monitors = crate::monitor::Registry::new(&services, &config_manager, &settings, 0);
        Self {
            services,
            monitors,
            settings_patch: Mutex::new(()),
            config_manager,
            settings: Mutex::new(settings),
            settings_revision: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(config_manager: ConfigManager) -> Self {
        let settings = config_manager.load_settings();
        config_manager.set_source(&settings);
        Self::new(config_manager, settings)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsEnvelope {
    pub revision: u64,
    pub settings: OverlaySettings,
    pub source_key: String,
    pub source_generation: u64,
}

impl SettingsEnvelope {
    pub(crate) fn new(state: &AppState, revision: u64, settings: OverlaySettings) -> Self {
        Self {
            source_key: crate::config::source_key(&settings),
            source_generation: state.monitors.get(settings.provider).source_generation.load(Ordering::Acquire),
            revision,
            settings,
        }
    }
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
pub async fn preview_sound(kind: crate::notify::QuotaKind, provider: Option<Provider>, playback_id: Option<String>, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let provider = provider.unwrap_or_else(||crate::display::effective(&state));
        let pref = state.settings.lock().await.preference(provider);
        let path = match kind {
            crate::notify::QuotaKind::FiveHour => pref.five_hour_sound_path.clone(),
            crate::notify::QuotaKind::Week => pref.weekly_sound_path.clone(),
        }
        .ok_or_else(|| "sound_path_missing".to_string())?;
        let audio = state.audio.get().ok_or_else(|| "sound_audio_busy".to_string())?.clone();
        tauri::async_runtime::spawn_blocking(move || audio.preview(path.into(), kind, crate::audio::PlaybackContext { provider, playback_id: playback_id.unwrap_or_default() }))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, state, provider, playback_id);
        Err("sound_audio_busy".into())
    }
}

#[tauri::command]
pub fn stop_preview_sound(playback_id: Option<String>, state: State<'_, Arc<AppState>>) {
    #[cfg(windows)]
    if let Some(audio) = state.audio.get() {
        audio.stop_preview(playback_id);
    }
    #[cfg(not(windows))]
    let _ = (state, playback_id);
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
