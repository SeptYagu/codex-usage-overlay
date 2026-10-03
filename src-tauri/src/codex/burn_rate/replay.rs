//! Offline Phase 5 candidates. No candidate changes production publication/state.
use super::*;

mod adaptive;

#[derive(Clone, Copy, Debug, serde::Serialize)]
struct Parameters {
    half_life: f64,
    window: i64,
}

fn weighted_fit(state: &QuotaRateState, horizon: i64, span: u32, p: Parameters) -> Option<f64> {
    if !p.half_life.is_finite() || p.half_life <= 0.0 || p.window <= 0 {
        return None;
    }
    let end = state.last_observed_at?;
    let events: Vec<_> = state
        .crossings
        .iter()
        .filter(|e| {
            e.segment_id == state.segment_id
                && e.observed_at <= end
                && end.saturating_sub(e.observed_at) <= horizon
        })
        .collect();
    let newest = *events.last()?;
    // Locality is relative to the newest crossing; accepted idle must not erase
    // an otherwise eligible fit before its hard evidence horizon expires.
    let local: Vec<_> = events
        .into_iter()
        .filter(|e| newest.observed_at.saturating_sub(e.observed_at) <= p.window)
        .collect();
    if newest.level.saturating_sub(local.first()?.level) < span {
        return None;
    }
    let points: Vec<_> = local
        .iter()
        .map(|e| {
            let t = e.observed_at.saturating_sub(newest.observed_at) as f64;
            (t, e.level as f64, (t / p.half_life).exp2())
        })
        .collect();
    let sum: f64 = points.iter().map(|v| v.2).sum();
    let mean_t = points.iter().map(|v| v.0 * v.2).sum::<f64>() / sum;
    let mean_l = points.iter().map(|v| v.1 * v.2).sum::<f64>() / sum;
    let covariance = points
        .iter()
        .map(|v| v.2 * (v.0 - mean_t) * (v.1 - mean_l))
        .sum::<f64>();
    let variance = points
        .iter()
        .map(|v| v.2 * (v.0 - mean_t).powi(2))
        .sum::<f64>();
    let rate = covariance / variance * 3600.0;
    (variance.is_finite() && variance > 0.0 && rate.is_finite() && rate > 0.0).then_some(rate)
}

fn tolerant_optional<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(raw).ok())
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
struct Candidate {
    #[serde(flatten)]
    quota: QuotaRateState,
    #[serde(
        default,
        rename = "lastCrossingObservationAt",
        deserialize_with = "tolerant_optional"
    )]
    upper: Option<i64>,
    #[serde(
        default,
        rename = "idleRateCeiling",
        deserialize_with = "tolerant_optional"
    )]
    ceiling: Option<f64>,
}

impl Candidate {
    fn normalize(&mut self) -> bool {
        let before = (self.upper, self.ceiling);
        let newest = self
            .quota
            .crossings
            .iter()
            .rev()
            .find(|e| e.segment_id == self.quota.segment_id);
        match (newest, self.quota.last_observed_at) {
            (Some(event), Some(end)) if event.observed_at <= end => {
                if !self
                    .upper
                    .is_some_and(|bound| event.observed_at <= bound && bound <= end)
                {
                    self.upper = Some(end);
                    self.ceiling = None;
                }
                if self.ceiling.is_some_and(|v| !v.is_finite() || v <= 0.0) {
                    self.ceiling = None;
                }
            }
            _ => {
                self.upper = None;
                self.ceiling = None;
            }
        }
        before != (self.upper, self.ceiling)
    }

    fn accept(
        &mut self,
        at: i64,
        used: f64,
        max_gap: i64,
        horizon: i64,
        span: u32,
        p: Parameters,
    ) -> Option<f64> {
        self.accept_snapshot(
            at,
            QuotaSnapshot {
                used_percent: Some(used),
                resets_at: Some(10_000_000),
                window_duration_mins: None,
            },
            max_gap,
            horizon,
            span,
            p,
        )
    }

    fn accept_snapshot(
        &mut self,
        at: i64,
        snapshot: QuotaSnapshot,
        max_gap: i64,
        horizon: i64,
        span: u32,
        p: Parameters,
    ) -> Option<f64> {
        let accepted = self.observe(at, &snapshot, max_gap);
        valid_used_percent(&snapshot)?;
        self.publish(horizon, span, p, accepted)
    }

