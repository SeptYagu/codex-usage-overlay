//! Durable resume eligibility. Cooperative acceptance is never pause evidence.
use super::status::ControlCapabilities;
use crate::{
    codex::RawCodexUsage,
    config::{ConfigManager, OverlaySettings},
};
use serde::{Deserialize, Serialize};

const MAX_PLANS: usize = 512;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResumePhase {
    WaitingForQuota,
    Ready,
    NeedsReview,
    Submitting,
    SubmissionUnknown,
    Completed,
    Cancelled,
    Expired,
}

impl ResumePhase {
    fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Expired)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResumePlan {
    id: String,
    account_id: String,
    host_id: String,
    thread_id: String,
    interrupted_turn_id: String,
    activity_revision: String,
    eligible_after: i64,
    expires_at: i64,
    phase: ResumePhase,
    reason: Option<String>,
    #[serde(default)]
    first_recovered_at: Option<i64>,
    #[serde(default)]
    last_observed_at: Option<i64>,
}

impl ResumePlan {
    fn valid(&self) -> bool {
        [
            &self.id,
            &self.account_id,
            &self.host_id,
            &self.thread_id,
            &self.interrupted_turn_id,
            &self.activity_revision,
        ]
        .iter()
        .all(|v| !v.trim().is_empty() && v.len() <= 1024)
            && self.eligible_after > 0
            && self.eligible_after.checked_add(86400) == Some(self.expires_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumePlanSummary {
    pub id: String,
    pub thread_id: String,
    pub phase: ResumePhase,
    pub eligible_after: i64,
    pub reason: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResumeFile {
    schema_version: u32,
    plans: Vec<ResumePlan>,
}

pub struct ResumeManager {
    file: ResumeFile,
    revision: u64,
    dirty: bool,
    pub error: Option<String>,
}

/// The direct adapter must establish all of this before creating a plan.
pub struct PauseEvidence<'a> {
    pub action_id: &'a str,
    pub account_id: &'a str,
    pub host_id: &'a str,
    pub thread_id: &'a str,
    pub turn_id: &'a str,
    pub activity_revision: &'a str,
    pub reset_at: i64,
    pub weekly_triggered: bool,
}

impl ResumeManager {
    pub fn load(config: &ConfigManager) -> Self {
        let mut result = Self {
            file: ResumeFile {
                schema_version: 1,
                plans: vec![],
            },
            revision: 0,
            dirty: false,
            error: None,
        };
        let path = config.resume_state_path();
        if !path.exists() {
            return result;
        }
        let loaded = (|| {
            if std::fs::metadata(&path).ok()?.len() > MAX_FILE_BYTES {
                return None;
            }
            let file: ResumeFile = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            if file.schema_version != 1
                || file.plans.len() > MAX_PLANS
                || file.plans.iter().any(|p| !p.valid())
            {
                return None;
            }
            let mut ids = std::collections::HashSet::new();
            if file.plans.iter().any(|p| !ids.insert(p.id.clone())) {
                return None;
            }
            Some(file)
        })();
        match loaded {
            Some(mut file) => {
                for plan in &mut file.plans {
                    if !plan.phase.terminal() {
                        plan.phase = if matches!(
                            plan.phase,
                            ResumePhase::Submitting | ResumePhase::SubmissionUnknown
                        ) {
                            ResumePhase::SubmissionUnknown
                        } else {
                            ResumePhase::NeedsReview
                        };
                        plan.reason = Some("restartReviewRequired".into());
                        plan.first_recovered_at = None;
                        result.dirty = true;
                    }
                }
                result.file = file;
            }
            None => result.error = Some("resumeStateInvalid".into()),
        }
        result
    }

    pub fn plans(&self) -> Vec<ResumePlanSummary> {
        self.file
            .plans
            .iter()
            .map(|p| ResumePlanSummary {
                id: p.id.clone(),
                thread_id: p.thread_id.clone(),
                phase: p.phase,
                eligible_after: p.eligible_after,
                reason: p.reason.clone(),
            })
            .collect()
    }

    // Gated until a supported adapter can provide confirmed ownership evidence.
    #[allow(dead_code)]
    pub fn record_confirmed(
        &mut self,
        evidence: PauseEvidence<'_>,
        settings: &OverlaySettings,
        capabilities: ControlCapabilities,
        now: i64,
    ) -> Result<(), &'static str> {
        if self.error.as_deref() == Some("resumeStateInvalid") {
            return Err("resumeStateInvalid");
        }
        if !settings.auto_pause_on_low_quota
            || !settings.auto_resume_after_quota_reset
            || !capabilities.can_auto_resume()
        {
            return Err("resumeUnsupported");
        }
        if evidence.weekly_triggered
            || evidence.reset_at <= now
            || evidence
                .reset_at
                .checked_sub(now)
                .is_none_or(|distance| distance > 21600)
        {
            return Err("resumeIneligible");
        }
        if self.file.plans.iter().any(|p| p.id == evidence.action_id) {
            return Ok(());
        }
        self.file
            .plans
            .retain(|p| !p.phase.terminal() || now < p.expires_at);
        if self.file.plans.len() >= MAX_PLANS {
            return Err("resumePlanLimit");
        }
        let eligible_after = evidence
            .reset_at
            .checked_add(120)
            .ok_or("resumeIneligible")?;
        let plan = ResumePlan {
            id: evidence.action_id.into(),
            account_id: evidence.account_id.into(),
            host_id: evidence.host_id.into(),
            thread_id: evidence.thread_id.into(),
            interrupted_turn_id: evidence.turn_id.into(),
            activity_revision: evidence.activity_revision.into(),
            eligible_after,
            expires_at: eligible_after
                .checked_add(86400)
                .ok_or("resumeIneligible")?,
            phase: ResumePhase::WaitingForQuota,
            reason: None,
            first_recovered_at: None,
            last_observed_at: None,
        };
        if !plan.valid() {
            return Err("resumeIneligible");
        }
        self.file.plans.push(plan);
        self.changed();
        Ok(())
    }

    pub fn cancel_all(&mut self, reason: &str) {
        let mut changed = false;
        for plan in &mut self.file.plans {
            if !plan.phase.terminal() && plan.phase != ResumePhase::SubmissionUnknown {
                plan.phase = ResumePhase::Cancelled;
                plan.reason = Some(reason.into());
                changed = true;
            }
        }
        if changed {
            self.changed();
        }
    }

    pub fn observe(
        &mut self,
        raw: Option<&RawCodexUsage>,
        settings: &OverlaySettings,
        capabilities: ControlCapabilities,
        now: i64,
    ) {
        if !settings.auto_pause_on_low_quota || !settings.auto_resume_after_quota_reset {
            self.cancel_all("resumeDisabled");
            return;
        }
        let before = serde_json::to_vec(&self.file).ok();
        for plan in &mut self.file.plans {
            if plan.phase.terminal()
                || matches!(
                    plan.phase,
                    ResumePhase::Submitting | ResumePhase::SubmissionUnknown
                )
            {
                continue;
            }
            if now >= plan.expires_at {
                plan.phase = ResumePhase::Expired;
                plan.reason = Some("resumeExpired".into());
                continue;
            }
            if let Some(raw) = raw {
                if raw.account_id.as_deref() != Some(plan.account_id.as_str()) {
                    plan.phase = ResumePhase::Cancelled;
                    plan.reason = Some("accountChanged".into());
                    continue;
                }
            }
            if !capabilities.can_auto_resume() {
                plan.phase = ResumePhase::NeedsReview;
                plan.reason = Some("resumeUnsupported".into());
                plan.first_recovered_at = None;
                continue;
            }
            if plan.phase == ResumePhase::NeedsReview {
                continue;
            }
            let recovered = raw.is_some_and(|raw| {
                raw.fetched_at <= now
                    && now.saturating_sub(raw.fetched_at) <= 30
                    && raw.fetched_at >= plan.eligible_after
                    && raw.five_hour.used_percent.is_some_and(|v| {
                        v.is_finite()
                            && (0.0..=100.0).contains(&v)
                            && 100.0 - v
                                >= f64::from(
                                    settings
                                        .auto_pause_five_hour_threshold_percent
                                        .saturating_add(2),
                                )
                    })
                    && raw.weekly.used_percent.is_some_and(|v| {
                        v.is_finite()
                            && (0.0..=100.0).contains(&v)
                            && 100.0 - v
                                >= f64::from(
                                    settings
                                        .auto_pause_weekly_threshold_percent
                                        .saturating_add(2),
                                )
                    })
            });
            if !recovered {
                plan.phase = ResumePhase::WaitingForQuota;
                plan.first_recovered_at = None;
                plan.reason = Some("resumeWaitingForQuota".into());
                continue;
            }
            let at = raw.unwrap().fetched_at;
            if plan.last_observed_at.is_some_and(|last| at <= last) {
                continue;
            }
            plan.last_observed_at = Some(at);
            let first = *plan.first_recovered_at.get_or_insert(at);
            plan.phase = if at.saturating_sub(first) >= 15 {
                ResumePhase::Ready
            } else {
                ResumePhase::WaitingForQuota
            };
            plan.reason = None;
        }
        if before != serde_json::to_vec(&self.file).ok() {
            self.changed();
        }
    }

    // No production caller exists until direct control and activity observation
    // are verified. Persist this transition before a supported continuation call.
    #[allow(dead_code)]
    pub fn begin_submission(
        &mut self,
        id: &str,
        host: &str,
        thread: &str,
        turn: &str,
        activity_revision: &str,
        capabilities: ControlCapabilities,
        now: i64,
    ) -> Result<(), &'static str> {
        if !capabilities.can_auto_resume() {
            return Err("resumeUnsupported");
        }
        let plan = self
            .file
            .plans
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("resumeIneligible")?;
        if plan.phase != ResumePhase::Ready
            || now >= plan.expires_at
            || plan
                .last_observed_at
                .is_none_or(|at| now < at || now.saturating_sub(at) > 30)
        {
            return Err("resumeIneligible");
        }
        if plan.host_id != host
            || plan.thread_id != thread
            || plan.interrupted_turn_id != turn
            || plan.activity_revision != activity_revision
        {
            plan.phase = ResumePhase::Cancelled;
            plan.reason = Some("userActivityChanged".into());
            self.changed();
            return Err("userActivityChanged");
        }
        plan.phase = ResumePhase::Submitting;
        self.changed();
        Ok(())
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.revision = self.revision.saturating_add(1);
    }
    pub fn pending_write(&self) -> Result<Option<(u64, Vec<u8>)>, String> {
        if !self.dirty {
            return Ok(None);
        }
        Ok(Some((
            self.revision,
            serde_json::to_vec_pretty(&self.file).map_err(|e| e.to_string())?,
        )))
    }
    pub fn written(&mut self, revision: u64) {
        if self.revision == revision {
            self.dirty = false;
            self.error = None;
        }
    }
}

pub async fn update(state: &crate::commands::AppState, raw: Option<&RawCodexUsage>) {
    let _update = state.resume_update.lock().await;
    let settings = state.settings.lock().await.clone();
    let capabilities = state
        .auto_pause
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .transport
        .capabilities();
    let pending = {
        let mut manager = state.resume.lock().await;
        manager.observe(raw, &settings, capabilities, super::unix_now());
        manager.pending_write()
    };
    match pending {
        Ok(Some((revision, bytes))) => {
            let config = state.config_manager.clone();
            let result = tokio::task::spawn_blocking(move || {
                let path = config.resume_state_path();
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(tmp, path))
            })
            .await;
            let mut manager = state.resume.lock().await;
            match result {
                Ok(Ok(())) => manager.written(revision),
                _ => manager.error = Some("resumeStateWriteFailed".into()),
            }
        }
        Err(_) => state.resume.lock().await.error = Some("resumeStateWriteFailed".into()),
        Ok(None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::QuotaSnapshot;
    fn caps() -> ControlCapabilities {
        ControlCapabilities {
            cooperative_delivery: true,
            direct_interrupt: true,
            completion_observation: true,
            user_activity_observation: true,
            continuation_control: true,
        }
    }
    fn settings() -> OverlaySettings {
        OverlaySettings {
            auto_pause_on_low_quota: true,
            auto_resume_after_quota_reset: true,
            ..Default::default()
        }
    }
    fn manager() -> ResumeManager {
        ResumeManager {
            file: ResumeFile {
                schema_version: 1,
                plans: vec![],
            },
            revision: 0,
            dirty: false,
            error: None,
        }
    }
    fn evidence() -> PauseEvidence<'static> {
        PauseEvidence {
            action_id: "action",
            account_id: "account",
            host_id: "local",
            thread_id: "chat",
            turn_id: "turn",
            activity_revision: "revision",
            reset_at: 1000,
            weekly_triggered: false,
        }
    }
    fn raw(at: i64, five: f64, week: Option<f64>) -> RawCodexUsage {
        RawCodexUsage {
            account_id: Some("account".into()),
            five_hour: QuotaSnapshot {
                used_percent: Some(five),
                ..Default::default()
            },
            weekly: QuotaSnapshot {
                used_percent: week,
                ..Default::default()
            },
            credits_display: "—".into(),
            credits_balance: None,
            has_credits: false,
            fetched_at: at,
        }
    }
    #[test]
    fn cooperative_delivery_cannot_create_a_resume_plan() {
        let mut m = manager();
        assert_eq!(
            m.record_confirmed(evidence(), &settings(), ControlCapabilities::default(), 900),
            Err("resumeUnsupported")
        );
        assert!(m.plans().is_empty());
    }
    #[test]
    fn both_raw_windows_and_two_separated_fresh_samples_are_required() {
        let mut m = manager();
        m.record_confirmed(evidence(), &settings(), caps(), 900)
            .unwrap();
        for sample in [
            raw(1120, 93.001, Some(50.0)),
            raw(1135, 90.0, Some(96.001)),
            raw(1150, 90.0, None),
        ] {
            m.observe(Some(&sample), &settings(), caps(), sample.fetched_at);
            assert_ne!(m.file.plans[0].phase, ResumePhase::Ready);
        }
        m.observe(
            Some(&raw(1160, 93.0, Some(96.0))),
            &settings(),
            caps(),
            1160,
        );
        m.observe(
            Some(&raw(1160, 93.0, Some(96.0))),
            &settings(),
            caps(),
            1175,
        );
        assert_ne!(m.file.plans[0].phase, ResumePhase::Ready);
        m.observe(
            Some(&raw(1175, 93.0, Some(96.0))),
            &settings(),
            caps(),
            1175,
        );
        assert_eq!(m.file.plans[0].phase, ResumePhase::Ready);
        assert_eq!(
            m.begin_submission(
                "action",
                "local",
                "chat",
                "turn",
                "new-user-action",
                caps(),
                1175
            ),
            Err("userActivityChanged")
        );
        assert_eq!(m.file.plans[0].phase, ResumePhase::Cancelled);
    }
    #[test]
    fn a_failed_read_resets_recovery_and_account_changes_cancel() {
        let mut m = manager();
        m.record_confirmed(evidence(), &settings(), caps(), 900)
            .unwrap();
        m.observe(Some(&raw(1120, 0.0, Some(0.0))), &settings(), caps(), 1120);
        m.observe(None, &settings(), caps(), 1121);
        m.observe(Some(&raw(1135, 0.0, Some(0.0))), &settings(), caps(), 1135);
        assert_ne!(m.file.plans[0].phase, ResumePhase::Ready);
        let mut sample = raw(1150, 0.0, Some(0.0));
        sample.account_id = Some("other".into());
        m.observe(Some(&sample), &settings(), caps(), 1150);
        assert_eq!(m.file.plans[0].phase, ResumePhase::Cancelled);
    }
    #[test]
    fn restart_does_not_retry_an_unknown_submission() {
        let dir = std::env::temp_dir().join(format!("overlay-resume-test-{}", std::process::id()));
        let config = ConfigManager::with_runtime_dir(dir.clone());
        let mut m = manager();
        m.record_confirmed(evidence(), &settings(), caps(), 900)
            .unwrap();
        m.observe(Some(&raw(1120, 0.0, Some(0.0))), &settings(), caps(), 1120);
        m.observe(Some(&raw(1135, 0.0, Some(0.0))), &settings(), caps(), 1135);
        m.begin_submission("action", "local", "chat", "turn", "revision", caps(), 1135)
            .unwrap();
        std::fs::write(
            config.resume_state_path(),
            m.pending_write().unwrap().unwrap().1,
        )
        .unwrap();
        let loaded = ResumeManager::load(&config);
        assert_eq!(loaded.file.plans[0].phase, ResumePhase::SubmissionUnknown);
        std::fs::remove_file(config.resume_state_path()).unwrap();
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn weekly_missing_and_past_resets_cannot_create_plans() {
        let mut m = manager();
        let mut weekly = evidence();
        weekly.weekly_triggered = true;
        assert_eq!(
            m.record_confirmed(weekly, &settings(), caps(), 900),
            Err("resumeIneligible")
        );
        for reset_at in [0, 900, 30_000] {
            let mut e = evidence();
            e.reset_at = reset_at;
            assert_eq!(
                m.record_confirmed(e, &settings(), caps(), 900),
                Err("resumeIneligible")
            );
        }
        assert!(m.plans().is_empty());
    }

    #[test]
    fn stale_future_and_disabled_observations_never_resume_and_plans_expire() {
        let mut m = manager();
        m.record_confirmed(evidence(), &settings(), caps(), 900)
            .unwrap();
        for (at, now) in [(1120, 1151), (1160, 1150), (1119, 1120)] {
            m.observe(Some(&raw(at, 0.0, Some(0.0))), &settings(), caps(), now);
            assert_eq!(m.file.plans[0].phase, ResumePhase::WaitingForQuota);
            assert!(m.file.plans[0].first_recovered_at.is_none());
        }
        m.observe(Some(&raw(1160, 0.0, Some(0.0))), &settings(), caps(), 1160);
        m.observe(Some(&raw(1175, 0.0, Some(0.0))), &settings(), caps(), 1175);
        assert_eq!(m.file.plans[0].phase, ResumePhase::Ready);
        assert_eq!(
            m.begin_submission("action", "local", "chat", "turn", "revision", caps(), 1206),
            Err("resumeIneligible")
        );
        let mut disabled = settings();
        disabled.auto_resume_after_quota_reset = false;
        m.observe(None, &disabled, caps(), 1206);
        assert_eq!(m.file.plans[0].phase, ResumePhase::Cancelled);

        let mut m = manager();
        m.record_confirmed(evidence(), &settings(), caps(), 900)
            .unwrap();
        let expiry = m.file.plans[0].expires_at;
        m.observe(
            Some(&raw(expiry, 0.0, Some(0.0))),
            &settings(),
            caps(),
            expiry,
        );
        assert_eq!(m.file.plans[0].phase, ResumePhase::Expired);
    }

    #[test]
    fn corrupt_state_is_reported_and_preserved_instead_of_overwritten() {
        let dir =
            std::env::temp_dir().join(format!("overlay-resume-invalid-{}", std::process::id()));
        let config = ConfigManager::with_runtime_dir(dir.clone());
        let mut duplicate = manager();
        duplicate
            .record_confirmed(evidence(), &settings(), caps(), 900)
            .unwrap();
        duplicate.file.plans.push(duplicate.file.plans[0].clone());
        let cases = [
            b"{broken".to_vec(),
            br#"{"schemaVersion":2,"plans":[]}"#.to_vec(),
            vec![b' '; MAX_FILE_BYTES as usize + 1],
            serde_json::to_vec(&duplicate.file).unwrap(),
        ];
        for bytes in cases {
            std::fs::write(config.resume_state_path(), &bytes).unwrap();
            let mut loaded = ResumeManager::load(&config);
            assert_eq!(loaded.error.as_deref(), Some("resumeStateInvalid"));
            assert!(loaded.pending_write().unwrap().is_none());
            assert_eq!(
                loaded.record_confirmed(evidence(), &settings(), caps(), 900),
                Err("resumeStateInvalid")
            );
            assert_eq!(std::fs::read(config.resume_state_path()).unwrap(), bytes);
        }
        std::fs::remove_file(config.resume_state_path()).unwrap();
        let _ = std::fs::remove_dir(dir);
    }
}
