use serde::{Deserialize, Serialize};
use std::fs;

use crate::config::ConfigManager;

use super::{QuotaSnapshot, RawCodexUsage};

use personal::{
    MaturedOutcome, PendingOutcome, PendingPersonalForecast, PersonalCalibrationState,
    RejectionReason,
};
use shadow::{
    append_evidence_event, compact_evidence_log, count_evidence_events, lifecycle_event,
    matured_event, rejected_event, FiveHourShadowEstimate, FiveHourShadowState, ShadowEvidenceEvent,
    MAX_EVIDENCE_EVENTS, SHADOW_ALGORITHM_VERSION,
};
use weighted::{
    WeightedParameters, WeightedShadowState, FIVE_HOUR_HARD_EVIDENCE_HORIZON_SECS,
    FIVE_HOUR_MIN_LEVEL_SPAN,
};

const SCHEMA_VERSION: u32 = 2;
const LEGACY_SCHEMA_VERSION: u32 = 1;
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

/// What the account transition did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccountTransition {
    /// The account did not change.
    None,
    /// The account was attributed for the first time.
    Attributed,
    /// A confirmed account switch cleared the per-account state.
    Switched,
}

/// The personal outcome of one observation.
enum PersonalOutcome {
    /// The interval matured and trained the learner.
    Matured {
        pending: PendingPersonalForecast,
        matured: MaturedOutcome,
        before: PersonalCalibrationState,
    },
    /// The interval was rejected without learning.
    Rejected {
        pending: PendingPersonalForecast,
        reason: RejectionReason,
    },
}

/// The learner values that are stored with a pending interval.
#[derive(Clone, Copy, Debug)]
struct LearnerSnapshot {
    raw_factor: f64,
    trust: f64,
    effective_factor: f64,
    evidence: f64,
    informative_updates: u64,
}

/// What one accepted quota sample did to the history.
///
/// The classification is the transition signal the shadow path needs: it
/// tells a real quota cycle, an overnight resume and a confirmed correction
/// apart, and it records whether the production baseline advanced, which is
/// the definition of an accepted observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuotaAcceptance {
    /// The persisted state changed.
    changed: bool,
    /// The production baseline advanced.
    accepted: bool,
    /// The transition that ended the previous segment, if any.
    transition: Option<SegmentTransition>,
}

/// The reason a new quota segment started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SegmentTransition {
    /// The quota cycle completed.
    QuotaCycle,
    /// The observation gap exceeded the production maximum.
    LongIdle,
    /// A decrease was confirmed by a later observation.
    Correction,
}

impl SegmentTransition {
    /// The stable evidence-log code of the transition.
    fn code(&self) -> &'static str {
        match self {
            Self::QuotaCycle => "quota_cycle",
            Self::LongIdle => "long_idle",
            Self::Correction => "correction",
        }
    }

    /// The stable rejection reason of a pending interval the transition
    /// invalidated.
    fn rejection_reason(&self) -> RejectionReason {
        match self {
            Self::QuotaCycle => RejectionReason::QuotaCycle,
            Self::LongIdle => RejectionReason::LongIdle,
            Self::Correction => RejectionReason::Correction,
        }
    }
}

/// The result of one shadow pass.
struct ShadowPass {
    state_changed: bool,
    estimate: FiveHourShadowEstimate,
}

pub struct BurnRateTracker {
    state: BurnRateStateFile,
    /// The in-memory state is newer than the persisted copy.
    state_dirty: bool,
    /// How many records the local evidence log holds.
    evidence_lines: usize,
    /// The last shadow diagnostic. It never reaches the frontend.
    shadow_estimate: FiveHourShadowEstimate,
}

