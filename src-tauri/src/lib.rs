mod codex;
mod commands;
mod config;
mod dock;
mod notify;
mod tray;
#[cfg(windows)]
mod audio;

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::Mutex;

use codex::CodexClient;
use commands::AppState;
use config::ConfigManager;
use tray::setup_tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config_manager = ConfigManager::new();
    let initial_settings = config_manager.load_settings();

    let app_state = Arc::new(AppState {
        client: Mutex::new(CodexClient::new()),
        settings_patch: Mutex::new(()),
        config_manager: config_manager.clone(),
        dock: dock::DockManager::new(config_manager.clone()),
        last_usage: Mutex::new(None),
        settings: Mutex::new(initial_settings.clone()),
        settings_revision: AtomicU64::new(0),
        autostart_update: Mutex::new(()),
        update_check: Mutex::new(()),
        update_install: Mutex::new(()),
        update_installing: std::sync::atomic::AtomicBool::new(false),
        pending_update: Mutex::new(None),
        prepared_update: std::sync::Mutex::new(None),
        available_update: Mutex::new(None),
        update_error: Mutex::new(None),
        last_auto_notified_version: Mutex::new(None),
        reset_state: Mutex::new(notify::load_reset_state(&config_manager)),
        pending_reset_fetches: Mutex::new(notify::PendingResetFetches::default()),
        #[cfg(windows)]
        audio: std::sync::OnceLock::new(),
    });

    let app_state_clone = app_state.clone();
    let window_state = app_state.clone();

    tauri::Builder::default()
        .on_window_event(move |window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. }
                    if window.label() == "settings" || window.label() == "tray-menu" =>
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
                tauri::WindowEvent::Focused(false) if window.label() == "tray-menu" => {
                    let _ = window.hide();
                }
                tauri::WindowEvent::ScaleFactorChanged { .. } if window.label() == "main" => {
                    let app = window.app_handle().clone();
                    let state = window_state.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(80)).await;
                        dock::keep_docked_in_work_area(&app, &state).await;
                    });
                }
                _ => {}
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
            commands::get_dock_state,
            commands::dock_mouse_enter,
            commands::dock_mouse_leave,
            commands::dock_window_resized,
            commands::exit_app,
            commands::show_overlay_menu,
            commands::toggle_overlay_window,
            commands::check_for_updates,
            commands::get_available_update,
            commands::get_update_error,
            commands::get_update_installing,
            commands::get_update_ready,
            commands::install_update,
        ])
        .setup(move |app| {
            // Setup system tray
            let tray_ready = match setup_tray(app.handle()) {
                Ok(_) => true,
                Err(error) => {
                    eprintln!("Could not start system tray: {error}");
                    false
                }
            };

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
                dock::restore_startup(&main_win, &app_state_clone.dock, &initial_settings);
                if let Err(error) = dock::install_native_window_hook(&main_win, app.handle()) {
                    eprintln!("Could not install the main window event hook: {error}");
                }
            }
            if initial_settings.mouse_passthrough {
                let restored = tray_ready
                    && commands::set_main_mouse_passthrough(app.handle(), true).is_ok();
                if !restored {
                    eprintln!("Saved mouse passthrough was not restored; keeping the overlay interactive");
                    let mut safe_settings = initial_settings.clone();
                    safe_settings.mouse_passthrough = false;
                    if let Err(error) = app_state_clone.config_manager.save_settings_checked(&safe_settings) {
                        eprintln!("Could not persist the safe mouse passthrough fallback: {error}");
                    }
                    if let Ok(mut settings) = app_state_clone.settings.try_lock() {
                        *settings = safe_settings;
                    }
                }
            }

            // Capture the actual end of a native drag, plus native menu open/close locks.
            let drag_state = app_state_clone.clone();
            app.listen("window_drag_started", move |_| drag_state.dock.begin_drag());
            let drag_state = app_state_clone.clone();
            let drag_app = app.handle().clone();
            app.listen("window_drag_ended", move |_| {
                let state = drag_state.clone();
                let app = drag_app.clone();
                tauri::async_runtime::spawn(async move { dock::drag_ended(&app, &state).await; });
            });
            let menu_state = app_state_clone.clone();
            app.listen("overlay_menu_opened", move |_| menu_state.dock.begin_menu());
            let menu_state = app_state_clone.clone();
            let menu_app = app.handle().clone();
            app.listen("overlay_menu_closed", move |_| {
                dock::menu_changed(&menu_app, &menu_state, false);
            });

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
                commands::install_prepared_update_on_exit(app, app.state::<Arc<AppState>>().inner());
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
