//! The shadow lifecycle tests.
//!
//! They drive the real tracker with a real 5H series, so the causal
//! ordering, the frozen rejection rules, the state migration and the audit
//! outbox are exercised exactly as the application exercises them.
//!
//! The harness mirrors the real refresh path: the sample is accepted, the
//! state is persisted and the audit outbox is flushed. Without the flush the
//! personal transitions stay paused on purpose, so the tests would not see
//! the behaviour the application has.

use super::shadow::ShadowEventKind;
use super::*;
use crate::codex::RawCodexUsage;
use crate::config::ConfigManager;

const RESET_AT: i64 = 1_000_000;

fn quota(used: f64) -> QuotaSnapshot {
    QuotaSnapshot {
        used_percent: Some(used),
        resets_at: Some(RESET_AT),
        window_duration_mins: None,
    }
}

fn usage(at: i64, five: f64, week: f64) -> RawCodexUsage {
    RawCodexUsage {
        account_id: Some("acct-a".into()),
        five_hour: quota(five),
        weekly: quota(week),
        credits_display: "-".into(),
        credits_balance: None,
        has_credits: false,
        fetched_at: at,
    }
}

/// A tracker with a runtime directory, so a refresh can persist and flush
/// exactly like the application does.
struct Harness {
    tracker: BurnRateTracker,
    config: ConfigManager,
    dir: std::path::PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Harness {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("codex-shadow-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config = ConfigManager::with_runtime_dir(dir.clone());
        Self {
            tracker: BurnRateTracker::from_state(BurnRateStateFile::default()),
            config,
            dir,
        }
    }

    /// One refresh, the way the application performs it: the sample is
    /// accepted, the state is persisted and the audit outbox is flushed.
    ///
    /// A failing write is not fatal, exactly like in the application.
    fn refresh(&mut self, sample: &RawCodexUsage) -> BurnRateEstimate {
        let estimate = self.tracker.accept_usage(sample);
        let _ = self.persist();
        let _ = self.tracker.flush_audit_outbox(&self.config);
        estimate
    }

    /// Accepts the sample and persists the state, but keeps the audit outbox
    /// occupied. The personal transitions stay paused while it is.
    fn accept(&mut self, sample: &RawCodexUsage) -> BurnRateEstimate {
        let estimate = self.tracker.accept_usage(sample);
        let _ = self.persist();
        estimate
    }

    fn persist(&mut self) -> Result<(), String> {
        if self.tracker.is_dirty() {
            self.tracker.save(&self.config)
        } else {
            Ok(())
        }
    }

    /// A tracker that burned to the first eligible origin at t = 1180.
    ///
    /// The series burns one quota point per minute, so the Fixed Weighted
    /// estimator settles at exactly 60 points per hour and the five-minute
    /// feedback can mature.
    fn origin(tag: &str) -> Self {
        let mut harness = Self::new(tag);
        harness.refresh(&usage(1000, 10.0, 10.0));
        harness.refresh(&usage(1060, 11.0, 10.0));
        harness.refresh(&usage(1120, 12.0, 10.0));
        harness.refresh(&usage(1180, 13.0, 10.0));
        let pending = harness
            .tracker
            .state
            .five_hour_shadow
            .pending
            .as_ref()
            .expect("the third crossing creates the first eligible origin");
        assert_eq!(pending.origin_at, 1180);
        assert!((pending.origin_fixed_rate_per_hour - 60.0).abs() < 1e-9);
        harness
    }

    /// Advances the series by one minute and one quota point.
    fn step(&mut self, at: i64, used: f64) {
        self.refresh(&usage(at, used, 10.0));
    }

    fn shadow(&self) -> &FiveHourShadowState {
        &self.tracker.state.five_hour_shadow
    }

    fn personal(&self) -> &PersonalCalibrationState {
        &self.tracker.state.five_hour_shadow.personal
    }

    /// The records of the local evidence log.
    fn events(&self) -> Vec<ShadowEvidenceEvent> {
        let Ok(content) = fs::read_to_string(self.config.shadow_evidence_path()) else {
            return Vec::new();
        };
        content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn last_event(&self) -> Option<ShadowEvidenceEvent> {
        self.events().pop()
    }
}

#[test]
fn no_pending_forecast_while_the_fixed_weighted_rate_is_unavailable() {
    let mut harness = Harness::new("no-fit");
    harness.refresh(&usage(1000, 10.0, 10.0));
    harness.refresh(&usage(1060, 11.0, 10.0));
    harness.refresh(&usage(1120, 12.0, 10.0));
    // Two crossings are not a span of two levels.
    assert!(harness.shadow().pending.is_none());
    assert_eq!(harness.tracker.shadow_estimate().fixed_weighted_per_hour, None);
    assert_eq!(
        harness.tracker.shadow_estimate().personalized_per_hour,
        None
    );
}

#[test]
fn the_first_eligible_origin_creates_exactly_one_pending_interval() {
    let mut harness = Harness::origin("first-origin");
    let pending = harness.shadow().pending.as_ref().unwrap();
    assert_eq!(pending.origin_at, 1180);
    assert_eq!(pending.origin_used_percent, 13.0);
    assert_eq!(pending.origin_reset_at, RESET_AT);
    assert_eq!(pending.last_valid_at, 1180);
    // Only one interval exists at a time.
    harness.step(1240, 14.0);
    let pending = harness.shadow().pending.as_ref().unwrap();
    assert_eq!(pending.origin_at, 1180);
    assert_eq!(pending.last_valid_at, 1240);
}

#[test]
fn a_refresh_before_the_horizon_cannot_train() {
    let mut harness = Harness::origin("before-horizon");
    for minute in 1..=4 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 0);
    assert_eq!(learner.informative_updates, 0);
    assert_eq!(learner.a, 10.0);
    assert_eq!(learner.b, 10.0);
    assert_eq!(learner.evidence, 0.0);
    assert!(harness.events().is_empty());
}

#[test]
fn the_first_valid_target_matures_and_trains_the_learner() {
    let mut harness = Harness::origin("mature");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    // The target is at 1480, exactly 300 seconds after the origin.
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 1);
    assert_eq!(learner.informative_updates, 1);
    // p = 60 points per hour for 300 seconds is 5 points, and 5 were observed.
    assert!((learner.a - 34.9).abs() < 1e-9);
    assert!((learner.b - 34.9).abs() < 1e-9);
    assert!((learner.evidence - 1.0).abs() < 1e-12);
    assert_eq!(learner.last_update_at, Some(1480));

