//! Causal, offline forecasts of future observed quota increments. No production wiring.
use super::*;

#[derive(Clone, Deserialize)]
struct Observation {
    at: i64,
    five: f64,
    weekly: f64,
    reset: i64,
    #[serde(rename = "weekReset")]
    week_reset: i64,
}

impl Observation {
    fn used(&self, q: Quota) -> f64 {
        if q.name == "5H" {
            self.five
        } else {
            self.weekly
        }
    }
    fn reset(&self, q: Quota) -> i64 {
        if q.name == "5H" {
            self.reset
        } else {
            self.week_reset
        }
    }
    fn snapshot(&self, q: Quota) -> QuotaSnapshot {
        QuotaSnapshot {
            used_percent: Some(self.used(q)),
            resets_at: Some(self.reset(q)),
            window_duration_mins: None,
        }
    }
}

#[derive(Deserialize)]
struct Session {
    id: usize,
    observations: Vec<Observation>,
}

#[derive(Deserialize)]
struct Input {
    metadata: serde_json::Value,
    sessions: Vec<Session>,
}

#[derive(Serialize)]
struct Forecast {
    session: usize,
    quota: &'static str,
    horizon: i64,
    at: i64,
    target_at: i64,
    split: &'static str,
    observed_delta: f64,
    predictions: Vec<Option<f64>>,
}

fn causal_rates(observations: &[Observation], q: Quota, v: Variant) -> Vec<Option<f64>> {
    let mut experiment = Experiment::default();
    observations
        .iter()
        .map(|o| experiment.accept_snapshot(o.at, o.snapshot(q), q, v))
        .collect()
}

// Conservative scoring exclusion: no inference across resets, corrections, gaps or saturation.
fn target(observations: &[Observation], origin: usize, horizon: i64, q: Quota) -> Option<usize> {
    let first = &observations[origin];
    if first.used(q) >= 100.0 {
        return None;
    }
    for j in origin + 1..observations.len() {
        let previous = &observations[j - 1];
        let current = &observations[j];
        let gap = current.at - previous.at;
        if gap <= 0
            || gap > 90
            || current.reset(q) != first.reset(q)
            || current.used(q) < previous.used(q)
            || current.used(q) >= 100.0
        {
            return None;
        }
        let elapsed = current.at - first.at;
        if elapsed >= horizon {
            return (elapsed <= horizon + 90).then_some(j);
        }
    }
    None
}

fn forecasts(session: &Session, q: Quota, horizon: i64, split_at: i64) -> Vec<Forecast> {
    let estimates: Vec<_> = VARIANTS
        .iter()
        .map(|&v| causal_rates(&session.observations, q, v))
        .collect();
    let mut out = Vec::new();
    let mut next_origin = i64::MIN;
    for (i, first) in session.observations.iter().enumerate() {
        if first.at < next_origin {
            continue;
        }
        let Some(j) = target(&session.observations, i, horizon, q) else {
            continue;
        };
        let last = &session.observations[j];
        // Do not score an earlier-period forecast whose outcome crosses the holdout boundary.
        if first.at < split_at && last.at >= split_at {
            continue;
        }
        let duration = (last.at - first.at) as f64;
        let predictions = estimates
            .iter()
            .map(|rates| {
                rates[i].map(|rate| (rate * duration / 3600.0).clamp(0.0, 100.0 - first.used(q)))
            })
            .collect();
        out.push(Forecast {
            session: session.id,
            quota: q.name,
            horizon,
            at: first.at,
            target_at: last.at,
            split: if first.at >= split_at {
                "holdout"
            } else {
                "earlier"
            },
            observed_delta: last.used(q) - first.used(q),
            predictions,
        });
        next_origin = last.at; // Non-overlapping intervals within each session/quota/horizon.
    }
    out
}

#[test]
#[ignore = "local quota history forecast; requires extracted real-input.json"]
fn real_usage_forecast_replay() {
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../output/burn-rate-replay");
    let input = std::env::var_os("BURN_RATE_REAL_INPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| output.join("real-input.json"));
    let input: Input = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    // Fixed before scoring; not selected to favor a variant. 2026-10-02 00:00 UTC.
    let split_at = 1_790_899_200;
    let mut rows = Vec::new();
    for session in &input.sessions {
        assert!(session.observations.windows(2).all(|w| w[0].at < w[1].at));
        for q in QUOTAS {
            for horizon in [300, 900, 1800, 3600] {
                rows.extend(forecasts(session, q, horizon, split_at));
            }
        }
    }
    assert!(
        !rows.is_empty(),
        "No continuous forecast intervals available"
    );
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(
        output.join("real-forecasts.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "metadata": input.metadata, "splitAt": split_at,
            "variants": VARIANTS, "forecasts": rows,
        }))
        .unwrap(),
    )
    .unwrap();
    println!("Wrote {} local forecast intervals", rows.len());
}

#[test]
fn future_observations_do_not_change_origin_estimates() {
    let q = QUOTAS[0];
    let prefix: Vec<_> = (0..20)
        .map(|i| Observation {
            at: i * 30,
            five: 10.0 + i as f64,
            weekly: 10.0,
            reset: 10_000,
            week_reset: 100_000,
        })
        .collect();
    let mut suffix = prefix.clone();
    suffix.push(Observation {
        at: 600,
        five: 80.0,
        ..prefix.last().unwrap().clone()
    });
    for v in VARIANTS {
        let before = causal_rates(&prefix, q, v);
        let after = causal_rates(&suffix, q, v);
        assert_eq!(before, after[..prefix.len()]);
    }
}

#[test]
fn scoring_rejects_gaps_resets_corrections_and_saturation() {
    let q = QUOTAS[0];
    let base: Vec<_> = (0..12)
        .map(|i| Observation {
            at: i * 30,
            five: 10.0 + i as f64,
            weekly: 10.0,
            reset: 10_000,
            week_reset: 100_000,
        })
        .collect();
    assert_eq!(target(&base, 0, 300, q), Some(10));
    for mutation in 0..4 {
        let mut rows = base.clone();
        match mutation {
            0 => rows[5].at += 100,
            1 => rows[5].reset += 1,
            2 => rows[5].five = 1.0,
            _ => rows[5].five = 100.0,
        }
        assert_eq!(target(&rows, 0, 300, q), None);
    }
}

#[test]
fn forecast_intervals_do_not_overlap_or_cross_holdout_boundary() {
    let session = Session {
        id: 0,
        observations: (0..61)
            .map(|i| Observation {
                at: i * 30,
                five: 10.0 + (i / 2) as f64,
                weekly: 10.0,
                reset: 10_000,
                week_reset: 100_000,
            })
            .collect(),
    };
    let rows = forecasts(&session, QUOTAS[0], 300, 750);
    assert!(rows.iter().any(|r| r.split == "earlier"));
    assert!(rows.iter().any(|r| r.split == "holdout"));
    assert!(rows.windows(2).all(|w| w[0].target_at <= w[1].at));
    assert!(rows.iter().all(|r| r.at >= 750 || r.target_at < 750));
}
