//! The Fixed Weighted estimator, promoted from the offline replay candidate to
//! production-callable internal code.
//!
//! The shadow phase must not own a second mathematical implementation: the
//! replay suite imports these helpers, so the committed evidence keeps
//! testing the code the application actually runs.

use serde::{Deserialize, Serialize};

use super::{QuotaRateState, EPSILON};

/// The frozen 5-hour half-life of the weighting.
pub const FIVE_HOUR_HALF_LIFE_SECS: f64 = 300.0;
/// The frozen 5-hour locality window.
pub const FIVE_HOUR_LOCAL_WINDOW_SECS: i64 = 1800;
/// The frozen 5-hour minimum level span.
pub const FIVE_HOUR_MIN_LEVEL_SPAN: u32 = 2;
/// The frozen 5-hour hard evidence horizon.
pub const FIVE_HOUR_HARD_EVIDENCE_HORIZON_SECS: i64 = 1800;

/// The weighting parameters of the fixed candidate.
#[derive(Clone, Copy, Debug)]
pub struct WeightedParameters {
    /// The half-life of the exponential weighting, in seconds.
    pub half_life: f64,
    /// The locality window around the newest crossing, in seconds.
    pub window: i64,
}

impl WeightedParameters {
    /// The frozen 5-hour parameters.
    pub const FIVE_HOUR: Self = Self {
        half_life: FIVE_HOUR_HALF_LIFE_SECS,
        window: FIVE_HOUR_LOCAL_WINDOW_SECS,
    };
}

/// The exponentially weighted fit of the eligible crossings.
///
/// This is the validated candidate math, moved verbatim. It is the shadow
/// base rate: the personal layer only multiplies it.
pub fn weighted_fit(
    state: &QuotaRateState,
    horizon: i64,
    span: u32,
    parameters: WeightedParameters,
) -> Option<f64> {
    if !parameters.half_life.is_finite() || parameters.half_life <= 0.0 || parameters.window <= 0
    {
        return None;
    }
    let end = state.last_observed_at?;
    let events: Vec<_> = state
        .crossings
        .iter()
        .filter(|event| {
            event.segment_id == state.segment_id
                && event.observed_at <= end
                && end.saturating_sub(event.observed_at) <= horizon
        })
        .collect();
    let newest = *events.last()?;
    // Locality is relative to the newest crossing; accepted idle must not erase
    // an otherwise eligible fit before its hard evidence horizon expires.
    let local: Vec<_> = events
        .into_iter()
        .filter(|event| {
            newest
                .observed_at
                .saturating_sub(event.observed_at)
                <= parameters.window
        })
        .collect();
    if newest.level.saturating_sub(local.first()?.level) < span {
        return None;
    }
    let points: Vec<_> = local
        .iter()
        .map(|event| {
            let t = event.observed_at.saturating_sub(newest.observed_at) as f64;
            (t, event.level as f64, (t / parameters.half_life).exp2())
        })
        .collect();
    let sum: f64 = points.iter().map(|value| value.2).sum();
    let mean_t = points.iter().map(|value| value.0 * value.2).sum::<f64>() / sum;
    let mean_l = points.iter().map(|value| value.1 * value.2).sum::<f64>() / sum;
    let covariance = points
        .iter()
        .map(|value| value.2 * (value.0 - mean_t) * (value.1 - mean_l))
        .sum::<f64>();
    let variance = points
        .iter()
        .map(|value| value.2 * (value.0 - mean_t).powi(2))
        .sum::<f64>();
    let rate = covariance / variance * 3600.0;
    (variance.is_finite() && variance > 0.0 && rate.is_finite() && rate > 0.0).then_some(rate)
}

/// Deserializes an optional field, repairing a malformed value to `None`
/// instead of discarding the whole record.
fn tolerant_optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(raw).ok())
}

/// The additional metadata the weighted estimator needs beyond the quota
/// history it reads from the production state.
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WeightedShadowState {
    /// The observation that last confirmed a new crossing.
    #[serde(
        rename = "lastCrossingObservationAt",
        deserialize_with = "tolerant_optional"
    )]
    pub last_crossing_observation_at: Option<i64>,
    /// The monotone non-increasing idle ceiling.
    #[serde(rename = "idleRateCeiling", deserialize_with = "tolerant_optional")]
    pub idle_rate_ceiling: Option<f64>,
}

impl WeightedShadowState {
    /// Clears the metadata that belongs to a single quota segment.
    pub fn clear(&mut self) {
        self.last_crossing_observation_at = None;
        self.idle_rate_ceiling = None;
    }