    let event = harness.last_event().expect("the matured outcome is recorded");
    assert_eq!(event.kind, ShadowEventKind::Matured);
    assert_eq!(event.origin_at, Some(1180));
    assert_eq!(event.target_at, Some(1480));
    assert_eq!(event.elapsed_seconds, Some(300));
    assert_eq!(event.observed_delta, Some(5.0));
    assert_eq!(event.fixed_predicted_delta, Some(5.0));
    assert_eq!(event.q, Some(1.0));
    assert_eq!(event.matured_feedbacks_after, Some(1));
    assert_eq!(event.informative_updates_after, Some(1));
    assert_eq!(event.account_generation, 0);
    assert_eq!(event.event_sequence, 1);
}

#[test]
fn the_matured_update_precedes_the_same_time_shadow_estimate() {
    let mut harness = Harness::origin("estimate-order");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    let estimate = harness.tracker.shadow_estimate();
    assert!((estimate.raw_factor.unwrap() - 1.0).abs() < 1e-9);
    assert!((estimate.trust - 1.0 / 9.0).abs() < 1e-12);
    assert!((estimate.effective_factor - 1.0).abs() < 1e-9);
    assert!((estimate.evidence - 1.0).abs() < 1e-12);
    assert_eq!(estimate.matured_feedbacks, 1);
    assert_eq!(estimate.informative_updates, 1);
    assert!((estimate.fixed_weighted_per_hour.unwrap() - 60.0).abs() < 1e-9);
}

#[test]
fn a_mature_target_immediately_becomes_the_next_origin() {
    let mut harness = Harness::origin("next-origin");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    let pending = harness.shadow().pending.as_ref().unwrap();
    assert_eq!(pending.origin_at, 1480);
    assert_eq!(pending.origin_used_percent, 18.0);
    // The new origin carries the post-update learner state.
    assert!((pending.origin_trust - 1.0 / 9.0).abs() < 1e-12);
    assert!((pending.origin_evidence - 1.0).abs() < 1e-12);
    assert_eq!(pending.origin_informative_updates, 1);
    assert!((pending.origin_raw_factor - 1.0).abs() < 1e-9);
}

#[test]
fn the_intervals_never_overlap() {
    let mut harness = Harness::origin("no-overlap");
    let mut used = 13.0;
    for round in 0..2 {
        for minute in 1..=5 {
            used += 1.0;
            harness.step(1180 + round * 300 + minute * 60, used);
        }
        let pending = harness.shadow().pending.as_ref().unwrap();
        assert_eq!(pending.origin_at, 1180 + round * 300 + 300);
    }
    assert_eq!(harness.personal().matured_feedbacks, 2);
}

