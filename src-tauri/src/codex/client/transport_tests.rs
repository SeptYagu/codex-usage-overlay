use super::*;
use tokio::io::{duplex, AsyncReadExt};

#[tokio::test]
async fn handshake_waits_for_initialization_before_sending_usage_request() {
    let (client, server) = duplex(4096);
    let (client_read, mut client_write) = tokio::io::split(client);
    let (server_read, mut server_write) = tokio::io::split(server);
    let mut reader = BufReader::new(client_read).lines();
    let server = async move {
        let mut requests = BufReader::new(server_read).lines();
        let initialize: Value =
            serde_json::from_str(&requests.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(initialize["method"], "initialize");
        assert_eq!(
            initialize["params"]["clientInfo"]["version"],
            env!("CARGO_PKG_VERSION")
        );
        // Detect the previous batch of initialize/initialized/query messages.
        assert!(timeout(Duration::from_millis(20), requests.next_line())
            .await
            .is_err());
        server_write
            .write_all(b"not json\n\n{\"method\":\"warning\",\"params\":{}}\n")
            .await
            .unwrap();
        send_message(&mut server_write, json!({ "id": 1, "result": {} }))
            .await
            .unwrap();
        let initialized: Value =
            serde_json::from_str(&requests.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(initialized["method"], "initialized");
        let query: Value =
            serde_json::from_str(&requests.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(query["method"], "account/rateLimits/read");
        send_message(&mut server_write, json!({ "id": 900, "result": {} }))
            .await
            .unwrap();
        send_message(
            &mut server_write,
            json!({ "id": 2, "method": "unrelatedServerRequest" }),
        )
        .await
        .unwrap();
        send_message(
            &mut server_write,
            json!({ "id": 2, "result": {
                "rateLimits": { "primary": { "usedPercent": 99.75 } }
            }}),
        )
        .await
        .unwrap();
    };
    let (usage, ()) = timeout(Duration::from_secs(3), async {
        tokio::join!(request_usage(&mut reader, &mut client_write), server)
    })
    .await
    .unwrap();
    assert_eq!(usage.unwrap().five_hour.used_percent, Some(99.75));
}

#[tokio::test]
async fn failed_initialization_returns_immediately_without_querying_usage() {
    let mut reader =
        BufReader::new(&b"{\"id\":1,\"error\":{\"message\":\"unsupported client\"}}\n"[..]).lines();
    let mut written = Vec::new();
    let error = request_usage(&mut reader, &mut written)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("initialize") && error.contains("Rpc"));
    let requests = String::from_utf8(written).unwrap();
    assert_eq!(requests.lines().count(), 1);
    assert!(!requests.contains("account/rateLimits/read"));
}

#[tokio::test]
async fn missing_or_malformed_response_fails_without_waiting_for_timeout() {
    for response in ["", "{\"id\":1}\n", "{\"id\":1,\"result\":null}\n"] {
        let mut reader = BufReader::new(response.as_bytes()).lines();
        assert!(read_response(&mut reader, 1, "initialize").await.is_err());
    }
}

use serde_json::json;

#[test]
#[ignore = "requires an explicitly selected installed Codex and live authenticated quota service"]
fn live_quota_reader_smoke() {
    let exe = std::env::var_os("CODEX_OVERLAY_SMOKE_EXE")
        .expect("select the installed helper explicitly");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut client = CodexClient {
            exe_path: Some(PathBuf::from(exe)),
        };
        let samples = std::env::var("CODEX_OVERLAY_SMOKE_SAMPLES")
            .ok()
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(3)
            .clamp(1, 3);
        for index in 0..samples {
            let started = Instant::now();
            let mut observer = |event| {
                if let ReadEvent::Finished(report) = event {
                    println!(
                        "quota-reader-smoke {}",
                        serde_json::to_string(&report).unwrap()
                    );
                }
                async {}
            };
            let result = client.fetch_usage(&mut observer).await.unwrap();
            assert_eq!(result.usage.five_hour.window_duration_mins, Some(300));
            assert_eq!(result.usage.weekly.window_duration_mins, Some(10080));
            println!(
                "quota-reader-smoke sample={} valid=true actionFresh={}",
                index + 1,
                result.timing.action_eligible()
            );
            if index + 1 < samples {
                tokio::time::sleep_until(started + Duration::from_secs(60)).await;
            }
        }
    });
}

