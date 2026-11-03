//! Native, memory-only connection to the public DSH gateway.
//!
//! Automatic attachment uses a dedicated companion discovery record and the
//! official Desktop CLI for bootstrap; manual launch-link pairing remains a
//! compatibility option. No private DSH authentication files, ChatGPT OAuth,
//! local `.codex` files, or Codex named pipes are read. Manual pairing cookies
//! last only as long as their Mux WebSocket. Companion bearer credentials stay
//! native/in memory and are never persisted in overlay settings or sent to UI.
//! Public profile identity selects a reconnect; instance/account identify data.
//! Automatic attachment reads the plugin's active account but authorizes no
//! pause sessions. Quota reads require the selected *active* account. Status-before
//! and status-after are consistency checks, not an atomic host snapshot. The
//! public session API cannot prove a running turn's Codex account: target binding
//! is the user's declaration, verified only against the current active account
//! and the running session's last-used provider. A session may finish between
//! list and steer; the public host API can then cold-start it. This is a
//! cooperative pause message, never a hard gate or confirmed execution stop.
pub(crate) mod auto;
mod bridge;
mod parser;
pub(crate) mod setup;
mod url;

use crate::codex::RawCodexUsage;
use futures_util::{SinkExt, StreamExt};
use reqwest::{
    header::{HeaderValue, COOKIE, LOCATION, SET_COOKIE},
    Client, Url,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpStream, sync::Mutex, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    client_async_tls_with_config,
    tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Message},
};
use uuid::Uuid;

const HTTP_BUDGET: Duration = Duration::from_secs(5);
const MAX_RESPONSE: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub label: String,
    pub active: bool,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub busy: bool,
}
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub connected: bool,
    pub instance_id: Option<String>,
    pub origin: Option<String>,
    pub active_account_id: Option<String>,
    pub connection_revision: u64,
    pub accounts: Vec<Account>,
    pub sessions: Vec<Session>,
    pub error: Option<String>,
    pub auto_phase: String,
    pub auto_message: Option<String>,
    pub auto_revision: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPauseResult {
    pub id: String,
    pub accepted: bool,
    pub method: String,
    pub message: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseResult {
    pub accepted: bool,
    pub stopped: bool,
    pub sessions: Vec<SessionPauseResult>,
}

/// Clones share one serialized connection and never duplicate its secret cookie.
#[derive(Clone, Default)]
pub struct Manager {
    inner: Arc<Mutex<State>>,
    epoch: Arc<AtomicU64>,
    revision: Arc<AtomicU64>,
    live: Arc<StdMutex<Option<Arc<AtomicBool>>>>,
    cached: Arc<StdMutex<ConnectionStatus>>,
}

/// A successful handshake which has not changed the active source or connection.
/// Dropping a cancelled/obsolete preparation also drops its private credentials.
pub(crate) struct PreparedConnection {
    connection: Connection,
    status: ConnectionStatus,
    epoch: u64,
}
impl PreparedConnection {
    pub(crate) fn status(&self) -> &ConnectionStatus {
        &self.status
    }
}

/// Acquired before the caller's short, synchronous cancellation/commit gate.
/// No discovery, HTTP request or filesystem await is performed while held.
pub(crate) struct Installation<'a> {
    manager: &'a Manager,
    state: tokio::sync::MutexGuard<'a, State>,
}
impl Installation<'_> {
    pub(crate) fn accepts(&self, prepared: &PreparedConnection) -> bool {
        self.manager.epoch.load(Ordering::Acquire) == prepared.epoch
            && prepared.connection.alive.load(Ordering::Acquire)
    }
    pub(crate) fn install(&mut self, prepared: PreparedConnection) -> ConnectionStatus {
        // The owner must keep its cancellation gate through accepts/save/install.
        self.manager.epoch.fetch_add(1, Ordering::AcqRel);
        let revision = self.manager.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let mut status = prepared.status;
        status.connection_revision = revision;
        *self.manager.live.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(prepared.connection.alive.clone());
        self.state.connection = Some(prepared.connection);
        self.state.last_usage_ms.clear();
        self.state.error = None;
        *self
            .manager
            .cached
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = status.clone();
        status
    }
}
#[derive(Default)]
struct State {
    connection: Option<Connection>,
    error: Option<String>,
    last_usage_ms: std::collections::HashMap<String, i64>,
}
struct Connection {
    client: Client,
    origin: Url,
    auth: Auth,
    instance_id: String,
    alive: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
enum Auth {
    Cookie(HeaderValue),
    Bridge {
        bearer: HeaderValue,
        discovery_path: std::path::PathBuf,
        discovery_directory: std::path::PathBuf,
    },
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

impl Manager {
    pub(crate) fn revoke(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let live = self.live.lock().unwrap_or_else(|p| p.into_inner()).take();
        if live.is_some_and(|alive| alive.swap(false, Ordering::AcqRel)) {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        *self.cached.lock().unwrap_or_else(|p| p.into_inner()) = ConnectionStatus {
            connection_revision: self.revision.load(Ordering::Acquire),
            ..Default::default()
        };
    }

    pub(crate) async fn preparation(
        &self,
        connection_url: &str,
    ) -> Result<PreparedConnection, String> {
        let epoch = self.epoch.load(Ordering::Acquire);
        let pairing = url::pairing_url(connection_url)?;
        let connection = timeout(Duration::from_secs(15), establish(pairing))
            .await
            .map_err(|_| "DSH pairing timed out".to_owned())??;
        let status = connection_status(&connection).await;
        if !status.connected {
            return Err("DSH connection closed during pairing".into());
        }
        Ok(PreparedConnection {
            connection,
            status,
            epoch,
        })
    }

    pub(crate) async fn prepare_bridge_in_directory(
        &self,
        path: &std::path::Path,
        directory: &std::path::Path,
    ) -> Result<PreparedConnection, String> {
        let epoch = self.epoch.load(Ordering::Acquire);
        let record = bridge::read_record_in_directory(path, directory).await?;
        let connection = bridge_connection(record)?;
        let status = connection_status(&connection).await;
        if !status.connected {
            return Err("DSH companion is not available".into());
        }
        Ok(PreparedConnection {
            connection,
            status,
            epoch,
        })
    }

    pub(crate) async fn installation(&self) -> Installation<'_> {
        Installation {
            manager: self,
            state: self.inner.lock().await,
        }
    }

