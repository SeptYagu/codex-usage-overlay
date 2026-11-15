use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

/// The current Codex desktop client bridge family.
pub const APP_TOOLS_PIPE_PREFIX: &str = "codex-app-tools-";
/// The older Codex desktop client bridge family.
pub const LEGACY_BROWSER_USE_PIPE_PREFIX: &str = "codex-browser-use-";

/// The environment variable that pins an explicit bridge pipe.
const EXPLICIT_PIPE_ENV: &str = "CODEX_APP_TOOLS_PIPE_PATH";
/// The environment variable that names the calling Codex thread.
const CALLER_THREAD_ENV: &str = "CODEX_THREAD_ID";

/// A JSON-RPC response larger than this is rejected without reading it.
#[cfg(windows)]
const MAX_RESPONSE_BYTES: u32 = 8 * 1024 * 1024;
/// The delay between retries while the pipe reports that it is busy.
#[cfg(windows)]
const PIPE_BUSY_RETRY: Duration = Duration::from_millis(40);
/// How often a busy pipe is retried before the candidate is abandoned.
#[cfg(windows)]
const PIPE_BUSY_MAX_RETRIES: u32 = 8;
/// The sub-deadline of a single bridge call.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
/// The maximum directory depth scanned under `.codex/sessions`.
const MAX_SESSION_SCAN_DEPTH: usize = 4;
/// How many recent session files are inspected for a caller thread identity.
const MAX_SESSION_FILES: usize = 32;
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// A Codex session that may receive a pause request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveThread {
    pub id: String,
    pub host_id: Option<String>,
    pub turn_id: Option<String>,
}

/// Collects the eligible targets of one listing.
fn collect_active_threads(
    items: &[Value],
    threads: &mut Vec<ActiveThread>,
    seen: &mut HashSet<(Option<String>, String)>,
) {
    for item in items {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if item.get("kind").and_then(Value::as_str) != Some("codex") {
            continue;
        }
        if item.get("status").and_then(Value::as_str) != Some("active") {
            continue;
        }
        let host_id = item
            .get("hostId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(ToOwned::to_owned);
        let turn_id = item
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|turn| !turn.is_empty())
            .map(ToOwned::to_owned);
        if !seen.insert((host_id.clone(), id.to_owned())) {
            continue;
        }
        threads.push(ActiveThread {
            id: id.to_owned(),
            host_id,
            turn_id,
        });
    }
}

/// Parses the `list_threads` payload into the sessions that may be targeted.
///
/// Only `kind == "codex"` and `status == "active"` records with a non-empty id
/// survive, and the `pinnedThreads` and `threads` groups are merged and
/// de-duplicated by `(hostId, id)`.
pub fn parse_active_threads(text: &str) -> Result<Vec<ActiveThread>, BridgeError> {
    let value = match serde_json::from_str::<Value>(text) {
        Ok(Value::String(inner)) => serde_json::from_str::<Value>(&inner)
            .map_err(|_| BridgeError::Protocol("invalid thread listing JSON".into()))?,
        Ok(value) => value,
        Err(_) => return Err(BridgeError::Protocol("invalid thread listing JSON".into())),
    };

    let mut threads = Vec::new();
    let mut seen: HashSet<(Option<String>, String)> = HashSet::new();
    match &value {
        Value::Array(items) => collect_active_threads(items, &mut threads, &mut seen),
        Value::Object(_) => {
            let mut has_group = false;
            for key in ["pinnedThreads", "threads"] {
                if let Some(items) = value.get(key) {
                    let items = items.as_array().ok_or_else(|| {
                        BridgeError::Protocol(format!("invalid thread listing group: {key}"))
                    })?;
                    has_group = true;
                    collect_active_threads(items, &mut threads, &mut seen);
                }
            }
            if !has_group {
                return Err(BridgeError::Protocol(
                    "thread listing carries no thread groups".into(),
                ));
            }
        }
        _ => return Err(BridgeError::Protocol("invalid thread listing shape".into())),
    }
    Ok(threads)
}

/// Strictly checks the 8-4-4-4-12 UUID shape used by local Codex thread identities.
pub fn looks_like_thread_id(value: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != GROUPS.len() {
        return false;
    }
    parts.iter().zip(GROUPS).all(|(part, expected)| {
        part.len() == expected && part.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

/// Whether an enumerated pipe name belongs to a supported bridge family.
pub fn is_supported_pipe_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    for prefix in [APP_TOOLS_PIPE_PREFIX, LEGACY_BROWSER_USE_PIPE_PREFIX] {
        if let Some(suffix) = lowered.strip_prefix(prefix) {
            if looks_like_thread_id(suffix) {
                return true;
            }
        }
    }
    false
}

/// Builds the deterministic candidate pipe order.
pub fn ordered_candidate_pipe_paths(explicit: Option<&str>, names: &[String]) -> Vec<String> {
    let mut ordered: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    fn push(value: &str, ordered: &mut Vec<String>, seen: &mut HashSet<String>) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }
        if seen.insert(trimmed.to_ascii_lowercase()) {
            ordered.push(trimmed.to_owned());
        }
    }

    if let Some(explicit) = explicit {
        push(explicit, &mut ordered, &mut seen);
    }

    for prefix in [APP_TOOLS_PIPE_PREFIX, LEGACY_BROWSER_USE_PIPE_PREFIX] {
        let mut family: Vec<&String> = names
            .iter()
            .filter(|name| {
                name.to_ascii_lowercase().starts_with(prefix) && is_supported_pipe_name(name)
            })
            .collect();
        family.sort_by_key(|left| left.to_ascii_lowercase());
        for name in family {
            push(name, &mut ordered, &mut seen);
        }
    }

    ordered
}