impl BurnRateTracker {
    pub fn load(config: &ConfigManager) -> Self {
        let state = fs::read_to_string(config.burn_rate_state_path())
            .ok()
            .map(|raw| load_state(&raw))
            .unwrap_or_default();
        let evidence_lines = count_evidence_events(&config.shadow_evidence_path());
        let mut tracker = Self {
            state,
            state_dirty: false,
            evidence_lines,
            shadow_estimate: FiveHourShadowEstimate::default(),
        };
        // A retained audit event is flushed before any new feedback is
        // processed, so a crash between the learner update and the log append
        // does not lose the outcome.
        if let Err(error) = tracker.flush_audit_outbox(config) {
            eprintln!("Could not append the retained shadow evidence: {error}");
        }
        tracker
    }

    #[cfg(test)]
    pub fn from_state(state: BurnRateStateFile) -> Self {
        Self {
            state,
            state_dirty: false,
            evidence_lines: 0,
            shadow_estimate: FiveHourShadowEstimate::default(),
        }
    }

    pub fn save(&mut self, config: &ConfigManager) -> Result<(), String> {
        let path = config.burn_rate_state_path();
        let tmp = path.with_extension("json.tmp");
        let serialized = serde_json::to_vec_pretty(&self.state).map_err(|e| e.to_string())?;
        fs::write(&tmp, serialized).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        // The state is durable now, so a retained audit event may be appended.
        self.state_dirty = false;
        Ok(())
    }

    /// The last shadow diagnostic of the tracker.
    ///
    /// It is internal evidence and never reaches the frontend. The shadow
    /// phase deliberately publishes nothing, so the accessor is only called
    /// by the offline tests that audit the learner.
    #[allow(dead_code)]
    pub fn shadow_estimate(&self) -> &FiveHourShadowEstimate {
        &self.shadow_estimate
    }

    /// Whether the persisted copy is older than the in-memory state.
    ///
    /// It stays true after a failed write, so the next refresh retries the
    /// persistence instead of silently dropping the change.
    pub fn is_dirty(&self) -> bool {
        self.state_dirty
    }

    /// Appends a retained audit event to the local evidence log.
    ///
    /// The event is only appended when the learner state that produced it is
    /// durable. A failed append keeps the event in the slot, so the next
    /// refresh retries it and no personal outcome is lost.
    pub fn flush_audit_outbox(&mut self, config: &ConfigManager) -> Result<(), String> {
        let Some(event) = self.state.five_hour_shadow.audit_outbox.clone() else {
            return Ok(());
        };
        if self.state_dirty {
            // The learner state behind the event is not durable yet.
            return Err("the burn-rate state is not persisted".to_string());
        }
        append_evidence_event(&config.shadow_evidence_path(), &event)?;
        self.evidence_lines += 1;
        if self.evidence_lines > MAX_EVIDENCE_EVENTS {
            self.evidence_lines = compact_evidence_log(&config.shadow_evidence_path())?;
        }
        // The slot is cleared in memory; the persisted copy is replaced with
        // the next state-changing save.
        self.state.five_hour_shadow.audit_outbox = None;
        Ok(())
    }