#[test]
fn a_gap_above_the_frozen_limit_rejects_without_learning() {
    let mut harness = Harness::origin("gap");
    // The gap is 120 seconds, above the frozen 90 second limit.
    harness.step(1300, 16.0);
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 0);
    assert_eq!(learner.informative_updates, 0);
    assert_eq!(learner.a, 10.0);
    let event = harness.last_event().expect("the rejection is recorded");
    assert_eq!(event.kind, ShadowEventKind::Rejected);
    assert_eq!(event.rejection_reason.as_deref(), Some("gap"));
    assert_eq!(event.actual_gap_seconds, Some(120));
}

#[test]
fn thirty_second_polling_matures_normally() {
    let mut harness = Harness::origin("cadence-30");
    let mut at = 1180;
    let mut used = 13.0;
    while at < 1480 {
        at += 30;
        used += 0.5;
        harness.step(at, used);
    }
    assert_eq!(harness.personal().matured_feedbacks, 1);
    assert_eq!(harness.personal().informative_updates, 1);
}

#[test]
fn sixty_second_polling_with_bounded_jitter_matures_normally() {
    let mut harness = Harness::origin("cadence-60-jitter");
    // Every gap stays below the frozen 90 second limit.
    for step_index in 1..=5 {
        harness.step(1180 + step_index * 75, 13.0 + step_index as f64 * 1.25);
    }
    assert_eq!(harness.personal().matured_feedbacks, 1);
    assert_eq!(harness.personal().informative_updates, 1);
}

#[test]
fn a_hundred_and_twenty_second_polling_never_trains() {
    let mut harness = Harness::origin("cadence-120");
    for step_index in 1..=5 {
        harness.step(1180 + step_index * 120, 13.0 + step_index as f64);
    }
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 0);
    assert_eq!(learner.informative_updates, 0);
    assert_eq!(learner.a, 10.0);
    let event = harness.last_event().expect("the rejection is recorded");
    assert_eq!(event.rejection_reason.as_deref(), Some("gap"));
}

#[test]
fn three_hundred_second_polling_never_trains() {
    let mut harness = Harness::origin("cadence-300");
    for step_index in 1..=3 {
        harness.step(1180 + step_index * 300, 13.0 + step_index as f64);
    }
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 0);
    assert_eq!(learner.informative_updates, 0);
    assert_eq!(learner.a, 10.0);
}

#[test]
fn returning_to_a_fast_cadence_starts_fresh_intervals_without_resetting_the_learner() {
    let mut harness = Harness::origin("cadence-return");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    assert_eq!(harness.personal().matured_feedbacks, 1);
    let learned = harness.personal().clone();

    // A slow cadence rejects the interval without touching the learner.
    harness.step(1780, 24.0);
    assert_eq!(harness.personal().matured_feedbacks, 1);
    assert_eq!(harness.personal().a, learned.a);
    assert_eq!(harness.personal().evidence, learned.evidence);

    // A fast cadence starts a fresh interval and trains again.
    let mut at = 1780;
    let mut used = 24.0;
    while at < 2080 {
        at += 30;
        used += 0.5;
        harness.step(at, used);
    }
    assert_eq!(harness.personal().matured_feedbacks, 2);
    assert_eq!(harness.personal().informative_updates, 2);
}

#[test]
fn a_reset_change_rejects_the_interval_without_clearing_the_personal_state() {
    let mut harness = Harness::origin("reset-change");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    let learned = harness.personal().clone();
    assert_eq!(learned.matured_feedbacks, 1);

    // The new origin uses the same reset metadata.
    let origin_at = harness.shadow().pending.as_ref().unwrap().origin_at;
    let mut corrected = usage(origin_at + 30, 19.0, 10.0);
    corrected.five_hour.resets_at = Some(RESET_AT + 1_000_000);
    harness.refresh(&corrected);
    let event = harness.last_event().expect("the rejection is recorded");
    assert_eq!(event.kind, ShadowEventKind::Rejected);
    assert_eq!(event.rejection_reason.as_deref(), Some("reset_mismatch"));
    // The long lived learner state is untouched.
    assert_eq!(harness.personal().a, learned.a);
    assert_eq!(harness.personal().b, learned.b);
    assert_eq!(harness.personal().evidence, learned.evidence);
    assert_eq!(harness.personal().matured_feedbacks, 1);
    // The production tracker did not mistake it for a quota cycle.
    assert_eq!(harness.tracker.state.five_hour.segment_id, 0);
}

