use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex as StdMutex;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Position, Size,
    WebviewUrl, WebviewWindowBuilder, Wry,
};

pub const TRAY_ID: &str = "main-tray";
const SETTINGS_DEFAULT_WIDTH: f64 = 480.0;
const SETTINGS_DEFAULT_HEIGHT: f64 = 660.0;
const SETTINGS_MIN_WIDTH: f64 = 380.0;
const SETTINGS_MIN_HEIGHT: f64 = 400.0;
const SETTINGS_MAX_WIDTH: f64 = 1600.0;
const SETTINGS_MAX_HEIGHT: f64 = 1600.0;
/// Safety bounds for the tray menu popup width, in logical pixels.
const TRAY_MENU_MIN_WIDTH: f64 = 300.0;
const TRAY_MENU_MAX_WIDTH: f64 = 500.0;
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
                    MouseButton::Left | MouseButton::Right => open_tray_menu_window(app),
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

pub fn layout_tray_menu(
    app: &AppHandle,
    generation: u64,
    revision: u64,
    height_logical: f64,
    width_logical: f64,
) -> Result<f64, String> {
    let _layout_guard = TRAY_MENU_LAYOUT_LOCK.lock().map_err(|e| e.to_string())?;
    if generation != tray_menu_generation()
        || revision <= TRAY_MENU_LAYOUT_REVISION.load(Ordering::SeqCst)
        || !height_logical.is_finite()
        || height_logical <= 0.0
        || !width_logical.is_finite()
        || width_logical <= 0.0 {
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
    let desired_width = width_logical.clamp(TRAY_MENU_MIN_WIDTH, TRAY_MENU_MAX_WIDTH);
    let width = (desired_width * scale).ceil() as i32;
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
    let (window, created) = if let Some(window) = app.get_webview_window("settings") {
        (window, false)
    } else {
        match WebviewWindowBuilder::new(
            app,
            "settings",
            WebviewUrl::App("index.html#settings".into()),
        )
        .title("Settings")
        .inner_size(SETTINGS_DEFAULT_WIDTH, SETTINGS_DEFAULT_HEIGHT)
        .min_inner_size(SETTINGS_MIN_WIDTH, SETTINGS_MIN_HEIGHT)
        .max_inner_size(SETTINGS_MAX_WIDTH, SETTINGS_MAX_HEIGHT)
        .resizable(true)
        .maximizable(false)
        .decorations(true)
        .always_on_top(false)
        .skip_taskbar(false)
        .build()
        {
            Ok(window) => (window, true),
            Err(_) => return,
        }
    };
    let was_visible = window.is_visible().unwrap_or(false);
    let _ = window.unminimize();
    // Only restore persisted geometry when the window is (re)appearing; if it is
    // already on screen, keep wherever the user has currently placed it.
    if created || !was_visible {
        apply_settings_geometry(app, &window);
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// Places the settings window using its persisted geometry when it still lands on a
/// connected monitor's work area (clamped inside); otherwise falls back to the
/// default size centered on the primary monitor, so a detached display can never
/// strand the window off-screen.
fn apply_settings_geometry(app: &AppHandle, window: &tauri::WebviewWindow) {
    let saved = app
        .try_state::<std::sync::Arc<crate::commands::AppState>>()
        .and_then(|state| state.config_manager.load_settings_geometry());

    let Some(geometry) = saved else {
        center_settings_window(window);
        return;
    };

    let work_areas: Vec<MonitorWorkArea> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(monitor_work_area)
        .collect();

    let Some(placement) = plan_settings_geometry(&work_areas, &geometry) else {
        center_settings_window(window);
        return;
    };

    let _ = window.set_size(Size::Physical(PhysicalSize::new(
        placement.width.round().max(1.0) as u32,
        placement.height.round().max(1.0) as u32,
    )));
    let _ = window.set_position(Position::Physical(PhysicalPosition::new(
        placement.x.round() as i32,
        placement.y.round() as i32,
    )));
}

fn center_settings_window(window: &tauri::WebviewWindow) {
    let _ = window.set_size(Size::Logical(LogicalSize::new(
        SETTINGS_DEFAULT_WIDTH,
        SETTINGS_DEFAULT_HEIGHT,
    )));
    let _ = window.center();
}

fn monitor_work_area(monitor: &tauri::Monitor) -> MonitorWorkArea {
    let work = monitor.work_area();
    MonitorWorkArea {
        left: work.position.x as f64,
        top: work.position.y as f64,
        width: work.size.width as f64,
        height: work.size.height as f64,
        scale_factor: monitor.scale_factor(),
    }
}

/// Axis-aligned rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl PhysicalRect {
    pub fn new(left: f64, top: f64, width: f64, height: f64) -> Self {
        Self { left, top, width, height }
    }

    pub fn right(&self) -> f64 {
        self.left + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }

    /// Overlapping area with `other`; zero when they only touch or miss entirely.
    pub fn intersection_area(&self, other: &PhysicalRect) -> f64 {
        let width = (self.right().min(other.right()) - self.left.max(other.left)).max(0.0);
        let height = (self.bottom().min(other.bottom()) - self.top.max(other.top)).max(0.0);
        width * height
    }
}

/// A monitor work area in physical pixels, decoupled from `tauri::Monitor` so the
/// placement maths stays pure and unit-testable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorWorkArea {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
}

impl MonitorWorkArea {
    pub fn rect(&self) -> PhysicalRect {
        PhysicalRect::new(self.left, self.top, self.width, self.height)
    }

    fn has_usable_scale(&self) -> bool {
        self.scale_factor.is_finite() && self.scale_factor > 0.0
    }
}

/// The resolved physical geometry to apply to the settings window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettingsPlacement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Projects the persisted logical geometry into physical pixels. When the record
/// carries the scale factor of the monitor it was saved on, that one factor is used
/// for every candidate monitor — which is what makes the comparison below
/// independent of enumeration order. Records written before the field existed fall
/// back to the candidate's own factor (the historical behaviour).
fn projected_physical_rect(
    geometry: &crate::config::SettingsWindowGeometry,
    candidate_scale: f64,
) -> PhysicalRect {
    let scale = geometry
        .scale_factor
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(candidate_scale);
    PhysicalRect::new(
        geometry.x * scale,
        geometry.y * scale,
        geometry.width * scale,
        geometry.height * scale,
    )
}

/// Picks the index of the monitor whose work area overlaps the saved geometry by the
/// largest non-zero area. Comparing areas (instead of taking the first hit) removes
/// both the false positive produced by projecting with the wrong scale factor
/// and any dependency on monitor enumeration order.
pub fn select_work_area(
    work_areas: &[MonitorWorkArea],
    geometry: &crate::config::SettingsWindowGeometry,
) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, work) in work_areas.iter().enumerate() {
        if !work.has_usable_scale() {
            continue;
        }
        let area = projected_physical_rect(geometry, work.scale_factor)
            .intersection_area(&work.rect());
        let is_better = match best {
            Some((_, best_area)) => area > best_area,
            None => true,
        };
        if area > 0.0 && is_better {
            best = Some((index, area));
        }
    }
    best.map(|(index, _)| index)
}

