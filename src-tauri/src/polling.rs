//! One polling policy for background, manual and reset-triggered reads.
use crate::{codex::CodexUsage, config::ProviderPreference};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct PollState {
    fast: bool,
    recovery_samples: u8,
    failures: u32,
    last_started: Option<Instant>,
    last_completed: Option<Instant>,
    deadline: Option<Instant>,
    pub effective_interval_seconds: u64,
    recovery: ReadRecovery,
    server_not_before: Option<Instant>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReadRecovery {
    pub transient: bool,
    pub retry_after_seconds: Option<u64>,
    pub display_only: bool,
}

impl PollState {
    pub fn started(&mut self, now: Instant) {
        self.last_started = Some(now);
    }

    #[cfg(test)]
    pub fn completed(
        &mut self,
        pref: &ProviderPreference,
        usage: Option<&CodexUsage>,
        now: Instant,
    ) {
        self.completed_with_recovery(pref, usage, now, ReadRecovery::default());
    }

    pub fn completed_with_recovery(&mut self, pref: &ProviderPreference, usage: Option<&CodexUsage>, now: Instant, recovery: ReadRecovery) {
        self.recovery = recovery;
        self.server_not_before = recovery.retry_after_seconds.and_then(|secs| now.checked_add(Duration::from_secs(secs)));
        self.last_completed = Some(now);
        let active_protection = pref.monitor_enabled && pref.protection_enabled;
        if let Some(usage) = usage {
            self.failures = 0;
            if !active_protection {
                self.fast = false;
                self.recovery_samples = 0;
            } else {
                let five_threshold = pref.five_hour_threshold_percent;
                let week_threshold = pref.weekly_threshold_percent;
                let near = usage.five_hour_remaining_percent.is_some_and(|v| {
                    v <= five_threshold
                        .saturating_add(5)
                }) || usage.week_remaining_percent.is_some_and(|v| {
                    v <= week_threshold
                        .saturating_add(5)
                });
                let recovered = usage.five_hour_remaining_percent.is_some_and(|v| {
                    v > five_threshold
                        .saturating_add(7)
                }) && usage.week_remaining_percent.is_some_and(|v| {
                    v > week_threshold
                        .saturating_add(7)
                });
                if near {
                    self.fast = true;
                    self.recovery_samples = 0;
                } else if self.fast && recovered {
                    self.recovery_samples += 1;
                    if self.recovery_samples >= 2 {
                        self.fast = false;
                    }
                } else {
                    self.recovery_samples = 0;
                }
            }
        } else {
            self.failures = self.failures.saturating_add(1);
            self.recovery_samples = 0;
        }
        self.recalculate(pref, now);
    }

    pub fn recalculate(&mut self, pref: &ProviderPreference, now: Instant) {
        let normal = u64::from(pref.refresh_interval_seconds.clamp(15, 3600));
        let active_protection = pref.monitor_enabled && pref.protection_enabled;
        if !active_protection {
            self.fast = false;
        }
        let base = if active_protection && self.fast {
            15
        } else {
            normal
        };
        self.effective_interval_seconds = if self.recovery.display_only { 15 }
        else if self.recovery.transient && self.failures > 0 {
            15u64.saturating_mul(1u64 << self.failures.saturating_sub(1).min(10)).min(normal.max(300))
        } else { base
            .saturating_mul(1u64 << self.failures.min(10))
            .min(normal.max(300)) };
        let start = if self.recovery.transient || self.recovery.display_only { self.last_completed } else { self.last_started }.unwrap_or(now);
        let floor = self
            .last_completed
            .map(|at| at + Duration::from_secs(5))
            .unwrap_or(now);
        self.deadline =
            Some((start + Duration::from_secs(self.effective_interval_seconds)).max(floor).max(self.server_not_before.unwrap_or(now)));
    }

    pub fn request_now(&mut self, now: Instant) {
        self.deadline = Some(now.max(self.server_not_before.unwrap_or(now)));
    }

    pub fn server_wait(&self, now: Instant) -> Option<Duration> {
        self.server_not_before.filter(|at| *at > now).map(|at| at.duration_since(now))
    }

    pub fn healthy_interval_seconds(&self, pref: &ProviderPreference) -> u64 {
        if pref.monitor_enabled && pref.protection_enabled && self.fast { 15 } else { u64::from(pref.refresh_interval_seconds.clamp(15, 3600)) }
    }

    pub fn deadline(&mut self, now: Instant) -> Instant {
        *self
            .deadline
            .get_or_insert(now + Duration::from_millis(500))
    }

    pub fn next_poll_at(&self, now: Instant, unix_now: i64) -> Option<i64> {
        self.deadline.map(|deadline| {
            unix_now.saturating_add(
                deadline
                    .saturating_duration_since(now)
                    .as_secs()
                    .min(i64::MAX as u64) as i64,
            )
        })
    }
}

/// Seconds until a sample fetched at `fetched_at` next crosses a freshness
/// window: the monitor snapshot's (`2 x refresh`, at least 120 s) or the
/// protection status's (`2 x healthy`, at least 60 s). `None` once both passed.
pub fn seconds_until_stale(now: i64, fetched_at: i64, refresh_seconds: u64, healthy_seconds: u64) -> Option<u64> {
    [
        refresh_seconds.saturating_mul(2).max(120),
        healthy_seconds.saturating_mul(2).max(60),
    ]
    .into_iter()
    .filter_map(|window| {
        let stale_at = fetched_at.saturating_add(window as i64).saturating_add(1);
        u64::try_from(stale_at.saturating_sub(now)).ok().filter(|wait| *wait > 0)
    })
    .min()
}

pub async fn wait_until_due(state: &crate::commands::ProviderRuntime) {
    loop {
        let wake = state.poll_wake.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        let deadline = state
            .polling
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .deadline(Instant::now());
        tokio::select! {
            _ = tokio::time::sleep_until(deadline.into()) => return,
            _ = &mut wake => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn usage(five: Option<u32>, week: Option<u32>) -> CodexUsage {
        CodexUsage {
            five_hour_remaining_percent: five,
            week_remaining_percent: week,
            five_hour_burn_rate_per_hour: None,
            week_burn_rate_per_hour: None,
            credits_display: "—".into(),
            credits_balance: None,
            has_credits: false,
            five_hour_resets_at: None,
            week_resets_at: None,
            fetched_at: 1,
            antigravity: Default::default(),
        }
    }
    fn pref(provider: crate::config::Provider) -> ProviderPreference {
        ProviderPreference {
            protection_enabled: true,
            ..ProviderPreference::default_for(provider)
        }
    }
    #[test]
    fn stale_deadline_is_the_next_freshness_window_to_expire() {
        assert_eq!(seconds_until_stale(1000, 1000, 60, 15), Some(61));
        assert_eq!(seconds_until_stale(1061, 1000, 60, 15), Some(60));
        assert_eq!(seconds_until_stale(1120, 1000, 60, 60), Some(1));
        assert_eq!(seconds_until_stale(1121, 1000, 60, 60), None);
        assert_eq!(seconds_until_stale(1000, 1000, 300, 300), Some(601));
    }
    #[test]
    fn near_boundaries_and_two_sample_recovery_control_cadence() {
        let now = Instant::now();
        let mut poll = PollState::default();
        let p = pref(crate::config::Provider::Codex);
        poll.started(now);
        poll.completed(&p, Some(&usage(Some(11), Some(8))), now);
        assert_eq!(poll.effective_interval_seconds, 60);
        poll.completed(&p, Some(&usage(Some(10), Some(8))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
        poll.completed(&p, Some(&usage(Some(13), Some(10))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
        poll.completed(&p, Some(&usage(Some(13), None)), now);
        poll.completed(&p, Some(&usage(Some(13), Some(10))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
        poll.completed(&p, Some(&usage(Some(13), Some(10))), now);
        assert_eq!(poll.effective_interval_seconds, 60);
        poll.completed(&p, Some(&usage(Some(90), Some(7))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
    }
    #[test]
    fn dsh_cadence_uses_its_own_thresholds_not_local_codex_defaults() {
        let now = Instant::now();
        let mut p = pref(crate::config::Provider::Dsh);
        let mut poll = PollState::default();
        poll.completed(&p, Some(&usage(Some(9), Some(9))), now);
        assert_eq!(poll.effective_interval_seconds, 60, "DSH 0% thresholds ignore local 5%/2%");
        poll.completed(&p, Some(&usage(Some(5), Some(9))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
        p.five_hour_threshold_percent = 10;
        poll.completed(&p, Some(&usage(Some(15), Some(50))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
    }
    #[test]
    fn errors_back_off_and_long_reads_do_not_catch_up() {
        let now = Instant::now();
        let mut poll = PollState::default();
        let p = pref(crate::config::Provider::Codex);
        poll.started(now);
        poll.completed(
            &p,
            Some(&usage(Some(5), Some(50))),
            now + Duration::from_secs(20),
        );
        assert_eq!(poll.deadline(now), now + Duration::from_secs(25));
        for expected in [30, 60, 120, 240, 300, 300] {
            poll.completed(&p, None, now);
            assert_eq!(poll.effective_interval_seconds, expected);
        }
        poll.completed(&p, Some(&usage(Some(5), Some(50))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
    }
    #[test]
    fn transient_recovery_starts_after_completion_and_does_not_reset_on_settings_changes() {
        let now = Instant::now();
        let mut poll = PollState::default();
        let p = pref(crate::config::Provider::Codex);
        for expected in [15, 30, 60, 120, 240, 300, 300] {
            poll.started(now);
            let completed = now + Duration::from_secs(45);
            poll.completed_with_recovery(&p, None, completed, ReadRecovery { transient: true, ..Default::default() });
            assert_eq!(poll.effective_interval_seconds, expected);
            assert_eq!(poll.deadline(completed), completed + Duration::from_secs(expected));
            poll.recalculate(&p, completed);
            assert_eq!(poll.effective_interval_seconds, expected);
            assert_eq!(poll.healthy_interval_seconds(&p), 60);
        }
        poll.completed(&p, Some(&usage(Some(80), Some(80))), now);
        assert_eq!(poll.effective_interval_seconds, 60);
    }

    #[test]
    fn server_delay_and_display_only_recovery_are_respected() {
        let now = Instant::now();
        let mut poll = PollState::default();
        let p = pref(crate::config::Provider::Codex);
        poll.started(now);
        poll.completed_with_recovery(&p, None, now, ReadRecovery { transient: true, retry_after_seconds: Some(90), display_only: false });
        poll.request_now(now + Duration::from_secs(1));
        assert_eq!(poll.deadline(now), now + Duration::from_secs(90));
        assert_eq!(poll.server_wait(now), Some(Duration::from_secs(90)));
        let received = now + Duration::from_secs(100);
        poll.completed_with_recovery(&p, Some(&usage(Some(70), Some(80))), received, ReadRecovery { display_only: true, ..Default::default() });
        assert_eq!(poll.deadline(received), received + Duration::from_secs(15));
        assert_eq!(poll.server_wait(received), None);
    }

    #[test]
    fn disabling_and_immediate_wake_replace_the_old_deadline() {
        let now = Instant::now();
        let mut poll = PollState::default();
        let mut p = pref(crate::config::Provider::Codex);
        poll.started(now);
        poll.completed(&p, Some(&usage(Some(0), Some(0))), now);
        p.protection_enabled = false;
        p.refresh_interval_seconds = 300;
        poll.recalculate(&p, now);
        assert_eq!(poll.effective_interval_seconds, 300);
        assert_eq!(poll.deadline(now), now + Duration::from_secs(300));
        poll.request_now(now);
        assert_eq!(poll.deadline(now), now);
    }

    #[tokio::test]
    async fn a_settings_wakeup_interrupts_an_old_sleep_without_waiting_for_its_deadline() {
        let dir = std::env::temp_dir().join(format!("overlay-poll-wakeup-{}", std::process::id()));
        let owner = crate::commands::AppState::for_test(
            crate::config::ConfigManager::with_runtime_dir(dir.clone()),
        );
        let state = owner.monitors.get(crate::config::Provider::Codex);
        {
            let mut polling = state.polling.lock().unwrap();
            polling.started(Instant::now());
            let p = ProviderPreference {
                refresh_interval_seconds: 300,
                ..ProviderPreference::default_for(crate::config::Provider::Codex)
            };
            polling.recalculate(&p, Instant::now());
        }
        let waiter = {
            let state = state.clone();
            tokio::spawn(async move {
                wait_until_due(&state).await;
            })
        };
        tokio::task::yield_now().await;
        state.polling.lock().unwrap().request_now(Instant::now());
        state.poll_wake.notify_one();
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .unwrap()
            .unwrap();
        let _ = std::fs::remove_dir(dir);
    }
}
