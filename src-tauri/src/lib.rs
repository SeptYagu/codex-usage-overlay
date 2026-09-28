mod codex;
mod commands;
mod config;
mod notify;
mod tray;
#[cfg(windows)]
mod audio;

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
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
        settings_revision: AtomicU64::new(0),
        autostart_update: Mutex::new(()),
        update_check: Mutex::new(()),
        pending_update: Mutex::new(None),
        available_update: Mutex::new(None),
        last_auto_notified_version: Mutex::new(None),
        reset_state: Mutex::new(notify::load_reset_state(&config_manager)),
        pending_reset_fetches: Mutex::new(notify::PendingResetFetches::default()),
        #[cfg(windows)]
        audio: std::sync::OnceLock::new(),
    });

    let app_state_clone = app_state.clone();

    tauri::Builder::default()
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. }
                if window.label() == "settings" || window.label() == "tray-menu" =>
            {
                api.prevent_close();
                let _ = window.hide();
            }
            tauri::WindowEvent::Focused(false) if window.label() == "tray-menu" => {
                let _ = window.hide();
            }
            _ => {}
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::fetch_usage,
            commands::get_settings,
            commands::patch_settings,
            commands::open_settings_window,
            commands::is_installed_version,
            commands::get_tray_menu_generation,
            commands::layout_tray_menu,
            notify::get_notification_status,
            commands::pick_sound_file,
            commands::preview_sound,
            commands::stop_preview_sound,
            commands::set_autostart,
            commands::start_dragging,
            commands::exit_app,
            commands::show_overlay_menu,
            commands::toggle_overlay_window,
            commands::check_for_updates,
            commands::get_available_update,
            commands::install_update,
        ])
        .setup(move |app| {
            // Setup system tray
            let _ = setup_tray(app.handle());

            #[cfg(windows)]
            match audio::spawn_worker(app.handle().clone()) {
                Ok(audio) => { let _ = app_state_clone.audio.set(audio); }
                Err(error) => eprintln!("Could not start audio worker: {error}"),
            }

            // Set initial settings window title based on language
            if let Some(settings_win) = app.get_webview_window("settings") {
                let title = match commands::resolve_locale(&initial_settings.language) {
                    "zh-CN" => "设置",
                    "zh-Hant" => "設定",
                    _ => "Settings",
                };
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

            // Check for stable updates periodically when automatic checks are enabled.
            let update_handle = app.handle().clone();
            let update_state = app_state_clone.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(15)).await;
                loop {
                    let auto_check = update_state.settings.lock().await.auto_check_updates;
                    if auto_check {
                        commands::check_and_notify_auto_update(&update_handle, &update_state).await;
                    }
                    tokio::time::sleep(Duration::from_secs(12 * 60 * 60)).await;
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                commands::shutdown_audio(app);
            }
        });
}

pub(crate) async fn fetch_usage_background(handle: &AppHandle, state: &Arc<AppState>) {
    let mut client = state.client.lock().await;
    state.config_manager.write_status("reading", None);

    match client.fetch_usage(Duration::from_secs(20)).await {
        Ok(usage) => {
            state.config_manager.write_status("ok", None);
            notify::process_usage_success(handle, state, &usage).await;
        }
        Err(e) => {
            let err_msg = e.to_string();
            state.config_manager.write_status("error", Some(&err_msg));
            tray::update_tray_tooltip(handle, "Codex 用量读取失败");
            tray::update_tray_icon(handle, None, None);
        }
    }
}