    fn observe(&mut self, at: i64, snapshot: &QuotaSnapshot, max_gap: i64) -> bool {
        // Migration happens before the incoming observation advances the baseline.
        self.normalize();
        let before = self.quota.clone();
        accept_quota(&mut self.quota, snapshot, at, max_gap);
        if self.quota.segment_id != before.segment_id {
            self.upper = None;
            self.ceiling = None;
        }
        if self.quota.crossings.last().is_some()
            && self.quota.crossings.last() != before.crossings.last()
        {
            self.upper = Some(at);
            self.ceiling = None;
        }
        self.quota.last_observed_at != before.last_observed_at
    }

    fn publish(&mut self, horizon: i64, span: u32, p: Parameters, accepted: bool) -> Option<f64> {
        let fit = weighted_fit(&self.quota, horizon, span, p)?;
        let idle = self.quota.last_observed_at?.saturating_sub(self.upper?) as f64;
        if accepted && idle > 3600.0 / fit + EPSILON {
            let candidate = fit.min(3600.0 / idle);
            self.ceiling = Some(self.ceiling.map_or(candidate, |old| old.min(candidate)));
        } else if accepted && self.ceiling.is_some() {
            self.ceiling = self.ceiling.map(|old| old.min(fit));
        }
        Some(self.ceiling.map_or(fit, |c| c.min(fit)))
    }
}

fn observed(at: i64, phase: i64, period: i64, fractional: bool) -> f64 {
    let value = 10.0 + (at + phase) as f64 / period as f64;
    if fractional {
        value
    } else {
        (value + EPSILON).floor()
    }
}

#[derive(Default, serde::Serialize)]
struct ConstantEvidence {
    worst_error: f64,
    worst_legacy_error: f64,
    candidate_error_seconds: f64,
    legacy_error_seconds: f64,
    paired_seconds: i64,
    available: usize,
    unavailable: usize,
    worst_fixture: serde_json::Value,
    fixtures: Vec<serde_json::Value>,
}

fn constant_error(p: Parameters, scale: i64, span: u32) -> ConstantEvidence {
    let mut evidence = ConstantEvidence::default();
    let mut worst: f64 = 0.0;
    let mut available = 0;
    let mut unavailable = 0;
    for period in [60, 120] {
        for cadence in [15, 30, 60, 120, 300] {
            for duration in [0, 5, 20] {
                for phase in [0, 13, 29, 47] {
                    for fractional in [false, true] {
                        let mut c = Candidate::default();
                        let mut at = 0;
                        let mut index = 0;
                        let mut fixture_worst: f64 = 0.0;
                        let mut fixture_available = 0;
                        let mut fixture_unavailable = 0;
                        let mut worst_at = 0;
                        let mut fixture_legacy_worst: f64 = 0.0;
                        let mut candidate_error_seconds = 0.0;
                        let mut legacy_error_seconds = 0.0;
                        let mut paired_seconds = 0;
                        let mut previous_pair: Option<(i64, f64, f64)> = None;
                        while at <= 50 * period * scale {
                            let used = observed(at, phase * scale, period * scale, fractional);
                            let rate = c.accept(
                                at,
                                used,
                                if scale == 1 {
                                    FIVE_HOUR_MAX_GAP_SECS
                                } else {
                                    WEEKLY_MAX_GAP_SECS
                                },
                                1800 * scale,
                                span,
                                p,
                            );
                            let eligible: Vec<_> = c
                                .quota
                                .crossings
                                .iter()
                                .filter(|e| at - e.observed_at <= 1800 * scale)
                                .collect();
                            if let Some((previous_at, candidate_error, legacy_error)) =
                                previous_pair.take()
                            {
                                let dt = at - previous_at;
                                candidate_error_seconds += candidate_error * dt as f64;
                                legacy_error_seconds += legacy_error * dt as f64;
                                paired_seconds += dt;
                            }
                            if eligible.len() >= 8
                                && eligible.last().unwrap().level - eligible[0].level >= 7
                            {
                                let target = 3600.0 / (period * scale) as f64;
                                let legacy = estimate_rate(&c.quota, 1800 * scale, span);
                                if let Some(legacy) = legacy {
                                    fixture_legacy_worst =
                                        fixture_legacy_worst.max((legacy / target - 1.0).abs());
                                    if let Some(candidate) = rate {
                                        previous_pair = Some((
                                            at,
                                            (candidate / target - 1.0).abs(),
                                            (legacy / target - 1.0).abs(),
                                        ));
                                    }
                                }
                                if let Some(rate) = rate {
                                    let error =
                                        (rate / (3600.0 / (period * scale) as f64) - 1.0).abs();
                                    if error > fixture_worst {
                                        fixture_worst = error;
                                        worst_at = at;
                                    }
                                    worst = worst.max(error);
                                    available += 1;
                                    fixture_available += 1;
                                } else {
                                    unavailable += 1;
                                    fixture_unavailable += 1;
                                }
                            }
                            // Sleep-after-fetch cadence with deterministic 0..5s jitter.
                            at += (cadence + duration + index % 6) * scale;
                            index += 1;
                        }
                        evidence.worst_legacy_error =
                            evidence.worst_legacy_error.max(fixture_legacy_worst);
                        evidence.candidate_error_seconds += candidate_error_seconds;
                        evidence.legacy_error_seconds += legacy_error_seconds;
                        evidence.paired_seconds += paired_seconds;
                        let fixture = serde_json::json!({"periodSeconds":period*scale,"cadenceSeconds":cadence*scale,
                            "fetchDurationSeconds":duration*scale,"phaseSeconds":phase*scale,"fractional":fractional,
                            "worstLegacyRelativeError":fixture_legacy_worst,"candidateErrorSeconds":candidate_error_seconds,
                            "legacyErrorSeconds":legacy_error_seconds,"pairedSeconds":paired_seconds,
                            "worstRelativeError":fixture_worst,"worstAt":worst_at,"available":fixture_available,"unavailable":fixture_unavailable});
                        if fixture_worst >= evidence.worst_error {
                            evidence.worst_error = fixture_worst;
                            evidence.worst_fixture = fixture.clone();
                        }
                        evidence.fixtures.push(fixture);
                    }
                }
            }
        }
    }
    evidence.worst_error = worst;
    evidence.available = available;
    evidence.unavailable = unavailable;
    evidence
}

