use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewWindow,
};

use crate::commands::AppState;
use crate::config::{ConfigManager, DockPersistence, OverlaySettings, WindowPosition};

pub const SNAP_MARGIN_LOGICAL: f64 = 16.0;
pub const PILL_WIDTH_LOGICAL: f64 = 46.0;
pub const PILL_HEIGHT_LOGICAL: f64 = 100.0;
pub const EDGE_INSET_LOGICAL: f64 = 2.0;
const COLLAPSE_DELAY: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockState {
    Free,
    Docked(Edge),
    ManualHidden,
}

#[derive(Debug)]
struct Runtime {
    state: DockState,
    expanded: bool,
    manual_hidden: bool,
    dragging: bool,
    menu_open: bool,
    pointer_inside: bool,
    timer_generation: u64,
    anchor_center: Option<(i32, i32)>,
}

#[derive(Clone)]
pub struct DockManager {
    config: ConfigManager,
    persisted: Arc<Mutex<DockPersistence>>,
    runtime: Arc<Mutex<Runtime>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockStateInfo {
    pub docked: bool,
    pub edge: Option<Edge>,
    pub expanded: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DockManager {
    pub fn new(config: ConfigManager) -> Self {
        let persisted = config.load_dock_state().unwrap_or_default();
        let state = match (persisted.docked, persisted.edge) {
            (true, Some(edge)) => DockState::Docked(edge),
            _ => DockState::Free,
        };
        Self {
            config,
            persisted: Arc::new(Mutex::new(persisted)),
            runtime: Arc::new(Mutex::new(Runtime {
                state,
                expanded: !matches!(state, DockState::Docked(_)),
                manual_hidden: false,
                dragging: false,
                menu_open: false,
                pointer_inside: false,
                timer_generation: 0,
                anchor_center: None,
            })),
        }
    }

    pub fn info(&self) -> DockStateInfo {
        let runtime = lock(&self.runtime);
        let persisted = lock(&self.persisted);
        DockStateInfo {
            docked: persisted.docked,
            edge: persisted.edge,
            expanded: runtime.expanded,
            hidden: runtime.manual_hidden,
        }
    }

    fn save_docked(&self, edge: Option<Edge>) {
        let next = DockPersistence {
            docked: edge.is_some(),
            edge,
        };
        *lock(&self.persisted) = next.clone();
        self.config.save_dock_state(&next);
    }

    pub fn begin_drag(&self) {
        let mut runtime = lock(&self.runtime);
        runtime.dragging = true;
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
    }

    fn set_anchor_center(&self, center: (i32, i32)) {
        lock(&self.runtime).anchor_center = Some(center);
    }

    fn anchor_center(&self) -> Option<(i32, i32)> {
        lock(&self.runtime).anchor_center
    }

    fn docked_edge(&self) -> Option<Edge> {
        match lock(&self.runtime).state {
            DockState::Docked(edge) => Some(edge),
            _ => None,
        }
    }

    /// True while the user is dragging the window or its context menu is open.
    /// Programmatic repositioning must stand down during these interactions.
    pub fn is_interacting(&self) -> bool {
        let runtime = lock(&self.runtime);
        runtime.dragging || runtime.menu_open
    }

    pub fn cancel_drag(&self) {
        let mut runtime = lock(&self.runtime);
        runtime.dragging = false;
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
    }

    fn set_free(&self) {
        let mut runtime = lock(&self.runtime);
        runtime.state = if runtime.manual_hidden {
            DockState::ManualHidden
        } else {
            DockState::Free
        };
        runtime.expanded = true;
        runtime.dragging = false;
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
    }

    fn revert_expansion(&self) {
        lock(&self.runtime).expanded = false;
    }

    fn revert_collapse(&self) {
        lock(&self.runtime).expanded = true;
    }

    pub fn begin_menu(&self) {
        let mut runtime = lock(&self.runtime);
        runtime.menu_open = true;
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
    }

    pub fn finish_drag(&self, edge: Option<Edge>) {
        let mut runtime = lock(&self.runtime);
        runtime.dragging = false;
        runtime.pointer_inside = false;
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
        if let Some(edge) = edge {
            runtime.state = DockState::Docked(edge);
            runtime.expanded = false;
        } else {
            runtime.state = DockState::Free;
            runtime.expanded = true;
        }
    }

    pub fn enter_pill(&self) -> bool {
        let mut runtime = lock(&self.runtime);
        runtime.pointer_inside = true;
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
        if matches!(runtime.state, DockState::Docked(_)) && !runtime.expanded {
            runtime.expanded = true;
            true
        } else {
            false
        }
    }

    pub fn leave_window(&self) -> Option<u64> {
        let mut runtime = lock(&self.runtime);
        runtime.pointer_inside = false;
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
        if matches!(runtime.state, DockState::Docked(_))
            && runtime.expanded
            && !runtime.dragging
            && !runtime.menu_open
        {
            Some(runtime.timer_generation)
        } else {
            None
        }
    }

    pub fn menu_closed(&self) -> Option<u64> {
        let mut runtime = lock(&self.runtime);
        runtime.menu_open = false;
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
        if matches!(runtime.state, DockState::Docked(_))
            && runtime.expanded
            && !runtime.pointer_inside
            && !runtime.dragging
        {
            Some(runtime.timer_generation)
        } else {
            None
        }
    }

    fn collapse_if_current(&self, generation: u64) -> Option<Edge> {
        let mut runtime = lock(&self.runtime);
        if runtime.timer_generation != generation
            || !matches!(runtime.state, DockState::Docked(_))
            || runtime.dragging
            || runtime.menu_open
            || runtime.pointer_inside
            || !runtime.expanded
        {
            return None;
        }
        let DockState::Docked(edge) = runtime.state else {
            return None;
        };
        runtime.expanded = false;
        Some(edge)
    }

    fn set_manual_hidden(&self, hidden: bool) -> Option<Edge> {
        let mut runtime = lock(&self.runtime);
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
        runtime.manual_hidden = hidden;
        runtime.dragging = false;
        runtime.menu_open = false;
        if hidden {
            runtime.state = DockState::ManualHidden;
            return None;
        }
        let persisted = lock(&self.persisted).clone();
        if persisted.docked {
            if let Some(edge) = persisted.edge {
                runtime.state = DockState::Docked(edge);
                runtime.expanded = false;
                return Some(edge);
            }
        }
        runtime.state = DockState::Free;
        runtime.expanded = true;
        None
    }

    fn disable_auto_hide(&self) {
        self.save_docked(None);
        let mut runtime = lock(&self.runtime);
        runtime.timer_generation = runtime.timer_generation.wrapping_add(1);
        if !runtime.manual_hidden {
            runtime.state = DockState::Free;
            runtime.expanded = true;
        }
    }

    fn startup_state(&self, enabled: bool) -> Option<Edge> {
        let persisted = lock(&self.persisted).clone();
        let mut runtime = lock(&self.runtime);
        if enabled && persisted.docked {
            if let Some(edge) = persisted.edge {
                runtime.state = DockState::Docked(edge);
                runtime.expanded = false;
                return Some(edge);
            }
            drop(runtime);
            self.save_docked(None);
            runtime = lock(&self.runtime);
        }
        if !enabled && persisted.docked {
            drop(runtime);
            self.save_docked(None);
            runtime = lock(&self.runtime);
        }
        runtime.state = DockState::Free;
        runtime.expanded = true;
        None
    }
}

pub fn detect_edge(win: PhysicalRect, work: PhysicalRect, scale_factor: f64) -> Option<Edge> {
    let threshold = (SNAP_MARGIN_LOGICAL * scale_factor.max(0.1)).round() as i64;
    let wx = work.x as i64;
    let wy = work.y as i64;
    let wr = wx + work.width as i64;
    let wb = wy + work.height as i64;
    let x = win.x as i64;
    let y = win.y as i64;
    let right = x + win.width as i64;
    let bottom = y + win.height as i64;
    [
        (x - wx, Edge::Left),
        (wr - right, Edge::Right),
        (y - wy, Edge::Top),
        (wb - bottom, Edge::Bottom),
    ]
    .into_iter()
    .filter(|(distance, _)| distance.abs() <= threshold)
    .min_by_key(|(distance, _)| distance.abs())
    .map(|(_, edge)| edge)
}

pub fn pill_geometry(
    edge: Edge,
    work: PhysicalRect,
    monitor_scale: f64,
    scale_percent: u32,
    anchor_center: (i32, i32),
) -> PhysicalRect {
    let factor = monitor_scale.max(0.1) * (scale_percent.clamp(100, 250) as f64 / 100.0);
    let vertical_width = (PILL_WIDTH_LOGICAL * factor).round().max(1.0) as u32;
    let vertical_height = (PILL_HEIGHT_LOGICAL * factor).round().max(1.0) as u32;
    let (mut width, mut height) = match edge {
        Edge::Left | Edge::Right => (vertical_width, vertical_height),
        Edge::Top | Edge::Bottom => (vertical_height, vertical_width),
    };
    let work_width = work.width.max(1);
    let work_height = work.height.max(1);
    width = width.min(work_width);
    height = height.min(work_height);
    let inset = (EDGE_INSET_LOGICAL * monitor_scale.max(0.1)).round() as i32;
    let inset_x = inset.min(((work_width - width) / 2).min(i32::MAX as u32) as i32);
    let inset_y = inset.min(((work_height - height) / 2).min(i32::MAX as u32) as i32);
    let x = match edge {
        Edge::Left => work.x.saturating_add(inset_x),
        Edge::Right => work
            .x
            .saturating_add(work_width.min(i32::MAX as u32) as i32)
            .saturating_sub(width.min(i32::MAX as u32) as i32)
            .saturating_sub(inset_x),
        Edge::Top | Edge::Bottom => {
            let max_x = work.x as i64 + (work_width as i64 - width as i64).max(0);
            (anchor_center.0 as i64 - width as i64 / 2).clamp(work.x as i64, max_x) as i32
        }
    };
    let y = match edge {
        Edge::Top => work.y.saturating_add(inset_y),
        Edge::Bottom => work
            .y
            .saturating_add(work_height.min(i32::MAX as u32) as i32)
            .saturating_sub(height.min(i32::MAX as u32) as i32)
            .saturating_sub(inset_y),
        Edge::Left | Edge::Right => {
            let max_y = work.y as i64 + (work_height as i64 - height as i64).max(0);
            (anchor_center.1 as i64 - height as i64 / 2).clamp(work.y as i64, max_y) as i32
        }
    };
    PhysicalRect {
        x,
        y,
        width,
        height,
    }
}

pub fn restore_startup(window: &WebviewWindow, manager: &DockManager, settings: &OverlaySettings) {
    let size_ok = set_full_size(window, settings).is_ok();
    if size_ok {
        if let Err(error) = place_full_at_anchor(window, manager) {
            eprintln!("Could not restore overlay anchor: {error}");
        }
    }
    let edge = manager.startup_state(settings.auto_edge_hide);
    if let Some(edge) = edge {
        let center = manager
            .anchor_center()
            .unwrap_or_else(|| window_center(window));
        if let Err(error) = apply_pill(window, edge, settings, center) {
            eprintln!("Could not restore docked overlay: {error}");
            manager.set_free();
            manager.save_docked(None);
            if set_full_size(window, settings).is_ok() {
                let _ = place_full_at_anchor(window, manager);
            }
        }
    }
}

pub async fn drag_ended(app: &AppHandle, state: &Arc<AppState>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(position) = window.outer_position() else {
        state.dock.cancel_drag();
        return;
    };
    let Ok(size) = window.outer_size() else {
        state.dock.cancel_drag();
        return;
    };
    let anchor = WindowPosition {
        left: position.x as f64,
        top: position.y as f64,
    };
    state.config_manager.save_position(&anchor);
    let anchor_center = (
        position
            .x
            .saturating_add((size.width / 2).min(i32::MAX as u32) as i32),
        position
            .y
            .saturating_add((size.height / 2).min(i32::MAX as u32) as i32),
    );
    state.dock.set_anchor_center(anchor_center);
    let settings = state.settings.lock().await.clone();
    let edge = if settings.auto_edge_hide {
        work_area(&window).and_then(|(work, scale)| {
            detect_edge(
                PhysicalRect {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                },
                work,
                scale,
            )
        })
    } else {
        None
    };
    if let Some(edge) = edge {
        if let Err(error) = apply_pill(&window, edge, &settings, anchor_center) {
            eprintln!("Could not collapse overlay at screen edge: {error}");
            state.dock.finish_drag(None);
            state.dock.save_docked(None);
        } else {
            state.dock.finish_drag(Some(edge));
            state.dock.save_docked(Some(edge));
        }
    } else {
        state.dock.finish_drag(None);
        state.dock.save_docked(None);
    }
    emit_state(app, &state.dock);
}

pub fn mouse_enter(app: &AppHandle, state: &Arc<AppState>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if state.dock.enter_pill() {
        let state_clone = state.clone();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            let settings = state_clone.settings.lock().await.clone();
            // Capture the pill centre before the provisional resize: it is the
            // anchor the transition rectangle stays aligned with along the
            // non-docked axis (spec §3.4). Resizing first would make
            // `window_center` report the enlarged window's centre instead.
            let anchor_center = state_clone
                .dock
                .anchor_center()
                .unwrap_or_else(|| window_center(&window));
            if let Err(error) = set_full_size(&window, &settings)
                .and_then(|_| place_full_for_transition(&window, &state_clone.dock, anchor_center))
            {
                eprintln!("Could not expand docked overlay: {error}");
                state_clone.dock.revert_expansion();
                return;
            }
            emit_state(&app_clone, &state_clone.dock);
        });
    }
}

