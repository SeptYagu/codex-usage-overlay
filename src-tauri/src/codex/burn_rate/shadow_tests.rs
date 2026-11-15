use std::fs;
use std::path::PathBuf;

use super::*;
use crate::codex::{QuotaSnapshot, RawCodexUsage};
use crate::config::ConfigManager;

const RESET_AT: i64 = 100_000;

fn usage(at: i64, five: f64, week: f64) -> RawCodexUsage {
    RawCodexUsage {
        account_id: Some("acct-a".into()),
        five_hour: QuotaSnapshot {
            used_percent: Some(five),
            resets_at: Some(RESET_AT),
            window_duration_mins: None,
        },
        weekly: QuotaSnapshot {
            used_percent: Some(week),
            resets_at: Some(RESET_AT),
            window_duration_mins: None,
        },
        credits_display: "-".into(),
        credits_balance: None,
        has_credits: false,
        fetched_at: at,
        antigravity_weekly: None,
    }
}

struct Harness {
    dir: PathBuf,
    config: ConfigManager,
    tracker: BurnRateTracker,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Harness {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-burn-calibration-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let config = ConfigManager::with_runtime_dir(dir.clone());
        let tracker = BurnRateTracker::load(&config);
        Self { dir, config, tracker }
    }

    fn refresh(&mut self, sample: &RawCodexUsage) -> BurnRateEstimate {
        let estimate = self.tracker.accept_usage(sample);
        if estimate.state_changed || self.tracker.is_dirty() {
            self.tracker.save(&self.config).unwrap();
        }
        estimate
    }

    fn origin(tag: &str) -> Self {
        let mut harness = Self::new(tag);
        harness.refresh(&usage(1000, 10.0, 10.0));
        harness.refresh(&usage(1060, 11.0, 10.0));
        harness.refresh(&usage(1120, 12.0, 10.0));
        harness.refresh(&usage(1180, 13.0, 10.0));
        let pending = harness.shadow().pending.as_ref().unwrap();
        assert_eq!(pending.origin_at, 1180);
        assert!((pending.origin_fixed_rate_per_hour - 60.0).abs() < 1e-9);
        harness
    }

    fn step(&mut self, at: i64, used: f64) {
        self.refresh(&usage(at, used, 10.0));
    }

    fn shadow(&self) -> &FiveHourShadowState {
        &self.tracker.state.five_hour_shadow
    }
}

#[test]
fn pending_origin_waits_for_weighted_rate_and_does_not_overlap() {
    let mut harness = Harness::new("no-overlap");
    harness.refresh(&usage(1000, 10.0, 10.0));
    harness.refresh(&usage(1060, 11.0, 10.0));
    harness.refresh(&usage(1120, 12.0, 10.0));
    assert!(harness.shadow().pending.is_none());

    let est = harness.refresh(&usage(1180, 13.0, 10.0));
    let cand = harness.tracker.candidates(&est);
    assert!((cand.weighted.unwrap() - 60.0).abs() < 1e-9);
    assert_eq!(cand.learning, "accumulating");
    assert_eq!(harness.shadow().pending.as_ref().unwrap().origin_at, 1180);

    harness.step(1240, 14.0);
    let pending = harness.shadow().pending.as_ref().unwrap();
    assert_eq!(pending.origin_at, 1180);
    assert_eq!(pending.last_valid_at, 1240);
}

#[test]
fn five_minute_maturation_trains_affine_and_starts_next_origin() {
    let mut harness = Harness::origin("mature");
    for minute in 1..=4 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    assert_eq!(harness.shadow().affine.n, 0.0);

    let est = harness.refresh(&usage(1480, 18.0, 10.0));
    assert!(harness.shadow().affine.n > 0.0);
    let cand = harness.tracker.candidates(&est);
    assert_eq!(cand.learning, "active");
    assert!(cand.personalized.is_some());
    assert_eq!(harness.shadow().pending.as_ref().unwrap().origin_at, 1480);
    assert_eq!(harness.shadow().pending.as_ref().unwrap().origin_used_percent, 18.0);
}

