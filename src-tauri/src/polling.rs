//! One polling policy for background, manual and reset-triggered reads.
use crate::{codex::CodexUsage, config::OverlaySettings};
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
}

impl PollState {
    pub fn started(&mut self, now: Instant) {
        self.last_started = Some(now);
    }

    pub fn completed(
        &mut self,
        settings: &OverlaySettings,
        usage: Option<&CodexUsage>,
        now: Instant,
    ) {
        self.last_completed = Some(now);
        if let Some(usage) = usage {
            self.failures = 0;
            if !settings.auto_pause_on_low_quota {
                self.fast = false;
                self.recovery_samples = 0;
            } else {
                let near = usage.five_hour_remaining_percent.is_some_and(|v| {
                    v <= settings
                        .auto_pause_five_hour_threshold_percent
                        .saturating_add(5)
                }) || usage.week_remaining_percent.is_some_and(|v| {
                    v <= settings
                        .auto_pause_weekly_threshold_percent
                        .saturating_add(5)
                });
                let recovered = usage.five_hour_remaining_percent.is_some_and(|v| {
                    v > settings
                        .auto_pause_five_hour_threshold_percent
                        .saturating_add(7)
                }) && usage.week_remaining_percent.is_some_and(|v| {
                    v > settings
                        .auto_pause_weekly_threshold_percent
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
        self.recalculate(settings, now);
    }

    pub fn recalculate(&mut self, settings: &OverlaySettings, now: Instant) {
        let normal = u64::from(settings.refresh_interval_seconds.clamp(15, 3600));
        if !settings.auto_pause_on_low_quota {
            self.fast = false;
        }
        let base = if settings.auto_pause_on_low_quota && self.fast {
            15
        } else {
            normal
        };
        self.effective_interval_seconds = base
            .saturating_mul(1u64 << self.failures.min(10))
            .min(normal.max(300));
        let start = self.last_started.unwrap_or(now);
        let floor = self
            .last_completed
            .map(|at| at + Duration::from_secs(5))
            .unwrap_or(now);
        self.deadline =
            Some((start + Duration::from_secs(self.effective_interval_seconds)).max(floor));
    }

    pub fn request_now(&mut self, now: Instant) {
        self.deadline = Some(now);
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

pub async fn wait_until_due(state: &crate::commands::AppState) {
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
        }
    }
    fn settings() -> OverlaySettings {
        OverlaySettings {
            auto_pause_on_low_quota: true,
            ..Default::default()
        }
    }
    #[test]
    fn near_boundaries_and_two_sample_recovery_control_cadence() {
        let now = Instant::now();
        let mut poll = PollState::default();
        let settings = settings();
        poll.started(now);
        poll.completed(&settings, Some(&usage(Some(11), Some(8))), now);
        assert_eq!(poll.effective_interval_seconds, 60);
        poll.completed(&settings, Some(&usage(Some(10), Some(8))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
        poll.completed(&settings, Some(&usage(Some(13), Some(10))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
        poll.completed(&settings, Some(&usage(Some(13), None)), now);
        poll.completed(&settings, Some(&usage(Some(13), Some(10))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
        poll.completed(&settings, Some(&usage(Some(13), Some(10))), now);
        assert_eq!(poll.effective_interval_seconds, 60);
        poll.completed(&settings, Some(&usage(Some(90), Some(7))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
    }
    #[test]
    fn errors_back_off_and_long_reads_do_not_catch_up() {
        let now = Instant::now();
        let mut poll = PollState::default();
        let settings = settings();
        poll.started(now);
        poll.completed(
            &settings,
            Some(&usage(Some(5), Some(50))),
            now + Duration::from_secs(20),
        );
        assert_eq!(poll.deadline(now), now + Duration::from_secs(25));
        for expected in [30, 60, 120, 240, 300, 300] {
            poll.completed(&settings, None, now);
            assert_eq!(poll.effective_interval_seconds, expected);
        }
        poll.completed(&settings, Some(&usage(Some(5), Some(50))), now);
        assert_eq!(poll.effective_interval_seconds, 15);
    }
    #[test]
    fn disabling_and_immediate_wake_replace_the_old_deadline() {
        let now = Instant::now();
        let mut poll = PollState::default();
        let mut settings = settings();
        poll.started(now);
        poll.completed(&settings, Some(&usage(Some(0), Some(0))), now);
        settings.auto_pause_on_low_quota = false;
        settings.refresh_interval_seconds = 300;
        poll.recalculate(&settings, now);
        assert_eq!(poll.effective_interval_seconds, 300);
        assert_eq!(poll.deadline(now), now + Duration::from_secs(300));
        poll.request_now(now);
        assert_eq!(poll.deadline(now), now);
    }

    #[tokio::test]
    async fn a_settings_wakeup_interrupts_an_old_sleep_without_waiting_for_its_deadline() {
        let dir = std::env::temp_dir().join(format!("overlay-poll-wakeup-{}", std::process::id()));
        let state = std::sync::Arc::new(crate::commands::AppState::for_test(
            crate::config::ConfigManager::with_runtime_dir(dir.clone()),
        ));
        {
            let mut polling = state.polling.lock().unwrap();
            polling.started(Instant::now());
            let settings = OverlaySettings {
                refresh_interval_seconds: 300,
                ..Default::default()
            };
            polling.recalculate(&settings, Instant::now());
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