    #[cfg(test)]
    pub async fn connect(&self, connection_url: &str) -> Result<ConnectionStatus, String> {
        self.revoke();
        let prepared = self.preparation(connection_url).await?;
        let mut installation = self.installation().await;
        if !installation.accepts(&prepared) {
            return Err("DSH pairing was cancelled".into());
        }
        Ok(installation.install(prepared))
    }

    #[cfg(test)]
    pub async fn disconnect(&self) {
        self.revoke();
        self.clear_revoked().await;
    }

    pub(crate) async fn clear_revoked(&self) {
        let mut state = self.inner.lock().await;
        if state
            .connection
            .as_ref()
            .is_some_and(|c| !c.alive.load(Ordering::Acquire))
        {
            state.connection.take();
            state.last_usage_ms.clear();
            state.error = None;
        }
    }

    pub(crate) fn cached_status(&self) -> ConnectionStatus {
        let mut status = self
            .cached
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let alive = self
            .live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|alive| alive.load(Ordering::Acquire));
        if !alive {
            status.connected = false;
            status.accounts.clear();
            status.sessions.clear();
            status.active_account_id = None;
        }
        status
    }

    /// Only the origin is exposed; never return the connect URL or cookie.
    pub async fn origin(&self) -> Option<String> {
        self.inner
            .lock()
            .await
            .connection
            .as_ref()
            .map(|c| c.origin.origin().ascii_serialization())
    }

    pub async fn status(&self) -> ConnectionStatus {
        let mut state = self.inner.lock().await;
        let mut status = status_locked(&mut state).await;
        status.connection_revision = self.revision.load(Ordering::Acquire);
        *self.cached.lock().unwrap_or_else(|p| p.into_inner()) = status.clone();
        status
    }

    pub async fn read_usage(&self, account_id: &str) -> Result<RawCodexUsage, String> {
        let mut state = self.inner.lock().await;
        let connection = live_connection(&state)?;
        let started_ms = now_ms();
        let previous_ms = state.last_usage_ms.get(account_id).copied();
        let (raw, fetched_ms) = timeout(Duration::from_secs(20), async {
            let before = parser::accounts(
                &connection
                    .rpc("codex-subscription/status", json!({}))
                    .await?,
            )?;
            parser::active_account(&before, account_id)?;
            let reading = connection
                .rpc("codex-subscription/usage/accounts", json!({"force":true}))
                .await?;
            let after = parser::accounts(
                &connection
                    .rpc("codex-subscription/status", json!({}))
                    .await?,
            )?;
            parser::consistent_usage_identity(&before, &reading, &after, account_id)?;
            parser::fresh_usage(&reading, account_id, now_ms(), started_ms, previous_ms)
        })
        .await
        .map_err(|_| "DSH quota read timed out".to_owned())??;
        state
            .last_usage_ms
            .insert(account_id.to_owned(), fetched_ms);
        Ok(raw)
    }

