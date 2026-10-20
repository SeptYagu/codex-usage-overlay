//! The shadow state tree, the audit outbox and the local evidence log.
//!
//! The shadow phase collects evidence for a later publication decision. It
//! never changes a user-visible value, and it never leaves the audit trail
//! behind the learner state: a matured outcome is durable before it is
//! appended, and an unappended event is never overwritten.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::personal::{
    MaturedOutcome, PendingPersonalForecast, PersonalCalibrationState, RejectionReason,
};
use super::weighted::WeightedShadowState;
use super::QuotaRateState;

/// The algorithm version of the shadow learner.
///
/// A change to a frozen learner constant, the confidence rule, the forecast
/// horizon, the trust rule or the base-estimator semantics must bump this
/// version. A persisted state with a different version is isolated instead of
/// being mixed with evidence generated under another algorithm.
pub const SHADOW_ALGORITHM_VERSION: u32 = 1;
/// The schema version of one evidence record.
pub const EVIDENCE_SCHEMA_VERSION: u32 = 1;
/// The maximum number of retained evidence events.
pub const MAX_EVIDENCE_EVENTS: usize = 10_000;

/// The kind of one evidence record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShadowEventKind {
    /// A five-minute interval matured and trained the learner.
    Matured,
    /// A five-minute interval was rejected without learning.
    Rejected,
    /// A lifecycle transition cleared part of the shadow state.
    Lifecycle,
}

/// One diagnostic evidence record.
///
/// The record carries enough information to replay the learner offline. The
/// raw account identifier is never part of it: the audit identity is the
/// account generation and the event sequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowEvidenceEvent {
    /// The schema version of the record.
    pub schema_version: u32,
    /// The algorithm version that produced the record.
    pub algorithm_version: u32,
    /// The kind of the record.
    pub kind: ShadowEventKind,
    /// The account generation of the record.
    pub account_generation: u64,
    /// The event sequence of the record.
    pub event_sequence: u64,
    /// When the record was produced.
    pub at: i64,
    /// The origin of the interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_at: Option<i64>,
    /// The target of the interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_at: Option<i64>,
    /// The elapsed time of the interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<i64>,
    /// The used percentage at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_used: Option<f64>,
    /// The used percentage at the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_used: Option<f64>,
    /// The published rate at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_published_rate: Option<f64>,
    /// The Fixed Weighted rate at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_fixed_rate: Option<f64>,
    /// The personalized rate at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_shadow_rate: Option<f64>,
    /// The published prediction of the interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_predicted_delta: Option<f64>,
    /// The Fixed Weighted prediction of the interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_predicted_delta: Option<f64>,
    /// The personalized prediction of the interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_predicted_delta: Option<f64>,
    /// The observed quota-point increase of the interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_delta: Option<f64>,
    /// The raw personal factor at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_raw_factor: Option<f64>,
    /// The trust at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_trust: Option<f64>,
    /// The effective factor at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_effective_factor: Option<f64>,
    /// The evidence at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_evidence: Option<f64>,
    /// The informative update count at the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_informative_updates: Option<u64>,
    /// The feedback confidence of the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<f64>,
    /// The learner numerator before the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a_before: Option<f64>,
    /// The learner denominator before the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_before: Option<f64>,
    /// The trust evidence before the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_before: Option<f64>,
    /// The learner numerator after the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a_after: Option<f64>,
    /// The learner denominator after the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_after: Option<f64>,
    /// The trust evidence after the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_after: Option<f64>,
    /// The matured counter after the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matured_feedbacks_after: Option<u64>,
    /// The informative counter after the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub informative_updates_after: Option<u64>,
    /// The raw personal factor at the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_raw_factor: Option<f64>,
    /// The trust at the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_trust: Option<f64>,
    /// The effective factor at the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_effective_factor: Option<f64>,
    /// The personalized rate at the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_shadow_rate: Option<f64>,
    /// The stable rejection code of a rejected interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    /// The observed gap that caused a rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_gap_seconds: Option<i64>,
    /// The lifecycle transition that a lifecycle record describes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
}

impl ShadowEvidenceEvent {
    /// The common part of every evidence record.
    ///
    /// The audit identity is filled in by the tracker when the record is
    /// placed into the outbox, because only the tracker owns the sequence.
    fn base(kind: ShadowEventKind, at: i64) -> Self {
        Self {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            algorithm_version: SHADOW_ALGORITHM_VERSION,
            kind,
            account_generation: 0,
            event_sequence: 0,
            at,
            origin_at: None,
            target_at: None,
            elapsed_seconds: None,
            origin_used: None,
            target_used: None,
            origin_published_rate: None,
            origin_fixed_rate: None,
            origin_shadow_rate: None,
            published_predicted_delta: None,
            fixed_predicted_delta: None,
            shadow_predicted_delta: None,
            observed_delta: None,
            origin_raw_factor: None,
            origin_trust: None,
            origin_effective_factor: None,
            origin_evidence: None,
            origin_informative_updates: None,
            q: None,
            a_before: None,
            b_before: None,
            evidence_before: None,
            a_after: None,
            b_after: None,
            evidence_after: None,
            matured_feedbacks_after: None,
            informative_updates_after: None,
            target_raw_factor: None,
            target_trust: None,
            target_effective_factor: None,
            target_shadow_rate: None,
            rejection_reason: None,
            actual_gap_seconds: None,
            lifecycle: None,
        }
    }
}