#[test]
fn a_confirmed_segment_transition_rejects_before_training() {
    let mut harness = Harness::origin("segment");
    // A gap beyond the production maximum starts a new quota segment.
    harness.step(1880, 20.0);
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 0);
    assert_eq!(learner.informative_updates, 0);
    assert_eq!(learner.a, 10.0);
    let event = harness.last_event().expect("the rejection is recorded");
    assert_eq!(event.kind, ShadowEventKind::Rejected);
    assert_eq!(event.rejection_reason.as_deref(), Some("transition"));
    // The weighted metadata of the old segment is gone.
    assert_eq!(harness.shadow().weighted.last_crossing_observation_at, None);
    assert_eq!(harness.shadow().weighted.idle_rate_ceiling, None);
}

#[test]
fn a_decrease_rejects_the_interval() {
    let mut harness = Harness::origin("decrease");
    // The used percentage drops below the last valid observation.
    harness.step(1240, 12.0);
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 0);
    assert_eq!(learner.a, 10.0);
    let event = harness.last_event().expect("the rejection is recorded");
    assert_eq!(event.rejection_reason.as_deref(), Some("decrease"));
}

#[test]
fn a_saturation_rejects_the_interval() {
    let mut harness = Harness::origin("saturation");
    harness.step(1240, 100.0);
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 0);
    assert_eq!(learner.a, 10.0);
    let event = harness.last_event().expect("the rejection is recorded");
    assert_eq!(event.rejection_reason.as_deref(), Some("saturation"));
}

#[test]
fn a_missing_sample_never_fabricates_an_outcome() {
    let mut harness = Harness::origin("missing");
    // The sample is unusable, so it must not advance the interval.
    let mut missing = usage(1240, 14.0, 10.0);
    missing.five_hour = QuotaSnapshot::default();
    missing.weekly = QuotaSnapshot::default();
    harness.refresh(&missing);
    let pending = harness.shadow().pending.as_ref().unwrap();
    assert_eq!(pending.last_valid_at, 1180);
    assert_eq!(harness.personal().matured_feedbacks, 0);
    assert!(harness.events().is_empty());

    // The next valid observation is compared with the last valid one, so the
    // 120 second gap rejects instead of silently bridging.
    harness.step(1300, 16.0);
    assert_eq!(harness.personal().matured_feedbacks, 0);
    let event = harness.last_event().expect("the rejection is recorded");
    assert_eq!(event.rejection_reason.as_deref(), Some("gap"));
}

#[test]
fn a_same_second_duplicate_creates_no_evidence() {
    let mut harness = Harness::origin("duplicate");
    let before = harness.shadow().clone();
    // The very same second, with the very same reading.
    harness.refresh(&usage(1180, 13.0, 10.0));
    let after = harness.shadow();
    assert_eq!(after.personal, before.personal);
    assert_eq!(after.pending, before.pending);
    assert_eq!(after.event_sequence, before.event_sequence);
    assert!(harness.events().is_empty());
}

#[test]
fn a_backward_timestamp_cannot_train() {
    let mut harness = Harness::origin("backwards");
    harness.step(1120, 12.0);
    let learner = harness.personal();
    assert_eq!(learner.matured_feedbacks, 0);
    assert_eq!(learner.a, 10.0);
    let event = harness.last_event().expect("the rejection is recorded");
    assert_eq!(event.rejection_reason.as_deref(), Some("backwards"));
}

#[test]
fn future_observations_cannot_change_an_earlier_shadow_prediction() {
    let mut harness = Harness::origin("no-look-ahead");
    let stored = harness.shadow().pending.clone().unwrap();
    for minute in 1..=4 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    let pending = harness.shadow().pending.as_ref().unwrap();
    assert_eq!(pending.origin_at, stored.origin_at);
    assert_eq!(pending.origin_used_percent, stored.origin_used_percent);
    assert_eq!(
        pending.origin_fixed_rate_per_hour,
        stored.origin_fixed_rate_per_hour
    );
    assert_eq!(
        pending.origin_shadow_rate_per_hour,
        stored.origin_shadow_rate_per_hour
    );
    assert_eq!(
        pending.origin_published_rate_per_hour,
        stored.origin_published_rate_per_hour
    );
}

