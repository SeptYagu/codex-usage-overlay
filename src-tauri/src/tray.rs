use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex as StdMutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Position, Size,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry,
};

pub const TRAY_ID: &str = "main-tray";
/// Label of the tray popup webview window.
pub const TRAY_MENU_WINDOW_LABEL: &str = "tray-menu";
const SETTINGS_DEFAULT_WIDTH: f64 = 960.0;
const SETTINGS_DEFAULT_HEIGHT: f64 = 620.0;
const SETTINGS_MIN_WIDTH: f64 = 760.0;
const SETTINGS_MIN_HEIGHT: f64 = 500.0;
const SETTINGS_MAX_WIDTH: f64 = 1600.0;
const SETTINGS_MAX_HEIGHT: f64 = 1600.0;
/// Safety bounds for the tray menu popup width, in logical pixels. The floor is the
/// frontend contract's own minimum (`TRAY_MENU_MIN_WIDTH` in `TrayMenuView.tsx`), so
/// the popup stays compact in the common case and the two ends cannot disagree.
const TRAY_MENU_MIN_WIDTH: f64 = 280.0;
const TRAY_MENU_MAX_WIDTH: f64 = 500.0;
/// Debounce window for tray clicks: a rapid double click (or a burst of clicks)
/// coalesces into a single show/hide intent instead of firing a toggle per event.
pub const TRAY_MENU_CLICK_DEBOUNCE: Duration = Duration::from_millis(280);
/// Length of the focus grace period that follows a successful `window.show()`.
pub const TRAY_MENU_FOCUS_GRACE: Duration = Duration::from_millis(180);
static TRAY_MENU_GENERATION: AtomicU64 = AtomicU64::new(0);
static TRAY_MENU_LAYOUT_REVISION: AtomicU64 = AtomicU64::new(0);
static TRAY_MENU_LAYOUT_LOCK: StdMutex<()> = StdMutex::new(());
/// Live click debounce/Toggle state. The decision logic itself lives in
/// `TrayMenuClickGate`, which takes its clock as a parameter and is therefore
/// drivable from an offline unit test.
static TRAY_MENU_CLICK_GATE: StdMutex<TrayMenuClickGate> = StdMutex::new(TrayMenuClickGate::new());

/// What a gated tray click should do.
///
/// Modelling the decision as a value — instead of performing the window calls
/// inline — is what makes the debounce and the Toggle semantics falsifiable without
/// an `AppHandle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuClickAction {
    /// The popup was not visible: open (or re-open) it and bump the generation.
    Open,
    /// The popup was already visible: collapse it without re-opening it.
    Hide,
    /// Swallowed by the debounce window: do nothing at all.
    Ignore,
}

/// 250~300 ms click debounce plus an idempotent Toggle state machine, replacing the
/// mechanical `DoubleClick` listener.
///
/// Rapid clicks and double clicks coalesce into one Toggle intent. While the menu is
/// already expanded a double click is a single deliberate collapse — it is never
/// re-opened, so the generation does not churn.
///
/// The clock is a parameter rather than `Instant::now()`, so tests drive it
/// deterministically and never depend on a real sleep.
#[derive(Debug, Default)]
pub struct TrayMenuClickGate {
    last_click: Option<Instant>,
}

impl TrayMenuClickGate {
    pub const fn new() -> Self {
        Self { last_click: None }
    }

    pub fn on_click(&mut self, now: Instant, menu_visible: bool) -> TrayMenuClickAction {
        if let Some(last) = self.last_click {
            if now.saturating_duration_since(last) < TRAY_MENU_CLICK_DEBOUNCE {
                return TrayMenuClickAction::Ignore;
            }
        }
        self.last_click = Some(now);
        if menu_visible {
            TrayMenuClickAction::Hide
        } else {
            TrayMenuClickAction::Open
        }
    }
}

/// Result of driving the tray popup's focus grace period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuFocusAction {
    /// Leave the popup exactly as it is.
    Keep,
    /// Close the popup.
    Hide,
}

/// The tray popup's focus grace state: when it was last shown and whether a blur
/// arrived inside the grace window.
///
/// Holding the state in a struct with an injectable clock — instead of hiding it
/// inside the `on_window_event` closure — is what lets the guard below be asserted
/// offline.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrayMenuFocusState {
    /// The plan's `SHOW_TIMESTAMP`: the instant `window.show()` last succeeded. It
    /// is the start of the grace period and the guard's only reference point.
    show_timestamp: Option<Instant>,
    pending_blur: bool,
}

