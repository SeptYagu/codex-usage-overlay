mod affine;
use serde::{Deserialize, Serialize};
use std::fs;

use crate::config::ConfigManager;

use super::{QuotaSnapshot, RawCodexUsage};

use weighted::{
    WeightedParameters, WeightedShadowState, FIVE_HOUR_HARD_EVIDENCE_HORIZON_SECS,
    FIVE_HOUR_MIN_LEVEL_SPAN,
};

const SCHEMA_VERSION: u32 = 2;
const LEGACY_SCHEMA_VERSION: u32 = 1;
const SHADOW_ALGORITHM_VERSION: u32 = 1;
const FEEDBACK_HORIZON_SECS: i64 = 300;
const FEEDBACK_MAX_ELAPSED_SECS: i64 = 390;
const FEEDBACK_MAX_GAP_SECS: i64 = 90;
const FIVE_HOUR_HORIZON_SECS: i64 = 30 * 60;
const WEEKLY_HORIZON_SECS: i64 = 24 * 60 * 60;
const FIVE_HOUR_MAX_GAP_SECS: i64 = 10 * 60;
const WEEKLY_MAX_GAP_SECS: i64 = 6 * 60 * 60;
const MAX_CROSSINGS: usize = 64;
const EPSILON: f64 = 1e-9;

enum PendingOutcome {
    Continue,
    Duplicate,
    Rejected,
    Matured(affine::AffineSample),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInterval {
    pub algorithm_version: u32,
    pub origin_at: i64,
    pub origin_used_percent: f64,
    pub origin_reset_at: i64,
    pub origin_fixed_rate_per_hour: f64,
    pub last_valid_at: i64,
    pub last_valid_used_percent: f64,
    pub origin_segment_id: u64,
    #[serde(default, skip_serializing)]
    pub latched_rejection: Option<serde_json::Value>,
}

impl PendingInterval {
    fn build(
        origin_at: i64,
        used: Option<f64>,
        resets_at: Option<i64>,
        fixed_rate: Option<f64>,
        segment_id: u64,
    ) -> Option<Self> {
        let used = used.filter(|&v| v < 100.0)?;
        let resets_at = resets_at?;
        let fixed_rate = fixed_rate.filter(|&r| r.is_finite() && r > 0.0)?;
        if origin_at <= 0 {
            return None;
        }
        Some(Self {
            algorithm_version: SHADOW_ALGORITHM_VERSION,
            origin_at,
            origin_used_percent: used,
            origin_reset_at: resets_at,
            origin_fixed_rate_per_hour: fixed_rate,
            last_valid_at: origin_at,
            last_valid_used_percent: used,
            origin_segment_id: segment_id,
            latched_rejection: None,
        })
    }

    fn observe(
        &mut self,
        observed_at: i64,
        used_percent: f64,
        resets_at: Option<i64>,
    ) -> PendingOutcome {
        if observed_at == self.last_valid_at {
            return PendingOutcome::Duplicate;
        }
        if observed_at < self.last_valid_at
            || observed_at - self.last_valid_at > FEEDBACK_MAX_GAP_SECS
            || resets_at != Some(self.origin_reset_at)
            || used_percent + EPSILON < self.last_valid_used_percent
            || used_percent >= 100.0
        {
            return PendingOutcome::Rejected;
        }
        let elapsed = observed_at - self.origin_at;
        if elapsed > FEEDBACK_MAX_ELAPSED_SECS {
            return PendingOutcome::Rejected;
        }
        if elapsed < FEEDBACK_HORIZON_SECS {
            self.last_valid_at = observed_at;
            self.last_valid_used_percent = used_percent;
            return PendingOutcome::Continue;
        }
        let remaining = (100.0 - self.origin_used_percent).max(0.0);
        let fixed = (self.origin_fixed_rate_per_hour * elapsed as f64 / 3600.0).clamp(0.0, remaining);
        let observed_delta = used_percent - self.origin_used_percent;
        let confidence = if observed_delta.abs() <= EPSILON && fixed < 0.75 {
            0.0
        } else {
            1.0
        };
        PendingOutcome::Matured(affine::AffineSample {
            elapsed_seconds: elapsed,
            observed_delta,
            fixed_predicted_delta: fixed,
            confidence,
        })
    }

