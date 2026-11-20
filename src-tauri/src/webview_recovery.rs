//! Recovers the WebView2 behind each window when one of its processes dies.
//!
//! The overlay is a transparent window: when its WebView2 content is gone the
//! window is still visible and topmost but draws nothing, and no tray action
//! brings the content back. WebView2 reports the failure through
//! `ProcessFailed`:
//!
//! - a render process that exits or stops responding is recovered in place
//!   with `Reload`;
//! - a browser process exit closes every webview it hosts (the overlay, the
//!   settings window and the tray menu share one), so the app restarts, which
//!   rebuilds all of them from the persisted state.
#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tauri::{AppHandle, WebviewWindow};

/// A browser-process exit this soon after launch points at a broken WebView2
/// runtime rather than a one-off kill; restarting again would only loop.
const RESTART_MIN_UPTIME: Duration = Duration::from_secs(60);
/// WebView2 repeats `RENDER_PROCESS_UNRESPONSIVE` every few seconds while a
/// renderer hangs, so each window reloads at most this often.
const RELOAD_MIN_INTERVAL: Duration = Duration::from_secs(30);

// `COREWEBVIEW2_PROCESS_FAILED_KIND` values.
const BROWSER_PROCESS_EXITED: i32 = 0;
const RENDER_PROCESS_EXITED: i32 = 1;
const RENDER_PROCESS_UNRESPONSIVE: i32 = 2;

static STARTED: OnceLock<Instant> = OnceLock::new();
static EXITING: AtomicBool = AtomicBool::new(false);
static RESTARTING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recovery {
    Ignore,
    Reload,
    Restart,
}

/// What a `ProcessFailed` of `kind` calls for. GPU, utility and other helper
/// processes are restarted by WebView2 itself and need nothing.
fn recovery_for(
    kind: i32,
    uptime: Duration,
    since_last_reload: Option<Duration>,
    exiting: bool,
) -> Recovery {
    if exiting {
        return Recovery::Ignore;
    }
    match kind {
        BROWSER_PROCESS_EXITED if uptime >= RESTART_MIN_UPTIME => Recovery::Restart,
        RENDER_PROCESS_EXITED | RENDER_PROCESS_UNRESPONSIVE
            if since_last_reload.is_none_or(|elapsed| elapsed >= RELOAD_MIN_INTERVAL) =>
        {
            Recovery::Reload
        }
        _ => Recovery::Ignore,
    }
}

/// Lets exactly one caller request the restart, and none once the app is
/// already shutting down. Every webview of the dead browser process reports
/// the same failure, so the claim deduplicates them.
fn claim_restart(exiting: &AtomicBool, restarting: &AtomicBool) -> bool {
    !exiting.load(Ordering::SeqCst) && !restarting.swap(true, Ordering::SeqCst)
}

/// Marks the app as shutting down. WebView2 processes ending after this are
/// part of the exit, and must never turn an exit into a restart.
pub fn note_exiting() {
    EXITING.store(true, Ordering::SeqCst);
}

/// True while the current shutdown is a recovery restart.
pub fn restarting() -> bool {
    RESTARTING.load(Ordering::SeqCst)
}

fn restart(app: &AppHandle) {
    if claim_restart(&EXITING, &RESTARTING) {
        eprintln!("The WebView2 browser process exited; restarting to rebuild every window");
        // Non-blocking, unlike `restart()`: this runs on the main thread inside
        // a WebView2 callback, and the event loop has to deliver the exit.
        app.request_restart();
    }
}

