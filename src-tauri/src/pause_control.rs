//! Automatic pause control for a running Codex session.
//!
//! When a quota window runs low, the overlay can ask the Codex desktop client
//! to pause the conversation instead of leaving the session to burn the
//! remaining quota. The request is a *user message* driven collaboration
//! request: the overlay never terminates a Codex process, never writes a
//! transcript and never starts an app-server of its own to race the desktop
//! client for session writes.
//!
//! The module is deliberately separate from `codex/client.rs` (which owns the
//! quota read) and from `notify.rs` (which owns reset notifications and their
//! persisted state). The auto-pause ledger is short-lived in-memory state and
//! is never written to disk.
//!
//! Layout:
//!
//! * pure decision functions — [`triggered_quotas`], [`pause_message`],
//!   [`parse_active_threads`], [`ordered_candidate_pipe_paths`],
//!   [`looks_like_thread_id`] — carry the contract and run on every platform;
//! * [`BridgeTransport`] is the transport seam. The production implementation
//!   speaks the Windows named-pipe protocol; the offline tests substitute a
//!   scripted bridge so the whole delivery path can be exercised offline;
//! * [`AutoPauseController`] owns the de-duplication ledger, the queued sample
//!   and the worker scheduling.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::codex::CodexUsage;
use crate::commands::AppState;
use crate::config::OverlaySettings;

/// The fixed pause request. The production path never assembles a prompt from
/// user input, settings or a translation resource.
pub const PAUSE_MESSAGE: &str = "\u{6682}\u{505C}";

/// The message that asks Codex to schedule a one-time follow-up. It is only
/// used when the 5-hour window triggered and its reset time is usable.
const SCHEDULED_PAUSE_MESSAGE: &str = "\u{6682}\u{505C}\u{FF0C}\u{8BBE}\u{7F6E}\u{4E00}\u{6B21}\u{6027}\u{5B9A}\u{65F6}\u{4EFB}\u{52A1}\u{FF0C}\u{5728}";

/// The current Codex desktop client bridge family.
pub const APP_TOOLS_PIPE_PREFIX: &str = "codex-app-tools-";
/// The older Codex desktop client bridge family.
pub const LEGACY_BROWSER_USE_PIPE_PREFIX: &str = "codex-browser-use-";

/// The deadline that covers one whole delivery batch — discovery, handshake and
/// every send. It is deliberately *not* a per-pipe timeout: with several pipe
/// and caller-identity combinations a per-call budget would stretch the total
/// wait to tens of seconds.
pub const DELIVERY_DEADLINE: Duration = Duration::from_secs(5);

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
///
/// The batch deadline is the hard bound, but a candidate whose pipe accepts the
/// connection and never answers would otherwise consume the whole budget and
/// starve every later candidate. The sub-deadline keeps a stuck combination
/// from monopolising the batch.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
/// The defensive cap on each ledger collection.
const MAX_COLLECTION_ENTRIES: usize = 512;
/// The maximum directory depth scanned under `.codex/sessions`.
const MAX_SESSION_SCAN_DEPTH: usize = 4;
/// How many recent session files are inspected for a caller thread identity.
const MAX_SESSION_FILES: usize = 32;
/// A sanity bound for the 5-hour reset distance.
///
/// The 5-hour window resets within five hours of the moment it started, so a
/// reset further away than six hours does not belong to that window. Such a
/// value never produces a scheduled message.
const MAX_PLAUSIBLE_RESET_DISTANCE_SECS: i64 = 6 * 60 * 60;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// The quota window a pause request belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QuotaKind {
    FiveHour,
    Weekly,
}

/// A quota window that crossed its configured threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub quota: QuotaKind,
    /// The reset timestamp of the window, when the sample carried one.
    pub resets_at: Option<i64>,
}

/// Decides which quota windows crossed their threshold.
///
/// This function only looks at the switch, the remaining percentages and the
/// reset cycles. It never touches the bridge, the network or the disk.
pub fn triggered_quotas(settings: &OverlaySettings, usage: &CodexUsage) -> Vec<Trigger> {
    if !settings.auto_pause_on_low_quota {
        return Vec::new();
    }

    let mut triggers = Vec::new();
    if let Some(remaining) = usage.five_hour_remaining_percent {
        if remaining <= settings.auto_pause_five_hour_threshold_percent {
            triggers.push(Trigger {
                quota: QuotaKind::FiveHour,
                resets_at: usage.five_hour_resets_at,
            });
        }
    }
    if let Some(remaining) = usage.week_remaining_percent {
        if remaining <= settings.auto_pause_weekly_threshold_percent {
            triggers.push(Trigger {
                quota: QuotaKind::Weekly,
                resets_at: usage.week_resets_at,
            });
        }
    }
    triggers
}