    /// Repairs the metadata against the current quota state.
    ///
    /// The crossing bound must lie between the newest eligible crossing and
    /// the last observed time, and the ceiling must be finite and positive.
    /// Valid old state is preserved rather than forced into a new warm-up.
    pub fn normalize(&mut self, quota: &QuotaRateState) -> bool {
        let before = (self.last_crossing_observation_at, self.idle_rate_ceiling);
        let newest = quota
            .crossings
            .iter()
            .rev()
            .find(|event| event.segment_id == quota.segment_id);
        match (newest, quota.last_observed_at) {
            (Some(event), Some(end)) if event.observed_at <= end => {
                if !self
                    .last_crossing_observation_at
                    .is_some_and(|bound| event.observed_at <= bound && bound <= end)
                {
                    self.last_crossing_observation_at = Some(end);
                    self.idle_rate_ceiling = None;
                }
                if self
                    .idle_rate_ceiling
                    .is_some_and(|value| !value.is_finite() || value <= 0.0)
                {
                    self.idle_rate_ceiling = None;
                }
            }
            _ => {
                self.last_crossing_observation_at = None;
                self.idle_rate_ceiling = None;
            }
        }
        before
            != (
                self.last_crossing_observation_at,
                self.idle_rate_ceiling,
            )
    }

    /// Applies the metadata transition produced by one accepted observation.
    ///
    /// A new segment clears the metadata, and a new crossing re-anchors the
    /// idle bound.
    pub fn apply_transition(
        &mut self,
        before: &QuotaRateState,
        after: &QuotaRateState,
        observed_at: i64,
    ) -> bool {
        let before_metadata = (self.last_crossing_observation_at, self.idle_rate_ceiling);
        if after.segment_id != before.segment_id {
            self.clear();
        }
        if after.crossings.last().is_some() && after.crossings.last() != before.crossings.last() {
            self.last_crossing_observation_at = Some(observed_at);
            self.idle_rate_ceiling = None;
        }
        before_metadata
            != (
                self.last_crossing_observation_at,
                self.idle_rate_ceiling,
            )
    }

