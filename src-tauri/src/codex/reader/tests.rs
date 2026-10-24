use super::*;

struct Scripted {
    outcomes: Vec<(Duration, Result<(), ReadFailure>)>,
    calls: u8,
}
impl AttemptSource for Scripted {
    async fn attempt(
        &mut self,
        _deadline: Instant,
        attempt: u8,
        observer: &mut impl ReadObserver,
    ) -> Result<ObservedUsage, ReadFailure> {
        self.calls += 1;
        let (delay, result) = self.outcomes.remove(0);
        let started = Instant::now();
        sleep(delay).await;
        let mut report = ReadDiagnostic::new(attempt);
        report.elapsed_ms = millis(delay);
        report.success = result.is_ok();
        report.failure = result.as_ref().err().cloned();
        if let Err(error) = &result {
            observer.event(ReadEvent::Failed(error.clone())).await;
        }
        observer.event(ReadEvent::Finished(report)).await;
        result?;
        let usage = RawCodexUsage {
            account_id: Some("test".into()),
            five_hour: Default::default(),
            weekly: Default::default(),
            credits_display: "—".into(),
            credits_balance: None,
            has_credits: false,
            fetched_at: unix_now(),
        };
        Ok(ObservedUsage {
            usage,
            timing: ObservationTiming {
                quota_started: started,
                received: Instant::now(),
            },
        })
    }
}

#[tokio::test(start_paused = true)]
async fn observed_slow_responses_succeed_without_extending_action_freshness() {
    for delay in [24_110, 27_474, 35_000] {
        let mut source = Scripted {
            outcomes: vec![(Duration::from_millis(delay), Ok(()))],
            calls: 0,
        };
        let value = read_with_policy(&mut source, &mut |_| async {}, REFRESH_BUDGET)
            .await
            .unwrap();
        assert_eq!(value.timing.action_eligible(), delay <= 30_000);
        assert_eq!(source.calls, 1);
        if delay == 27_474 {
            sleep(Duration::from_millis(2_527)).await;
            assert!(!value.timing.action_eligible());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn retries_only_early_transient_failures_with_one_shared_budget() {
    let mut source = Scripted {
        outcomes: vec![
            (
                Duration::from_secs(1),
                Err(ReadFailure::new(ReadStage::Quota, FailureKind::Transport)),
            ),
            (Duration::from_secs(2), Ok(())),
        ],
        calls: 0,
    };
    let started = Instant::now();
    read_with_policy(&mut source, &mut |_| async {}, REFRESH_BUDGET)
        .await
        .unwrap();
    assert_eq!(source.calls, 2);
    assert_eq!(started.elapsed(), Duration::from_secs(4));
    let mut source = Scripted {
        outcomes: vec![
            (
                Duration::from_secs(10),
                Err(ReadFailure::timeout(ReadStage::Initialize)),
            ),
            (Duration::from_secs(60), Ok(())),
        ],
        calls: 0,
    };
    let started = Instant::now();
    assert!(
        read_with_policy(&mut source, &mut |_| async {}, REFRESH_BUDGET)
            .await
            .is_err()
    );
    assert_eq!(started.elapsed(), REFRESH_BUDGET);
    assert_eq!(source.calls, 2);
}

#[tokio::test(start_paused = true)]
async fn permanent_or_late_failures_do_not_retry() {
    for (delay, error) in [
        (
            1,
            ReadFailure::new(ReadStage::Quota, FailureKind::Authentication),
        ),
        (
            1,
            ReadFailure::new(ReadStage::Parse, FailureKind::InvalidQuota),
        ),
        (1, ReadFailure::new(ReadStage::Quota, FailureKind::Rpc)),
        (43, ReadFailure::timeout(ReadStage::Quota)),
    ] {
        let mut source = Scripted {
            outcomes: vec![(Duration::from_secs(delay), Err(error))],
            calls: 0,
        };
        assert!(
            read_with_policy(&mut source, &mut |_| async {}, REFRESH_BUDGET)
                .await
                .is_err()
        );
        assert_eq!(source.calls, 1);
    }
}

#[test]
fn rpc_retry_requires_explicit_transient_status_and_respects_retry_delay() {
    let value = serde_json::json!({"code": -32603, "message": "secret token", "data": {"status": 503, "retryAfterSeconds": 90}});
    let error = ReadFailure::rpc(ReadStage::Quota, &value);
    assert!(error.transient());
    assert!(!should_retry(
        &error,
        1,
        Duration::from_secs(40),
        Duration::from_secs(90)
    ));
    assert!(!error.to_string().contains("secret"));
    for status in [401, 403, 400] {
        assert!(!ReadFailure::rpc(
            ReadStage::Quota,
            &serde_json::json!({"data":{"status":status}})
        )
        .transient());
    }
}

#[test]
fn history_retains_failure_after_success_and_bounds_age_size_and_count() {
    let mut history = ReadHistory::default();
    let mut failed = ReadDiagnostic::new(1);
    failed.failure = Some(ReadFailure::timeout(ReadStage::Quota));
    history.record(failed.clone());
    for _ in 0..250 {
        let mut event = ReadDiagnostic::new(1);
        event.success = true;
        history.record(event);
    }
    assert_eq!(history.last_failure, Some(failed));
    assert_eq!(history.events.len(), HISTORY_EVENTS);
    assert!(serde_json::to_vec(&history).unwrap().len() <= HISTORY_BYTES);
    history.prune(unix_now() + HISTORY_AGE + 10);
    assert!(
        history.events.is_empty()
            && history.last_failure.is_none()
            && history.last_success.is_none()
    );
}

#[test]
fn diagnostics_roundtrip_and_corruption_are_nonfatal_and_reject_sensitive_version() {
    let dir = std::env::temp_dir().join(format!("quota-history-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("history.json");
    let mut history = ReadHistory::default();
    let mut event = ReadDiagnostic::new(1);
    event.success = true;
    history.record(event);
    history.save(&path).unwrap();
    assert!(ReadHistory::load(&path).last_success.is_some());
    let mut event = ReadDiagnostic::new(2);
    event.success = true;
    event.cli_version = Some("user@host/secret".into());
    history.record(event);
    assert!(history.last_success.is_none());
    std::fs::write(&path, b"invalid").unwrap();
    assert!(ReadHistory::load(&path).events.is_empty());
    assert!(history.save(&dir).is_err());
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir(dir);
}