    /// Sends a normal public session collaboration request only. Acceptance is
    /// not proof that the model obeyed the request or that execution stopped.
    pub async fn pause(
        &self,
        account_id: &str,
        session_ids: &[String],
        prompt: &str,
    ) -> Result<PauseResult, String> {
        if session_ids.is_empty()
            || session_ids.len() > 32
            || prompt.trim().is_empty()
            || prompt.len() > 4096
        {
            return Err("Select existing DSH sessions and a bounded pause message".into());
        }
        let mut unique = std::collections::HashSet::new();
        if session_ids.iter().any(|id| {
            id.is_empty()
                || id.len() > 256
                || id.chars().any(char::is_control)
                || !unique.insert(id)
        }) {
            return Err("Invalid or duplicate DSH session targets".into());
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let state = tokio::time::timeout_at(deadline, self.inner.lock())
            .await
            .map_err(|_| {
                "DSH pause timed out waiting for the connection; no request was sent".to_owned()
            })?;
        let connection = live_connection(&state)?;
        pause_sessions(connection, account_id, session_ids, prompt, deadline).await
    }
}

fn bridge_connection(record: bridge::Record) -> Result<Connection, String> {
    Ok(Connection {
        client: http_client()?,
        origin: record.origin,
        instance_id: record.instance_id,
        auth: Auth::Bridge {
            bearer: record.bearer,
            discovery_path: record.path,
            discovery_directory: record.directory,
        },
        alive: Arc::new(AtomicBool::new(true)),
        worker: None,
    })
}

#[cfg(test)]
async fn install_bridge(
    state: &mut State,
    record: bridge::Record,
) -> Result<ConnectionStatus, String> {
    state.connection.take();
    state.last_usage_ms.clear();
    state.error = None;
    state.connection = Some(bridge_connection(record)?);
    let status = status_locked(state).await;
    if !status.connected {
        state.connection.take();
        return Err(status
            .error
            .unwrap_or_else(|| "DSH companion is not available".into()));
    }
    Ok(status)
}

fn live_connection(state: &State) -> Result<&Connection, String> {
    state
        .connection
        .as_ref()
        .filter(|connection| connection.alive.load(Ordering::Acquire))
        .ok_or_else(|| "DSH is disconnected; paste a fresh DSH connection link".into())
}

async fn status_locked(state: &mut State) -> ConnectionStatus {
    let Some(connection) = state
        .connection
        .as_ref()
        .filter(|c| c.alive.load(Ordering::Acquire))
    else {
        state.connection.take();
        return ConnectionStatus {
            error: state.error.clone(),
            ..Default::default()
        };
    };
    connection_status(connection).await
}

async fn connection_status(connection: &Connection) -> ConnectionStatus {
    let mut result = ConnectionStatus {
        connected: connection.alive.load(Ordering::Acquire),
        instance_id: Some(connection.instance_id.clone()),
        origin: Some(connection.origin.origin().ascii_serialization()),
        ..Default::default()
    };
    match connection
        .rpc("codex-subscription/status", json!({}))
        .await
        .and_then(|value| parser::accounts(&value))
    {
        Ok(accounts) => result.accounts = accounts,
        Err(error) => result.error = Some(error),
    }
    match connection
        .rpc("session/list", json!({"args":{"request":{}}}))
        .await
        .and_then(|value| parser::sessions(&value))
    {
        Ok(sessions) => result.sessions = sessions,
        Err(error) => {
            if result.error.is_none() {
                result.error = Some(error);
            }
        }
    }
    if !connection.alive.load(Ordering::Acquire) {
        result.connected = false;
        result.accounts.clear();
        result.sessions.clear();
    }
    result.active_account_id = result
        .accounts
        .iter()
        .find(|a| a.active)
        .map(|a| a.id.clone());
    result
}

fn http_client() -> Result<Client, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve(
            "localhost",
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        )
        .timeout(HTTP_BUDGET)
        .build()
        .map_err(|_| "Could not initialize the DSH HTTP client".into())
}

async fn establish(pairing: url::PairingUrl) -> Result<Connection, String> {
    let client = http_client()?;
    // Never format reqwest errors: they can include the complete secret URL.
    let response = client
        .get(pairing.connect.clone())
        .send()
        .await
        .map_err(|_| "DSH pairing transport failed")?;
    if response.status() != reqwest::StatusCode::FOUND {
        return Err("DSH rejected the launch link; obtain a new connection link".into());
    }
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|h| h.to_str().ok())
        .ok_or("Missing DSH pairing redirect")?;
    let mux_url = url::mux_redirect(&pairing.connect, location)?;
    let cookie = pairing_cookie(response.headers())?;
    let mut request = mux_url
        .as_str()
        .into_client_request()
        .map_err(|_| "Invalid DSH WebSocket handshake")?;
    request.headers_mut().insert(COOKIE, cookie.clone());
    request.headers_mut().insert(
        "origin",
        HeaderValue::from_str(&pairing.origin.origin().ascii_serialization())
            .map_err(|_| "Invalid DSH origin")?,
    );
    // Resolve localhost ourselves so even a changed DNS record cannot send a
    // pairing cookie outside loopback. TLS still verifies the original host.
    let tcp = TcpStream::connect(url::socket_address(&mux_url)?)
        .await
        .map_err(|_| "DSH WebSocket connection failed")?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(256 * 1024))
        .max_frame_size(Some(256 * 1024));
    let (mut ws, _) = client_async_tls_with_config(request, tcp, Some(config), None)
        .await
        .map_err(|_| "DSH WebSocket authentication failed")?;
    let alive = Arc::new(AtomicBool::new(true));
    let worker_alive = alive.clone();
    let worker = tokio::spawn(async move {
        // The gateway permits an empty Mux stream set. No fabricated hello,
        // token, RPC frame or core hook is sent. Reading and flushing handles
        // WebSocket control Ping/Pong and owns the paired peer's lifetime.
        while let Some(message) = ws.next().await {
            match message {
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(Message::Ping(_)) => {
                    if ws.flush().await.is_err() {
                        break;
                    }
                }
                Ok(_) => {}
            }
        }
        worker_alive.store(false, Ordering::Release);
    });
    Ok(Connection {
        client,
        origin: pairing.origin,
        auth: Auth::Cookie(cookie),
        instance_id: Uuid::new_v4().to_string(),
        alive,
        worker: Some(worker),
    })
}