/// Builds the fixed pause message for the triggered windows.
///
/// The matrix is:
///
/// | Triggered | Message |
/// | --- | --- |
/// | 5-hour only, reset time usable | the scheduled pause request |
/// | 5-hour only, reset time missing, expired or unusable | `暂停` |
/// | Weekly only | `暂停` |
/// | both | `暂停`, Weekly wins |
///
/// `observed_at` is the `fetchedAt` of the sample that triggered the request,
/// so the remaining time is computed from the same fresh snapshot rather than
/// from a slightly later wall clock.
pub fn pause_message(triggers: &[Trigger], observed_at: i64) -> String {
    let weekly = triggers
        .iter()
        .any(|trigger| trigger.quota == QuotaKind::Weekly);
    if weekly {
        return PAUSE_MESSAGE.to_string();
    }

    let Some(five_hour) = triggers
        .iter()
        .find(|trigger| trigger.quota == QuotaKind::FiveHour)
    else {
        return PAUSE_MESSAGE.to_string();
    };

    let Some(resets_at) = five_hour.resets_at else {
        return PAUSE_MESSAGE.to_string();
    };
    // A missing or unusable observation time cannot produce a reliable
    // remaining duration, so the request falls back to the plain pause.
    if observed_at <= 0 {
        return PAUSE_MESSAGE.to_string();
    }
    // A malformed reset time must not overflow into a plausible looking
    // duration.
    let Some(distance) = resets_at.checked_sub(observed_at) else {
        return PAUSE_MESSAGE.to_string();
    };
    if distance <= 0 || distance > MAX_PLAUSIBLE_RESET_DISTANCE_SECS {
        return PAUSE_MESSAGE.to_string();
    }

    // Round a partial minute up so the scheduled task never fires early.
    let total_minutes = (distance + 59) / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    format!(
        "{SCHEDULED_PAUSE_MESSAGE}{hours}h {minutes}min +2min\u{540E}\u{53D1}\u{9001}\u{2018}\u{7EE7}\u{7EED}\u{2019}\u{4E24}\u{4E2A}\u{5B57}"
    )
}

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
pub fn parse_active_threads(text: &str) -> Vec<ActiveThread> {
    // A client that wraps the payload in one more JSON string would otherwise
    // empty the target list without any visible error.
    let value = match serde_json::from_str::<Value>(text) {
        Ok(Value::String(inner)) => match serde_json::from_str::<Value>(&inner) {
            Ok(value) => value,
            Err(_) => return Vec::new(),
        },
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    let mut threads = Vec::new();
    let mut seen: HashSet<(Option<String>, String)> = HashSet::new();
    match &value {
        Value::Array(items) => collect_active_threads(items, &mut threads, &mut seen),
        Value::Object(_) => {
            for key in ["pinnedThreads", "threads"] {
                if let Some(Value::Array(items)) = value.get(key) {
                    collect_active_threads(items, &mut threads, &mut seen);
                }
            }
        }
        _ => {}
    }
    threads
}

/// Strictly checks the 8-4-4-4-12 UUID shape used by the local Codex thread
/// identities.
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
///
/// Only a supported prefix with a valid UUID-shaped suffix is accepted.
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
///
/// The explicit environment path comes first, then the current
/// `codex-app-tools-*` pipes sorted by name, then the legacy
/// `codex-browser-use-*` pipes sorted by name. Paths are de-duplicated without
/// regard to case, so an explicit path and an enumerated path that name the
/// same pipe are probed only once.
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
        family.sort_by(|left, right| left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()));
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
///
/// The pipe is the outer loop, so the current client is always preferred over
/// the legacy client, and the explicit environment identity is always tried
/// first inside each pipe.
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
///
/// The environment variable comes first; the recent session files follow. Only
/// the identity is collected — the session bodies are never read and never
/// logged.
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
///
/// The walk visits the newest names first and stops as soon as `limit` files
/// were collected. The session tree is ordered by date and by rollout
/// timestamp, so the largest names are the newest files and the bound keeps
/// the newest ones instead of an arbitrary subset. A long session history
/// therefore costs no more than a short one.
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
///
/// The current client writes them compressed, so the extension chain is
/// `.jsonl.zst`; the uncompressed `.jsonl` of older clients is still accepted.
fn is_session_file(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lowered = name.to_ascii_lowercase();
    lowered.ends_with(".jsonl") || lowered.ends_with(".jsonl.zst")
}

/// Extracts the 36 character UUID that a session file name ends with.
///
/// The name may carry an extension chain after the UUID (`…-<uuid>.jsonl`,
/// `…-<uuid>.jsonl.zst`), so the extension chain is peeled off until the
/// trailing 36 characters form a UUID.
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

/// The outcome of one `tools/call` against a bridge pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallOutcome {
    /// The tool result's own `success` flag.
    pub success: bool,
    /// The first content item's text, when the result carried one.
    pub text: Option<String>,
}

/// Why a bridge call did not produce a usable result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    /// The pipe could not be opened, or the call did not complete.
    Unavailable(String),
    /// The peer answered, but not with a usable tool result.
    Protocol(String),
}

/// The transport seam.
///
/// The production implementation speaks the Windows named-pipe protocol; the
/// offline tests substitute a scripted bridge. The trait is object safe so the
/// controller can hold it and an offline test can replace it.
pub trait BridgeTransport: Send + Sync {
    fn call_tool<'a>(
        &'a self,
        pipe: &'a str,
        caller_thread_id: &'a str,
        tool_name: &'a str,
        arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>>;
}

/// The production transport: a JSON-RPC `tools/call` over a Windows named
/// pipe.
#[cfg(windows)]
pub struct NamedPipeTransport;