#[derive(Debug, serde::Serialize)]
struct Response {
    target: f64,
    legacy_error: f64,
    candidate_error: f64,
    legacy_first: Option<i64>,
    candidate_first: Option<i64>,
    legacy_sustained: Option<i64>,
    candidate_sustained: Option<i64>,
    legacy_excursion: f64,
    candidate_excursion: f64,
    missing: usize,
}

fn response(p: Parameters, scale: i64, span: u32, new_period: i64) -> Response {
    let change = 1800 * scale;
    let target = 3600.0 / (new_period * scale) as f64;
    let old = 60.0 / scale as f64;
    let mut r = Response {
        target,
        legacy_error: 0.0,
        candidate_error: 0.0,
        legacy_first: None,
        candidate_first: None,
        legacy_sustained: None,
        candidate_sustained: None,
        legacy_excursion: 0.0,
        candidate_excursion: 0.0,
        missing: 0,
    };
    let mut c = Candidate::default();
    let mut previous_at = change;
    let mut previous_rates: (Option<f64>, Option<f64>) = (None, None);
    let mut run = [0; 2];
    for at in (0..=change + 20 * new_period * scale).step_by((30 * scale) as usize) {
        let used = if at <= change {
            10.0 + at as f64 / (60 * scale) as f64
        } else {
            40.0 + (at - change) as f64 / (new_period * scale) as f64
        };
        let candidate = c.accept(
            at,
            used.floor(),
            if scale == 1 {
                FIVE_HOUR_MAX_GAP_SECS
            } else {
                WEEKLY_MAX_GAP_SECS
            },
            1800 * scale,
            span,
            p,
        );
        let legacy = estimate_rate(&c.quota, 1800 * scale, span);
        if at >= change {
            if at <= change + 6 * new_period * scale {
                let dt = (at - previous_at) as f64;
                if let Some(rate) = previous_rates.0 {
                    r.legacy_error += (rate - target).abs() * dt;
                }
                if let Some(rate) = previous_rates.1 {
                    r.candidate_error += (rate - target).abs() * dt;
                }
                if legacy.is_some() && candidate.is_none() {
                    r.missing += 1;
                }
            }
            for (i, rate) in [legacy, candidate].into_iter().enumerate() {
                let in_band = rate.is_some_and(|v| (v / target - 1.0).abs() <= 0.1 + EPSILON);
                run[i] = if in_band { run[i] + 1 } else { 0 };
                let first = if i == 0 {
                    &mut r.legacy_first
                } else {
                    &mut r.candidate_first
                };
                if in_band && first.is_none() {
                    *first = Some(at - change);
                }
                let sustained = if i == 0 {
                    &mut r.legacy_sustained
                } else {
                    &mut r.candidate_sustained
                };
                if run[i] >= 3 && sustained.is_none() {
                    *sustained = Some(at - change - 60 * scale);
                }
                if let Some(v) = rate {
                    let excursion = (old.min(target) - v).max(v - old.max(target)).max(0.0);
                    if i == 0 {
                        r.legacy_excursion = r.legacy_excursion.max(excursion);
                    } else {
                        r.candidate_excursion = r.candidate_excursion.max(excursion);
                    }
                }
            }
            previous_at = at;
            previous_rates = (legacy, candidate);
        }
    }
    r
}