fn pairing_cookie(headers: &reqwest::header::HeaderMap) -> Result<HeaderValue, String> {
    let mut cookies = Vec::new();
    for raw in headers.get_all(SET_COOKIE) {
        let raw = raw.to_str().map_err(|_| "Malformed DSH pairing cookie")?;
        let pair = raw
            .split(';')
            .next()
            .ok_or("Malformed DSH pairing cookie")?
            .trim();
        let (name, value) = pair.split_once('=').ok_or("Malformed DSH pairing cookie")?;
        if name.is_empty()
            || value.is_empty()
            || pair.len() > 8192
            || name
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && b != b'_' && b != b'-')
        {
            return Err("Malformed DSH pairing cookie".into());
        }
        cookies.push(pair);
        if cookies.len() > 8 {
            return Err("Too many DSH pairing cookies".into());
        }
    }
    if cookies.is_empty() {
        return Err("DSH did not issue a pairing cookie".into());
    }
    let joined = cookies.join("; ");
    if joined.len() > 16_384 {
        return Err("DSH pairing cookie is too large".into());
    }
    let mut header = HeaderValue::from_str(&joined).map_err(|_| "Malformed DSH pairing cookie")?;
    header.set_sensitive(true);
    Ok(header)
}

impl Connection {
    async fn rpc(&self, method: &str, payload: Value) -> Result<Value, String> {
        if !self.alive.load(Ordering::Acquire) {
            return Err("DSH gateway connection has closed; pair again".into());
        }
        if !matches!(
            method,
            "codex-subscription/status"
                | "codex-subscription/usage/accounts"
                | "session/list"
                | "session/prompt"
        ) {
            return Err("DSH RPC method is outside the integration allowlist".into());
        }
        let rpc_id = Uuid::new_v4().to_string();
        let endpoint = self
            .origin
            .join(&format!("api/{method}"))
            .map_err(|_| "Invalid DSH RPC endpoint")?;
        let request = self.client.post(endpoint);
        let request = match &self.auth {
            Auth::Cookie(cookie) => request.header(COOKIE, cookie.clone()),
            Auth::Bridge {
                bearer,
                discovery_path,
                discovery_directory,
            } => {
                // Deletion/rotation/profile replacement invalidates the binding
                // even if another HTTP service later reuses the same port.
                let current =
                    bridge::read_record_in_directory(discovery_path, discovery_directory).await;
                if !current.is_ok_and(|current| {
                    current.instance_id == self.instance_id
                        && current.origin == self.origin
                        && current.bearer == *bearer
                }) {
                    self.alive.store(false, Ordering::Release);
                    return Err(
                        "DSH companion discovery identity changed or disappeared; reconnect".into(),
                    );
                }
                request.header(reqwest::header::AUTHORIZATION, bearer.clone())
            }
        };
        let response = request
            .json(
                &json!({"type":"client-request","rpcId":rpc_id,"method":method,"payload":payload}),
            )
            .send()
            .await
            .map_err(|_| {
                if matches!(&self.auth, Auth::Bridge { .. }) {
                    self.alive.store(false, Ordering::Release);
                }
                "DSH RPC transport failed".to_owned()
            })?;
        if matches!(response.status().as_u16(), 401 | 403) {
            self.alive.store(false, Ordering::Release);
            return Err("DSH authentication expired; paste a fresh connection link".into());
        }
        if !response.status().is_success() {
            return Err(format!(
                "DSH RPC failed (HTTP {})",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE as u64)
        {
            return Err("DSH RPC response exceeds the size limit".into());
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Could not read DSH RPC response")?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
                return Err("DSH RPC response exceeds the size limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !self.alive.load(Ordering::Acquire) {
            return Err("DSH connection closed during the request".into());
        }
        let value =
            serde_json::from_slice(&bytes).map_err(|_| "Malformed DSH RPC JSON response")?;
        parser::rpc_response(value, &rpc_id)
    }
}

async fn pause_sessions(
    connection: &Connection,
    account_id: &str,
    session_ids: &[String],
    prompt: &str,
    deadline: tokio::time::Instant,
) -> Result<PauseResult, String> {
    let mut results = Vec::with_capacity(session_ids.len());
    let mut exhausted = false;
    for id in session_ids {
        let attempt = async {
            let before = parser::accounts(
                &connection
                    .rpc("codex-subscription/status", json!({}))
                    .await?,
            )?;
            parser::active_account(&before, account_id)?;
            let listed = connection
                .rpc("session/list", json!({"args":{"request":{}}}))
                .await?;
            parser::pause_target(&listed, id)?;
            let after = parser::accounts(
                &connection
                    .rpc("codex-subscription/status", json!({}))
                    .await?,
            )?;
            parser::consistent_identity(&before, &after, account_id)?;
            let result = connection
                .rpc(
                    "session/prompt",
                    json!({"args":{"request":{
                        "requestId":Uuid::new_v4().to_string(),"sessionId":id,"mode":"steer",
                        "content":[{"type":"text","text":prompt}]
                    }}}),
                )
                .await?;
            if result["accepted"] != true {
                return Err("DSH did not accept the session collaboration request".to_owned());
            }
            Ok(())
        };
        let result = if exhausted {
            Err("DSH pause budget exhausted; this target was not requested".to_owned())
        } else {
            match tokio::time::timeout_at(deadline, attempt).await {
                Ok(result) => result,
                Err(_) => {
                    exhausted = true;
                    Err(
                        "DSH pause timed out; request acceptance and stopping are not confirmed"
                            .to_owned(),
                    )
                }
            }
        };
        let accepted = result.is_ok();
        results.push(SessionPauseResult {
            id: id.clone(),
            accepted,
            method: "session/prompt:steer".into(),
            message: if accepted {
                "Pause collaboration request accepted; stopping is not confirmed"
            } else {
                "Pause collaboration request was not confirmed"
            }
            .into(),
            error: result.err(),
        });
    }
    // accepted means at least one target accepted; per-target receipts preserve
    // partial success and prevent claiming a global stop from an RPC receipt.
    Ok(PauseResult {
        accepted: results.iter().any(|result| result.accepted),
        stopped: false,
        sessions: results,
    })
}

#[cfg(test)]
mod transport_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookie_is_secret_and_missing_cookie_fails_closed() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert!(pairing_cookie(&headers).is_err());
        headers.insert(
            SET_COOKIE,
            HeaderValue::from_static("peer=SECRET; HttpOnly; SameSite=Strict; Path=/"),
        );
        let cookie = pairing_cookie(&headers).unwrap();
        assert!(cookie.is_sensitive());
        assert_eq!(cookie.to_str().unwrap(), "peer=SECRET");
        assert!(!format!("{cookie:?}").contains("SECRET"));
    }
    #[tokio::test]
    async fn disconnected_manager_never_falls_back_to_local_codex() {
        let manager = Manager::default();
        assert!(!manager.status().await.connected);
        assert!(manager.origin().await.is_none());
        assert!(manager.read_usage("a").await.is_err());
        assert!(manager
            .pause("a", &["existing".into()], "暂停")
            .await
            .is_err());
        manager.disconnect().await;
    }
}
