use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Wry,
};

pub const TRAY_ID: &str = "main-tray";

pub fn setup_tray(app: &AppHandle, autostart_enabled: bool, is_installed: bool) -> Result<TrayIcon<Wry>, tauri::Error> {
    let toggle_i = MenuItem::with_id(app, "toggle_overlay", "显示/隐藏悬浮窗", true, None::<&str>)?;
    let refresh_i = MenuItem::with_id(app, "refresh_usage", "立即刷新用量", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "open_settings", "浮窗设置…", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let exit_i = MenuItem::with_id(app, "exit_app", "退出悬浮窗", true, None::<&str>)?;

    let menu = if is_installed {
        let autostart_i = CheckMenuItem::with_id(
            app,
            "toggle_autostart",
            "开机时自动启动",
            true,
            autostart_enabled,
            None::<&str>,
        )?;
        Menu::with_items(
            app,
            &[
                &toggle_i,
                &refresh_i,
                &sep1,
                &settings_i,
                &sep2,
                &autostart_i,
                &sep3,
                &exit_i,
            ],
        )?
    } else {
        Menu::with_items(
            app,
            &[
                &toggle_i,
                &refresh_i,
                &sep1,
                &settings_i,
                &sep2,
                &exit_i,
            ],
        )?
    };

    // Initial dual-ring gauge icon
    let initial_icon = generate_dual_ring_icon(None, None);

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Codex 用量悬浮窗")
        .icon(initial_icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                toggle_main_window(app);
            }
        })
        .build(app)?;

    Ok(tray)
}

pub fn handle_menu_action(app: &AppHandle, id: &str) {
    match id {
        "toggle_overlay" => {
            toggle_main_window(app);
        }
        "refresh_usage" => {
            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = app_clone.emit("trigger_refresh", ());
            });
        }
        "open_settings" => {
            open_settings_window(app);
        }
        "toggle_autostart" => {
            if !crate::config::is_installed_environment() {
                return;
            }
            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_autostart::ManagerExt;
                let autostart_mgr = app_clone.autolaunch();
                if let Ok(is_enabled) = autostart_mgr.is_enabled() {
                    let new_state = !is_enabled;
                    if new_state {
                        let _ = autostart_mgr.enable();
                    } else {
                        let _ = autostart_mgr.disable();
                    }
                    if let Some(state) = app_clone.try_state::<std::sync::Arc<crate::commands::AppState>>() {
                        let mut settings = state.settings.lock().await;
                        settings.auto_start = new_state;
                        state.config_manager.save_settings(&settings);
                        let _ = app_clone.emit("settings_updated", &*settings);
                    }
                }
            });
        }
        "exit_app" => {
            app.exit(0);
        }
        _ => {}
    }
}

pub fn toggle_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(is_visible) = window.is_visible() {
            if is_visible {
                let _ = window.hide();
            } else {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    }
}

pub fn open_settings_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        use tauri::{WebviewUrl, WebviewWindowBuilder};
        let _ = WebviewWindowBuilder::new(
            app,
            "settings",
            WebviewUrl::App("index.html#settings".into()),
        )
        .title("Overlay Settings")
        .inner_size(380.0, 560.0)
        .min_inner_size(380.0, 350.0)
        .max_inner_size(380.0, 900.0)
        .resizable(true)
        .maximizable(false)
        .decorations(true)
        .always_on_top(false)
        .skip_taskbar(false)
        .build();
    }
}

pub fn update_tray_tooltip(app: &AppHandle, text: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let truncated = if text.len() > 63 {
            &text[..63]
        } else {
            text
        };
        let _ = tray.set_tooltip(Some(truncated));
    }
}

pub fn update_tray_icon(app: &AppHandle, five_hour_pct: Option<f64>, week_pct: Option<f64>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let icon = generate_dual_ring_icon(five_hour_pct, week_pct);
        let _ = tray.set_icon(Some(icon));
    }
}