#[test]
fn the_visible_usage_values_are_unchanged_by_the_shadow() {
    let mut harness = Harness::new("visible");
    let mut last: Option<f64> = None;
    for minute in 0..10 {
        let at = 1000 + minute * 60;
        let used = 10.0 + minute as f64;
        let estimate = harness.refresh(&usage(at, used, 10.0));
        // The production estimate is exactly the pre-shadow computation.
        let expected = estimate_rate(
            &harness.tracker.state.five_hour,
            FIVE_HOUR_HORIZON_SECS,
            2,
        );
        assert_eq!(estimate.five_hour_per_hour, expected);
        last = estimate.five_hour_per_hour;
    }
    // The frontend contract cannot carry a shadow value.
    let ui = usage(1600, 19.0, 10.0).to_ui(last, None);
    let json = serde_json::to_value(&ui).unwrap();
    let keys: Vec<String> = json
        .as_object()
        .unwrap()
        .keys()
        .map(|key| key.to_string())
        .collect();
    for forbidden in [
        "personalizedPerHour",
        "fixedWeightedPerHour",
        "rawFactor",
        "trust",
        "effectiveFactor",
        "shadow",
        "evidence",
    ] {
        assert!(!keys.contains(&forbidden.to_string()), "{forbidden} leaked");
    }
}

#[test]
fn a_successful_refresh_persists_the_shadow_state() {
    let harness = Harness::origin("persist");
    let raw = fs::read_to_string(harness.config.burn_rate_state_path()).unwrap();
    assert!(raw.contains("fiveHourShadow"));
    let loaded = BurnRateTracker::load(&harness.config);
    let pending = loaded.state.five_hour_shadow.pending.as_ref().unwrap();
    assert_eq!(pending.origin_at, 1180);
    assert!((pending.origin_fixed_rate_per_hour - 60.0).abs() < 1e-9);
}

#[test]
fn a_failed_usage_read_does_not_fabricate_feedback_or_erase_the_learner_state() {
    let mut harness = Harness::origin("failed-read");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    let before = fs::read_to_string(harness.config.burn_rate_state_path()).unwrap();

    // A failed read never reaches the tracker, so the state is untouched.
    let loaded = BurnRateTracker::load(&harness.config);
    assert_eq!(
        loaded.state.five_hour_shadow.personal,
        harness.tracker.state.five_hour_shadow.personal
    );
    assert_eq!(
        before,
        fs::read_to_string(harness.config.burn_rate_state_path()).unwrap()
    );
}

#[test]
fn an_account_switch_increments_the_generation_and_clears_the_personal_state() {
    let mut harness = Harness::origin("account-switch");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    assert_eq!(harness.personal().matured_feedbacks, 1);

    let mut switched = usage(1600, 20.0, 12.0);
    switched.account_id = Some("acct-b".into());
    harness.refresh(&switched);

    let shadow = harness.shadow();
    assert_eq!(shadow.account_generation, 1);
    assert_eq!(shadow.personal.matured_feedbacks, 0);
    assert_eq!(shadow.personal.informative_updates, 0);
    assert_eq!(shadow.personal.a, 10.0);
    assert_eq!(shadow.personal.b, 10.0);
    assert_eq!(shadow.personal.evidence, 0.0);
    assert!(shadow.pending.is_none());
    assert_eq!(shadow.weighted.last_crossing_observation_at, None);
    // The audit identity restarts with the new generation.
    assert_eq!(shadow.event_sequence, 1);
    let event = harness.last_event().expect("the lifecycle is recorded");
    assert_eq!(event.kind, ShadowEventKind::Lifecycle);
    assert_eq!(event.lifecycle.as_deref(), Some("account_switch"));
    assert_eq!(event.account_generation, 1);
    assert_eq!(event.event_sequence, 1);
}

#[test]
fn a_quota_reset_preserves_the_personal_state_but_clears_the_pending_interval() {
    let mut harness = Harness::new("quota-reset");
    // The reset boundary is close, so the cycle really completes later.
    let mut first = usage(1000, 10.0, 10.0);
    first.five_hour.resets_at = Some(2000);
    harness.refresh(&first);
    for minute in 1..=8 {
        let mut sample = usage(1000 + minute * 60, 10.0 + minute as f64, 10.0);
        sample.five_hour.resets_at = Some(2000);
        harness.refresh(&sample);
    }
    assert_eq!(harness.personal().matured_feedbacks, 1);
    let learned = harness.personal().clone();
    let segment_before = harness.tracker.state.five_hour.segment_id;

    // The boundary moves past the current time: the quota cycle completed.
    let mut after_reset = usage(2000, 3.0, 10.0);
    after_reset.five_hour.resets_at = Some(9000);
    harness.refresh(&after_reset);

    assert!(harness.tracker.state.five_hour.segment_id > segment_before);
    let shadow = harness.shadow();
    // The personal calibration survives a quota cycle.
    assert_eq!(shadow.personal.a, learned.a);
    assert_eq!(shadow.personal.b, learned.b);
    assert_eq!(shadow.personal.evidence, learned.evidence);
    assert_eq!(shadow.personal.matured_feedbacks, learned.matured_feedbacks);
    // The generation is not an account change.
    assert_eq!(shadow.account_generation, 0);
    // The pending interval and the weighted segment metadata are gone.
    assert!(shadow.pending.is_none());
    assert_eq!(shadow.weighted.last_crossing_observation_at, None);
}

