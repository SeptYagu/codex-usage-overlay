//! Bounded quota reads and privacy-preserving local diagnostics.
use super::client::RawCodexUsage;
use serde::{Deserialize, Serialize};
use std::{fmt, future::Future, time::Duration};
use tokio::time::{sleep, timeout_at, Instant};

pub const REFRESH_BUDGET: Duration = Duration::from_secs(45);
pub const CLEANUP_BUDGET: Duration = Duration::from_secs(2);
pub const INITIALIZE_BUDGET: Duration = Duration::from_secs(10);
pub const ACTION_MAX_AGE: Duration = Duration::from_secs(30);
const HISTORY_BYTES: usize = 256 * 1024;
const HISTORY_EVENTS: usize = 200;
const HISTORY_AGE: i64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadStage {
    Discovery,
    Spawn,
    Initialize,
    Quota,
    Parse,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    Timeout,
    Transport,
    Rpc,
    Authentication,
    InvalidResponse,
    InvalidQuota,
    MissingExecutable,
    Permission,
    Spawn,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFailure {
    pub stage: ReadStage,
    pub kind: FailureKind,
    pub rpc_code: Option<i64>,
    pub http_status: Option<u16>,
    pub retry_after_seconds: Option<u64>,
}

impl ReadFailure {
    pub fn new(stage: ReadStage, kind: FailureKind) -> Self {
        Self {
            stage,
            kind,
            rpc_code: None,
            http_status: None,
            retry_after_seconds: None,
        }
    }
    pub fn timeout(stage: ReadStage) -> Self {
        Self::new(stage, FailureKind::Timeout)
    }
    pub fn transient(&self) -> bool {
        matches!(self.kind, FailureKind::Timeout | FailureKind::Transport)
            || (self.kind == FailureKind::Rpc
                && matches!(self.http_status, Some(429 | 500 | 502 | 503 | 504)))
    }
    pub fn io(stage: ReadStage, error: &std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        let kind = match error.kind() {
            PermissionDenied => FailureKind::Permission,
            NotFound if stage == ReadStage::Spawn => FailureKind::MissingExecutable,
            BrokenPipe | ConnectionReset | ConnectionAborted | UnexpectedEof | Interrupted
            | TimedOut => FailureKind::Transport,
            _ => {
                if stage == ReadStage::Spawn {
                    FailureKind::Spawn
                } else {
                    FailureKind::InvalidResponse
                }
            }
        };
        Self::new(stage, kind)
    }
    pub fn rpc(stage: ReadStage, error: &serde_json::Value) -> Self {
        let status = error
            .pointer("/data/status")
            .or_else(|| error.pointer("/data/httpStatus"))
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok());
        Self {
            stage,
            kind: if matches!(status, Some(401 | 403)) {
                FailureKind::Authentication
            } else {
                FailureKind::Rpc
            },
            rpc_code: error.get("code").and_then(|v| v.as_i64()),
            http_status: status,
            retry_after_seconds: error
                .pointer("/data/retryAfterSeconds")
                .and_then(|v| v.as_u64()),
        }
    }
}
impl fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let method = match self.stage {
            ReadStage::Initialize => "initialize",
            ReadStage::Quota => "account/rateLimits/read",
            ReadStage::Parse => "quota payload",
            ReadStage::Discovery => "executable discovery",
            ReadStage::Spawn => "process spawn",
            ReadStage::Refresh => "refresh",
        };
        if self.kind == FailureKind::Timeout {
            write!(f, "读取 Codex 用量超时 / Codex {method} timed out")
        } else {
            write!(
                f,
                "Codex {method} failed ({:?}, RPC {:?}, HTTP {:?})",
                self.kind, self.rpc_code, self.http_status
            )
        }
    }
}
impl std::error::Error for ReadFailure {}

