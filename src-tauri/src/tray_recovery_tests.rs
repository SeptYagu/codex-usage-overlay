use super::*;

struct Shell {
    missing: bool,
    repair_fails: bool,
    repairs: usize,
    tooltip_updates: usize,
    calls: Vec<&'static str>,
}

impl TrayUpdateOps for Shell {
    fn set_icon(&mut self) -> Result<(), String> {
        self.calls.push("icon");
        if self.missing { Err("shell unavailable".into()) } else { Ok(()) }
    }

    fn set_tooltip(&mut self) -> Result<(), String> {
        self.calls.push("tooltip");
        self.tooltip_updates += 1;
        Ok(())
    }

    fn recover_in_place(&mut self) -> Result<(), String> {
        self.calls.push("recover");
        self.repairs += 1;
        if self.repair_fails { Err("taskbar unavailable".into()) } else {
            self.missing = false;
            Ok(())
        }
    }
}

fn shell(missing: bool, repair_fails: bool) -> Shell {
    Shell { missing, repair_fails, repairs: 0, tooltip_updates: 0, calls: Vec::new() }
}

#[test]
fn missing_icon_is_recovered_before_the_latest_frame_and_tooltip_are_written() {
    let mut ops = shell(true, false);
    repaint_existing_tray(&mut ops, &mut None, Instant::now()).unwrap();
    assert_eq!(ops.calls, ["icon", "recover", "icon", "tooltip"]);
    assert!(!ops.missing);
}

#[test]
fn long_failure_storm_is_throttled_and_recovers_when_the_shell_returns() {
    let mut ops = shell(true, true);
    let mut last = None;
    let start = Instant::now();
    for second in 0..10_000 {
        let result = repaint_existing_tray(&mut ops, &mut last, start + Duration::from_secs(second));
        assert_eq!(result.is_err(), second % 30 == 0);
    }
    assert_eq!(ops.repairs, 334);
    assert_eq!(ops.tooltip_updates, 0);
    ops.repair_fails = false;
    repaint_existing_tray(&mut ops, &mut last, start + Duration::from_secs(10_020)).unwrap();
    assert_eq!(ops.repairs, 335);
    assert_eq!(ops.tooltip_updates, 1);
    repaint_existing_tray(&mut ops, &mut last, start + Duration::from_secs(10_021)).unwrap();
    assert_eq!(ops.repairs, 335);
    assert_eq!(ops.tooltip_updates, 2);
}

#[test]
fn recovery_retries_at_the_boundary_without_blocking_healthy_updates() {
    let mut ops = shell(true, false);
    let start = Instant::now();
    let mut last = Some(start);
    repaint_existing_tray(&mut ops, &mut last, start + Duration::from_millis(29_999)).unwrap();
    assert_eq!(ops.calls, ["icon"]);
    repaint_existing_tray(&mut ops, &mut last, start + Duration::from_secs(30)).unwrap();
    assert_eq!(ops.repairs, 1);
    repaint_existing_tray(&mut ops, &mut last, start + Duration::from_secs(31)).unwrap();
    assert_eq!(ops.repairs, 1);
    assert_eq!(ops.tooltip_updates, 2);
}

#[cfg(windows)]
#[test]
#[ignore = "Creates a temporary real tray icon; run on an interactive Windows desktop"]
fn native_recovery_keeps_one_window_and_releases_it_on_the_owner_thread() {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowThreadProcessId};

    unsafe extern "system" fn count_window(hwnd: HWND, data: LPARAM) -> BOOL {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); }
        if pid == std::process::id() {
            let mut name = [0u16; 64];
            let len = unsafe { GetClassNameW(hwnd, &mut name) } as usize;
            if String::from_utf16_lossy(&name[..len]) == "tray_icon_app" {
                unsafe { *(data.0 as *mut usize) += 1; }
            }
        }
        BOOL(1)
    }
    fn count() -> usize {
        let mut value = 0usize;
        unsafe { EnumWindows(Some(count_window), LPARAM(&mut value as *mut usize as isize)).unwrap(); }
        value
    }

    // Force the initial update to fail while exercising the real recovery and
    // retry operations against Explorer, rather than merely testing a model.
    struct MissingIcon<T>(T, bool);
    impl<T: TrayUpdateOps> TrayUpdateOps for MissingIcon<T> {
        fn set_icon(&mut self) -> Result<(), String> {
            if std::mem::take(&mut self.1) { Err("injected missing shell icon".into()) }
            else { self.0.set_icon() }
        }
        fn set_tooltip(&mut self) -> Result<(), String> { self.0.set_tooltip() }
        fn recover_in_place(&mut self) -> Result<(), String> { self.0.recover_in_place() }
    }

    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = "com.codex.usage.overlay.tray-recovery-test".into();
    let baseline = count();
    let app = tauri::Builder::default().any_thread().build(context).unwrap();
    let presentation = || tray_presentation(&OverlaySettings::default(), None, TrayReadStatus::Pending);
    let tray = app.handle().create_tray(presentation()).unwrap();
    let original = tray.with_inner_tray_icon(|inner| inner.window_handle() as usize).unwrap();
    assert_eq!(count(), baseline + 1);
    for _ in 0..100 {
        let frame = presentation();
        let native = NativeTrayUpdate { tray: &tray, presentation: &frame };
        repaint_existing_tray(&mut MissingIcon(native, true), &mut None, Instant::now()).unwrap();
        assert_eq!(tray.with_inner_tray_icon(|inner| inner.window_handle() as usize).unwrap(), original);
        assert_eq!(count(), baseline + 1, "recovery leaked a native tray window");
    }
    drop(tray);
    drop(app.handle().remove_tray_by_id(TRAY_ID));
    assert_eq!(count(), baseline, "owner-thread teardown leaked the tray window");
}