impl TrayMenuFocusState {
    /// Anchors the grace window at the instant `window.show()` succeeded.
    pub fn on_shown(&mut self, now: Instant) {
        self.show_timestamp = Some(now);
        self.pending_blur = false;
    }

    /// Records a `Focused(false)` event.
    ///
    /// Inside the grace window the blur is *remembered* rather than acted on: on
    /// Windows a background process clicking the tray icon often gets a transient
    /// kill-focus while the popup is still being focused, and hiding on it is what
    /// made the click look like it did nothing. A blur after the grace window has
    /// closed is an ordinary dismissal and hides at once.
    pub fn on_blur(&mut self, now: Instant) -> TrayMenuFocusAction {
        if self.in_grace(now) {
            self.pending_blur = true;
            TrayMenuFocusAction::Keep
        } else {
            self.clear();
            TrayMenuFocusAction::Hide
        }
    }

    /// The grace timer's authoritative verdict — the unique guard contract.
    ///
    /// The popup is hidden **only** when a blur was actually observed *and* the
    /// window is still unfocused. A window that never received `Focused(false)` must
    /// stay visible even if Windows never granted it the foreground and
    /// `is_focused()` reports false; otherwise the always-on-top popup would be
    /// killed by a focus state it never observed.
    pub fn on_grace_expired(&mut self, is_focused: bool) -> TrayMenuFocusAction {
        let should_hide = self.pending_blur && !is_focused;
        self.clear();
        if should_hide {
            TrayMenuFocusAction::Hide
        } else {
            TrayMenuFocusAction::Keep
        }
    }

    fn in_grace(&self, now: Instant) -> bool {
        self.show_timestamp
            .is_some_and(|shown| now.saturating_duration_since(shown) < TRAY_MENU_FOCUS_GRACE)
    }

    fn clear(&mut self) {
        self.show_timestamp = None;
        self.pending_blur = false;
    }
}


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
                    MouseButton::Left | MouseButton::Right => handle_tray_click(app),
                    _ => {}
                }
            }
        })
        .build(app)?;

    Ok(tray)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuWindowEffect {
    Open,
    Hide,
    None,
}

/// Arbitrates what window action must follow a click gate decision.
pub fn plan_tray_click_effect(action: TrayMenuClickAction) -> TrayMenuWindowEffect {
    match action {
        TrayMenuClickAction::Open => TrayMenuWindowEffect::Open,
        TrayMenuClickAction::Hide => TrayMenuWindowEffect::Hide,
        TrayMenuClickAction::Ignore => TrayMenuWindowEffect::None,
    }
}

pub trait TrayClickWindowOps {
    fn open_menu(&mut self);
    fn hide_menu(&mut self);
}

pub fn execute_tray_click_effect<W: TrayClickWindowOps>(
    ops: &mut W,
    effect: TrayMenuWindowEffect,
) {
    match effect {
        TrayMenuWindowEffect::Open => ops.open_menu(),
        TrayMenuWindowEffect::Hide => ops.hide_menu(),
        TrayMenuWindowEffect::None => {}
    }
}

/// Entry point for a tray icon click: debounce it, then open or collapse the popup.
///
/// Both buttons share one intent (spec: "点击托盘时若已可见则收起"), so rapid clicks
/// and double clicks produce at most one action.
pub fn handle_tray_click(app: &AppHandle) {
    let visible = app
        .get_webview_window(TRAY_MENU_WINDOW_LABEL)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    let action = match TRAY_MENU_CLICK_GATE.lock() {
        Ok(mut gate) => gate.on_click(Instant::now(), visible),
        Err(_) => TrayMenuClickAction::Ignore,
    };
    let effect = plan_tray_click_effect(action);
    struct LiveClickOps<'a>(&'a AppHandle);
    impl<'a> TrayClickWindowOps for LiveClickOps<'a> {
        fn open_menu(&mut self) {
            open_tray_menu_window(self.0);
        }
        fn hide_menu(&mut self) {
            if let Some(window) = self.0.get_webview_window(TRAY_MENU_WINDOW_LABEL) {
                let _ = window.hide();
            }
        }
    }
    execute_tray_click_effect(&mut LiveClickOps(app), effect);
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

