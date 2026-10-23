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
    assert!(error.contains("initialize") && error.contains("unsupported client"));
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
                let usage = match mode.as_str() {
                    "success" => {
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
            .stderr(Stdio::inherit())
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
    async fn success_and_protocol_errors_stop_the_owned_process() {
        for (mode, expected_error) in [
            ("success", None),
            ("init-error", Some("init failed")),
            ("usage-error", Some("usage failed")),
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
        assert!(error.to_string().contains("stdout"));
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