/// One pipe plus the caller identity presented to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCandidate {
    pub pipe: String,
    pub caller_thread_id: String,
}

/// Crosses the ordered pipes with the ordered caller identities.
pub fn build_candidates(
    explicit: Option<&str>,
    pipe_names: &[String],
    thread_ids: &[String],
) -> Vec<BridgeCandidate> {
    let mut candidates = Vec::new();
    for pipe in ordered_candidate_pipe_paths(explicit, pipe_names) {
        for thread_id in thread_ids {
            candidates.push(BridgeCandidate {
                pipe: pipe.clone(),
                caller_thread_id: thread_id.clone(),
            });
        }
    }
    candidates
}

/// The local Codex thread identities that may act as the caller.
pub fn candidate_caller_thread_ids() -> Vec<String> {
    let mut identities: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    if let Ok(raw) = std::env::var(CALLER_THREAD_ENV) {
        let trimmed = raw.trim();
        if looks_like_thread_id(trimmed) && seen.insert(trimmed.to_ascii_lowercase()) {
            identities.push(trimmed.to_owned());
        }
    }

    for identity in session_file_thread_ids() {
        if seen.insert(identity.to_ascii_lowercase()) {
            identities.push(identity);
        }
    }
    identities
}

/// The thread identities found in the most recent local session files.
fn session_file_thread_ids() -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let root = home.join(".codex").join("sessions");
    if !root.is_dir() {
        return Vec::new();
    }

    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    collect_session_files(
        &root,
        0,
        MAX_SESSION_SCAN_DEPTH,
        MAX_SESSION_FILES,
        &mut files,
    );
    files.sort_by(|left, right| right.0.cmp(&left.0));

    let mut identities = Vec::new();
    for (_, path) in files.into_iter().take(MAX_SESSION_FILES) {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(identity) = extract_trailing_uuid(name) {
            identities.push(identity);
        }
    }
    identities
}

/// Collects the `.jsonl` session files under `dir`, bounded by `max_depth`.
fn collect_session_files(
    dir: &std::path::Path,
    depth: usize,
    max_depth: usize,
    limit: usize,
    out: &mut Vec<(std::time::SystemTime, std::path::PathBuf)>,
) {
    if depth > max_depth || out.len() >= limit {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    paths.sort_by(|left, right| right.cmp(left));

    for path in paths {
        if out.len() >= limit {
            return;
        }
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            collect_session_files(&path, depth + 1, max_depth, limit, out);
        } else if metadata.is_file() && is_session_file(&path) {
            let modified = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            out.push((modified, path));
        }
    }
}

/// Whether a file is a Codex session transcript.
fn is_session_file(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lowered = name.to_ascii_lowercase();
    lowered.ends_with(".jsonl") || lowered.ends_with(".jsonl.zst")
}

/// Extracts the 36 character UUID that a session file name ends with.
fn extract_trailing_uuid(name: &str) -> Option<String> {
    let mut candidate = name;
    loop {
        if let Some(uuid) = trailing_uuid(candidate) {
            return Some(uuid);
        }
        match candidate.rfind('.') {
            Some(index) if index > 0 => candidate = &candidate[..index],
            _ => return None,
        }
    }
}

/// The trailing 36 characters of a value, when they form a UUID.
fn trailing_uuid(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() < 36 {
        return None;
    }
    let start = bytes.len() - 36;
    if !value.is_char_boundary(start) {
        return None;
    }
    let tail = &value[start..];
    looks_like_thread_id(tail).then(|| tail.to_owned())
}

/// How the candidate list is produced.
#[derive(Clone, Default)]
pub struct BridgeDiscovery {
    pub explicit_pipe_path: Option<String>,
    pub pipe_names: Vec<String>,
    pub caller_thread_ids: Vec<String>,
}

impl BridgeDiscovery {
    /// Reads the live environment and the machine's named pipes.
    pub fn discover() -> Self {
        Self {
            explicit_pipe_path: std::env::var(EXPLICIT_PIPE_ENV)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            pipe_names: enumerate_pipe_names(),
            caller_thread_ids: candidate_caller_thread_ids(),
        }
    }
}

