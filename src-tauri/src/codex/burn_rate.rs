use serde::{Deserialize, Serialize};
use std::fs;

use crate::config::ConfigManager;

use super::{QuotaSnapshot, RawCodexUsage};

const SCHEMA_VERSION: u32 = 1;
const FIVE_HOUR_HORIZON_SECS: i64 = 30 * 60;
const WEEKLY_HORIZON_SECS: i64 = 24 * 60 * 60;
const FIVE_HOUR_MAX_GAP_SECS: i64 = 10 * 60;
const WEEKLY_MAX_GAP_SECS: i64 = 6 * 60 * 60;
const MAX_CROSSINGS: usize = 64;
const EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrossingEvent {
    pub level: u32,
    pub observed_at: i64,
    pub segment_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionCandidate {
    pub used_percent: f64,
    pub observed_at: i64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaRateState {
    pub resets_at: Option<i64>,
    pub segment_id: u64,
    pub last_observed_percent: Option<f64>,
    pub last_observed_at: Option<i64>,
    pub correction_candidate: Option<CorrectionCandidate>,
    pub crossings: Vec<CrossingEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BurnRateStateFile {
    pub schema_version: u32,
    pub account_id: Option<String>,
    pub five_hour: QuotaRateState,
    pub weekly: QuotaRateState,
}

impl Default for BurnRateStateFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            account_id: None,
            five_hour: QuotaRateState::default(),
            weekly: QuotaRateState::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BurnRateEstimate {
    pub five_hour_per_hour: Option<f64>,
    pub weekly_per_hour: Option<f64>,
    pub state_changed: bool,
}

pub struct BurnRateTracker {
    state: BurnRateStateFile,
}

impl BurnRateTracker {
    pub fn load(config: &ConfigManager) -> Self {
        let state = fs::read_to_string(config.burn_rate_state_path())
            .ok()
            .and_then(|raw| serde_json::from_str::<BurnRateStateFile>(&raw).ok())
            .filter(|state| state.schema_version == SCHEMA_VERSION)
            .unwrap_or_default();
        Self { state }
    }

    #[cfg(test)]
    pub fn from_state(state: BurnRateStateFile) -> Self {
        Self { state }
    }

    pub fn save(&self, config: &ConfigManager) -> Result<(), String> {
        let path = config.burn_rate_state_path();
        let tmp = path.with_extension("json.tmp");
        let serialized = serde_json::to_vec_pretty(&self.state).map_err(|e| e.to_string())?;
        fs::write(&tmp, serialized).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    pub fn accept_usage(&mut self, usage: &RawCodexUsage) -> BurnRateEstimate {
        let mut changed = self.accept_account(usage.account_id.as_deref());
        changed |= accept_quota(
            &mut self.state.five_hour,
            &usage.five_hour,
            usage.fetched_at,
            FIVE_HOUR_MAX_GAP_SECS,
        );
        changed |= accept_quota(
            &mut self.state.weekly,
            &usage.weekly,
            usage.fetched_at,
            WEEKLY_MAX_GAP_SECS,
        );

        BurnRateEstimate {
            five_hour_per_hour: valid_used_percent(&usage.five_hour)
                .and_then(|_| estimate_rate(&self.state.five_hour, FIVE_HOUR_HORIZON_SECS, 2)),
            weekly_per_hour: valid_used_percent(&usage.weekly)
                .and_then(|_| estimate_rate(&self.state.weekly, WEEKLY_HORIZON_SECS, 1)),
            state_changed: changed,
        }
    }

    fn accept_account(&mut self, incoming: Option<&str>) -> bool {
        let Some(incoming) = incoming.filter(|id| !id.is_empty()) else {
            return false;
        };
        if self.state.account_id.as_deref() == Some(incoming) {
            return false;
        }

        let had_attributed_or_unattributed_history = self.state.account_id.is_some()
            || self.state.five_hour.last_observed_at.is_some()
            || self.state.weekly.last_observed_at.is_some();
        if had_attributed_or_unattributed_history {
            self.state.five_hour = QuotaRateState::default();
            self.state.weekly = QuotaRateState::default();
        }
        self.state.account_id = Some(incoming.to_string());
        true
    }
}

fn valid_used_percent(snapshot: &QuotaSnapshot) -> Option<f64> {
    snapshot.used_percent.filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
}

fn accept_quota(
    state: &mut QuotaRateState,
    snapshot: &QuotaSnapshot,
    observed_at: i64,
    max_gap_secs: i64,
) -> bool {
    let Some(current) = valid_used_percent(snapshot) else {
        return false;
    };

    let (boundary_changed, actual_cycle_transition) =
        accept_reset_boundary(state, snapshot.resets_at, observed_at);
    let mut changed = boundary_changed;
    if actual_cycle_transition {
        start_new_segment(state);
        set_baseline(state, current, observed_at);
        return true;
    }

    let Some(previous_at) = state.last_observed_at else {
        set_baseline(state, current, observed_at);
        return true;
    };
    let previous = state.last_observed_percent.unwrap_or(current);

    if observed_at < previous_at {
        return changed;
    }
    if observed_at.saturating_sub(previous_at) > max_gap_secs {
        start_new_segment(state);
        set_baseline(state, current, observed_at);
        return true;
    }

    if current + EPSILON < previous {
        if state.correction_candidate.is_some() {
            start_new_segment(state);
            set_baseline(state, current, observed_at);
        } else {
            state.correction_candidate = Some(CorrectionCandidate {
                used_percent: current,
                observed_at,
                resets_at: snapshot.resets_at,
            });
        }
        return true;
    }

    if state.correction_candidate.take().is_some() {
        changed = true;
    }

    if observed_at == previous_at {
        if (current - previous).abs() <= EPSILON {
            return changed;
        }
        return changed;
    }

    if current > previous + EPSILON {
        let first_level = previous.floor() as i64 + 1;
        let last_level = current.floor() as i64;
        if last_level >= first_level {
            let count = (last_level - first_level + 1) as i64;
            let interval = observed_at - previous_at;
            for index in 1..=count {
                let level = (first_level + index - 1) as u32;
                let crossing_at = previous_at + interval * index / (count + 1);
                state.crossings.push(CrossingEvent {
                    level,
                    observed_at: crossing_at,
                    segment_id: state.segment_id,
                });
            }
            prune_crossings(state);
        }
    }

    state.last_observed_percent = Some(current);
    state.last_observed_at = Some(observed_at);
    true
}

fn accept_reset_boundary(
    state: &mut QuotaRateState,
    incoming: Option<i64>,
    now: i64,
) -> (bool, bool) {
    let Some(new_boundary) = incoming.filter(|v| *v > 0) else {
        return (false, false);
    };
    let Some(old_boundary) = state.resets_at else {
        state.resets_at = Some(new_boundary);
        return (true, false);
    };
    if new_boundary <= old_boundary {
        return (false, false);
    }
    state.resets_at = Some(new_boundary);
    (true, old_boundary <= now)
}

fn start_new_segment(state: &mut QuotaRateState) {
    state.segment_id = state.segment_id.saturating_add(1);
    state.last_observed_percent = None;
    state.last_observed_at = None;
    state.correction_candidate = None;
    state.crossings.clear();
}

fn set_baseline(state: &mut QuotaRateState, percent: f64, observed_at: i64) {
    state.last_observed_percent = Some(percent);
    state.last_observed_at = Some(observed_at);
    state.correction_candidate = None;
}

fn prune_crossings(state: &mut QuotaRateState) {
    if state.crossings.len() > MAX_CROSSINGS {
        let remove = state.crossings.len() - MAX_CROSSINGS;
        state.crossings.drain(0..remove);
    }
}

fn estimate_rate(
    state: &QuotaRateState,
    horizon_secs: i64,
    minimum_level_span: u32,
) -> Option<f64> {
    let end_time = state.last_observed_at?;
    let eligible: Vec<&CrossingEvent> = state
        .crossings
        .iter()
        .filter(|event| {
            event.segment_id == state.segment_id
                && event.observed_at <= end_time
                && end_time.saturating_sub(event.observed_at) <= horizon_secs
        })
        .collect();
    let latest = *eligible.last()?;

    let earliest = eligible.into_iter().find(|event| {
        let span = latest.level.saturating_sub(event.level);
        span >= minimum_level_span && span <= 3
    })?;
    let seconds = end_time.saturating_sub(earliest.observed_at);
    if seconds <= 0 {
        return None;
    }
    let span = latest.level.saturating_sub(earliest.level);
    if span < minimum_level_span {
        return None;
    }
    let rate = span as f64 * 3600.0 / seconds as f64;
    (rate.is_finite() && rate >= 0.0).then_some(rate)
}

#[cfg(test)]
mod replay;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::RawCodexUsage;

    fn quota(used: f64, reset: i64) -> QuotaSnapshot {
        QuotaSnapshot {
            used_percent: Some(used),
            resets_at: Some(reset),
            window_duration_mins: None,
        }
    }

    fn usage(at: i64, five: f64, week: f64) -> RawCodexUsage {
        RawCodexUsage {
            account_id: Some("acct-a".into()),
            five_hour: quota(five, 20_000),
            weekly: quota(week, 90_000),
            credits_display: "—".into(),
            credits_balance: None,
            has_credits: false,
            fetched_at: at,
        }
    }

    #[test]
    fn baseline_and_repeated_observation_create_no_rate() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        assert_eq!(
            tracker
                .accept_usage(&usage(1000, 10.0, 10.0))
                .five_hour_per_hour,
            None
        );
        let estimate = tracker.accept_usage(&usage(1060, 10.0, 10.0));
        assert_eq!(estimate.five_hour_per_hour, None);
        assert_eq!(tracker.state.five_hour.last_observed_at, Some(1060));
        assert!(tracker.state.five_hour.crossings.is_empty());
    }

    #[test]
    fn repeated_sample_places_single_crossing_at_midpoint() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(1060, 10.0, 10.0));
        tracker.accept_usage(&usage(1120, 11.0, 10.0));
        assert_eq!(tracker.state.five_hour.crossings[0].level, 11);
        assert_eq!(tracker.state.five_hour.crossings[0].observed_at, 1090);
    }

    #[test]
    fn multi_point_jump_distributes_crossings_inside_interval() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(1060, 13.0, 10.0));
        let events = &tracker.state.five_hour.crossings;
        assert_eq!(
            events.iter().map(|e| e.level).collect::<Vec<_>>(),
            vec![11, 12, 13]
        );
        assert_eq!(
            events.iter().map(|e| e.observed_at).collect::<Vec<_>>(),
            vec![1015, 1030, 1045]
        );
    }

    #[test]
    fn five_hour_requires_two_point_span_and_decays_on_idle_observations() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        assert_eq!(
            tracker
                .accept_usage(&usage(1060, 11.0, 10.0))
                .five_hour_per_hour,
            None
        );
        assert_eq!(
            tracker
                .accept_usage(&usage(1120, 12.0, 10.0))
                .five_hour_per_hour,
            None
        );
        let active = tracker
            .accept_usage(&usage(1180, 13.0, 10.0))
            .five_hour_per_hour
            .unwrap();
        let decayed = tracker
            .accept_usage(&usage(1480, 13.0, 10.0))
            .five_hour_per_hour
            .unwrap();
        assert!(decayed < active);
    }

    #[test]
    fn stale_five_hour_evidence_becomes_unavailable() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(1060, 13.0, 10.0));
        assert!(tracker
            .accept_usage(&usage(1100, 13.0, 10.0))
            .five_hour_per_hour
            .is_some());
        assert_eq!(
            tracker
                .accept_usage(&usage(3000, 13.0, 10.0))
                .five_hour_per_hour,
            None
        );
    }

    #[test]
    fn weekly_uses_one_point_span_and_longer_horizon() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(4600, 10.0, 11.0));
        let rate = tracker
            .accept_usage(&usage(8200, 10.0, 12.0))
            .weekly_per_hour;
        assert!(rate.is_some());
    }

    #[test]
    fn long_gap_starts_new_segment_without_synthetic_crossings() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(2000, 20.0, 10.0));
        assert!(tracker.state.five_hour.crossings.is_empty());
        assert_eq!(tracker.state.five_hour.last_observed_percent, Some(20.0));
        assert!(tracker.state.five_hour.segment_id > 0);
    }

    #[test]
    fn future_reset_shift_does_not_clear_history_but_crossed_boundary_does() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(1060, 13.0, 10.0));
        let before = tracker.state.five_hour.crossings.len();
        let mut shifted = usage(1120, 13.0, 10.0);
        shifted.five_hour.resets_at = Some(30_000);
        let shifted_estimate = tracker.accept_usage(&shifted);
        assert!(shifted_estimate.state_changed);
        assert_eq!(tracker.state.five_hour.resets_at, Some(30_000));
        assert_eq!(tracker.state.five_hour.crossings.len(), before);

        let mut after_reset = usage(30_001, 1.0, 10.0);
        after_reset.five_hour.resets_at = Some(40_000);
        tracker.accept_usage(&after_reset);
        assert!(tracker.state.five_hour.crossings.is_empty());
        assert_eq!(tracker.state.five_hour.last_observed_percent, Some(1.0));
    }

    #[test]
    fn regression_requires_confirmation_then_rebaselines() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 20.0, 10.0));
        tracker.accept_usage(&usage(1060, 21.0, 10.0));
        tracker.accept_usage(&usage(1120, 18.0, 10.0));
        assert!(tracker.state.five_hour.correction_candidate.is_some());
        assert_eq!(tracker.state.five_hour.last_observed_percent, Some(21.0));
        tracker.accept_usage(&usage(1180, 19.0, 10.0));
        assert!(tracker.state.five_hour.correction_candidate.is_none());
        assert_eq!(tracker.state.five_hour.last_observed_percent, Some(19.0));
        assert!(tracker.state.five_hour.crossings.is_empty());
    }

    #[test]
    fn single_regression_followed_by_recovery_is_discarded() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 20.0, 10.0));
        tracker.accept_usage(&usage(1060, 18.0, 10.0));
        tracker.accept_usage(&usage(1120, 21.0, 10.0));
        assert!(tracker.state.five_hour.correction_candidate.is_none());
        assert_eq!(tracker.state.five_hour.last_observed_percent, Some(21.0));
    }

    #[test]
    fn account_switch_clears_both_histories() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(1060, 13.0, 12.0));
        let mut switched = usage(1120, 50.0, 50.0);
        switched.account_id = Some("acct-b".into());
        let estimate = tracker.accept_usage(&switched);
        assert_eq!(estimate.five_hour_per_hour, None);
        assert_eq!(estimate.weekly_per_hour, None);
        assert!(tracker.state.five_hour.crossings.is_empty());
        assert!(tracker.state.weekly.crossings.is_empty());
    }

    #[test]
    fn history_is_bounded() {
        let mut state = QuotaRateState::default();
        state.segment_id = 7;
        state.crossings = (0..100)
            .map(|i| CrossingEvent {
                level: i,
                observed_at: i as i64,
                segment_id: 7,
            })
            .collect();
        prune_crossings(&mut state);
        assert_eq!(state.crossings.len(), MAX_CROSSINGS);
        assert_eq!(state.crossings[0].level, 36);
    }

    #[test]
    fn invalid_quota_is_unavailable_without_affecting_other_quota_or_history() {
        for invalid in [None, Some(f64::NAN), Some(f64::INFINITY), Some(f64::NEG_INFINITY), Some(-0.1), Some(100.1)] {
            for invalid_five_hour in [true, false] {
                let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
                tracker.accept_usage(&usage(1000, 10.0, 10.0));
                let established = tracker.accept_usage(&usage(1060, 13.0, 12.0));
                assert!(established.five_hour_per_hour.is_some());
                assert!(established.weekly_per_hour.is_some());
                let saved = tracker.state.clone();
                let mut incoming = usage(1120, 13.0, 12.0);
                if invalid_five_hour {
                    incoming.five_hour.used_percent = invalid;
                } else {
                    incoming.weekly.used_percent = invalid;
                }
                let estimate = tracker.accept_usage(&incoming);
                if invalid_five_hour {
                    assert_eq!(estimate.five_hour_per_hour, None);
                    assert!(estimate.weekly_per_hour.is_some());
                    assert_eq!(tracker.state.five_hour, saved.five_hour);
                    assert_eq!(tracker.state.weekly.last_observed_at, Some(1120));
                } else {
                    assert_eq!(estimate.weekly_per_hour, None);
                    assert!(estimate.five_hour_per_hour.is_some());
                    assert_eq!(tracker.state.weekly, saved.weekly);
                    assert_eq!(tracker.state.five_hour.last_observed_at, Some(1120));
                }
            }
        }
    }

    #[test]
    fn repeated_missing_samples_never_publish_rates_even_after_evidence_horizons() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(1060, 13.0, 12.0));
        let saved = tracker.state.clone();
        for at in [1120, 4000, 100_000] {
            let mut missing = usage(at, 13.0, 12.0);
            missing.five_hour = QuotaSnapshot::default();
            missing.weekly = QuotaSnapshot::default();
            let estimate = tracker.accept_usage(&missing);
            assert_eq!(estimate.five_hour_per_hour, None);
            assert_eq!(estimate.weekly_per_hour, None);
            assert!(!estimate.state_changed);
            assert_eq!(tracker.state, saved);
        }
    }

    #[test]
    fn valid_recovery_reuses_recent_history_and_rebaselines_after_long_gap() {
        for gap in [180, WEEKLY_MAX_GAP_SECS + 1] {
            let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
            tracker.accept_usage(&usage(1000, 10.0, 10.0));
            tracker.accept_usage(&usage(1060, 13.0, 12.0));
            let saved = tracker.state.clone();
            let mut missing = usage(1120, 13.0, 12.0);
            missing.five_hour.used_percent = None;
            missing.weekly.used_percent = None;
            tracker.accept_usage(&missing);
            let recovered = tracker.accept_usage(&usage(1060 + gap, 13.0, 12.0));
            if gap == 180 {
                assert!(recovered.five_hour_per_hour.is_some());
                assert!(recovered.weekly_per_hour.is_some());
                assert_eq!(tracker.state.five_hour.crossings, saved.five_hour.crossings);
                assert_eq!(tracker.state.weekly.crossings, saved.weekly.crossings);
            } else {
                assert_eq!(recovered.five_hour_per_hour, None);
                assert_eq!(recovered.weekly_per_hour, None);
                assert!(tracker.state.five_hour.crossings.is_empty());
                assert!(tracker.state.weekly.crossings.is_empty());
                assert!(tracker.state.five_hour.segment_id > saved.five_hour.segment_id);
                assert!(tracker.state.weekly.segment_id > saved.weekly.segment_id);
            }
        }
    }

    #[test]
    fn persisted_state_round_trips_and_bad_schema_falls_back() {
        let dir = std::env::temp_dir().join(format!("codex-burn-rate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config = ConfigManager::with_runtime_dir(dir.clone());
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(1060, 13.0, 12.0));
        tracker.save(&config).unwrap();
        let loaded = BurnRateTracker::load(&config);
        assert_eq!(loaded.state, tracker.state);

        let mut wrong = tracker.state.clone();
        wrong.schema_version = 99;
        fs::write(
            config.burn_rate_state_path(),
            serde_json::to_vec(&wrong).unwrap(),
        )
        .unwrap();
        assert_eq!(
            BurnRateTracker::load(&config).state,
            BurnRateStateFile::default()
        );

        fs::write(config.burn_rate_state_path(), b"{broken").unwrap();
        assert_eq!(
            BurnRateTracker::load(&config).state,
            BurnRateStateFile::default()
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