#[test]
fn interval_rejects_gap_decrease_saturation_too_late_reset_and_backwards() {
    let cases: [(&str, i64, f64, i64); 6] = [
        ("gap", 1280, 14.0, RESET_AT),
        ("backwards", 1120, 13.0, RESET_AT),
        ("decrease", 1240, 12.0, RESET_AT),
        ("saturation", 1480, 100.0, RESET_AT),
        ("too-late", 1580, 18.0, RESET_AT),
        ("reset-mismatch", 1480, 18.0, RESET_AT + 10_000),
    ];
    for (name, at, used, resets_at) in cases {
        let mut harness = Harness::origin(name);
        if name == "too-late" {
            for minute in 1..=4 {
                harness.step(1180 + minute * 60, 13.0 + minute as f64);
            }
        }
        let mut sample = usage(at, used, 10.0);
        sample.five_hour.resets_at = Some(resets_at);
        harness.refresh(&sample);
        assert_eq!(harness.shadow().affine.n, 0.0, "{name} must not train affine");
    }
}

#[test]
fn account_switch_resets_calibration_and_pending_state() {
    let mut harness = Harness::origin("account-switch");
    for minute in 1..=5 {
        harness.step(1180 + minute * 60, 13.0 + minute as f64);
    }
    assert!(harness.shadow().affine.n > 0.0);

    let mut switched = usage(1600, 20.0, 12.0);
    switched.account_id = Some("acct-b".into());
    harness.refresh(&switched);

    assert_eq!(harness.shadow().affine.n, 0.0);
    assert!(harness.shadow().pending.is_none());
    assert_eq!(harness.shadow().weighted.last_crossing_observation_at, None);
}

#[test]
fn normalize_rejects_invalid_or_desynchronized_pending_intervals() {
    let harness = Harness::origin("normalize");
    let quota = harness.tracker.state.five_hour.clone();
    let pending = harness.shadow().pending.clone().unwrap();
    assert!(pending.is_valid_for(&quota));

    let mut broken = pending.clone();
    broken.origin_used_percent = 150.0;
    assert!(!broken.is_valid_for(&quota));

    broken = pending.clone();
    broken.last_valid_used_percent = pending.origin_used_percent - 1.0;
    assert!(!broken.is_valid_for(&quota));

    broken = pending.clone();
    broken.origin_fixed_rate_per_hour = 0.0;
    assert!(!broken.is_valid_for(&quota));

    broken = pending.clone();
    broken.origin_segment_id = quota.segment_id + 1;
    assert!(!broken.is_valid_for(&quota));

    broken = pending.clone();
    broken.last_valid_at = quota.last_observed_at.unwrap_or(0) + 60;
    assert!(!broken.is_valid_for(&quota));

    let mut shadow = FiveHourShadowState::default();
    shadow.pending = Some(broken);
    assert!(shadow.normalize(&quota));
    assert!(shadow.pending.is_none());
}