    pub fn accept_usage(&mut self, usage: &RawCodexUsage) -> BurnRateEstimate {
        let mut changed = false;

        // The weighted metadata is repaired against the production history
        // before the incoming observation advances the baseline.
        changed |= self
            .state
            .five_hour_shadow
            .normalize(&self.state.five_hour);

        let account_transition = self.accept_account(usage.account_id.as_deref());
        changed |= account_transition != AccountTransition::None;

        // The shadow path needs the pre-accept state, so the transition it
        // observes is the one the production tracker just made.
        let before = self.state.five_hour.clone();
        let acceptance = accept_quota(
            &mut self.state.five_hour,
            &usage.five_hour,
            usage.fetched_at,
            FIVE_HOUR_MAX_GAP_SECS,
        );
        changed |= acceptance.changed;
        let five_hour_per_hour = valid_used_percent(&usage.five_hour)
            .and_then(|_| estimate_rate(&self.state.five_hour, FIVE_HOUR_HORIZON_SECS, 2));

        let shadow = self.process_five_hour_shadow(
            usage,
            &before,
            acceptance,
            five_hour_per_hour,
            account_transition,
        );
        changed |= shadow.state_changed;
        self.shadow_estimate = shadow.estimate;

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

    fn accept_account(&mut self, incoming: Option<&str>) -> AccountTransition {
        let Some(incoming) = incoming.filter(|id| !id.is_empty()) else {
            return AccountTransition::None;
        };
        if self.state.account_id.as_deref() == Some(incoming) {
            return AccountTransition::None;
        }

        let had_attributed_or_unattributed_history = self.state.account_id.is_some()
            || self.state.five_hour.last_observed_at.is_some()
            || self.state.weekly.last_observed_at.is_some();
        self.state.account_id = Some(incoming.to_string());
        if !had_attributed_or_unattributed_history {
            return AccountTransition::Attributed;
        }

        self.state.five_hour = QuotaRateState::default();
        self.state.weekly = QuotaRateState::default();
        // No personal calibration and no audit identity may cross accounts.
        self.state.five_hour_shadow.weighted = WeightedShadowState::default();
        self.state.five_hour_shadow.personal = PersonalCalibrationState::default();
        self.state.five_hour_shadow.pending = None;
        self.state.five_hour_shadow.account_generation =
            self.state.five_hour_shadow.account_generation.saturating_add(1);
        self.state.five_hour_shadow.event_sequence = 0;
        AccountTransition::Switched
    }

    /// Runs the shadow pass of one accepted observation.
    ///
    /// The order is the validated causal order: the weighted metadata
    /// transition, the personal outcome, the current Fixed Weighted rate, the
    /// personalized rate and finally the new origin snapshot. Nothing here
    /// reaches the frontend.
    fn process_five_hour_shadow(
        &mut self,
        usage: &RawCodexUsage,
        before: &QuotaRateState,
        acceptance: QuotaAcceptance,
        published_rate: Option<f64>,
        account_transition: AccountTransition,
    ) -> ShadowPass {
        let observed_at = usage.fetched_at;
        let mut changed = false;
        let after = self.state.five_hour.clone();
        let used = valid_used_percent(&usage.five_hour);
        let resets_at = usage.five_hour.resets_at.filter(|value| *value > 0);
        let transition = acceptance.transition;

        // The Fixed Weighted metadata follows the production crossing history.
        if self
            .state
            .five_hour_shadow
            .weighted
            .apply_transition(before, &after, observed_at)
        {
            changed = true;
        }

        // The personal outcome is paused while an audit event is still waiting
        // for the durable evidence log. The weighted estimator keeps running.
        let audit_busy = self.state.five_hour_shadow.audit_outbox.is_some();
        let mut outcome: Option<PersonalOutcome> = None;

        if !audit_busy {
            if let Some(transition) = transition {
                // A confirmed transition invalidates the pending interval
                // before it can train.
                if let Some(pending) = self.state.five_hour_shadow.pending.take() {
                    outcome = Some(PersonalOutcome::Rejected {
                        pending,
                        reason: transition.rejection_reason(),
                    });
                    changed = true;
                }
            } else if let Some(mut pending) = self.state.five_hour_shadow.pending.take() {
                match used {
                    Some(used) => match pending.observe(observed_at, used, resets_at) {
                        PendingOutcome::Continue => {
                            changed = true;
                            self.state.five_hour_shadow.pending = Some(pending);
                        }
                        PendingOutcome::Duplicate => {
                            self.state.five_hour_shadow.pending = Some(pending);
                        }
                        PendingOutcome::Rejected(reason) => {
                            outcome = Some(PersonalOutcome::Rejected { pending, reason });
                            changed = true;
                        }
                        PendingOutcome::Matured(matured) => {
                            let learner_before = self.state.five_hour_shadow.personal.clone();
                            self.state.five_hour_shadow.personal.apply_outcome(
                                matured.confidence,
                                matured.predicted_delta,
                                matured.observed_delta,
                                observed_at,
                            );
                            outcome = Some(PersonalOutcome::Matured {
                                pending,
                                matured,
                                before: learner_before,
                            });
                            changed = true;
                        }
                    },
                    None => {
                        // A missing sample neither advances nor rejects the
                        // interval; the next valid observation is compared with
                        // the last valid one.
                        self.state.five_hour_shadow.pending = Some(pending);
                    }
                }
            }
        }

        // The current Fixed Weighted rate, published after the outcome.
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

        // The personalized rate, from the post-outcome learner state.
        let (learner, raw_factor, trust, effective_factor) = {
            let personal = &self.state.five_hour_shadow.personal;
            (
                LearnerSnapshot {
                    raw_factor: personal.raw_factor(),
                    trust: personal.trust(),
                    effective_factor: personal.effective_factor(),
                    evidence: personal.evidence,
                    informative_updates: personal.informative_updates,
                },
                personal.raw_factor(),
                personal.trust(),
                personal.effective_factor(),
            )
        };
        let personalized = fixed_rate.map(|rate| rate * effective_factor);

        // The evidence event is formed only now, so it carries the state that
        // the outcome produced. A retained event is never overwritten: the
        // audit slot holds one event, and a second one would destroy a record
        // whose learner update already survived.
        if self.state.five_hour_shadow.audit_outbox.is_none() {
            let event = match outcome {
                Some(PersonalOutcome::Matured {
                    pending,
                    matured,
                    before: learner_before,
                }) => Some(matured_event(
                    &pending,
                    &matured,
                    &learner_before,
                    &self.state.five_hour_shadow.personal,
                    observed_at,
                    personalized,
                )),
                Some(PersonalOutcome::Rejected { pending, reason }) => Some(rejected_event(
                    &pending,
                    reason,
                    observed_at,
                    used,
                    // A backwards observation has no gap at all.
                    (observed_at >= pending.last_valid_at)
                        .then(|| observed_at - pending.last_valid_at),
                )),
                None => lifecycle_event_for(account_transition, transition, observed_at),
            };
            if let Some(mut event) = event {
                self.state.five_hour_shadow.event_sequence =
                    self.state.five_hour_shadow.event_sequence.saturating_add(1);
                event.account_generation = self.state.five_hour_shadow.account_generation;
                event.event_sequence = self.state.five_hour_shadow.event_sequence;
                self.state.five_hour_shadow.audit_outbox = Some(event);
                changed = true;
            }
        }

        // A finished interval is replaced by a fresh origin at the same
        // observation. The production must have accepted the observation, so
        // the origin is never built from a sample the burn-rate history
        // refused.
        if !audit_busy
            && acceptance.accepted
            && self.state.five_hour_shadow.pending.is_none()
        {
            if let Some(pending) = build_pending(
                observed_at,
                used,
                resets_at,
                published_rate,
                fixed_rate,
                personalized,
                learner,
            ) {
                self.state.five_hour_shadow.pending = Some(pending);
                changed = true;
            }
        }

        ShadowPass {
            state_changed: changed,
            estimate: FiveHourShadowEstimate {
                fixed_weighted_per_hour: fixed_rate,
                personalized_per_hour: personalized,
                raw_factor: fixed_rate.map(|_| raw_factor),
                trust,
                effective_factor,
                evidence: learner.evidence,
                matured_feedbacks: self.state.five_hour_shadow.personal.matured_feedbacks,
                informative_updates: self.state.five_hour_shadow.personal.informative_updates,
            },
        }
    }
}

/// The lifecycle evidence of an observation without a personal outcome.
fn lifecycle_event_for(
    account_transition: AccountTransition,
    transition: Option<SegmentTransition>,
    at: i64,
) -> Option<ShadowEvidenceEvent> {
    if account_transition == AccountTransition::Switched {
        return Some(lifecycle_event("account_switch", at));
    }
    transition.map(|transition| lifecycle_event(transition.code(), at))
}

/// Builds the origin snapshot of a new five-minute learning interval.
///
/// The snapshot is causal: it stores the rates that were published at this
/// observation, so the matured outcome can score them later without
/// reconstructing them from the target time.
fn build_pending(
    origin_at: i64,
    used: Option<f64>,
    resets_at: Option<i64>,
    published_rate: Option<f64>,
    fixed_rate: Option<f64>,
    personalized: Option<f64>,
    learner: LearnerSnapshot,
) -> Option<PendingPersonalForecast> {
    let used = used?;
    if used >= 100.0 {
        return None;
    }
    let resets_at = resets_at?;
    let fixed_rate = fixed_rate?;
    if !fixed_rate.is_finite() || fixed_rate <= 0.0 {
        return None;
    }
    if origin_at <= 0 {
        return None;
    }
    Some(PendingPersonalForecast {
        algorithm_version: SHADOW_ALGORITHM_VERSION,
        origin_at,
        origin_used_percent: used,
        origin_reset_at: resets_at,
        origin_published_rate_per_hour: published_rate,
        origin_fixed_rate_per_hour: fixed_rate,
        origin_raw_factor: learner.raw_factor,
        origin_trust: learner.trust,
        origin_effective_factor: learner.effective_factor,
        origin_shadow_rate_per_hour: personalized,
        origin_evidence: learner.evidence,
        origin_informative_updates: learner.informative_updates,
        last_valid_at: origin_at,
        last_valid_used_percent: used,
    })
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
    let (changed, transition) =
        accept_quota_sample(state, snapshot, observed_at, max_gap_secs);
    QuotaAcceptance {
        changed,
        // The observation was accepted when the production baseline advanced.
        accepted: state.last_observed_at != before_observed_at,
        transition,
    }
}

/// Applies one quota sample and classifies the transition it caused.
///
/// The state semantics are unchanged; only the transition cause is reported
/// alongside the changed flag.
fn accept_quota_sample(
    state: &mut QuotaRateState,
    snapshot: &QuotaSnapshot,
    observed_at: i64,
    max_gap_secs: i64,
) -> (bool, Option<SegmentTransition>) {
    let Some(current) = valid_used_percent(snapshot) else {
        return (false, None);
    };

    let (boundary_changed, actual_cycle_transition) =
        accept_reset_boundary(state, snapshot.resets_at, observed_at);
    let mut changed = boundary_changed;
    if actual_cycle_transition {
        start_new_segment(state);
        set_baseline(state, current, observed_at);
        return (true, Some(SegmentTransition::QuotaCycle));
    }

    let Some(previous_at) = state.last_observed_at else {
        set_baseline(state, current, observed_at);
        return (true, None);
    };
    let previous = state.last_observed_percent.unwrap_or(current);

    if observed_at < previous_at {
        return (changed, None);
    }
    if observed_at.saturating_sub(previous_at) > max_gap_secs {
        start_new_segment(state);
        set_baseline(state, current, observed_at);
        return (true, Some(SegmentTransition::LongIdle));
    }

    if current + EPSILON < previous {
        if state.correction_candidate.is_some() {
            start_new_segment(state);
            set_baseline(state, current, observed_at);
            return (true, Some(SegmentTransition::Correction));
        } else {
            state.correction_candidate = Some(CorrectionCandidate {
                used_percent: current,
                observed_at,
                resets_at: snapshot.resets_at,
            });
            return (true, None);
        }
    }

    if state.correction_candidate.take().is_some() {
        changed = true;
    }

    if observed_at == previous_at {
        if (current - previous).abs() <= EPSILON {
            return (changed, None);
        }
        return (changed, None);
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
    (true, None)
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
mod shadow_tests;

pub mod personal;
pub mod shadow;
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
