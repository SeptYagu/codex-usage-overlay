use std::sync::Arc;
use std::time::Duration;
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
    state.config_manager.save_settings(&new_settings);
    *state.settings.lock().await = new_settings.clone();

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
            "zh-CN" => "浮窗设置",
            "zh-Hant" => "浮窗設定",
            _ => "Overlay Settings",
        };
        let _ = settings_win.set_title(title);
    }

    // Update system tray context menu
    let is_installed = crate::config::is_installed_environment();
    let autostart_enabled = is_installed && new_settings.auto_start;
    let _ = crate::tray::update_tray_menu(&app, &new_settings.language, autostart_enabled, is_installed);

    // Broadcast updated settings to all windows
    let _ = app.emit("settings_updated", &new_settings);
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
pub async fn set_autostart(enable: bool, app: AppHandle) -> Result<(), String> {
    if !crate::config::is_installed_environment() {
        return Err("Autostart is only supported in installer versions".to_string());
    }
    use tauri_plugin_autostart::ManagerExt;
    let autostart_mgr = app.autolaunch();
    if enable {
        let _ = autostart_mgr.enable();
    } else {
        let _ = autostart_mgr.disable();
    }
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
        "zh-CN" => ("立即刷新用量", "浮窗设置…", "隐藏悬浮窗", "退出悬浮窗"),
        "zh-Hant" => ("立即重新整理用量", "浮窗設定…", "隱藏懸浮窗", "結束懸浮窗"),
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