/// Discovers the bridge on a blocking thread.
pub(super) async fn discover_bridge() -> BridgeDiscovery {
    tokio::task::spawn_blocking(BridgeDiscovery::discover).await.unwrap_or_default()
}

/// The outcome of one `tools/call` against a bridge pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallOutcome {
    pub success: bool,
    pub text: Option<String>,
}

/// Why a bridge call did not produce a usable result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    Unavailable(String),
    Protocol(String),
}

/// The transport seam for bridge RPC calls.
pub trait BridgeTransport: Send + Sync {
    fn call_tool_observed<'a>(
        &'a self,
        pipe: &'a str,
        caller: &'a str,
        tool: &'a str,
        arguments: Value,
        started: &'a AtomicBool,
        may_submit: &'a (dyn Fn() -> bool + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>> {
        // Unknown transports conservatively treat the invocation as possibly submitted.
        if !may_submit() {
            return Box::pin(async {
                Err(BridgeError::Unavailable(
                    "cancelled before submission".into(),
                ))
            });
        }
        started.store(true, Ordering::Release);
        self.call_tool(pipe, caller, tool, arguments)
    }

    fn call_tool<'a>(
        &'a self,
        pipe: &'a str,
        caller_thread_id: &'a str,
        tool_name: &'a str,
        arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>>;
}

/// The production transport: JSON-RPC `tools/call` over a Windows named pipe.
#[cfg(windows)]
pub struct NamedPipeTransport;

#[cfg(windows)]
impl BridgeTransport for NamedPipeTransport {
    fn call_tool_observed<'a>(
        &'a self,
        pipe: &'a str,
        caller: &'a str,
        tool: &'a str,
        arguments: Value,
        started: &'a AtomicBool,
        may_submit: &'a (dyn Fn() -> bool + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>> {
        Box::pin(async move {
            if !may_submit() {
                return Err(BridgeError::Unavailable(
                    "cancelled before submission".into(),
                ));
            }
            let request = build_request(tool, arguments, caller);
            let body =
                serde_json::to_vec(&request).map_err(|e| BridgeError::Protocol(e.to_string()))?;
            let mut client = open_pipe(pipe).await?;
            if !may_submit() {
                return Err(BridgeError::Unavailable(
                    "cancelled before submission".into(),
                ));
            }
            started.store(true, Ordering::Release);
            write_frame(&mut client, &body).await?;
            parse_matching_tool_outcome(&read_frame(&mut client).await?, &request["id"])
        })
    }

    fn call_tool<'a>(
        &'a self,
        pipe: &'a str,
        caller_thread_id: &'a str,
        tool_name: &'a str,
        arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>> {
        Box::pin(async move {
            let request = build_request(tool_name, arguments, caller_thread_id);
            let body = serde_json::to_vec(&request)
                .map_err(|error| BridgeError::Protocol(error.to_string()))?;
            let mut client = open_pipe(pipe).await?;
            write_frame(&mut client, &body).await?;
            let payload = read_frame(&mut client).await?;
            parse_tool_outcome(&payload)
        })
    }
}

/// The transport used on platforms without the Codex desktop bridge.
#[cfg(not(windows))]
pub struct UnsupportedTransport;

#[cfg(not(windows))]
impl BridgeTransport for UnsupportedTransport {
    fn call_tool<'a>(
        &'a self,
        _pipe: &'a str,
        _caller_thread_id: &'a str,
        _tool_name: &'a str,
        _arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>> {
        Box::pin(async {
            Err(BridgeError::Unavailable(
                "the Codex desktop bridge is only available on Windows".into(),
            ))
        })
    }
}

/// The default transport for the current platform.
pub(super) fn default_transport() -> std::sync::Arc<dyn BridgeTransport> {
    #[cfg(windows)]
    {
        std::sync::Arc::new(NamedPipeTransport)
    }
    #[cfg(not(windows))]
    {
        std::sync::Arc::new(UnsupportedTransport)
    }
}

/// Builds the JSON-RPC envelope for one `tools/call`.
fn build_request(tool_name: &str, arguments: Value, caller_thread_id: &str) -> Value {
    let sequence = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    json!({
        "jsonrpc": "2.0",
        "id": sequence,
        "method": "tools/call",
        "params": {
            "arguments": arguments,
            "callerSource": "codex",
            "callId": format!("auto-pause-{sequence}"),
            "namespace": "codex_app",
            "threadId": caller_thread_id,
            "tool": tool_name,
            "turnId": format!("auto-pause-turn-{sequence}"),
        }
    })
}

/// Parses the JSON-RPC envelope into a tool outcome.
fn parse_tool_outcome(payload: &[u8]) -> Result<ToolCallOutcome, BridgeError> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| BridgeError::Protocol(format!("malformed response: {error}")))?;
    if let Some(error) = value.get("error") {
        return Err(BridgeError::Protocol(format!(
            "bridge returned an error: {error}"
        )));
    }
    let Some(result) = value.get("result") else {
        return Err(BridgeError::Protocol(
            "bridge response carries no result".into(),
        ));
    };
    let success = result
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text = result
        .get("contentItems")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Ok(ToolCallOutcome { success, text })
}

