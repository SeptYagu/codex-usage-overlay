//! The personal calibration learner and the causal five-minute feedback engine.
//!
//! The formulas are frozen: the learner only learns a multiplicative bias and
//! the trust scalar only attenuates it. The global Fixed Weighted estimator
//! stays the permanent anchor, and a new installation starts exactly at it.

use serde::{Deserialize, Serialize};

use super::EPSILON;

/// The decay of the learner and of the trust evidence.
pub const LEARNER_LAMBDA: f64 = 0.99;
/// The prior strength of the learner.
pub const LEARNER_PRIOR: f64 = 10.0;
/// The trust constant: the evidence at which the trust reaches one half.
pub const TRUST_K: f64 = 8.0;
/// The lower bound of the raw personal factor.
pub const RAW_FACTOR_MIN: f64 = 0.75;
/// The upper bound of the raw personal factor.
pub const RAW_FACTOR_MAX: f64 = 1.50;
/// The target horizon of one learning interval, in seconds.
pub const FEEDBACK_HORIZON_SECS: i64 = 300;
/// The latest arrival that may still mature an interval, in seconds.
pub const FEEDBACK_MAX_ELAPSED_SECS: i64 = 390;
/// The largest accepted-observation gap inside one interval, in seconds.
pub const FEEDBACK_MAX_GAP_SECS: i64 = 90;

/// Why a pending interval was rejected.
///
/// The codes are stable: they are written into the local evidence log and are
/// read by later offline analysis. The three segment causes are separate
/// codes, because a later analysis has to tell a real quota cycle, an
/// overnight resume and a confirmed correction apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectionReason {
    /// The accepted-observation gap exceeded the frozen limit.
    Gap,
    /// The reset timestamp differed from the origin.
    ResetMismatch,
    /// The used percentage decreased.
    Decrease,
    /// The used percentage reached saturation.
    Saturation,
    /// The target arrived too late.
    TooLate,
    /// The timestamps moved backwards.
    Backwards,
    /// The quota cycle completed and started a new segment.
    QuotaCycle,
    /// The observation gap exceeded the production maximum and started a new
    /// segment.
    LongIdle,
    /// A decrease was confirmed by a later observation and started a new
    /// segment.
    Correction,
}

impl RejectionReason {
    /// The stable evidence-log code of the reason.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Gap => "gap",
            Self::ResetMismatch => "reset_mismatch",
            Self::Decrease => "decrease",
            Self::Saturation => "saturation",
            Self::TooLate => "too_late",
            Self::Backwards => "backwards",
            Self::QuotaCycle => "quota_cycle",
            Self::LongIdle => "long_idle",
            Self::Correction => "correction",
        }
    }
}

/// The persisted personal calibration state.
///
/// `matured_feedbacks` counts every matured interval, including the
/// low-information ones. `informative_updates` counts only the intervals
/// that carried information. Neither counter determines the trust; only the
/// decayed `evidence` does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PersonalCalibrationState {
    /// The numerator of the raw personal factor.
    pub a: f64,
    /// The denominator of the raw personal factor.
    pub b: f64,
    /// The decayed trust evidence.
    pub evidence: f64,
    /// How many intervals matured, including the low-information ones.
    pub matured_feedbacks: u64,
    /// How many matured intervals carried information.
    pub informative_updates: u64,
    /// When the learner last changed.
    pub last_update_at: Option<i64>,
}

impl Default for PersonalCalibrationState {
    fn default() -> Self {
        Self {
            a: LEARNER_PRIOR,
            b: LEARNER_PRIOR,
            evidence: 0.0,
            matured_feedbacks: 0,
            informative_updates: 0,
            last_update_at: None,
        }
    }
}

impl PersonalCalibrationState {
    /// The clamped raw personal factor.
    pub fn raw_factor(&self) -> f64 {
        if self.b > 0.0 {
            (self.a / self.b).clamp(RAW_FACTOR_MIN, RAW_FACTOR_MAX)
        } else {
            1.0
        }
    }

    /// The trust of the current evidence.
    pub fn trust(&self) -> f64 {
        if self.evidence <= 0.0 {
            0.0
        } else {
            self.evidence / (self.evidence + TRUST_K)
        }
    }

    /// The factor that is applied to the global estimate.
    pub fn effective_factor(&self) -> f64 {
        1.0 + self.trust() * (self.raw_factor() - 1.0)
    }