/// Reads the live focus-grace state, or `None` when the app state is unavailable.
fn with_tray_menu_focus<R>(app: &AppHandle, f: impl FnOnce(&mut TrayMenuFocusState) -> R) -> Option<R> {
    let state = app.try_state::<Arc<crate::commands::AppState>>()?;
    state.tray_menu_focus.lock().ok().map(|mut focus| f(&mut focus))
}

/// Anchors the focus grace period at the instant `window.show()` succeeded.
fn note_tray_menu_shown(app: &AppHandle) {
    let _ = with_tray_menu_focus(app, |focus| focus.on_shown(Instant::now()));
}

/// Pure decision for window blur event: whether to hide immediately or wait for grace period.
pub fn should_hide_on_tray_menu_blur(focus: &mut TrayMenuFocusState, now: Instant) -> bool {
    focus.on_blur(now) == TrayMenuFocusAction::Hide
}

/// Drives the grace period for a `Focused(false)` event on the tray popup.
///
/// Returns true when the caller must hide the popup right away — i.e. when the blur
/// arrived *after* the grace window, or when no focus state is available at all.
pub fn on_tray_menu_blur(app: &AppHandle) -> bool {
    with_tray_menu_focus(app, |focus| should_hide_on_tray_menu_blur(focus, Instant::now()))
        .unwrap_or(true)
}

/// The grace timer: the only place where a blur *inside* the grace window can close
/// the popup. It re-checks the window's focus state on expiry, so the `pending_blur`
/// flag and the focus state must *both* agree before anything is hidden.
fn schedule_tray_menu_grace_check(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(TRAY_MENU_FOCUS_GRACE).await;
        let Some(window) = app.get_webview_window(TRAY_MENU_WINDOW_LABEL) else {
            return;
        };
        let is_focused = window.is_focused().unwrap_or(false);
        let hide = with_tray_menu_focus(&app, |focus| focus.on_grace_expired(is_focused))
            .map(|action| action == TrayMenuFocusAction::Hide)
            .unwrap_or(false);
        if hide {
            let _ = window.hide();
        }
    });
}

/// Resolves the tray icon's physical anchor rectangle.
///
/// `rect` is `None` when the Windows API fails — which it does while the icon sits
/// in the notification-area overflow flyout. That case degrades to the bottom-right
/// corner of the fallback monitor's work area, so the popup is still shown inside
/// the screen instead of the layout aborting before `show()`.
fn tray_icon_anchor(
    rect: Option<(i32, i32, i32, i32)>,
    work: Option<(i32, i32, i32, i32)>,
) -> Option<(i32, i32, i32, i32)> {
    match rect {
        Some(rect) => Some(rect),
        None => work.map(|(left, top, width, height)| {
            (left.saturating_add(width), top.saturating_add(height), 0, 0)
        }),
    }
}

/// The physical rectangle the tray popup is placed at, plus the logical height it
/// reports back to the frontend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrayMenuPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub height_logical: f64,
}

/// Places the popup next to the given icon anchor. Pure, so `rect() == None` can be
/// asserted offline: the caller owns the window calls.
fn plan_tray_menu_placement(
    icon: (i32, i32, i32, i32),
    work: (i32, i32, i32, i32),
    scale: f64,
    height_logical: f64,
    width_logical: f64,
) -> TrayMenuPlacement {
    let (icon_x, icon_y, icon_width, icon_height) = icon;
    let (left, top, work_width, work_height) = work;
    let margin = (8.0 * scale).ceil() as i32;
    let right = left + work_width;
    let bottom = top + work_height;
    let desired_width = width_logical.clamp(TRAY_MENU_MIN_WIDTH, TRAY_MENU_MAX_WIDTH);
    let width = (desired_width * scale).ceil() as i32;
    let width = width.min(work_width - margin * 2).max(1);
    let desired_height = (height_logical * scale).ceil() as i32;
    let height = desired_height.min(work_height - margin * 2).max(1);
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
    TrayMenuPlacement {
        x,
        y,
        width: width as u32,
        height: height as u32,
        height_logical: height as f64 / scale,
    }
}