#[test]
fn a_v1_state_migrates_to_v2_without_losing_history() {
    let legacy = serde_json::json!({
        "schemaVersion": 1,
        "accountId": "acct-a",
        "fiveHour": {
            "resetsAt": 2000, "segmentId": 3, "lastObservedPercent": 42.0,
            "lastObservedAt": 1000, "correctionCandidate": null,
            "crossings": [{"level": 11, "observedAt": 900, "segmentId": 3}]
        },
        "weekly": {
            "resetsAt": 9000, "segmentId": 1, "lastObservedPercent": 10.0,
            "lastObservedAt": 1000, "correctionCandidate": null, "crossings": []
        }
    });
    let state = load_state(&legacy.to_string());
    assert_eq!(state.schema_version, SCHEMA_VERSION);
    assert_eq!(state.account_id.as_deref(), Some("acct-a"));
    assert_eq!(state.five_hour.crossings.len(), 1);
    assert_eq!(state.five_hour.last_observed_percent, Some(42.0));
    assert_eq!(state.five_hour.last_observed_at, Some(1000));
    assert_eq!(state.five_hour.segment_id, 3);
    assert_eq!(state.weekly.last_observed_at, Some(1000));
    // The shadow subtree starts neutral.
    assert_eq!(state.five_hour_shadow, FiveHourShadowState::default());
}

#[test]
fn a_v2_state_round_trips() {
    let harness = Harness::origin("round-trip");
    let raw = fs::read_to_string(harness.config.burn_rate_state_path()).unwrap();
    let state = load_state(&raw);
    assert_eq!(state.schema_version, SCHEMA_VERSION);
    assert_eq!(state, harness.tracker.state);
}

#[test]
fn a_malformed_shadow_subtree_does_not_destroy_the_legacy_state() {
    let broken = serde_json::json!({
        "schemaVersion": 2,
        "accountId": "acct-a",
        "fiveHour": {
            "resetsAt": 2000, "segmentId": 3, "lastObservedPercent": 42.0,
            "lastObservedAt": 1000, "correctionCandidate": null,
            "crossings": [{"level": 11, "observedAt": 900, "segmentId": 3}]
        },
        "weekly": {
            "resetsAt": 9000, "segmentId": 1, "lastObservedPercent": 10.0,
            "lastObservedAt": 1000, "correctionCandidate": null, "crossings": []
        },
        "fiveHourShadow": {
            "algorithmVersion": 1, "accountGeneration": "not-a-number",
            "eventSequence": 0, "auditOutbox": null, "weighted": {}, "personal": {}
        }
    });
    let state = load_state(&broken.to_string());
    // The recoverable core survives, the shadow is reset.
    assert_eq!(state.five_hour.crossings.len(), 1);
    assert_eq!(state.five_hour.last_observed_percent, Some(42.0));
    assert_eq!(state.weekly.last_observed_at, Some(1000));
    assert_eq!(state.five_hour_shadow, FiveHourShadowState::default());
}

#[test]
fn an_algorithm_version_mismatch_preserves_the_history_and_the_audit_sequence() {
    let mut state = BurnRateStateFile::default();
    state.five_hour.crossings.push(CrossingEvent {
        level: 11,
        observed_at: 900,
        segment_id: 0,
    });
    state.five_hour.last_observed_at = Some(1000);
    state.five_hour_shadow.algorithm_version = SHADOW_ALGORITHM_VERSION + 1;
    state.five_hour_shadow.account_generation = 4;
    state.five_hour_shadow.event_sequence = 7;
    state.five_hour_shadow.personal.a = 50.0;
    state.five_hour_shadow.personal.evidence = 12.0;
    let raw = serde_json::to_string(&state).unwrap();

    let loaded = load_state(&raw);
    // The legacy history and the audit identity survive.
    assert_eq!(loaded.five_hour.crossings, state.five_hour.crossings);
    assert_eq!(loaded.five_hour_shadow.account_generation, 4);
    assert_eq!(loaded.five_hour_shadow.event_sequence, 7);
    // The incompatible learner state returns to neutral.
    assert_eq!(
        loaded.five_hour_shadow.algorithm_version,
        SHADOW_ALGORITHM_VERSION
    );
    assert_eq!(loaded.five_hour_shadow.personal.a, 10.0);
    assert_eq!(loaded.five_hour_shadow.personal.evidence, 0.0);
    assert!(loaded.five_hour_shadow.pending.is_none());
}