pub fn mouse_leave(app: &AppHandle, state: &Arc<AppState>) {
    if let Some(generation) = state.dock.leave_window() {
        schedule_collapse(app, state, generation);
    }
}

pub fn menu_changed(app: &AppHandle, state: &Arc<AppState>, open: bool) {
    if open {
        state.dock.begin_menu();
    } else if let Some(generation) = state.dock.menu_closed() {
        schedule_collapse(app, state, generation);
    }
}

fn schedule_collapse(app: &AppHandle, state: &Arc<AppState>, generation: u64) {
    let app = app.clone();
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(COLLAPSE_DELAY).await;
        let Some(edge) = state.dock.collapse_if_current(generation) else {
            return;
        };
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let settings = state.settings.lock().await.clone();
        let center = state
            .dock
            .anchor_center()
            .unwrap_or_else(|| window_center(&window));
        if let Err(error) = apply_pill(&window, edge, &settings, center) {
            eprintln!("Could not collapse docked overlay: {error}");
            state.dock.revert_collapse();
            return;
        }
        emit_state(&app, &state.dock);
    });
}

pub async fn auto_hide_changed(
    app: &AppHandle,
    state: &Arc<AppState>,
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        return Ok(());
    }
    let previous = state.dock.info();
    if let Some(window) = app.get_webview_window("main") {
        let settings = state.settings.lock().await.clone();
        if !state.dock.info().hidden {
            set_full_size(&window, &settings)?;
            if let Err(error) = place_full_at_anchor(&window, &state.dock) {
                if let Some(edge) = previous.edge.filter(|_| previous.docked) {
                    let center = state
                        .dock
                        .anchor_center()
                        .unwrap_or_else(|| window_center(&window));
                    let _ = apply_pill(&window, edge, &settings, center);
                }
                return Err(error);
            }
        }
    }
    state.dock.disable_auto_hide();
    emit_state(app, &state.dock);
    Ok(())
}

