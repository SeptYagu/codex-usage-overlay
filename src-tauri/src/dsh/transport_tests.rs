//! Offline loopback protocol fixtures, not evidence of a real DSH session.
use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::JoinSet,
};
use tokio_tungstenite::tungstenite::{handshake::derive_accept_key, protocol::Role};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behavior {
    Good,
    SwitchAccount,
    Cached,
    BadRpcId,
    CrossOrigin,
    DeniedWebSocket,
    Bridge,
}
struct Fixture {
    behavior: Behavior,
    active_b: AtomicBool,
    ws_alive: AtomicBool,
    pong_count: AtomicUsize,
    requests: Mutex<Vec<Value>>,
    close_ws: Notify,
    ws_closed: Notify,
}
struct Server {
    origin: String,
    fixture: Arc<Fixture>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let fixture = Arc::new(Fixture {
            behavior,
            active_b: AtomicBool::new(false),
            ws_alive: AtomicBool::new(false),
            pong_count: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            close_ws: Notify::new(),
            ws_closed: Notify::new(),
        });
        let shared = fixture.clone();
        let task = tokio::spawn(async move {
            let mut handlers = JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let (stream,_)=accepted.unwrap(); let state=shared.clone();
                        handlers.spawn(async move { handle(stream,state).await; });
                    },
                    Some(result)=handlers.join_next(), if !handlers.is_empty()=> { result.unwrap(); }
                }
            }
        });
        Self {
            origin,
            fixture,
            task,
        }
    }
    fn link(&self) -> String {
        format!("{}/connect?token=SECRET&interactions=none", self.origin)
    }
}