#[tokio::test(start_paused = true)]
async fn real_protocol_accepts_slow_quota_and_keeps_request_start_for_action_age() {
    for delay in [24_110, 27_474, 35_000] {
        let (client, server) = duplex(4096);
        let (client_read, mut writer) = tokio::io::split(client);
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut reader = BufReader::new(client_read).lines();
        let server = async move {
            let mut requests = BufReader::new(server_read).lines();
            let _: Value =
                serde_json::from_str(&requests.next_line().await.unwrap().unwrap()).unwrap();
            send_message(
                &mut server_write,
                json!({"id":1,"result":{"userAgent":"codex_cli_rs/0.163.0-alpha.11.2"}}),
            )
            .await
            .unwrap();
            requests.next_line().await.unwrap().unwrap();
            let query: Value =
                serde_json::from_str(&requests.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(query["method"], "account/rateLimits/read");
            tokio::time::sleep(Duration::from_millis(delay)).await;
            send_message(
                &mut server_write,
                json!({"id":2,"result":{"rateLimits":{"primary":{"usedPercent":80}}}}),
            )
            .await
            .unwrap();
        };
        let mut report = ReadDiagnostic::new(1);
        let mut observer = |_| async {};
        let (result, ()) = tokio::join!(
            request_usage_observed(
                &mut reader,
                &mut writer,
                Instant::now() + Duration::from_secs(43),
                &mut observer,
                &mut report
            ),
            server
        );
        let result = result.unwrap();
        assert_eq!(result.usage.five_hour.used_percent, Some(80.0));
        assert_eq!(report.quota_ms, Some(delay));
        assert_eq!(
            report.cli_version, None,
            "initialize userAgent is not authoritative CLI version metadata"
        );
        assert_eq!(result.timing.action_eligible(), delay <= 30_000);
    }
}

#[tokio::test(start_paused = true)]
async fn initialization_has_its_own_cap_and_quota_stall_uses_remaining_budget() {
    for initialize_ok in [false, true] {
        let (client, server) = duplex(4096);
        let (client_read, mut writer) = tokio::io::split(client);
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut reader = BufReader::new(client_read).lines();
        let server = tokio::spawn(async move {
            let mut requests = BufReader::new(server_read).lines();
            requests.next_line().await.unwrap();
            if initialize_ok {
                send_message(&mut server_write, json!({"id":1,"result":{}}))
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_secs(100)).await;
        });
        let started = Instant::now();
        let mut report = ReadDiagnostic::new(1);
        let error = request_usage_observed(
            &mut reader,
            &mut writer,
            started + Duration::from_secs(43),
            &mut |_| async {},
            &mut report,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(
            error.stage,
            if initialize_ok {
                ReadStage::Quota
            } else {
                ReadStage::Initialize
            }
        );
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(if initialize_ok { 43 } else { 10 })
        );
        server.abort();
    }
}

// These process tests exercise the real cleanup boundary without using a Codex
// account or terminating any pre-existing application. Each helper self-exits
// after 30 seconds even if a regression prevents our cleanup from working.
#[cfg(windows)]
mod process_tests {
    use super::*;
    use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle};
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::WaitForSingleObject;

    const FIXTURE_MODE: &str = "CODEX_OVERLAY_TEST_READER_MODE";
    const READY: &str = "__OVERLAY_USAGE_READER_READY__";

    // Reuse the compiled test binary instead of starting Windows PowerShell.
    // Cold shell startup exceeded the old 10-second setup budget on CI. This
    // ignored entry point is executed by all four ordinary lifecycle tests;
    // it is a child-process fixture, not an additional skipped regression.
    #[test]
    #[ignore = "child-process fixture invoked by the active lifecycle tests"]
    fn reader_process_fixture() {
        use std::io::{BufRead, Write};

        fn read_request(input: &mut impl BufRead) -> Value {
            let mut line = String::new();
            assert!(input.read_line(&mut line).unwrap() > 0);
            serde_json::from_str(&line).unwrap()
        }

        let mode = std::env::var(FIXTURE_MODE).expect("spawn through mock_child");
        let mut input = std::io::stdin().lock();
        let mut output = std::io::stdout().lock();
        writeln!(output, "{READY}").unwrap();
        output.flush().unwrap();

        if mode != "wait" {
            assert_eq!(read_request(&mut input)["method"], "initialize");
            if mode == "early-eof" {
                return;
            }
            let initialized = if mode == "init-error" {
                json!({"id": 1, "error": {"message": "init failed"}})
            } else {
                json!({"id": 1, "result": {}})
            };
            writeln!(output, "{initialized}").unwrap();
            output.flush().unwrap();
            if mode != "init-error" {
                assert_eq!(read_request(&mut input)["method"], "initialized");
                assert_eq!(
                    read_request(&mut input)["method"],
                    "account/rateLimits/read"
                );
                if mode == "stderr-noise" {
                    let mut stderr = std::io::stderr().lock();
                    stderr.write_all(&vec![b'x'; 128 * 1024]).unwrap();
                    writeln!(stderr).unwrap();
                    writeln!(stderr, "private user@host token=should-never-be-saved").unwrap();
                    writeln!(stderr, "{{\"target\":\"codex_models_manager::manager\",\"fields\":{{\"message\":\"failed to refresh available models: request timed out\"}}}}").unwrap();
                    stderr.flush().unwrap();
                }
                let usage = match mode.as_str() {
                    "success" | "stderr-noise" => {
                        json!({"id": 2, "result": {"rateLimits": {"primary": {"usedPercent": 99}}}})
                    }
                    "usage-error" => json!({"id": 2, "error": {"message": "usage failed"}}),
                    _ => panic!("unknown reader fixture mode: {mode}"),
                };
                writeln!(output, "{usage}").unwrap();
                output.flush().unwrap();
            }
        }
        // Keep the real process alive even after stdin closes. A successful
        // lifecycle assertion must observe our cleanup, not natural EOF exit.
        std::thread::sleep(Duration::from_secs(30));
    }

    async fn mock_child(mode: &str) -> (Child, OwnedHandle) {
        let mut child = usage_command(&std::env::current_exe().unwrap())
            .args([
                "--exact",
                "codex::client::transport_tests::process_tests::reader_process_fixture",
                "--ignored",
                "--nocapture",
                "--format=terse",
            ])
            .env(FIXTURE_MODE, mode)
            .stderr(if mode == "stderr-noise" {
                Stdio::piped()
            } else {
                Stdio::inherit()
            })
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let handle = unsafe { BorrowedHandle::borrow_raw(child.raw_handle().unwrap()) }
            .try_clone_to_owned()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        timeout(Duration::from_secs(10), async {
            // Skip test-harness preamble, reading exactly through the marker
            // line so subsequent protocol data remains in the child's pipe.
            let mut line = Vec::new();
            loop {
                let byte = stdout
                    .read_u8()
                    .await
                    .expect("reader fixture exited before ready");
                if byte == b'\n' {
                    if String::from_utf8_lossy(&line).trim_end().ends_with(READY) {
                        break;
                    }
                    line.clear();
                } else {
                    line.push(byte);
                    assert!(line.len() <= 4096, "unexpected reader fixture preamble");
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("reader fixture {mode} (PID {pid}) did not become ready"));
        child.stdout = Some(stdout);
        (child, handle)
    }

    async fn assert_stopped(handle: OwnedHandle) {
        let status = tokio::task::spawn_blocking(move || unsafe {
            WaitForSingleObject(HANDLE(handle.as_raw_handle()), 5000)
        })
        .await
        .unwrap();
        assert_eq!(status, WAIT_OBJECT_0, "owned usage reader was left running");
    }

    #[tokio::test]
    async fn noisy_stderr_cannot_block_the_reader_or_enter_diagnostics() {
        let (child, handle) = mock_child("stderr-noise").await;
        let mut report = ReadDiagnostic::new(1);
        let result = attempt_child(
            child,
            Instant::now() + Duration::from_secs(5),
            &mut |_| async {},
            &mut report,
        )
        .await;
        assert!(result.is_ok());
        assert_stopped(handle).await;
        assert_eq!(report.stderr.oversized_lines, 1);
        assert_eq!(report.stderr.model_timeouts, 1);
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("user@host") && !json.contains("should-never"));
    }

    struct Helpers {
        modes: Vec<&'static str>,
        handles: Vec<OwnedHandle>,
    }
    impl reader::AttemptSource for Helpers {
        async fn attempt(
            &mut self,
            deadline: Instant,
            attempt: u8,
            observer: &mut impl ReadObserver,
        ) -> std::result::Result<ObservedUsage, ReadFailure> {
            let (child, handle) = mock_child(self.modes.remove(0)).await;
            self.handles.push(handle);
            attempt_child(child, deadline, observer, &mut ReadDiagnostic::new(attempt)).await
        }
    }

    #[tokio::test]
    async fn retry_and_cancellation_stop_every_owned_helper() {
        let mut helpers = Helpers {
            modes: vec!["early-eof", "success"],
            handles: Vec::new(),
        };
        assert!(
            reader::read_with_policy(&mut helpers, &mut |_| async {}, Duration::from_secs(20))
                .await
                .is_ok()
        );
        assert_eq!(helpers.handles.len(), 2);
        for handle in helpers.handles {
            assert_stopped(handle).await;
        }
        let mut helpers = Helpers {
            modes: vec!["early-eof", "success"],
            handles: Vec::new(),
        };
        let retry_seen = std::sync::Arc::new(tokio::sync::Notify::new());
        {
            let notify = retry_seen.clone();
            let mut observer = move |event| {
                if matches!(event, ReadEvent::Retrying) {
                    notify.notify_one();
                }
                async {}
            };
            let read =
                reader::read_with_policy(&mut helpers, &mut observer, Duration::from_secs(20));
            tokio::pin!(read);
            tokio::select! {
                _ = retry_seen.notified() => {},
                result = &mut read => panic!("reader completed before retry cancellation: {}", result.is_ok()),
            }
            // Dropping the policy future here cancels the recovery delay.
        }
        assert_eq!(helpers.handles.len(), 1);
        for handle in helpers.handles {
            assert_stopped(handle).await;
        }
    }

    #[tokio::test]
    async fn success_and_protocol_errors_stop_the_owned_process() {
        for (mode, expected_error) in [
            ("success", None),
            ("init-error", Some("initialize")),
            ("usage-error", Some("account/rateLimits/read")),
        ] {
            let (child, handle) = mock_child(mode).await;
            let result = fetch_usage_from_child(child, Duration::from_secs(5)).await;
            assert_stopped(handle).await;
            match expected_error {
                Some(message) => assert!(result.unwrap_err().to_string().contains(message)),
                None => assert_eq!(result.unwrap().five_hour.used_percent, Some(99.0)),
            }
        }
    }

    #[tokio::test]
    async fn timeout_stops_the_owned_process() {
        let (child, handle) = mock_child("wait").await;
        let error = fetch_usage_from_child(child, Duration::from_millis(30))
            .await
            .unwrap_err();
        assert_stopped(handle).await;
        assert!(error.to_string().contains("超时"));
    }

    #[tokio::test]
    async fn missing_pipe_still_stops_the_owned_process() {
        let (mut child, handle) = mock_child("wait").await;
        drop(child.stdout.take());
        let error = fetch_usage_from_child(child, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_stopped(handle).await;
        assert!(error.to_string().contains("process spawn"));
    }

    #[tokio::test]
    async fn cancelling_the_read_stops_the_owned_process() {
        let (child, handle) = mock_child("wait").await;
        // Drop the entire read future while it awaits initialization, before
        // either its own timeout or the explicit kill can run.
        assert!(timeout(
            Duration::from_millis(30),
            fetch_usage_from_child(child, Duration::from_secs(20))
        )
        .await
        .is_err());
        assert_stopped(handle).await;
    }
}