#[test]
#[ignore = "parameter-grid evidence; run explicitly with --ignored --nocapture"]
fn parameter_grid() {
    let mut report = Vec::new();
    for (quota, scale, span) in [("5H", 1, 2), ("WK", 48, 1)] {
        for half_life in [
            15, 30, 45, 60, 90, 120, 180, 240, 300, 360, 450, 600, 900, 1800, 3600,
        ] {
            for window in [180, 240, 300, 450, 600, 900, 1200, 1800] {
                let p = Parameters {
                    half_life: (half_life * scale) as f64,
                    window: window * scale,
                };
                let constant = constant_error(p, scale, span);
                let acceleration = response(p, scale, span, 30);
                let deceleration = response(p, scale, span, 120);
                let response_pass = [&acceleration, &deceleration].into_iter().all(|r| {
                    r.missing == 0
                        && r.candidate_error <= r.legacy_error * 0.9
                        && r.candidate_excursion <= r.legacy_excursion + 1e-8
                        && r.candidate_sustained.is_some()
                        && (r.legacy_sustained.is_none()
                            || r.candidate_sustained <= r.legacy_sustained)
                });
                let pass = constant.worst_error <= 0.05 && response_pass;
                let row = serde_json::json!({"quota":quota,"halfLifeSeconds":p.half_life,"localWindowSeconds":p.window,
                    "worstConstantRelativeError":constant.worst_error,"constantAvailable":constant.available,"constantUnavailable":constant.unavailable,
                    "worstLegacyConstantRelativeError":constant.worst_legacy_error,
                    "candidateConstantRelativeErrorSeconds":constant.candidate_error_seconds,
                    "legacyConstantRelativeErrorSeconds":constant.legacy_error_seconds,"constantPairedSeconds":constant.paired_seconds,
                    "worstConstantFixture":constant.worst_fixture,"constantFixtures":constant.fixtures,
                    "acceleration":acceleration,"deceleration":deceleration,"responsePass":response_pass,"pass":pass});
                if pass {
                    println!("PASS {row}");
                }
                report.push(row);
            }
        }
    }
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../output/burn-rate-replay");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("parameter-grid.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "{} candidates; {} pass",
        report.len(),
        report.iter().filter(|v| v["pass"] == true).count()
    );
}

fn original() -> Parameters {
    Parameters {
        half_life: 300.0,
        window: 1800,
    }
}

fn warm(p: Parameters) -> Candidate {
    let mut c = Candidate::default();
    for minute in 0..=30 {
        c.accept(minute * 60, 10.0 + minute as f64, 600, 1800, 2, p);
    }
    c
}

#[test]
fn exactly_linear_crossings_preserve_the_slope_under_all_weights() {
    for period in [60, 120, 2880, 5760] {
        let state = QuotaRateState {
            segment_id: 3,
            last_observed_at: Some(30 * period),
            crossings: (1..=30)
                .map(|i| CrossingEvent {
                    level: i as u32,
                    observed_at: i * period - period / 2,
                    segment_id: 3,
                })
                .collect(),
            ..Default::default()
        };
        for half_life in [15.0, 30.0, 300.0, 1800.0, 14400.0, 21600.0] {
            let p = Parameters {
                half_life,
                window: 86400,
            };
            let rate = weighted_fit(&state, 86400, 2, p).unwrap();
            assert!(
                (rate / (3600.0 / period as f64) - 1.0).abs() <= 1e-9,
                "period={period}, {p:?}: {rate}"
            );
        }
    }
}