    pub fn is_valid_for(&self, quota: &QuotaRateState) -> bool {
        self.latched_rejection.is_none()
            && self.algorithm_version == SHADOW_ALGORITHM_VERSION
            && self.origin_at > 0
            && self.last_valid_at >= self.origin_at
            && self.origin_used_percent.is_finite()
            && (0.0..100.0).contains(&self.origin_used_percent)
            && self.last_valid_used_percent.is_finite()
            && self.last_valid_used_percent >= self.origin_used_percent - EPSILON
            && self.last_valid_used_percent < 100.0
            && self.origin_reset_at > 0
            && self.origin_fixed_rate_per_hour.is_finite()
            && self.origin_fixed_rate_per_hour > 0.0
            && self.origin_segment_id == quota.segment_id
            && quota.last_observed_at.is_some_and(|at| at >= self.last_valid_at)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FiveHourShadowState {
    pub algorithm_version: u32,
    pub weighted: WeightedShadowState,
    #[serde(default, deserialize_with = "affine::recover")]
    pub affine: affine::AffineCalibration,
    pub pending: Option<PendingInterval>,
}

impl Default for FiveHourShadowState {
    fn default() -> Self {
        Self {
            algorithm_version: SHADOW_ALGORITHM_VERSION,
            weighted: WeightedShadowState::default(),
            affine: affine::AffineCalibration::default(),
            pending: None,
        }
    }
}

impl FiveHourShadowState {
    pub fn isolate_algorithm_version(&mut self) -> bool {
        if self.algorithm_version == SHADOW_ALGORITHM_VERSION {
            return false;
        }
        *self = Self::default();
        true
    }

    pub fn normalize(&mut self, quota: &QuotaRateState) -> bool {
        let before = self.clone();
        self.weighted.normalize(quota);
        if !self.affine.valid(quota.last_observed_at) {
            self.affine = affine::AffineCalibration::default();
        }
        if self.pending.as_ref().is_some_and(|pending| !pending.is_valid_for(quota)) {
            self.pending = None;
        }
        before != *self
    }
}

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
    /// The shadow subtree. It is new in schema v2 and starts neutral.
    pub five_hour_shadow: FiveHourShadowState,
}

impl Default for BurnRateStateFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            account_id: None,
            five_hour: QuotaRateState::default(),
            weekly: QuotaRateState::default(),
            five_hour_shadow: FiveHourShadowState::default(),
        }
    }
}

/// The legacy core of the state, without the shadow subtree.
///
/// It is the recovery shape: a file whose shadow subtree is malformed keeps
/// its quota history.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct BurnRateStateCore {
    account_id: Option<String>,
    five_hour: QuotaRateState,
    weekly: QuotaRateState,
}

/// Reads a persisted state and migrates it to the current schema.
///
/// A legacy file keeps its quota history and gains a neutral shadow subtree.
/// A file whose shadow subtree is malformed keeps the recoverable core and
/// resets the shadow, so a corrupt experiment never costs the user history.
/// An unknown future schema falls back without pretending compatibility.
pub fn load_state(raw: &str) -> BurnRateStateFile {
    let version = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| value.get("schemaVersion").and_then(serde_json::Value::as_u64))
        .unwrap_or(0) as u32;
    match version {
        SCHEMA_VERSION => match serde_json::from_str::<BurnRateStateFile>(raw) {
            Ok(mut state) => {
                state.five_hour_shadow.isolate_algorithm_version();
                state.five_hour_shadow.normalize(&state.five_hour);
                state
            }
            // The shadow subtree is unreadable; the core may still be.
            Err(_) => migrate_legacy_state(raw),
        },
        LEGACY_SCHEMA_VERSION => migrate_legacy_state(raw),
        _ => BurnRateStateFile::default(),
    }
}

