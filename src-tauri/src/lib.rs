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
        last_valid_settings_geometry: std::sync::Mutex::new(None),
        tray_menu_focus: std::sync::Mutex::new(tray::TrayMenuFocusState::default()),
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
                tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "settings" => {
                    api.prevent_close();
                    let cached = cached_settings_geometry(&window_state);
                    persist_settings_geometry(
                        &window_state.config_manager,
                        window.outer_position(),
                        window.inner_size(),
                        window.scale_factor().unwrap_or(1.0),
                        window.is_minimized().unwrap_or(false),
                        cached.as_ref(),
                    );
                    let _ = window.hide();
                }
                tauri::WindowEvent::CloseRequested { api, .. } if window.label() == tray::TRAY_MENU_WINDOW_LABEL => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // The only point where the "pre-minimize" geometry can still be
                // observed. A minimized window reports a 0x0 client rect (and
                // fires a degenerate `Resized`), so the reading is filtered and
                // only a healthy one refreshes the cache.
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
                    if window.label() == "settings" =>
                {
                    remember_valid_settings_geometry(
                        &window_state,
                        window.outer_position(),
                        window.inner_size(),
                        window.scale_factor().unwrap_or(1.0),
                        window.is_minimized().unwrap_or(false),
                    );
                }
                tauri::WindowEvent::Focused(false) if window.label() == tray::TRAY_MENU_WINDOW_LABEL => {
                    // Inside the post-show grace period the blur is only remembered:
                    // the grace timer re-checks the focus state and is the single
                    // decision point. A blur after the grace period is an ordinary
                    // dismissal and still hides immediately.
                    if tray::on_tray_menu_blur(window.app_handle()) {
                        let _ = window.hide();
                    }
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
                // The settings window is hidden (not destroyed) when the user closes
                // it, so a user who resizes it and then quits straight from the tray
                // never hits `CloseRequested`. Persist here as a fallback; the run
                // loop invokes this before windows are torn down.
                if let Some(window) = app.get_webview_window("settings") {
                    // Only when it is actually on screen: a never-opened settings
                    // window still reports the OS default placement, and persisting
                    // that would override the centered-on-first-open default.
                    //
                    // A minimized window still reports `is_visible() == true` on
                    // Windows while its client rect collapses to 0x0, so the live
                    // reading is rejected inside `persist_settings_geometry` and the
                    // last known-good geometry captured from `Moved`/`Resized` is
                    // written instead — that is what makes "resize, minimize, quit
                    // from the tray" restore correctly.
                    if window.is_visible().unwrap_or(false) {
                        let state = app.state::<Arc<AppState>>();
                        let cached = cached_settings_geometry(state.inner());
                        persist_settings_geometry(
                            &state.config_manager,
                            window.outer_position(),
                            window.inner_size(),
                            window.scale_factor().unwrap_or(1.0),
                            window.is_minimized().unwrap_or(false),
                            cached.as_ref(),
                        );
                    }
                }
                commands::shutdown_audio(app);
                commands::install_prepared_update_on_exit(app, app.state::<Arc<AppState>>().inner());
            }
        });
}

/// Converts a raw window reading into a persistable logical record.
///
/// Returns `None` for any unusable reading: a minimized window (on Windows it
/// still reports `is_visible() == true` while its client rect collapses to 0x0),
/// a degenerate zero-sized rect, an unreadable position/size, or a scale factor
/// that is not a positive finite number.
fn sanitize_geometry(
    position: Result<tauri::PhysicalPosition<i32>, tauri::Error>,
    size: Result<tauri::PhysicalSize<u32>, tauri::Error>,
    scale: f64,
    is_minimized: bool,
) -> Option<config::SettingsWindowGeometry> {
    if is_minimized {
        return None;
    }
    let (position, size) = (position.ok()?, size.ok()?);
    if size.width == 0 || size.height == 0 {
        return None;
    }
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    Some(config::SettingsWindowGeometry {
        x: position.x as f64 / scale,
        y: position.y as f64 / scale,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
        scale_factor: Some(scale),
    })
}