#[test]
fn insufficient_expired_degenerate_or_wrong_segment_evidence_is_unavailable() {
    let mut c = Candidate::default();
    assert!(c.accept(0, 10.0, 600, 1800, 2, original()).is_none());
    assert!(c.accept(60, 12.0, 600, 1800, 2, original()).is_none());
    assert!(weighted_fit(&c.quota, 1800, 1, original()).is_some());
    c.quota.crossings[0].segment_id = 999;
    assert!(weighted_fit(&c.quota, 1800, 1, original()).is_none());
    c = warm(original());
    c.quota.last_observed_at = Some(4000);
    assert!(weighted_fit(&c.quota, 1800, 2, original()).is_none());
    c.quota.last_observed_at = Some(1800);
    for event in &mut c.quota.crossings {
        event.observed_at = 1770;
    }
    assert!(weighted_fit(&c.quota, 1800, 2, original()).is_none());
    for half_life in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(weighted_fit(
            &warm(original()).quota,
            1800,
            2,
            Parameters {
                half_life,
                window: 1800
            }
        )
        .is_none());
    }
}

#[test]
fn unchanged_manual_refresh_uses_the_confirmation_bound_not_the_midpoint() {
    let mut c = warm(original());
    assert_eq!(c.quota.crossings.last().unwrap().observed_at, 1770);
    assert_eq!(c.upper, Some(1800));
    let rate = c.accept(1845, 40.0, 600, 1800, 2, original()).unwrap();
    assert!((rate - 60.0).abs() < 1e-8);
    assert_eq!(c.ceiling, None);
    let fractional = c.accept(1850, 40.8, 600, 1800, 2, original()).unwrap();
    assert!((fractional - 60.0).abs() < 1e-8);
    assert_eq!(c.upper, Some(1800));
    assert!((c.accept(1860, 40.9, 600, 1800, 2, original()).unwrap() - 60.0).abs() < 1e-8);
    assert_eq!(c.ceiling, None);
    c.accept(1861, 40.9, 600, 1800, 2, original());
    assert!(c.ceiling.unwrap() < 60.0);
    assert_eq!(c.upper, Some(1800));
    c.accept(1870, 41.0, 600, 1800, 2, original());
    assert_eq!(c.upper, Some(1870));
    assert_eq!(c.ceiling, None);
}

#[test]
fn genuine_idle_only_lowers_the_ceiling_until_evidence_expires() {
    let mut c = warm(original());
    let mut previous = 60.0;
    let mut expired = false;
    for at in (1830..=4000).step_by(30) {
        let rate = c.accept(at, 40.0, 600, 1800, 2, original());
        if let Some(rate) = rate {
            assert!(!expired);
            assert!(rate <= previous + 1e-8, "{at}: {rate} > {previous}");
            previous = rate;
        } else {
            expired = true;
        }
        assert_eq!(c.upper, Some(1800));
    }
    assert!(expired);
    // A changing fit caused by partial horizon expiry must never lift a ceiling.
    let mut c = warm(original());
    c.ceiling = Some(5.0);
    c.quota.last_observed_at = Some(2700);
    assert_eq!(
        c.accept(2730, 40.0, 600, 1800, 2, original()),
        Some(3600.0 / 930.0)
    );
}

#[test]
fn candidate_migration_preserves_crossings_and_does_not_follow_unchanged_polls() {
    let source = warm(original());
    let legacy = serde_json::to_string(&source.quota).unwrap();
    let mut c: Candidate = serde_json::from_str(&legacy).unwrap();
    assert!(c.normalize());
    assert_eq!(c.upper, Some(1800));
    assert_eq!(c.quota, source.quota);
    assert!(!c.normalize());
    for at in [1845, 1900, 2200] {
        c.accept(at, 40.0, 600, 1800, 2, original());
        let json = serde_json::to_string(&c).unwrap();
        let mut reloaded: Candidate = serde_json::from_str(&json).unwrap();
        assert!(!reloaded.normalize());
        assert_eq!(c, reloaded);
        assert_eq!(reloaded.upper, Some(1800));
        c = reloaded;
    }
    assert!(c.ceiling.unwrap() < 60.0);
}

