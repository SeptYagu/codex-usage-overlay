//! Opt-in cooperative DSH pause control. Never calls the local Codex bridge.
//! A receipt means accepted steering input, never a confirmed interruption.
use crate::{codex::RawCodexUsage, commands::AppState, config::{OverlaySettings, Provider}};
use std::{collections::HashSet, sync::{Arc, atomic::Ordering}};

#[derive(Default)]
pub struct ControlState {
    epoch: u64,
    attempted: HashSet<String>,
    pub running: bool,
    pub accepted: u64,
    pub last_accepted_at: Option<i64>,
    pub failed: bool,
}
impl ControlState {
    fn begin_attempt(&mut self, epoch: u64, keys: &[String]) -> bool {
        if self.epoch != epoch || !self.running { return false; }
        self.attempted.extend(keys.iter().cloned());
        true
    }
    pub fn invalidate(&mut self) { self.epoch = self.epoch.wrapping_add(1); self.running = false; }
    pub fn reset(&mut self) { let epoch = self.epoch.wrapping_add(1); *self = Self { epoch, ..Self::default() }; }
}

fn trigger_cycles(settings: &OverlaySettings, raw: &RawCodexUsage, now: i64) -> Vec<String> {
    if settings.provider != Provider::Dsh || !settings.auto_pause_on_low_quota
        || settings.dsh_instance_id.is_empty() || settings.dsh_base_url.is_empty()
        || settings.dsh_account_id.is_empty() || settings.dsh_session_ids.is_empty()
        || raw.account_id.as_deref() != Some(settings.dsh_account_id.as_str())
        || raw.fetched_at > now.saturating_add(5) || now.saturating_sub(raw.fetched_at) > 60 {
        return Vec::new();
    }
    [(&raw.five_hour, settings.dsh_five_hour_threshold_percent, "five"),
        (&raw.weekly, settings.dsh_weekly_threshold_percent, "week")]
        .into_iter().filter_map(|(quota, threshold, name)| {
            let used = quota.used_percent?;
            let reset = quota.resets_at?;
            // Judge the raw floating-point reading, not its rounded UI. Thus
            // 99.6% used does not trigger an exhausted-only (0%) threshold.
            (used.is_finite() && (0.0..=100.0).contains(&used) && reset > now
                && 100.0 - used <= f64::from(threshold))
                .then(|| format!("{name}:{reset}"))
        }).collect()
}