#[cfg(any(windows, test))]
fn parse_matching_tool_outcome(
    payload: &[u8],
    expected: &Value,
) -> Result<ToolCallOutcome, BridgeError> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|_| BridgeError::Protocol("malformed response".into()))?;
    if value.get("id") != Some(expected) {
        return Err(BridgeError::Protocol(
            "response ID does not match the submitted request".into(),
        ));
    }
    parse_tool_outcome(payload)
}

#[cfg(any(windows, test))]
fn pipe_path(pipe: &str) -> String {
    if pipe.starts_with(r"\\.\pipe\") {
        pipe.to_owned()
    } else {
        format!(r"\\.\pipe\{pipe}")
    }
}

#[cfg(windows)]
fn enumerate_pipe_names() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(r"\\.\pipe\") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

#[cfg(not(windows))]
fn enumerate_pipe_names() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
async fn open_pipe(
    pipe: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, BridgeError> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let path = pipe_path(pipe);
    let mut attempt = 0u32;
    loop {
        match ClientOptions::new().open(&path) {
            Ok(client) => return Ok(client),
            Err(error) if is_pipe_busy(&error) && attempt < PIPE_BUSY_MAX_RETRIES => {
                attempt += 1;
                tokio::time::sleep(PIPE_BUSY_RETRY).await;
            }
            Err(error) => return Err(BridgeError::Unavailable(error.to_string())),
        }
    }
}

#[cfg(windows)]
fn is_pipe_busy(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(231)
}

#[cfg(windows)]
async fn write_frame(
    client: &mut tokio::net::windows::named_pipe::NamedPipeClient,
    body: &[u8],
) -> Result<(), BridgeError> {
    use tokio::io::AsyncWriteExt;

    client
        .write_all(&(body.len() as u32).to_le_bytes())
        .await
        .map_err(|error| BridgeError::Unavailable(error.to_string()))?;
    client
        .write_all(body)
        .await
        .map_err(|error| BridgeError::Unavailable(error.to_string()))?;
    client
        .flush()
        .await
        .map_err(|error| BridgeError::Unavailable(error.to_string()))
}

#[cfg(windows)]
async fn read_frame(
    client: &mut tokio::net::windows::named_pipe::NamedPipeClient,
) -> Result<Vec<u8>, BridgeError> {
    use tokio::io::AsyncReadExt;

    let mut length_bytes = [0u8; 4];
    client
        .read_exact(&mut length_bytes)
        .await
        .map_err(|error| BridgeError::Unavailable(error.to_string()))?;
    let length = u32::from_le_bytes(length_bytes);
    if length > MAX_RESPONSE_BYTES {
        return Err(BridgeError::Protocol(
            "bridge response exceeds the size limit".into(),
        ));
    }
    let mut body = vec![0u8; length as usize];
    client
        .read_exact(&mut body)
        .await
        .map_err(|error| BridgeError::Unavailable(error.to_string()))?;
    Ok(body)
}

/// What one candidate reported about the active sessions.
pub(super) enum Handshake {
    Targets(Vec<ActiveThread>),
    NoActiveSession,
}

/// Performs the read-only `list_threads` handshake against one candidate.
pub(super) async fn handshake(
    transport: &dyn BridgeTransport,
    candidate: &BridgeCandidate,
) -> Result<Handshake, BridgeError> {
    let outcome = tokio::time::timeout(
        CALL_TIMEOUT,
        transport.call_tool(
            &candidate.pipe,
            &candidate.caller_thread_id,
            "list_threads",
            json!({ "limit": 50 }),
        ),
    )
    .await
    .map_err(|_| BridgeError::Unavailable("the list_threads call timed out".into()))??;
    if !outcome.success {
        return Err(BridgeError::Protocol(
            "the list_threads call was not confirmed".into(),
        ));
    }
    let Some(text) = outcome.text else {
        return Err(BridgeError::Protocol(
            "the list_threads result carries no content".into(),
        ));
    };
    let threads = parse_active_threads(&text)?;
    if threads.is_empty() {
        return Ok(Handshake::NoActiveSession);
    }
    Ok(Handshake::Targets(threads))
}

/// Reduces the listed records to one target per session, preferring entries with `hostId`.
pub(super) fn session_targets(threads: &[ActiveThread]) -> Vec<&ActiveThread> {
    let mut targets: Vec<&ActiveThread> = Vec::new();
    for thread in threads {
        match targets.iter_mut().find(|target| target.id == thread.id) {
            Some(existing) => {
                if existing.host_id.is_none() && thread.host_id.is_some() {
                    *existing = thread;
                }
            }
            None => targets.push(thread),
        }
    }
    targets
}

/// Sends the pause request to one target.
pub(super) async fn send_pause(
    transport: &dyn BridgeTransport,
    candidate: &BridgeCandidate,
    target: &ActiveThread,
    prompt: &str,
    started: &AtomicBool,
    may_submit: &(dyn Fn() -> bool + Send + Sync),
) -> Result<bool, BridgeError> {
    let mut arguments = json!({ "threadId": target.id, "prompt": prompt });
    if let Some(host_id) = target.host_id.as_deref() {
        arguments["hostId"] = json!(host_id);
    }
    let caller = if looks_like_thread_id(&target.id) {
        &target.id
    } else {
        &candidate.caller_thread_id
    };
    let outcome = tokio::time::timeout(
        CALL_TIMEOUT,
        transport.call_tool_observed(
            &candidate.pipe,
            caller,
            "send_message_to_thread",
            arguments,
            started,
            may_submit,
        ),
    )
    .await
    .map_err(|_| BridgeError::Unavailable("the send_message_to_thread call timed out".into()))??;
    Ok(outcome.success)
}

/// Sends a resume message to the specified thread over an available bridge candidate.
pub async fn send_resume_to_thread(
    transport: &dyn BridgeTransport,
    target_thread_id: &str,
    prompt: &str,
    is_current: impl Fn() -> bool + Send,
) -> Result<String, String> {
    if !is_current() {
        return Err("quotaStale".into());
    }
    let discovery = discover_bridge().await;
    let candidates = build_candidates(
        discovery.explicit_pipe_path.as_deref(),
        &discovery.pipe_names,
        &discovery.caller_thread_ids,
    );
    send_resume_with_candidates(transport, target_thread_id, prompt, &candidates, is_current).await
}

async fn send_resume_with_candidates(
    transport: &dyn BridgeTransport,
    target_thread_id: &str,
    prompt: &str,
    candidates: &[BridgeCandidate],
    is_current: impl Fn() -> bool + Send,
) -> Result<String, String> {
    if candidates.is_empty() {
        return Err("bridgeUnavailable".into());
    }
    let arguments = json!({
        "threadId": target_thread_id,
        "prompt": prompt,
    });
    for candidate in candidates {
        if !is_current() {
            return Err("quotaStale".into());
        }
        let caller = if looks_like_thread_id(target_thread_id) {
            target_thread_id
        } else {
            &candidate.caller_thread_id
        };
        let call = transport.call_tool(
            &candidate.pipe,
            caller,
            "send_message_to_thread",
            arguments.clone(),
        );
        if let Ok(Ok(outcome)) = tokio::time::timeout(CALL_TIMEOUT, call).await {
            if outcome.success {
                return Ok(prompt.to_string());
            }
        }
    }
    Err("resumeDeliveryFailed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pause_control::testing;
    use serde_json::json;

    #[tokio::test(start_paused = true)]
    async fn manual_resume_rechecks_freshness_before_each_candidate() {
        let transport = testing::ScriptedTransport::new();
        transport.set_send_success(false);
        transport.set_send_delay_ms(1500);
        let candidates = vec![
            BridgeCandidate {
                pipe: "first".into(),
                caller_thread_id: "caller".into(),
            },
            BridgeCandidate {
                pipe: "second".into(),
                caller_thread_id: "caller".into(),
            },
        ];
        let started = tokio::time::Instant::now();
        let result = send_resume_with_candidates(
            &transport,
            "test-thread",
            "Continue",
            &candidates,
            || started.elapsed() < Duration::from_secs(1),
        )
        .await;
        assert_eq!(result, Err("quotaStale".into()));
        assert_eq!(transport.calls().len(), 1);
    }

    #[test]
    fn only_active_codex_sessions_are_targeted() {
        let payload = json!({
            "pinnedThreads": [
                {"id": "a", "kind": "codex", "status": "active", "hostId": "h"},
            ],
            "threads": [
                {"id": "a", "kind": "codex", "status": "active", "hostId": "h"},
                {"id": "b", "kind": "codex", "status": "idle"},
                {"id": "c", "kind": "chatgpt", "status": "active"},
                {"id": "", "kind": "codex", "status": "active"},
                {"id": "d", "kind": "codex", "status": "active"},
            ],
        })
        .to_string();

        let threads = parse_active_threads(&payload).unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "a");
        assert_eq!(threads[0].host_id.as_deref(), Some("h"));
        assert_eq!(threads[1].id, "d");
        assert_eq!(threads[1].host_id, None);
    }

    #[test]
    fn an_unparseable_payload_is_not_an_idle_observation() {
        assert!(parse_active_threads("not json").is_err());
        assert!(parse_active_threads("{}").is_err());
    }

    #[test]
    fn the_live_listing_shape_is_parsed_as_expected() {
        let payload = json!({
            "pinnedThreads": [],
            "threads": [
                {
                    "id": "0199b0d2-1111-7000-8000-000000000001",
                    "kind": "chatgpt",
                    "status": "idle",
                    "hostId": "local",
                },
                {
                    "id": "0199b0d2-2222-7000-8000-000000000002",
                    "kind": "codex",
                    "status": "idle",
                    "hostId": "local",
                },
                {
                    "id": "0199b0d2-3333-7000-8000-000000000003",
                    "kind": "codex",
                    "status": "notLoaded",
                    "hostId": "local",
                },
                {
                    "id": "0199b0d2-4444-7000-8000-000000000004",
                    "kind": "codex",
                    "status": "active",
                    "hostId": "local",
                },
            ],
        })
        .to_string();

        let threads = parse_active_threads(&payload).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "0199b0d2-4444-7000-8000-000000000004");
        assert_eq!(threads[0].host_id.as_deref(), Some("local"));

        let wrapped = serde_json::to_string(&Value::String(payload)).unwrap();
        assert_eq!(parse_active_threads(&wrapped).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn malformed_listings_fail_the_handshake_while_real_empty_lists_are_idle() {
        let transport = testing::ScriptedTransport::new();
        let candidate = BridgeCandidate {
            pipe: "test".into(),
            caller_thread_id: "caller".into(),
        };
        for text in ["not json", "{}", "null", "{\"threads\":null}", "{\"threads\":\"bad\"}"] {
            transport.set_list_threads(ToolCallOutcome {
                success: true,
                text: Some(text.into()),
            });
            assert!(matches!(
                handshake(&transport, &candidate).await,
                Err(BridgeError::Protocol(_))
            ));
        }
        for text in [
            "[]",
            "{\"threads\":[]}",
            "{\"pinnedThreads\":[],\"threads\":[]}",
        ] {
            transport.set_list_threads(ToolCallOutcome {
                success: true,
                text: Some(text.into()),
            });
            assert!(matches!(
                handshake(&transport, &candidate).await,
                Ok(Handshake::NoActiveSession)
            ));
        }
    }

    #[test]
    fn the_session_walk_is_bounded_and_prefers_the_newest_files() {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-session-walk-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let older = dir.join("2026").join("10").join("18");
        let newer = dir.join("2026").join("10").join("19");
        std::fs::create_dir_all(&older).unwrap();
        std::fs::create_dir_all(&newer).unwrap();
        for index in 0..20 {
            let name = format!(
                "rollout-2026-10-18T00-00-{index:02}-123e4567-e89b-12d3-a456-426614174000.jsonl.zst"
            );
            std::fs::write(older.join(name), b"{}").unwrap();
        }
        for index in 0..5 {
            let name = format!(
                "rollout-2026-10-19T00-00-{index:02}-123e4567-e89b-12d3-a456-426614174000.jsonl.zst"
            );
            std::fs::write(newer.join(name), b"{}").unwrap();
        }

        let mut files = Vec::new();
        collect_session_files(&dir, 0, MAX_SESSION_SCAN_DEPTH, 8, &mut files);

        assert_eq!(files.len(), 8, "the walk must stop at the limit");
        assert!(
            files.iter().take(5).all(|(_, path)| path.starts_with(&newer)),
            "the newest date folder must be visited first"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_thread_id_check_is_strict() {
        assert!(looks_like_thread_id("123e4567-e89b-12d3-a456-426614174000"));
        assert!(!looks_like_thread_id("123e4567-e89b-12d3-a456-42661417400"));
        assert!(!looks_like_thread_id("123e4567e89b12d3a456426614174000"));
        assert!(!looks_like_thread_id("123e4567-e89b-12d3-a456-42661417400g"));
        assert!(!looks_like_thread_id(""));
    }

    #[test]
    fn the_candidate_order_prefers_the_explicit_path_then_the_current_family() {
        let current = format!("{APP_TOOLS_PIPE_PREFIX}123e4567-e89b-12d3-a456-426614174000");
        let legacy = format!("{LEGACY_BROWSER_USE_PIPE_PREFIX}123e4567-e89b-12d3-a456-426614174001");
        let unrelated = "some-other-pipe".to_string();
        let names = vec![legacy.clone(), current.clone(), unrelated.clone()];

        let ordered = ordered_candidate_pipe_paths(Some("explicit-pipe"), &names);
        assert_eq!(
            ordered,
            vec!["explicit-pipe".to_string(), current, legacy]
        );
    }

    #[test]
    fn the_candidate_order_is_case_insensitively_deduplicated() {
        let current = format!("{APP_TOOLS_PIPE_PREFIX}123e4567-e89b-12d3-a456-426614174000");
        let names = vec![current.clone(), current.to_ascii_uppercase()];
        let ordered = ordered_candidate_pipe_paths(None, &names);
        assert_eq!(ordered.len(), 1);
    }

    #[test]
    fn an_unsupported_or_malformed_pipe_name_is_not_a_candidate() {
        let bare_prefix = format!("{APP_TOOLS_PIPE_PREFIX}not-a-uuid");
        let no_suffix = APP_TOOLS_PIPE_PREFIX.to_string();
        let names = vec![bare_prefix.clone(), no_suffix, "other-pipe".to_string()];
        assert!(ordered_candidate_pipe_paths(None, &names).is_empty());
        assert!(!is_supported_pipe_name(&bare_prefix));
    }

    #[test]
    fn the_candidates_cross_the_pipes_with_the_caller_identities() {
        let first = format!("{APP_TOOLS_PIPE_PREFIX}123e4567-e89b-12d3-a456-426614174000");
        let second = format!("{APP_TOOLS_PIPE_PREFIX}123e4567-e89b-12d3-a456-426614174001");
        let names = vec![first.clone(), second.clone()];
        let threads = vec!["a".to_string(), "b".to_string()];
        let candidates = build_candidates(None, &names, &threads);
        assert_eq!(candidates.len(), 4);
        assert_eq!(candidates[0].pipe, first);
        assert_eq!(candidates[0].caller_thread_id, "a");
        assert_eq!(candidates[1].caller_thread_id, "b");
        assert_eq!(candidates[2].pipe, second);
    }

    #[test]
    fn the_session_file_names_carry_the_uuid_before_the_extension_chain() {
        let uuid = "01a06af9-8ddb-7052-939a-904e2f644a73";
        assert_eq!(
            extract_trailing_uuid(&format!("rollout-2026-09-04T01-52-14-{uuid}.jsonl.zst")),
            Some(uuid.to_string())
        );
        assert_eq!(
            extract_trailing_uuid(&format!("rollout-2026-09-04T01-52-14-{uuid}.jsonl")),
            Some(uuid.to_string())
        );
        assert_eq!(
            extract_trailing_uuid(&format!("rollout-2026-09-04T01-52-14-{uuid}")),
            Some(uuid.to_string())
        );
        assert_eq!(extract_trailing_uuid("rollout-not-a-uuid.jsonl"), None);
        assert_eq!(extract_trailing_uuid(""), None);
    }

    #[test]
    fn the_compressed_and_plain_session_files_are_both_recognised() {
        let path = std::path::Path::new("rollout-2026-09-04.jsonl");
        assert!(is_session_file(path));
        assert!(is_session_file(std::path::Path::new(
            "rollout-2026-09-04.jsonl.zst"
        )));
        assert!(!is_session_file(std::path::Path::new(
            "rollout-2026-09-04.json"
        )));
        assert!(!is_session_file(std::path::Path::new(
            "rollout-2026-09-04.jsonl.zst.bak"
        )));
    }

    #[test]
    fn the_caller_identities_reject_a_malformed_environment_value() {
        for identity in candidate_caller_thread_ids() {
            assert!(looks_like_thread_id(&identity), "{identity}");
        }
    }

    #[test]
    fn a_json_rpc_error_is_not_a_confirmed_result() {
        let error =
            serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -1}})).unwrap();
        assert!(matches!(
            parse_tool_outcome(&error),
            Err(BridgeError::Protocol(_))
        ));

        let without_result = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1})).unwrap();
        assert!(matches!(
            parse_tool_outcome(&without_result),
            Err(BridgeError::Protocol(_))
        ));

        let malformed = b"{ not json";
        assert!(matches!(
            parse_tool_outcome(malformed),
            Err(BridgeError::Protocol(_))
        ));
    }

    #[test]
    fn a_confirmed_tool_result_carries_its_text() {
        let payload = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "success": true,
                "contentItems": [{"text": "{\"threads\": []}"}],
            },
        }))
        .unwrap();
        let outcome = parse_tool_outcome(&payload).expect("the result must parse");
        assert!(outcome.success);
        assert_eq!(outcome.text.as_deref(), Some("{\"threads\": []}"));
    }

    #[test]
    fn an_unconfirmed_tool_result_is_not_a_success() {
        let payload = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"success": false, "contentItems": []},
        }))
        .unwrap();
        let outcome = parse_tool_outcome(&payload).expect("the result must parse");
        assert!(!outcome.success);
        assert_eq!(outcome.text, None);
    }

    #[test]
    fn the_request_carries_the_fixed_protocol_fields() {
        let request = build_request("list_threads", json!({"limit": 50}), "thread-1");
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["method"], "tools/call");
        assert_eq!(request["params"]["callerSource"], "codex");
        assert_eq!(request["params"]["namespace"], "codex_app");
        assert_eq!(request["params"]["threadId"], "thread-1");
        assert_eq!(request["params"]["tool"], "list_threads");
        assert_eq!(request["params"]["arguments"]["limit"], 50);
        assert!(request["params"]["callId"].as_str().is_some());
        assert!(request["params"]["turnId"].as_str().is_some());
        assert!(request["id"].as_u64().is_some());

        let parameters = request["params"].as_object().expect("params are an object");
        let mut keys: Vec<&str> = parameters.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "arguments",
                "callId",
                "callerSource",
                "namespace",
                "threadId",
                "tool",
                "turnId",
            ]
        );
        assert!(!parameters.contains_key("name"));
    }

    #[test]
    fn the_request_id_increases_monotonically() {
        let first = build_request("list_threads", json!({}), "t");
        let second = build_request("list_threads", json!({}), "t");
        assert!(second["id"].as_u64().unwrap() > first["id"].as_u64().unwrap());
    }

    #[test]
    fn submitted_responses_require_the_original_rpc_id() {
        let good = br#"{"jsonrpc":"2.0","id":42,"result":{"success":true}}"#;
        assert!(parse_matching_tool_outcome(good, &json!(42)).unwrap().success);
        assert!(parse_matching_tool_outcome(good, &json!(43)).is_err());
        assert!(
            parse_matching_tool_outcome(br#"{"result":{"success":true}}"#, &json!(42)).is_err()
        );
    }
}