pub async fn toggle_overlay_window(app: &AppHandle, state: &Arc<AppState>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        state.dock.set_manual_hidden(true);
        let _ = window.hide();
    } else {
        let settings = state.settings.lock().await.clone();
        let edge = state.dock.set_manual_hidden(false);
        if let Some(edge) = edge.filter(|_| settings.auto_edge_hide) {
            let center = state
                .dock
                .anchor_center()
                .unwrap_or_else(|| window_center(&window));
            if let Err(error) = apply_pill(&window, edge, &settings, center) {
                eprintln!("Could not restore docked overlay: {error}");
                state.dock.set_free();
                state.dock.save_docked(None);
                if let Err(fallback_error) = set_full_size(&window, &settings)
                    .and_then(|_| place_full_at_anchor(&window, &state.dock))
                {
                    eprintln!("Could not restore overlay window: {fallback_error}");
                }
            }
        } else {
            if let Err(error) = set_full_size(&window, &settings)
                .and_then(|_| place_full_at_anchor(&window, &state.dock))
            {
                eprintln!("Could not restore overlay window: {error}");
            }
        }
        let _ = window.show();
        let _ = window.set_focus();
    }
    emit_state(app, &state.dock);
}

pub async fn keep_docked_in_work_area(app: &AppHandle, state: &Arc<AppState>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let info = state.dock.info();
    // Never fight the user: a background usage refresh must not reposition the
    // window while it is being dragged or its menu is open (spec §3.1 T6/§3.7).
    if info.hidden || state.dock.is_interacting() {
        return;
    }
    let settings = state.settings.lock().await.clone();
    if let Some(edge) = info.edge.filter(|_| info.docked) {
        if info.expanded {
            if let Err(error) = place_full_at_anchor(&window, &state.dock) {
                eprintln!("Could not reposition expanded overlay: {error}");
            }
        } else {
            let (work, scale) = match work_area(&window) {
                Some(geometry) => geometry,
                None => return,
            };
            let center = estimated_anchor_center(&window, &state.dock, &settings, work, scale);
            if let Some(center) = center {
                state.dock.set_anchor_center(center);
            }
            if let Err(error) = apply_pill(
                &window,
                edge,
                &settings,
                center.unwrap_or_else(|| window_center(&window)),
            ) {
                eprintln!("Could not reposition docked overlay: {error}");
            }
        }
    }
}