/// Resolves the tray icon anchor box from a raw tray rect result, degrading safely
/// to `None` on any error (so `tray_icon_anchor` falls back to the monitor corner).
pub fn resolve_tray_icon_anchor_box(
    tray_rect_res: Result<Option<(i32, i32, i32, i32)>, String>,
    fallback_work: Option<(i32, i32, i32, i32)>,
) -> Option<(i32, i32, i32, i32)> {
    let icon_rect = tray_rect_res.ok().flatten();
    tray_icon_anchor(icon_rect, fallback_work)
}

/// Fallback-aware anchor computation guaranteeing that any tray rect error
/// degrades safely to the fallback monitor work area without aborting layout.
pub fn compute_tray_anchor_with_fallback(
    rect_result: Result<Option<(i32, i32, i32, i32)>, String>,
    fallback_work: Option<(i32, i32, i32, i32)>,
) -> Option<(i32, i32, i32, i32)> {
    resolve_tray_icon_anchor_box(rect_result, fallback_work)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuShowAction {
    SetSize { width: u32, height: u32 },
    SetPosition { x: i32, y: i32 },
    Show,
    NoteShown,
    SetFocus,
    ScheduleGraceCheck,
}

pub trait TrayMenuShowOps {
    fn set_size(&mut self, width: u32, height: u32);
    fn set_position(&mut self, x: i32, y: i32);
    fn show(&mut self);
    fn note_shown(&mut self);
    fn set_focus(&mut self);
    fn schedule_grace_check(&mut self);
}

pub fn execute_tray_menu_show_action<W: TrayMenuShowOps>(
    ops: &mut W,
    action: TrayMenuShowAction,
) {
    match action {
        TrayMenuShowAction::SetSize { width, height } => ops.set_size(width, height),
        TrayMenuShowAction::SetPosition { x, y } => ops.set_position(x, y),
        TrayMenuShowAction::Show => ops.show(),
        TrayMenuShowAction::NoteShown => ops.note_shown(),
        TrayMenuShowAction::SetFocus => ops.set_focus(),
        TrayMenuShowAction::ScheduleGraceCheck => ops.schedule_grace_check(),
    }
}

pub fn drive_tray_menu_show_sequence<F: FnMut(TrayMenuShowAction)>(
    placement: &TrayMenuPlacement,
    mut emit_action: F,
) {
    emit_action(TrayMenuShowAction::SetSize {
        width: placement.width,
        height: placement.height,
    });
    emit_action(TrayMenuShowAction::SetPosition {
        x: placement.x,
        y: placement.y,
    });
    emit_action(TrayMenuShowAction::Show);
    emit_action(TrayMenuShowAction::NoteShown);
    emit_action(TrayMenuShowAction::SetFocus);
    emit_action(TrayMenuShowAction::ScheduleGraceCheck);
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
    let window = app.get_webview_window(TRAY_MENU_WINDOW_LABEL).ok_or("Tray menu window unavailable")?;
    let tray = app.tray_by_id(TRAY_ID).ok_or("Tray icon unavailable")?;
    // `tray.rect()` fails while the icon lives in the notification-area overflow
    // flyout. It must degrade to the fallback corner rather than short-circuit the
    // layout: the `?` that used to sit here skipped `window.show()` entirely, so a
    // click on such an icon looked like it did nothing at all.
    let fallback_monitor = window.current_monitor().ok().flatten().or_else(|| window.primary_monitor().ok().flatten());
    let fallback_scale = fallback_monitor.as_ref().map(tauri::Monitor::scale_factor).unwrap_or(1.0);
    let raw_icon_rect = tray.rect().map_err(|e| e.to_string()).map(|opt_rect| {
        opt_rect.map(|rect| {
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
        })
    });
    let fallback_work = fallback_monitor.as_ref().map(|monitor| {
        let work = monitor.work_area();
        (work.position.x, work.position.y, work.size.width as i32, work.size.height as i32)
    });
    let (icon_x, icon_y, icon_width, icon_height) =
        compute_tray_anchor_with_fallback(raw_icon_rect, fallback_work)
            .ok_or("Tray monitor unavailable")?;
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
    let placement = plan_tray_menu_placement(
        (icon_x, icon_y, icon_width, icon_height),
        (work.position.x, work.position.y, work.size.width as i32, work.size.height as i32),
        scale,
        height_logical,
        width_logical,
    );
    struct LiveShowOps<'a> {
        window: &'a WebviewWindow,
        app: &'a AppHandle,
    }
    impl<'a> TrayMenuShowOps for LiveShowOps<'a> {
        fn set_size(&mut self, width: u32, height: u32) {
            let _ = self.window.set_size(Size::Physical(PhysicalSize::new(width, height)));
        }
        fn set_position(&mut self, x: i32, y: i32) {
            let _ = self.window.set_position(Position::Physical(PhysicalPosition::new(x, y)));
        }
        fn show(&mut self) {
            let _ = self.window.show();
        }
        fn note_shown(&mut self) {
            note_tray_menu_shown(self.app);
        }
        fn set_focus(&mut self) {
            let _ = self.window.set_focus();
        }
        fn schedule_grace_check(&mut self) {
            schedule_tray_menu_grace_check(self.app);
        }
    }
    let mut show_ops = LiveShowOps { window: &window, app };
    drive_tray_menu_show_sequence(&placement, |action| {
        execute_tray_menu_show_action(&mut show_ops, action);
    });
    Ok(placement.height_logical)
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
    use crate::config::{ConfigManager, SettingsWindowGeometry};

    /// Models `handle_tray_click`: the gate decides, and an `Open` is what bumps
    /// `TRAY_MENU_GENERATION` in `open_tray_menu_window`. Driving the loop here keeps
    /// the assertion deterministic without a live tray or window.
    #[derive(Default)]
    struct ClickHarness {
        gate: TrayMenuClickGate,
        visible: bool,
        opens: usize,
        hides: usize,
    }

    impl ClickHarness {
        fn click_at(&mut self, at: Instant, visible_at_click: bool) -> TrayMenuClickAction {
            let action = self.gate.on_click(at, visible_at_click);
            match action {
                TrayMenuClickAction::Open => {
                    self.opens += 1;
                    self.visible = true;
                }
                TrayMenuClickAction::Hide => {
                    self.hides += 1;
                    self.visible = false;
                }
                TrayMenuClickAction::Ignore => {}
            }
            action
        }
    }

    #[test]
    fn click_debounce_coalesces_a_burst_into_one_toggle() {
        // The debounce window must stay inside the specified 250~300ms band.
        assert!(TRAY_MENU_CLICK_DEBOUNCE >= Duration::from_millis(250));
        assert!(TRAY_MENU_CLICK_DEBOUNCE <= Duration::from_millis(300));

        let start = Instant::now();
        let mut harness = ClickHarness::default();
        // Three clicks inside one window: one Toggle, i.e. one `Open` (and therefore
        // exactly one generation bump) and a single visibility flip.
        assert_eq!(harness.click_at(start, false), TrayMenuClickAction::Open);
        assert_eq!(harness.click_at(start + Duration::from_millis(40), false), TrayMenuClickAction::Ignore);
        assert_eq!(harness.click_at(start + Duration::from_millis(90), false), TrayMenuClickAction::Ignore);
        assert_eq!(harness.opens, 1);
        assert_eq!(harness.hides, 0);
        assert!(harness.visible);

        // Past the window the next click is a fresh intent again.
        let later = start + TRAY_MENU_CLICK_DEBOUNCE;
        assert_eq!(harness.click_at(later, true), TrayMenuClickAction::Hide);
        assert_eq!(harness.opens, 1);
        assert_eq!(harness.hides, 1);
        assert!(!harness.visible);
    }

    #[test]
    fn a_double_click_on_an_expanded_menu_is_a_single_collapse() {
        let start = Instant::now();
        let mut harness = ClickHarness { visible: true, ..ClickHarness::default() };
        // Already visible: the first click collapses it, the second is swallowed, so
        // the popup can never be re-opened by the trailing click of a double click.
        assert_eq!(harness.click_at(start, true), TrayMenuClickAction::Hide);
        assert_eq!(harness.click_at(start + Duration::from_millis(120), true), TrayMenuClickAction::Ignore);
        assert_eq!(harness.opens, 0);
        assert_eq!(harness.hides, 1);
        assert!(!harness.visible);
        // Generation delta this round is opens == 0 (<= 1).
        assert!(harness.opens <= 1);
    }

    /// P3-1: the guard is `pending_blur == true && !is_focused`.
    #[test]
    fn grace_period_hides_only_after_an_observed_blur_and_a_lost_focus() {
        let shown = Instant::now();

        // ① Blur inside the grace window and still unfocused on expiry -> hide.
        let mut focus = TrayMenuFocusState::default();
        focus.on_shown(shown);
        assert_eq!(focus.on_blur(shown + Duration::from_millis(30)), TrayMenuFocusAction::Keep);
        assert!(focus.pending_blur);
        assert_eq!(focus.on_grace_expired(false), TrayMenuFocusAction::Hide);
        // The expiry clears the timing state either way.
        assert!(!focus.pending_blur);
        assert_eq!(focus.show_timestamp, None);

        // ② Blur inside the grace window but focused again by expiry -> stay visible.
        let mut focus = TrayMenuFocusState::default();
        focus.on_shown(shown);
        focus.on_blur(shown + Duration::from_millis(30));
        assert_eq!(focus.on_grace_expired(true), TrayMenuFocusAction::Keep);
        assert!(!focus.pending_blur);

        // ③ Core guard: no blur was ever observed, and Windows never granted the
        // foreground so `is_focused()` is false. The popup must stay visible —
        // dropping the `pending_blur` half of the guard turns this assertion red.
        let mut focus = TrayMenuFocusState::default();
        focus.on_shown(shown);
        assert!(!focus.pending_blur);
        assert_eq!(focus.on_grace_expired(false), TrayMenuFocusAction::Keep);

        // ④ A blur after the window has closed is an ordinary dismissal.
        let mut focus = TrayMenuFocusState::default();
        focus.on_shown(shown);
        assert_eq!(focus.on_blur(shown + TRAY_MENU_FOCUS_GRACE), TrayMenuFocusAction::Hide);

        // ⑤ A popup that was never shown has no grace window to protect.
        let mut focus = TrayMenuFocusState::default();
        assert_eq!(focus.on_blur(shown), TrayMenuFocusAction::Hide);

        // The grace period must stay inside the specified 150~200ms band.
        assert!(TRAY_MENU_FOCUS_GRACE >= Duration::from_millis(150));
        assert!(TRAY_MENU_FOCUS_GRACE <= Duration::from_millis(200));
    }

    /// The state the window-event closure and the grace timer share must be the one
    /// `AppState::for_test` hands out, so the guard above governs the real path.
    #[test]
    fn app_state_carries_the_shared_tray_menu_focus_state() {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-tray-focus-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let state = Arc::new(crate::commands::AppState::for_test(
            ConfigManager::with_runtime_dir(dir.clone()),
        ));

        let shown = Instant::now();
        {
            let mut focus = state.tray_menu_focus.lock().expect("focus state");
            focus.on_shown(shown);
            focus.on_blur(shown + Duration::from_millis(20));
        }
        let mut focus = state.tray_menu_focus.lock().expect("focus state");
        assert!(focus.pending_blur, "the blur must have been remembered, not acted on");
        assert_eq!(focus.on_grace_expired(false), TrayMenuFocusAction::Hide);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_tray_icon_rect_degrades_to_the_fallback_corner() {
        let work = (0, 0, 1920, 1040);
        // `tray.rect()` failing must not abort the layout: fall back to the corner.
        assert_eq!(tray_icon_anchor(None, Some(work)), Some((1920, 1040, 0, 0)));
        // A readable rect wins, whether or not a fallback work area exists.
        assert_eq!(tray_icon_anchor(Some((10, 20, 24, 24)), Some(work)), Some((10, 20, 24, 24)));
        assert_eq!(tray_icon_anchor(Some((10, 20, 24, 24)), None), Some((10, 20, 24, 24)));
        // Nothing to place against at all: an error, never a panic.
        assert_eq!(tray_icon_anchor(None, None), None);
    }

    /// The offline half of "`rect() == None` must still show the popup": the
    /// placement itself is computed from the fallback anchor and stays on screen.
    #[test]
    fn a_popup_without_a_tray_icon_rect_still_lands_inside_the_work_area() {
        let work = (0, 0, 1920, 1040);
        let anchor = tray_icon_anchor(None, Some(work)).expect("the fallback corner must resolve");
        let placement = plan_tray_menu_placement(anchor, work, 1.0, 300.0, 380.0);
        assert_eq!(placement.width, 380, "the requested width must survive the fallback");
        assert!(placement.x >= 0 && placement.y >= 0);
        assert!(placement.x + placement.width as i32 <= 1920);
        assert!(placement.y + placement.height as i32 <= 1040);
    }

    #[test]
    fn the_popup_width_uses_the_shared_logical_bounds() {
        let work = (0, 0, 1920, 1040);
        let icon = (1900, 1040, 24, 24);
        // The backend floor matches the frontend contract (`TrayMenuView.tsx`).
        assert_eq!(plan_tray_menu_placement(icon, work, 1.0, 300.0, 100.0).width, 280);
        assert_eq!(plan_tray_menu_placement(icon, work, 1.0, 300.0, 280.0).width, 280);
        assert_eq!(plan_tray_menu_placement(icon, work, 1.0, 300.0, 900.0).width, 500);
        // The reported logical height divides the clamped physical height back out.
        let scaled = plan_tray_menu_placement(icon, work, 2.0, 300.0, 300.0);
        assert!((scaled.height_logical - 300.0).abs() < 0.001, "was {}", scaled.height_logical);
    }

    /// Primary display 1920x1080 @100% at the origin, secondary 2560x1440 @150%
    /// placed to its right — the mixed-DPI layout from the review handoff.
    fn mixed_dpi_layout() -> [MonitorWorkArea; 2] {
        [
            MonitorWorkArea { left: 0.0, top: 0.0, width: 1920.0, height: 1080.0, scale_factor: 1.0 },
            MonitorWorkArea { left: 1920.0, top: 0.0, width: 2560.0, height: 1440.0, scale_factor: 1.5 },
        ]
    }

    /// Window lived on the secondary display: physical (2000, 200), inner size
    /// 1200x990, i.e. logical (1333.33, 133.33, 800, 660) at scale 1.5 — comfortably
    /// above `SETTINGS_MIN_WIDTH` so the restore is not clamped.
    fn geometry_on_secondary() -> SettingsWindowGeometry {
        SettingsWindowGeometry {
            x: 1333.33,
            y: 133.33,
            width: 800.0,
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
        assert!((placement.width - 1200.0).abs() < 0.5, "width was {}", placement.width);
        assert!((placement.height - 990.0).abs() < 0.5, "height was {}", placement.height);
    }

    /// A record saved before the settings window grew must not be restored narrower
    /// than the new floor: the two-column layout needs `SETTINGS_MIN_WIDTH`.
    #[test]
    fn a_legacy_narrow_geometry_is_widened_to_the_new_minimum() {
        let areas = mixed_dpi_layout();
        let narrow = SettingsWindowGeometry {
            x: 1333.33,
            y: 133.33,
            width: 480.0,
            height: 660.0,
            scale_factor: Some(1.5),
        };
        let placement = plan_settings_geometry(&areas, &narrow).expect("placement");
        assert!(
            (placement.width - SETTINGS_MIN_WIDTH * 1.5).abs() < 0.5,
            "width was {}",
            placement.width
        );
    }

    #[test]
    fn overlap_area_beats_the_historical_false_positive() {
        let areas = mixed_dpi_layout();
        let geometry = geometry_on_secondary();

        // Reproduce the old bug: projecting with the primary display's own 100%
        // scale puts the window at [1333..2133]x[133..793], overlapping the primary
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

    #[test]
    fn tray_click_effect_dispatches_hide_and_never_reopens() {
        assert_eq!(plan_tray_click_effect(TrayMenuClickAction::Open), TrayMenuWindowEffect::Open);
        assert_eq!(plan_tray_click_effect(TrayMenuClickAction::Hide), TrayMenuWindowEffect::Hide);
        assert_eq!(plan_tray_click_effect(TrayMenuClickAction::Ignore), TrayMenuWindowEffect::None);
    }

    #[test]
    fn tray_icon_anchor_box_degrades_when_tray_rect_fails() {
        let fallback_work = Some((0, 0, 1920, 1080));
        let error_res: Result<Option<(i32, i32, i32, i32)>, String> = Err("overflow flyout".into());
        let anchor = resolve_tray_icon_anchor_box(error_res, fallback_work);
        assert!(anchor.is_some(), "tray.rect() error must not short-circuit anchor resolution");
        assert_eq!(anchor, Some((1920, 1080, 0, 0)));

        let none_res: Result<Option<(i32, i32, i32, i32)>, String> = Ok(None);
        let anchor_none = resolve_tray_icon_anchor_box(none_res, fallback_work);
        assert_eq!(anchor_none, Some((1920, 1080, 0, 0)));

        let success_res: Result<Option<(i32, i32, i32, i32)>, String> = Ok(Some((100, 200, 24, 24)));
        let anchor_success = resolve_tray_icon_anchor_box(success_res, fallback_work);
        assert_eq!(anchor_success, Some((100, 200, 24, 24)));
    }

    #[test]
    fn show_sequence_emits_note_shown_and_grace_check_in_exact_order() {
        let placement = TrayMenuPlacement {
            x: 100,
            y: 200,
            width: 300,
            height: 400,
            height_logical: 400.0,
        };
        let mut actions = Vec::new();
        drive_tray_menu_show_sequence(&placement, |a| actions.push(a));
        assert_eq!(
            actions,
            vec![
                TrayMenuShowAction::SetSize { width: 300, height: 400 },
                TrayMenuShowAction::SetPosition { x: 100, y: 200 },
                TrayMenuShowAction::Show,
                TrayMenuShowAction::NoteShown,
                TrayMenuShowAction::SetFocus,
                TrayMenuShowAction::ScheduleGraceCheck,
            ]
        );
    }

    #[test]
    fn blur_decision_inside_grace_period_defers_and_outside_hides() {
        let mut focus = TrayMenuFocusState::default();
        let shown_at = Instant::now();
        focus.on_shown(shown_at);

        // Inside grace window (100ms < 180ms): MUST defer, should_hide == false
        let inside = shown_at + Duration::from_millis(100);
        assert_eq!(should_hide_on_tray_menu_blur(&mut focus, inside), false);

        // Outside grace window (250ms > 180ms): MUST hide immediately
        let outside = shown_at + Duration::from_millis(250);
        assert_eq!(should_hide_on_tray_menu_blur(&mut focus, outside), true);
    }

    #[test]
    fn execute_tray_click_effect_dispatches_exact_window_ops() {
        struct MockOps(Vec<&'static str>);
        impl TrayClickWindowOps for MockOps {
            fn open_menu(&mut self) { self.0.push("open"); }
            fn hide_menu(&mut self) { self.0.push("hide"); }
        }
        let mut ops = MockOps(Vec::new());
        execute_tray_click_effect(&mut ops, TrayMenuWindowEffect::Hide);
        assert_eq!(ops.0, vec!["hide"]);

        let mut ops2 = MockOps(Vec::new());
        execute_tray_click_effect(&mut ops2, TrayMenuWindowEffect::Open);
        assert_eq!(ops2.0, vec!["open"]);

        let mut ops3 = MockOps(Vec::new());
        execute_tray_click_effect(&mut ops3, TrayMenuWindowEffect::None);
        assert!(ops3.0.is_empty());
    }

    #[test]
    fn compute_tray_anchor_with_fallback_survives_tray_rect_error() {
        let fallback_work = Some((0, 0, 1920, 1080));
        let err_res: Result<Option<(i32, i32, i32, i32)>, String> = Err("overflow flyout".into());
        let anchor = compute_tray_anchor_with_fallback(err_res, fallback_work);
        assert_eq!(anchor, Some((1920, 1080, 0, 0)));
    }

    #[test]
    fn execute_tray_menu_show_action_delegates_each_variant() {
        struct MockShowOps(Vec<&'static str>);
        impl TrayMenuShowOps for MockShowOps {
            fn set_size(&mut self, _w: u32, _h: u32) { self.0.push("size"); }
            fn set_position(&mut self, _x: i32, _y: i32) { self.0.push("pos"); }
            fn show(&mut self) { self.0.push("show"); }
            fn note_shown(&mut self) { self.0.push("note_shown"); }
            fn set_focus(&mut self) { self.0.push("focus"); }
            fn schedule_grace_check(&mut self) { self.0.push("grace_check"); }
        }
        let mut ops = MockShowOps(Vec::new());
        execute_tray_menu_show_action(&mut ops, TrayMenuShowAction::NoteShown);
        execute_tray_menu_show_action(&mut ops, TrayMenuShowAction::ScheduleGraceCheck);
        assert_eq!(ops.0, vec!["note_shown", "grace_check"]);
    }
}