/// Migrates a legacy state to the current schema.
fn migrate_legacy_state(raw: &str) -> BurnRateStateFile {
    match serde_json::from_str::<BurnRateStateCore>(raw) {
        Ok(core) => BurnRateStateFile {
            schema_version: SCHEMA_VERSION,
            account_id: core.account_id,
            five_hour: core.five_hour,
            weekly: core.weekly,
            five_hour_shadow: FiveHourShadowState::default(),
        },
        Err(_) => BurnRateStateFile::default(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BurnRateEstimate {
    pub five_hour_per_hour: Option<f64>,
    pub weekly_per_hour: Option<f64>,
    pub state_changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuotaAcceptance {
    changed: bool,
    accepted: bool,
    transitioned: bool,
}

pub struct BurnRateTracker {
    state: BurnRateStateFile,
    /// The in-memory state is newer than the persisted copy.
    state_dirty: bool,
    weighted_five_hour_per_hour: Option<f64>,
    affine_enabled: bool,
    local_codex: bool,
    observation_gap: bool,
    observation_unavailable: bool,
}

impl BurnRateTracker {
    pub fn load(config: &ConfigManager) -> Self {
        let current = config.burn_rate_state_path();
        let legacy = config.legacy_burn_rate_state_path();
        let migrated = !current.is_file() && current != legacy && legacy.is_file();
        let state = fs::read_to_string(if migrated { &legacy } else { &current })
            .ok()
            .map(|raw| load_state(&raw))
            .unwrap_or_default();
        Self {
            state,
            state_dirty: migrated,
            weighted_five_hour_per_hour: None,
            affine_enabled: config.is_local_codex(),
            local_codex: config.is_local_codex(),
            observation_gap: false,
            observation_unavailable: false,
        }
    }

    #[cfg(test)]
    pub fn from_state(state: BurnRateStateFile) -> Self {
        Self {
            state,
            state_dirty: false,
            weighted_five_hour_per_hour: None,
            affine_enabled: true,
            local_codex: true,
            observation_gap: false,
            observation_unavailable: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_snapshot(&self) -> serde_json::Value {
        serde_json::to_value(&self.state).unwrap()
    }

    pub fn candidates(&self, estimate: &BurnRateEstimate) -> crate::presentation::Candidates {
        if !self.affine_enabled || (self.state.account_id.is_none() && !self.local_codex) {
            return crate::presentation::Candidates {
                legacy: estimate.five_hour_per_hour,
                weighted: None,
                personalized: None,
                learning: "identityUnavailable",
            };
        }
        let learning = if self.state_dirty {
            "persistencePaused"
        } else if self.observation_gap {
            "observationGap"
        } else if self.observation_unavailable {
            "observationUnavailable"
        } else if self.state.five_hour_shadow.affine.n == 0.0 {
            "accumulating"
        } else {
            "active"
        };
        crate::presentation::Candidates {
            legacy: estimate.five_hour_per_hour,
            weighted: self.weighted_five_hour_per_hour,
            personalized: self.affine_enabled.then(|| {
                self.weighted_five_hour_per_hour
                    .and_then(|rate| self.state.five_hour_shadow.affine.rate(rate))
            }).flatten(),
            learning,
        }
    }

    pub fn save(&mut self, config: &ConfigManager) -> Result<(), String> {
        let path = config.burn_rate_state_path();
        let tmp = path.with_extension("json.tmp");
        let serialized = serde_json::to_vec_pretty(&self.state).map_err(|e| e.to_string())?;
        fs::write(&tmp, serialized).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        self.state_dirty = false;
        Ok(())
    }

    /// Whether the persisted copy is older than the in-memory state.
    pub fn is_dirty(&self) -> bool {
        self.state_dirty
    }

    pub fn accept_usage(&mut self, usage: &RawCodexUsage) -> BurnRateEstimate {
        self.affine_enabled = self.local_codex || usage.account_id.as_deref().is_some_and(|id| !id.is_empty());
        let mut changed = false;

        changed |= self.state.five_hour_shadow.normalize(&self.state.five_hour);
        changed |= self.accept_account(usage.account_id.as_deref());

        let before = self.state.five_hour.clone();
        self.observation_gap = before.last_observed_at.is_some_and(|at| usage.fetched_at.saturating_sub(at) > FEEDBACK_MAX_GAP_SECS);
        self.observation_unavailable = valid_used_percent(&usage.five_hour).is_none();
        let acceptance = accept_quota(
            &mut self.state.five_hour,
            &usage.five_hour,
            usage.fetched_at,
            FIVE_HOUR_MAX_GAP_SECS,
        );
        changed |= acceptance.changed;
        let five_hour_per_hour = valid_used_percent(&usage.five_hour)
            .and_then(|_| estimate_rate(&self.state.five_hour, FIVE_HOUR_HORIZON_SECS, 2));

        let (calibration_changed, weighted_rate) = if self.affine_enabled {
            self.process_five_hour_calibration(usage, &before, acceptance)
        } else {
            (false, None)
        };
        changed |= calibration_changed;
        self.weighted_five_hour_per_hour = weighted_rate;

        changed |= accept_quota(
            &mut self.state.weekly,
            &usage.weekly,
            usage.fetched_at,
            WEEKLY_MAX_GAP_SECS,
        )
        .changed;

        if changed {
            self.state_dirty = true;
        }

        BurnRateEstimate {
            five_hour_per_hour,
            weekly_per_hour: valid_used_percent(&usage.weekly)
                .and_then(|_| estimate_rate(&self.state.weekly, WEEKLY_HORIZON_SECS, 1)),
            state_changed: changed,
        }
    }

    fn accept_account(&mut self, incoming: Option<&str>) -> bool {
        let Some(incoming) = incoming.filter(|id| !id.is_empty()) else {
            if self.local_codex || self.state.account_id.is_none() {
                return false;
            }
            self.state.account_id = None;
            self.reset_account_history();
            return true;
        };
        if self.state.account_id.as_deref() == Some(incoming) {
            return false;
        }

        let had_history = self.state.account_id.is_some()
            || self.state.five_hour.last_observed_at.is_some()
            || self.state.weekly.last_observed_at.is_some();
        self.state.account_id = Some(incoming.to_string());
        if had_history {
            self.reset_account_history();
        }
        true
    }

    fn reset_account_history(&mut self) {
        self.state.five_hour = QuotaRateState::default();
        self.state.weekly = QuotaRateState::default();
        self.state.five_hour_shadow = FiveHourShadowState::default();
    }

    fn process_five_hour_calibration(
        &mut self,
        usage: &RawCodexUsage,
        before: &QuotaRateState,
        acceptance: QuotaAcceptance,
    ) -> (bool, Option<f64>) {
        let observed_at = usage.fetched_at;
        let mut changed = false;
        let after = self.state.five_hour.clone();
        let used = valid_used_percent(&usage.five_hour);
        let resets_at = usage.five_hour.resets_at.filter(|value| *value > 0);

        changed |= self
            .state
            .five_hour_shadow
            .weighted
            .apply_transition(before, &after, observed_at);

        if let Some(mut pending) = self.state.five_hour_shadow.pending.take() {
            if acceptance.transitioned || after.segment_id != pending.origin_segment_id {
                changed = true;
            } else if let Some(used) = used {
                match pending.observe(observed_at, used, resets_at) {
                    PendingOutcome::Continue => {
                        changed = true;
                        self.state.five_hour_shadow.pending = Some(pending);
                    }
                    PendingOutcome::Duplicate => {
                        self.state.five_hour_shadow.pending = Some(pending);
                    }
                    PendingOutcome::Rejected => {
                        changed = true;
                    }
                    PendingOutcome::Matured(sample) => {
                        self.state.five_hour_shadow.affine.learn(
                            pending.origin_fixed_rate_per_hour,
                            &sample,
                            observed_at,
                        );
                        changed = true;
                    }
                }
            } else {
                self.state.five_hour_shadow.pending = Some(pending);
            }
        }

        let metadata_before = (
            self.state.five_hour_shadow.weighted.last_crossing_observation_at,
            self.state.five_hour_shadow.weighted.idle_rate_ceiling,
        );
        let fixed_rate = self.state.five_hour_shadow.weighted.publish(
            &after,
            FIVE_HOUR_HARD_EVIDENCE_HORIZON_SECS,
            FIVE_HOUR_MIN_LEVEL_SPAN,
            WeightedParameters::FIVE_HOUR,
            acceptance.accepted,
        );
        if metadata_before
            != (
                self.state.five_hour_shadow.weighted.last_crossing_observation_at,
                self.state.five_hour_shadow.weighted.idle_rate_ceiling,
            )
        {
            changed = true;
        }

        if acceptance.accepted && self.state.five_hour_shadow.pending.is_none() {
            if let Some(pending) =
                PendingInterval::build(observed_at, used, resets_at, fixed_rate, after.segment_id)
            {
                self.state.five_hour_shadow.pending = Some(pending);
                changed = true;
            }
        }

        (changed, fixed_rate)
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
) -> QuotaAcceptance {
    let before_observed_at = state.last_observed_at;
    let (changed, transitioned) =
        accept_quota_sample(state, snapshot, observed_at, max_gap_secs);
    QuotaAcceptance {
        changed,
        accepted: state.last_observed_at != before_observed_at,
        transitioned,
    }
}

fn accept_quota_sample(
    state: &mut QuotaRateState,
    snapshot: &QuotaSnapshot,
    observed_at: i64,
    max_gap_secs: i64,
) -> (bool, bool) {
    let Some(current) = valid_used_percent(snapshot) else {
        return (false, false);
    };

    let (boundary_changed, actual_cycle_transition) =
        accept_reset_boundary(state, snapshot.resets_at, observed_at);
    let mut changed = boundary_changed;
    if actual_cycle_transition {
        start_new_segment(state);
        set_baseline(state, current, observed_at);
        return (true, true);
    }

    let Some(previous_at) = state.last_observed_at else {
        set_baseline(state, current, observed_at);
        return (true, false);
    };
    let previous = state.last_observed_percent.unwrap_or(current);

    if observed_at < previous_at {
        return (changed, false);
    }
    if observed_at.saturating_sub(previous_at) > max_gap_secs {
        start_new_segment(state);
        set_baseline(state, current, observed_at);
        return (true, true);
    }

    if current + EPSILON < previous {
        if state.correction_candidate.is_some() {
            start_new_segment(state);
            set_baseline(state, current, observed_at);
            return (true, true);
        } else {
            state.correction_candidate = Some(CorrectionCandidate {
                used_percent: current,
                observed_at,
                resets_at: snapshot.resets_at,
            });
            return (true, false);
        }
    }

    if state.correction_candidate.take().is_some() {
        changed = true;
    }

    if observed_at == previous_at {
        return (changed, false);
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
    (true, false)
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
mod shadow_tests;

pub mod weighted;

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
            antigravity_weekly: None,
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
    fn missing_non_codex_identity_isolates_identified_and_temporary_history() {
        let mut tracker = BurnRateTracker::from_state(BurnRateStateFile::default());
        tracker.local_codex = false;
        tracker.accept_usage(&usage(1000, 10.0, 10.0));
        tracker.accept_usage(&usage(1060, 13.0, 12.0));
        let mut unknown = usage(1120, 50.0, 50.0);
        unknown.account_id = None;
        let estimate = tracker.accept_usage(&unknown);
        assert_eq!(estimate.five_hour_per_hour, None);
        assert_eq!(tracker.state.account_id, None);
        assert_eq!(tracker.candidates(&estimate).learning, "identityUnavailable");
        assert!(tracker.state.five_hour.crossings.is_empty());
        unknown.fetched_at = 1180;
        unknown.five_hour.used_percent = Some(53.0);
        tracker.accept_usage(&unknown);
        let restored = tracker.accept_usage(&usage(1240, 14.0, 14.0));
        assert_eq!(restored.five_hour_per_hour, None);
        assert_eq!(tracker.state.account_id.as_deref(), Some("acct-a"));
        assert!(tracker.state.five_hour.crossings.is_empty());
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