pub fn window_resized(app: &AppHandle, state: &Arc<AppState>) {
    let app = app.clone();
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        if !state.dock.info().docked || !state.dock.info().expanded {
            return;
        }
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let _ = place_full_at_anchor(&window, &state.dock);
    });
}

fn emit_state(app: &AppHandle, manager: &DockManager) {
    let _ = app.emit("dock_state_changed", manager.info());
}

fn apply_pill(
    window: &WebviewWindow,
    edge: Edge,
    settings: &OverlaySettings,
    anchor_center: (i32, i32),
) -> Result<(), String> {
    let (work, scale) =
        work_area_for_point(window, Some((anchor_center.0 as f64, anchor_center.1 as f64)))
            .ok_or_else(|| "Monitor work area unavailable".to_string())?;
    let rect = pill_geometry(edge, work, scale, settings.scale_percent, anchor_center);
    window
        .set_size(Size::Physical(PhysicalSize::new(rect.width, rect.height)))
        .map_err(|error| error.to_string())?;
    window
        .set_position(Position::Physical(PhysicalPosition::new(rect.x, rect.y)))
        .map_err(|error| error.to_string())
}

/// Provisional full-window size written before the frontend measures the real
/// capsule. The estimate only has to keep the window under the cursor until
/// `fitCapsuleSize` reports the authoritative content size, so it must never
/// shrink below the current window: expanding from an edge pill (which can be
/// taller than the capsule) would otherwise move the cursor outside the window
/// and trigger an immediate `mouseleave`.
fn set_full_size(window: &WebviewWindow, settings: &OverlaySettings) -> Result<(), String> {
    let monitor_scale = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1.0);
    let scale = settings.scale_percent as f64 / 100.0 * monitor_scale;
    let current = window
        .outer_size()
        .unwrap_or_else(|_| PhysicalSize::new(0, 0));
    let size = provisional_size(current, scale, settings.show_credits);
    window
        .set_size(Size::Physical(size))
        .map_err(|error| error.to_string())?;
    emit_size_invalidated(window);
    Ok(())
}