#[derive(Debug, Clone, Copy)]
pub struct ObservationTiming {
    pub quota_started: Instant,
    pub received: Instant,
}
impl ObservationTiming {
    pub fn action_eligible(self) -> bool {
        self.quota_started.elapsed() <= ACTION_MAX_AGE
    }
    pub fn latency(self) -> Duration {
        self.received.saturating_duration_since(self.quota_started)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StderrSummary {
    pub model_timeouts: u32,
    pub plugin_failures: u32,
    pub other_lines: u32,
    pub oversized_lines: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadDiagnostic {
    pub started_at: i64,
    pub attempt: u8,
    pub elapsed_ms: u64,
    pub initialize_ms: Option<u64>,
    pub quota_ms: Option<u64>,
    pub success: bool,
    pub failure: Option<ReadFailure>,
    pub cli_version: Option<String>,
    #[serde(default)]
    pub executable_kind: Option<ExecutableKind>,
    #[serde(default)]
    pub executable_modified_at: Option<u64>,
    pub stderr: StderrSummary,
    pub cleanup_failed: bool,
}
impl ReadDiagnostic {
    pub fn new(attempt: u8) -> Self {
        Self {
            started_at: unix_now(),
            attempt,
            elapsed_ms: 0,
            initialize_ms: None,
            quota_ms: None,
            success: false,
            failure: None,
            cli_version: None,
            executable_kind: None,
            executable_modified_at: None,
            stderr: Default::default(),
            cleanup_failed: false,
        }
    }
    fn valid(&self, now: i64) -> bool {
        self.started_at >= now.saturating_sub(HISTORY_AGE)
            && self.started_at <= now.saturating_add(5)
            && (1..=2).contains(&self.attempt)
            && self.elapsed_ms <= 60_000
            && self.initialize_ms.is_none_or(|v| v <= 60_000)
            && self.quota_ms.is_none_or(|v| v <= 60_000)
            && self.success == self.failure.is_none()
            && self.cli_version.as_ref().is_none_or(|s| safe_version(s))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutableKind {
    Codex,
    Custom,
}
pub fn safe_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_digit()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
}

pub enum ReadEvent {
    Started(u8),
    Initializing,
    Quota,
    Failed(ReadFailure),
    Finished(ReadDiagnostic),
    Retrying,
}
pub trait ReadObserver: Send {
    fn event(&mut self, event: ReadEvent) -> impl Future<Output = ()> + Send;
}
impl<F, Fut> ReadObserver for F
where
    F: FnMut(ReadEvent) -> Fut + Send,
    Fut: Future<Output = ()> + Send,
{
    fn event(&mut self, event: ReadEvent) -> impl Future<Output = ()> + Send {
        self(event)
    }
}

pub struct ObservedUsage {
    pub usage: RawCodexUsage,
    pub timing: ObservationTiming,
}
pub trait AttemptSource: Send {
    fn attempt(
        &mut self,
        deadline: Instant,
        attempt: u8,
        observer: &mut impl ReadObserver,
    ) -> impl Future<Output = Result<ObservedUsage, ReadFailure>> + Send;
}

pub async fn read_with_policy(
    source: &mut impl AttemptSource,
    observer: &mut impl ReadObserver,
    budget: Duration,
) -> Result<ObservedUsage, ReadFailure> {
    let deadline = Instant::now() + budget;
    timeout_at(deadline, async {
        for attempt in 1..=2 {
            observer.event(ReadEvent::Started(attempt)).await;
            let result = source.attempt(deadline, attempt, observer).await;
            match result {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let delay = Duration::from_secs(error.retry_after_seconds.unwrap_or(1).max(1));
                    if !should_retry(
                        &error,
                        attempt,
                        deadline.saturating_duration_since(Instant::now()),
                        delay,
                    ) {
                        return Err(error);
                    }
                    observer.event(ReadEvent::Retrying).await;
                    sleep(delay).await;
                }
            }
        }
        unreachable!()
    })
    .await
    .unwrap_or_else(|_| Err(ReadFailure::timeout(ReadStage::Refresh)))
}

fn should_retry(error: &ReadFailure, attempt: u8, remaining: Duration, delay: Duration) -> bool {
    attempt == 1
        && error.transient()
        && remaining
            >= delay
                .saturating_add(Duration::from_secs(10))
                .saturating_add(CLEANUP_BUDGET)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadHistory {
    schema_version: u32,
    pub last_success: Option<ReadDiagnostic>,
    pub last_failure: Option<ReadDiagnostic>,
    events: Vec<ReadDiagnostic>,
}
impl Default for ReadHistory {
    fn default() -> Self {
        Self {
            schema_version: 1,
            last_success: None,
            last_failure: None,
            events: Vec::new(),
        }
    }
}
impl ReadHistory {
    pub fn load(path: &std::path::Path) -> Self {
        use std::io::Read;
        let mut bytes = Vec::new();
        let history = std::fs::File::open(path).ok().and_then(|file| {
            file.take((HISTORY_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() > HISTORY_BYTES {
                return None;
            }
            serde_json::from_slice::<Self>(&bytes).ok()
        });
        let mut history = history
            .filter(|h| h.schema_version == 1)
            .unwrap_or_default();
        history.prune(unix_now());
        history
    }
    pub fn record(&mut self, event: ReadDiagnostic) {
        if event.success {
            self.last_success = Some(event.clone());
        } else {
            self.last_failure = Some(event.clone());
        }
        self.events.push(event);
        self.prune(unix_now());
    }
    fn prune(&mut self, now: i64) {
        self.events.retain(|event| event.valid(now));
        if self
            .last_success
            .as_ref()
            .is_some_and(|v| !v.valid(now) || !v.success)
        {
            self.last_success = None;
        }
        if self
            .last_failure
            .as_ref()
            .is_some_and(|v| !v.valid(now) || v.success)
        {
            self.last_failure = None;
        }
        if self.events.len() > HISTORY_EVENTS {
            self.events.drain(..self.events.len() - HISTORY_EVENTS);
        }
        while serde_json::to_vec(self).is_ok_and(|v| v.len() > HISTORY_BYTES)
            && !self.events.is_empty()
        {
            self.events.remove(0);
        }
    }
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut current = self.clone();
        current.prune(unix_now());
        let bytes = serde_json::to_vec(&current)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(tmp, path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderSnapshot {
    pub phase: String,
    pub attempt: u8,
    pub last_success: Option<ReadDiagnostic>,
    pub last_failure: Option<ReadDiagnostic>,
}
pub struct ReaderState {
    pub history: ReadHistory,
    pub phase: &'static str,
    pub attempt: u8,
    pub observation: Option<ObservationTiming>,
    pub failure: Option<ReadFailure>,
    pub revision: u64,
    pub writer_running: bool,
}
impl ReaderState {
    pub fn load(config: &crate::config::ConfigManager) -> Self {
        Self {
            history: ReadHistory::load(&config.quota_history_path()),
            phase: "idle",
            attempt: 0,
            observation: None,
            failure: None,
            revision: 0,
            writer_running: false,
        }
    }
    pub fn snapshot(&self) -> ReaderSnapshot {
        ReaderSnapshot {
            phase: self.phase.into(),
            attempt: self.attempt,
            last_success: self.history.last_success.clone(),
            last_failure: self.history.last_failure.clone(),
        }
    }
    pub fn record(&mut self, diagnostic: ReadDiagnostic) {
        self.history.record(diagnostic);
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn display_only(&self) -> bool {
        self.observation
            .is_some_and(|o| o.latency() > ACTION_MAX_AGE)
    }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests;