pub fn schedule(state: &Arc<AppState>, settings: OverlaySettings, raw: RawCodexUsage, generation: u64) {
    let cycles = trigger_cycles(&settings, &raw, crate::codex::reader::unix_now());
    if cycles.is_empty() { return; }
    let revision = state.settings_revision.load(Ordering::Acquire);
    let (epoch, targets, keys) = {
        let mut control = state.dsh_control.lock().unwrap_or_else(|p| p.into_inner());
        if control.running { return; }
        let source = crate::config::source_key(&settings);
        let targets: Vec<_> = settings.dsh_session_ids.iter().filter(|id| cycles.iter().any(|cycle|
            !control.attempted.contains(&format!("{source}:{}:{id}:{cycle}", settings.dsh_account_id)))).cloned().collect();
        if targets.is_empty() { return; }
        if control.attempted.len() > 4096 { return; }
        let mut keys = Vec::new();
        for id in &targets { for cycle in &cycles {
            keys.push(format!("{source}:{}:{id}:{cycle}", settings.dsh_account_id));
        }}
        // Only reserve the worker here. A settings change can still cancel it
        // before dispatch, in which case no attempt keys must be consumed.
        control.running = true;
        (control.epoch, targets, keys)
    };
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        // The commit lock serializes source changes with the actual request.
        // No network result from a previous binding may control a new source.
        let _commit = state.settings_patch.lock().await;
        let current = state.settings.lock().await.clone();
        let valid = state.source_generation.load(Ordering::Acquire) == generation
            && state.settings_revision.load(Ordering::Acquire) == revision
            && crate::config::same_source(&settings, &current)
            && !trigger_cycles(&current, &raw, crate::codex::reader::unix_now()).is_empty()
            && state.dsh_control.lock().unwrap_or_else(|p| p.into_inner()).epoch == epoch;
        let dispatched = valid && state.dsh_control.lock().unwrap_or_else(|p| p.into_inner()).begin_attempt(epoch, &keys);
        // Mark immediately before the transport: an ambiguous timeout is not
        // retried. A new quota cycle or explicit source rebind re-arms it.
        let result = if dispatched {
            let prompt = crate::pause_control::pause_message_for_language(&current.language);
            Some(state.dsh.pause(&current.dsh_account_id, &targets, prompt).await)
        } else { None };
        let mut control = state.dsh_control.lock().unwrap_or_else(|p| p.into_inner());
        if control.epoch != epoch { return; }
        control.running = false;
        if let Some(result) = result {
            match result {
                Ok(result) => {
                    let accepted = result.sessions.iter().filter(|s| s.accepted).count() as u64;
                    control.accepted = control.accepted.saturating_add(accepted);
                    control.failed = result.sessions.iter().any(|s| !s.accepted);
                    if accepted > 0 { control.last_accepted_at = Some(crate::codex::reader::unix_now()); }
                },
                Err(_) => control.failed = true,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::QuotaSnapshot;
    fn setup() -> (OverlaySettings, RawCodexUsage) {
        let settings = OverlaySettings { provider: Provider::Dsh, auto_pause_on_low_quota: true,
            dsh_base_url: "http://127.0.0.1:1".into(), dsh_instance_id: "instance".into(),
            dsh_account_id: "account".into(), dsh_session_ids: vec!["session".into()], ..Default::default() };
        let raw = RawCodexUsage { account_id: Some("account".into()),
            five_hour: QuotaSnapshot { used_percent: Some(100.0), resets_at: Some(2000), ..Default::default() },
            weekly: QuotaSnapshot { used_percent: Some(60.0), resets_at: Some(9000), ..Default::default() },
            credits_display: "—".into(), credits_balance: None, has_credits: false, fetched_at: 1000 };
        (settings, raw)
    }
    #[test] fn exhausted_means_raw_100_not_rounded_ui_zero() {
        let (settings, mut raw) = setup();
        assert_eq!(trigger_cycles(&settings, &raw, 1000), vec!["five:2000"]);
        raw.five_hour.used_percent = Some(99.6);
        assert!(trigger_cycles(&settings, &raw, 1000).is_empty());
    }
    #[test] fn invalid_missing_stale_wrong_identity_and_other_providers_never_control() {
        let (mut settings, mut raw) = setup();
        settings.provider = Provider::Codex;
        assert!(trigger_cycles(&settings, &raw, 1000).is_empty());
        settings.provider = Provider::Dsh;
        raw.account_id = Some("other".into());
        assert!(trigger_cycles(&settings, &raw, 1000).is_empty());
        raw.account_id = Some("account".into());
        assert!(trigger_cycles(&settings, &raw, 1061).is_empty());
        raw.five_hour.used_percent = Some(f64::NAN);
        assert!(trigger_cycles(&settings, &raw, 1000).is_empty());
        raw.five_hour.used_percent = Some(100.0); raw.five_hour.resets_at = None;
        assert!(trigger_cycles(&settings, &raw, 1000).is_empty());
    }
    #[test] fn dsh_thresholds_are_independent_and_both_windows_can_trigger() {
        let (mut settings, mut raw) = setup();
        raw.five_hour.used_percent = Some(97.0);
        assert!(trigger_cycles(&settings, &raw, 1000).is_empty());
        settings.dsh_five_hour_threshold_percent = 3;
        raw.weekly.used_percent = Some(100.0);
        assert_eq!(trigger_cycles(&settings, &raw, 1000), vec!["five:2000", "week:9000"]);
        assert_eq!(settings.auto_pause_five_hour_threshold_percent, 5);
    }
}
