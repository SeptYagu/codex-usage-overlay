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

fn is_chinese_locale(language_setting: &str) -> bool {
    match language_setting {
        "zh-CN" => true,
        "en-US" => false,
        _ => {
            let locale = sys_locale::get_locale().unwrap_or_else(|| "en".to_string());
            locale.starts_with("zh")
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
            let error_text = if is_chinese_locale(&current_settings.language) {
                "Codex 用量读取失败"
            } else {
                "Failed to read Codex usage"
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
        let error_text = if is_chinese_locale(&new_settings.language) {
            "Codex 用量读取失败"
        } else {
            "Failed to read Codex usage"
        };
        update_tray_tooltip(&app, error_text);
    }

    // Update settings window title
    if let Some(settings_win) = app.get_webview_window("settings") {
        let is_cn = is_chinese_locale(&new_settings.language);
        let title = if is_cn { "浮窗设置" } else { "Overlay Settings" };
        let _ = settings_win.set_title(title);
    }

    // Broadcast updated settings to all windows
    let _ = app.emit("settings_updated", &new_settings);
    Ok(())
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) {
    show_settings_win(&app);
}

#[tauri::command]
pub async fn set_autostart(enable: bool, app: AppHandle) -> Result<(), String> {
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
    let is_cn = is_chinese_locale(&current_settings.language);

    let refresh_str = if is_cn { "立即刷新用量" } else { "Refresh Now" };
    let settings_str = if is_cn { "浮窗设置…" } else { "Settings…" };
    let hide_str = if is_cn { "隐藏悬浮窗" } else { "Hide" };
    let exit_str = if is_cn { "退出悬浮窗" } else { "Exit" };

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
