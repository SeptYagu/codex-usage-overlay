use super::{triggered_quotas, unix_now, AutoPauseController, BridgeConnection};
use crate::{commands::AppState, config::OverlaySettings, tray::TrayReadStatus};
use serde::Serialize;
use std::collections::BTreeMap;
use tauri::Emitter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlCapabilities {
    pub cooperative_delivery: bool,
    pub direct_interrupt: bool,
    pub completion_observation: bool,
    pub user_activity_observation: bool,
    pub continuation_control: bool,
}

impl Default for ControlCapabilities {
    fn default() -> Self {
        // The installed tools/call bridge demonstrates message delivery only.
        Self {
            cooperative_delivery: true,
            direct_interrupt: false,
            completion_observation: false,
            user_activity_observation: false,
            continuation_control: false,
        }
    }
}

impl ControlCapabilities {
    pub fn can_auto_resume(self) -> bool {
        self.direct_interrupt
            && self.completion_observation
            && self.user_activity_observation
            && self.continuation_control
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetStatus {
    pub thread_id: String,
    pub host_id: Option<String>,
    pub turn_id: Option<String>,
    pub period_id: String,
    pub active: bool,
    pub phase: String,
    pub reason: Option<String>,
    pub requested_at: i64,
    pub accepted_at: Option<i64>,
    pub interrupted_at: Option<i64>,
}

#[derive(Default)]
pub(super) struct Telemetry {
    pub targets: BTreeMap<String, TargetStatus>,
    pub account_known: bool,
    pub last_probe_at: Option<i64>,
    pub probe_running: bool,
    pub probe_failures: u32,
}

impl Telemetry {
    pub fn record(
        &mut self,
        thread: &super::ActiveThread,
        period_id: &str,
        phase: &str,
        reason: Option<&str>,
    ) {
        let now = unix_now();
        self.targets
            .retain(|_, target| now.saturating_sub(target.requested_at) < 86400);
        let key = period_id.to_owned();
        if !self.targets.contains_key(&key) && self.targets.len() >= 512 {
            if let Some(oldest) = self
                .targets
                .iter()
                .min_by_key(|(_, v)| v.requested_at)
                .map(|(k, _)| k.clone())
            {
                self.targets.remove(&oldest);
            }
        }
        let target = self.targets.entry(key).or_insert_with(|| TargetStatus {
            thread_id: thread.id.clone(),
            host_id: thread.host_id.clone(),
            turn_id: thread.turn_id.clone(),
            period_id: period_id.into(),
            active: true,
            phase: phase.into(),
            reason: None,
            requested_at: now,
            accepted_at: None,
            interrupted_at: None,
        });
        target.phase = phase.into();
        target.reason = reason.map(str::to_owned);
        if phase == "accepted" {
            target.accepted_at = Some(now);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoPauseStatus {
    pub revision: u64,
    pub enabled: bool,
    pub auto_resume_enabled: bool,
    pub connection: BridgeConnection,
    pub capabilities: ControlCapabilities,
    pub mode: String,
    pub summary: String,
    pub reason: Option<String>,
    /// Deprecated alias of accepted_request_count.
    pub confirmed_count: u64,
    pub accepted_request_count: u64,
    pub confirmed_interruption_count: u64,
    pub last_confirmed_at: Option<i64>,
    pub last_probe_at: Option<i64>,
    pub last_successful_sample_at: Option<i64>,
    pub effective_interval_seconds: u64,
    pub next_poll_at: Option<i64>,
    pub quota_health: String,
    pub targets: Vec<TargetStatus>,
    pub resume_plans: Vec<super::resume::ResumePlanSummary>,
    pub resume_state_error: Option<String>,
}

impl AutoPauseStatus {
    pub(super) fn from_controller(controller: &AutoPauseController, enabled: bool) -> Self {
        Self {
            revision: 0,
            enabled,
            auto_resume_enabled: false,
            connection: controller.connection,
            capabilities: controller.transport.capabilities(),
            mode: "cooperative".into(),
            summary: if enabled { "checking" } else { "off" }.into(),
            reason: None,
            confirmed_count: controller.confirmed_count,
            accepted_request_count: controller.confirmed_count,
            confirmed_interruption_count: 0,
            last_confirmed_at: controller.last_confirmed_at,
            last_probe_at: controller.telemetry.last_probe_at,
            last_successful_sample_at: None,
            effective_interval_seconds: 60,
            next_poll_at: None,
            quota_health: "pending".into(),
            targets: controller
                .telemetry
                .targets
                .values()
                .filter(|t| unix_now().saturating_sub(t.requested_at) < 86400)
                .cloned()
                .map(|mut target| {
                    target.active = controller
                        .periods
                        .values()
                        .any(|p| p.period_id == target.period_id);
                    target
                })
                .collect(),
            resume_plans: Vec::new(),
            resume_state_error: None,
        }
    }

    pub fn tooltip(&self, language: &str) -> &'static str {
        let zh = crate::commands::resolve_locale(language);
        match (zh, self.summary.as_str()) {
            (_, "off") => match zh {
                "zh-CN" => "暂停保护：关闭",
                "zh-Hant" => "暫停保護：關閉",
                _ => "Pause protection: off",
            },
            (_, "armed") => match zh {
                "zh-CN" => "暂停保护：就绪",
                "zh-Hant" => "暫停保護：就緒",
                _ => "Pause protection: armed",
            },
            (_, "quotaLow") => match zh {
                "zh-CN" => "额度不足：无活跃会话",
                "zh-Hant" => "額度不足：無活躍會話",
                _ => "Quota low: no active chats",
            },
            (_, "pauseRequested") => match zh {
                "zh-CN" => "暂停请求：未确认停止",
                "zh-Hant" => "暫停請求：未確認停止",
                _ => "Pause requested: stopping unconfirmed",
            },
            (_, "needsAttention") => match zh {
                "zh-CN" => "暂停保护：需要关注",
                "zh-Hant" => "暫停保護：需要關注",
                _ => "Pause protection: needs attention",
            },
            _ => match zh {
                "zh-CN" => "暂停保护：检查中",
                "zh-Hant" => "暫停保護：檢查中",
                _ => "Pause protection: checking",
            },
        }
    }
}

#[derive(Default)]
pub struct StatusCache {
    latest: Option<AutoPauseStatus>,
    emitted_revision: u64,
}

pub async fn snapshot(state: &AppState) -> AutoPauseStatus {
    // Keep the same settings-first ordering as the presentation pipeline.
    let settings = state.settings.lock().await;
    let read = *state.tray_read_status.lock().await;
    let usage = state.last_usage.lock().await.clone();
    let (mut status, running, account_known) = {
        let controller = state.auto_pause.lock().unwrap_or_else(|p| p.into_inner());
        (
            controller.status(settings.auto_pause_on_low_quota),
            controller.running,
            controller.telemetry.account_known,
        )
    };
    let now = unix_now();
    {
        let polling = state.polling.lock().unwrap_or_else(|p| p.into_inner());
        status.effective_interval_seconds = if polling.effective_interval_seconds == 0 {
            u64::from(settings.refresh_interval_seconds.clamp(15, 3600))
        } else {
            polling.effective_interval_seconds
        };
        status.next_poll_at = polling.next_poll_at(std::time::Instant::now(), now);
    }
    status.auto_resume_enabled = settings.auto_resume_after_quota_reset;
    status.last_successful_sample_at = usage.as_ref().map(|v| v.fetched_at);
    let stale = usage.as_ref().is_some_and(|u| {
        u.fetched_at > now.saturating_add(5)
            || now.saturating_sub(u.fetched_at)
                > (status.effective_interval_seconds.saturating_mul(2).max(60) as i64)
    });
    status.quota_health = match read {
        TrayReadStatus::Pending => "pending",
        TrayReadStatus::Error => "error",
        TrayReadStatus::Success if stale => "stale",
        _ => "success",
    }
    .into();
    {
        let resume = state.resume.lock().await;
        status.resume_plans = resume.plans();
        status.resume_state_error = resume.error.clone();
    }
    derive_summary(
        &mut status,
        &settings,
        usage.as_ref(),
        running,
        account_known,
    );
    let mut cache = state
        .protection_status
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(previous) = &cache.latest {
        status.revision = previous.revision;
        if *previous == status {
            return status;
        }
    }
    status.revision = status.revision.saturating_add(1);
    cache.latest = Some(status.clone());
    status
}

fn derive_summary(
    status: &mut AutoPauseStatus,
    settings: &OverlaySettings,
    usage: Option<&crate::codex::CodexUsage>,
    running: bool,
    account_known: bool,
) {
    if !status.enabled {
        status.summary = "off".into();
        return;
    }
    let reason = if status.quota_health == "error" {
        Some("quotaReadFailed")
    } else if status.quota_health == "stale" {
        Some("quotaStale")
    } else if usage.is_some() && !account_known {
        Some("accountUnknown")
    } else if usage.is_some_and(|u| {
        u.five_hour_remaining_percent.is_none() || u.week_remaining_percent.is_none()
    }) {
        Some("quotaUnavailable")
    } else if let Some(reason) = status.resume_state_error.as_deref() {
        Some(reason)
    } else if status.resume_plans.iter().any(|p| {
        matches!(
            p.phase,
            super::resume::ResumePhase::NeedsReview | super::resume::ResumePhase::SubmissionUnknown
        )
    }) {
        Some("resumeNeedsReview")
    } else if status.auto_resume_enabled && !status.capabilities.can_auto_resume() {
        Some("resumeUnsupported")
    } else if status.connection == BridgeConnection::Unavailable {
        Some("bridgeUnavailable")
    } else {
        None
    };
    if let Some(reason) = reason {
        status.summary = "needsAttention".into();
        status.reason = Some(reason.into());
        return;
    }
    if status.quota_health == "pending" || status.connection == BridgeConnection::Unknown {
        status.summary = "checking".into();
        return;
    }
    if let Some(usage) = usage {
        if !triggered_quotas(settings, usage).is_empty() {
            if status
                .targets
                .iter()
                .any(|t| t.active && t.phase == "failed")
            {
                status.summary = "needsAttention".into();
                status.reason = Some("deliveryFailed".into());
            } else if running
                || status
                    .targets
                    .iter()
                    .any(|t| t.active && (t.phase == "accepted" || t.phase == "requested"))
            {
                status.summary = "pauseRequested".into();
                status.reason = Some("stoppingUnconfirmed".into());
            } else {
                status.summary = "quotaLow".into();
            }
            return;
        }
    }
    status.summary = "armed".into();
}

pub async fn publish(app: &tauri::AppHandle, state: &AppState) {
    let status = snapshot(state).await;
    let should_emit = {
        let mut cache = state
            .protection_status
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if cache.emitted_revision >= status.revision {
            false
        } else {
            cache.emitted_revision = status.revision;
            true
        }
    };
    if should_emit {
        let _ = app.emit("auto_pause_status_changed", &status);
        crate::tray::repaint_tray(app, state).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cooperative_capabilities_never_allow_automatic_resume() {
        assert!(!ControlCapabilities::default().can_auto_resume());
    }
    #[test]
    fn accepted_requests_are_not_interruption_confirmation() {
        let mut controller = AutoPauseController::new();
        controller.confirmed_count = 3;
        let status = controller.status(true);
        assert_eq!(status.accepted_request_count, 3);
        assert_eq!(status.confirmed_count, 3);
        assert_eq!(status.confirmed_interruption_count, 0);
        assert!(!status.capabilities.direct_interrupt);
    }

    fn state(tag: &str) -> (AppState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "overlay-protection-status-{tag}-{}",
            std::process::id()
        ));
        (
            AppState::for_test(crate::config::ConfigManager::with_runtime_dir(dir.clone())),
            dir,
        )
    }
    fn usage(five: Option<u32>) -> crate::codex::CodexUsage {
        crate::codex::CodexUsage {
            five_hour_remaining_percent: five,
            week_remaining_percent: Some(50),
            five_hour_burn_rate_per_hour: None,
            week_burn_rate_per_hour: None,
            credits_display: "—".into(),
            credits_balance: None,
            has_credits: false,
            five_hour_resets_at: Some(unix_now() + 3600),
            week_resets_at: None,
            fetched_at: unix_now(),
        }
    }
    #[tokio::test]
    async fn snapshots_are_versioned_and_getters_do_not_consume_pending_events() {
        let (state, dir) = state("ordering");
        let first = snapshot(&state).await;
        assert_eq!(first.summary, "off");
        assert_eq!(snapshot(&state).await.revision, first.revision);
        state.settings.lock().await.auto_pause_on_low_quota = true;
        *state.last_usage.lock().await = Some(usage(Some(3)));
        *state.tray_read_status.lock().await = TrayReadStatus::Success;
        {
            let mut controller = state.auto_pause.lock().unwrap();
            controller.connection = BridgeConnection::Available;
            controller.telemetry.account_known = true;
            let thread = super::super::ActiveThread {
                id: "chat".into(),
                host_id: Some("local".into()),
                turn_id: None,
            };
            controller.refresh_periods(std::slice::from_ref(&thread));
            let period = controller.period_id("chat").unwrap();
            controller
                .telemetry
                .record(&thread, &period, "accepted", Some("stoppingUnconfirmed"));
            controller.confirmed_count = 1;
        }
        let next = snapshot(&state).await;
        assert!(next.revision > first.revision);
        assert_eq!(next.summary, "pauseRequested");
        assert_eq!(next.accepted_request_count, 1);
        assert_eq!(next.confirmed_interruption_count, 0);
        assert!(state.protection_status.lock().unwrap().emitted_revision < next.revision);
        state.auto_pause.lock().unwrap().refresh_periods(&[]);
        let idle = snapshot(&state).await;
        assert_eq!(
            idle.summary, "quotaLow",
            "historical acceptance cannot imply an active request"
        );
        assert!(!idle.targets[0].active);
        state.settings.lock().await.auto_pause_on_low_quota = false;
        assert_eq!(snapshot(&state).await.summary, "off");
        let _ = std::fs::remove_dir(dir);
    }
    #[tokio::test]
    async fn missing_stale_or_failed_quota_cannot_display_armed() {
        let (state, dir) = state("health");
        state.settings.lock().await.auto_pause_on_low_quota = true;
        *state.tray_read_status.lock().await = TrayReadStatus::Success;
        {
            let mut controller = state.auto_pause.lock().unwrap();
            controller.connection = BridgeConnection::Available;
            controller.telemetry.account_known = true;
        }
        *state.last_usage.lock().await = Some(usage(None));
        assert_eq!(
            snapshot(&state).await.reason.as_deref(),
            Some("quotaUnavailable")
        );
        let mut old = usage(Some(80));
        old.fetched_at -= 1000;
        *state.last_usage.lock().await = Some(old);
        assert_eq!(snapshot(&state).await.reason.as_deref(), Some("quotaStale"));
        let mut future = usage(Some(80));
        future.fetched_at += 1000;
        *state.last_usage.lock().await = Some(future);
        assert_eq!(snapshot(&state).await.reason.as_deref(), Some("quotaStale"));
        *state.tray_read_status.lock().await = TrayReadStatus::Error;
        assert_eq!(
            snapshot(&state).await.reason.as_deref(),
            Some("quotaReadFailed")
        );
        let _ = std::fs::remove_dir(dir);
    }
    #[test]
    fn target_history_is_bounded_and_transport_health_retries_are_coalesced() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let (state, dir) = state("bounded");
        let state = Arc::new(state);
        let count = Arc::new(AtomicUsize::new(0));
        {
            let mut controller = state.auto_pause.lock().unwrap();
            for i in 0..513 {
                controller.telemetry.record(
                    &super::super::ActiveThread {
                        id: format!("chat-{i}"),
                        host_id: None,
                        turn_id: None,
                    },
                    &format!("period-{i}"),
                    "failed",
                    Some("deliveryFailed"),
                );
            }
            assert_eq!(controller.telemetry.targets.len(), 512);
            controller.connection = BridgeConnection::Unavailable;
            controller.telemetry.last_probe_at = Some(unix_now() - 301);
            let count = count.clone();
            controller.set_probe_spawner(Arc::new(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
            }));
        }
        super::super::probe_bridge_status(&state);
        super::super::probe_bridge_status(&state);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir(dir);
    }
}
