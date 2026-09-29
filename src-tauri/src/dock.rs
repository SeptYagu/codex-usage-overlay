use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

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
/// Physical-pixel margin kept between the cursor and the nearest window edge
/// after an authoritative reposition, so rounding cannot push the pointer out
/// and trigger `mouseleave` (spec §3.4).
const CURSOR_COVER_MARGIN: i64 = 1;
/// Upper bound on how long background repositioning stands down for the
/// authoritative size write-back of a hover expand. It only has to outlive one
/// IPC round trip, and the lease ensures a lost `dock_window_resized` can
/// never latch the guard on (spec §3.4).
const HOVER_RESIZE_LEASE: Duration = Duration::from_millis(1500);

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
    hover: Option<Hover>,
}

/// Session state captured when the pointer entered the collapsed pill, valid
/// for the whole expanded session it opens.
///
/// `anchor_center` is the centre the expanded session is anchored to.
///
/// `entered_cursor` is the cursor point recorded at entry (`mouse_enter`) and is
/// the session's *only* cursor input: every reposition of the expanded window
/// reuses it, so the placement is a pure function of the session and cannot
/// move the window — or drop the pointer — as the pointer travels across the
/// capsule (spec §3.4). It is `None` when the system reported no pointer
/// position at entry, in which case placement falls back to `anchor_center`.
///
/// `resize_deadline` is the acquire of the transition guard: background
/// repositioning stands down until the frontend reports the authoritative
/// capsule size, or until the lease expires (spec §3.4).
#[derive(Debug, Clone, Copy)]
struct Hover {
    anchor_center: (i32, i32),
    entered_cursor: Option<(i32, i32)>,
    resize_deadline: Option<Instant>,
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

impl PhysicalRect {
    /// Right edge, in physical pixels. Widened to `i64` so the arithmetic below can
    /// never overflow on an extreme virtual-desktop coordinate.
    pub fn right(&self) -> i64 {
        self.x as i64 + self.width as i64
    }

    pub fn bottom(&self) -> i64 {
        self.y as i64 + self.height as i64
    }