/// Remembers the settings window's current geometry as the last known-good value,
/// skipping minimized and degenerate readings so the cache can never be poisoned
/// by the unusable window state that follows a minimize.
fn remember_valid_settings_geometry(
    state: &Arc<AppState>,
    position: Result<tauri::PhysicalPosition<i32>, tauri::Error>,
    size: Result<tauri::PhysicalSize<u32>, tauri::Error>,
    scale: f64,
    is_minimized: bool,
) {
    remember_valid_settings_geometry_into(
        &state.last_valid_settings_geometry,
        position,
        size,
        scale,
        is_minimized,
    );
}

/// The cache half of `remember_valid_settings_geometry`, driven by the bare mutex
/// so the capture path can be exercised without a window, an `AppState` or an
/// event loop.
fn remember_valid_settings_geometry_into(
    cache: &std::sync::Mutex<Option<config::SettingsWindowGeometry>>,
    position: Result<tauri::PhysicalPosition<i32>, tauri::Error>,
    size: Result<tauri::PhysicalSize<u32>, tauri::Error>,
    scale: f64,
    is_minimized: bool,
) {
    let Some(geometry) = sanitize_geometry(position, size, scale, is_minimized) else {
        return;
    };
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(geometry);
    }
}

/// Reads the cached last known-good settings geometry, if any.
fn cached_settings_geometry(state: &Arc<AppState>) -> Option<config::SettingsWindowGeometry> {
    cached_settings_geometry_from(&state.last_valid_settings_geometry)
}

/// The cache half of `cached_settings_geometry`.
fn cached_settings_geometry_from(
    cache: &std::sync::Mutex<Option<config::SettingsWindowGeometry>>,
) -> Option<config::SettingsWindowGeometry> {
    cache.lock().ok().and_then(|guard| guard.clone())
}