#[test]
fn a_short_restart_resumes_the_pending_interval_with_its_origin_snapshot() {
    let harness = Harness::origin("short-restart");
    let mut tracker = BurnRateTracker::load(&harness.config);
    let pending = tracker.state.five_hour_shadow.pending.as_ref().unwrap();
    assert_eq!(pending.origin_at, 1180);
    assert_eq!(pending.origin_used_percent, 13.0);
    assert!((pending.origin_fixed_rate_per_hour - 60.0).abs() < 1e-9);
    for minute in 1..=5 {
        tracker.accept_usage(&usage(1180 + minute * 60, 13.0 + minute as f64, 10.0));
    }
    assert_eq!(
        tracker.state.five_hour_shadow.personal.matured_feedbacks,
        1
    );
}

#[test]
fn a_long_restart_gap_rejects_the_pending_but_preserves_the_learner() {
    let mut harness = Harness::origin("long-restart");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    let learned = harness.personal().clone();
    let mut tracker = BurnRateTracker::load(&harness.config);
    // The application was asleep for far longer than the feedback limit, but
    // the production history survived it.
    tracker.accept_usage(&usage(1700, 24.0, 10.0));
    assert_eq!(
        tracker.state.five_hour_shadow.personal.matured_feedbacks,
        learned.matured_feedbacks
    );
    assert_eq!(tracker.state.five_hour_shadow.personal.a, learned.a);
    assert_eq!(tracker.state.five_hour_shadow.personal.b, learned.b);
    assert_eq!(
        tracker.state.five_hour_shadow.personal.evidence,
        learned.evidence
    );
    let event = tracker.state.five_hour_shadow.audit_outbox.as_ref().unwrap();
    assert_eq!(event.rejection_reason.as_deref(), Some("gap"));
}

#[test]
fn a_persisted_audit_outbox_is_retried_after_a_restart() {
    let mut harness = Harness::origin("outbox-retry");
    // The event is recorded but never flushed, so the slot stays occupied.
    for minute in 1..=5 {
        harness.accept(&usage(1180 + minute * 60, 13.0 + minute as f64, 10.0));
    }
    assert!(harness.shadow().audit_outbox.is_some());
    harness.persist().unwrap();
    // The event never reached the log yet.
    assert!(!harness.config.shadow_evidence_path().exists());

    // The startup flushes the retained event.
    let tracker = BurnRateTracker::load(&harness.config);
    let log = fs::read_to_string(harness.config.shadow_evidence_path()).unwrap();
    let lines: Vec<&str> = log.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);
    let event: ShadowEvidenceEvent = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(event.kind, ShadowEventKind::Matured);
    assert_eq!(event.target_at, Some(1480));
    assert!(tracker.state.five_hour_shadow.audit_outbox.is_none());
}

#[test]
fn a_duplicate_outbox_append_is_harmless() {
    let mut harness = Harness::origin("outbox-duplicate");
    for minute in 1..=5 {
        harness.accept(&usage(1180 + minute * 60, 13.0 + minute as f64, 10.0));
    }
    // The outbox is kept, so the event is only persisted, not appended.
    harness.persist().unwrap();

    // The startup flushes the retained event.
    let first = BurnRateTracker::load(&harness.config);
    let log = fs::read_to_string(harness.config.shadow_evidence_path()).unwrap();
    assert_eq!(
        log.lines().filter(|line| !line.trim().is_empty()).count(),
        1
    );
    assert!(first.state.five_hour_shadow.audit_outbox.is_none());

    // The persisted slot still holds the event, so a later start appends it
    // again. The audit key makes the duplicate harmless.
    let mut second = BurnRateTracker::load(&harness.config);
    second.flush_audit_outbox(&harness.config).unwrap();
    let log = fs::read_to_string(harness.config.shadow_evidence_path()).unwrap();
    let lines: Vec<&str> = log.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2);
    let first: ShadowEvidenceEvent = serde_json::from_str(lines[0]).unwrap();
    let second: ShadowEvidenceEvent = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first.account_generation, second.account_generation);
    assert_eq!(first.event_sequence, second.event_sequence);
}