    /// Overlapping area in square physical pixels; zero when the rectangles only
    /// touch or miss each other entirely.
    pub fn intersection_area(&self, other: &PhysicalRect) -> i64 {
        let width = (self.right().min(other.right()) - (self.x as i64).max(other.x as i64)).max(0);
        let height = (self.bottom().min(other.bottom()) - (self.y as i64).max(other.y as i64)).max(0);
        width * height
    }
}

/// Centre of a rectangle, saturating rather than wrapping on extreme coordinates.
fn center_of(rect: PhysicalRect) -> (i32, i32) {
    (
        rect.x.saturating_add((rect.width / 2).min(i32::MAX as u32) as i32),
        rect.y.saturating_add((rect.height / 2).min(i32::MAX as u32) as i32),
    )
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
                hover: None,
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

    /// Records the centre the expanded session is anchored to and the cursor
    /// point it entered at, and arms the write-back guard (spec §3.4). Both
    /// stay frozen for the whole session, so every later reposition of the
    /// expanded window is session-stable and idempotent.
    fn begin_hover(&self, anchor_center: (i32, i32), entered_cursor: Option<(i32, i32)>) {
        lock(&self.runtime).hover = Some(Hover {
            anchor_center,
            entered_cursor,
            resize_deadline: Some(Instant::now() + HOVER_RESIZE_LEASE),
        });
    }

    fn hover(&self) -> Option<Hover> {
        lock(&self.runtime).hover
    }

    /// True while the authoritative capsule size for a hover expand is still on
    /// its way. Background repositioning must stand down for that window: it
    /// would otherwise pull the window back to the static anchor and undo the
    /// coverage the transition just wrote (spec §3.4).
    fn is_awaiting_resize(&self) -> bool {
        lock(&self.runtime).hover.is_some_and(|hover| {
            hover
                .resize_deadline
                .is_some_and(|deadline| Instant::now() < deadline)
        })
    }

    /// Releases the write-back guard. The pill itself is kept: the rest of the
    /// expanded session is still centred on it.
    fn finish_hover_resize(&self) {
        let mut runtime = lock(&self.runtime);
        if let Some(hover) = runtime.hover {
            runtime.hover = Some(Hover {
                resize_deadline: None,
                ..hover
            });
        }
    }

    /// Centre of the expanded session's free axis: the centre recorded on hover
    /// entry, falling back to the runtime anchor.
    fn expanded_anchor_center(&self) -> Option<(i32, i32)> {
        let runtime = lock(&self.runtime);
        match runtime.hover {
            Some(hover) => Some(hover.anchor_center),
            None => runtime.anchor_center,
        }
    }

    fn drop_hover(&self) {
        lock(&self.runtime).hover = None;
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
        let mut runtime = lock(&self.runtime);
        runtime.expanded = false;
        // A failed expansion never reports an authoritative size to wait for.
        runtime.hover = None;
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
        // The drag ends the pill's hover session and arms the next one.
        runtime.hover = None;
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
        runtime.hover = None;
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

/// True when the two one-dimensional spans share more than an endpoint.
fn spans(a_start: i32, a_len: u32, b_start: i32, b_len: u32) -> bool {
    (a_start as i64) < (b_start as i64) + (b_len as i64)
        && (b_start as i64) < (a_start as i64) + (a_len as i64)
}

/// True when `edge` of `work` faces empty desktop instead of another monitor.
///
/// Whole-edge topology: as soon as *any* part of that edge touches (within a small
/// alignment tolerance) another monitor's work area that also overlaps on the other
/// axis, the whole edge is an inter-monitor seam. That is a deliberate
/// simplification (spec §3.2 T-2): keeping the cursor free to cross between screens
/// matters more than snapping inside the gap of a staggered layout, because folding
/// there would strand a capsule in the seam.
pub fn is_external_boundary(
    work: PhysicalRect,
    edge: Edge,
    all_monitors: &[(PhysicalRect, f64)],
) -> bool {
    const TOLERANCE: i64 = 4;
    for (other, _) in all_monitors {
        if *other == work {
            continue;
        }
        let touches = match edge {
            Edge::Left => {
                (work.x as i64 - other.right()).abs() <= TOLERANCE
                    && spans(work.y, work.height, other.y, other.height)
            }
            Edge::Right => {
                (work.right() - other.x as i64).abs() <= TOLERANCE
                    && spans(work.y, work.height, other.y, other.height)
            }
            Edge::Top => {
                (work.y as i64 - other.bottom()).abs() <= TOLERANCE
                    && spans(work.x, work.width, other.x, other.width)
            }
            Edge::Bottom => {
                (work.bottom() - other.y as i64).abs() <= TOLERANCE
                    && spans(work.x, work.width, other.x, other.width)
            }
        };
        if touches {
            return false;
        }
    }
    true
}

/// Edge detection restricted to physical outer boundaries, and tolerant of a
/// window that was dragged *past* one.
///
/// Two differences from a plain proximity test:
/// - a seam never matches, so a window released in the channel between two screens
///   is never folded (spec §3.2);
/// - a distance `<= 0` still matches, so a window dragged 50px off the left bezel
///   snaps back to `Edge::Left` instead of being left stranded off-screen, which is
///   what the old `distance.abs() <= threshold` test did.
pub fn detect_edge_multi_monitor(
    win: PhysicalRect,
    work: PhysicalRect,
    scale_factor: f64,
    all_monitors: &[(PhysicalRect, f64)],
) -> Option<Edge> {
    let threshold = (SNAP_MARGIN_LOGICAL * scale_factor.max(0.1)).round() as i64;
    let x = win.x as i64;
    let y = win.y as i64;
    [
        (x - work.x as i64, Edge::Left),
        (work.right() - (x + win.width as i64), Edge::Right),
        (y - work.y as i64, Edge::Top),
        (work.bottom() - (y + win.height as i64), Edge::Bottom),
    ]
    .into_iter()
    // Only a physical outer boundary may fold; an inter-monitor seam never does.
    .filter(|(_, edge)| is_external_boundary(work, *edge, all_monitors))
    // Inside the snap threshold, or already dragged out of bounds (`distance <= 0`).
    .filter(|(distance, _)| *distance <= threshold)
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

/// Picks the host monitor for a released window by intersecting it with every work
/// area. This is the single authoritative attribution rule for a cross-screen drag.
///
/// Totality — every input falls into exactly one of these branches:
/// 1. the screen with the strictly largest intersection area wins, which is what
///    makes "dragged more than half onto screen B, released, lands on B" hold;
/// 2. a tie that involves the host (`fallback_index`, the screen the window came
///    from) goes to the host, so an exact 50/50 release across a seam does not
///    change screens;
/// 3. a tie between two *non-host* screens is broken by the smallest `(x, y)`, so
///    the outcome cannot depend on `available_monitors()`' enumeration order;
/// 4. with no overlap anywhere (a staggered-monitor void, or fully off the desktop)
///    the host screen is returned — never an invented work area — and an
///    out-of-range `fallback_index` degrades to the first screen, mirroring
///    `select_work_area`'s chain;
/// 5. an empty monitor list returns `None` instead of panicking.
pub fn select_monitor_by_overlap(
    win: PhysicalRect,
    monitors: &[(PhysicalRect, f64)],
    fallback_index: usize,
) -> Option<(PhysicalRect, f64)> {
    if monitors.is_empty() {
        return None;
    }
    let host = if fallback_index < monitors.len() { fallback_index } else { 0 };
    let mut best_area = 0i64;
    for (work, _) in monitors {
        best_area = best_area.max(win.intersection_area(work));
    }
    if best_area == 0 {
        return monitors.get(host).copied();
    }
    let tied: Vec<usize> = monitors
        .iter()
        .enumerate()
        .filter(|(_, (work, _))| win.intersection_area(work) == best_area)
        .map(|(index, _)| index)
        .collect();
    let index = if tied.contains(&fallback_index) {
        fallback_index
    } else {
        // `tied` is non-empty (it contains at least the maximum), so the `min_by_key`
        // always has an element; the fallback index keeps the function total.
        *tied
            .iter()
            .min_by_key(|&&index| (monitors[index].0.x, monitors[index].0.y))
            .unwrap_or(&tied[0])
    };
    Some(monitors[index])
}

/// Clamps `rect` into `work` while keeping its size, returning the corrected
/// on-screen rectangle.
fn clamped_rect(rect: PhysicalRect, work: PhysicalRect) -> PhysicalRect {
    let clamped = clamp_position(
        PhysicalPosition::new(rect.x, rect.y),
        PhysicalSize::new(rect.width, rect.height),
        work,
    );
    PhysicalRect {
        x: clamped.x,
        y: clamped.y,
        width: rect.width,
        height: rect.height,
    }
}

/// The authoritative result of a finished drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragOutcome {
    /// The physical rectangle actually applied to the window: the pill's when the
    /// release folded it onto an outer edge, otherwise the settled full rectangle.
    pub rect: PhysicalRect,
    /// Coordinate persisted for the next session. It is always the corrected,
    /// on-screen top-left of the *expanded* overlay — never the raw release
    /// position, so a window released off-screen cannot restart off-screen.
    pub anchor: (i32, i32),
    /// Centre recorded on the dock manager so an expanded session re-anchors exactly
    /// where the release settled.
    pub anchor_center: (i32, i32),
    /// The edge the overlay folded onto, if any.
    pub edge: Option<Edge>,
}

/// Resolves what a released drag settles on, given the host work area that
/// `select_monitor_by_overlap` arbitrated.
///
/// Pure — `drag_ended` only applies the returned rectangle — which is what lets the
/// state matrix (fold on an outer boundary, clamp on a seam or off-desktop release)
/// be asserted offline.
pub fn resolve_drag_outcome(
    win: PhysicalRect,
    work: PhysicalRect,
    scale: f64,
    all_monitors: &[(PhysicalRect, f64)],
    auto_edge_hide: bool,
    scale_percent: u32,
) -> DragOutcome {
    // The release is first settled inside the host work area: a window dragged off
    // the desktop (or across a seam) is pulled back in whole, so neither the pill
    // anchor nor the outcome can be derived from an off-screen point.
    let settled = clamped_rect(win, work);
    match detect_edge_multi_monitor(win, work, scale, all_monitors) {
        Some(edge) if auto_edge_hide => {
            let pill = pill_geometry(edge, work, scale, scale_percent, center_of(settled));
            DragOutcome {
                rect: pill,
                anchor: (settled.x, settled.y),
                anchor_center: center_of(pill),
                edge: Some(edge),
            }
        }
        // Auto hiding off, a seam release, or a plain in-area release: slide it in
        // without folding. The clamp is a no-op when it is already inside.
        _ => DragOutcome {
            rect: settled,
            anchor: (settled.x, settled.y),
            anchor_center: center_of(settled),
            edge: None,
        },
    }
}

/// Persists and applies the arbitrated drag outcome to session state and config manager.
///
/// Pure receiver mapping: guarantees that the exact `outcome.anchor` coordinates are
/// persisted to disk, and `outcome.anchor_center` and `outcome.edge` are committed to dock.
pub fn apply_drag_outcome_to_session<S, D>(
    outcome: &DragOutcome,
    mut save_pos: S,
    mut update_dock: D,
) where
    S: FnMut(f64, f64),
    D: FnMut((i32, i32), Option<Edge>),
{
    save_pos(outcome.anchor.0 as f64, outcome.anchor.1 as f64);
    update_dock(outcome.anchor_center, outcome.edge);
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

/// Every attached monitor's work area, in physical pixels.
fn available_work_areas(window: &WebviewWindow) -> Vec<(PhysicalRect, f64)> {
    window
        .available_monitors()
        .map(|monitors| monitors.iter().map(monitor_bounds).collect())
        .unwrap_or_default()
}

/// The screen the window came from: the monitor under its centre, used to break a
/// 50/50 seam tie. Falls back to the first entry so the result is always an index
/// into `monitors`.
fn monitor_index_for(
    window: &WebviewWindow,
    monitors: &[(PhysicalRect, f64)],
    win: PhysicalRect,
) -> usize {
    if let Some(index) = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|current| monitor_bounds(&current))
        .and_then(|current| monitors.iter().position(|candidate| *candidate == current))
    {
        return index;
    }
    let center = center_of(win);
    monitors
        .iter()
        .position(|(rect, _)| monitor_contains(*rect, center.0 as f64, center.1 as f64))
        .unwrap_or(0)
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
    let win = PhysicalRect {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    };
    let settings = state.settings.lock().await.clone();
    let monitors = available_work_areas(&window);
    if monitors.is_empty() {
        // A session with no monitor information at all: keep the release position,
        // reposition nothing and never panic (module 3 R-1's caller-side contract).
        state
            .config_manager
            .save_position(&WindowPosition { left: position.x as f64, top: position.y as f64 });
        state.dock.set_anchor_center(center_of(win));
        state.dock.finish_drag(None);
        state.dock.save_docked(None);
        emit_state(app, &state.dock);
        return;
    }

    // Drag is free-roaming; only the release is arbitrated. The host screen comes
    // from the intersection areas, with the screen the window came from breaking an
    // exact 50/50 tie (spec §3.2).
    let fallback_index = monitor_index_for(&window, &monitors, win);
    let (work, scale) = select_monitor_by_overlap(win, &monitors, fallback_index)
        .unwrap_or_else(|| monitors[fallback_index]);
    let outcome = resolve_drag_outcome(
        win,
        work,
        scale,
        &monitors,
        settings.auto_edge_hide,
        settings.scale_percent,
    );

    if outcome.rect.width != win.width || outcome.rect.height != win.height {
        if let Err(error) = window
            .set_size(Size::Physical(PhysicalSize::new(outcome.rect.width, outcome.rect.height)))
        {
            eprintln!("Could not resize overlay after drag: {error}");
        }
    }
    if let Err(error) = window
        .set_position(Position::Physical(PhysicalPosition::new(outcome.rect.x, outcome.rect.y)))
    {
        eprintln!("Could not reposition overlay after drag: {error}");
    }
    // The anchor is written only here, after the host-screen arbitration, the seam/
    // boundary decision and the reposition above — so what lands on disk is the
    // authoritative final coordinate, and a restart cannot drift.
    apply_drag_outcome_to_session(
        &outcome,
        |left, top| state.config_manager.save_position(&WindowPosition { left, top }),
        |center, edge| {
            state.dock.set_anchor_center(center);
            state.dock.finish_drag(edge);
            state.dock.save_docked(edge);
        },
    );
    emit_state(app, &state.dock);
}

pub fn mouse_enter(app: &AppHandle, state: &Arc<AppState>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if state.dock.enter_pill() {
        // Capture the centre the expanded session is anchored to *before* the
        // provisional resize: it anchors the transition rectangle, the
        // authoritative write-back and every later reposition of this session
        // (spec §3.4). Resizing first would make `window_center` report the
        // enlarged window's centre instead.
        let anchor_center = state
            .dock
            .anchor_center()
            .unwrap_or_else(|| window_center(&window));
        // Record the pointer position once, at the instant the session opens.
        // Every later reposition of this session reuses this point: reading the
        // live cursor instead would move the window on each background refresh
        // and, once the pointer slid off the pill onto the wider capsule, drop
        // it outside (spec §3.4).
        let entered_cursor = window
            .cursor_position()
            .ok()
            .map(|cursor| (cursor.x.round() as i32, cursor.y.round() as i32));
        state.dock.begin_hover(anchor_center, entered_cursor);
        let state_clone = state.clone();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            let settings = state_clone.settings.lock().await.clone();
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
    // window while it is being dragged or its menu is open (spec §3.1 T6/§3.7),
    // nor while the hover expand's authoritative size write-back is still
    // outstanding (spec §3.4).
    if info.hidden || state.dock.is_awaiting_resize() || state.dock.is_interacting() {
        return;
    }
    let settings = state.settings.lock().await.clone();
    if let Some(edge) = info.edge.filter(|_| info.docked) {
        if info.expanded {
            if let Err(error) = place_docked_expanded(&window, &state.dock) {
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
            state.dock.drop_hover();
            return;
        }
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        // The frontend has just written the authoritative capsule size while
        // the window is still pinned to the pill's edge. Re-place it so the
        // cursor stays inside the final rectangle: the capsule can be shorter
        // than the pill, so centring on the saved anchor alone would leave the
        // cursor in the pill's outer bands outside the window (spec §3.4).
        let result = place_docked_expanded(&window, &state.dock);
        state.dock.finish_hover_resize();
        if let Err(error) = result {
            eprintln!("Could not reposition docked overlay: {error}");
        }
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
/// size, `window_resized` replaces this rectangle through
/// `place_docked_expanded`.
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

/// Places the expanded window while it is docked. The docked axis hugs the
/// same work-area edge as the pill; the free axis is centred on the session's
/// anchor and then kept covering the cursor (spec §3.4). Every reposition of an
/// expanded docked window goes through here — the authoritative size write-back
/// and background refreshes alike — so the two can never disagree about where
/// the window belongs.
fn place_docked_expanded(window: &WebviewWindow, manager: &DockManager) -> Result<(), String> {
    let Some(edge) = manager.docked_edge() else {
        return place_full_at_anchor(window, manager);
    };
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let anchor_center = manager
        .expanded_anchor_center()
        .unwrap_or_else(|| window_center(window));
    let (work, _scale) = work_area_for_point(
        window,
        Some((anchor_center.0 as f64, anchor_center.1 as f64)),
    )
    .ok_or_else(|| "Monitor work area unavailable".to_string())?;
    // The cursor may rest on the pill's outer bands, which the shorter capsule
    // cannot all cover, so the rectangle has to be nudged to keep it inside.
    // The reference is the point recorded when the pointer entered the pill —
    // fixed for the whole session — so the nudge is constant and the window
    // never chases the live pointer (spec §3.4).
    let cursor = manager.hover().and_then(|hover| hover.entered_cursor);
    let rect = docked_expanded_final_rect(edge, work, size, anchor_center, cursor);
    window
        .set_position(Position::Physical(PhysicalPosition::new(rect.x, rect.y)))
        .map_err(|error| error.to_string())
}

/// Geometry of the expanded rectangle once the capsule size is authoritative.
/// Along the docked axis the rectangle hugs the work-area edge the pill is
/// pinned to. Along the free axis it is centred on `anchor_center` (the pill's
/// centre) and then nudged to keep `cursor` — the session's fixed entry point —
/// inside, before being clamped into the work area.
///
/// Centring alone is not enough: the capsule is always shorter than the pill
/// along the free axis (a left/right pill is `PILL_HEIGHT_LOGICAL` = 100
/// logical px, 1.45x the capsule's 69 CSS px — at 175% that is 306 physical px
/// against 211), so the pill's outer bands would stay outside the window and a
/// cursor resting there would receive `mouseleave` the moment the authoritative
/// size lands. The contract for this phase is therefore to cover the cursor's
/// position rather than the whole pill (spec §3.4).
fn docked_expanded_final_rect(
    edge: Edge,
    work: PhysicalRect,
    size: PhysicalSize<u32>,
    anchor_center: (i32, i32),
    cursor: Option<(i32, i32)>,
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
        Edge::Top | Edge::Bottom => cover_cursor(
            anchor_center.0 as i64 - width / 2,
            cursor.map(|(x, _)| x as i64),
            width,
            work_left,
            max_x,
        ),
    };
    let y = match edge {
        Edge::Top => work_top,
        Edge::Bottom => max_y,
        Edge::Left | Edge::Right => cover_cursor(
            anchor_center.1 as i64 - height / 2,
            cursor.map(|(_, y)| y as i64),
            height,
            work_top,
            max_y,
        ),
    };
    PhysicalRect {
        x: x as i32,
        y: y as i32,
        width: size.width,
        height: size.height,
    }
}

/// Moves a `size`-long span by as little as needed so `cursor` stays inside it
/// with `CURSOR_COVER_MARGIN` to spare, then clamps the span into `[min, max]`.
///
/// The cursor lies inside the work area and the span fits there, so a covering
/// position always exists within the clamp bounds and clamping afterwards
/// cannot push the cursor back out.
fn cover_cursor(coord: i64, cursor: Option<i64>, size: i64, min: i64, max: i64) -> i64 {
    let coord = match cursor {
        Some(cursor) => {
            // Never let the margin invert the clamp bounds on tiny windows.
            let margin = CURSOR_COVER_MARGIN.min(size / 2);
            coord.clamp(cursor - size + margin, cursor - margin)
        }
        None => coord,
    };
    coord.clamp(min, max)
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

    /// A single-monitor desktop: every edge of `work` is a physical outer boundary.
    fn solo(work: PhysicalRect, scale: f64) -> Vec<(PhysicalRect, f64)> {
        vec![(work, scale)]
    }

    #[test]
    fn snap_threshold_is_scaled_and_matches_out_of_bounds_releases() {
        let wa = work();
        let near_left = PhysicalRect {
            x: -1947,
            y: 200,
            width: 300,
            height: 100,
        };
        // Dragged past the bezel: the old `distance.abs() <= threshold` test missed
        // this and left the window stranded off-screen, so it must still snap.
        let beyond_left = PhysicalRect {
            x: -1950,
            y: 200,
            width: 300,
            height: 100,
        };
        assert_eq!(detect_edge_multi_monitor(near_left, wa, 1.75, &solo(wa, 1.75)), Some(Edge::Left));
        assert_eq!(detect_edge_multi_monitor(beyond_left, wa, 1.75, &solo(wa, 1.75)), Some(Edge::Left));

        // A window resting well inside the work area matches nothing.
        let middle = PhysicalRect {
            x: -900,
            y: 400,
            width: 300,
            height: 100,
        };
        assert_eq!(detect_edge_multi_monitor(middle, wa, 1.75, &solo(wa, 1.75)), None);
    }

    #[test]
    fn snap_chooses_the_closest_edge_when_multiple_edges_match() {
        let rect = PhysicalRect {
            x: -1924,
            y: 4,
            width: 300,
            height: 100,
        };
        assert_eq!(detect_edge_multi_monitor(rect, work(), 1.0, &solo(work(), 1.0)), Some(Edge::Left));
    }

    /// Two 1920x1040 screens side by side at 100%, i.e. A: 0..1920, B: 1920..3840.
    fn side_by_side() -> [(PhysicalRect, f64); 2] {
        [
            (PhysicalRect { x: 0, y: 0, width: 1920, height: 1040 }, 1.0),
            (PhysicalRect { x: 1920, y: 0, width: 1920, height: 1040 }, 1.0),
        ]
    }

    #[test]
    fn a_cross_screen_release_lands_on_the_screen_holding_the_majority() {
        let monitors = side_by_side();
        // 1000x400 window released with 700px (70%) over screen B.
        let mostly_b = PhysicalRect { x: 1620, y: 200, width: 1000, height: 400 };
        assert_eq!(select_monitor_by_overlap(mostly_b, &monitors, 0), Some(monitors[1]));
        // The same window released with only 400px (40%) over screen B: mostly A, so
        // despite the host being A this is decided by area, not by the host.
        let mostly_a = PhysicalRect { x: 1320, y: 200, width: 1000, height: 400 };
        assert_eq!(select_monitor_by_overlap(mostly_a, &monitors, 1), Some(monitors[0]));
    }

    #[test]
    fn an_exact_seam_tie_stays_on_the_host_screen_in_any_order() {
        let monitors = side_by_side();
        // Exactly 50/50 across the seam.
        let straddling = PhysicalRect { x: 1420, y: 200, width: 1000, height: 400 };
        assert_eq!(monitors[0].0.intersection_area(&straddling), monitors[1].0.intersection_area(&straddling));
        // The host keeps the window: from A it stays on A, from B it stays on B.
        assert_eq!(select_monitor_by_overlap(straddling, &monitors, 0), Some(monitors[0]));
        assert_eq!(select_monitor_by_overlap(straddling, &monitors, 1), Some(monitors[1]));

        // Reversing the enumeration must not change the physical answer: the host is
        // now index 1 on the same screen.
        let reversed = [monitors[1], monitors[0]];
        assert_eq!(select_monitor_by_overlap(straddling, &reversed, 1), Some(reversed[1]));
    }

    #[test]
    fn a_tie_between_two_non_host_screens_is_geometrically_stable() {
        // Three screens in a row; the window straddles B|C 50/50 and never touches
        // the host A, so the host cannot break the tie.
        let a = (PhysicalRect { x: 0, y: 0, width: 1920, height: 1040 }, 1.0);
        let b = (PhysicalRect { x: 1920, y: 0, width: 1920, height: 1040 }, 1.0);
        let c = (PhysicalRect { x: 3840, y: 0, width: 1920, height: 1040 }, 1.0);
        let straddling = PhysicalRect { x: 3740, y: 200, width: 200, height: 400 };
        assert_eq!(b.0.intersection_area(&straddling), c.0.intersection_area(&straddling));
        assert_eq!(a.0.intersection_area(&straddling), 0);

        assert_eq!(select_monitor_by_overlap(straddling, &[a, b, c], 0), Some(b));
        // A different `available_monitors()` order must yield the same physical
        // screen, not merely a different index.
        assert_eq!(select_monitor_by_overlap(straddling, &[a, c, b], 0), Some(b));
        assert_eq!(select_monitor_by_overlap(straddling, &[c, b, a], 2), Some(b));
    }

    #[test]
    fn the_strictly_largest_share_wins_even_without_a_majority() {
        // Staggered layout: D sits below B. The window is on the A|B|D corner and no
        // single screen holds >50%, but A holds the strictly largest share — so A
        // wins and the "no majority keeps the original screen" reading is excluded.
        let a = (PhysicalRect { x: 0, y: 0, width: 1920, height: 1040 }, 1.0);
        let b = (PhysicalRect { x: 1920, y: 0, width: 1920, height: 1040 }, 1.0);
        let d = (PhysicalRect { x: 1920, y: 1040, width: 1920, height: 1040 }, 1.0);
        let win = PhysicalRect { x: 1700, y: 900, width: 400, height: 200 };
        let union = win.width as i64 * win.height as i64;
        let shares = [a.0.intersection_area(&win), b.0.intersection_area(&win), d.0.intersection_area(&win)];
        assert!(shares.iter().all(|share| *share * 2 < union), "no screen may hold a majority: {shares:?}");
        assert!(shares[0] > shares[1] && shares[1] > shares[2], "A must be the strict maximum: {shares:?}");

        // D is the host (index 2), yet A wins.
        assert_eq!(select_monitor_by_overlap(win, &[a, b, d], 2), Some(a));
    }

    #[test]
    fn zero_overlap_falls_back_to_the_host_then_the_first_screen() {
        // Different scale factors so the assertion covers the scale that comes back.
        let a = (PhysicalRect { x: 0, y: 0, width: 1920, height: 1040 }, 1.0);
        let b = (PhysicalRect { x: 1920, y: 0, width: 1920, height: 1040 }, 1.5);
        let monitors = [a, b];
        // Off the desktop entirely: the host's real work area and scale come back.
        let offscreen = PhysicalRect { x: 10_000, y: 10_000, width: 300, height: 100 };
        assert_eq!(select_monitor_by_overlap(offscreen, &monitors, 0), Some(a));
        assert_eq!(select_monitor_by_overlap(offscreen, &monitors, 1), Some(b));
        // An out-of-range host index degrades to the first screen.
        assert_eq!(select_monitor_by_overlap(offscreen, &monitors, 99), Some(a));
    }

    #[test]
    fn an_empty_monitor_list_returns_none() {
        let win = PhysicalRect { x: 0, y: 0, width: 300, height: 100 };
        assert_eq!(select_monitor_by_overlap(win, &[], 0), None);
        assert_eq!(select_monitor_by_overlap(win, &[], 7), None);
    }

    #[test]
    fn an_internal_seam_is_never_treated_as_an_outer_boundary() {
        let monitors = side_by_side();
        let a = monitors[0].0;
        let b = monitors[1].0;
        // B's left edge and A's right edge touch, so neither is external.
        assert!(!is_external_boundary(a, Edge::Right, &monitors));
        assert!(!is_external_boundary(b, Edge::Left, &monitors));
        // Their outer edges still are.
        assert!(is_external_boundary(a, Edge::Left, &monitors));
        assert!(is_external_boundary(b, Edge::Right, &monitors));
        assert!(is_external_boundary(a, Edge::Top, &monitors));
        assert!(is_external_boundary(a, Edge::Bottom, &monitors));
    }

    #[test]
    fn a_release_in_the_seam_channel_never_folds() {
        let monitors = side_by_side();
        let a = monitors[0].0;
        let b = monitors[1].0;
        // Released in the channel, overlapping both screens' seam edges.
        let in_channel = PhysicalRect { x: 1900, y: 400, width: 40, height: 200 };
        assert_eq!(detect_edge_multi_monitor(in_channel, a, 1.0, &monitors), None);
        assert_eq!(detect_edge_multi_monitor(in_channel, b, 1.0, &monitors), None);

        // Even with auto edge hiding on, an unbroken channel release slides into the
        // host screen without collapsing into a pill.
        let outcome = resolve_drag_outcome(in_channel, b, 1.0, &monitors, true, 175);
        assert_eq!(outcome.edge, None);
        assert_eq!(outcome.rect, PhysicalRect { x: 1920, y: 400, width: 40, height: 200 });
        assert!(contains(b, outcome.rect));
    }

    #[test]
    fn an_outer_boundary_release_folds_or_clamps_with_auto_hide() {
        let monitors = side_by_side();
        let a = monitors[0].0;
        // Dragged 50px off the left bezel of the leftmost screen.
        let overshot = PhysicalRect { x: -50, y: 400, width: 300, height: 100 };
        assert_eq!(detect_edge_multi_monitor(overshot, a, 1.0, &monitors), Some(Edge::Left));

        // Auto edge hiding on: fold into a pill pinned to that edge.
        let folded = resolve_drag_outcome(overshot, a, 1.0, &monitors, true, 175);
        assert_eq!(folded.edge, Some(Edge::Left));
        let settled_center = center_of(PhysicalRect { x: 0, y: 400, width: 300, height: 100 });
        assert_eq!(folded.rect, pill_geometry(Edge::Left, a, 1.0, 175, settled_center));
        assert!(contains(a, folded.rect));

        // Auto edge hiding off: clamp back inside instead of folding, and never leave
        // the window off-screen.
        let clamped = resolve_drag_outcome(overshot, a, 1.0, &monitors, false, 175);
        assert_eq!(clamped.edge, None);
        assert_eq!(clamped.rect, PhysicalRect { x: 0, y: 400, width: 300, height: 100 });
        assert!(contains(a, clamped.rect));
    }

    #[test]
    fn a_seam_release_slides_the_window_fully_into_the_target_screen() {
        let monitors = side_by_side();
        let b = monitors[1].0;
        // 70% over B: the host is arbitrated to B and the window is then pulled whole
        // into B's work area, so it no longer straddles the physical seam.
        let mostly_b = PhysicalRect { x: 1620, y: 200, width: 1000, height: 400 };
        let work = select_monitor_by_overlap(mostly_b, &monitors, 0).expect("a host").0;
        assert_eq!(work, b);
        let outcome = resolve_drag_outcome(mostly_b, work, 1.0, &monitors, true, 175);
        assert_eq!(outcome.edge, None);
        assert_eq!(outcome.rect, PhysicalRect { x: 1920, y: 200, width: 1000, height: 400 });
        assert!(contains(b, outcome.rect));
    }

    /// P3-4: what is persisted is the corrected on-screen coordinate, never the raw
    /// release point, so a restart cannot drift off-screen.
    #[test]
    fn the_persisted_anchor_is_the_final_corrected_coordinate() {
        let monitors = side_by_side();
        let a = monitors[0].0;
        let overshot = PhysicalRect { x: -50, y: 400, width: 300, height: 100 };

        // Clamped release: the off-screen x is corrected to 0 before anything is
        // written, and the recorded centre follows the corrected rectangle.
        let clamped = resolve_drag_outcome(overshot, a, 1.0, &monitors, false, 175);
        assert_eq!(clamped.anchor, (0, 400));
        assert_ne!(clamped.anchor, (overshot.x, overshot.y));
        assert!(monitor_contains(a, clamped.anchor.0 as f64, clamped.anchor.1 as f64));
        assert_eq!(clamped.anchor_center, center_of(clamped.rect));

        // Folded release: the persisted anchor is still the corrected expanded
        // coordinate (what `estimated_anchor_center` re-derives from on the next
        // refresh), and the recorded centre is the pill's own final centre.
        let folded = resolve_drag_outcome(overshot, a, 1.0, &monitors, true, 175);
        assert_eq!(folded.anchor, (0, 400));
        assert_ne!(folded.anchor, (overshot.x, overshot.y));
        assert_eq!(folded.anchor_center, center_of(folded.rect));

        // Cross-screen drag: the anchor is the corrected target-screen coordinate.
        let mostly_b = PhysicalRect { x: 1620, y: 200, width: 1000, height: 400 };
        let crossed = resolve_drag_outcome(mostly_b, monitors[1].0, 1.0, &monitors, true, 175);
        assert_eq!(crossed.anchor, (1920, 200));
        assert_ne!(crossed.anchor, (mostly_b.x, mostly_b.y));
        assert!(monitor_contains(monitors[1].0, crossed.anchor.0 as f64, crossed.anchor.1 as f64));
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

    /// Physical capsule size the frontend reports for the v1.1.0 capsule at
    /// 175% (284.31 x 69 CSS px untransformed), as measured in rounds 1-3.
    fn capsule() -> PhysicalSize<u32> {
        PhysicalSize::new(871, 211)
    }

    fn physical_work_area() -> PhysicalRect {
        PhysicalRect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        }
    }

    fn pill_on(edge: Edge, anchor_center: (i32, i32)) -> PhysicalRect {
        pill_geometry(edge, physical_work_area(), 1.75, 175, anchor_center)
    }

    /// A cursor counts as inside while it lies within the inclusive bounds.
    fn contains_point(rect: PhysicalRect, x: i32, y: i32) -> bool {
        let left = rect.x as i64;
        let top = rect.y as i64;
        x as i64 >= left
            && x as i64 <= left + rect.width as i64
            && y as i64 >= top
            && y as i64 <= top + rect.height as i64
    }

    /// R3-1: the authoritative size write-back must leave the window covering
    /// the cursor no matter where on the pill it entered. The capsule (211
    /// physical px tall at 175%) is shorter than a left/right pill (306), so
    /// "cover the whole pill" is unreachable once the real size lands — this
    /// asserts the reachable contract of §3.4: cover the cursor's position.
    #[test]
    fn docked_expanded_final_rect_keeps_the_cursor_inside_on_every_edge() {
        let wa = physical_work_area();
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            // Offsets span the middle of the edge and both sides of it, i.e.
            // every snap position a user can legitimately release at.
            for offset in [-400, -120, -28, 0, 28, 120, 400] {
                let anchor = match edge {
                    Edge::Left | Edge::Right => (1280, 720 + offset),
                    Edge::Top | Edge::Bottom => (1280 + offset, 720),
                };
                let pill = pill_on(edge, anchor);
                // Any point of the pill can hold the cursor at write-back time.
                for x in pill.x..=pill.x + pill.width as i32 {
                    for y in pill.y..=pill.y + pill.height as i32 {
                        let rect = docked_expanded_final_rect(edge, wa, capsule(), anchor, Some((x, y)));
                        assert!(
                            contains_point(rect, x, y),
                            "{edge:?} at offset {offset}: final {rect:?} must contain cursor ({x}, {y}) of pill {pill:?}"
                        );
                        let flush = match edge {
                            Edge::Left => rect.x == wa.x,
                            Edge::Right => rect.x + rect.width as i32 == wa.x + wa.width as i32,
                            Edge::Top => rect.y == wa.y,
                            Edge::Bottom => rect.y + rect.height as i32 == wa.y + wa.height as i32,
                        };
                        assert!(
                            flush && contains(wa, rect),
                            "{edge:?} at offset {offset}: final {rect:?} must hug the edge inside {wa:?}"
                        );
                    }
                }
            }
        }
    }

    /// The exact geometry of round 3's counterexample: the cursor rests in the
    /// pill's outer bands, which the anchor-centred rectangle misses.
    #[test]
    fn docked_expanded_final_rect_covers_round_three_counterexample() {
        let wa = physical_work_area();
        let anchor_center = (2124, 505);
        let pill = pill_on(Edge::Right, anchor_center);
        assert_eq!(
            pill,
            PhysicalRect {
                x: 2415,
                y: 352,
                width: 141,
                height: 306
            }
        );
        // The pre-fix placement centred the capsule on the anchor, i.e. y in
        // [400, 611]; y = 360 (pill y in [352, 658]) fell outside.
        let top_band = docked_expanded_final_rect(Edge::Right, wa, capsule(), anchor_center, Some((2480, 360)));
        assert!(contains_point(top_band, 2480, 360), "{top_band:?}");
        let bottom_band = docked_expanded_final_rect(Edge::Right, wa, capsule(), anchor_center, Some((2480, 650)));
        assert!(contains_point(bottom_band, 2480, 650), "{bottom_band:?}");
    }

    #[test]
    fn docked_expanded_final_rect_centres_on_the_anchor_without_a_cursor() {
        let rect = docked_expanded_final_rect(
            Edge::Right,
            physical_work_area(),
            capsule(),
            (2124, 505),
            None,
        );
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (1689, 400, 871, 211));
    }

    #[test]
    fn hover_guard_stands_down_until_the_write_back_or_the_lease_expires() {
        let manager = manager_with(DockState::Docked(Edge::Right), true);
        let entered_cursor = (2435, 657);
        assert!(!manager.is_awaiting_resize());
        assert_eq!(manager.expanded_anchor_center(), None);

        manager.begin_hover((2124, 505), Some(entered_cursor));
        assert!(manager.is_awaiting_resize());
        assert_eq!(manager.hover().and_then(|hover| hover.entered_cursor), Some(entered_cursor));
        assert_eq!(manager.expanded_anchor_center(), Some((2124, 505)));

        // The authoritative write-back releases the guard but keeps the session
        // anchor and entry cursor, so later repositions of the same session stay
        // session-stable.
        manager.finish_hover_resize();
        assert!(!manager.is_awaiting_resize());
        assert_eq!(manager.hover().and_then(|hover| hover.entered_cursor), Some(entered_cursor));
        assert_eq!(manager.expanded_anchor_center(), Some((2124, 505)));

        // A lost `dock_window_resized` cannot latch the guard on.
        manager.begin_hover((2124, 505), Some(entered_cursor));
        let hover = manager.hover().unwrap();
        lock(&manager.runtime).hover = Some(Hover {
            resize_deadline: Some(Instant::now() - Duration::from_millis(1)),
            ..hover
        });
        assert!(!manager.is_awaiting_resize());
    }

    #[test]
    fn collapse_and_a_failed_expansion_drop_the_hover_session() {
        let manager = manager_with(DockState::Docked(Edge::Right), true);
        manager.begin_hover((2124, 505), Some((2435, 657)));
        assert!(manager.is_awaiting_resize());
        let timer = manager.leave_window().unwrap();
        assert!(manager.collapse_if_current(timer).is_some());
        assert!(manager.hover().is_none());
        assert!(!manager.is_awaiting_resize());

        let manager = manager_with(DockState::Docked(Edge::Left), false);
        assert!(manager.enter_pill());
        manager.begin_hover((74, 253), Some((74, 253)));
        assert!(manager.is_awaiting_resize());
        manager.revert_expansion();
        assert!(!manager.info().expanded);
        assert!(!manager.is_awaiting_resize());
        assert!(manager.hover().is_none());
    }

    /// R4-1: an expanded hover session is session-stable. `place_docked_expanded`
    /// takes its cursor input from the point recorded when the pointer entered
    /// the pill, never from the live pointer, so every reposition of a session —
    /// the authoritative size write-back and each background usage refresh
    /// alike — computes the identical rectangle.
    ///
    /// The pre-fix code re-read the live cursor and clamped only while the
    /// pointer still rested on the 141 physical px-wide pill hit area. The
    /// capsule is 871 px wide, so sliding left inside it left that hit area, the
    /// clamp was dropped, and the next refresh jumped the window by up to 96
    /// physical px — enough to drop a pointer that was inside the window a
    /// moment earlier.
    #[test]
    fn docked_expanded_placement_is_session_stable_and_idempotent() {
        let wa = physical_work_area();
        let anchor_center = (2124, 505);
        let pill = pill_on(Edge::Right, anchor_center);
        // Right-edge pill: 141 x 306 physical px, far narrower than the capsule.
        assert_eq!((pill.width, pill.height), (141, 306));

        // Mirrors `place_docked_expanded`: the cursor input is the session's
        // recorded entry point, read back from the manager; the live pointer is
        // deliberately not an input.
        let place = |manager: &DockManager, _live: (i32, i32)| {
            let anchor = manager.expanded_anchor_center().unwrap();
            let cursor = manager.hover().and_then(|hover| hover.entered_cursor);
            docked_expanded_final_rect(Edge::Right, wa, capsule(), anchor, cursor)
        };

        // Enter on the pill's bottom band — the R4-1 counterexample — then slide
        // left within the capsule, off the pill hit area (x < 2415) but still
        // inside the window. Every reposition must yield the same rectangle and
        // keep covering the live pointer.
        let manager = manager_with(DockState::Docked(Edge::Right), true);
        let entered = (2435, 657);
        manager.begin_hover(anchor_center, Some(entered));
        // The write-back releases the guard; the session inputs stay frozen.
        manager.finish_hover_resize();
        let placed = place(&manager, entered);
        assert!(contains_point(placed, entered.0, entered.1));
        for live in [(2435, 657), (2415, 657), (2375, 657), (2200, 640), (1900, 500)] {
            let again = place(&manager, live);
            assert_eq!(again, placed, "session must not move while the pointer is at {live:?}");
            assert!(
                contains_point(again, live.0, live.1),
                "session rect {again:?} must keep covering the live pointer {live:?}"
            );
        }

        // The same holds for an entry on the pill's top band.
        let manager = manager_with(DockState::Docked(Edge::Right), true);
        let entered = (2435, 352);
        manager.begin_hover(anchor_center, Some(entered));
        manager.finish_hover_resize();
        let placed = place(&manager, entered);
        for live in [(2435, 352), (2375, 352), (1900, 360)] {
            let again = place(&manager, live);
            assert_eq!(again, placed, "session must not move while the pointer is at {live:?}");
            assert!(contains_point(again, live.0, live.1));
        }

        // Regression guard for the exact R4-1 failure: once the pointer left the
        // pill hit area but was still inside the window, the pre-fix placement
        // fell back to the anchor-centred rectangle and dropped it.
        let pre_fix = docked_expanded_final_rect(Edge::Right, wa, capsule(), anchor_center, None);
        assert_ne!(pre_fix, place(&manager, (2375, 352)));
        assert!(!contains_point(pre_fix, 2375, 657));
        assert!(!contains_point(pre_fix, 2375, 352));
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
                hover: None,
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

    #[test]
    fn drag_outcome_persists_settled_anchor_and_never_constants() {
        let outcome = DragOutcome {
            rect: PhysicalRect { x: 50, y: 60, width: 300, height: 200 },
            anchor: (123, 456),
            anchor_center: (273, 556),
            edge: Some(Edge::Left),
        };
        let mut saved_coords = (0.0, 0.0);
        let mut dock_state = ((0, 0), None);
        apply_drag_outcome_to_session(
            &outcome,
            |left, top| saved_coords = (left, top),
            |center, edge| dock_state = (center, edge),
        );
        assert_eq!(saved_coords, (123.0, 456.0), "persisted anchor must match outcome.anchor exactly");
        assert_eq!(dock_state, ((273, 556), Some(Edge::Left)));
    }
}