#[cfg(all(windows, test))]
mod windows_protocol_tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::ServerOptions;

    fn unique_pipe_name(tag: &str) -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{APP_TOOLS_PIPE_PREFIX}{tag}-{stamp}")
    }

    async fn spawn_server(response: Value, received: Arc<StdMutex<Vec<Value>>>) -> String {
        let name = unique_pipe_name("sim");
        let path = pipe_path(&name);
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)
            .expect("the simulated pipe must be creatable");

        tokio::spawn(async move {
            if server.connect().await.is_err() {
                return;
            }
            let mut length_bytes = [0u8; 4];
            if server.read_exact(&mut length_bytes).await.is_err() {
                return;
            }
            let length = u32::from_le_bytes(length_bytes) as usize;
            let mut body = vec![0u8; length];
            if server.read_exact(&mut body).await.is_err() {
                return;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                received.lock().unwrap().push(value);
            }
            let payload = serde_json::to_vec(&response).unwrap();
            let _ = server.write_all(&(payload.len() as u32).to_le_bytes()).await;
            let _ = server.write_all(&payload).await;
            let _ = server.flush().await;
        });

        name
    }

    #[tokio::test]
    async fn the_transport_speaks_the_real_named_pipe_protocol() {
        let received = Arc::new(StdMutex::new(Vec::new()));
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "success": true,
                "contentItems": [{"text": "{\"threads\":[{\"id\":\"s1\",\"kind\":\"codex\",\"status\":\"active\"}]}"}],
            },
        });
        let name = spawn_server(response, received.clone()).await;

        let transport = NamedPipeTransport;
        let outcome = transport
            .call_tool(&name, "caller-1", "list_threads", json!({"limit": 50}))
            .await
            .expect("the handshake must succeed");
        assert!(outcome.success);
        assert!(outcome
            .text
            .as_deref()
            .is_some_and(|text| text.contains("\"s1\"")));

        let recorded = received.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["method"], "tools/call");
        assert_eq!(recorded[0]["params"]["tool"], "list_threads");
        assert_eq!(recorded[0]["params"]["namespace"], "codex_app");
        assert_eq!(recorded[0]["params"]["threadId"], "caller-1");
        assert_eq!(recorded[0]["params"]["arguments"]["limit"], 50);
    }

    #[tokio::test]
    async fn an_oversized_response_is_refused() {
        let name = unique_pipe_name("huge");
        let path = pipe_path(&name);
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)
            .expect("the simulated pipe must be creatable");
        tokio::spawn(async move {
            if server.connect().await.is_err() {
                return;
            }
            let mut length_bytes = [0u8; 4];
            if server.read_exact(&mut length_bytes).await.is_err() {
                return;
            }
            let _ = server.write_all(&(MAX_RESPONSE_BYTES + 1).to_le_bytes()).await;
            let _ = server.flush().await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let transport = NamedPipeTransport;
        let result = transport
            .call_tool(&name, "caller-1", "list_threads", json!({}))
            .await;
        assert!(matches!(result, Err(BridgeError::Protocol(_))));
    }

    #[tokio::test]
    async fn a_silent_peer_does_not_answer() {
        let name = unique_pipe_name("silent");
        let path = pipe_path(&name);
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)
            .expect("the simulated pipe must be creatable");
        tokio::spawn(async move {
            let _ = server.connect().await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        });

        let transport = NamedPipeTransport;
        let result = tokio::time::timeout(
            Duration::from_millis(300),
            transport.call_tool(&name, "caller-1", "list_threads", json!({})),
        )
        .await;
        assert!(result.is_err(), "a silent peer must not produce a result");
    }
}

#[cfg(all(test, windows))]
mod cancelled_transport_tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_native_submission_never_opens_a_pipe_or_marks_it_written() {
        let started = AtomicBool::new(false);
        let cancelled = || false;
        let result = NamedPipeTransport
            .call_tool_observed(
                "unused-cancelled-test-pipe",
                "caller",
                "send_message_to_thread",
                json!({}),
                &started,
                &cancelled,
            )
            .await;
        assert!(result.is_err());
        assert!(!started.load(Ordering::Acquire));
    }
}