#[test]
fn an_unflushed_outbox_is_never_overwritten_by_a_second_personal_outcome() {
    let mut harness = Harness::origin("outbox-paused");
    // The event is recorded but never flushed, so the slot stays occupied.
    for minute in 1..=5 {
        harness.accept(&usage(1180 + minute * 60, 13.0 + minute as f64, 10.0));
    }
    let first = harness.shadow().audit_outbox.clone().unwrap();
    assert_eq!(first.kind, ShadowEventKind::Matured);

    // The audit slot is occupied, so the personal transitions are paused.
    for minute in 6..=12 {
        harness.accept(&usage(1180 + minute * 60, 13.0 + minute as f64, 10.0));
    }
    let outbox = harness.shadow().audit_outbox.as_ref().unwrap();
    assert_eq!(outbox.kind, ShadowEventKind::Matured);
    assert_eq!(outbox.target_at, first.target_at);
    assert_eq!(outbox.event_sequence, first.event_sequence);
    // The paused interval never trained the learner a second time.
    assert_eq!(harness.personal().matured_feedbacks, 1);
}

#[test]
fn a_state_save_failure_does_not_append_the_event_and_pauses_the_personal_transitions() {
    let mut harness = Harness::origin("save-failure");
    for minute in 1..=5 {
        harness.accept(&usage(1180 + minute * 60, 13.0 + minute as f64, 10.0));
    }
    // The state path is a directory, so the atomic rename can never succeed.
    let _ = fs::remove_file(harness.config.burn_rate_state_path());
    fs::create_dir_all(harness.config.burn_rate_state_path()).unwrap();

    harness.accept(&usage(1540, 19.0, 10.0));
    assert!(harness.persist().is_err());
    // The event is not durable, so it must not reach the evidence log.
    assert!(harness.tracker.flush_audit_outbox(&harness.config).is_err());
    assert!(!harness.config.shadow_evidence_path().exists());
    assert!(harness.shadow().audit_outbox.is_some());

    // The personal transitions stay paused while the state is not durable.
    let matured = harness.personal().matured_feedbacks;
    for minute in 6..=12 {
        harness.accept(&usage(1180 + minute * 60, 13.0 + minute as f64, 10.0));
    }
    assert_eq!(harness.personal().matured_feedbacks, matured);
}

#[test]
fn an_evidence_log_append_failure_leaves_the_outbox_retryable() {
    let mut harness = Harness::origin("log-failure");
    for minute in 1..=5 {
        harness.accept(&usage(1180 + minute * 60, 13.0 + minute as f64, 10.0));
    }
    // The evidence path is a directory, so the append can never succeed.
    let _ = fs::remove_file(harness.config.shadow_evidence_path());
    fs::create_dir_all(harness.config.shadow_evidence_path()).unwrap();

    harness.accept(&usage(1540, 19.0, 10.0));
    harness.persist().unwrap();
    assert!(harness.tracker.flush_audit_outbox(&harness.config).is_err());
    // The event is still in the slot, so a later refresh can retry it.
    assert!(harness.shadow().audit_outbox.is_some());
    // The visible production behavior is unaffected.
    let estimate = harness.tracker.accept_usage(&usage(1600, 20.0, 10.0));
    assert!(estimate.five_hour_per_hour.is_some());
}

#[test]
fn the_evidence_log_is_bounded() {
    let dir = std::env::temp_dir().join(format!("codex-shadow-bounded-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("evidence.jsonl");
    let mut content = String::new();
    for index in 0..=MAX_EVIDENCE_EVENTS {
        content.push_str(&format!("{{\"index\":{index}}}\n"));
    }
    fs::write(&path, content.as_bytes()).unwrap();
    let retained = shadow::compact_evidence_log(&path).unwrap();
    assert_eq!(retained, MAX_EVIDENCE_EVENTS);
    let lines = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = lines.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(lines.len(), MAX_EVIDENCE_EVENTS);
    // The newest events are the ones that survive.
    assert!(lines[0].contains(&format!("\"index\":{}", 1)));
    assert!(lines[MAX_EVIDENCE_EVENTS - 1].contains(&format!("\"index\":{}", MAX_EVIDENCE_EVENTS)));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_evidence_log_contains_no_account_identifier() {
    let mut harness = Harness::origin("no-account");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    let log = fs::read_to_string(harness.config.shadow_evidence_path()).unwrap();
    assert!(!log.contains("acct-a"));
    assert!(!log.contains("accountId"));
}
