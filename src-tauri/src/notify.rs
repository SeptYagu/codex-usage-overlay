use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{async_runtime::JoinHandle, AppHandle, Emitter, Manager};

use crate::codex::{CodexUsage, RawCodexUsage};
use crate::commands::{AppState, ProviderRuntime};
use crate::config::{ConfigManager, Provider, SoundMode};
use crate::tray::{TrayPresentationSink, TrayReadStatus};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_observed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub had_usage: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
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
    pub fn cancel_all(&mut self) {
        for pending in [&mut self.five_hour, &mut self.weekly] {
            if let Some(fetch) = pending.take() { fetch.task.abort(); }
        }
    }
    fn cancel_kind(&mut self, kind: QuotaKind) {
        if let Some(fetch) = self.slot_mut(kind).take() { fetch.task.abort(); }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FiveHourTransition {
    changed: bool,
    notify_boundary: Option<i64>,
    cancel_timer: bool,
}

fn advance_five_hour(
    cycle: &mut QuotaCycleState,
    raw: &RawCodexUsage,
    now: i64,
    max_age: i64,
    display_only: bool,
) -> FiveHourTransition {
    let skipped = FiveHourTransition { changed: false, notify_boundary: None, cancel_timer: false };
    let window = raw.notification_five_hour.as_ref().unwrap_or(&raw.five_hour);
    let at = raw.fetched_at;
    let identity_changed = matches!((&cycle.account_id, &raw.account_id), (Some(old), Some(new)) if old != new)
        || (cycle.account_id.is_none() && raw.account_id.is_some() && cycle.last_seen_resets_at.is_some());
    if display_only || at <= 0 || at > now.saturating_add(5) || now.saturating_sub(at) > max_age
        || (!identity_changed && cycle.last_observed_at.is_some_and(|previous| at <= previous)) {
        return skipped;
    }
    if window.resets_at.is_some_and(|boundary| boundary <= 0) {
        return skipped;
    }
    let Some(used) = window.used_percent.filter(|used| used.is_finite() && (0.0..=100.0).contains(used)) else {
        if identity_changed {
            *cycle = QuotaCycleState { account_id: raw.account_id.clone(), last_observed_at: Some(at), ..Default::default() };
            return FiveHourTransition { changed: true, notify_boundary: None, cancel_timer: true };
        }
        return skipped;
    };

    if identity_changed {
        *cycle = QuotaCycleState::default();
    }
    if let Some(account) = &raw.account_id {
        cycle.account_id = Some(account.clone());
    }
    cycle.last_observed_at = Some(at);

    let new_boundary = window.resets_at.filter(|boundary| *boundary > 0);
    let old_boundary = cycle.last_seen_resets_at;
    let mut notify_boundary = None;
    match (old_boundary, new_boundary) {
        (None, Some(new)) => {
            cycle.last_seen_resets_at = Some(new);
            cycle.had_usage = Some(used > 0.0);
        }
        (Some(old), Some(new)) if new > old => {
            // A later deadline can be the first message in a new window. It
            // does not establish that the old window recovered while idle.
            if old <= at { cycle.last_handled_boundary = Some(old); }
            cycle.last_seen_resets_at = Some(new);
            cycle.had_usage = Some(used > 0.0);
        }
        (Some(old), Some(new)) if new == old => {
            if used > 0.0 { cycle.had_usage = Some(true); }
        }
        (Some(old), None) if used == 0.0 && old <= at => {
            // Only a fresh, explicit zero-usage reading with no active reset
            // time confirms recovery. Missing percentages do not reach here.
            if cycle.had_usage == Some(true) && cycle.last_handled_boundary != Some(old) {
                notify_boundary = Some(old);
            }
            cycle.last_handled_boundary = Some(old);
            cycle.had_usage = Some(false);
        }
        _ => {}
    }
    FiveHourTransition { changed: true, notify_boundary, cancel_timer: identity_changed }
}

pub async fn process_usage_success<S: TrayPresentationSink>(sink: &S, state: &Arc<ProviderRuntime>, usage: &CodexUsage, raw: &RawCodexUsage) {
    let settings = {
        let settings = state.settings.lock().await;
        let mut status = state.tray_read_status.lock().await;
        let mut cached = state.last_usage.lock().await;
        *cached = Some(usage.clone());
        if usage.five_hour_remaining_percent.is_some() || usage.week_remaining_percent.is_some() || usage.has_credits {
            state.has_reading.store(true, std::sync::atomic::Ordering::Release);
        }
        *status = TrayReadStatus::Success;
        {
            let _tray = state.tray_gate.lock().unwrap_or_else(|p| p.into_inner());
            if state.is_displayed() { sink.present(crate::tray::runtime_presentation(state,&settings, cached.as_ref(), *status)); }
        }
        settings.clone()
    };
    crate::monitor::publish_runtime(sink, state).await;
    if let Some(app) = sink.app_handle() {
        if state.is_displayed() {
            if let Some(owner) = app.try_state::<Arc<AppState>>() {
                crate::dock::keep_in_work_area(app, owner.inner()).await;
            }
        }

        let pref = settings.preference(state.provider);
        if let Err(error) = maybe_notify_cycle(
            app,
            state,
            QuotaKind::FiveHour,
            usage.five_hour_resets_at,
            raw,
            pref.five_hour_reset_notification,
            pref.five_hour_sound_mode,
            pref.five_hour_sound_path.clone(),
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
            raw,
            pref.weekly_reset_notification,
            pref.weekly_sound_mode,
            pref.weekly_sound_path.clone(),
            &settings.language,
        )
        .await
        {
            record_notify_error(app, state, QuotaKind::Week, error).await;
        }
    }
}

async fn maybe_notify_cycle(
    app: &AppHandle,
    state: &Arc<ProviderRuntime>,
    kind: QuotaKind,
    new_resets_at: Option<i64>,
    raw: &RawCodexUsage,
    enabled: bool,
    sound_mode: SoundMode,
    sound_path: Option<String>,
    language: &str,
) -> Result<(), String> {
    if (kind == QuotaKind::Week || state.provider == Provider::Antigravity)
        && new_resets_at.is_none_or(|boundary| boundary <= 0) {
        return Ok(());
    }
    let now = unix_now();
    let mut current = state.reset_state.lock().await;
    let mut next = current.clone();
    let (changed, notify_boundary, cancel_timer) = if kind == QuotaKind::FiveHour && state.provider != Provider::Antigravity {
        let display_only = state.provider == Provider::Codex
            && state.reader.lock().unwrap_or_else(|p| p.into_inner()).display_only();
        let max_age = if state.provider == Provider::Claude { agent_bridge::SNAPSHOT_FRESH_SECS } else if state.provider == Provider::Dsh { 300 } else { 30 };
        let transition = advance_five_hour(next.slot_mut(kind), raw, now, max_age, display_only);
        (transition.changed, transition.notify_boundary, transition.cancel_timer)
    } else {
        let transition = advance_cycle(next.slot_mut(kind), new_resets_at, now);
        (transition.changed, transition.notify_boundary, false)
    };
    if changed {
        save_reset_state(&state.config_manager, &next)?;
        *current = next;
    }
    // Only the boundary accepted into (and persisted with) the cycle state may
    // arm the post-reset timer. Feeding the raw response here would let a stale
    // or clock-regressed timestamp abort the timer of the current cycle.
    let accepted_boundary = current.slot(kind).last_seen_resets_at;
    drop(current);

    if kind == QuotaKind::FiveHour && state.provider != Provider::Antigravity && !changed {
        return Ok(());
    }
    if cancel_timer {
        state.pending_reset_fetches.lock().await.cancel_kind(kind);
    }
    schedule_post_reset_fetch(app, state, kind, accepted_boundary).await;
    if enabled {
        if let Some(boundary) = notify_boundary {
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
    state: &Arc<ProviderRuntime>,
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
    state: &Arc<ProviderRuntime>,
    kind: QuotaKind,
    sound_mode: SoundMode,
    sound_path: Option<String>,
    language: &str,
) -> Result<(), String> {
    let (base_title, body) = notification_copy(kind, language);
    let provider = match state.provider {
        Provider::Claude => "Claude",
        Provider::Dsh => "DSH",
        Provider::Antigravity => "Antigravity",
        Provider::Codex => "Codex",
    };
    let title = format!("{provider} — {base_title}");
    let title = title.as_str();
    match sound_mode {
        SoundMode::Windows => show_default_notification(app, title, body),
        SoundMode::Custom => {
            #[cfg(windows)]
            if let (Some(path), Some(audio)) = (sound_path.filter(|path| !path.is_empty()), state.audio.get()) {
                let toast_result = show_toast(title, body, None, &app.config().identifier);
                audio.enqueue_notification(path.into(), kind, state.provider);
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

/// Announces a found update from the backend, so a check that finishes right
/// at launch never depends on the overlay page having registered a listener.
pub fn show_update_notification(app: &AppHandle, language: &str, version: &str, ready: bool) {
    let (title, body) = update_notification_copy(language, version, ready);
    if let Err(error) = show_default_notification(app, title, &body) {
        eprintln!("Could not show update notification: {error}");
    }
}

/// `ready` means the update was downloaded and installs when the app quits.
pub(crate) fn update_notification_copy(language: &str, version: &str, ready: bool) -> (&'static str, String) {
    match crate::commands::resolve_locale(language) {
        "zh-CN" => ("Codex 用量悬浮窗更新", if ready {
            format!("{version} 版已下载，将在退出时安装。")
        } else {
            format!("发现新版本 {version}。")
        }),
        "zh-Hant" => ("Codex 用量懸浮窗更新", if ready {
            format!("{version} 版已下載，將在退出時安裝。")
        } else {
            format!("發現新版本 {version}。")
        }),
        "ru-RU" => ("Обновление Codex Usage Overlay", if ready {
            format!("Версия {version} загружена и будет установлена при выходе.")
        } else {
            format!("Доступна версия {version}.")
        }),
        _ => ("Codex Usage Overlay update", if ready {
            format!("Version {version} has been downloaded and installs when you quit.")
        } else {
            format!("Version {version} is available.")
        }),
    }
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
        (QuotaKind::FiveHour, "ru-RU") => ("Лимит обновлён", "Пятичасовой лимит обновлён."),
        (QuotaKind::Week, "ru-RU") => ("Лимит обновлён", "Недельный лимит обновлён."),
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
    _app: &AppHandle,
    state: &Arc<ProviderRuntime>,
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
        if state.monitor_enabled.load(std::sync::atomic::Ordering::Acquire) {
            // This boundary belongs to this provider, even when another is shown.
            state.polling.lock().unwrap_or_else(|p| p.into_inner()).request_now(std::time::Instant::now());
            state.poll_wake.notify_one();
        }
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
    use crate::codex::QuotaSnapshot;

    fn five_hour_sample(at: i64, used: Option<f64>, boundary: Option<i64>, account: &str) -> RawCodexUsage {
        RawCodexUsage {
            account_id: Some(account.into()),
            five_hour: QuotaSnapshot { used_percent: used, resets_at: boundary, window_duration_mins: Some(300) },
            weekly: QuotaSnapshot::default(),
            credits_display: "—".into(), credits_balance: None, has_credits: false,
            fetched_at: at, antigravity_weekly: None, quota_pool: None, notification_five_hour: None,
        }
    }

    #[test]
    fn an_idle_recovery_notifies_once_but_starting_the_next_window_does_not() {
        for first_new_usage in [0.0, 5.0] {
            let mut cycle = QuotaCycleState::default();
            assert_eq!(advance_five_hour(&mut cycle, &five_hour_sample(900, Some(20.0), Some(1_000), "a"), 900, 30, false).notify_boundary, None);
            assert_eq!(advance_five_hour(&mut cycle, &five_hour_sample(1_031, Some(0.0), None, "a"), 1_031, 30, false).notify_boundary, Some(1_000));
            cycle = serde_json::from_slice(&serde_json::to_vec(&cycle).unwrap()).unwrap();
            assert_eq!(advance_five_hour(&mut cycle, &five_hour_sample(1_032, Some(0.0), None, "a"), 1_032, 30, false).notify_boundary, None);
            assert_eq!(advance_five_hour(&mut cycle, &five_hour_sample(1_040, Some(first_new_usage), Some(2_840), "a"), 1_040, 30, false).notify_boundary, None);
            assert_eq!(cycle.last_handled_boundary, Some(1_000));
            assert_eq!(cycle.had_usage, Some(first_new_usage > 0.0));
        }
    }

    #[test]
    fn first_message_after_an_expired_window_is_silent_for_all_three_providers() {
        for provider in [Provider::Codex, Provider::Claude, Provider::Dsh] {
            for new_usage in [0.0, 7.0] {
                let mut cycle = QuotaCycleState::default();
                let max_age = if provider == Provider::Claude { 900 } else if provider == Provider::Dsh { 300 } else { 30 };
                advance_five_hour(&mut cycle, &five_hour_sample(900, Some(20.0), Some(1_000), "a"), 900, max_age, false);
                let next = five_hour_sample(1_200, Some(new_usage), Some(3_000), "a");
                assert_eq!(advance_five_hour(&mut cycle, &next, 1_200, max_age, false).notify_boundary, None, "{provider:?}");
                assert_eq!(cycle.last_seen_resets_at, Some(3_000));
            }
        }
    }

    #[test]
    fn stale_missing_and_projected_claude_readings_cannot_confirm_recovery() {
        let mut cycle = QuotaCycleState::default();
        advance_five_hour(&mut cycle, &five_hour_sample(900, Some(20.0), Some(1_000), "a"), 900, 900, false);
        let baseline = cycle.clone();
        assert!(!advance_five_hour(&mut cycle, &five_hour_sample(901, Some(0.0), None, "a"), 1_901, 900, false).changed);
        assert!(!advance_five_hour(&mut cycle, &five_hour_sample(901, None, None, "a"), 901, 900, false).changed);
        assert_eq!(cycle, baseline);
        let mut projected = five_hour_sample(902, Some(0.0), None, "a");
        projected.notification_five_hour = Some(QuotaSnapshot { used_percent: Some(20.0), resets_at: Some(1_000), window_duration_mins: Some(300) });
        assert_eq!(advance_five_hour(&mut cycle, &projected, 1_001, 900, false).notify_boundary, None);
        assert_eq!(cycle.last_seen_resets_at, Some(1_000));
    }

    #[test]
    fn older_samples_cannot_replace_the_current_window_or_its_fetch_target() {
        let mut cycle = QuotaCycleState::default();
        advance_five_hour(&mut cycle, &five_hour_sample(1_000, Some(10.0), Some(1_100), "a"), 1_000, 30, false);
        advance_five_hour(&mut cycle, &five_hour_sample(1_200, Some(4.0), Some(3_000), "a"), 1_200, 30, false);
        let before = cycle.clone();
        assert!(!advance_five_hour(&mut cycle, &five_hour_sample(1_100, Some(0.0), None, "a"), 1_200, 300, false).changed);
        assert_eq!(cycle, before);
        assert_eq!(post_reset_fetch_target(None, cycle.last_seen_resets_at, 1_200), Some(3_000));
    }

    #[test]
    fn legacy_state_and_account_switch_rebuild_the_baseline_without_an_alert() {
        let mut cycle: QuotaCycleState = serde_json::from_str(r#"{"lastSeenResetsAt":1000}"#).unwrap();
        assert_eq!(advance_five_hour(&mut cycle, &five_hour_sample(1_031, Some(0.0), None, "a"), 1_031, 30, false).notify_boundary, None);
        assert_eq!(cycle.last_handled_boundary, None);
        assert_eq!(cycle.last_seen_resets_at, None);
        let next = five_hour_sample(1_050, Some(40.0), Some(2_850), "a");
        assert_eq!(advance_five_hour(&mut cycle, &next, 1_050, 30, false).notify_boundary, None);
        let switch = advance_five_hour(&mut cycle, &five_hour_sample(1_060, Some(0.0), None, "b"), 1_060, 30, false);
        assert!(switch.cancel_timer);
        assert_eq!(switch.notify_boundary, None);
        assert_eq!(cycle.account_id.as_deref(), Some("b"));
        assert_eq!(cycle.last_seen_resets_at, None);
    }

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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
