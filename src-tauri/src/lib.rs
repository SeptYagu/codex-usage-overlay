mod codex;
mod commands;
mod config;
mod tray;

use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Listener, Manager, PhysicalPosition, Position};
use tokio::sync::Mutex;

use codex::CodexClient;
use commands::AppState;
use config::{ConfigManager, WindowPosition};
use tray::setup_tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config_manager = ConfigManager::new();
    let initial_settings = config_manager.load_settings();

    let app_state = Arc::new(AppState {
        client: Mutex::new(CodexClient::new()),
        config_manager: config_manager.clone(),
        last_usage: Mutex::new(None),
        settings: Mutex::new(initial_settings.clone()),
    });

    let app_state_clone = app_state.clone();

    tauri::Builder::default()
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "settings" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .on_menu_event(|app, event| {
            tray::handle_menu_action(app, event.id().as_ref());
        })
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::fetch_usage,
            commands::get_settings,
            commands::save_settings,
            commands::open_settings_window,
            commands::is_installed_version,
            commands::set_autostart,
            commands::start_dragging,
            commands::exit_app,
            commands::show_overlay_menu,
        ])
        .setup(move |app| {
            // Setup system tray
            let is_installed = config::is_installed_environment();
            if !is_installed {
                use tauri_plugin_autostart::ManagerExt;
                let _ = app.autolaunch().disable();
            }
            let autostart_enabled = is_installed && initial_settings.auto_start;
            let _ = setup_tray(app.handle(), autostart_enabled, is_installed);

            // Set initial settings window title based on language
            if let Some(settings_win) = app.get_webview_window("settings") {
                let is_cn = match initial_settings.language.as_str() {
                    "zh-CN" => true,
                    "en-US" => false,
                    _ => sys_locale::get_locale().unwrap_or_default().starts_with("zh"),
                };
                let title = if is_cn { "浮窗设置" } else { "Overlay Settings" };
                let _ = settings_win.set_title(title);
            }

            // Setup main overlay window position and initial size
            if let Some(main_win) = app.get_webview_window("main") {
                // Compute initial size based on saved/default settings
                let scale = initial_settings.scale_percent as f64 / 100.0;
                let base_w = if initial_settings.show_credits { 220.0 } else { 160.0 };
                let base_h = 50.0;
                let init_w = ((base_w * scale).ceil() as u32).max(180);
                let init_h = ((base_h * scale).ceil() as u32).max(50);
                let _ = main_win.set_size(tauri::Size::Logical(tauri::LogicalSize {
                    width: init_w as f64,
                    height: init_h as f64,
                }));

                // Restore position
                if let Some(pos) = config_manager.load_position() {
                    let _ = main_win.set_position(Position::Physical(PhysicalPosition {
                        x: pos.left.round() as i32,
                        y: pos.top.round() as i32,
                    }));
                } else if let Ok(Some(monitor)) = main_win.primary_monitor() {
                    // Default to top-right corner
                    let screen_size = monitor.size();
                    let target_x = (screen_size.width as i32).saturating_sub(init_w as i32 + 20);
                    let target_y = 40;
                    let _ = main_win.set_position(Position::Physical(PhysicalPosition {
                        x: target_x,
                        y: target_y,
                    }));
                }

                // Listen to window move events to persist position (guard against (0, 0) snap)
                let cm = config_manager.clone();
                main_win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(pos) = event {
                        if pos.x > 0 || pos.y > 0 {
                            cm.save_position(&WindowPosition {
                                left: pos.x as f64,
                                top: pos.y as f64,
                            });
                        }
                    }
                });
            }

            // Background polling loop
            let handle = app.handle().clone();
            let state_for_ticker = app_state_clone.clone();

            tauri::async_runtime::spawn(async move {
                // Initial small delay
                tokio::time::sleep(Duration::from_millis(500)).await;

                loop {
                    // Determine refresh interval
                    let interval_secs = {
                        let cfg = state_for_ticker.settings.lock().await;
                        cfg.refresh_interval_seconds.max(15) as u64
                    };

                    // Auto fetch usage
                    let _ = fetch_usage_background(&handle, &state_for_ticker).await;

                    // Sleep for the configured interval
                    tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                }
            });

            // Listen for manual trigger from tray
            let handle_for_trigger = app.handle().clone();
            let state_for_trigger = app_state_clone.clone();
            app.listen("trigger_refresh", move |_| {
                let h = handle_for_trigger.clone();
                let s = state_for_trigger.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = fetch_usage_background(&h, &s).await;
                });
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn fetch_usage_background(handle: &AppHandle, state: &Arc<AppState>) {
    use tauri::Emitter;

    let mut client = state.client.lock().await;
    state.config_manager.write_status("reading", None);

    match client.fetch_usage(Duration::from_secs(20)).await {
        Ok(usage) => {
            state.config_manager.write_status("ok", None);
            *state.last_usage.lock().await = Some(usage.clone());

            let current_settings = state.settings.lock().await.clone();
            let five = usage.five_hour_remaining_percent.map(|p| p.to_string()).unwrap_or_else(|| "--".to_string());
            let week = usage.week_remaining_percent.map(|p| p.to_string()).unwrap_or_else(|| "--".to_string());
            let tooltip = if current_settings.show_credits {
                format!("5H {}% | WK {}% | CR {}", five, week, usage.credits_display)
            } else {
                format!("5H {}% | WK {}%", five, week)
            };
            tray::update_tray_tooltip(handle, &tooltip);
            tray::update_tray_icon(
                handle,
                usage.five_hour_remaining_percent.map(|p| p as f64),
                usage.week_remaining_percent.map(|p| p as f64),
            );

            let _ = handle.emit("usage_updated", &usage);
        }
        Err(e) => {
            let err_msg = e.to_string();
            state.config_manager.write_status("error", Some(&err_msg));
            tray::update_tray_tooltip(handle, "Codex 用量读取失败");
            tray::update_tray_icon(handle, None, None);
        }
    }
}