/// Builds the evidence record of a matured five-minute outcome.
///
/// The record carries the origin snapshot, the three causal predictions that
/// were made at the origin, the observed increase and the learner state
/// before and after the update. The learner's own prediction is always the
/// Fixed Weighted one.
pub fn matured_event(
    pending: &PendingPersonalForecast,
    matured: &MaturedOutcome,
    before: &PersonalCalibrationState,
    after: &PersonalCalibrationState,
    target_at: i64,
    target_shadow_rate: Option<f64>,
) -> ShadowEvidenceEvent {
    ShadowEvidenceEvent {
        origin_at: Some(pending.origin_at),
        target_at: Some(target_at),
        elapsed_seconds: Some(matured.elapsed_seconds),
        origin_used: Some(pending.origin_used_percent),
        target_used: Some(pending.origin_used_percent + matured.observed_delta),
        origin_published_rate: pending.origin_published_rate_per_hour,
        origin_fixed_rate: Some(pending.origin_fixed_rate_per_hour),
        origin_shadow_rate: pending.origin_shadow_rate_per_hour,
        published_predicted_delta: matured.published_predicted_delta,
        fixed_predicted_delta: Some(matured.fixed_predicted_delta),
        shadow_predicted_delta: matured.shadow_predicted_delta,
        observed_delta: Some(matured.observed_delta),
        origin_raw_factor: Some(pending.origin_raw_factor),
        origin_trust: Some(pending.origin_trust),
        origin_effective_factor: Some(pending.origin_effective_factor),
        origin_evidence: Some(pending.origin_evidence),
        origin_informative_updates: Some(pending.origin_informative_updates),
        q: Some(matured.confidence),
        a_before: Some(before.a),
        b_before: Some(before.b),
        evidence_before: Some(before.evidence),
        a_after: Some(after.a),
        b_after: Some(after.b),
        evidence_after: Some(after.evidence),
        matured_feedbacks_after: Some(after.matured_feedbacks),
        informative_updates_after: Some(after.informative_updates),
        target_raw_factor: Some(after.raw_factor()),
        target_trust: Some(after.trust()),
        target_effective_factor: Some(after.effective_factor()),
        target_shadow_rate,
        ..ShadowEvidenceEvent::base(ShadowEventKind::Matured, target_at)
    }
}

/// Builds the evidence record of a rejected five-minute outcome.
///
/// A rejected interval never changes the learner, so the record carries only
/// the origin snapshot, the stable rejection code and the timing metadata.
pub fn rejected_event(
    pending: &PendingPersonalForecast,
    reason: RejectionReason,
    target_at: i64,
    target_used: Option<f64>,
    actual_gap_seconds: Option<i64>,
) -> ShadowEvidenceEvent {
    ShadowEvidenceEvent {
        origin_at: Some(pending.origin_at),
        target_at: Some(target_at),
        origin_used: Some(pending.origin_used_percent),
        target_used,
        origin_published_rate: pending.origin_published_rate_per_hour,
        origin_fixed_rate: Some(pending.origin_fixed_rate_per_hour),
        origin_shadow_rate: pending.origin_shadow_rate_per_hour,
        origin_raw_factor: Some(pending.origin_raw_factor),
        origin_trust: Some(pending.origin_trust),
        origin_effective_factor: Some(pending.origin_effective_factor),
        origin_evidence: Some(pending.origin_evidence),
        origin_informative_updates: Some(pending.origin_informative_updates),
        rejection_reason: Some(reason.code().to_string()),
        actual_gap_seconds,
        ..ShadowEvidenceEvent::base(ShadowEventKind::Rejected, target_at)
    }
}

/// Builds the evidence record of a lifecycle transition.
pub fn lifecycle_event(lifecycle: &str, at: i64) -> ShadowEvidenceEvent {
    ShadowEvidenceEvent {
        lifecycle: Some(lifecycle.to_string()),
        ..ShadowEvidenceEvent::base(ShadowEventKind::Lifecycle, at)
    }
}