/// Registers the recovery handler on `window`'s WebView2. Call it once for
/// each window, right after the window is created.
#[cfg(windows)]
pub fn watch(window: &WebviewWindow) {
    use tauri::Manager;
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND;
    use webview2_com::ProcessFailedEventHandler;

    let started = *STARTED.get_or_init(Instant::now);
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let scheduled = window.with_webview(move |webview| {
        let handler_label = label.clone();
        let mut last_reload: Option<Instant> = None;
        let handler = ProcessFailedEventHandler::create(Box::new(move |sender, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            unsafe { args.ProcessFailedKind(&mut kind)? };
            let now = Instant::now();
            let recovery = recovery_for(
                kind.0,
                now.saturating_duration_since(started),
                last_reload.map(|at| now.saturating_duration_since(at)),
                EXITING.load(Ordering::SeqCst),
            );
            match recovery {
                Recovery::Reload => {
                    if let Some(webview) = sender {
                        eprintln!(
                            "WebView2 render process failed for the {handler_label} window (kind {}); reloading",
                            kind.0
                        );
                        last_reload = Some(now);
                        unsafe { webview.Reload()? };
                    }
                }
                Recovery::Restart => restart(&app),
                Recovery::Ignore => {}
            }
            Ok(())
        }));
        let mut token = 0i64;
        let registered = unsafe { webview.controller().CoreWebView2() }
            .and_then(|core| unsafe { core.add_ProcessFailed(&handler, &mut token) });
        if let Err(error) = registered {
            eprintln!("Could not watch the {label} window's WebView2 for process failures: {error}");
        }
    });
    if let Err(error) = scheduled {
        eprintln!("Could not reach a window's WebView2 to watch for process failures: {error}");
    }
}

#[cfg(not(windows))]
pub fn watch(_window: &WebviewWindow) {}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTLED: Duration = Duration::from_secs(600);

    #[test]
    fn a_browser_process_exit_restarts_only_after_the_app_has_settled() {
        assert_eq!(recovery_for(BROWSER_PROCESS_EXITED, SETTLED, None, false), Recovery::Restart);
        assert_eq!(
            recovery_for(BROWSER_PROCESS_EXITED, RESTART_MIN_UPTIME, None, false),
            Recovery::Restart
        );
        assert_eq!(
            recovery_for(BROWSER_PROCESS_EXITED, Duration::from_secs(5), None, false),
            Recovery::Ignore
        );
    }

    #[test]
    fn a_failed_render_process_reloads_at_most_once_per_interval() {
        for kind in [RENDER_PROCESS_EXITED, RENDER_PROCESS_UNRESPONSIVE] {
            assert_eq!(recovery_for(kind, SETTLED, None, false), Recovery::Reload);
            assert_eq!(
                recovery_for(kind, SETTLED, Some(Duration::from_secs(3)), false),
                Recovery::Ignore
            );
            assert_eq!(
                recovery_for(kind, SETTLED, Some(RELOAD_MIN_INTERVAL), false),
                Recovery::Reload
            );
        }
        // A renderer can fail right after launch too; only restarts wait.
        assert_eq!(
            recovery_for(RENDER_PROCESS_EXITED, Duration::ZERO, None, false),
            Recovery::Reload
        );
    }

    #[test]
    fn helper_process_failures_need_no_recovery() {
        // Frame renderer, utility, sandbox helper, GPU, PPAPI and unknown.
        for kind in 3..=9 {
            assert_eq!(recovery_for(kind, SETTLED, None, false), Recovery::Ignore);
        }
    }

    #[test]
    fn nothing_recovers_once_the_app_is_exiting() {
        for kind in 0..=9 {
            assert_eq!(recovery_for(kind, SETTLED, None, true), Recovery::Ignore);
        }
    }

    #[test]
    fn one_restart_is_requested_per_browser_failure_and_none_during_exit() {
        let (exiting, restarting) = (AtomicBool::new(false), AtomicBool::new(false));
        assert!(claim_restart(&exiting, &restarting));
        assert!(!claim_restart(&exiting, &restarting));

        let (exiting, restarting) = (AtomicBool::new(true), AtomicBool::new(false));
        assert!(!claim_restart(&exiting, &restarting));
        assert!(!restarting.load(Ordering::SeqCst));
    }
}