/// Provisional full-window size written before the frontend measures the real
/// capsule. The estimate never shrinks below the current window, so a docked
/// pill (which can be taller than the capsule) stays covered while the
/// authoritative size is on its way.
fn provisional_size(
    current: PhysicalSize<u32>,
    scale: f64,
    show_credits: bool,
) -> PhysicalSize<u32> {
    let estimated_width = ((if show_credits { 220.0 } else { 160.0 }) * scale)
        .round()
        .max(1.0) as u32;
    let estimated_height = (50.0 * scale).round().max(1.0) as u32;
    PhysicalSize::new(
        estimated_width.max(current.width),
        estimated_height.max(current.height),
    )
}

/// Tells the frontend that the window was resized programmatically and that its
/// cached content measurement is stale, so it re-measures and writes the real
/// capsule size (spec §3.4).
fn emit_size_invalidated(window: &WebviewWindow) {
    let _ = window.app_handle().emit("overlay_size_invalidated", ());
}

fn place_full_at_anchor(window: &WebviewWindow, manager: &DockManager) -> Result<(), String> {
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let saved = manager.config.load_position().map(|position| {
        PhysicalPosition::new(position.left.round() as i32, position.top.round() as i32)
    });
    // Resolve the work area from the monitor that owns the saved anchor, not the
    // one the window currently sits on: at startup the window is still on the
    // primary monitor, so clamping against it would discard a secondary anchor.
    let (work, scale) = match saved {
        Some(anchor) => work_area_for_point(window, Some((anchor.x as f64, anchor.y as f64))),
        None => work_area(window),
    }
    .ok_or_else(|| "Monitor work area unavailable".to_string())?;
    let anchor = saved.unwrap_or_else(|| {
        let margin = (20.0 * scale).round() as i32;
        let top = (40.0 * scale).round() as i32;
        PhysicalPosition::new(
            work.x
                .saturating_add(work.width.min(i32::MAX as u32) as i32)
                .saturating_sub(size.width.min(i32::MAX as u32) as i32)
                .saturating_sub(margin),
            work.y.saturating_add(top),
        )
    });
    let clamped = clamp_position(anchor, size, work);
    manager.set_anchor_center((
        clamped
            .x
            .saturating_add((size.width / 2).min(i32::MAX as u32) as i32),
        clamped
            .y
            .saturating_add((size.height / 2).min(i32::MAX as u32) as i32),
    ));
    window
        .set_position(Position::Physical(clamped))
        .map_err(|error| error.to_string())
}

/// Places the expanded window for the hover-expand transition while the window
/// is still docked. The pill is pinned to a work-area edge, so the provisional
/// rectangle is kept flush against that same edge and centred on the pill along
/// the other axis. That way the rectangle written by `mouse_enter` covers the
/// cursor's current hit area, so the overlay cannot collapse the instant it
/// expands (spec §3.4). Once the frontend reports the authoritative capsule
/// size, `window_resized` re-clamps the saved anchor through
/// `place_full_at_anchor`.
fn place_full_for_transition(
    window: &WebviewWindow,
    manager: &DockManager,
    anchor_center: (i32, i32),
) -> Result<(), String> {
    let Some(edge) = manager.docked_edge() else {
        // Not docked: fall back to the regular anchor placement.
        return place_full_at_anchor(window, manager);
    };
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let (work, _scale) = work_area_for_point(
        window,
        Some((anchor_center.0 as f64, anchor_center.1 as f64)),
    )
    .ok_or_else(|| "Monitor work area unavailable".to_string())?;
    let rect = transition_rect(edge, work, size, anchor_center);
    window
        .set_position(Position::Physical(PhysicalPosition::new(rect.x, rect.y)))
        .map_err(|error| error.to_string())
}