pub fn generate_dual_ring_icon(
    five_hour_pct: Option<f64>,
    week_pct: Option<f64>,
) -> tauri::image::Image<'static> {
    const SIZE: usize = 32;
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);

    for y in 0..SIZE {
        for x in 0..SIZE {
            // 2x2 supersampling for crisp, buttery anti-aliasing
            let sub_offsets = [0.25, 0.75];
            let mut accum = [0.0; 4];
            for &sy in &sub_offsets {
                for &sx in &sub_offsets {
                    let sample = sample_pixel(
                        x as f64 + sx,
                        y as f64 + sy,
                        five_hour_pct,
                        week_pct,
                    );
                    accum[0] += sample[0];
                    accum[1] += sample[1];
                    accum[2] += sample[2];
                    accum[3] += sample[3];
                }
            }
            rgba.push((accum[0] / 4.0).round().clamp(0.0, 255.0) as u8);
            rgba.push((accum[1] / 4.0).round().clamp(0.0, 255.0) as u8);
            rgba.push((accum[2] / 4.0).round().clamp(0.0, 255.0) as u8);
            rgba.push((accum[3] / 4.0).round().clamp(0.0, 255.0) as u8);
        }
    }

    tauri::image::Image::new_owned(rgba, SIZE as u32, SIZE as u32)
}

fn sample_pixel(
    x: f64,
    y: f64,
    five_hour_pct: Option<f64>,
    week_pct: Option<f64>,
) -> [f64; 4] {
    let cx = 15.5;
    let cy = 15.5;
    let dx = x - cx;
    let dy = y - cy;
    let r = (dx * dx + dy * dy).sqrt();

    // Outside the backing plate circle
    if r > 15.2 {
        return [0.0, 0.0, 0.0, 0.0];
    }

    // Plate edge antialias factor
    let plate_alpha = if r > 14.7 {
        ((15.2 - r) / 0.5).clamp(0.0, 1.0)
    } else {
        1.0
    };

    // Calculate clockwise angle from 12 o'clock in [0, 2*PI]
    // Top is (0, -r), so -dy is positive
    let angle = dx.atan2(-dy);
    let cw_angle = if angle < 0.0 {
        angle + 2.0 * std::f64::consts::PI
    } else {
        angle
    };

    // Base dark backing plate color: #1B1E26
    let mut color = [27.0, 30.0, 38.0, 255.0 * plate_alpha];

    // Subtle plate rim
    if r >= 14.2 && r <= 15.0 {
        color = [50.0, 58.0, 72.0, 255.0 * plate_alpha];
    }

    // Outer ring: radius 12.0, width 2.4 (from 10.8 to 13.2)
    if r >= 10.8 && r <= 13.2 {
        let max_angle = match five_hour_pct {
            Some(p) => 2.0 * std::f64::consts::PI * (p.clamp(0.0, 100.0) / 100.0),
            None => 0.0,
        };
        let is_active = five_hour_pct.is_some() && cw_angle <= max_angle;
        let c = if is_active {
            get_color_for_percent(five_hour_pct)
        } else {
            [48.0, 54.0, 68.0, 220.0] // Inactive / depleted track
        };
        color = blend_over(color, c);
    }

    // Inner ring: radius 7.5, width 2.2 (from 6.4 to 8.6)
    if r >= 6.4 && r <= 8.6 {
        let max_angle = match week_pct {
            Some(p) => 2.0 * std::f64::consts::PI * (p.clamp(0.0, 100.0) / 100.0),
            None => 0.0,
        };
        let is_active = week_pct.is_some() && cw_angle <= max_angle;
        let c = if is_active {
            get_color_for_percent(week_pct)
        } else {
            [48.0, 54.0, 68.0, 220.0] // Inactive / depleted track
        };
        color = blend_over(color, c);
    }

    // Center core dot: radius <= 1.8
    if r <= 1.8 {
        color = blend_over(color, [100.0, 225.0, 255.0, 240.0]); // Soft cyan core
    }

    color
}

fn get_color_for_percent(pct: Option<f64>) -> [f64; 4] {
    match pct {
        Some(p) if p >= 50.0 => [82.0, 210.0, 115.0, 255.0], // Green #52D273
        Some(p) if p >= 20.0 => [255.0, 200.0, 87.0, 255.0], // Amber #FFC857
        Some(_) => [255.0, 92.0, 92.0, 255.0],                // Red #FF5C5C
        None => [120.0, 130.0, 145.0, 200.0],
    }
}

fn blend_over(dst: [f64; 4], src: [f64; 4]) -> [f64; 4] {
    let src_a = src[3] / 255.0;
    let dst_a = dst[3] / 255.0;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a <= 0.0 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let r = (src[0] * src_a + dst[0] * dst_a * (1.0 - src_a)) / out_a;
    let g = (src[1] * src_a + dst[1] * dst_a * (1.0 - src_a)) / out_a;
    let b = (src[2] * src_a + dst[2] * dst_a * (1.0 - src_a)) / out_a;
    [r, g, b, out_a * 255.0]
}