/// Persists the settings window's current physical geometry as logical values plus
/// the scale factor of the display it sits on, so the restore path can rebuild the
/// exact physical rectangle even in mixed-DPI layouts.
///
/// The position/size/scale/is-minimized are passed in (rather than a window handle)
/// because the two call sites — `WindowEvent::CloseRequested` and `RunEvent::Exit`
/// — hand out different window types that only share these accessors.
///
/// A minimized window must never be persisted as-is: on Windows it still reports
/// `is_visible() == true` while its client rect collapses to 0x0, so writing that
/// would overwrite a previously valid record with an unusable one. The same applies
/// to any degenerate (zero-sized) reading. Both are therefore rejected by
/// `sanitize_geometry`; when that happens and a `cached` last known-good geometry
/// exists (captured by `remember_valid_settings_geometry` on `Moved`/`Resized`), the
/// cache is persisted instead so a resize-then-minimize-then-quit session still
/// restores the user's placement.
fn persist_settings_geometry(
    config_manager: &ConfigManager,
    position: Result<tauri::PhysicalPosition<i32>, tauri::Error>,
    size: Result<tauri::PhysicalSize<u32>, tauri::Error>,
    scale: f64,
    is_minimized: bool,
    cached: Option<&config::SettingsWindowGeometry>,
) {
    match sanitize_geometry(position, size, scale, is_minimized) {
        Some(geometry) => config_manager.save_settings_geometry(&geometry),
        None => {
            if let Some(geometry) = cached {
                config_manager.save_settings_geometry(geometry);
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::{PhysicalPosition, PhysicalSize};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "codex-usage-overlay-lib-geometry-{tag}-{}",
            std::process::id()
        ))
    }

    /// A fresh manager backed by an isolated temp directory, plus that directory so
    /// the caller can clean it up the way the config tests do.
    fn temp_manager(tag: &str) -> (ConfigManager, std::path::PathBuf) {
        let dir = temp_dir(tag);
        let _ = std::fs::remove_dir_all(&dir);
        (ConfigManager::with_runtime_dir(dir.clone()), dir)
    }

    /// A previously persisted, valid record that must survive any rejected write.
    fn valid_geometry() -> config::SettingsWindowGeometry {
        config::SettingsWindowGeometry {
            x: 120.0,
            y: 80.0,
            width: 480.0,
            height: 660.0,
            scale_factor: Some(1.0),
        }
    }

    fn assert_record_unchanged(manager: &ConfigManager) {
        let loaded = manager
            .load_settings_geometry()
            .expect("the previously saved geometry must still load");
        assert_eq!((loaded.x, loaded.y), (120.0, 80.0));
        assert_eq!((loaded.width, loaded.height), (480.0, 660.0));
        assert_eq!(loaded.scale_factor, Some(1.0));
    }

    #[test]
    fn persist_skips_zero_sized_geometry() {
        let (manager, dir) = temp_manager("persist-zero-size");
        manager.save_settings_geometry(&valid_geometry());

        // A minimized window reports 0x0 client size; persisting that would clobber
        // the valid record, so it has to be rejected.
        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(0, 0)),
            Ok(PhysicalSize::new(0, 0)),
            1.0,
            false,
            None,
        );
        assert_record_unchanged(&manager);

        // Either degenerate axis alone is enough to reject the write.
        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(10, 20)),
            Ok(PhysicalSize::new(0, 660)),
            1.0,
            false,
            None,
        );
        assert_record_unchanged(&manager);

        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(10, 20)),
            Ok(PhysicalSize::new(480, 0)),
            1.0,
            false,
            None,
        );
        assert_record_unchanged(&manager);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_skips_minimized_windows() {
        let (manager, dir) = temp_manager("persist-minimized");
        manager.save_settings_geometry(&valid_geometry());

        // Even if a size somehow reads as non-zero, a minimized window must be
        // treated as unusable and must not overwrite the stored record.
        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(10, 20)),
            Ok(PhysicalSize::new(480, 660)),
            1.0,
            true,
            None,
        );
        assert_record_unchanged(&manager);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_rejects_an_unusable_scale_factor() {
        let (manager, dir) = temp_manager("persist-bad-scale");
        manager.save_settings_geometry(&valid_geometry());

        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            persist_settings_geometry(
                &manager,
                Ok(PhysicalPosition::new(10, 20)),
                Ok(PhysicalSize::new(480, 660)),
                scale,
                false,
                None,
            );
            assert_record_unchanged(&manager);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_writes_valid_geometry_as_logical_values() {
        let (manager, dir) = temp_manager("persist-valid");

        // Positive control: a healthy reading must still be persisted, converted to
        // logical units with the display's scale factor.
        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(300, 400)),
            Ok(PhysicalSize::new(960, 1320)),
            2.0,
            false,
            None,
        );

        let loaded = manager
            .load_settings_geometry()
            .expect("geometry should be saved");
        assert_eq!((loaded.x, loaded.y), (150.0, 200.0));
        assert_eq!((loaded.width, loaded.height), (480.0, 660.0));
        assert_eq!(loaded.scale_factor, Some(2.0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The geometry captured while the window was still healthy — i.e. what the
    /// settings window's `Moved`/`Resized` handler stores before the user minimizes.
    fn captured_geometry() -> config::SettingsWindowGeometry {
        config::SettingsWindowGeometry {
            x: 42.0,
            y: 24.0,
            width: 520.0,
            height: 700.0,
            scale_factor: Some(1.0),
        }
    }

    fn assert_record_is_captured(manager: &ConfigManager) {
        let loaded = manager
            .load_settings_geometry()
            .expect("the captured geometry must have been persisted");
        assert_eq!((loaded.x, loaded.y), (42.0, 24.0));
        assert_eq!((loaded.width, loaded.height), (520.0, 700.0));
        assert_eq!(loaded.scale_factor, Some(1.0));
    }

    /// CR3-1: a session that resizes the settings window, minimizes it and then
    /// quits has no usable live reading at persist time. The geometry captured on
    /// `Moved`/`Resized` must be what lands on disk — replacing a stale record.
    #[test]
    fn persist_writes_captured_geometry_when_minimized() {
        let (manager, dir) = temp_manager("persist-minimized-capture");
        // A stale record from a previous session must be replaced, not kept.
        manager.save_settings_geometry(&valid_geometry());
        let captured = captured_geometry();

        // The real Windows reading for a minimized window: the client rect
        // collapses to 0x0 and the position is meaningless.
        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(0, 0)),
            Ok(PhysicalSize::new(0, 0)),
            1.0,
            true,
            Some(&captured),
        );
        assert_record_is_captured(&manager);

        // Even if the OS hands back a plausible-looking reading while minimized,
        // the capture still wins: the `is_minimized` guard must not be relaxed.
        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(999, 999)),
            Ok(PhysicalSize::new(300, 300)),
            1.0,
            true,
            Some(&captured),
        );
        assert_record_is_captured(&manager);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round 2 acceptance #1: a brand-new profile (no record on disk yet) that
    /// resizes, minimizes and quits must restore instead of falling back to the
    /// centered default.
    #[test]
    fn persist_writes_captured_geometry_for_a_fresh_profile() {
        let (manager, dir) = temp_manager("persist-minimized-fresh");
        assert!(manager.load_settings_geometry().is_none());

        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(0, 0)),
            Ok(PhysicalSize::new(0, 0)),
            1.0,
            true,
            Some(&captured_geometry()),
        );
        assert_record_is_captured(&manager);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Negative control: with no capture available, a minimized reading must still
    /// write nothing rather than inventing a record.
    #[test]
    fn persist_skips_minimized_window_without_a_capture() {
        let (manager, dir) = temp_manager("persist-minimized-no-capture");
        assert!(manager.load_settings_geometry().is_none());

        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(0, 0)),
            Ok(PhysicalSize::new(0, 0)),
            1.0,
            true,
            None,
        );
        assert!(
            manager.load_settings_geometry().is_none(),
            "a minimized window with no capture must not invent a record"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The capture side must reject the unusable readings too, otherwise a
    /// degenerate sample taken right after minimizing would poison the cache and
    /// defeat the fallback.
    #[test]
    fn sanitize_rejects_unusable_readings_so_the_capture_survives() {
        let healthy = sanitize_geometry(
            Ok(PhysicalPosition::new(300, 400)),
            Ok(PhysicalSize::new(960, 1320)),
            2.0,
            false,
        )
        .expect("a healthy reading must be usable");
        assert_eq!((healthy.x, healthy.y), (150.0, 200.0));
        assert_eq!((healthy.width, healthy.height), (480.0, 660.0));

        assert!(sanitize_geometry(
            Ok(PhysicalPosition::new(10, 20)),
            Ok(PhysicalSize::new(480, 660)),
            1.0,
            true,
        )
        .is_none());
        assert!(sanitize_geometry(
            Ok(PhysicalPosition::new(10, 20)),
            Ok(PhysicalSize::new(0, 0)),
            1.0,
            false,
        )
        .is_none());
    }

    /// A state that exists only to hold the geometry cache: no window, no event
    /// loop, no network. It is built through the real `AppState` so these cases go
    /// through the same capture/read functions the window handlers call.
    fn test_state(tag: &str) -> (Arc<AppState>, std::path::PathBuf) {
        let (manager, dir) = temp_manager(tag);
        (Arc::new(AppState::for_test(manager)), dir)
    }

    /// A healthy reading: 300x400 @2.0 -> 150x200 logical, 960x1320 @2.0 -> 480x660.
    fn capture_healthy(state: &Arc<AppState>) {
        remember_valid_settings_geometry(
            state,
            Ok(PhysicalPosition::new(300, 400)),
            Ok(PhysicalSize::new(960, 1320)),
            2.0,
            false,
        );
    }

    fn assert_cache_is_healthy(state: &Arc<AppState>) {
        let cached = cached_settings_geometry(state).expect("the healthy reading must be cached");
        assert_eq!((cached.x, cached.y), (150.0, 200.0));
        assert_eq!((cached.width, cached.height), (480.0, 660.0));
        assert_eq!(cached.scale_factor, Some(2.0));
    }

    /// CR3-1 wiring, part 1: the capture point must actually fill the cache and the
    /// read must hand back exactly what was captured. Fails if
    /// `remember_valid_settings_geometry` (or the cache half it delegates to) is
    /// turned into a no-op, or if `cached_settings_geometry` stops reading.
    #[test]
    fn capture_fills_the_cache_and_read_back_matches() {
        let (state, dir) = test_state("cache-capture");

        assert!(
            cached_settings_geometry(&state).is_none(),
            "a session that never captured anything must not report a cached geometry"
        );

        capture_healthy(&state);
        assert_cache_is_healthy(&state);

        // A later healthy reading replaces the previous one.
        remember_valid_settings_geometry(
            &state,
            Ok(PhysicalPosition::new(100, 200)),
            Ok(PhysicalSize::new(1000, 1200)),
            2.0,
            false,
        );
        let cached = cached_settings_geometry(&state).expect("the second reading must be cached");
        assert_eq!((cached.x, cached.y), (50.0, 100.0));
        assert_eq!((cached.width, cached.height), (500.0, 600.0));
        assert_eq!(cached.scale_factor, Some(2.0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CR3-1 wiring, part 2: every unusable sample must be dropped by the capture
    /// point. If any of them got through, the first degenerate reading after a
    /// minimize would poison the cache and the minimize-then-quit fallback would
    /// persist garbage.
    #[test]
    fn capture_never_poisons_the_cache_with_an_unusable_reading() {
        let (state, dir) = test_state("cache-not-poisoned");
        capture_healthy(&state);
        assert_cache_is_healthy(&state);

        // Minimized: Windows reports a 0x0 rect at a meaningless position.
        remember_valid_settings_geometry(
            &state,
            Ok(PhysicalPosition::new(-32000, -32000)),
            Ok(PhysicalSize::new(0, 0)),
            1.0,
            true,
        );
        assert_cache_is_healthy(&state);

        // Minimized with a plausible-looking rect: `is_minimized` alone must reject it.
        remember_valid_settings_geometry(
            &state,
            Ok(PhysicalPosition::new(999, 999)),
            Ok(PhysicalSize::new(300, 300)),
            1.0,
            true,
        );
        assert_cache_is_healthy(&state);

        // Degenerate rects while not minimized.
        for size in [
            PhysicalSize::new(0, 0),
            PhysicalSize::new(0, 660),
            PhysicalSize::new(480, 0),
        ] {
            remember_valid_settings_geometry(
                &state,
                Ok(PhysicalPosition::new(10, 20)),
                Ok(size),
                1.0,
                false,
            );
            assert_cache_is_healthy(&state);
        }

        // Unusable scale factors.
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            remember_valid_settings_geometry(
                &state,
                Ok(PhysicalPosition::new(10, 20)),
                Ok(PhysicalSize::new(480, 660)),
                scale,
                false,
            );
            assert_cache_is_healthy(&state);
        }

        // An unreadable accessor result is dropped as well.
        remember_valid_settings_geometry(
            &state,
            Err(tauri::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "unreadable",
            ))),
            Ok(PhysicalSize::new(480, 660)),
            1.0,
            false,
        );
        assert_cache_is_healthy(&state);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CR3-1 wiring, part 3: the end-of-session fallback must go through the same
    /// read entry point the handlers use, not a cache passed in by the caller.
    #[test]
    fn persist_falls_back_to_the_cache_through_the_read_wiring() {
        let (manager, dir) = temp_manager("persist-cache-wiring");
        let state = Arc::new(AppState::for_test(manager.clone()));

        remember_valid_settings_geometry(
            &state,
            Ok(PhysicalPosition::new(42, 24)),
            Ok(PhysicalSize::new(520, 700)),
            1.0,
            false,
        );

        // Minimized at quit time, so the live reading is unusable.
        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(-32000, -32000)),
            Ok(PhysicalSize::new(0, 0)),
            1.0,
            true,
            cached_settings_geometry(&state).as_ref(),
        );
        assert_record_is_captured(&manager);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The live reading must win over the cache whenever it is usable: the cache
    /// only exists to stand in for an unusable reading. Reversing that priority
    /// persists the stale placement instead and fails here.
    #[test]
    fn persist_prefers_the_live_reading_over_a_non_empty_cache() {
        let (manager, dir) = temp_manager("persist-live-wins");
        let state = Arc::new(AppState::for_test(manager.clone()));

        // The placement captured earlier in the session...
        remember_valid_settings_geometry(
            &state,
            Ok(PhysicalPosition::new(42, 24)),
            Ok(PhysicalSize::new(520, 700)),
            1.0,
            false,
        );
        let cached = cached_settings_geometry(&state);
        assert!(
            cached.is_some(),
            "this case is only meaningful while the cache is non-empty"
        );

        // ...versus the healthy live reading taken at persist time.
        persist_settings_geometry(
            &manager,
            Ok(PhysicalPosition::new(300, 400)),
            Ok(PhysicalSize::new(960, 1320)),
            2.0,
            false,
            cached.as_ref(),
        );

        let loaded = manager
            .load_settings_geometry()
            .expect("the live reading must have been persisted");
        assert_eq!((loaded.x, loaded.y), (150.0, 200.0));
        assert_eq!((loaded.width, loaded.height), (480.0, 660.0));
        assert_eq!(loaded.scale_factor, Some(2.0));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