/// Geometry of the provisional expand rectangle. Along the docked axis the
/// rectangle hugs the work-area edge the pill is pinned to; along the other
/// axis it is centred on `anchor_center` (the pill's centre) and clamped into
/// the work area. Because `size` is at least the pill's size on both axes, the
/// result always contains the pill rectangle.
fn transition_rect(
    edge: Edge,
    work: PhysicalRect,
    size: PhysicalSize<u32>,
    anchor_center: (i32, i32),
) -> PhysicalRect {
    let work_left = work.x as i64;
    let work_top = work.y as i64;
    let work_right = work_left + work.width as i64;
    let work_bottom = work_top + work.height as i64;
    let width = size.width as i64;
    let height = size.height as i64;
    let max_x = (work_right - width).max(work_left);
    let max_y = (work_bottom - height).max(work_top);
    let x = match edge {
        Edge::Left => work_left,
        Edge::Right => max_x,
        Edge::Top | Edge::Bottom => (anchor_center.0 as i64 - width / 2).clamp(work_left, max_x),
    };
    let y = match edge {
        Edge::Top => work_top,
        Edge::Bottom => max_y,
        Edge::Left | Edge::Right => (anchor_center.1 as i64 - height / 2).clamp(work_top, max_y),
    };
    PhysicalRect {
        x: x as i32,
        y: y as i32,
        width: size.width,
        height: size.height,
    }
}

fn window_center(window: &WebviewWindow) -> (i32, i32) {
    match (window.outer_position(), window.outer_size()) {
        (Ok(position), Ok(size)) => (
            position
                .x
                .saturating_add((size.width / 2).min(i32::MAX as u32) as i32),
            position
                .y
                .saturating_add((size.height / 2).min(i32::MAX as u32) as i32),
        ),
        _ => (0, 0),
    }
}

fn estimated_anchor_center(
    window: &WebviewWindow,
    manager: &DockManager,
    settings: &OverlaySettings,
    work: PhysicalRect,
    monitor_scale: f64,
) -> Option<(i32, i32)> {
    let anchor = manager
        .config
        .load_position()
        .map(|position| {
            PhysicalPosition::new(position.left.round() as i32, position.top.round() as i32)
        })
        .or_else(|| window.outer_position().ok())?;
    let scale = settings.scale_percent as f64 / 100.0 * monitor_scale;
    let width = ((if settings.show_credits { 220.0 } else { 160.0 }) * scale)
        .round()
        .max(1.0) as u32;
    let height = (50.0 * scale).round().max(1.0) as u32;
    let clamped = clamp_position(anchor, PhysicalSize::new(width, height), work);
    Some((
        clamped
            .x
            .saturating_add((width / 2).min(i32::MAX as u32) as i32),
        clamped
            .y
            .saturating_add((height / 2).min(i32::MAX as u32) as i32),
    ))
}

fn clamp_position(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    work: PhysicalRect,
) -> PhysicalPosition<i32> {
    let left = work.x as i64;
    let top = work.y as i64;
    let right = left + work.width as i64;
    let bottom = top + work.height as i64;
    let max_x = right - size.width as i64;
    let max_y = bottom - size.height as i64;
    PhysicalPosition::new(
        (position.x as i64).clamp(left, max_x.max(left)) as i32,
        (position.y as i64).clamp(top, max_y.max(top)) as i32,
    )
}

fn monitor_bounds(monitor: &tauri::Monitor) -> (PhysicalRect, f64) {
    let area = monitor.work_area();
    (
        PhysicalRect {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        monitor.scale_factor(),
    )
}

fn monitor_contains(rect: PhysicalRect, x: f64, y: f64) -> bool {
    let left = rect.x as f64;
    let top = rect.y as f64;
    x >= left && x < left + rect.width as f64 && y >= top && y < top + rect.height as f64
}

/// Picks the work area that owns `point`. When the point lies outside every
/// monitor (e.g. a saved anchor on a display that is no longer attached) it
/// falls back to the monitor under the window and then the first available one,
/// so the window is always clamped into a usable area.
fn select_work_area(
    monitors: &[(PhysicalRect, f64)],
    fallback: usize,
    point: Option<(f64, f64)>,
) -> Option<(PhysicalRect, f64)> {
    if let Some((x, y)) = point {
        if let Some(found) = monitors
            .iter()
            .find(|(rect, _)| monitor_contains(*rect, x, y))
        {
            return Some(*found);
        }
    }
    monitors
        .get(fallback)
        .or_else(|| monitors.first())
        .copied()
}

fn work_area_for_point(
    window: &WebviewWindow,
    point: Option<(f64, f64)>,
) -> Option<(PhysicalRect, f64)> {
    let monitors: Vec<(PhysicalRect, f64)> = window
        .available_monitors()
        .ok()?
        .iter()
        .map(monitor_bounds)
        .collect();
    if monitors.is_empty() {
        return None;
    }
    let fallback = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|current| monitor_bounds(&current))
        .and_then(|current| monitors.iter().position(|candidate| *candidate == current))
        .unwrap_or(0);
    select_work_area(&monitors, fallback, point)
}