#[test]
fn malformed_optional_metadata_never_discards_valid_crossing_history() {
    let source = warm(original());
    for bound in [
        serde_json::json!("bad"),
        serde_json::json!({}),
        serde_json::json!(1769),
        serde_json::json!(1801),
        serde_json::Value::Null,
    ] {
        for ceiling in [
            serde_json::json!("bad"),
            serde_json::json!([]),
            serde_json::json!(-1),
            serde_json::json!(0),
            serde_json::json!(10),
        ] {
            let mut raw = serde_json::to_value(&source).unwrap();
            raw["lastCrossingObservationAt"] = bound.clone();
            raw["idleRateCeiling"] = ceiling;
            let mut c: Candidate = serde_json::from_value(raw).unwrap();
            c.normalize();
            assert_eq!(c.quota, source.quota);
            assert_eq!(c.upper, Some(1800));
            assert_eq!(c.ceiling, None);
            assert!((weighted_fit(&c.quota, 1800, 2, original()).unwrap() - 60.0).abs() < 1e-8);
        }
    }
    let mut raw = serde_json::to_value(&source).unwrap();
    raw["lastCrossingObservationAt"] = serde_json::json!(1790);
    raw["idleRateCeiling"] = serde_json::json!("bad");
    let mut c: Candidate = serde_json::from_value(raw).unwrap();
    c.normalize();
    assert_eq!(c.upper, Some(1790));
    assert_eq!(c.ceiling, None);
}

#[test]
fn invalid_stale_conflicting_and_pending_correction_samples_do_not_advance_idle_metadata() {
    for (at, value) in [
        (1900, f64::NAN),
        (1900, f64::INFINITY),
        (1900, -1.0),
        (1900, 101.0),
        (1700, 40.0),
        (1800, 41.0),
        (1900, 39.0),
    ] {
        let mut c = warm(original());
        c.ceiling = Some(10.0);
        c.accept(at, value, 600, 1800, 2, original());
        assert_eq!(c.upper, Some(1800), "{at}, {value}");
        assert_eq!(c.ceiling, Some(10.0), "{at}, {value}");
    }
}

#[test]
fn gap_correction_and_reset_clear_candidate_metadata_without_synthetic_crossings() {
    for transition in ["gap", "correction", "reset"] {
        let mut c = warm(original());
        c.ceiling = Some(10.0);
        match transition {
            "gap" => {
                c.accept(2401, 41.0, 600, 1800, 2, original());
            }
            "correction" => {
                c.accept(1830, 39.0, 600, 1800, 2, original());
                c.accept(1860, 39.0, 600, 1800, 2, original());
            }
            _ => {
                c.quota.resets_at = Some(1800);
                c.accept_snapshot(
                    1830,
                    QuotaSnapshot {
                        used_percent: Some(0.0),
                        resets_at: Some(5000),
                        window_duration_mins: None,
                    },
                    600,
                    1800,
                    2,
                    original(),
                );
            }
        }
        assert!(c.quota.crossings.is_empty());
        assert_eq!(c.upper, None);
        assert_eq!(c.ceiling, None);
    }
}

#[test]
fn supported_slow_polling_plus_fetch_time_survives_but_3600s_retains_gap_policy() {
    let mut c = Candidate::default();
    for at in (0..=3200).step_by(325) {
        c.accept(at, observed(at, 0, 60, false), 600, 1800, 2, original());
        assert_eq!(c.quota.segment_id, 0);
    }
    assert!(c.upper.is_some());
    c.accept(6800, 90.0, 600, 1800, 2, original());
    assert_eq!(c.quota.segment_id, 1);
    assert!(c.quota.crossings.is_empty());
    assert_eq!(c.upper, None);
}

#[test]
fn original_five_minute_candidate_is_a_negative_response_control() {
    let faster = response(original(), 1, 2, 30);
    let slower = response(original(), 1, 2, 120);
    assert!(faster.candidate_error > faster.legacy_error);
    assert!(slower.candidate_error > slower.legacy_error);
}

#[test]
fn fractional_raw_warmup_is_a_negative_constant_rate_control() {
    // Constant 1%/min, offset by 47s, with successful 120s sleeps plus
    // 5s fetch duration and 0..3s scheduling jitter. The unchanged production
    // reconstruction distributes 2/3 crossings uniformly inside each interval.
    // After nine reconstructed crossings even long weighting fails the 5% gate.
    for half_life in [300.0, 3600.0] {
        let p = Parameters {
            half_life,
            window: 1800,
        };
        let mut c = Candidate::default();
        let mut rate = None;
        for at in [0, 125, 251, 378, 506] {
            rate = c.accept(at, observed(at, 47, 60, true), 600, 1800, 2, p);
        }
        assert_eq!(c.quota.crossings.len(), 9);
        assert_eq!(
            c.quota.crossings.last().unwrap().level - c.quota.crossings[0].level,
            8
        );
        let rate = rate.unwrap();
        assert!((rate / 60.0 - 1.0).abs() > 0.05, "{p:?}: {rate}");
    }
}
