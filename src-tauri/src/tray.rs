use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex as StdMutex;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewUrl,
    WebviewWindowBuilder, Wry,
};

pub const TRAY_ID: &str = "main-tray";
static TRAY_MENU_GENERATION: AtomicU64 = AtomicU64::new(0);
static TRAY_MENU_LAYOUT_REVISION: AtomicU64 = AtomicU64::new(0);
static TRAY_MENU_LAYOUT_LOCK: StdMutex<()> = StdMutex::new(());

pub fn setup_tray(app: &AppHandle) -> Result<TrayIcon<Wry>, tauri::Error> {
    // Initial dual-ring gauge icon
    let initial_icon = generate_dual_ring_icon(None, None);

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Codex Usage Overlay")
        .icon(initial_icon)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, .. } = event {
                let app = tray.app_handle();
                match button {
                    MouseButton::Left => toggle_main_window(app),
                    MouseButton::Right => open_tray_menu_window(app),
                    _ => {}
                }
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
        "exit_app" => {
            crate::commands::shutdown_audio(app);
            app.exit(0);
        }
        _ => {}
    }
}

pub fn open_tray_menu_window(app: &AppHandle) {
    let Ok(_layout_guard) = TRAY_MENU_LAYOUT_LOCK.lock() else { return };
    let generation = TRAY_MENU_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    TRAY_MENU_LAYOUT_REVISION.store(0, Ordering::SeqCst);
    let Some(_tray) = app.tray_by_id(TRAY_ID) else { return };
    let window = if let Some(window) = app.get_webview_window("tray-menu") {
        window
    } else {
        match WebviewWindowBuilder::new(
            app,
            "tray-menu",
            WebviewUrl::App("index.html#tray-menu".into()),
        )
        .title("Tray Menu")
        .inner_size(300.0, 40.0)
        .resizable(false)
        .maximizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .shadow(true)
        .build()
        {
            Ok(window) => window,
            Err(_) => return,
        }
    };
    let _ = window.hide();
    drop(_layout_guard);
    let _ = window.emit("tray_menu_opened", generation);
}

pub fn tray_menu_generation() -> u64 {
    TRAY_MENU_GENERATION.load(Ordering::SeqCst)
}

pub fn layout_tray_menu(app: &AppHandle, generation: u64, revision: u64, height_logical: f64) -> Result<f64, String> {
    let _layout_guard = TRAY_MENU_LAYOUT_LOCK.lock().map_err(|e| e.to_string())?;
    if generation != tray_menu_generation()
        || revision <= TRAY_MENU_LAYOUT_REVISION.load(Ordering::SeqCst)
        || !height_logical.is_finite()
        || height_logical <= 0.0 {
        return Err("Stale or invalid tray menu layout request".into());
    }
    TRAY_MENU_LAYOUT_REVISION.store(revision, Ordering::SeqCst);
    let window = app.get_webview_window("tray-menu").ok_or("Tray menu window unavailable")?;
    let tray = app.tray_by_id(TRAY_ID).ok_or("Tray icon unavailable")?;
    let rect = tray.rect().map_err(|e| e.to_string())?;
    let fallback_monitor = window.current_monitor().ok().flatten().or_else(|| window.primary_monitor().ok().flatten());
    let fallback_scale = fallback_monitor.as_ref().map(tauri::Monitor::scale_factor).unwrap_or(1.0);
    let (icon_x, icon_y, icon_width, icon_height) = if let Some(rect) = rect {
        let x = match rect.position {
            Position::Physical(p) => p.x,
            Position::Logical(p) => (p.x * fallback_scale).round() as i32,
        };
        let y = match rect.position {
            Position::Physical(p) => p.y,
            Position::Logical(p) => (p.y * fallback_scale).round() as i32,
        };
        let width = match rect.size {
            Size::Physical(s) => s.width as i32,
            Size::Logical(s) => (s.width * fallback_scale).round() as i32,
        };
        let height = match rect.size {
            Size::Physical(s) => s.height as i32,
            Size::Logical(s) => (s.height * fallback_scale).round() as i32,
        };
        (x, y, width, height)
    } else {
        let work = fallback_monitor.as_ref().ok_or("Tray monitor unavailable")?.work_area();
        (work.position.x + work.size.width as i32, work.position.y + work.size.height as i32, 0, 0)
    };
    let center_x = icon_x + icon_width / 2;
    let center_y = icon_y + icon_height / 2;
    let monitor = app.available_monitors().map_err(|e| e.to_string())?.into_iter().find(|monitor| {
        let p = monitor.position();
        let s = monitor.size();
        center_x >= p.x && center_x < p.x + s.width as i32
            && center_y >= p.y && center_y < p.y + s.height as i32
    }).or(fallback_monitor).ok_or("Tray monitor unavailable")?;
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    let margin = (8.0 * scale).ceil() as i32;
    let width = (300.0 * scale).ceil() as i32;
    let width = width.min(work.size.width as i32 - margin * 2).max(1);
    let desired_height = (height_logical * scale).ceil() as i32;
    let height = desired_height.min(work.size.height as i32 - margin * 2).max(1);
    let left = work.position.x;
    let top = work.position.y;
    let right = left + work.size.width as i32;
    let bottom = top + work.size.height as i32;
    let (x, y) = if icon_x + icon_width <= left {
        (left + margin, icon_y + icon_height - height)
    } else if icon_x >= right {
        (right - width - margin, icon_y + icon_height - height)
    } else if icon_y + icon_height <= top {
        (icon_x + icon_width - width, top + margin)
    } else {
        (icon_x + icon_width - width, icon_y - height - margin)
    };
    let x = x.clamp(left, (right - width).max(left));
    let y = y.clamp(top, (bottom - height).max(top));
    window.set_size(Size::Physical(PhysicalSize::new(width as u32, height as u32))).map_err(|e| e.to_string())?;
    window.set_position(Position::Physical(PhysicalPosition::new(x, y))).map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(height as f64 / scale)
}

pub fn toggle_main_window(app: &AppHandle) {
    if let Some(state) = app.try_state::<std::sync::Arc<crate::commands::AppState>>() {
        let state = state.inner().clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::dock::toggle_overlay_window(&app, &state).await;
        });
    } else if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
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
        .title("Settings")
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