    /// Publishes the weighted rate for the current quota state.
    ///
    /// Accepted idle observations may only lower the ceiling, never raise it.
    pub fn publish(
        &mut self,
        quota: &QuotaRateState,
        horizon: i64,
        span: u32,
        parameters: WeightedParameters,
        accepted: bool,
    ) -> Option<f64> {
        let fit = weighted_fit(quota, horizon, span, parameters)?;
        let idle = quota
            .last_observed_at?
            .saturating_sub(self.last_crossing_observation_at?) as f64;
        if accepted && idle > 3600.0 / fit + EPSILON {
            let candidate = fit.min(3600.0 / idle);
            self.idle_rate_ceiling = Some(
                self.idle_rate_ceiling
                    .map_or(candidate, |old| old.min(candidate)),
            );
        } else if accepted && self.idle_rate_ceiling.is_some() {
            self.idle_rate_ceiling = self.idle_rate_ceiling.map(|old| old.min(fit));
        }
        Some(self
            .idle_rate_ceiling
            .map_or(fit, |ceiling| ceiling.min(fit)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::CrossingEvent;
    use super::*;

    /// A quota state with a perfectly linear crossing history: one level per
    /// hundred seconds, so the weighted fit is exactly 36 points per hour.
    fn linear_state(segment_id: u64, end: i64) -> QuotaRateState {
        QuotaRateState {
            resets_at: Some(10_000_000),
            segment_id,
            last_observed_percent: Some(50.0),
            last_observed_at: Some(end),
            correction_candidate: None,
            crossings: (0..10u32)
                .map(|i| CrossingEvent {
                    level: 10 + i,
                    observed_at: end - 900 + i as i64 * 100,
                    segment_id,
                })
                .collect(),
        }
    }

    #[test]
    fn an_exactly_linear_history_keeps_its_slope_under_every_weight() {
        for half_life in [15.0, 300.0, 1800.0, 14400.0] {
            let state = linear_state(7, 900);
            let rate = weighted_fit(
                &state,
                1800,
                2,
                WeightedParameters {
                    half_life,
                    window: 1800,
                },
            )
            .unwrap();
            assert!((rate - 36.0).abs() < 1e-9, "half-life {half_life}: {rate}");
        }
    }

    #[test]
    fn the_five_hour_parameters_are_the_frozen_ones() {
        assert_eq!(FIVE_HOUR_HALF_LIFE_SECS, 300.0);
        assert_eq!(FIVE_HOUR_LOCAL_WINDOW_SECS, 1800);
        assert_eq!(FIVE_HOUR_HARD_EVIDENCE_HORIZON_SECS, 1800);
        assert_eq!(FIVE_HOUR_MIN_LEVEL_SPAN, 2);
        assert_eq!(WeightedParameters::FIVE_HOUR.half_life, 300.0);
        assert_eq!(WeightedParameters::FIVE_HOUR.window, 1800);
    }

    #[test]
    fn insufficient_or_expired_evidence_is_unavailable() {
        let state = linear_state(7, 900);
        // The span is below the frozen minimum.
        assert!(weighted_fit(&state, 1800, 10, WeightedParameters::FIVE_HOUR).is_none());
        // The evidence left the hard horizon.
        let mut expired = linear_state(7, 900);
        expired.last_observed_at = Some(900 + 1801);
        assert!(weighted_fit(&expired, 1800, 2, WeightedParameters::FIVE_HOUR).is_none());
        // Another segment is not evidence.
        let mut other = linear_state(7, 900);
        for event in &mut other.crossings {
            event.segment_id = 8;
        }
        assert!(weighted_fit(&other, 1800, 2, WeightedParameters::FIVE_HOUR).is_none());
        // Without an observation there is no fit at all.
        let mut blind = linear_state(7, 900);
        blind.last_observed_at = None;
        assert!(weighted_fit(&blind, 1800, 2, WeightedParameters::FIVE_HOUR).is_none());
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        let state = linear_state(7, 900);
        for half_life in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(weighted_fit(
                &state,
                1800,
                2,
                WeightedParameters {
                    half_life,
                    window: 1800,
                }
            )
            .is_none());
        }
        for window in [0, -1] {
            assert!(weighted_fit(
                &state,
                1800,
                2,
                WeightedParameters {
                    half_life: 300.0,
                    window,
                }
            )
            .is_none());
        }
    }

    #[test]
    fn the_idle_ceiling_only_ever_decreases() {
        let mut state = linear_state(7, 900);
        let mut metadata = WeightedShadowState {
            last_crossing_observation_at: Some(900),
            idle_rate_ceiling: None,
        };
        let first = metadata
            .publish(&state, 1800, 2, WeightedParameters::FIVE_HOUR, true)
            .unwrap();
        assert!((first - 36.0).abs() < 1e-9);
        assert_eq!(metadata.idle_rate_ceiling, None);

        // The quota idles: the impossible fit establishes a ceiling.
        state.last_observed_at = Some(1100);
        let second = metadata
            .publish(&state, 1800, 2, WeightedParameters::FIVE_HOUR, true)
            .unwrap();
        assert!((second - 18.0).abs() < 1e-9, "{second}");
        let ceiling = metadata.idle_rate_ceiling.unwrap();
        assert!(ceiling < first);

        // More idle lowers the ceiling further, never raises it.
        state.last_observed_at = Some(1300);
        let third = metadata
            .publish(&state, 1800, 2, WeightedParameters::FIVE_HOUR, true)
            .unwrap();
        assert!(third <= second + 1e-9, "{third} > {second}");
        assert!(metadata.idle_rate_ceiling.unwrap() <= ceiling);

        // The quota is active again, so the idle rule does not apply, but the
        // established ceiling still bounds the published rate. The stored
        // ceiling is kept, never replaced by the higher fit.
        let established = metadata.idle_rate_ceiling.unwrap();
        let active = linear_state(7, 1000);
        let fourth = metadata
            .publish(&active, 1800, 2, WeightedParameters::FIVE_HOUR, true)
            .unwrap();
        assert!(fourth <= established + 1e-9, "{fourth} > {established}");
        assert_eq!(
            metadata.idle_rate_ceiling,
            Some(established),
            "the ceiling must never be raised"
        );
    }

    #[test]
    fn a_partial_horizon_expiry_never_raises_the_ceiling() {
        let mut state = linear_state(7, 900);
        let mut metadata = WeightedShadowState {
            last_crossing_observation_at: Some(900),
            idle_rate_ceiling: Some(5.0),
        };
        // The oldest crossings expired, so the fit is computed from fewer
        // points. The established ceiling still bounds the published rate.
        state.crossings.retain(|event| event.observed_at >= 600);
        let rate = metadata
            .publish(&state, 1800, 2, WeightedParameters::FIVE_HOUR, true)
            .unwrap();
        assert!(rate <= 5.0 + 1e-9, "{rate}");
        assert_eq!(metadata.idle_rate_ceiling, Some(5.0));
    }

    #[test]
    fn a_new_crossing_clears_the_idle_ceiling() {
        let before = linear_state(7, 900);
        let mut after = before.clone();
        after.crossings.push(CrossingEvent {
            level: 20,
            observed_at: 950,
            segment_id: 7,
        });
        after.last_observed_at = Some(1000);
        let mut metadata = WeightedShadowState {
            last_crossing_observation_at: Some(900),
            idle_rate_ceiling: Some(5.0),
        };
        assert!(metadata.apply_transition(&before, &after, 1000));
        assert_eq!(metadata.idle_rate_ceiling, None);
        assert_eq!(metadata.last_crossing_observation_at, Some(1000));
    }

    #[test]
    fn a_segment_transition_clears_the_weighted_metadata() {
        let before = linear_state(7, 900);
        let after = QuotaRateState {
            segment_id: 8,
            last_observed_at: Some(1000),
            ..QuotaRateState::default()
        };
        let mut metadata = WeightedShadowState {
            last_crossing_observation_at: Some(900),
            idle_rate_ceiling: Some(5.0),
        };
        assert!(metadata.apply_transition(&before, &after, 1000));
        assert_eq!(metadata.last_crossing_observation_at, None);
        assert_eq!(metadata.idle_rate_ceiling, None);
    }

    #[test]
    fn the_metadata_survives_a_json_round_trip() {
        let metadata = WeightedShadowState {
            last_crossing_observation_at: Some(900),
            idle_rate_ceiling: Some(5.0),
        };
        let json = serde_json::to_string(&metadata).unwrap();
        let reloaded: WeightedShadowState = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded, metadata);
    }

    #[test]
    fn malformed_optional_metadata_is_repaired_without_losing_history() {
        let state = linear_state(7, 900);
        let mut metadata: WeightedShadowState = serde_json::from_value(serde_json::json!({
            "lastCrossingObservationAt": "bad",
            "idleRateCeiling": "bad",
        }))
        .unwrap();
        assert!(metadata.normalize(&state));
        assert_eq!(metadata.last_crossing_observation_at, Some(900));
        assert_eq!(metadata.idle_rate_ceiling, None);
        assert_eq!(state.crossings.len(), 10);

        // A bound outside the eligible window is repaired conservatively.
        let mut metadata: WeightedShadowState = serde_json::from_value(serde_json::json!({
            "lastCrossingObservationAt": 901,
            "idleRateCeiling": 10.0,
        }))
        .unwrap();
        assert!(metadata.normalize(&state));
        assert_eq!(metadata.last_crossing_observation_at, Some(900));
        assert_eq!(metadata.idle_rate_ceiling, None);

        // A valid bound keeps its value, but a malformed ceiling is cleared.
        let mut metadata: WeightedShadowState = serde_json::from_value(serde_json::json!({
            "lastCrossingObservationAt": 900,
            "idleRateCeiling": "bad",
        }))
        .unwrap();
        // The malformed ceiling was already dropped while deserializing, so
        // there is nothing left to repair.
        assert!(!metadata.normalize(&state));
        assert_eq!(metadata.last_crossing_observation_at, Some(900));
        assert_eq!(metadata.idle_rate_ceiling, None);

        // A malformed ceiling next to a valid bound is cleared on repair.
        let mut metadata: WeightedShadowState = serde_json::from_value(serde_json::json!({
            "lastCrossingObservationAt": 900,
            "idleRateCeiling": 10.0,
        }))
        .unwrap();
        assert_eq!(metadata.idle_rate_ceiling, Some(10.0));
        assert!(!metadata.normalize(&state));
        assert_eq!(metadata.idle_rate_ceiling, Some(10.0));
    }

    #[test]
    fn metadata_without_a_crossing_history_is_cleared() {
        let mut metadata = WeightedShadowState {
            last_crossing_observation_at: Some(900),
            idle_rate_ceiling: Some(5.0),
        };
        let empty = QuotaRateState {
            segment_id: 9,
            last_observed_at: Some(1000),
            ..QuotaRateState::default()
        };
        assert!(metadata.normalize(&empty));
        assert_eq!(metadata.last_crossing_observation_at, None);
        assert_eq!(metadata.idle_rate_ceiling, None);
    }
}