/// The persisted shadow subtree of the burn-rate state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FiveHourShadowState {
    /// The algorithm version the subtree belongs to.
    pub algorithm_version: u32,
    /// The generation of the current account.
    pub account_generation: u64,
    /// The sequence of the audit events of the current generation.
    pub event_sequence: u64,
    /// The event that still has to reach the durable evidence log.
    pub audit_outbox: Option<ShadowEvidenceEvent>,
    /// The metadata of the Fixed Weighted estimator.
    pub weighted: WeightedShadowState,
    /// The personal calibration learner.
    pub personal: PersonalCalibrationState,
    /// The pending five-minute interval.
    pub pending: Option<PendingPersonalForecast>,
}

impl Default for FiveHourShadowState {
    fn default() -> Self {
        Self {
            algorithm_version: SHADOW_ALGORITHM_VERSION,
            account_generation: 0,
            event_sequence: 0,
            audit_outbox: None,
            weighted: WeightedShadowState::default(),
            personal: PersonalCalibrationState::default(),
            pending: None,
        }
    }
}

impl FiveHourShadowState {
    /// Isolates a state that was produced by a different algorithm version.
    ///
    /// The legacy quota history and the audit identity survive; the
    /// incompatible personal, weighted and pending state returns to neutral
    /// defaults instead of being mixed with foreign evidence.
    pub fn isolate_algorithm_version(&mut self) -> bool {
        if self.algorithm_version == SHADOW_ALGORITHM_VERSION {
            return false;
        }
        self.algorithm_version = SHADOW_ALGORITHM_VERSION;
        self.weighted = WeightedShadowState::default();
        self.personal = PersonalCalibrationState::default();
        self.pending = None;
        true
    }

    /// Repairs a corrupt subtree.
    ///
    /// The weighted metadata is repaired against the production quota
    /// history, a corrupt learner resets only the personal subtree, and the
    /// pending interval is dropped when it cannot be resumed safely.
    pub fn normalize(&mut self, quota: &QuotaRateState) -> bool {
        let before = self.clone();
        self.weighted.normalize(quota);
        self.personal.normalize();
        if let Some(pending) = &mut self.pending {
            if pending.algorithm_version != SHADOW_ALGORITHM_VERSION
                || !pending.origin_fixed_rate_per_hour.is_finite()
                || pending.origin_fixed_rate_per_hour <= 0.0
                || !pending.origin_used_percent.is_finite()
                || !pending.last_valid_used_percent.is_finite()
                || pending.origin_at <= 0
                || pending.last_valid_at < pending.origin_at
            {
                self.pending = None;
            }
        }
        before != *self
    }
}

/// The internal diagnostic result of one refresh.
///
/// It is never part of the frontend contract.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FiveHourShadowEstimate {
    /// The Fixed Weighted base rate.
    pub fixed_weighted_per_hour: Option<f64>,
    /// The personalized rate.
    pub personalized_per_hour: Option<f64>,
    /// The clamped raw personal factor.
    pub raw_factor: Option<f64>,
    /// The trust of the current evidence.
    pub trust: f64,
    /// The factor that is applied to the base rate.
    pub effective_factor: f64,
    /// The decayed trust evidence.
    pub evidence: f64,
    /// How many intervals matured.
    pub matured_feedbacks: u64,
    /// How many matured intervals carried information.
    pub informative_updates: u64,
}

impl Default for FiveHourShadowEstimate {
    fn default() -> Self {
        Self {
            fixed_weighted_per_hour: None,
            personalized_per_hour: None,
            raw_factor: None,
            trust: 0.0,
            effective_factor: 1.0,
            evidence: 0.0,
            matured_feedbacks: 0,
            informative_updates: 0,
        }
    }
}

/// Counts the records of an evidence log.
pub fn count_evidence_events(path: &Path) -> usize {
    match fs::read_to_string(path) {
        Ok(content) => content.lines().filter(|line| !line.trim().is_empty()).count(),
        Err(_) => 0,
    }
}

/// Appends one event to the local evidence log.
///
/// The log is diagnostic evidence, not telemetry. A failure is reported to the
/// caller and must never fail a usage refresh.
pub fn append_evidence_event(path: &Path, event: &ShadowEvidenceEvent) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut line = serde_json::to_string(event).map_err(|error| error.to_string())?;
    line.push('\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(line.as_bytes())
        .map_err(|error| error.to_string())
}

/// Keeps only the newest events of an oversized evidence log.
pub fn compact_evidence_log(path: &Path) -> Result<usize, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut lines: Vec<&str> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() <= MAX_EVIDENCE_EVENTS {
        return Ok(lines.len());
    }
    let excess = lines.len() - MAX_EVIDENCE_EVENTS;
    lines.drain(0..excess);
    let retained = lines.len();
    let mut rebuilt = String::new();
    for line in lines {
        rebuilt.push_str(line);
        rebuilt.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, rebuilt.as_bytes()).map_err(|error| error.to_string())?;
    fs::rename(&tmp, path).map_err(|error| error.to_string())?;
    Ok(retained)
}