#[test]
fn v1_and_legacy_v2_states_migrate_losslessly_and_corrupt_subtrees_recover() {
    let v1 = serde_json::json!({
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
    let migrated = load_state(&v1.to_string());
    assert_eq!(migrated.schema_version, SCHEMA_VERSION);
    assert_eq!(migrated.five_hour.crossings.len(), 1);
    assert_eq!(migrated.five_hour_shadow, FiveHourShadowState::default());

    let legacy_v2 = serde_json::json!({
        "schemaVersion": 2,
        "accountId": "acct-a",
        "fiveHour": {
            "resetsAt": RESET_AT, "segmentId": 0, "lastObservedPercent": 15.0,
            "lastObservedAt": 1300, "correctionCandidate": null,
            "crossings": [{"level": 11, "observedAt": 1060, "segmentId": 0}]
        },
        "weekly": {
            "resetsAt": RESET_AT, "segmentId": 0, "lastObservedPercent": 10.0,
            "lastObservedAt": 1300, "correctionCandidate": null, "crossings": []
        },
        "fiveHourShadow": {
            "algorithmVersion": 1,
            "accountGeneration": 3,
            "eventSequence": 42,
            "auditOutbox": {"schemaVersion": 1, "kind": "matured", "eventAt": 1100},
            "personal": {"a": 12.0, "b": 10.0, "evidence": 2.0, "maturedFeedbacks": 2, "informativeUpdates": 2},
            "weighted": {"lastCrossingObservationAt": 1060, "idleRateCeiling": 55.0},
            "affine": {"version": 2, "n": 0.99, "x": 4.95, "y": 4.95, "xx": 24.75, "xy": 24.75, "feedbacks": 1, "lastUpdateAt": 1200},
            "pending": {
                "algorithmVersion": 1,
                "originAt": 1180,
                "originUsedPercent": 13.0,
                "originResetAt": RESET_AT,
                "originFixedRatePerHour": 60.0,
                "originShadowRatePerHour": 62.0,
                "lastValidAt": 1300,
                "lastValidUsedPercent": 15.0,
                "originSegmentId": 0
            }
        }
    });
    let upgraded = load_state(&legacy_v2.to_string());
    assert_eq!(upgraded.five_hour_shadow.weighted.idle_rate_ceiling, Some(55.0));
    assert!((upgraded.five_hour_shadow.affine.n - 0.99).abs() < 1e-12);
    assert_eq!(upgraded.five_hour_shadow.pending.as_ref().unwrap().origin_at, 1180);

    let mut latched_v2 = legacy_v2.clone();
    latched_v2["fiveHourShadow"]["pending"]["latchedRejection"] = serde_json::json!({"reason": "gap", "observedAt": 1300});
    let dropped_latched = load_state(&latched_v2.to_string());
    assert!(dropped_latched.five_hour_shadow.pending.is_none());

    let malformed = serde_json::json!({
        "schemaVersion": 2,
        "accountId": "acct-a",
        "fiveHour": v1["fiveHour"],
        "weekly": v1["weekly"],
        "fiveHourShadow": { "algorithmVersion": "bad" }
    });
    let recovered = load_state(&malformed.to_string());
    assert_eq!(recovered.five_hour.crossings.len(), 1);
    assert_eq!(recovered.five_hour_shadow, FiveHourShadowState::default());

    let mut mismatched = upgraded.clone();
    mismatched.five_hour_shadow.algorithm_version = SHADOW_ALGORITHM_VERSION + 1;
    let isolated = load_state(&serde_json::to_string(&mismatched).unwrap());
    assert_eq!(isolated.five_hour_shadow.algorithm_version, SHADOW_ALGORITHM_VERSION);
    assert_eq!(isolated.five_hour_shadow.affine, affine::AffineCalibration::default());
    assert!(isolated.five_hour_shadow.pending.is_none());
    assert_eq!(isolated.five_hour.crossings.len(), 1);
}

#[test]
fn published_rates_match_golden_vector_and_never_leak_calibration_fields() {
    let samples: [(i64, f64, f64); 14] = [
        (1_000, 10.0, 10.0), (1_060, 13.0, 12.0), (1_120, 16.0, 14.0), (1_240, 19.0, 15.0),
        (1_360, 19.0, 15.0), (1_420, 22.0, 16.0), (1_540, 25.0, 18.0), (1_600, 28.0, 18.0),
        (1_660, 24.0, 19.0), (1_720, 26.0, 20.0), (1_840, 30.0, 22.0), (1_900, 33.0, 24.0),
        (2_600, 40.0, 30.0), (2_660, 44.0, 33.0),
    ];
    let expected: [(Option<f64>, Option<f64>); 14] = [
        (None, None), (Some(160.0), Some(90.0)), (Some(144.0), Some(108.0)),
        (Some(80.0), Some(54.0)), (Some(42.352941176471), Some(33.75)),
        (Some(51.428571428571), Some(31.764705882353)), (Some(80.0), Some(30.0)),
        (Some(120.0), Some(25.714285714286)), (Some(120.0), Some(40.0)),
        (None, Some(41.538461538462)), (Some(112.5), Some(51.428571428571)),
        (Some(128.571428571429), Some(77.142857142857)), (None, Some(27.0)),
        (Some(225.0), Some(67.5)),
    ];
    let mut harness = Harness::new("golden");
    for (index, (at, five, week)) in samples.iter().enumerate() {
        let estimate = harness.refresh(&usage(*at, *five, *week));
        match (estimate.five_hour_per_hour, expected[index].0) {
            (Some(actual), Some(want)) => assert!((actual - want).abs() < 1e-9),
            (None, None) => {}
            (actual, want) => panic!("sample {index}: 5H {actual:?} vs {want:?}"),
        }
        match (estimate.weekly_per_hour, expected[index].1) {
            (Some(actual), Some(want)) => assert!((actual - want).abs() < 1e-9),
            (None, None) => {}
            (actual, want) => panic!("sample {index}: weekly {actual:?} vs {want:?}"),
        }
    }

    let json = serde_json::to_value(usage(1600, 19.0, 10.0).to_ui(Some(120.0), Some(25.0))).unwrap();
    for forbidden in ["personalizedPerHour", "fixedWeightedPerHour", "shadow", "evidence"] {
        assert!(json.get(forbidden).is_none());
    }
}