fn place_within_work_area(
    work: &MonitorWorkArea,
    geometry: &crate::config::SettingsWindowGeometry,
) -> SettingsPlacement {
    let scale = if work.has_usable_scale() { work.scale_factor } else { 1.0 };
    let left = work.left;
    let top = work.top;
    let right = work.left + work.width;
    let bottom = work.top + work.height;

    let min_width = SETTINGS_MIN_WIDTH * scale;
    let min_height = SETTINGS_MIN_HEIGHT * scale;
    let max_width = (right - left).min(SETTINGS_MAX_WIDTH * scale).max(min_width);
    let max_height = (bottom - top).min(SETTINGS_MAX_HEIGHT * scale).max(min_height);
    let width = (geometry.width * scale).clamp(min_width, max_width);
    let height = (geometry.height * scale).clamp(min_height, max_height);
    let x = (geometry.x * scale).clamp(left, (right - width).max(left));
    let y = (geometry.y * scale).clamp(top, (bottom - height).max(top));

    SettingsPlacement { x, y, width, height }
}

/// Resolves the saved geometry to a physical placement, or `None` when no connected
/// work area overlaps it (e.g. the monitor it lived on was detached), in which case
/// the caller centres the window on the primary monitor.
pub fn plan_settings_geometry(
    work_areas: &[MonitorWorkArea],
    geometry: &crate::config::SettingsWindowGeometry,
) -> Option<SettingsPlacement> {
    let index = select_work_area(work_areas, geometry)?;
    Some(place_within_work_area(&work_areas[index], geometry))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SettingsWindowGeometry;

    /// Primary display 1920x1080 @100% at the origin, secondary 2560x1440 @150%
    /// placed to its right — the mixed-DPI layout from the review handoff.
    fn mixed_dpi_layout() -> [MonitorWorkArea; 2] {
        [
            MonitorWorkArea { left: 0.0, top: 0.0, width: 1920.0, height: 1080.0, scale_factor: 1.0 },
            MonitorWorkArea { left: 1920.0, top: 0.0, width: 2560.0, height: 1440.0, scale_factor: 1.5 },
        ]
    }

    /// Window lived on the secondary display: physical (2000, 200), inner size
    /// 720x990, i.e. logical (1333.33, 133.33, 480, 660) at scale 1.5.
    fn geometry_on_secondary() -> SettingsWindowGeometry {
        SettingsWindowGeometry {
            x: 1333.33,
            y: 133.33,
            width: 480.0,
            height: 660.0,
            scale_factor: Some(1.5),
        }
    }

    #[test]
    fn mixed_dpi_selects_the_saved_display() {
        let areas = mixed_dpi_layout();
        assert_eq!(select_work_area(&areas, &geometry_on_secondary()), Some(1));
    }

    #[test]
    fn mixed_dpi_selection_is_independent_of_enumeration_order() {
        let forward = mixed_dpi_layout();
        let mut reversed = mixed_dpi_layout();
        reversed.reverse();

        let forward_pick = select_work_area(&forward, &geometry_on_secondary());
        let reversed_pick = select_work_area(&reversed, &geometry_on_secondary());

        assert_eq!(forward_pick, Some(1));
        assert_eq!(reversed_pick, Some(0));
        // Both orders must land on the same physical monitor (the 150% secondary).
        assert_eq!(forward[forward_pick.unwrap()], reversed[reversed_pick.unwrap()]);
    }

    #[test]
    fn mixed_dpi_restores_physical_position_and_logical_size() {
        let areas = mixed_dpi_layout();
        let placement = plan_settings_geometry(&areas, &geometry_on_secondary()).expect("placement");
        assert!((placement.x - 2000.0).abs() < 1.0, "x was {}", placement.x);
        assert!((placement.y - 200.0).abs() < 1.0, "y was {}", placement.y);
        assert!((placement.width - 720.0).abs() < 0.5, "width was {}", placement.width);
        assert!((placement.height - 990.0).abs() < 0.5, "height was {}", placement.height);
    }

    #[test]
    fn overlap_area_beats_the_historical_false_positive() {
        let areas = mixed_dpi_layout();
        let geometry = geometry_on_secondary();

        // Reproduce the old bug: projecting with the primary display's own 100%
        // scale puts the window at [1333..1813]x[133..793], fully inside the primary
        // work area — a bogus positive that the old first-hit `find` would accept.
        let legacy_projection = projected_physical_rect(
            &SettingsWindowGeometry { scale_factor: None, ..geometry.clone() },
            1.0,
        );
        assert!(legacy_projection.intersection_area(&areas[0].rect()) > 0.0);

        // With the saved scale factor recorded the projection is unambiguous and the
        // largest-overlap pick lands on the secondary display.
        assert_eq!(select_work_area(&areas, &geometry), Some(1));
    }

    #[test]
    fn detached_display_falls_back_to_no_placement() {
        let areas = mixed_dpi_layout();
        // Saved on a third monitor that has since been unplugged: nothing on the
        // current layout overlaps it, so the caller must centre instead.
        let geometry = SettingsWindowGeometry {
            x: 5000.0,
            y: 300.0,
            width: 480.0,
            height: 660.0,
            scale_factor: Some(1.5),
        };
        assert_eq!(select_work_area(&areas, &geometry), None);
        assert_eq!(plan_settings_geometry(&areas, &geometry), None);
    }

    #[test]
    fn legacy_geometry_without_scale_factor_still_resolves_by_area() {
        let areas = mixed_dpi_layout();
        let geometry = SettingsWindowGeometry {
            x: 1333.33,
            y: 133.33,
            width: 480.0,
            height: 660.0,
            scale_factor: None,
        };
        assert_eq!(select_work_area(&areas, &geometry), Some(1));
    }

    #[test]
    fn saved_geometry_is_clamped_inside_the_selected_work_area() {
        let areas = mixed_dpi_layout();
        let geometry = SettingsWindowGeometry {
            x: 1800.0,
            y: 1000.0,
            width: 4000.0,
            height: 3000.0,
            scale_factor: Some(1.0),
        };
        let index = select_work_area(&areas, &geometry).expect("a monitor should match");
        let work = areas[index];
        let placement = plan_settings_geometry(&areas, &geometry).expect("placement");

        assert!(placement.width <= work.width, "width {} > {}", placement.width, work.width);
        assert!(placement.height <= work.height, "height {} > {}", placement.height, work.height);
        assert!(placement.x >= work.left);
        assert!(placement.y >= work.top);
        assert!(placement.x + placement.width <= work.rect().right() + 0.001);
        assert!(placement.y + placement.height <= work.rect().bottom() + 0.001);
    }

    #[test]
    fn degenerate_monitor_scale_is_ignored() {
        let areas = [
            MonitorWorkArea { left: 0.0, top: 0.0, width: 1920.0, height: 1080.0, scale_factor: 0.0 },
            MonitorWorkArea { left: 1920.0, top: 0.0, width: 2560.0, height: 1440.0, scale_factor: 1.5 },
        ];
        assert_eq!(select_work_area(&areas, &geometry_on_secondary()), Some(1));
    }

    #[test]
    fn intersection_area_is_zero_when_rectangles_only_touch() {
        let a = PhysicalRect::new(0.0, 0.0, 100.0, 100.0);
        let b = PhysicalRect::new(100.0, 0.0, 100.0, 100.0);
        assert_eq!(a.intersection_area(&b), 0.0);
        let c = PhysicalRect::new(50.0, 50.0, 100.0, 100.0);
        assert_eq!(a.intersection_area(&c), 2500.0);
    }
}