#[cfg(windows)]
impl BridgeTransport for NamedPipeTransport {
    fn call_tool<'a>(
        &'a self,
        pipe: &'a str,
        caller_thread_id: &'a str,
        tool_name: &'a str,
        arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>> {
        Box::pin(async move {
            let request = build_request(tool_name, arguments, caller_thread_id);
            let body =
                serde_json::to_vec(&request).map_err(|error| BridgeError::Protocol(error.to_string()))?;
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
fn default_transport() -> Arc<dyn BridgeTransport> {
    #[cfg(windows)]
    {
        Arc::new(NamedPipeTransport)
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnsupportedTransport)
    }
}

/// Builds the JSON-RPC envelope for one `tools/call`.
///
/// The desktop client validates the request against a closed schema, so the
/// parameter names are fixed: `arguments`, `callerSource`, `callId`,
/// `namespace`, `threadId`, `tool` and `turnId`. The tool *name* lives under
/// `tool`; `arguments` carries the tool's own arguments.
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

/// The Windows named-pipe path of a pipe name.
fn pipe_path(pipe: &str) -> String {
    if pipe.starts_with(r"\\.\pipe\") {
        pipe.to_owned()
    } else {
        format!(r"\\.\pipe\{pipe}")
    }
}

/// Lists the named pipes the machine currently exposes.
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

/// Lists the named pipes the machine currently exposes.
#[cfg(not(windows))]
fn enumerate_pipe_names() -> Vec<String> {
    Vec::new()
}

/// Opens a pipe, retrying while Windows reports that it is busy.
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

/// Whether the error is the Windows "pipe is busy" status.
#[cfg(windows)]
fn is_pipe_busy(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(231)
}

/// Writes one length-prefixed frame.
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

/// Reads one length-prefixed frame, rejecting an oversized response.
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
enum Handshake {
    /// The candidate answered and reported these targets.
    Targets(Vec<ActiveThread>),
    /// The candidate answered and reported no active Codex session.
    ///
    /// This is a valid observation, not a failure. The bridge is reachable
    /// and the session list is empty. It is also the only way to learn that
    /// a session left the active list, so it must not be discarded.
    NoActiveSession,
}

/// Performs the read-only `list_threads` handshake against one candidate.
///
/// A candidate is only usable when the tool call was confirmed and the result
/// carries parseable content. An empty session list is reported separately,
/// because it is an observation about the sessions rather than a broken
/// bridge.
async fn handshake(
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
    let threads = parse_active_threads(&text);
    if threads.is_empty() {
        return Ok(Handshake::NoActiveSession);
    }
    Ok(Handshake::Targets(threads))
}

/// Reduces the listed records to one target per session.
///
/// The listing may report the same session twice, once with and once without
/// a host id. The client needs the host id to route the message, so the
/// record that carries it wins.
fn session_targets(threads: &[ActiveThread]) -> Vec<&ActiveThread> {
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
///
/// Only a tool result whose `success` is `true` counts as a positive
/// confirmation; every other outcome is a failure that may be retried.
async fn send_pause(
    transport: &dyn BridgeTransport,
    candidate: &BridgeCandidate,
    target: &ActiveThread,
    prompt: &str,
) -> Result<bool, BridgeError> {
    let mut arguments = json!({ "threadId": target.id, "prompt": prompt });
    if let Some(host_id) = target.host_id.as_deref() {
        arguments["hostId"] = json!(host_id);
    }
    let outcome = tokio::time::timeout(
        CALL_TIMEOUT,
        transport.call_tool(
            &candidate.pipe,
            &candidate.caller_thread_id,
            "send_message_to_thread",
            arguments,
        ),
    )
    .await
    .map_err(|_| BridgeError::Unavailable("the send_message_to_thread call timed out".into()))??;
    Ok(outcome.success)
}

/// The de-duplication key of one delivered request.
///
/// The base key is the account identity, the quota window, its reset cycle and
/// the target session. The session component is the *qualified* target
/// identity, because a session that goes idle and comes back starts a new
/// active period and may be prompted again in the same quota cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeliveryKey {
    account: String,
    quota: QuotaKind,
    resets_at: Option<i64>,
    thread_id: String,
}

/// The active period bookkeeping of one target session.
#[derive(Debug, Clone)]
struct ThreadPeriod {
    /// The identity used in the delivery key.
    period_id: String,
}

/// The snapshot a delivery worker consumes.
#[derive(Debug, Clone)]
pub struct PendingSample {
    pub settings: OverlaySettings,
    pub usage: CodexUsage,
    pub account_id: String,
}

/// How the candidate list is produced.
#[derive(Clone, Default)]
pub struct BridgeDiscovery {
    /// The explicitly configured pipe, if any.
    pub explicit_pipe_path: Option<String>,
    /// The enumerated pipe names.
    pub pipe_names: Vec<String>,
    /// The local caller identities.
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

/// The outcome of one bridge lookup, as the settings window shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BridgeConnection {
    /// No handshake has completed yet.
    Unknown,
    /// A candidate answered the `list_threads` handshake.
    Available,
    /// No candidate answered the last batch.
    Unavailable,
}

/// The auto-pause status the settings window reads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoPauseStatus {
    /// Whether the feature is switched on.
    pub enabled: bool,
    /// Whether a bridge candidate is reachable.
    pub connection: BridgeConnection,
    /// How many pause requests the bridge positively confirmed.
    pub confirmed_count: u64,
    /// When the last positive confirmation happened, in unix seconds.
    pub last_confirmed_at: Option<i64>,
}

/// Starts a delivery worker. The production implementation uses the Tauri
/// runtime; the offline tests replace it so the worker runs on the test
/// runtime.
type WorkerSpawner = Arc<dyn Fn(Arc<AppState>) + Send + Sync>;

/// The short-lived in-memory state of the auto-pause feature.
///
/// It guards the de-duplication ledger, the queued sample and the worker
/// scheduling. It is a standard mutex on purpose: the critical sections never
/// wait on I/O, so the delivery worker can take a snapshot and release it
/// before touching a pipe.
pub struct AutoPauseController {
    /// The confirmed deliveries.
    sent: Vec<DeliveryKey>,
    /// The reserved but not yet confirmed deliveries.
    pending: Vec<DeliveryKey>,
    /// Whether a delivery worker is running.
    running: bool,
    /// The newest sample waiting for the running worker.
    queued: Option<PendingSample>,
    /// The active period of every currently listed session.
    periods: HashMap<String, ThreadPeriod>,
    /// The counter that mints synthetic period identities.
    period_counter: u64,
    /// The account the ledger belongs to.
    current_account: Option<String>,
    /// The bridge transport.
    transport: Arc<dyn BridgeTransport>,
    /// Replaces the live discovery. Only the offline tests set this.
    discovery_override: Option<BridgeDiscovery>,
    /// The last observed bridge availability.
    connection: BridgeConnection,
    /// How many requests were positively confirmed.
    confirmed_count: u64,
    /// When the last positive confirmation happened.
    last_confirmed_at: Option<i64>,
    /// The deadline of one delivery batch.
    delivery_deadline: Duration,
    /// Starts the delivery worker.
    spawn_worker: WorkerSpawner,
    /// Starts the read-only bridge probe.
    spawn_probe: WorkerSpawner,
}

impl Default for AutoPauseController {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoPauseController {
    /// Builds a controller with the production transport.
    pub fn new() -> Self {
        Self {
            sent: Vec::new(),
            pending: Vec::new(),
            running: false,
            queued: None,
            periods: HashMap::new(),
            period_counter: 0,
            current_account: None,
            transport: default_transport(),
            discovery_override: None,
            connection: BridgeConnection::Unknown,
            confirmed_count: 0,
            last_confirmed_at: None,
            delivery_deadline: DELIVERY_DEADLINE,
            spawn_worker: Arc::new(|state| {
                tauri::async_runtime::spawn(async move {
                    run_delivery_worker(state).await;
                });
            }),
            spawn_probe: Arc::new(|state| {
                tauri::async_runtime::spawn(async move {
                    probe_bridge(&state).await;
                });
            }),
        }
    }

    /// The status the settings window shows.
    pub fn status(&self, enabled: bool) -> AutoPauseStatus {
        AutoPauseStatus {
            enabled,
            connection: self.connection,
            confirmed_count: self.confirmed_count,
            last_confirmed_at: self.last_confirmed_at,
        }
    }

    /// The bridge transport, for the offline tests.
    #[cfg(test)]
    pub(crate) fn set_transport(&mut self, transport: Arc<dyn BridgeTransport>) {
        self.transport = transport;
    }

    /// Replaces the live discovery, for the offline tests.
    #[cfg(test)]
    pub(crate) fn set_discovery(&mut self, discovery: BridgeDiscovery) {
        self.discovery_override = Some(discovery);
    }

    /// Replaces the worker starter, for the offline tests.
    #[cfg(test)]
    pub(crate) fn set_worker_spawner(&mut self, spawner: WorkerSpawner) {
        self.spawn_worker = spawner;
    }

    /// Replaces the probe starter, for the offline tests.
    #[cfg(test)]
    pub(crate) fn set_probe_spawner(&mut self, spawner: WorkerSpawner) {
        self.spawn_probe = spawner;
    }

    /// Shortens the batch deadline, for the offline tests.
    #[cfg(test)]
    pub(crate) fn set_delivery_deadline(&mut self, deadline: Duration) {
        self.delivery_deadline = deadline;
    }

    /// Whether a delivery worker is still running, for the offline tests.
    #[cfg(test)]
    pub(crate) fn running_flag(&self) -> bool {
        self.running
    }

    /// How many requests are reserved but not yet confirmed.
    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// How many requests were positively confirmed.
    #[cfg(test)]
    pub(crate) fn sent_len(&self) -> usize {
        self.sent.len()
    }

    /// The candidate list for the next batch.
    ///
    /// The discovery is ordinary file system work, so it is produced before
    /// the controller is locked and the result is passed in.
    fn candidates(discovery: &BridgeDiscovery) -> Vec<BridgeCandidate> {
        build_candidates(
            discovery.explicit_pipe_path.as_deref(),
            &discovery.pipe_names,
            &discovery.caller_thread_ids,
        )
    }

    /// Drops the ledger entries that no longer apply.
    ///
    /// A confirmed or reserved request for another account is cleared, and a
    /// request whose quota cycle has ended is dropped. The account switch also
    /// clears the active period bookkeeping.
    fn prune(&mut self, account: &str, observed_at: i64) {
        if self.current_account.as_deref() != Some(account) {
            self.sent.clear();
            self.pending.clear();
            self.periods.clear();
            self.current_account = Some(account.to_owned());
        }
        let still_relevant = |key: &DeliveryKey| {
            key.resets_at
                .map(|resets_at| resets_at > observed_at)
                .unwrap_or(true)
        };
        self.sent.retain(still_relevant);
        self.pending.retain(still_relevant);
    }

    /// Releases the active period bookkeeping of every session.
    ///
    /// The period counter keeps counting, so a session that comes back later
    /// receives a fresh period identity. The de-duplication ledger keeps its
    /// entries: they belong to the periods that really happened.
    fn clear_active_periods(&mut self) {
        self.periods.clear();
    }

    /// Refreshes the active periods from a fresh listing.
    ///
    /// A session that is no longer listed ends its active period and its
    /// helper state is dropped, so a later return starts a new period.
    fn refresh_periods(&mut self, threads: &[ActiveThread]) {
        let active: HashSet<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
        self.periods.retain(|id, _| active.contains(id.as_str()));

        for thread in threads {
            if !self.periods.contains_key(&thread.id) {
                let period_id = self.next_period_id(thread);
                self.periods.insert(thread.id.clone(), ThreadPeriod { period_id });
            } else if let Some(turn) = thread
                .turn_id
                .as_deref()
                .map(str::trim)
                .filter(|turn| !turn.is_empty())
            {
                let period_id = format!("{}#t{}", thread.id, turn);
                if let Some(period) = self.periods.get_mut(&thread.id) {
                    period.period_id = period_id;
                }
            }
        }
    }

    /// Mints the period identity of a session.
    ///
    /// A bridge that reports a stable turn id identifies the active period
    /// with it. Without a turn id the identity is a counter that only advances
    /// after the session was actually observed leaving the active list, so two
    /// real turns that swap between two observations are not mistaken for a
    /// new period.
    fn next_period_id(&mut self, thread: &ActiveThread) -> String {
        match thread
            .turn_id
            .as_deref()
            .map(str::trim)
            .filter(|turn| !turn.is_empty())
        {
            Some(turn) => format!("{}#t{}", thread.id, turn),
            None => {
                self.period_counter += 1;
                format!("{}#p{}", thread.id, self.period_counter)
            }
        }
    }

    /// The current period identity of a session.
    fn period_id(&self, thread_id: &str) -> Option<String> {
        self.periods
            .get(thread_id)
            .map(|period| period.period_id.clone())
    }

    /// Reserves the quota keys of one target.
    fn reserve(
        &mut self,
        account: &str,
        thread_id: &str,
        triggers: &[Trigger],
    ) -> Reservation {
        let keys: Vec<DeliveryKey> = triggers
            .iter()
            .map(|trigger| DeliveryKey {
                account: account.to_owned(),
                quota: trigger.quota,
                resets_at: trigger.resets_at,
                thread_id: thread_id.to_owned(),
            })
            .collect();

        let uncovered: Vec<DeliveryKey> = keys
            .iter()
            .filter(|key| !self.sent.contains(key) && !self.pending.contains(key))
            .cloned()
            .collect();
        if uncovered.is_empty() {
            return Reservation::Nothing;
        }

        if self
            .sent
            .iter()
            .any(|key| key.account == account && key.thread_id == thread_id)
        {
            // A confirmed delivery already happened in this active period, so
            // the session is not interrupted again. The remaining quota keys are
            // recorded under the existing confirmation instead.
            for key in uncovered {
                self.push_sent(key);
            }
            return Reservation::Backfilled;
        }

        for key in &uncovered {
            self.push_pending(key.clone());
        }
        Reservation::Send { keys: uncovered }
    }

    /// Records a confirmed delivery.
    fn confirm(&mut self, keys: &[DeliveryKey]) {
        self.pending.retain(|key| !keys.contains(key));
        for key in keys {
            self.push_sent(key.clone());
        }
    }

    /// Releases a reservation that was not confirmed.
    fn release(&mut self, keys: &[DeliveryKey]) {
        self.pending.retain(|key| !keys.contains(key));
    }

    /// Adds a confirmed key, keeping the collection bounded.
    fn push_sent(&mut self, key: DeliveryKey) {
        if self.sent.contains(&key) {
            return;
        }
        self.sent.push(key);
        while self.sent.len() > MAX_COLLECTION_ENTRIES {
            self.sent.remove(0);
        }
    }

    /// Adds a reserved key, keeping the collection bounded.
    fn push_pending(&mut self, key: DeliveryKey) {
        if self.pending.contains(&key) {
            return;
        }
        self.pending.push(key);
        while self.pending.len() > MAX_COLLECTION_ENTRIES {
            self.pending.remove(0);
        }
    }
}

/// What the reservation decided.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reservation {
    /// Every quota key of this trigger set is already covered.
    Nothing,
    /// A previous confirmed delivery in this active period already covered the
    /// conversation; the remaining keys were recorded without sending again.
    Backfilled,
    /// The keys are reserved and a message must be sent.
    Send { keys: Vec<DeliveryKey> },
}

/// Locks the controller, recovering from a poisoned lock.
///
/// A panic inside a critical section must not wedge the feature: the ledger
/// stays consistent because every mutation happens under the same lock.
fn lock_controller(state: &Arc<AppState>) -> std::sync::MutexGuard<'_, AutoPauseController> {
    state
        .auto_pause
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Accepts a fresh sample and schedules a delivery when it qualifies.
///
/// The call is synchronous and never waits for a pipe: it performs the fast
/// eligibility check, merges the sample into the queue and starts a worker
/// only when none is running.
pub fn schedule(
    state: &Arc<AppState>,
    settings: OverlaySettings,
    usage: CodexUsage,
    account_id: Option<String>,
) {
    // A sample without a usable account identity must not trigger a delivery:
    // the de-duplication key would be built from an ambiguous placeholder.
    let Some(account_id) = account_id
        .map(|account| account.trim().to_owned())
        .filter(|account| !account.is_empty())
    else {
        return;
    };
    if !settings.auto_pause_on_low_quota {
        // The switch is off: the active periods are released, so a session
        // that comes back later starts a new period instead of reusing the
        // identity of the period that was already paused.
        lock_controller(state).clear_active_periods();
        return;
    }
    if triggered_quotas(&settings, &usage).is_empty() {
        // The quota is back above the threshold: every session gets a fresh
        // low-quota pause protection when the quota drops again.
        lock_controller(state).clear_active_periods();
        // The bridge status is unknown until the first handshake, so a
        // read-only probe answers it while the quota is still comfortable.
        probe_bridge_status(state);
        return;
    }

    let sample = PendingSample {
        settings,
        usage,
        account_id,
    };

    let spawner = {
        let mut controller = lock_controller(state);
        controller.prune(&sample.account_id, sample.usage.fetched_at);
        // Only the newest snapshot is kept, so a fast refresh cannot pile up
        // stale messages.
        controller.queued = Some(sample);
        if controller.running {
            None
        } else {
            controller.running = true;
            Some(controller.spawn_worker.clone())
        }
    };

    if let Some(spawner) = spawner {
        spawner(state.clone());
    }
}

/// Releases the active period bookkeeping of every session.
///
/// The bookkeeping follows the switch and the quota. A session that was
/// paused in an earlier low-quota period must not be swallowed by the
/// de-duplication ledger of that period, so it receives a new period
/// identity as soon as the feature is off or the quota is comfortable again.
pub fn clear_active_periods(state: &Arc<AppState>) {
    lock_controller(state).clear_active_periods();
}

/// Probes the bridge when the connection status is still unknown.
///
/// The probe is read-only: it performs the `list_threads` handshake and never
/// sends a message to a session, so it is safe to run while the quota is
/// comfortable. It runs in the background, because a pipe call may block.
pub fn probe_bridge_status(state: &Arc<AppState>) {
    let spawner = {
        let controller = lock_controller(state);
        // A known status is not replaced by a probe, and a delivery in flight
        // already reports the bridge state.
        if controller.connection != BridgeConnection::Unknown || controller.running {
            return;
        }
        controller.spawn_probe.clone()
    };
    spawner(state.clone());
}

/// Performs the read-only bridge probe.
///
/// The status is reported, never guessed: a candidate that answered proves
/// that the bridge is reachable, and a candidate that reported targets also
/// refreshes the active periods.
pub(crate) async fn probe_bridge(state: &Arc<AppState>) {
    let (transport, override_discovery) = {
        let controller = lock_controller(state);
        (
            controller.transport.clone(),
            controller.discovery_override.clone(),
        )
    };
    let discovery = match override_discovery {
        Some(discovery) => discovery,
        None => discover_bridge().await,
    };
    let candidates = AutoPauseController::candidates(&discovery);
    let mut answered = false;
    let mut threads: Option<Vec<ActiveThread>> = None;
    for candidate in candidates {
        match handshake(transport.as_ref(), &candidate).await {
            Err(_) => continue,
            Ok(Handshake::NoActiveSession) => {
                answered = true;
                break;
            }
            Ok(Handshake::Targets(listed)) => {
                answered = true;
                threads = Some(listed);
                break;
            }
        }
    }
    let mut controller = lock_controller(state);
    controller.connection = if answered {
        BridgeConnection::Available
    } else {
        BridgeConnection::Unavailable
    };
    if let Some(threads) = threads {
        controller.refresh_periods(&threads);
    }
}

/// Consumes the queued samples until the queue is empty.
///
/// It is `pub(crate)` so the offline test can run the worker on the test
/// runtime instead of the Tauri runtime.
pub(crate) async fn run_delivery_worker(state: Arc<AppState>) {
    loop {
        let sample = {
            let mut controller = lock_controller(&state);
            match controller.queued.take() {
                Some(sample) => sample,
                None => {
                    controller.running = false;
                    return;
                }
            }
        };

        // The whole batch shares one deadline. A growing candidate list cannot
        // stretch the total wait, and a cancelled batch releases its
        // reservations through the guard below.
        let deadline = { lock_controller(&state).delivery_deadline };
        let _ = tokio::time::timeout(deadline, deliver_batch(&state, &sample)).await;
    }
}

/// Delivers one batch: discover, handshake, reserve and send.
async fn deliver_batch(state: &Arc<AppState>, sample: &PendingSample) {
    let triggers = triggered_quotas(&sample.settings, &sample.usage);
    if triggers.is_empty() {
        return;
    }
    let prompt = pause_message(&triggers, sample.usage.fetched_at);

    // The transport and the discovery override are read under the lock, but
    // the discovery itself is ordinary file system work. Running it while
    // the controller is held would block every other user of the state and
    // could not be interrupted by the batch deadline.
    let (transport, override_discovery) = {
        let controller = lock_controller(state);
        (
            controller.transport.clone(),
            controller.discovery_override.clone(),
        )
    };
    let discovery = match override_discovery {
        Some(discovery) => discovery,
        None => discover_bridge().await,
    };
    let candidates = AutoPauseController::candidates(&discovery);
    if candidates.is_empty() {
        let mut controller = lock_controller(state);
        controller.connection = BridgeConnection::Unavailable;
        return;
    }

    // A candidate that answered without reporting an active session is still
    // an observation: the sessions left the active list. The first candidate
    // that reports targets wins.
    let mut answered = false;
    let mut reported: Option<(BridgeCandidate, Vec<ActiveThread>)> = None;
    // A pipe that answered without reporting an active session is exhausted.
    // The remaining caller identities of the same pipe would repeat the same
    // read-only call against a pipe that is already known to be idle.
    let mut exhausted_pipes: HashSet<String> = HashSet::new();
    for candidate in candidates {
        if exhausted_pipes.contains(&candidate.pipe.to_ascii_lowercase()) {
            continue;
        }
        match handshake(transport.as_ref(), &candidate).await {
            // A failed handshake moves on to the next candidate; the pipe name
            // alone never proves that a bridge is usable.
            Err(_) => continue,
            Ok(Handshake::NoActiveSession) => {
                answered = true;
                exhausted_pipes.insert(candidate.pipe.to_ascii_lowercase());
                continue;
            }
            Ok(Handshake::Targets(threads)) => {
                answered = true;
                reported = Some((candidate, threads));
                break;
            }
        }
    }

    let Some((candidate, threads)) = reported else {
        // No candidate reported a target. When a candidate answered, the
        // observation is that nothing is active, so every active period ends
        // here and a session that returns later starts a new one. Without that
        // observation a returning session would keep its old period identity
        // and could never be prompted again in the same quota cycle.
        let mut controller = lock_controller(state);
        controller.connection = if answered {
            BridgeConnection::Available
        } else {
            BridgeConnection::Unavailable
        };
        if answered {
            controller.refresh_periods(&[]);
        }
        return;
    };

    {
        let mut controller = lock_controller(state);
        controller.connection = BridgeConnection::Available;
        controller.refresh_periods(&threads);
    }
    for thread in session_targets(&threads) {
        let period_id = { lock_controller(state).period_id(&thread.id) };
        let Some(period_id) = period_id else {
            continue;
        };
        let reservation =
            { lock_controller(state).reserve(&sample.account_id, &period_id, &triggers) };
        let keys = match reservation {
            Reservation::Nothing | Reservation::Backfilled => continue,
            Reservation::Send { keys } => keys,
        };

        // The guard releases the reservation on every failure path,
        // including a cancelled batch, so a timeout never blocks a retry.
        let mut guard = ReservationGuard::new(state.clone(), keys.clone());
        let confirmed = send_pause(transport.as_ref(), &candidate, thread, &prompt)
            .await
            .unwrap_or(false);
        if confirmed {
            let mut controller = lock_controller(state);
            controller.confirm(&keys);
            controller.confirmed_count += 1;
            controller.last_confirmed_at = Some(unix_now());
            drop(controller);
            guard.mark_confirmed();
        }
    }
}

/// Discovers the bridge on a blocking thread.
///
/// Enumerating the named pipes and walking the session directory is ordinary
/// file system work. Running it on the blocking pool keeps the quota refresh
/// path responsive and keeps the controller lock free while it runs.
async fn discover_bridge() -> BridgeDiscovery {
    match tokio::task::spawn_blocking(BridgeDiscovery::discover).await {
        Ok(discovery) => discovery,
        Err(_) => BridgeDiscovery::default(),
    }
}

/// Releases a reservation that was not confirmed.
struct ReservationGuard {
    state: Arc<AppState>,
    keys: Vec<DeliveryKey>,
    confirmed: bool,
}

impl ReservationGuard {
    fn new(state: Arc<AppState>, keys: Vec<DeliveryKey>) -> Self {
        Self {
            state,
            keys,
            confirmed: false,
        }
    }

    fn mark_confirmed(&mut self) {
        self.confirmed = true;
    }
}

impl Drop for ReservationGuard {
    fn drop(&mut self) {
        if self.confirmed {
            return;
        }
        let mut controller = lock_controller(&self.state);
        controller.release(&self.keys);
    }
}

/// The current unix time in seconds.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// The scripted bridge used by the offline tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::Mutex;

    /// One recorded `tools/call`.
    #[derive(Debug, Clone, PartialEq)]
    pub(crate) struct RecordedCall {
        pub pipe: String,
        pub caller_thread_id: String,
        pub tool_name: String,
        pub arguments: Value,
    }

    /// A bridge that answers from a script instead of a real Codex client.
    pub(crate) struct ScriptedTransport {
        inner: Mutex<ScriptedState>,
    }

    #[derive(Default)]
    struct ScriptedState {
        calls: Vec<RecordedCall>,
        list_threads: Option<ToolCallOutcome>,
        send_success: bool,
        send_error: Option<String>,
        send_delay_ms: u64,
    }

    impl ScriptedTransport {
        /// A bridge whose `list_threads` answers with one active session.
        pub(crate) fn new() -> Self {
            let state = ScriptedState {
                list_threads: Some(ToolCallOutcome {
                    success: true,
                    text: Some(
                        json!({
                            "pinnedThreads": [],
                            "threads": [{
                                "id": "session-1",
                                "kind": "codex",
                                "status": "active",
                                "hostId": "host-1",
                            }],
                        })
                        .to_string(),
                    ),
                }),
                send_success: true,
                ..ScriptedState::default()
            };
            Self {
                inner: Mutex::new(state),
            }
        }

        /// Replaces the `list_threads` outcome.
        pub(crate) fn set_list_threads(&self, outcome: ToolCallOutcome) {
            self.inner.lock().unwrap().list_threads = Some(outcome);
        }

        /// Replaces the `send_message_to_thread` outcome.
        pub(crate) fn set_send_success(&self, success: bool) {
            self.inner.lock().unwrap().send_success = success;
        }

        /// Makes every call fail with a transport error.
        pub(crate) fn set_send_error(&self, error: &str) {
            self.inner.lock().unwrap().send_error = Some(error.to_owned());
        }

        /// Hangs only the `send_message_to_thread` call, so the handshake
        /// succeeds and the batch is cut off while a request is in flight.
        pub(crate) fn set_send_delay_ms(&self, delay_ms: u64) {
            self.inner.lock().unwrap().send_delay_ms = delay_ms;
        }

        /// Every recorded call.
        pub(crate) fn calls(&self) -> Vec<RecordedCall> {
            self.inner.lock().unwrap().calls.clone()
        }

        /// The prompts that were sent.
        pub(crate) fn sent_prompts(&self) -> Vec<String> {
            self.inner
                .lock()
                .unwrap()
                .calls
                .iter()
                .filter(|call| call.tool_name == "send_message_to_thread")
                .filter_map(|call| {
                    call.arguments
                        .get("prompt")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect()
        }
    }

    impl BridgeTransport for ScriptedTransport {
        fn call_tool<'a>(
            &'a self,
            pipe: &'a str,
            caller_thread_id: &'a str,
            tool_name: &'a str,
            arguments: Value,
        ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>> {
            Box::pin(async move {
                let (list_threads, send_success, send_error, send_delay_ms) = {
                    let mut state = self.inner.lock().unwrap();
                    state.calls.push(RecordedCall {
                        pipe: pipe.to_owned(),
                        caller_thread_id: caller_thread_id.to_owned(),
                        tool_name: tool_name.to_owned(),
                        arguments: arguments.clone(),
                    });
                    (
                        state.list_threads.clone(),
                        state.send_success,
                        state.send_error.clone(),
                        state.send_delay_ms,
                    )
                };
                if tool_name == "list_threads" {
                    return match list_threads {
                        Some(outcome) => Ok(outcome),
                        None => Err(BridgeError::Protocol(
                            "the list_threads call was not confirmed".into(),
                        )),
                    };
                }
                if send_delay_ms > 0 {
                    // The send hangs, so the batch deadline has to cut it off.
                    tokio::time::sleep(Duration::from_millis(send_delay_ms)).await;
                }
                if let Some(error) = send_error {
                    return Err(BridgeError::Unavailable(error));
                }
                Ok(ToolCallOutcome {
                    success: send_success,
                    text: None,
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings() -> OverlaySettings {
        let mut settings = OverlaySettings::default();
        settings.auto_pause_on_low_quota = true;
        settings
    }

    fn usage(
        five_hour: Option<u32>,
        five_hour_resets_at: Option<i64>,
        week: Option<u32>,
        week_resets_at: Option<i64>,
        fetched_at: i64,
    ) -> CodexUsage {
        CodexUsage {
            five_hour_remaining_percent: five_hour,
            five_hour_burn_rate_per_hour: None,
            week_remaining_percent: week,
            week_burn_rate_per_hour: None,
            credits_display: "—".to_string(),
            credits_balance: None,
            has_credits: false,
            five_hour_resets_at,
            week_resets_at,
            fetched_at,
        }
    }

    #[test]
    fn the_switch_gates_every_trigger() {
        let mut disabled = OverlaySettings::default();
        disabled.auto_pause_on_low_quota = false;
        let sample = usage(Some(0), Some(1_800), Some(0), Some(1_800), 1_000);
        assert!(triggered_quotas(&disabled, &sample).is_empty());
        assert_eq!(triggered_quotas(&settings(), &sample).len(), 2);
    }

    #[test]
    fn each_threshold_triggers_at_its_own_boundary() {
        let current = settings();

        let five_hit = usage(Some(5), Some(1_800), Some(50), Some(1_800), 1_000);
        let triggers = triggered_quotas(&current, &five_hit);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].quota, QuotaKind::FiveHour);

        let five_missed = usage(Some(6), Some(1_800), Some(50), Some(1_800), 1_000);
        assert!(triggered_quotas(&current, &five_missed).is_empty());

        let weekly_hit = usage(Some(50), Some(1_800), Some(2), Some(1_800), 1_000);
        let triggers = triggered_quotas(&current, &weekly_hit);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].quota, QuotaKind::Weekly);

        let weekly_missed = usage(Some(50), Some(1_800), Some(3), Some(1_800), 1_000);
        assert!(triggered_quotas(&current, &weekly_missed).is_empty());
    }

    #[test]
    fn an_unknown_remaining_percentage_never_triggers() {
        let sample = usage(None, Some(1_800), None, Some(1_800), 1_000);
        assert!(triggered_quotas(&settings(), &sample).is_empty());
    }

    #[test]
    fn the_five_hour_message_schedules_a_one_time_task() {
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(1_000 + 3_600 + 120),
        }];
        let message = pause_message(&triggers, 1_000);
        // 3720 seconds is exactly 62 minutes, so the remaining time reads
        // "1h 2min".
        assert_eq!(
            message,
            "\u{6682}\u{505C}\u{FF0C}\u{8BBE}\u{7F6E}\u{4E00}\u{6B21}\u{6027}\u{5B9A}\u{65F6}\u{4EFB}\u{52A1}\u{FF0C}\u{5728}1h 2min +2min\u{540E}\u{53D1}\u{9001}\u{2018}\u{7EE7}\u{7EED}\u{2019}\u{4E24}\u{4E2A}\u{5B57}"
        );
    }

    #[test]
    fn a_partial_minute_rounds_up() {
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(1_000 + 61),
        }];
        assert!(pause_message(&triggers, 1_000).contains("0h 2min"));
    }

    #[test]
    fn an_unusable_reset_time_falls_back_to_the_plain_pause() {
        for resets_at in [
            None,
            Some(999),
            // Beyond the 5-hour window: a reset that far away is not one, and
            // a scheduled message would tell Codex to wait for days.
            Some(1_000 + 6 * 3_600 + 1),
            Some(1_000 + 30 * 24 * 3_600),
            // A malformed value must not overflow into a plausible duration.
            Some(i64::MIN),
            Some(i64::MAX),
        ] {
            let triggers = vec![Trigger {
                quota: QuotaKind::FiveHour,
                resets_at,
            }];
            assert_eq!(pause_message(&triggers, 1_000), PAUSE_MESSAGE);
        }
    }

    #[test]
    fn weekly_wins_over_the_five_hour_window() {
        let triggers = vec![
            Trigger {
                quota: QuotaKind::FiveHour,
                resets_at: Some(1_000 + 3_600),
            },
            Trigger {
                quota: QuotaKind::Weekly,
                resets_at: Some(1_000 + 86_400),
            },
        ];
        assert_eq!(pause_message(&triggers, 1_000), PAUSE_MESSAGE);
    }

    #[test]
    fn a_weekly_trigger_alone_uses_the_plain_pause() {
        let triggers = vec![Trigger {
            quota: QuotaKind::Weekly,
            resets_at: Some(1_000 + 86_400),
        }];
        assert_eq!(pause_message(&triggers, 1_000), PAUSE_MESSAGE);
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

        let threads = parse_active_threads(&payload);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "a");
        assert_eq!(threads[0].host_id.as_deref(), Some("h"));
        assert_eq!(threads[1].id, "d");
        assert_eq!(threads[1].host_id, None);
    }