fn work_area(window: &WebviewWindow) -> Option<(PhysicalRect, f64)> {
    let point = window
        .outer_position()
        .ok()
        .zip(window.outer_size().ok())
        .map(|(position, size)| {
            (
                position.x as f64 + size.width as f64 / 2.0,
                position.y as f64 + size.height as f64 / 2.0,
            )
        });
    work_area_for_point(window, point)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(windows)]
pub fn install_native_window_hook(window: &WebviewWindow, app: &AppHandle) -> Result<(), String> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};

    static HANDLES: OnceLock<Mutex<HashMap<isize, AppHandle>>> = OnceLock::new();
    const SUBCLASS_ID: usize = 0x434f_4445;

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass_id: usize,
        _ref_data: usize,
    ) -> LRESULT {
        let key = hwnd.0 as isize;
        let app = HANDLES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            .cloned();
        if let Some(app) = app {
            match message {
                0x0231 => {
                    let _ = app.emit("window_drag_started", ());
                }
                0x0232 => {
                    let _ = app.emit("window_drag_ended", ());
                }
                0x0211 => {
                    let _ = app.emit("overlay_menu_opened", ());
                }
                0x0212 => {
                    let _ = app.emit("overlay_menu_closed", ());
                }
                _ => {}
            }
        }
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        if message == 0x0082 {
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), subclass_id) };
            HANDLES
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
        }
        result
    }

    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let registry = HANDLES.get_or_init(|| Mutex::new(HashMap::new()));
    lock(registry).insert(hwnd.0 as isize, app.clone());
    let ok: BOOL = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) };
    if !ok.as_bool() {
        lock(registry).remove(&(hwnd.0 as isize));
        return Err("Could not install the native window event hook".into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn install_native_window_hook(_window: &WebviewWindow, _app: &AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> PhysicalRect {
        PhysicalRect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        }
    }

    #[test]
    fn snap_threshold_is_scaled_and_accepts_negative_coordinates() {
        let wa = work();
        let near_left = PhysicalRect {
            x: -1947,
            y: 200,
            width: 300,
            height: 100,
        };
        let far_left = PhysicalRect {
            x: -1950,
            y: 200,
            width: 300,
            height: 100,
        };
        assert_eq!(detect_edge(near_left, wa, 1.75), Some(Edge::Left));
        assert_eq!(detect_edge(far_left, wa, 1.75), None);
    }

    #[test]
    fn snap_chooses_the_closest_edge_when_multiple_edges_match() {
        let rect = PhysicalRect {
            x: -1924,
            y: 4,
            width: 300,
            height: 100,
        };
        assert_eq!(detect_edge(rect, work(), 1.0), Some(Edge::Left));
    }

    #[test]
    fn pill_geometry_swaps_dimensions_for_top_and_bottom_and_scales() {
        let wa = PhysicalRect {
            x: 100,
            y: -50,
            width: 1920,
            height: 1080,
        };
        let left = pill_geometry(Edge::Left, wa, 1.75, 200, (1060, 490));
        let top = pill_geometry(Edge::Top, wa, 1.75, 200, (1060, 490));
        let bottom = pill_geometry(Edge::Bottom, wa, 1.75, 200, (1060, 490));
        assert_eq!((left.width, left.height), (161, 350));
        assert_eq!((top.width, top.height), (350, 161));
        assert_eq!((top.x, top.y), (885, -46));
        assert_eq!((bottom.x, bottom.y), (885, 865));
        assert_eq!((left.x, left.y), (104, 315));
    }

    #[test]
    fn pill_geometry_shrinks_to_fit_small_work_areas() {
        let work = PhysicalRect {
            x: -10,
            y: 20,
            width: 12,
            height: 20,
        };
        let left = pill_geometry(Edge::Left, work, 1.0, 250, (0, 30));
        let top = pill_geometry(Edge::Top, work, 1.0, 250, (0, 30));
        assert_eq!(left, work);
        assert_eq!(top, work);
    }

    fn contains(outer: PhysicalRect, inner: PhysicalRect) -> bool {
        let outer_left = outer.x as i64;
        let outer_top = outer.y as i64;
        let outer_right = outer_left + outer.width as i64;
        let outer_bottom = outer_top + outer.height as i64;
        let inner_left = inner.x as i64;
        let inner_top = inner.y as i64;
        let inner_right = inner_left + inner.width as i64;
        let inner_bottom = inner_top + inner.height as i64;
        inner_left >= outer_left
            && inner_top >= outer_top
            && inner_right <= outer_right
            && inner_bottom <= outer_bottom
    }

    #[test]
    fn transition_rect_covers_pill_rect_on_all_edges() {
        let wa = PhysicalRect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let monitor_scale = 1.75;
        let scale_percent = 175;
        let content_scale = monitor_scale * scale_percent as f64 / 100.0;
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            // Offsets span the middle of the edge and both sides of it, i.e.
            // every snap position a user can legitimately release at.
            for offset in [-400, -120, -28, 0, 28, 120, 400] {
                let anchor = match edge {
                    Edge::Left | Edge::Right => (1280, 720 + offset),
                    Edge::Top | Edge::Bottom => (1280 + offset, 720),
                };
                let pill = pill_geometry(edge, wa, monitor_scale, scale_percent, anchor);
                // The provisional size written by `set_full_size` never shrinks
                // below the current window (the pill), mirroring the runtime.
                let size = provisional_size(
                    PhysicalSize::new(pill.width, pill.height),
                    content_scale,
                    true,
                );
                let rect = transition_rect(edge, wa, size, anchor);
                assert!(
                    contains(rect, pill),
                    "{edge:?} at offset {offset}: transition {rect:?} must cover pill {pill:?}"
                );
            }
        }
    }

    fn manager_with(state: DockState, expanded: bool) -> DockManager {
        let config = ConfigManager::new();
        let persisted = match state {
            DockState::Docked(edge) => DockPersistence {
                docked: true,
                edge: Some(edge),
            },
            _ => DockPersistence {
                docked: false,
                edge: None,
            },
        };
        DockManager {
            config,
            persisted: Arc::new(Mutex::new(persisted)),
            runtime: Arc::new(Mutex::new(Runtime {
                state,
                expanded,
                manual_hidden: false,
                dragging: false,
                menu_open: false,
                pointer_inside: false,
                timer_generation: 0,
                anchor_center: None,
            })),
        }
    }

    #[test]
    fn dock_timer_cannot_collapse_during_drag_or_after_pointer_reenters() {
        let manager = manager_with(DockState::Docked(Edge::Right), true);
        lock(&manager.runtime).timer_generation = 4;
        assert!(manager.collapse_if_current(3).is_none());
        manager.begin_drag();
        assert!(manager.collapse_if_current(5).is_none());
        manager.finish_drag(Some(Edge::Right));
        assert!(!manager.info().expanded);
        assert!(manager.enter_pill());
        let timer = manager.leave_window().unwrap();
        assert!(!manager.collapse_if_current(timer.wrapping_sub(1)).is_some());
        assert!(manager.collapse_if_current(timer).is_some());
    }

    #[test]
    fn programmatic_reposition_is_blocked_while_dragging_or_menu_is_open() {
        let manager = manager_with(DockState::Docked(Edge::Left), false);
        assert!(!manager.is_interacting());
        manager.begin_drag();
        assert!(manager.is_interacting());
        manager.cancel_drag();
        assert!(!manager.is_interacting());
        manager.begin_menu();
        assert!(manager.is_interacting());
        manager.menu_closed();
        assert!(!manager.is_interacting());
    }

    #[test]
    fn a_secondary_monitor_anchor_is_not_clamped_into_the_primary_monitor() {
        let primary = (
            PhysicalRect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
            1.5,
        );
        let secondary = (
            PhysicalRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1040,
            },
            1.0,
        );
        let monitors = [primary, secondary];

        // The window sits on the primary monitor (fallback index 0) while the
        // saved anchor belongs to the secondary one.
        let selected = select_work_area(&monitors, 0, Some((-1500.0, 300.0))).unwrap();
        assert_eq!(selected.0, secondary.0);

        let secondary_work = selected.0;
        let clamped = clamp_position(
            PhysicalPosition::new(-1500, 300),
            PhysicalSize::new(385, 88),
            secondary_work,
        );
        assert_eq!((clamped.x, clamped.y), (-1500, 300));

        // An anchor outside every monitor (display unplugged) falls back to the
        // monitor under the window so the window stays on screen.
        let selected = select_work_area(&monitors, 0, Some((-9999.0, 300.0))).unwrap();
        assert_eq!(selected.0, primary.0);
    }
}
