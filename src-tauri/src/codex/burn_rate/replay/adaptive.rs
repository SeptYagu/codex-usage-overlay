//! Ablation experiments only. Policy never sees synthetic truth/rate-change time.
use super::*;

#[derive(Clone, Copy, Debug, Serialize)]
struct Quota {
    name: &'static str,
    scale: i64,
    horizon: i64,
    gap: i64,
    span: u32,
}

const QUOTAS: [Quota; 2] = [
    Quota {
        name: "5H",
        scale: 1,
        horizon: FIVE_HOUR_HORIZON_SECS,
        gap: FIVE_HOUR_MAX_GAP_SECS,
        span: 2,
    },
    Quota {
        name: "WK",
        scale: 48,
        horizon: WEEKLY_HORIZON_SECS,
        gap: WEEKLY_MAX_GAP_SECS,
        span: 1,
    },
];

#[derive(Clone, Copy, Debug, Serialize)]
struct Variant {
    name: &'static str,
    precise: bool,
    adaptive: bool,
    votes: u32,
    early_recovery: bool,
    fast_half_life: i64,
}

const VARIANTS: [Variant; 9] = [
    Variant {
        name: "legacy",
        precise: false,
        adaptive: false,
        votes: 0,
        early_recovery: false,
        fast_half_life: 15,
    },
    Variant {
        name: "fixed",
        precise: false,
        adaptive: false,
        votes: 0,
        early_recovery: false,
        fast_half_life: 15,
    },
    Variant {
        name: "precision_only",
        precise: true,
        adaptive: false,
        votes: 0,
        early_recovery: false,
        fast_half_life: 15,
    },
    Variant {
        name: "adaptive_only",
        precise: false,
        adaptive: true,
        votes: 1,
        early_recovery: false,
        fast_half_life: 15,
    },
    Variant {
        name: "precision_adaptive",
        precise: true,
        adaptive: true,
        votes: 1,
        early_recovery: false,
        fast_half_life: 15,
    },
    Variant {
        name: "precision_adaptive_2votes",
        precise: true,
        adaptive: true,
        votes: 2,
        early_recovery: false,
        fast_half_life: 15,
    },
    Variant {
        name: "precision_adaptive_early_recovery",
        precise: true,
        adaptive: true,
        votes: 1,
        early_recovery: true,
        fast_half_life: 30,
    },
    Variant {
        name: "precision_adaptive_early_15s",
        precise: true,
        adaptive: true,
        votes: 1,
        early_recovery: true,
        fast_half_life: 15,
    },
    Variant {
        name: "precision_adaptive_conditional_30s",
        precise: true,
        adaptive: true,
        votes: 1,
        early_recovery: false,
        fast_half_life: 30,
    },
];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Bound {
    level: u32,
    at: i64,
    lower: i64,
    upper: i64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Experiment {
    candidate: Candidate,
    bounds: Vec<Bound>,
    seen_fractional: bool,
    direction: i32,
    votes: u32,
    fit_start: Option<i64>,
    changed_at: Option<i64>,
    settled_at: Option<i64>,
    changes: usize,
}

impl Experiment {
    fn accept(&mut self, at: i64, used: f64, q: Quota, v: Variant) -> Option<f64> {
        self.accept_snapshot(
            at,
            QuotaSnapshot {
                used_percent: Some(used),
                resets_at: Some(10_000_000),
                window_duration_mins: None,
            },
            q,
            v,
        )
    }

    fn accept_snapshot(
        &mut self,
        at: i64,
        snapshot: QuotaSnapshot,
        q: Quota,
        v: Variant,
    ) -> Option<f64> {
        let before = self.candidate.quota.clone();
        let accepted = self.candidate.observe(at, &snapshot, q.gap);
        if before.segment_id != self.candidate.quota.segment_id {
            self.bounds.clear();
            self.seen_fractional = false;
            self.direction = 0;
            self.votes = 0;
            self.fit_start = None;
            self.changed_at = None;
            self.settled_at = None;
        }
        let used = valid_used_percent(&snapshot)?;
        if accepted {
            self.seen_fractional |= (used - used.round()).abs() > EPSILON;
        }
        let new_crossing = self.candidate.quota.crossings.last().is_some()
            && self.candidate.quota.crossings.last() != before.crossings.last();
        if new_crossing && accepted && before.segment_id == self.candidate.quota.segment_id {
            let previous_at = before.last_observed_at.unwrap();
            let previous = before.last_observed_percent.unwrap();
            self.seen_fractional |= (previous - previous.round()).abs() > EPSILON;
            let previous_level = before.crossings.last().map_or(0, |e| e.level);
            for event in self
                .candidate
                .quota
                .crossings
                .iter_mut()
                .filter(|e| e.level > previous_level)
            {
                if v.precise && self.seen_fractional {
                    // Linear interpolation uses the actual level position, instead
                    // of equal spacing by count. Round only the timestamp to seconds.
                    let ratio =
                        ((event.level as f64 - previous) / (used - previous)).clamp(0.0, 1.0);
                    event.observed_at =
                        previous_at + ((at - previous_at) as f64 * ratio).round() as i64;
                }
                self.bounds.push(Bound {
                    level: event.level,
                    at: event.observed_at,
                    lower: previous_at,
                    upper: at,
                });
            }
            if self.bounds.len() > MAX_CROSSINGS {
                self.bounds.drain(0..self.bounds.len() - MAX_CROSSINGS);
            }
        }
        let stable = Parameters {
            half_life: (300 * q.scale) as f64,
            window: q.horizon,
        };
        if v.name == "legacy" {
            return estimate_rate(&self.candidate.quota, q.horizon, q.span);
        }
        if v.adaptive && new_crossing && accepted {
            self.detect(at, q, stable, v.votes);
        }
        let mut fit_state = self.candidate.quota.clone();
        if let Some(start) = self.fit_start {
            fit_state.crossings.retain(|e| e.observed_at >= start);
        }
        let fast_half_life = v.fast_half_life * q.scale;
        if let Some(changed_at) = self.changed_at {
            if self.settled_at.is_none() && new_crossing && accepted {
                let count = self
                    .bounds
                    .iter()
                    .filter(|b| b.upper > changed_at)
                    .map(|b| b.upper)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                let fast = weighted_fit(
                    &fit_state,
                    q.horizon,
                    q.span,
                    Parameters {
                        half_life: fast_half_life as f64,
                        window: q.horizon,
                    },
                );
                let slow = weighted_fit(&fit_state, q.horizon, q.span, stable);
                if count >= 3
                    && fast
                        .zip(slow)
                        .is_some_and(|(fast, slow)| (fast / slow - 1.0).abs() <= 0.1)
                {
                    self.settled_at = Some(at);
                }
            }
        }
        let parameters = if let Some(changed_at) = self.changed_at {
            let recovery_start = if v.early_recovery {
                Some(changed_at)
            } else {
                self.settled_at
            };
            let recovery = recovery_start.map_or(0.0, |start| {
                let age = self
                    .candidate
                    .quota
                    .last_observed_at
                    .unwrap()
                    .saturating_sub(start) as f64;
                (age / (600 * q.scale) as f64).clamp(0.0, 1.0)
            });
            Parameters {
                half_life: fast_half_life as f64
                    + (300 * q.scale - fast_half_life) as f64 * recovery,
                window: q.horizon,
            }
        } else {
            stable
        };
        // Reuse the same conservative idle publication, without deleting history.
        let original_state = std::mem::replace(&mut self.candidate.quota, fit_state);
        let rate = self
            .candidate
            .publish(q.horizon, q.span, parameters, accepted);
        self.candidate.quota = original_state;
        rate
    }

    fn detect(&mut self, at: i64, q: Quota, stable: Parameters, required_votes: u32) {
        let mut baseline = self.candidate.quota.clone();
        if let Some(start) = self.fit_start {
            baseline.crossings.retain(|e| e.observed_at >= start);
        }
        let Some(slow) = weighted_fit(&baseline, q.horizon, q.span, stable) else {
            return;
        };
        // Three distinct confirming observations provide two independent intervals.
        // All crossings from a multi-point jump share one observation, never votes.
        let mut groups: Vec<&Bound> = Vec::new();
        for bound in self.bounds.iter().rev().filter(|b| at - b.at <= q.horizon) {
            if groups.last().is_none_or(|last| last.upper != bound.upper) {
                groups.push(bound);
            }
            if groups.len() == 3 {
                break;
            }
        }
        if groups.len() < 3 {
            return;
        }
        let newest = groups[0];
        let oldest = groups[2];
        let span = newest.level.saturating_sub(oldest.level);
        if span < q.span {
            return;
        }
        let max_seconds = newest.upper - oldest.lower;
        let min_seconds = newest.lower - oldest.upper;
        if max_seconds <= 0 {
            return;
        }
        let minimum = span as f64 * 3600.0 / max_seconds as f64;
        let maximum = if min_seconds > 0 {
            span as f64 * 3600.0 / min_seconds as f64
        } else {
            f64::INFINITY
        };
        // A 10% guard prevents tiny baseline noise from activating a change.
        let direction = if slow < minimum * 0.9 {
            1
        } else if slow > maximum * 1.1 {
            -1
        } else {
            0
        };
        if direction == 0 {
            self.direction = 0;
            self.votes = 0;
            return;
        }
        self.votes = if self.direction == direction {
            self.votes + 1
        } else {
            1
        };
        self.direction = direction;
        if self.votes >= required_votes {
            self.fit_start = Some(oldest.at);
            self.changed_at = Some(at);
            self.settled_at = None;
            self.changes += 1;
            self.votes = 0;
            self.direction = 0;
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
struct Metrics {
    available: usize,
    unavailable: usize,
    error_seconds: f64,
    signed_error_seconds: f64,
    seconds: i64,
    peak_relative_error: f64,
    first_band: Option<i64>,
    sustained_band: Option<i64>,
    excursion: f64,
    changes: usize,
}

#[derive(Clone, Debug, Serialize)]
struct Fixture {
    quota: &'static str,
    cadence: i64,
    duration: i64,
    phase: i64,
    fractional: bool,
    period: i64,
    new_period: Option<i64>,
    manual: bool,
    jitter_seed: i64,
}

fn schedule(
    end: i64,
    cadence: i64,
    duration: i64,
    phase: i64,
    seed: i64,
    manual: bool,
    change: Option<i64>,
) -> Vec<i64> {
    let mut times = vec![0];
    let mut at = cadence + duration + phase;
    let mut i = 0;
    while at <= end {
        times.push(at);
        if manual {
            times.push((at + 7).min(end));
        }
        at += cadence + duration + if seed < 0 { 0 } else { (i * 5 + seed) % 6 };
        i += 1;
    }
    if let Some(change) = change {
        times.push(change);
        if manual {
            times.extend([change + 45, change + 59]);
        }
    }
    times.sort_unstable();
    times.dedup();
    times
}

fn run(q: Quota, f: &Fixture, v: Variant) -> Metrics {
    let mut experiment = Experiment::default();
    let change = 1800 * q.scale;
    let end = if let Some(new_period) = f.new_period {
        change + 20 * new_period
    } else {
        32 * f.period
    };
    let times = schedule(
        end,
        f.cadence,
        f.duration,
        f.phase,
        f.jitter_seed,
        f.manual,
        f.new_period.map(|_| change),
    );
    let mut out = Metrics::default();
    let mut held: Option<(i64, f64, bool)> = None;
    let mut band_run = 0;
    let mut band_start = 0;
    // Response integral ends at the actual observation confirming six new points.
    let mut response_end = None;
    let old_target = 3600.0 / f.period as f64;
    let target = 3600.0 / f.new_period.unwrap_or(f.period) as f64;
    for at in times {
        let value = 10.0
            + at.min(change) as f64 / f.period as f64
            + if at > change {
                (at - change) as f64 / f.new_period.unwrap_or(f.period) as f64
            } else {
                0.0
            };
        let used = if f.fractional {
            value
        } else {
            (value + EPSILON).floor()
        };
        if f.new_period.is_some() && response_end.is_none() && used.floor() >= 46.0 && at >= change
        {
            response_end = Some(at);
        }
        let rate = experiment.accept(at, used, q, v);
        let eligible: Vec<_> = experiment
            .candidate
            .quota
            .crossings
            .iter()
            .filter(|e| at - e.observed_at <= q.horizon)
            .collect();
        let warm = eligible.len() >= 8 && eligible.last().unwrap().level - eligible[0].level >= 7;
        let measure = if f.new_period.is_some() {
            at >= change && response_end.is_none_or(|end| at <= end)
        } else {
            warm
        };
        if let Some((previous_at, previous_rate, was_measured)) = held {
            if was_measured && measure {
                let dt = at - previous_at;
                out.error_seconds += (previous_rate / target - 1.0).abs() * dt as f64;
                out.signed_error_seconds += (previous_rate / target - 1.0) * dt as f64;
                out.seconds += dt;
            }
        }
        if measure {
            if let Some(rate) = rate {
                out.available += 1;
                out.peak_relative_error = out.peak_relative_error.max((rate / target - 1.0).abs());
            } else {
                out.unavailable += 1;
            }
        }
        if f.new_period.is_some() && at >= change {
            let in_band = rate.is_some_and(|rate| (rate / target - 1.0).abs() <= 0.1 + EPSILON);
            if in_band {
                if band_run == 0 {
                    band_start = at - change;
                }
                band_run += 1;
                out.first_band.get_or_insert(at - change);
                if band_run >= 3 {
                    out.sustained_band.get_or_insert(band_start);
                }
            } else {
                band_run = 0;
            }
            if let Some(rate) = rate {
                out.excursion = out.excursion.max(
                    (old_target.min(target) - rate)
                        .max(rate - old_target.max(target))
                        .max(0.0),
                );
            }
        }
        held = rate.map(|rate| (at, rate, measure));
    }
    out.changes = experiment.changes;
    out
}

#[test]
#[ignore = "expanded adaptive ablations; run explicitly with --ignored --nocapture"]
fn adaptive_comparison_matrix() {
    let mut constants = Vec::new();
    let mut responses = Vec::new();
    for q in QUOTAS {
        for cadence in [15, 30, 60, 120, 300] {
            for duration in [0, 5, 20] {
                for phase in [0, 13, 29, 47] {
                    for fractional in [false, true] {
                        for period in [60 * q.scale, 120 * q.scale] {
                            let fixture = Fixture {
                                quota: q.name,
                                cadence,
                                duration,
                                phase,
                                fractional,
                                period,
                                new_period: None,
                                manual: false,
                                jitter_seed: 0,
                            };
                            for variant in VARIANTS {
                                constants.push(serde_json::json!({"fixture":fixture,"variant":variant.name,"metrics":run(q,&fixture,variant)}));
                            }
                        }
                    }
                }
            }
            for duration in [0, 20] {
                for phase in [0, 17] {
                    for fractional in [false, true] {
                        for new_period in [30 * q.scale, 120 * q.scale] {
                            let fixture = Fixture {
                                quota: q.name,
                                cadence,
                                duration,
                                phase,
                                fractional,
                                period: 60 * q.scale,
                                new_period: Some(new_period),
                                manual: false,
                                jitter_seed: 3,
                            };
                            for variant in VARIANTS {
                                responses.push(serde_json::json!({"fixture":fixture,"variant":variant.name,"metrics":run(q,&fixture,variant)}));
                            }
                        }
                    }
                }
            }
        }
    }
    let mut canonical = Vec::new();
    for q in QUOTAS {
        for new_period in [30 * q.scale, 120 * q.scale] {
            let fixture = Fixture {
                quota: q.name,
                cadence: 30 * q.scale,
                duration: 0,
                phase: 0,
                fractional: false,
                period: 60 * q.scale,
                new_period: Some(new_period),
                manual: false,
                jitter_seed: -1,
            };
            for variant in VARIANTS {
                canonical.push(serde_json::json!({"fixture":fixture,"variant":variant.name,"metrics":run(q,&fixture,variant)}));
            }
        }
    }
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../output/burn-rate-replay");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("adaptive-comparison.json"),serde_json::to_vec_pretty(&serde_json::json!({"variants":VARIANTS,"constant":constants,"response":responses,"canonical":canonical})).unwrap()).unwrap();
    println!(
        "{} constant fixtures, {} response fixtures, {} canonical rows",
        constants.len(),
        responses.len(),
        canonical.len()
    );
}

#[test]
fn precision_interpolation_repairs_the_fractional_counterexample() {
    for v in [VARIANTS[2], VARIANTS[4]] {
        let mut e = Experiment::default();
        let mut original = Candidate::default();
        let mut rate = None;
        let mut original_rate = None;
        for at in [0, 125, 251, 378, 506] {
            rate = e.accept(at, observed(at, 47, 60, true), QUOTAS[0], v);
            original_rate = original.accept(
                at,
                observed(at, 47, 60, true),
                600,
                1800,
                2,
                super::original(),
            );
        }
        assert!((rate.unwrap() - 60.0).abs() < 1e-8);
        assert!((original_rate.unwrap() / 60.0 - 1.0).abs() > 0.05);
        println!(
            "fractional counterexample {}: original={} improved={}",
            v.name,
            original_rate.unwrap(),
            rate.unwrap()
        );
        assert_eq!(e.changes, 0);
    }
}

#[test]
fn multi_crossings_from_one_observation_cannot_fake_independent_change_evidence() {
    let mut e = Experiment::default();
    e.accept(0, 10.0, QUOTAS[0], VARIANTS[4]);
    e.accept(60, 20.0, QUOTAS[0], VARIANTS[4]);
    assert_eq!(e.bounds.len(), 10);
    assert_eq!(e.changes, 0);
    assert_eq!(e.fit_start, None);
}

fn canonical_fixture(q: Quota, period: i64) -> Fixture {
    Fixture {
        quota: q.name,
        cadence: 30 * q.scale,
        duration: 0,
        phase: 0,
        fractional: false,
        period: 60 * q.scale,
        new_period: Some(period * q.scale),
        manual: false,
        jitter_seed: -1,
    }
}

#[test]
fn conditional_recovery_improves_both_canonical_response_integrals_without_excursions() {
    for q in QUOTAS {
        for period in [30, 120] {
            let f = canonical_fixture(q, period);
            let old = run(q, &f, VARIANTS[0]);
            let new = run(q, &f, VARIANTS[4]);
            assert!(
                new.error_seconds <= old.error_seconds * 0.9,
                "{f:?}: {new:?} vs {old:?}"
            );
            assert_eq!(new.unavailable, 0);
            assert_eq!(new.seconds, old.seconds);
            assert!(new.excursion <= old.excursion + 1e-8);
            assert!(new.sustained_band.is_some());
            if let Some(old_time) = old.sustained_band {
                assert!(new.sustained_band.unwrap() <= old_time);
            }
        }
    }
}

fn active() -> Experiment {
    let mut e = Experiment::default();
    for at in (0..=2100).step_by(30) {
        let used = if at <= 1800 {
            10.0 + at as f64 / 60.0
        } else {
            40.0 + (at - 1800) as f64 / 30.0
        };
        e.accept(at, used.floor(), QUOTAS[0], VARIANTS[4]);
    }
    assert!(e.fit_start.is_some());
    assert_eq!(e.changes, 1);
    e
}

#[test]
fn active_idle_and_json_restart_preserve_the_clock_ceiling_and_filter() {
    let mut e = active();
    let start = e.fit_start;
    let mut previous = 120.0;
    let mut expired = false;
    for at in (2130..=4200).step_by(30) {
        let rate = e.accept(at, 50.0, QUOTAS[0], VARIANTS[4]);
        if let Some(rate) = rate {
            assert!(!expired);
            assert!(rate <= previous + 1e-8);
            previous = rate;
        } else {
            expired = true;
        }
        let saved = serde_json::to_string(&e).unwrap();
        let loaded: Experiment = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.candidate.upper, Some(2100));
        assert_eq!(loaded.fit_start, start);
        assert_eq!(
            loaded.candidate.ceiling.is_some(),
            e.candidate.ceiling.is_some()
        );
        if let Some((loaded, saved)) = loaded.candidate.ceiling.zip(e.candidate.ceiling) {
            assert!((loaded / saved - 1.0).abs() < 1e-12);
        }
        e = loaded;
    }
    assert!(expired);
    e.accept(4230, 51.0, QUOTAS[0], VARIANTS[4]);
    assert_eq!(e.candidate.upper, Some(4230));
    assert_eq!(e.candidate.ceiling, None);
}

#[test]
fn invalid_stale_conflicting_and_pending_corrections_leave_adaptive_metadata_unchanged() {
    for (at, value) in [
        (2160, f64::NAN),
        (2160, f64::INFINITY),
        (2160, -1.0),
        (2160, 101.0),
        (2070, 50.0),
        (2100, 51.0),
        (2130, 49.0),
    ] {
        let mut e = active();
        let before = serde_json::to_value(&e).unwrap();
        e.accept(at, value, QUOTAS[0], VARIANTS[4]);
        let after = serde_json::to_value(&e).unwrap();
        for key in [
            "bounds",
            "seen_fractional",
            "direction",
            "votes",
            "fit_start",
            "changed_at",
            "settled_at",
            "changes",
        ] {
            assert_eq!(after[key], before[key], "{key}: {at}, {value}");
        }
        assert_eq!(e.candidate.upper, Some(2100));
        assert_eq!(e.candidate.ceiling, None);
    }
}

#[test]
fn gap_correction_and_reset_clear_the_entire_adaptive_selection() {
    for transition in ["gap", "correction", "reset"] {
        let mut e = active();
        match transition {
            "gap" => {
                e.accept(2701, 70.0, QUOTAS[0], VARIANTS[4]);
            }
            "correction" => {
                e.accept(2130, 49.0, QUOTAS[0], VARIANTS[4]);
                e.accept(2160, 49.0, QUOTAS[0], VARIANTS[4]);
            }
            _ => {
                e.candidate.quota.resets_at = Some(2100);
                e.accept_snapshot(
                    2130,
                    QuotaSnapshot {
                        used_percent: Some(1.0),
                        resets_at: Some(5000),
                        window_duration_mins: None,
                    },
                    QUOTAS[0],
                    VARIANTS[4],
                );
            }
        }
        assert!(e.bounds.is_empty());
        assert!(e.candidate.quota.crossings.is_empty());
        assert_eq!(e.fit_start, None);
        assert_eq!(e.changed_at, None);
        assert_eq!(e.settled_at, None);
        assert_eq!(e.votes, 0);
        assert_eq!(e.candidate.upper, None);
        assert_eq!(e.candidate.ceiling, None);
    }
}

fn stress_truth(name: &str, at: i64, scale: i64) -> (f64, f64) {
    let t = at as f64 / scale as f64;
    let (used, rate) = match name {
        "ramp" if t <= 900.0 => (10.0 + t / 60.0, 60.0),
        "ramp" if t <= 1500.0 => {
            let dt = t - 900.0;
            (25.0 + dt / 60.0 + dt * dt / 72000.0, 60.0 + dt / 10.0)
        }
        "ramp" => (40.0 + (t - 1500.0) / 30.0, 120.0),
        "idle_resume" if t <= 900.0 => (10.0 + t / 60.0, 60.0),
        "idle_resume" if t <= 1500.0 => (25.0, 0.0),
        "idle_resume" => (25.0 + (t - 1500.0) / 30.0, 120.0),
        "oscillation" => {
            let cycle = (t / 240.0).floor();
            let tail = t - cycle * 240.0;
            if tail <= 120.0 {
                (10.0 + cycle * 3.0 + tail / 60.0, 60.0)
            } else {
                (12.0 + cycle * 3.0 + (tail - 120.0) / 120.0, 30.0)
            }
        }
        _ => (10.0 + t / 60.0, 60.0),
    };
    (used, rate / scale as f64)
}

#[test]
#[ignore = "manual/ramp/burst/precision-switch stress evidence"]
fn adaptive_stress_replays() {
    let mut rows = Vec::new();
    for q in QUOTAS {
        for cadence in [30, 300] {
            for scenario in [
                "manual_constant",
                "ramp",
                "oscillation",
                "idle_resume",
                "precision_switch",
            ] {
                let times = schedule(
                    2400 * q.scale,
                    cadence,
                    20,
                    11,
                    4,
                    scenario == "manual_constant",
                    None,
                );
                for v in VARIANTS {
                    let mut e = Experiment::default();
                    let mut error = 0.0;
                    let mut measured_seconds = 0;
                    let mut unavailable = 0;
                    let mut held: Option<(i64, f64)> = None;
                    for &at in &times {
                        let (value, truth) = stress_truth(scenario, at, q.scale);
                        let used = if scenario == "precision_switch" && at < 1200 * q.scale {
                            value
                        } else {
                            (value + EPSILON).floor()
                        };
                        let rate = e.accept(at, used, q, v);
                        if let Some((previous_at, previous_rate)) = held {
                            if previous_at >= 900 * q.scale {
                                // Integrate changing truth between accepted observations.
                                // Exploratory Simpson quadrature uses identical truth samples
                                // for all variants; discontinuities/sign changes may add error.
                                let midpoint = (previous_at + at) / 2;
                                let targets = [
                                    stress_truth(scenario, previous_at, q.scale).1,
                                    stress_truth(scenario, midpoint, q.scale).1,
                                    truth,
                                ];
                                let dt = at - previous_at;
                                error += ((previous_rate - targets[0]).abs()
                                    + 4.0 * (previous_rate - targets[1]).abs()
                                    + (previous_rate - targets[2]).abs())
                                    * dt as f64
                                    / 6.0;
                                measured_seconds += dt;
                            }
                        }
                        if at >= 900 * q.scale && rate.is_none() {
                            unavailable += 1;
                        }
                        if let Some(rate) = rate {
                            assert!(rate.is_finite() && rate > 0.0);
                        }
                        held = rate.map(|rate| (at, rate));
                    }
                    rows.push(serde_json::json!({"quota":q.name,"cadence":cadence,"scenario":scenario,"variant":v.name,
                        "absoluteErrorSeconds":error,"measuredSeconds":measured_seconds,"unavailable":unavailable,"changes":e.changes}));
                }
            }
        }
    }
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../output/burn-rate-replay");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("adaptive-stress.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    println!("{} stress rows", rows.len());
}
