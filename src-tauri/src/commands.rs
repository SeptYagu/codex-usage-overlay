use std::sync::Arc;
use std::time::Duration;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

use crate::codex::{CodexClient, CodexUsage};
use crate::config::{ConfigManager, OverlaySettings};
use crate::tray::{open_settings_window as show_settings_win, update_tray_icon, update_tray_tooltip};

pub struct AppState {
    pub client: Mutex<CodexClient>,
    pub config_manager: ConfigManager,
    pub last_usage: Mutex<Option<CodexUsage>>,
    pub settings: Mutex<OverlaySettings>,
    pub update_check: Mutex<()>,
    pub pending_update: Mutex<Option<tauri_plugin_updater::Update>>,
    pub available_update: Mutex<Option<AvailableUpdate>>,
    pub last_auto_notified_version: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
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
            *state.last_usage.lock().await = Some(usage.clone());

            // Update tray tooltip
            let current_settings = state.settings.lock().await.clone();
            let five = usage.five_hour_remaining_percent.map(|p| p.to_string()).unwrap_or_else(|| "--".to_string());
            let week = usage.week_remaining_percent.map(|p| p.to_string()).unwrap_or_else(|| "--".to_string());
            let tooltip = if current_settings.show_credits {
                format!("5H {}% | WK {}% | CR {}", five, week, usage.credits_display)
            } else {
                format!("5H {}% | WK {}%", five, week)
            };
            update_tray_tooltip(&app, &tooltip);
            update_tray_icon(
                &app,
                usage.five_hour_remaining_percent.map(|p| p as f64),
                usage.week_remaining_percent.map(|p| p as f64),
            );

            // Broadcast to all windows
            let _ = app.emit("usage_updated", &usage);
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
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<OverlaySettings, String> {
    let settings = state.settings.lock().await.clone();
    Ok(settings)
}

#[tauri::command]
pub async fn save_settings(
    new_settings: OverlaySettings,
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<(), String> {
    let auto_check_just_enabled = {
        let mut settings = state.settings.lock().await;
        let was_enabled = settings.auto_check_updates;
        *settings = new_settings.clone();
        state.config_manager.save_settings(&settings);
        !was_enabled && new_settings.auto_check_updates
    };

    // Update tray tooltip if we have last usage
    if let Some(usage) = state.last_usage.lock().await.as_ref() {
        let five = usage.five_hour_remaining_percent.map(|p| p.to_string()).unwrap_or_else(|| "--".to_string());
        let week = usage.week_remaining_percent.map(|p| p.to_string()).unwrap_or_else(|| "--".to_string());
        let tooltip = if new_settings.show_credits {
            format!("5H {}% | WK {}% | CR {}", five, week, usage.credits_display)
        } else {
            format!("5H {}% | WK {}%", five, week)
        };
        update_tray_tooltip(&app, &tooltip);
        update_tray_icon(
            &app,
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
        update_tray_tooltip(&app, error_text);
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
    let _ = app.emit("settings_updated", &new_settings);
    if auto_check_just_enabled {
        let app_for_check = app.clone();
        let state_for_check = state.inner().clone();
        tauri::async_runtime::spawn(async move {
            check_and_notify_auto_update(&app_for_check, &state_for_check).await;
        });
    }
    Ok(())
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
pub async fn set_autostart(
    enable: bool,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if !crate::config::is_installed_environment() {
        return Err("Autostart is only supported in installer versions".to_string());
    }
    use tauri_plugin_autostart::ManagerExt;
    let autostart_mgr = app.autolaunch();
    if enable {
        autostart_mgr.enable().map_err(|e| e.to_string())?;
    } else {
        autostart_mgr.disable().map_err(|e| e.to_string())?;
    }

    let updated = {
        let mut settings = state.settings.lock().await;
        settings.auto_start = enable;
        state.config_manager.save_settings(&settings);
        settings.clone()
    };
    let _ = app.emit("settings_updated", &updated);
    Ok(())
}

#[tauri::command]
pub fn start_dragging(window: tauri::WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
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

    window.popup_menu(&menu).map_err(|e: tauri::Error| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn toggle_overlay_window(app: AppHandle) {
    crate::tray::toggle_main_window(&app);
}

#[tauri::command]
pub async fn check_for_updates(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<AvailableUpdate>, String> {
    check_and_store_update(&app, &state).await
}

#[tauri::command]
pub async fn get_available_update(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<AvailableUpdate>, String> {
    Ok(state.available_update.lock().await.clone())
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
    Ok(info)
}

pub(crate) async fn check_and_notify_auto_update(app: &AppHandle, state: &AppState) {
    if let Ok(Some(update)) = check_and_store_update(app, state).await {
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
    let update = state
        .pending_update
        .lock()
        .await
        .take()
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

    if !helper.is_file() {
        let _ = std::fs::remove_file(&staged_exe);
        return Err("Portable update helper is missing".to_string());
    }

    let mut command = std::process::Command::new(helper);
    command
        .arg(std::process::id().to_string())
        .arg(&current_exe)
        .arg(&staged_exe)
        .args(std::env::args_os().skip(1));
    command.spawn().map_err(|error| {
        let _ = std::fs::remove_file(&staged_exe);
        error.to_string()
    })?;
    app.exit(0);
    Ok(())
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