    /// Applies one matured outcome.
    ///
    /// A low-information outcome still advances the decay clock but
    /// contributes no new evidence term.
    pub fn apply_outcome(&mut self, confidence: f64, predicted: f64, observed: f64, at: i64) {
        self.a = LEARNER_LAMBDA * self.a + confidence * predicted * observed;
        self.b = LEARNER_LAMBDA * self.b + confidence * predicted * predicted;
        self.evidence = LEARNER_LAMBDA * self.evidence + confidence;
        self.matured_feedbacks = self.matured_feedbacks.saturating_add(1);
        if confidence > 0.0 {
            self.informative_updates = self.informative_updates.saturating_add(1);
        }
        self.last_update_at = Some(at);
    }

    /// Repairs a corrupt learner state to the neutral defaults.
    pub fn normalize(&mut self) -> bool {
        let before = self.clone();
        if !self.a.is_finite() || self.a <= 0.0 {
            self.a = LEARNER_PRIOR;
        }
        if !self.b.is_finite() || self.b <= 0.0 {
            self.b = LEARNER_PRIOR;
        }
        if !self.evidence.is_finite() || self.evidence < 0.0 {
            self.evidence = 0.0;
        }
        if self.last_update_at.is_some_and(|at| at <= 0) {
            self.last_update_at = None;
        }
        before != *self
    }
}

/// The persisted origin snapshot of one pending learning interval.
///
/// The origin estimator snapshot is mandatory: the matured outcome scores the
/// predictions that were made at the origin, never a reconstruction from the
/// target time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPersonalForecast {
    /// The algorithm version that created the interval.
    pub algorithm_version: u32,
    /// When the interval started.
    pub origin_at: i64,
    /// The used percentage at the origin.
    pub origin_used_percent: f64,
    /// The reset timestamp at the origin.
    pub origin_reset_at: i64,
    /// The published rate at the origin.
    pub origin_published_rate_per_hour: Option<f64>,
    /// The Fixed Weighted rate at the origin.
    pub origin_fixed_rate_per_hour: f64,
    /// The raw personal factor at the origin.
    pub origin_raw_factor: f64,
    /// The trust at the origin.
    pub origin_trust: f64,
    /// The effective factor at the origin.
    pub origin_effective_factor: f64,
    /// The personalized rate at the origin.
    pub origin_shadow_rate_per_hour: Option<f64>,
    /// The evidence at the origin.
    pub origin_evidence: f64,
    /// The informative update count at the origin.
    pub origin_informative_updates: u64,
    /// The last valid observation that kept the interval alive.
    pub last_valid_at: i64,
    /// The used percentage of that observation.
    pub last_valid_used_percent: f64,
}

/// What one observation did to a pending interval.
#[derive(Clone, Debug, PartialEq)]
pub enum PendingOutcome {
    /// The interval is not ready yet and stays alive.
    Continue,
    /// A same-second refresh carries no new information.
    Duplicate,
    /// The interval matured.
    Matured(MaturedOutcome),
    /// The interval was rejected without learning.
    Rejected(RejectionReason),
}

/// A matured five-minute outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct MaturedOutcome {
    /// The elapsed time between the origin and the target.
    pub elapsed_seconds: i64,
    /// The Fixed Weighted prediction the learner trains on.
    pub predicted_delta: f64,
    /// The observed quota-point increase.
    pub observed_delta: f64,
    /// The frozen feedback confidence.
    pub confidence: f64,
    /// The published prediction made at the origin.
    pub published_predicted_delta: Option<f64>,
    /// The Fixed Weighted prediction made at the origin.
    pub fixed_predicted_delta: f64,
    /// The personalized prediction made at the origin.
    pub shadow_predicted_delta: Option<f64>,
}

/// Caps a predicted delta by the quota that was still available at the origin.
fn cap_delta(delta: f64, remaining: f64) -> f64 {
    delta.clamp(0.0, remaining)
}

