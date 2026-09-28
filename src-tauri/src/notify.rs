use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{async_runtime::JoinHandle, AppHandle, Emitter};

use crate::codex::CodexUsage;
use crate::commands::AppState;
use crate::config::{ConfigManager, SoundMode};
use crate::tray::{update_tray_icon, update_tray_tooltip};

const POST_RESET_GRACE_SECS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuotaKind {
    FiveHour,
    Week,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaCycleState {
    pub last_seen_resets_at: Option<i64>,
    pub last_handled_boundary: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ResetStateFile {
    pub five_hour: QuotaCycleState,
    pub weekly: QuotaCycleState,
    pub last_notify_error: Option<String>,
}

impl ResetStateFile {
    fn slot(&self, kind: QuotaKind) -> &QuotaCycleState {
        match kind {
            QuotaKind::FiveHour => &self.five_hour,
            QuotaKind::Week => &self.weekly,
        }
    }

    fn slot_mut(&mut self, kind: QuotaKind) -> &mut QuotaCycleState {
        match kind {
            QuotaKind::FiveHour => &mut self.five_hour,
            QuotaKind::Week => &mut self.weekly,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationStatus {
    Enabled,
    Disabled,
    Unavailable,
}

struct PendingFetch {
    boundary: i64,
    task: JoinHandle<()>,
}

#[derive(Default)]
pub struct PendingResetFetches {
    five_hour: Option<PendingFetch>,
    weekly: Option<PendingFetch>,
}

impl PendingResetFetches {
    fn slot_mut(&mut self, kind: QuotaKind) -> &mut Option<PendingFetch> {
        match kind {
            QuotaKind::FiveHour => &mut self.five_hour,
            QuotaKind::Week => &mut self.weekly,
        }
    }
}

pub fn load_reset_state(config: &ConfigManager) -> ResetStateFile {
    fs::read_to_string(config.reset_state_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_reset_state(config: &ConfigManager, state: &ResetStateFile) -> Result<(), String> {
    let path = config.reset_state_path();
    let tmp = path.with_extension("json.tmp");
    let serialized = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&tmp, serialized).map_err(|error| error.to_string())?;
    fs::rename(&tmp, &path).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CycleTransition {
    changed: bool,
    notify_boundary: Option<i64>,
}

fn advance_cycle(
    cycle: &mut QuotaCycleState,
    new_resets_at: Option<i64>,
    now: i64,
) -> CycleTransition {
    let Some(new_resets_at) = new_resets_at.filter(|timestamp| *timestamp > 0) else {
        return CycleTransition {
            changed: false,
            notify_boundary: None,
        };
    };

    let Some(old_resets_at) = cycle.last_seen_resets_at else {
        cycle.last_seen_resets_at = Some(new_resets_at);
        return CycleTransition {
            changed: true,
            notify_boundary: None,
        };
    };

    // Ignore timestamp regressions. A later valid response can still advance from
    // the last trustworthy boundary without turning clock/server drift into a reset.
    if new_resets_at <= old_resets_at {
        return CycleTransition {
            changed: false,
            notify_boundary: None,
        };
    }

    cycle.last_seen_resets_at = Some(new_resets_at);
    let crossed = old_resets_at <= now;
    let already_handled = cycle.last_handled_boundary == Some(old_resets_at);
    if crossed {
        cycle.last_handled_boundary = Some(old_resets_at);
    }

    CycleTransition {
        changed: true,
        notify_boundary: (crossed && !already_handled).then_some(old_resets_at),
    }
}

pub async fn process_usage_success(app: &AppHandle, state: &Arc<AppState>, usage: &CodexUsage) {
    *state.last_usage.lock().await = Some(usage.clone());
    let settings = state.settings.lock().await.clone();
    let five = usage
        .five_hour_remaining_percent
        .map(|percent| percent.to_string())
        .unwrap_or_else(|| "--".to_string());
    let week = usage
        .week_remaining_percent
        .map(|percent| percent.to_string())
        .unwrap_or_else(|| "--".to_string());
    let tooltip = if settings.show_credits {
        format!("5H {five}% | WK {week}% | CR {}", usage.credits_display)
    } else {
        format!("5H {five}% | WK {week}%")
    };
    update_tray_tooltip(app, &tooltip);
    update_tray_icon(
        app,
        usage
            .five_hour_remaining_percent
            .map(|percent| percent as f64),
        usage.week_remaining_percent.map(|percent| percent as f64),
    );
    let _ = app.emit("usage_updated", usage);
    crate::dock::keep_docked_in_work_area(app, state).await;

    if let Err(error) = maybe_notify_cycle(
        app,
        state,
        QuotaKind::FiveHour,
        usage.five_hour_resets_at,
        settings.five_hour_reset_notification,
        settings.five_hour_sound_mode,
        settings.five_hour_sound_path.clone(),
        &settings.language,
    )
    .await
    {
        record_notify_error(app, state, QuotaKind::FiveHour, error).await;
    }
    if let Err(error) = maybe_notify_cycle(
        app,
        state,
        QuotaKind::Week,
        usage.week_resets_at,
        settings.weekly_reset_notification,
        settings.weekly_sound_mode,
        settings.weekly_sound_path.clone(),
        &settings.language,
    )
    .await
    {
        record_notify_error(app, state, QuotaKind::Week, error).await;
    }
}

async fn maybe_notify_cycle(
    app: &AppHandle,
    state: &Arc<AppState>,
    kind: QuotaKind,
    new_resets_at: Option<i64>,
    enabled: bool,
    sound_mode: SoundMode,
    sound_path: Option<String>,
    language: &str,
) -> Result<(), String> {
    let Some(new_resets_at) = new_resets_at.filter(|timestamp| *timestamp > 0) else {
        return Ok(());
    };
    let now = unix_now();
    let mut current = state.reset_state.lock().await;
    let mut next = current.clone();
    let transition = advance_cycle(next.slot_mut(kind), Some(new_resets_at), now);
    if transition.changed {
        save_reset_state(&state.config_manager, &next)?;
        *current = next;
    }
    // Only the boundary accepted into (and persisted with) the cycle state may
    // arm the post-reset timer. Feeding the raw response here would let a stale
    // or clock-regressed timestamp abort the timer of the current cycle.
    let accepted_boundary = current.slot(kind).last_seen_resets_at;
    drop(current);

    schedule_post_reset_fetch(app, state, kind, accepted_boundary).await;
    if enabled {
        if let Some(boundary) = transition.notify_boundary {
            show_quota_reset(app, state, kind, sound_mode, sound_path, language)?;
            let mut reset_state = state.reset_state.lock().await;
            reset_state.last_notify_error = None;
            let _ = save_reset_state(&state.config_manager, &reset_state);
            let _ = app.emit("quota_reset", QuotaResetEvent { kind, boundary });
        }
    }
    Ok(())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaResetEvent {
    kind: QuotaKind,
    boundary: i64,
}

async fn record_notify_error(
    app: &AppHandle,
    state: &Arc<AppState>,
    kind: QuotaKind,
    error: String,
) {
    eprintln!("Could not notify for {kind:?} quota reset: {error}");
    let mut reset_state = state.reset_state.lock().await;
    reset_state.last_notify_error = Some(error.clone());
    let _ = save_reset_state(&state.config_manager, &reset_state);
    let _ = app.emit(
        "quota_reset_error",
        serde_json::json!({ "kind": kind, "error": error }),
    );
}

fn show_quota_reset(
    app: &AppHandle,
    state: &Arc<AppState>,
    kind: QuotaKind,
    sound_mode: SoundMode,
    sound_path: Option<String>,
    language: &str,
) -> Result<(), String> {
    let (title, body) = notification_copy(kind, language);
    match sound_mode {
        SoundMode::Windows => show_default_notification(app, title, body),
        SoundMode::Custom => {
            #[cfg(windows)]
            if let (Some(path), Some(audio)) = (sound_path.filter(|path| !path.is_empty()), state.audio.get()) {
                let toast_result = show_toast(title, body, None, &app.config().identifier);
                audio.enqueue_notification(path.into(), kind);
                return toast_result;
            }
            #[cfg(not(windows))]
            let _ = (state, sound_path);
            // No selected file or an unavailable worker: keep an audible Windows
            // notification instead of silently suppressing every sound.
            show_default_notification(app, title, body)
        }
    }
}

fn show_default_notification(app: &AppHandle, title: &str, body: &str) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| error.to_string())
}

fn notification_copy(kind: QuotaKind, language: &str) -> (&'static str, &'static str) {
    let locale = crate::commands::resolve_locale(language);
    match (kind, locale) {
        (_, "zh-CN") => (
            "额度已重置",
            if kind == QuotaKind::FiveHour {
                "5 小时额度已完成重置。"
            } else {
                "每周额度已完成重置。"
            },
        ),
        (_, "zh-Hant") => (
            "額度已重置",
            if kind == QuotaKind::FiveHour {
                "5 小時額度已完成重置。"
            } else {
                "每週額度已完成重置。"
            },
        ),
        (QuotaKind::FiveHour, _) => ("Quota reset", "Your 5-hour quota has reset."),
        (QuotaKind::Week, _) => ("Quota reset", "Your weekly quota has reset."),
    }
}

#[cfg(windows)]
fn show_toast(
    title: &str,
    body: &str,
    sound: Option<tauri_winrt_notification::Sound>,
    app_id: &str,
) -> Result<(), String> {
    let title = title.to_string();
    let body = body.to_string();
    let app_id = app_id.to_string();
    std::thread::Builder::new()
        .name("quota-toast".into())
        .spawn(move || {
            use windows::Win32::System::WinRT::{
                RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
            };
            let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|error| error.to_string())?;
            let result = tauri_winrt_notification::Toast::new(&app_id)
                .title(&title)
                .text1(&body)
                .sound(sound)
                .show()
                .map_err(|error| error.to_string());
            unsafe { RoUninitialize() };
            let _ = initialized;
            result
        })
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| "Notification thread failed".to_string())?
}

#[cfg(not(windows))]
fn show_toast(_title: &str, _body: &str, _sound: Option<()>, _app_id: &str) -> Result<(), String> {
    Err("Windows notifications are unavailable on this platform".into())
}

fn notification_status_impl(app_id: &str) -> NotificationStatus {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
        use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
        let app_id = app_id.to_string();

        return std::thread::Builder::new()
            .name("notification-status".into())
            .spawn(move || {
                if unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_err() {
                    return NotificationStatus::Unavailable;
                }
                let status =
                    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))
                        .and_then(|notifier| notifier.Setting())
                        .map(|setting| {
                            if setting == NotificationSetting::Enabled {
                                NotificationStatus::Enabled
                            } else {
                                NotificationStatus::Disabled
                            }
                        })
                        .unwrap_or(NotificationStatus::Unavailable);
                unsafe { RoUninitialize() };
                status
            })
            .ok()
            .and_then(|task| task.join().ok())
            .unwrap_or(NotificationStatus::Unavailable);
    }
    #[cfg(not(windows))]
    {
        let _ = app_id;
        NotificationStatus::Unavailable
    }
}

/// Boundary the post-reset timer should be armed for: `Some(boundary)` to
/// (re)schedule, `None` to leave any existing timer untouched. `accepted` must
/// be a boundary already validated and stored by `advance_cycle`; a stale or
/// clock-regressed response never reaches this point, so it can no longer abort
/// the timer of the current cycle.
fn post_reset_fetch_target(existing: Option<i64>, accepted: Option<i64>, now: i64) -> Option<i64> {
    let boundary = accepted?;
    if boundary.saturating_add(POST_RESET_GRACE_SECS) <= now {
        return None;
    }
    if existing == Some(boundary) {
        return None;
    }
    Some(boundary)
}

async fn schedule_post_reset_fetch(
    app: &AppHandle,
    state: &Arc<AppState>,
    kind: QuotaKind,
    accepted_boundary: Option<i64>,
) {
    let now = unix_now();
    let mut pending = state.pending_reset_fetches.lock().await;
    let slot = pending.slot_mut(kind);
    let existing = slot.as_ref().map(|current| current.boundary);
    let Some(boundary) = post_reset_fetch_target(existing, accepted_boundary, now) else {
        return;
    };
    if let Some(previous) = slot.take() {
        previous.task.abort();
    }

    let app = app.clone();
    let state = state.clone();
    let deadline = boundary.saturating_add(POST_RESET_GRACE_SECS);
    let wait = Duration::from_secs((deadline - now) as u64);
    let task = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(wait).await;
        {
            let mut pending = state.pending_reset_fetches.lock().await;
            let slot = pending.slot_mut(kind);
            if !slot
                .as_ref()
                .is_some_and(|current| current.boundary == boundary)
            {
                return;
            }
            *slot = None;
        }
        let _ = app.emit("trigger_refresh", ());
    });
    *slot = Some(PendingFetch { boundary, task });
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[tauri::command]
pub fn get_notification_status(app: AppHandle) -> NotificationStatus {
    notification_status_impl(&app.config().identifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_observation_only_establishes_baseline() {
        let mut cycle = QuotaCycleState::default();
        let transition = advance_cycle(&mut cycle, Some(1_000), 2_000);
        assert_eq!(
            transition,
            CycleTransition {
                changed: true,
                notify_boundary: None
            }
        );
        assert_eq!(cycle.last_seen_resets_at, Some(1_000));
        assert_eq!(cycle.last_handled_boundary, None);
    }

    #[test]
    fn crossing_an_expired_boundary_notifies_and_marks_it_once() {
        let mut cycle = QuotaCycleState {
            last_seen_resets_at: Some(1_000),
            last_handled_boundary: None,
        };
        let first = advance_cycle(&mut cycle, Some(2_000), 1_000);
        let repeated = advance_cycle(&mut cycle, Some(2_000), 2_100);
        assert_eq!(first.notify_boundary, Some(1_000));
        assert_eq!(cycle.last_handled_boundary, Some(1_000));
        assert_eq!(repeated.notify_boundary, None);
    }

    #[test]
    fn missing_future_and_regressed_timestamps_do_not_notify() {
        let mut cycle = QuotaCycleState {
            last_seen_resets_at: Some(2_000),
            last_handled_boundary: None,
        };
        assert_eq!(advance_cycle(&mut cycle, None, 3_000).notify_boundary, None);
        assert_eq!(
            advance_cycle(&mut cycle, Some(0), 3_000).notify_boundary,
            None
        );
        let regressed = advance_cycle(&mut cycle, Some(1_000), 3_000);
        assert!(!regressed.changed);
        assert_eq!(cycle.last_seen_resets_at, Some(2_000));
    }

    #[test]
    fn a_future_boundary_shift_does_not_count_as_a_reset() {
        let mut cycle = QuotaCycleState {
            last_seen_resets_at: Some(3_000),
            last_handled_boundary: None,
        };
        let transition = advance_cycle(&mut cycle, Some(4_000), 2_000);
        assert_eq!(transition.notify_boundary, None);
        assert_eq!(cycle.last_seen_resets_at, Some(4_000));
    }

    #[test]
    fn a_stale_regressed_response_does_not_abort_the_scheduled_future_fetch() {
        let now = 1_000_000;
        let future = now + 600;
        let mut cycle = QuotaCycleState::default();

        // A valid observation arms the post-reset timer for the future boundary.
        assert!(advance_cycle(&mut cycle, Some(future), now).changed);
        assert_eq!(
            post_reset_fetch_target(None, cycle.last_seen_resets_at, now),
            Some(future)
        );

        // A stale response regresses the timestamp; advance_cycle rejects it and
        // the accepted boundary (and therefore the pending timer) is preserved.
        let regressed = now - 600;
        let transition = advance_cycle(&mut cycle, Some(regressed), now);
        assert!(!transition.changed);
        assert_eq!(cycle.last_seen_resets_at, Some(future));
        assert_eq!(
            post_reset_fetch_target(Some(future), cycle.last_seen_resets_at, now),
            None
        );

        // Feeding the raw expired value would have cleared the timer under the
        // old logic; the planner now leaves the armed timer untouched instead.
        assert_eq!(
            post_reset_fetch_target(Some(future), Some(regressed), now),
            None
        );

        // A genuinely newer accepted boundary replaces the armed timer.
        assert_eq!(
            post_reset_fetch_target(Some(future), Some(future + 600), now),
            Some(future + 600)
        );
    }

    #[test]
    fn persisted_state_keeps_quota_slots_separate() {
        let mut state = ResetStateFile::default();
        state.five_hour.last_seen_resets_at = Some(1_000);
        state.weekly.last_seen_resets_at = Some(8_000);
        let restored: ResetStateFile =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(restored.five_hour.last_seen_resets_at, Some(1_000));
        assert_eq!(restored.weekly.last_seen_resets_at, Some(8_000));
    }
}