async fn request(stream: &mut TcpStream) -> (String, Vec<(String, String)>, Value) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 32768);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = head.split("\r\n");
    let path = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let headers: Vec<_> = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    let length = headers
        .iter()
        .find(|(key, _)| key == "content-length")
        .map(|(_, value)| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
    };
    (path, headers, body)
}
async fn respond(stream: &mut TcpStream, status: &str, extra: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{extra}\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}
async fn handle(mut stream: TcpStream, state: Arc<Fixture>) {
    let (path, headers, body) = request(&mut stream).await;
    if path.starts_with("/connect?") {
        let location = if state.behavior == Behavior::CrossOrigin {
            "http://example.org/api/remote.mux?token=SECRET"
        } else {
            "./api/remote.mux"
        };
        respond(
            &mut stream,
            "302 Found",
            &format!(
                "Location: {location}\r\nSet-Cookie: peer=fixture-secret; HttpOnly; Path=/\r\n"
            ),
            "",
        )
        .await;
        return;
    }
    let cookie = headers
        .iter()
        .find(|(key, _)| key == "cookie")
        .map(|(_, value)| value.as_str());
    if state.behavior == Behavior::Bridge {
        assert!(cookie.is_none());
        assert_eq!(
            headers
                .iter()
                .find(|(key, _)| key == "authorization")
                .map(|(_, value)| value.as_str()),
            Some("Bearer 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
    } else {
        assert_eq!(cookie, Some("peer=fixture-secret"));
    }
    if path == "/api/remote.mux" {
        if state.behavior == Behavior::DeniedWebSocket {
            respond(&mut stream, "403 Forbidden", "", "").await;
            return;
        }
        let key = headers
            .iter()
            .find(|(key, _)| key == "sec-websocket-key")
            .unwrap()
            .1
            .clone();
        let response=format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",derive_accept_key(key.as_bytes()));
        state.ws_alive.store(true, Ordering::Release);
        stream.write_all(response.as_bytes()).await.unwrap();
        let mut ws =
            tokio_tungstenite::WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
        ws.send(Message::Ping(vec![1, 2, 3].into())).await.unwrap();
        loop {
            tokio::select! {
                _=state.close_ws.notified()=>{let _=ws.close(None).await;break;},
                frame=ws.next()=>match frame {
                    Some(Ok(Message::Pong(_)))=>{state.pong_count.fetch_add(1,Ordering::AcqRel);},
                    Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,
                    Some(Ok(Message::Ping(_)))=>{let _=ws.flush().await;},
                    _=>{},
                }
            }
        }
        state.ws_alive.store(false, Ordering::Release);
        state.ws_closed.notify_one();
        return;
    }
    if state.behavior != Behavior::Bridge && !state.ws_alive.load(Ordering::Acquire) {
        respond(&mut stream, "401 Unauthorized", "", "").await;
        return;
    }
    let method = body["method"].as_str().unwrap();
    assert_eq!(path, format!("/api/{method}"));
    assert_eq!(body["type"], "client-request");
    assert!(Uuid::parse_str(body["rpcId"].as_str().unwrap()).is_ok());
    let value = match method {
        "codex-subscription/status" => {
            let active_b = state.active_b.load(Ordering::Acquire);
            json!({"authenticated":true,"provider":"openai-codex","accounts":[{"id":"a","label":"A","active":!active_b},{"id":"b","label":"B","active":active_b}]})
        }
        "codex-subscription/usage/accounts" => {
            assert_eq!(body["payload"], json!({"force":true}));
            let fetched = now_ms()
                - if state.behavior == Behavior::Cached {
                    60_000
                } else {
                    0
                };
            if state.behavior == Behavior::SwitchAccount {
                state.active_b.store(true, Ordering::Release);
            }
            json!({"accounts":[{"id":"a","usage":{"fetchedAt":fetched,"rateLimits":[{"id":"codex","windows":[{"windowSeconds":18000,"usedPercent":90,"remainingPercent":10,"resetsAt":fetched/1000+10000},{"windowSeconds":604800,"usedPercent":60,"remainingPercent":40,"resetsAt":fetched/1000+600000}]}]}},{"id":"b","error":"Fixture account not signed in"}]})
        }
        "session/list" => {
            assert_eq!(body["payload"], json!({"args":{"request":{}}}));
            json!({"items":[{"sessionId":"session-f5409f91-ec39-462d-9964-596e18e2a36c","agentAvailable":true,"running":true,"blank":false,"formatStatus":"current","projections":{"kind":"cached","asOfSeq":1,"values":{"title":"Fixture session","modelSelection":{"lastUsed":{"provider":"openai-codex"},"next":{"provider":"other"}}}}}]})
        }
        "session/prompt" => {
            let request = &body["payload"]["args"]["request"];
            assert_eq!(
                request["sessionId"],
                "session-f5409f91-ec39-462d-9964-596e18e2a36c"
            );
            assert_eq!(request["mode"], "steer");
            assert_eq!(request["content"], json!([{"type":"text","text":"暂停"}]));
            assert!(Uuid::parse_str(request["requestId"].as_str().unwrap()).is_ok());
            json!({"accepted":true})
        }
        _ => panic!("Unexpected fixture method"),
    };
    let rpc_id = if state.behavior == Behavior::BadRpcId {
        json!("wrong")
    } else {
        body["rpcId"].clone()
    };
    let response =
        json!({"type":"server-response","rpcId":rpc_id,"result":{"ok":true,"value":value}})
            .to_string();
    state.requests.lock().await.push(body);
    respond(
        &mut stream,
        "200 OK",
        "Content-Type: application/json\r\n",
        &response,
    )
    .await;
}

#[tokio::test]
async fn pairing_owns_websocket_and_sends_exact_namespaced_http_envelopes() {
    let server = Server::start(Behavior::Good).await;
    let manager = Manager::default();
    let status = manager.connect(&server.link()).await.unwrap();
    assert!(status.connected);
    assert!(status.error.is_none(), "{:?}", status.error);
    assert_eq!(status.sessions.len(), 1);
    assert_eq!(
        manager.origin().await.as_deref(),
        Some(server.origin.as_str())
    );
    let first_instance = status.instance_id.unwrap();
    let raw = manager.read_usage("a").await.unwrap();
    assert_eq!(raw.five_hour.used_percent, Some(90.0));
    let result = manager
        .pause(
            "a",
            &["session-f5409f91-ec39-462d-9964-596e18e2a36c".into()],
            "暂停",
        )
        .await
        .unwrap();
    assert!(result.accepted);
    assert!(!result.stopped);
    assert_eq!(result.sessions.len(), 1);
    assert!(server.fixture.pong_count.load(Ordering::Acquire) > 0);
    let requests = server.fixture.requests.lock().await.clone();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "session/prompt")
            .count(),
        1
    );
    manager.disconnect().await;
    assert!(!manager.status().await.connected);
    timeout(Duration::from_secs(2), server.fixture.ws_closed.notified())
        .await
        .unwrap();
    let status = manager.connect(&server.link()).await.unwrap();
    assert_ne!(status.instance_id.as_deref(), Some(first_instance.as_str()));
    manager.disconnect().await;
}
#[tokio::test]
async fn switched_account_and_earlier_flight_never_become_fresh_usage() {
    for behavior in [
        Behavior::SwitchAccount,
        Behavior::Cached,
        Behavior::BadRpcId,
    ] {
        let server = Server::start(behavior).await;
        let manager = Manager::default();
        manager.connect(&server.link()).await.unwrap();
        assert!(manager.read_usage("a").await.is_err());
        manager.disconnect().await;
    }
}
#[tokio::test]
async fn invalid_targets_are_not_prompted_and_partial_results_are_explicit() {
    let server = Server::start(Behavior::Good).await;
    let manager = Manager::default();
    manager.connect(&server.link()).await.unwrap();
    let result = manager
        .pause(
            "a",
            &[
                "missing".into(),
                "session-f5409f91-ec39-462d-9964-596e18e2a36c".into(),
            ],
            "暂停",
        )
        .await
        .unwrap();
    assert!(result.accepted);
    assert!(!result.stopped);
    assert!(!result.sessions[0].accepted);
    assert!(result.sessions[1].accepted);
    assert_eq!(
        server
            .fixture
            .requests
            .lock()
            .await
            .iter()
            .filter(|request| request["method"] == "session/prompt")
            .count(),
        1
    );
    manager.disconnect().await;
}
#[tokio::test]
async fn closed_mux_invalidates_native_connection() {
    let server = Server::start(Behavior::Good).await;
    let manager = Manager::default();
    manager.connect(&server.link()).await.unwrap();
    server.fixture.close_ws.notify_one();
    timeout(Duration::from_secs(2), server.fixture.ws_closed.notified())
        .await
        .unwrap();
    assert!(!manager.status().await.connected);
    assert!(manager.read_usage("a").await.is_err());
}
struct TempRecord {
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
}
impl Drop for TempRecord {
    fn drop(&mut self) {
        // These exact absolute paths are created below under a new UUID-owned
        // directory. Never recurse or delete any caller-provided path.
        if self.path.parent() == Some(self.directory.as_path()) {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_dir(&self.directory);
        }
    }
}
async fn fixture_record(origin: &str) -> TempRecord {
    let directory =
        std::env::temp_dir().join(format!("codex-overlay-dsh-fixture-{}", Uuid::new_v4()));
    tokio::fs::create_dir(&directory).await.unwrap();
    let directory = tokio::fs::canonicalize(directory).await.unwrap();
    let id = Uuid::new_v4().to_string();
    let path = directory.join(format!("{id}.json"));
    let value = json!({"schemaVersion":1,"instanceId":id,"origin":origin,"token":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","profileLabel":"Offline fixture"});
    tokio::fs::write(&path, value.to_string()).await.unwrap();
    TempRecord { directory, path }
}
#[tokio::test]
async fn companion_uses_bearer_and_record_rotation_invalidates_the_binding() {
    let server = Server::start(Behavior::Bridge).await;
    let manager = Manager::default();
    let record = fixture_record(&server.origin).await;
    let parsed = bridge::read_record_in_directory(&record.path, &record.directory)
        .await
        .unwrap();
    let status = {
        let mut state = manager.inner.lock().await;
        install_bridge(&mut state, parsed).await.unwrap()
    };
    assert!(status.connected);
    assert!(status.error.is_none());
    assert!(manager.read_usage("a").await.is_ok());
    let pause = manager
        .pause(
            "a",
            &["session-f5409f91-ec39-462d-9964-596e18e2a36c".into()],
            "暂停",
        )
        .await
        .unwrap();
    assert!(pause.accepted);
    assert!(!pause.stopped);
    assert!(!server.fixture.ws_alive.load(Ordering::Acquire));
    let requests_before = server.fixture.requests.lock().await.len();
    let mut value: Value =
        serde_json::from_slice(&tokio::fs::read(&record.path).await.unwrap()).unwrap();
    value["token"] = json!("fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210");
    tokio::fs::write(&record.path, value.to_string())
        .await
        .unwrap();
    assert!(!manager.status().await.connected);
    assert!(manager.read_usage("a").await.is_err());
    assert_eq!(requests_before, server.fixture.requests.lock().await.len());
}
#[tokio::test]
async fn discovery_records_cannot_escape_the_selected_directory_or_filename_identity() {
    let one = fixture_record("http://127.0.0.1:1").await;
    let two = fixture_record("http://127.0.0.1:2").await;
    assert!(bridge::read_record_in_directory(&one.path, &two.directory)
        .await
        .is_err());
    let mut value: Value =
        serde_json::from_slice(&tokio::fs::read(&one.path).await.unwrap()).unwrap();
    value["instanceId"] = json!(Uuid::new_v4().to_string());
    tokio::fs::write(&one.path, value.to_string())
        .await
        .unwrap();
    assert!(bridge::read_record_in_directory(&one.path, &one.directory)
        .await
        .is_err());
}
#[tokio::test(start_paused = true)]
async fn pause_total_budget_includes_waiting_for_another_operation() {
    let manager = Manager::default();
    let _held = manager.inner.lock().await;
    let started = tokio::time::Instant::now();
    let error = manager
        .pause(
            "a",
            &["session-f5409f91-ec39-462d-9964-596e18e2a36c".into()],
            "暂停",
        )
        .await
        .unwrap_err();
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert!(error.contains("no request was sent"));
}

#[tokio::test]
async fn redirect_and_websocket_rejection_do_not_expose_secret_or_fallback() {
    for behavior in [Behavior::CrossOrigin, Behavior::DeniedWebSocket] {
        let server = Server::start(behavior).await;
        let manager = Manager::default();
        let error = manager.connect(&server.link()).await.unwrap_err();
        assert!(!error.contains("SECRET"));
        assert!(!manager.status().await.connected);
        assert!(server.fixture.requests.lock().await.is_empty());
    }
}