impl PendingPersonalForecast {
    /// Advances the interval with one valid observation.
    ///
    /// The frozen rules are applied in the order of the validated protocol: a
    /// continuity break, a reset change, a decrease and a saturation all
    /// reject the interval, and only an observation inside the maturation
    /// window matures it.
    pub fn observe(
        &mut self,
        observed_at: i64,
        used_percent: f64,
        resets_at: Option<i64>,
    ) -> PendingOutcome {
        if observed_at == self.last_valid_at {
            return PendingOutcome::Duplicate;
        }
        if observed_at < self.last_valid_at {
            return PendingOutcome::Rejected(RejectionReason::Backwards);
        }
        if observed_at - self.last_valid_at > FEEDBACK_MAX_GAP_SECS {
            return PendingOutcome::Rejected(RejectionReason::Gap);
        }
        if resets_at != Some(self.origin_reset_at) {
            return PendingOutcome::Rejected(RejectionReason::ResetMismatch);
        }
        if used_percent + EPSILON < self.last_valid_used_percent {
            return PendingOutcome::Rejected(RejectionReason::Decrease);
        }
        if used_percent >= 100.0 {
            return PendingOutcome::Rejected(RejectionReason::Saturation);
        }

        let elapsed = observed_at - self.origin_at;
        if elapsed > FEEDBACK_MAX_ELAPSED_SECS {
            return PendingOutcome::Rejected(RejectionReason::TooLate);
        }
        if elapsed < FEEDBACK_HORIZON_SECS {
            self.last_valid_at = observed_at;
            self.last_valid_used_percent = used_percent;
            return PendingOutcome::Continue;
        }

        let remaining = (100.0 - self.origin_used_percent).max(0.0);
        let seconds = elapsed as f64 / 3600.0;
        let fixed = cap_delta(self.origin_fixed_rate_per_hour * seconds, remaining);
        let published = self
            .origin_published_rate_per_hour
            .map(|rate| cap_delta(rate * seconds, remaining));
        let shadow = self
            .origin_shadow_rate_per_hour
            .map(|rate| cap_delta(rate * seconds, remaining));
        let observed_delta = used_percent - self.origin_used_percent;
        // A zero observed increase carries no information unless the
        // prediction was already small.
        let confidence = if observed_delta.abs() <= EPSILON && fixed < 0.75 {
            0.0
        } else {
            1.0
        };
        PendingOutcome::Matured(MaturedOutcome {
            elapsed_seconds: elapsed,
            predicted_delta: fixed,
            observed_delta,
            confidence,
            published_predicted_delta: published,
            fixed_predicted_delta: fixed,
            shadow_predicted_delta: shadow,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::shadow::SHADOW_ALGORITHM_VERSION;
    use super::*;

    const ORIGIN_RESET: i64 = 100_000;

    fn pending(origin_at: i64, used: f64, fixed_rate: f64) -> PendingPersonalForecast {
        PendingPersonalForecast {
            algorithm_version: SHADOW_ALGORITHM_VERSION,
            origin_at,
            origin_used_percent: used,
            origin_reset_at: ORIGIN_RESET,
            origin_published_rate_per_hour: Some(fixed_rate),
            origin_fixed_rate_per_hour: fixed_rate,
            origin_raw_factor: 1.0,
            origin_trust: 0.0,
            origin_effective_factor: 1.0,
            origin_shadow_rate_per_hour: Some(fixed_rate),
            origin_evidence: 0.0,
            origin_informative_updates: 0,
            last_valid_at: origin_at,
            last_valid_used_percent: used,
        }
    }

    fn matured(outcome: PendingOutcome) -> MaturedOutcome {
        match outcome {
            PendingOutcome::Matured(matured) => matured,
            other => panic!("expected a matured outcome, got {other:?}"),
        }
    }

    /// Walks the interval forward with sub-limit gaps, without reaching the
    /// maturation window, so that only the rule under test can fire.
    fn walk_to(forecast: &mut PendingPersonalForecast, origin: i64, until: i64) {
        let mut at = origin;
        while at + 45 < until && at + 45 - origin < 300 {
            at += 45;
            let outcome =
                forecast.observe(at, forecast.last_valid_used_percent, Some(ORIGIN_RESET));
            assert!(
                matches!(outcome, PendingOutcome::Continue),
                "unexpected {outcome:?}"
            );
        }
    }

    #[test]
    fn the_learner_starts_neutral() {
        let state = PersonalCalibrationState::default();
        assert_eq!(state.a, 10.0);
        assert_eq!(state.b, 10.0);
        assert_eq!(state.evidence, 0.0);
        assert_eq!(state.raw_factor(), 1.0);
        assert_eq!(state.trust(), 0.0);
        assert_eq!(state.effective_factor(), 1.0);
        assert_eq!(state.matured_feedbacks, 0);
        assert_eq!(state.informative_updates, 0);
        assert_eq!(state.last_update_at, None);
    }

    #[test]
    fn one_known_update_produces_the_expected_state() {
        let mut state = PersonalCalibrationState::default();
        // p = 2, y = 3, q = 1
        state.apply_outcome(1.0, 2.0, 3.0, 1000);
        assert!((state.a - (0.99 * 10.0 + 6.0)).abs() < 1e-12);
        assert!((state.b - (0.99 * 10.0 + 4.0)).abs() < 1e-12);
        assert!((state.evidence - 1.0).abs() < 1e-12);
        assert!((state.raw_factor() - 15.9 / 13.9).abs() < 1e-12);
        assert_eq!(state.matured_feedbacks, 1);
        assert_eq!(state.informative_updates, 1);
        assert_eq!(state.last_update_at, Some(1000));
    }

    #[test]
    fn a_zero_confidence_outcome_only_decays() {
        let mut state = PersonalCalibrationState::default();
        state.apply_outcome(1.0, 2.0, 3.0, 1000);
        let (a, b, evidence) = (state.a, state.b, state.evidence);
        state.apply_outcome(0.0, 2.0, 3.0, 1300);
        assert!((state.a - 0.99 * a).abs() < 1e-12);
        assert!((state.b - 0.99 * b).abs() < 1e-12);
        assert!((state.evidence - 0.99 * evidence).abs() < 1e-12);
        // The interval matured, but it carried no information.
        assert_eq!(state.matured_feedbacks, 2);
        assert_eq!(state.informative_updates, 1);
    }

    #[test]
    fn the_raw_factor_clamps_to_the_frozen_bounds() {
        let mut state = PersonalCalibrationState::default();
        state.a = 100.0;
        state.b = 1.0;
        assert_eq!(state.raw_factor(), RAW_FACTOR_MAX);
        state.a = 1.0;
        state.b = 100.0;
        assert_eq!(state.raw_factor(), RAW_FACTOR_MIN);
    }

    #[test]
    fn the_evidence_follows_the_frozen_decay() {
        let mut state = PersonalCalibrationState::default();
        state.apply_outcome(1.0, 1.0, 1.0, 0);
        assert!((state.evidence - 1.0).abs() < 1e-12);
        state.apply_outcome(0.0, 1.0, 1.0, 0);
        assert!((state.evidence - 0.99).abs() < 1e-12);
    }

    #[test]
    fn the_trust_stays_inside_the_unit_interval() {
        let mut state = PersonalCalibrationState::default();
        for _ in 0..1000 {
            state.apply_outcome(1.0, 1.0, 1.0, 0);
            let trust = state.trust();
            assert!(trust.is_finite() && (0.0..=1.0).contains(&trust));
        }
    }

    #[test]
    fn the_effective_factor_stays_between_one_and_the_raw_factor() {
        let mut state = PersonalCalibrationState::default();
        for step in 0..500 {
            let observed = if step % 3 == 0 { 0.5 } else { 4.0 };
            state.apply_outcome(1.0, 2.0, observed, step);
            let raw = state.raw_factor();
            let effective = state.effective_factor();
            let low = raw.min(1.0);
            let high = raw.max(1.0);
            assert!(
                effective >= low - 1e-12 && effective <= high + 1e-12,
                "effective {effective} outside [{low}, {high}]"
            );
        }
    }

    #[test]
    fn continuous_evidence_approaches_one_hundred_from_below() {
        let mut state = PersonalCalibrationState::default();
        for _ in 0..1000 {
            state.apply_outcome(1.0, 1.0, 1.0, 0);
            assert!(state.evidence <= 100.0);
        }
        assert!(state.evidence < 100.0);
        assert!(state.evidence > 99.0);
        // The lifetime counter keeps counting past the trust ceiling.
        assert_eq!(state.informative_updates, 1000);
        assert_eq!(state.matured_feedbacks, 1000);
    }

    #[test]
    fn the_effective_factor_follows_the_frozen_formula() {
        // A prediction of two points that observed one makes the raw factor
        // differ from one, so the formula is actually exercised.
        let mut state = PersonalCalibrationState::default();
        state.apply_outcome(1.0, 2.0, 1.0, 0);
        let raw = state.raw_factor();
        let trust = state.trust();
        assert!(raw < 1.0, "the raw factor must differ from one");
        assert!(trust > 0.0);
        let expected = 1.0 + trust * (raw - 1.0);
        assert!(
            (state.effective_factor() - expected).abs() < 1e-12,
            "{} vs {expected}",
            state.effective_factor()
        );
        // The attenuation moves the factor towards one, never past it.
        assert!(state.effective_factor() > raw);
        assert!(state.effective_factor() < 1.0);
    }

    #[test]
    fn an_old_contribution_decays_exponentially() {
        for steps in [100, 200, 300] {
            let mut state = PersonalCalibrationState::default();
            state.apply_outcome(1.0, 2.0, 4.0, 0);
            let first = state.a;
            for _ in 0..steps {
                // A zero-confidence outcome only advances the decay clock.
                state.apply_outcome(0.0, 1.0, 1.0, 0);
            }
            let expected = 0.99f64.powi(steps) * first;
            assert!(
                (state.a - expected).abs() < 1e-9,
                "{steps} steps: {} vs {expected}",
                state.a
            );
        }
    }

    #[test]
    fn a_corrupt_learner_state_normalizes_to_the_neutral_defaults() {
        let mut state = PersonalCalibrationState {
            a: f64::NAN,
            b: -1.0,
            evidence: f64::INFINITY,
            matured_feedbacks: 3,
            informative_updates: 2,
            last_update_at: Some(-5),
        };
        assert!(state.normalize());
        assert_eq!(state.a, LEARNER_PRIOR);
        assert_eq!(state.b, LEARNER_PRIOR);
        assert_eq!(state.evidence, 0.0);
        assert_eq!(state.last_update_at, None);
        // The lifetime counters are not model evidence and survive.
        assert_eq!(state.matured_feedbacks, 3);
        assert_eq!(state.informative_updates, 2);
        // A second pass is a no-op.
        assert!(!state.normalize());
    }

    #[test]
    fn a_long_deterministic_sequence_stays_finite() {
        let mut state = PersonalCalibrationState::default();
        for step in 0..20_000 {
            let observed = ((step % 7) as f64) * 0.5;
            state.apply_outcome(1.0, 1.0 + (step % 3) as f64, observed, step as i64);
            assert!(state.a.is_finite() && state.b.is_finite() && state.evidence.is_finite());
            assert!(state.a > 0.0 && state.b > 0.0 && state.evidence >= 0.0);
            assert!(state.raw_factor().is_finite());
            assert!(state.trust().is_finite());
            assert!(state.effective_factor().is_finite());
        }
    }

    #[test]
    fn a_refresh_before_the_horizon_cannot_train() {
        let mut forecast = pending(1000, 10.0, 60.0);
        walk_to(&mut forecast, 1000, 1300);
        assert_eq!(
            forecast.observe(1299, 10.5, Some(ORIGIN_RESET)),
            PendingOutcome::Continue
        );
        assert_eq!(forecast.last_valid_at, 1299);
        assert_eq!(forecast.last_valid_used_percent, 10.5);
    }

    #[test]
    fn the_first_target_inside_the_window_matures() {
        let mut forecast = pending(1000, 10.0, 60.0);
        walk_to(&mut forecast, 1000, 1300);
        let outcome = matured(forecast.observe(1300, 11.0, Some(ORIGIN_RESET)));
        assert_eq!(outcome.elapsed_seconds, 300);
        assert!((outcome.observed_delta - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_actual_elapsed_time_is_used_in_the_prediction() {
        let mut forecast = pending(1000, 10.0, 60.0);
        walk_to(&mut forecast, 1000, 1360);
        let outcome = matured(forecast.observe(1360, 11.0, Some(ORIGIN_RESET)));
        // 60 points per hour for 360 seconds.
        assert!((outcome.fixed_predicted_delta - 6.0).abs() < 1e-12);
        assert_eq!(outcome.elapsed_seconds, 360);
    }

    #[test]
    fn every_prediction_is_capped_by_the_quota_remaining_at_the_origin() {
        let mut forecast = pending(1000, 95.0, 60.0);
        // The published and the personalized rate differ from the fixed one.
        forecast.origin_published_rate_per_hour = Some(30.0);
        forecast.origin_shadow_rate_per_hour = Some(120.0);
        walk_to(&mut forecast, 1000, 1300);
        let outcome = matured(forecast.observe(1300, 96.0, Some(ORIGIN_RESET)));
        // Five quota points were left at the origin.
        assert!((outcome.fixed_predicted_delta - 5.0).abs() < 1e-12);
        assert!((outcome.published_predicted_delta.unwrap() - 2.5).abs() < 1e-12);
        // The personalized prediction was capped on its own.
        assert!((outcome.shadow_predicted_delta.unwrap() - 5.0).abs() < 1e-12);
    }

    #[test]
    fn the_learner_always_trains_on_the_fixed_prediction() {
        let mut forecast = pending(1000, 10.0, 60.0);
        forecast.origin_shadow_rate_per_hour = Some(120.0);
        walk_to(&mut forecast, 1000, 1300);
        let outcome = matured(forecast.observe(1300, 11.0, Some(ORIGIN_RESET)));
        assert!((outcome.predicted_delta - outcome.fixed_predicted_delta).abs() < 1e-12);
        assert!((outcome.fixed_predicted_delta - 5.0).abs() < 1e-12);
        assert!((outcome.shadow_predicted_delta.unwrap() - 10.0).abs() < 1e-12);
    }

    #[test]
    fn a_gap_above_the_frozen_limit_rejects() {
        let mut forecast = pending(1000, 10.0, 60.0);
        assert_eq!(
            forecast.observe(1091, 10.5, Some(ORIGIN_RESET)),
            PendingOutcome::Rejected(RejectionReason::Gap)
        );
    }

    #[test]
    fn a_reset_change_rejects() {
        let mut forecast = pending(1000, 10.0, 60.0);
        walk_to(&mut forecast, 1000, 1300);
        assert_eq!(
            forecast.observe(1300, 10.5, Some(200_000)),
            PendingOutcome::Rejected(RejectionReason::ResetMismatch)
        );
    }

    #[test]
    fn a_decrease_rejects() {
        let mut forecast = pending(1000, 10.0, 60.0);
        walk_to(&mut forecast, 1000, 1300);
        assert_eq!(
            forecast.observe(1300, 9.5, Some(ORIGIN_RESET)),
            PendingOutcome::Rejected(RejectionReason::Decrease)
        );
    }

    #[test]
    fn a_saturation_rejects() {
        let mut forecast = pending(1000, 10.0, 60.0);
        walk_to(&mut forecast, 1000, 1300);
        assert_eq!(
            forecast.observe(1300, 100.0, Some(ORIGIN_RESET)),
            PendingOutcome::Rejected(RejectionReason::Saturation)
        );
    }

    #[test]
    fn a_too_late_target_rejects() {
        let mut forecast = pending(1000, 10.0, 60.0);
        // A persisted interval that already passed the maturation window.
        forecast.last_valid_at = 1390;
        assert_eq!(
            forecast.observe(1400, 11.0, Some(ORIGIN_RESET)),
            PendingOutcome::Rejected(RejectionReason::TooLate)
        );
    }

    #[test]
    fn a_backward_timestamp_rejects() {
        let mut forecast = pending(1000, 10.0, 60.0);
        forecast.last_valid_at = 1200;
        assert_eq!(
            forecast.observe(1100, 10.5, Some(ORIGIN_RESET)),
            PendingOutcome::Rejected(RejectionReason::Backwards)
        );
    }

    #[test]
    fn a_same_second_refresh_is_ignored() {
        let mut forecast = pending(1000, 10.0, 60.0);
        assert_eq!(
            forecast.observe(1000, 10.0, Some(ORIGIN_RESET)),
            PendingOutcome::Duplicate
        );
        assert_eq!(forecast.last_valid_at, 1000);
    }

    #[test]
    fn a_zero_observation_is_uninformative_only_for_a_small_prediction() {
        let mut small = pending(1000, 10.0, 0.5);
        walk_to(&mut small, 1000, 1300);
        let outcome = matured(small.observe(1300, 10.0, Some(ORIGIN_RESET)));
        assert_eq!(outcome.confidence, 0.0);
        assert!((outcome.observed_delta).abs() < 1e-12);

        let mut large = pending(1000, 10.0, 10.0);
        walk_to(&mut large, 1000, 1300);
        let outcome = matured(large.observe(1300, 10.0, Some(ORIGIN_RESET)));
        assert_eq!(outcome.confidence, 1.0);
    }

    #[test]
    fn the_rejection_codes_are_stable() {
        assert_eq!(RejectionReason::Gap.code(), "gap");
        assert_eq!(RejectionReason::ResetMismatch.code(), "reset_mismatch");
        assert_eq!(RejectionReason::Decrease.code(), "decrease");
        assert_eq!(RejectionReason::Saturation.code(), "saturation");
        assert_eq!(RejectionReason::TooLate.code(), "too_late");
        assert_eq!(RejectionReason::Backwards.code(), "backwards");
        assert_eq!(RejectionReason::QuotaCycle.code(), "quota_cycle");
        assert_eq!(RejectionReason::LongIdle.code(), "long_idle");
        assert_eq!(RejectionReason::Correction.code(), "correction");
    }
}