    #[test]
    fn an_unparseable_payload_yields_no_target() {
        assert!(parse_active_threads("not json").is_empty());
        assert!(parse_active_threads("{}").is_empty());
    }

    /// The shape the live Codex client returns, observed through a read-only
    /// handshake. The listing carries both groups and a mixture of kinds and
    /// states, so only the active Codex records may survive.
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

        let threads = parse_active_threads(&payload);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "0199b0d2-4444-7000-8000-000000000004");
        assert_eq!(threads[0].host_id.as_deref(), Some("local"));

        // A client that adds one more layer of JSON encoding must not empty
        // the target list.
        let wrapped = serde_json::to_string(&Value::String(payload)).unwrap();
        assert_eq!(parse_active_threads(&wrapped).len(), 1);
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
            vec![
                "explicit-pipe".to_string(),
                current.clone(),
                legacy.clone()
            ]
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

    /// The current client writes its transcripts compressed, so the thread
    /// identity sits *before* the extension chain rather than at the very end
    /// of the file name.
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
        // The environment is not set in the test process, so the list is empty
        // or holds only identities that were discovered from real session files.
        for identity in candidate_caller_thread_ids() {
            assert!(looks_like_thread_id(&identity), "{identity}");
        }
    }

    #[test]
    fn a_json_rpc_error_is_not_a_confirmed_result() {
        let error = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -1}})).unwrap();
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

        // The desktop client validates the request against a closed schema, so
        // an unknown parameter is rejected. The tool name lives under `tool`;
        // a stray `name` key would make the whole call fail.
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
    fn a_period_identity_is_minted_per_active_period() {
        let mut controller = AutoPauseController::new();
        let thread = ActiveThread {
            id: "session-1".to_string(),
            host_id: None,
            turn_id: None,
        };
        controller.refresh_periods(std::slice::from_ref(&thread));
        let first = controller.period_id("session-1").expect("a period is minted");
        assert!(first.starts_with("session-1#p"));

        // Still listed: the period does not change.
        controller.refresh_periods(std::slice::from_ref(&thread));
        assert_eq!(controller.period_id("session-1").as_deref(), Some(first.as_str()));

        // No longer listed: the helper state is dropped.
        controller.refresh_periods(&[]);
        assert_eq!(controller.period_id("session-1"), None);

        // Listed again: a new period, so the session may be prompted again.
        controller.refresh_periods(std::slice::from_ref(&thread));
        let second = controller.period_id("session-1").expect("a new period is minted");
        assert_ne!(first, second);
    }

    #[test]
    fn a_reported_turn_id_identifies_the_period() {
        let mut controller = AutoPauseController::new();
        let thread = ActiveThread {
            id: "session-1".to_string(),
            host_id: None,
            turn_id: Some("turn-9".to_string()),
        };
        controller.refresh_periods(std::slice::from_ref(&thread));
        assert_eq!(
            controller.period_id("session-1").as_deref(),
            Some("session-1#tturn-9")
        );
    }

    #[test]
    fn a_reservation_is_only_recorded_after_a_confirmation() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];

        let reservation = controller.reserve("acct", "session-1#p1", &triggers);
        let keys = match reservation {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        assert_eq!(keys.len(), 1);

        // A failure only releases the reservation.
        controller.release(&keys);
        let again = controller.reserve("acct", "session-1#p1", &triggers);
        assert!(matches!(again, Reservation::Send { .. }));
    }

    #[test]
    fn a_confirmed_reservation_is_not_repeated() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        let keys = match controller.reserve("acct", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &triggers),
            Reservation::Nothing
        ));
    }

    #[test]
    fn two_quota_windows_produce_one_message_and_two_keys() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![
            Trigger {
                quota: QuotaKind::FiveHour,
                resets_at: Some(2_000),
            },
            Trigger {
                quota: QuotaKind::Weekly,
                resets_at: Some(9_000),
            },
        ];
        let keys = match controller.reserve("acct", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        assert_eq!(keys.len(), 2);
        controller.confirm(&keys);

        // Both windows are covered now.
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &triggers),
            Reservation::Nothing
        ));
    }

    #[test]
    fn a_later_second_window_in_the_same_period_is_backfilled_without_a_message() {
        let mut controller = AutoPauseController::new();
        let five_hour = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        let weekly = vec![Trigger {
            quota: QuotaKind::Weekly,
            resets_at: Some(9_000),
        }];

        let keys = match controller.reserve("acct", "session-1#p1", &five_hour) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);

        // The weekly window crosses later in the same active period: the
        // session is not interrupted again, the key is recorded instead.
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &weekly),
            Reservation::Backfilled
        ));
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &weekly),
            Reservation::Nothing
        ));
    }

    #[test]
    fn a_new_session_may_be_prompted_on_its_own() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        let keys = match controller.reserve("acct", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);

        assert!(matches!(
            controller.reserve("acct", "session-2#p2", &triggers),
            Reservation::Send { .. }
        ));
    }

    #[test]
    fn an_account_switch_clears_the_ledger() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        let keys = match controller.reserve("acct-1", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);

        controller.prune("acct-2", 1_000);
        assert!(matches!(
            controller.reserve("acct-2", "session-1#p1", &triggers),
            Reservation::Send { .. }
        ));
    }

    #[test]
    fn an_expired_quota_cycle_is_dropped() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];

        // The account is established before anything is recorded. Without
        // this the later prune could pass by clearing the whole ledger on an
        // account switch, which would not prove that the expired cycle was
        // the reason.
        controller.prune("acct", 1_000);

        let keys = match controller.reserve("acct", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);

        // The cycle is still running, so the record still applies.
        controller.prune("acct", 1_500);
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &triggers),
            Reservation::Nothing
        ));

        // The cycle ended: the record no longer applies.
        controller.prune("acct", 3_000);
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &triggers),
            Reservation::Send { .. }
        ));
    }

    #[test]
    fn the_ledger_is_bounded() {
        let mut controller = AutoPauseController::new();
        for index in 0..(MAX_COLLECTION_ENTRIES + 50) {
            let triggers = vec![Trigger {
                quota: QuotaKind::FiveHour,
                resets_at: Some(2_000),
            }];
            let thread_id = format!("session-{index}");
            let keys = match controller.reserve("acct", &thread_id, &triggers) {
                Reservation::Send { keys } => keys,
                other => panic!("expected a send reservation, got {other:?}"),
            };
            controller.confirm(&keys);
        }
        assert!(controller.sent.len() <= MAX_COLLECTION_ENTRIES);
    }
}

/// The Windows named-pipe protocol simulation.
///
/// It builds a real pipe, speaks the real framing and checks the real
/// handshake, so the protocol is not only asserted against a mock.
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

    /// A pipe server that answers one request and records what it received.
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
        // A length prefix beyond the 8 MiB limit must be rejected without
        // reading the body.
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
