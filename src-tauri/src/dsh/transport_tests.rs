//! Offline loopback protocol fixtures, not evidence of a real DSH session.
use super::*;
const AUTO_ACCOUNT: &str = "51e9468d-0841-448a-9145-0d46e6319422";
const AUTO_ACCOUNT_B: &str = "501509df-7561-441d-a98e-7a5c92680c93";
const AUTO_SESSION: &str = "session-f5409f91-ec39-462d-9964-596e18e2a36c";
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
    AutoBridge,
}
struct Fixture {
    behavior: Behavior,
    active_b: AtomicBool,
    ws_alive: AtomicBool,
    pong_count: AtomicUsize,
    requests: Mutex<Vec<Value>>,
    close_ws: Notify,
    ws_closed: Notify,
    hold_status: AtomicBool,
    status_started: Notify,
    status_release: Notify,
    sessions_read: Notify,
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
            hold_status: AtomicBool::new(false),
            status_started: Notify::new(),
            status_release: Notify::new(),
            sessions_read: Notify::new(),
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
    let _ = stream.write_all(response.as_bytes()).await;
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
    if matches!(state.behavior, Behavior::Bridge | Behavior::AutoBridge) {
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
    if !matches!(state.behavior, Behavior::Bridge | Behavior::AutoBridge)
        && !state.ws_alive.load(Ordering::Acquire)
    {
        respond(&mut stream, "401 Unauthorized", "", "").await;
        return;
    }
    let method = body["method"].as_str().unwrap();
    assert_eq!(path, format!("/api/{method}"));
    assert_eq!(body["type"], "client-request");
    assert!(Uuid::parse_str(body["rpcId"].as_str().unwrap()).is_ok());
    let value = match method {
        "codex-subscription/status" => {
            if state.hold_status.swap(false, Ordering::AcqRel) {
                state.status_started.notify_one();
                state.status_release.notified().await;
            }
            let active_b = state.active_b.load(Ordering::Acquire);
            let (a, b) = if state.behavior == Behavior::AutoBridge {
                (AUTO_ACCOUNT, AUTO_ACCOUNT_B)
            } else {
                ("a", "b")
            };
            json!({"authenticated":true,"provider":"openai-codex","accounts":[{"id":a,"label":"A","active":!active_b},{"id":b,"label":"B","active":active_b}]})
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
            let (a, b) = if state.behavior == Behavior::AutoBridge {
                (AUTO_ACCOUNT, AUTO_ACCOUNT_B)
            } else {
                ("a", "b")
            };
            json!({"accounts":[{"id":a,"usage":{"fetchedAt":fetched,"rateLimits":[{"id":"codex","windows":[{"windowSeconds":18000,"usedPercent":90,"remainingPercent":10,"resetsAt":fetched/1000+10000},{"windowSeconds":604800,"usedPercent":60,"remainingPercent":40,"resetsAt":fetched/1000+600000}]}]}},{"id":b,"error":"Fixture account not signed in"}]})
        }
        "session/list" => {
            assert_eq!(body["payload"], json!({"args":{"_request":{}}}));
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
    if method == "session/list" {
        state.sessions_read.notify_one();
    }
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

// These exercise the native startup entry directly: no Settings webview, no
// real DSH, no installation, no local Codex files or named-pipe transport.
struct NoWindow;
impl crate::tray::TrayPresentationSink for NoWindow {
    fn present(&self, _: crate::tray::TrayPresentation) {}
    fn app_handle(&self) -> Option<&tauri::AppHandle> {
        None
    }
}
impl crate::commands::UsageSource for NoWindow {
    async fn read_usage(&self, state: &crate::commands::AppState) -> Result<RawCodexUsage, String> {
        let generation = state.source_generation.load(Ordering::Acquire);
        let settings = state.settings.lock().await.clone();
        crate::commands::read_dsh_usage(state, &settings, generation).await
    }
}
struct AutoRuntime {
    directory: std::path::PathBuf,
    state: Arc<crate::commands::AppState>,
    record: std::path::PathBuf,
    id: String,
}
impl Drop for AutoRuntime {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl AutoRuntime {
    async fn new(server: &Server) -> Self {
        let directory =
            std::env::temp_dir().join(format!("overlay-auto-fixture-{}", Uuid::new_v4()));
        std::fs::create_dir_all(directory.join("dsh-bridges")).unwrap();
        let manager = crate::config::ConfigManager::with_runtime_dir(directory.clone());
        let settings = crate::config::OverlaySettings {
            provider: crate::config::Provider::Dsh,
            auto_pause_on_low_quota: true,
            auto_pause_five_hour_threshold_percent: 100,
            auto_pause_weekly_threshold_percent: 100,
            ..Default::default()
        };
        manager.save_settings_checked(&settings).unwrap();
        let id = Uuid::new_v4().to_string();
        let record = directory.join("dsh-bridges").join(format!("{id}.json"));
        tokio::fs::write(
            &record,
            json!({"schemaVersion":1,"instanceId":id,"origin":server.origin,
            "token":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "updatedAt":now_ms(),"profileId":"a".repeat(64)})
            .to_string(),
        )
        .await
        .unwrap();
        Self {
            directory,
            state: Arc::new(crate::commands::AppState::for_test(manager)),
            record,
            id,
        }
    }
}
async fn assert_no_pause(server: &Server) {
    assert!(!server
        .fixture
        .requests
        .lock()
        .await
        .iter()
        .any(|request| request["method"] == "session/prompt"));
}

async fn enable_provider_monitors(runtime: &AutoRuntime) {
    let mut settings = runtime.state.settings.lock().await;
    settings.migrate();
    settings.provider = crate::config::Provider::Codex;
    settings.mirror_display_preferences();
    runtime.state.config_manager.save_settings_checked(&settings).unwrap();
    crate::monitor::initialize(&runtime.state, &settings);
}

#[tokio::test]
async fn automatic_dsh_connects_and_binds_in_background_without_resetting_other_providers() {
    use crate::config::Provider;
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    enable_provider_monitors(&runtime).await;
    let registry = runtime.state.monitors.get().unwrap();
    let codex = registry.get(Provider::Codex);
    let claude = registry.get(Provider::Claude);
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    let settings = runtime.state.settings.lock().await.clone();
    assert_eq!(settings.provider, Provider::Codex);
    assert_eq!(settings.dsh_account_id, AUTO_ACCOUNT);
    assert!(Arc::ptr_eq(&codex, &registry.get(Provider::Codex)));
    assert!(Arc::ptr_eq(&claude, &registry.get(Provider::Claude)));
    let dsh = registry.get(Provider::Dsh);
    let usage = crate::commands::refresh_usage(&NoWindow, &dsh).await.unwrap();
    assert!(usage.five_hour_remaining_percent.is_some());
    assert!(codex.last_usage.lock().await.is_none());
    assert!(dsh.config_manager.burn_rate_state_path().is_file());
    let status = runtime.state.dsh.cached_status();
    crate::commands::bind_dsh_targets(&NoWindow, &runtime.state, AUTO_ACCOUNT.into(),
        vec![AUTO_SESSION.into()], status.instance_id.clone().unwrap(), status.connection_revision).await.unwrap();
    let generation = registry.get(Provider::Dsh).source_generation.load(Ordering::Acquire);
    crate::commands::apply_settings_patch(&NoWindow, &runtime.state, json!({"provider":"claude"}), false).await.unwrap();
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert_eq!(runtime.state.settings.lock().await.provider, Provider::Claude);
    assert_eq!(runtime.state.settings.lock().await.dsh_session_ids, [AUTO_SESSION]);
    assert_eq!(runtime.state.dsh.cached_status().connection_revision, status.connection_revision);
    assert_eq!(registry.get(Provider::Dsh).source_generation.load(Ordering::Acquire), generation);
    crate::commands::apply_settings_patch(&NoWindow, &runtime.state,
        json!({"providerPreferences":{"dsh":{"monitorEnabled":false}}}), false).await.unwrap();
    let requests = server.fixture.requests.lock().await.len();
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert_eq!(server.fixture.requests.lock().await.len(), requests);
    assert!(!runtime.state.dsh.cached_status().connected);
    assert!(crate::commands::refresh_usage(&NoWindow, &registry.get(Provider::Dsh)).await.is_err());
    crate::commands::apply_settings_patch(&NoWindow, &runtime.state,
        json!({"providerPreferences":{"dsh":{"monitorEnabled":true}}}), false).await.unwrap();
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert!(runtime.state.dsh.cached_status().connected);
    assert!(runtime.state.settings.lock().await.dsh_session_ids.is_empty());
    assert_eq!(runtime.state.settings.lock().await.provider, Provider::Claude);
    assert_no_pause(&server).await;
    assert!(crate::commands::apply_settings_patch(&NoWindow, &runtime.state,
        json!({"dshAccountId":AUTO_ACCOUNT,"dshSessionIds":[AUTO_SESSION]}), false).await.is_err(),
        "a generic patch must not bypass the guarded connection revision");
    assert!(runtime.state.settings.lock().await.dsh_session_ids.is_empty());
}

#[tokio::test]
async fn disabling_background_dsh_cancels_an_automatic_handshake_before_installation() {
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    enable_provider_monitors(&runtime).await;
    server.fixture.hold_status.store(true, Ordering::Release);
    let state = runtime.state.clone();
    let attempt = tokio::spawn(async move { auto::reconcile(&NoWindow, &state, false).await; });
    timeout(Duration::from_secs(2), server.fixture.status_started.notified()).await.unwrap();
    crate::commands::apply_settings_patch(&NoWindow, &runtime.state,
        json!({"providerPreferences":{"dsh":{"monitorEnabled":false}}}), false).await.unwrap();
    timeout(Duration::from_secs(2), attempt).await.unwrap().unwrap();
    server.fixture.status_release.notify_one();
    assert!(!runtime.state.dsh.cached_status().connected);
    assert!(runtime.state.settings.lock().await.dsh_instance_id.is_empty());
    assert_eq!(runtime.state.settings.lock().await.provider, crate::config::Provider::Codex);
    assert_no_pause(&server).await;
}

#[tokio::test]
async fn native_disconnect_clears_background_dsh_binding_and_stays_suspended() {
    use crate::config::Provider;
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    enable_provider_monitors(&runtime).await;
    let registry = runtime.state.monitors.get().unwrap();
    let codex = registry.get(Provider::Codex);
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    let status = runtime.state.dsh.cached_status();
    crate::commands::bind_dsh_targets(&NoWindow, &runtime.state, AUTO_ACCOUNT.into(),
        vec![AUTO_SESSION.into()], status.instance_id.unwrap(), status.connection_revision).await.unwrap();
    let disconnected = crate::commands::disconnect_dsh(&NoWindow, &runtime.state).await.unwrap();
    assert!(!disconnected.connected);
    assert_eq!(disconnected.auto_phase, "suspended");
    let settings = runtime.state.config_manager.load_settings();
    assert_eq!(settings.provider, Provider::Codex);
    assert!(settings.dsh_instance_id.is_empty());
    assert!(settings.dsh_account_id.is_empty());
    assert!(settings.dsh_session_ids.is_empty());
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert!(!runtime.state.dsh.cached_status().connected);
    assert!(Arc::ptr_eq(&codex, &registry.get(Provider::Codex)));
    assert!(registry.get(Provider::Dsh).monitor_enabled.load(Ordering::Acquire));
    assert_no_pause(&server).await;
}

#[tokio::test]
async fn native_startup_discovers_without_settings_and_is_read_only_and_idempotent() {
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    let local = Arc::new(crate::pause_control::testing::ScriptedTransport::new());
    runtime
        .state
        .auto_pause
        .lock()
        .unwrap()
        .set_transport(local.clone());
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    let settings = runtime.state.settings.lock().await.clone();
    assert_eq!(settings.dsh_account_id, AUTO_ACCOUNT);
    assert_eq!(settings.dsh_instance_id, runtime.id);
    assert_eq!(settings.dsh_profile_id, "a".repeat(64));
    assert!(settings.dsh_session_ids.is_empty());
    assert!(settings.auto_pause_on_low_quota);
    assert_eq!(
        runtime.state.dsh_auto.status(&runtime.state.dsh).auto_phase,
        "ready"
    );
    let revision = runtime.state.settings_revision.load(Ordering::Acquire);
    let generation = runtime.state.source_generation.load(Ordering::Acquire);
    let connection_revision = runtime.state.dsh.cached_status().connection_revision;
    let disk = tokio::fs::read(runtime.directory.join("settings.json"))
        .await
        .unwrap();
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert_eq!(
        runtime.state.settings_revision.load(Ordering::Acquire),
        revision
    );
    assert_eq!(
        runtime.state.source_generation.load(Ordering::Acquire),
        generation
    );
    assert_eq!(
        runtime.state.dsh.cached_status().connection_revision,
        connection_revision
    );
    assert_eq!(
        tokio::fs::read(runtime.directory.join("settings.json"))
            .await
            .unwrap(),
        disk
    );
    let usage = crate::commands::refresh_usage(&NoWindow, &runtime.state)
        .await
        .unwrap();
    assert!(
        usage.five_hour_remaining_percent.unwrap()
            <= settings.auto_pause_five_hour_threshold_percent
    );
    assert_no_pause(&server).await;
    assert!(local.calls().is_empty());
    let text = String::from_utf8(disk).unwrap();
    assert!(!text.contains("0123456789abcdef"));
    assert!(!text.contains("token"));
}

#[tokio::test]
async fn late_automatic_handshake_cannot_commit_after_native_disconnect_and_retry_is_explicit() {
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    server.fixture.hold_status.store(true, Ordering::Release);
    let state = runtime.state.clone();
    let attempt = tokio::spawn(async move {
        auto::reconcile(&NoWindow, &state, false).await;
    });
    timeout(
        Duration::from_secs(2),
        server.fixture.status_started.notified(),
    )
    .await
    .unwrap();
    runtime.state.dsh_auto.suspend(&runtime.state.dsh);
    timeout(Duration::from_secs(2), attempt)
        .await
        .unwrap()
        .unwrap();
    server.fixture.status_release.notify_one();
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert!(!runtime.state.dsh.cached_status().connected);
    assert!(runtime.state.settings.lock().await.dsh_base_url.is_empty());
    assert_eq!(runtime.state.source_generation.load(Ordering::Acquire), 0);
    runtime.state.dsh_auto.retry(&runtime.state.dsh);
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert!(runtime.state.dsh.cached_status().connected);
    assert!(runtime
        .state
        .settings
        .lock()
        .await
        .dsh_session_ids
        .is_empty());
    assert_no_pause(&server).await;
}

#[tokio::test]
async fn prepared_connection_waiting_for_commit_cannot_override_provider_change() {
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    let gate = runtime.state.settings_patch.lock().await;
    let state = runtime.state.clone();
    let attempt = tokio::spawn(async move {
        auto::reconcile(&NoWindow, &state, false).await;
    });
    timeout(
        Duration::from_secs(2),
        server.fixture.sessions_read.notified(),
    )
    .await
    .unwrap();
    runtime
        .state
        .dsh_auto
        .provider_selected(&runtime.state.dsh, crate::config::Provider::Codex);
    drop(gate);
    timeout(Duration::from_secs(2), attempt)
        .await
        .unwrap()
        .unwrap();
    assert!(runtime
        .state
        .settings
        .lock()
        .await
        .dsh_instance_id
        .is_empty());
    assert_eq!(runtime.state.settings_revision.load(Ordering::Acquire), 0);
    assert_eq!(runtime.state.source_generation.load(Ordering::Acquire), 0);
    assert!(!runtime.state.dsh.cached_status().connected);
    assert_no_pause(&server).await;
}

#[tokio::test]
async fn automatic_reconnect_and_active_account_change_clear_targets_without_resetting_source_every_scan(
) {
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    let status = runtime.state.dsh.cached_status();
    crate::commands::bind_dsh_targets(
        &NoWindow,
        &runtime.state,
        AUTO_ACCOUNT.into(),
        vec![AUTO_SESSION.into()],
        status.instance_id.clone().unwrap(),
        status.connection_revision,
    )
    .await
    .unwrap();
    assert_eq!(
        runtime.state.settings.lock().await.dsh_session_ids,
        [AUTO_SESSION]
    );
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert_eq!(
        runtime.state.settings.lock().await.dsh_session_ids,
        [AUTO_SESSION]
    );
    server.fixture.active_b.store(true, Ordering::Release);
    let generation = runtime.state.source_generation.load(Ordering::Acquire);
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    let settings = runtime.state.settings.lock().await.clone();
    assert_eq!(settings.dsh_account_id, AUTO_ACCOUNT_B);
    assert!(settings.dsh_session_ids.is_empty());
    assert_eq!(
        runtime.state.source_generation.load(Ordering::Acquire),
        generation + 1
    );
    assert!(crate::commands::bind_dsh_targets(
        &NoWindow,
        &runtime.state,
        AUTO_ACCOUNT.into(),
        vec![AUTO_SESSION.into()],
        status.instance_id.clone().unwrap(),
        status.connection_revision
    )
    .await
    .is_err());
    runtime.state.dsh_auto.retry(&runtime.state.dsh);
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert!(runtime
        .state
        .settings
        .lock()
        .await
        .dsh_session_ids
        .is_empty());
    assert!(crate::commands::bind_dsh_targets(
        &NoWindow,
        &runtime.state,
        AUTO_ACCOUNT_B.into(),
        vec![AUTO_SESSION.into()],
        status.instance_id.unwrap(),
        status.connection_revision
    )
    .await
    .is_err());
    assert_no_pause(&server).await;
}

#[tokio::test]
async fn multiple_companions_are_ambiguous_and_discovery_never_changes_other_provider() {
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    let id2 = Uuid::new_v4().to_string();
    let mut record: Value =
        serde_json::from_slice(&tokio::fs::read(&runtime.record).await.unwrap()).unwrap();
    record["instanceId"] = json!(id2);
    record["profileId"] = json!("b".repeat(64));
    tokio::fs::write(
        runtime
            .directory
            .join("dsh-bridges")
            .join(format!("{id2}.json")),
        record.to_string(),
    )
    .await
    .unwrap();
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert_eq!(
        runtime.state.dsh_auto.status(&runtime.state.dsh).auto_phase,
        "ambiguous"
    );
    assert!(runtime
        .state
        .settings
        .lock()
        .await
        .dsh_account_id
        .is_empty());
    let read_only_requests = server.fixture.requests.lock().await.len();
    assert!(read_only_requests > 0);
    assert_no_pause(&server).await;
    runtime.state.settings.lock().await.provider = crate::config::Provider::Codex;
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    assert_eq!(
        server.fixture.requests.lock().await.len(),
        read_only_requests
    );
}

#[tokio::test]
async fn actual_quota_read_is_discarded_when_provider_changes_during_rpc() {
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    let local = Arc::new(crate::pause_control::testing::ScriptedTransport::new());
    runtime
        .state
        .auto_pause
        .lock()
        .unwrap()
        .set_transport(local.clone());
    server.fixture.hold_status.store(true, Ordering::Release);
    let state = runtime.state.clone();
    let reading =
        tokio::spawn(async move { crate::commands::refresh_usage(&NoWindow, &state).await });
    timeout(
        Duration::from_secs(2),
        server.fixture.status_started.notified(),
    )
    .await
    .unwrap();
    crate::commands::apply_settings_patch(
        &NoWindow,
        &runtime.state,
        json!({"provider":"codex"}),
        false,
    )
    .await
    .unwrap();
    server.fixture.status_release.notify_one();
    let error = timeout(Duration::from_secs(2), reading)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("source changed"));
    assert!(runtime.state.last_usage.lock().await.is_none());
    assert_no_pause(&server).await;
    assert!(local.calls().is_empty());
}

#[tokio::test]
async fn restarted_companion_selects_the_same_profile_but_never_inherits_target_authorization() {
    let server = Server::start(Behavior::AutoBridge).await;
    let runtime = AutoRuntime::new(&server).await;
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    let old = runtime.state.dsh.cached_status();
    crate::commands::bind_dsh_targets(
        &NoWindow,
        &runtime.state,
        AUTO_ACCOUNT.into(),
        vec![AUTO_SESSION.into()],
        old.instance_id.clone().unwrap(),
        old.connection_revision,
    )
    .await
    .unwrap();
    let mut record: Value =
        serde_json::from_slice(&tokio::fs::read(&runtime.record).await.unwrap()).unwrap();
    let new_id = Uuid::new_v4().to_string();
    record["instanceId"] = json!(new_id);
    let new_path = runtime
        .directory
        .join("dsh-bridges")
        .join(format!("{new_id}.json"));
    tokio::fs::write(new_path, record.to_string())
        .await
        .unwrap();
    // A crashed Host may leave a same-profile discovery file behind. Keep that
    // file, but point it at a closed listener so only the replacement is live.
    let stopped = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stopped_origin = format!("http://{}", stopped.local_addr().unwrap());
    drop(stopped);
    record["instanceId"] = json!(runtime.id);
    record["origin"] = json!(stopped_origin);
    tokio::fs::write(&runtime.record, record.to_string())
        .await
        .unwrap();
    auto::reconcile(&NoWindow, &runtime.state, false).await;
    let settings = runtime.state.settings.lock().await.clone();
    assert_eq!(settings.dsh_instance_id, new_id);
    assert_eq!(settings.dsh_profile_id, "a".repeat(64));
    assert!(settings.dsh_session_ids.is_empty());
    assert!(crate::commands::bind_dsh_targets(
        &NoWindow,
        &runtime.state,
        AUTO_ACCOUNT.into(),
        vec![AUTO_SESSION.into()],
        old.instance_id.unwrap(),
        old.connection_revision
    )
    .await
    .is_err());
    assert_no_pause(&server).await;
}
