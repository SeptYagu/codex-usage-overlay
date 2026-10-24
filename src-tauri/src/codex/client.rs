#[cfg(test)]
use anyhow::Context;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, Command};
#[cfg(test)]
use tokio::time::timeout;

use super::reader::{
    self, AttemptSource, FailureKind, ObservationTiming, ObservedUsage, ReadDiagnostic, ReadEvent,
    ReadFailure, ReadObserver, ReadStage, StderrSummary,
};
use tokio::time::{timeout_at, Instant};

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    pub used_percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub window_duration_mins: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RawCodexUsage {
    pub account_id: Option<String>,
    pub five_hour: QuotaSnapshot,
    pub weekly: QuotaSnapshot,
    pub credits_display: String,
    pub credits_balance: Option<String>,
    pub has_credits: bool,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsage {
    pub five_hour_remaining_percent: Option<u32>,
    pub five_hour_burn_rate_per_hour: Option<f64>,
    pub week_remaining_percent: Option<u32>,
    pub week_burn_rate_per_hour: Option<f64>,
    pub credits_display: String,
    pub credits_balance: Option<String>,
    pub has_credits: bool,
    pub five_hour_resets_at: Option<i64>,
    pub week_resets_at: Option<i64>,
    pub fetched_at: i64,
}

impl RawCodexUsage {
    pub fn to_ui(
        &self,
        five_hour_burn_rate_per_hour: Option<f64>,
        week_burn_rate_per_hour: Option<f64>,
    ) -> CodexUsage {
        CodexUsage {
            five_hour_remaining_percent: remaining_percent(self.five_hour.used_percent),
            five_hour_burn_rate_per_hour,
            week_remaining_percent: remaining_percent(self.weekly.used_percent),
            week_burn_rate_per_hour,
            credits_display: self.credits_display.clone(),
            credits_balance: self.credits_balance.clone(),
            has_credits: self.has_credits,
            five_hour_resets_at: self.five_hour.resets_at,
            week_resets_at: self.weekly.resets_at,
            fetched_at: self.fetched_at,
        }
    }
}

fn remaining_percent(used_percent: Option<f64>) -> Option<u32> {
    used_percent.map(|used| (100.0 - used).clamp(0.0, 100.0).round() as u32)
}

pub struct CodexClient {
    exe_path: Option<PathBuf>,
}

impl CodexClient {
    pub fn new() -> Self {
        Self { exe_path: None }
    }

    #[cfg(test)]
    fn spawn_with_recovery<T>(
        &mut self,
        mut find_executable: impl FnMut() -> Option<PathBuf>,
        mut spawn: impl FnMut(&Path) -> io::Result<T>,
    ) -> Result<T> {
        if !self.exe_path.as_ref().is_some_and(|p| p.is_file()) {
            self.exe_path = find_executable();
        }

        for attempt in 0..2 {
            let exe = self.exe_path.clone().ok_or_else(|| {
                anyhow!(
                    "找不到 Codex app-server。请确认已安装并登录 Codex，或设置 CODEX_CLI_PATH。"
                )
            })?;

            match spawn(&exe) {
                Ok(child) => return Ok(child),
                Err(error) => {
                    if error.kind() == io::ErrorKind::NotFound {
                        self.exe_path = None;
                        if attempt == 0 {
                            self.exe_path = find_executable();
                            continue;
                        }
                    }
                    return Err(error).with_context(|| format!("启动 Codex 进程失败: {:?}", exe));
                }
            }
        }

        unreachable!("each spawn attempt returns or retries once")
    }

    pub async fn fetch_usage(
        &mut self,
        observer: &mut impl ReadObserver,
    ) -> std::result::Result<ObservedUsage, ReadFailure> {
        reader::read_with_policy(self, observer, reader::REFRESH_BUDGET).await
    }
}

impl AttemptSource for CodexClient {
    async fn attempt(
        &mut self,
        deadline: Instant,
        attempt: u8,
        observer: &mut impl ReadObserver,
    ) -> std::result::Result<ObservedUsage, ReadFailure> {
        let started = Instant::now();
        let mut report = ReadDiagnostic::new(attempt);
        let result = async {
            if !self.exe_path.as_ref().is_some_and(|p| p.is_file()) {
                self.exe_path =
                    super::finder::find_codex_executable(deadline - reader::CLEANUP_BUDGET).await?;
            }
            for spawn_attempt in 0..2 {
                let exe = self.exe_path.as_ref().ok_or_else(|| {
                    ReadFailure::new(ReadStage::Discovery, FailureKind::MissingExecutable)
                })?;
                report.executable_kind = Some(
                    if exe
                        .file_name()
                        .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
                    {
                        reader::ExecutableKind::Codex
                    } else {
                        reader::ExecutableKind::Custom
                    },
                );
                report.executable_modified_at = exe
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|at| at.as_secs());
                // Check the shared deadline after discovery, before launching anything.
                if Instant::now() >= deadline - reader::CLEANUP_BUDGET {
                    return Err(ReadFailure::timeout(ReadStage::Discovery));
                }
                match usage_command(exe).arg("app-server").spawn() {
                    Ok(child) => {
                        let modified = report.executable_modified_at;
                        report.cli_version = cached_cli_version(exe, modified);
                        let mut version_probe = report
                            .cli_version
                            .is_none()
                            .then(|| start_version_probe(exe, deadline));
                        let result = attempt_child(child, deadline, observer, &mut report).await;
                        // Version metadata never delays the quota result.
                        if let Some(probe) = &mut version_probe {
                            if probe.0.is_finished() {
                                if let Ok(Some(version)) = (&mut probe.0).await {
                                    *CLI_VERSION.lock().unwrap_or_else(|p| p.into_inner()) =
                                        Some((exe.clone(), modified, version.clone()));
                                    report.cli_version = Some(version);
                                }
                            }
                        }
                        return result;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound && spawn_attempt == 0 => {
                        self.exe_path =
                            super::finder::find_codex_executable(deadline - reader::CLEANUP_BUDGET)
                                .await?;
                    }
                    Err(error) => return Err(ReadFailure::io(ReadStage::Spawn, &error)),
                }
            }
            unreachable!()
        }
        .await;
        // Protocol failures are reported before cleanup; discovery/spawn failures
        // reach the observer here, before the recovery delay.
        if report.failure.is_none() {
            if let Err(error) = &result {
                observer.event(ReadEvent::Failed(error.clone())).await;
            }
        }
        report.success = result.is_ok();
        report.failure = result.as_ref().err().cloned();
        report.elapsed_ms = reader::millis(started.elapsed());
        observer.event(ReadEvent::Finished(report)).await;
        result
    }
}

fn usage_command(exe: &Path) -> Command {
    let mut command = Command::new(exe);
    command
        .creation_flags(CREATE_NO_WINDOW)
        // Also covers cancellation and early errors before explicit cleanup.
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[cfg(test)]
async fn fetch_usage_from_child(child: Child, timeout_duration: Duration) -> Result<RawCodexUsage> {
    let mut observer = |_| async {};
    let mut report = ReadDiagnostic::new(1);
    attempt_child(
        child,
        Instant::now() + timeout_duration,
        &mut observer,
        &mut report,
    )
    .await
    .map(|value| value.usage)
    .map_err(Into::into)
}

async fn attempt_child(
    mut child: Child,
    deadline: Instant,
    observer: &mut impl ReadObserver,
    report: &mut ReadDiagnostic,
) -> std::result::Result<ObservedUsage, ReadFailure> {
    let stderr = std::sync::Arc::new(std::sync::Mutex::new(StderrSummary::default()));
    let mut drain = child.stderr.take().map(|stream| {
        let stderr = stderr.clone();
        DrainGuard(tokio::spawn(
            async move { drain_stderr(stream, stderr).await },
        ))
    });
    // For very short test budgets, still allow a request before Drop cleanup.
    let request_deadline = deadline
        .checked_sub(reader::CLEANUP_BUDGET)
        .filter(|d| *d > Instant::now())
        .unwrap_or(deadline);
    let request_future = async {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ReadFailure::new(ReadStage::Spawn, FailureKind::InvalidResponse))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ReadFailure::new(ReadStage::Spawn, FailureKind::InvalidResponse))?;
        let mut reader = BufReader::new(stdout).lines();

        request_usage_observed(&mut reader, &mut stdin, request_deadline, observer, report).await
    };

    let usage_result = match timeout_at(request_deadline, request_future).await {
        Ok(res) => res,
        Err(_) => Err(ReadFailure::timeout(ReadStage::Quota)),
    };

    if let Err(error) = &usage_result {
        report.failure = Some(error.clone());
        observer.event(ReadEvent::Failed(error.clone())).await;
    }

    // Only terminate the child owned by this read. Bound cleanup so a stuck
    // process cannot hold the refresh guard indefinitely; Drop retries the kill.
    report.cleanup_failed = !matches!(
        timeout_at(
            deadline.min(Instant::now() + reader::CLEANUP_BUDGET),
            child.kill()
        )
        .await,
        Ok(Ok(()))
    );
    if let Some(drain) = &mut drain {
        let _ = timeout_at(
            deadline.min(Instant::now() + Duration::from_millis(50)),
            &mut drain.0,
        )
        .await;
    }
    report.stderr = stderr.lock().unwrap_or_else(|p| p.into_inner()).clone();
    usage_result
}

struct DrainGuard(tokio::task::JoinHandle<()>);
impl Drop for DrainGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

type VersionIdentity = (PathBuf, Option<u64>, String);
static CLI_VERSION: std::sync::Mutex<Option<VersionIdentity>> = std::sync::Mutex::new(None);
fn cached_cli_version(exe: &Path, modified: Option<u64>) -> Option<String> {
    CLI_VERSION
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .filter(|(path, at, _)| path == exe && *at == modified)
        .map(|(_, _, version)| version.clone())
}
struct VersionProbe(tokio::task::JoinHandle<Option<String>>);
impl Drop for VersionProbe {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn start_version_probe(exe: &Path, deadline: Instant) -> VersionProbe {
    use tokio::io::AsyncReadExt;
    let exe = exe.to_owned();
    VersionProbe(tokio::spawn(async move {
        timeout_at(
            deadline.min(Instant::now() + Duration::from_secs(2)),
            async {
                let mut child = Command::new(exe)
                    .arg("--version")
                    .creation_flags(CREATE_NO_WINDOW)
                    .kill_on_drop(true)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok()?;
                let stdout = child.stdout.take()?;
                let mut bytes = Vec::new();
                stdout.take(256).read_to_end(&mut bytes).await.ok()?;
                let _ = child.kill().await;
                let text = std::str::from_utf8(&bytes).ok()?.trim();
                let version = text.strip_prefix("codex-cli ")?;
                reader::safe_version(version).then(|| version.to_owned())
            },
        )
        .await
        .ok()
        .flatten()
    }))
}

async fn drain_stderr(
    mut stream: impl tokio::io::AsyncRead + Unpin,
    summary: std::sync::Arc<std::sync::Mutex<StderrSummary>>,
) {
    use tokio::io::AsyncReadExt;
    let mut chunk = [0u8; 4096];
    let mut line = Vec::with_capacity(4096);
    let mut oversized = false;
    loop {
        let Ok(count) = stream.read(&mut chunk).await else {
            break;
        };
        if count == 0 {
            break;
        }
        for &byte in &chunk[..count] {
            if byte == b'\n' {
                let mut summary = summary.lock().unwrap_or_else(|p| p.into_inner());
                if oversized {
                    summary.oversized_lines = summary.oversized_lines.saturating_add(1);
                } else if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                    match (
                        value.get("target").and_then(Value::as_str),
                        value.pointer("/fields/message").and_then(Value::as_str),
                    ) {
                        (
                            Some("codex_models_manager::manager"),
                            Some("failed to refresh available models: request timed out"),
                        ) => summary.model_timeouts = summary.model_timeouts.saturating_add(1),
                        (
                            Some("codex_core_plugins::manager"),
                            Some(
                                "failed to load recommended plugins"
                                | "failed to warm featured plugin ids cache",
                            ),
                        ) => summary.plugin_failures = summary.plugin_failures.saturating_add(1),
                        _ => summary.other_lines = summary.other_lines.saturating_add(1),
                    }
                } else {
                    summary.other_lines = summary.other_lines.saturating_add(1);
                }
                line.clear();
                oversized = false;
            } else if !oversized {
                if line.len() < 4096 {
                    line.push(byte);
                } else {
                    line.clear();
                    oversized = true;
                }
            }
        }
    }
    if oversized || !line.is_empty() {
        let mut summary = summary.lock().unwrap_or_else(|p| p.into_inner());
        if oversized {
            summary.oversized_lines = summary.oversized_lines.saturating_add(1);
        } else {
            summary.other_lines = summary.other_lines.saturating_add(1);
        }
    }
}

async fn send_message<W: AsyncWrite + Unpin>(writer: &mut W, message: Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&message).map_err(io::Error::other)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_response<R: AsyncBufRead + Unpin>(
    reader: &mut Lines<R>,
    id: i64,
    method: &str,
) -> std::result::Result<Value, ReadFailure> {
    let stage = if method == "initialize" {
        ReadStage::Initialize
    } else {
        ReadStage::Quota
    };
    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|e| ReadFailure::io(stage, &e))?
    {
        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        // Notifications and server requests are not responses to our request.
        if message.get("method").is_some() || message.get("id").and_then(Value::as_i64) != Some(id)
        {
            continue;
        }
        if let Some(error) = message.get("error").filter(|value| !value.is_null()) {
            return Err(ReadFailure::rpc(stage, error));
        }
        return message
            .get("result")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| ReadFailure::new(stage, FailureKind::InvalidResponse));
    }
    Err(ReadFailure::new(stage, FailureKind::Transport))
}

#[cfg(test)]
async fn request_usage<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(
    reader: &mut Lines<R>,
    writer: &mut W,
) -> std::result::Result<RawCodexUsage, ReadFailure> {
    let mut observer = |_| async {};
    let mut report = ReadDiagnostic::new(1);
    request_usage_observed(
        reader,
        writer,
        Instant::now() + reader::REFRESH_BUDGET,
        &mut observer,
        &mut report,
    )
    .await
    .map(|v| v.usage)
}

async fn request_usage_observed<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(
    reader: &mut Lines<R>,
    writer: &mut W,
    deadline: Instant,
    observer: &mut impl ReadObserver,
    report: &mut ReadDiagnostic,
) -> std::result::Result<ObservedUsage, ReadFailure> {
    observer.event(ReadEvent::Initializing).await;
    let initialize_started = Instant::now();
    let initialize = async {
        send_message(
            writer,
            serde_json::json!({
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "codex-usage-overlay",
                        "title": "Codex Usage Overlay",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": { "experimentalApi": true }
                }
            }),
        )
        .await
        .map_err(|e| ReadFailure::io(ReadStage::Initialize, &e))?;
        read_response(reader, 1, "initialize").await
    };
    let initialize_result = timeout_at(
        deadline.min(initialize_started + reader::INITIALIZE_BUDGET),
        initialize,
    )
    .await;
    report.initialize_ms = Some(reader::millis(initialize_started.elapsed()));
    initialize_result.map_err(|_| ReadFailure::timeout(ReadStage::Initialize))??;
    send_message(
        writer,
        serde_json::json!({ "method": "initialized", "params": {} }),
    )
    .await
    .map_err(|e| ReadFailure::io(ReadStage::Initialize, &e))?;
    observer.event(ReadEvent::Quota).await;
    let quota_started = Instant::now();
    let quota = async {
        send_message(
        writer,
        serde_json::json!({
            "id": 2,
            "method": "account/rateLimits/read",
            "params": { "excludeResetCreditDetails": true, "supportsLunaReserveFallback": false }
        }),
    )
    .await.map_err(|e| ReadFailure::io(ReadStage::Quota, &e))?;
        read_response(reader, 2, "account/rateLimits/read").await
    };
    let result = timeout_at(deadline, quota)
        .await
        .map_err(|_| ReadFailure::timeout(ReadStage::Quota));
    report.quota_ms = Some(reader::millis(quota_started.elapsed()));
    let result = result??;
    let received = Instant::now();
    let usage = parse_usage_result(&result)
        .map_err(|_| ReadFailure::new(ReadStage::Parse, FailureKind::InvalidQuota))?;
    Ok(ObservedUsage {
        usage,
        timing: ObservationTiming {
            quota_started,
            received,
        },
    })
}

fn parse_quota(value: Option<&Value>) -> QuotaSnapshot {
    let Some(value) = value else {
        return QuotaSnapshot::default();
    };
    let used_percent = value
        .get("usedPercent")
        .and_then(Value::as_f64)
        .filter(|used| used.is_finite() && (0.0..=100.0).contains(used));
    let resets_at = value.get("resetsAt").and_then(Value::as_i64);
    let window_duration_mins = value
        .get("windowDurationMins")
        .and_then(Value::as_u64)
        .and_then(|mins| u32::try_from(mins).ok());
    QuotaSnapshot {
        used_percent,
        resets_at,
        window_duration_mins,
    }
}

fn extract_account_id(result: &Value, bucket: &Value) -> Option<String> {
    [
        result.get("accountId").and_then(Value::as_str),
        result
            .get("account")
            .and_then(|a| a.get("id"))
            .and_then(Value::as_str),
        result
            .get("account")
            .and_then(|a| a.get("accountId"))
            .and_then(Value::as_str),
        bucket.get("accountId").and_then(Value::as_str),
        bucket
            .get("account")
            .and_then(|a| a.get("id"))
            .and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.trim().is_empty())
    .map(ToOwned::to_owned)
}

fn parse_usage_result(result: &Value) -> Result<RawCodexUsage> {
    let bucket = result
        .get("rateLimitsByLimitId")
        .and_then(|by_limit| by_limit.get("codex"))
        .filter(|bucket| bucket.is_object())
        .or_else(|| {
            result.get("rateLimits").filter(|bucket| {
                bucket.is_object()
                    && bucket
                        .get("limitId")
                        .is_none_or(|id| id.is_null() || id.as_str() == Some("codex"))
            })
        });
    let bucket = bucket.ok_or_else(|| anyhow!("未在返回结果中找到 rateLimits 数据"))?;
    let five_hour = quota_for_window(bucket, 300, "primary");
    let weekly = quota_for_window(bucket, 10080, "secondary");

    let mut credits_display = "—".to_string();
    let mut credits_balance = None;
    let mut has_credits = false;

    if let Some(credits) = bucket.get("credits") {
        has_credits = credits
            .get("hasCredits")
            .and_then(|h| h.as_bool())
            .unwrap_or(false);
        if credits.get("unlimited").and_then(|u| u.as_bool()) == Some(true) {
            credits_display = "∞".to_string();
        } else if let Some(bal) = credits.get("balance") {
            let bal_str = match bal {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            };
            if !bal_str.is_empty() {
                credits_balance = Some(bal_str.clone());
                credits_display = bal_str;
            }
        }
    }

    let fetched_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    Ok(RawCodexUsage {
        account_id: extract_account_id(result, bucket),
        five_hour,
        weekly,
        credits_display,
        credits_balance,
        has_credits,
        fetched_at,
    })
}

fn quota_for_window(bucket: &Value, duration_mins: u32, legacy_key: &str) -> QuotaSnapshot {
    let value = [bucket.get("primary"), bucket.get("secondary")]
        .into_iter()
        .flatten()
        .find(|value| {
            value.get("windowDurationMins").and_then(Value::as_u64) == Some(duration_mins as u64)
        })
        .or_else(|| {
            bucket
                .get(legacy_key)
                .filter(|value| value.get("windowDurationMins").is_none_or(Value::is_null))
        });
    parse_quota(value)
}

#[cfg(test)]
mod transport_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn existing_executable() -> PathBuf {
        std::env::current_exe().unwrap()
    }

    fn replacement_executable() -> PathBuf {
        // Spawn is injected, so any existing file is a valid fixture.
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
    }

    #[test]
    fn valid_cached_executable_does_not_repeat_discovery() {
        let cached = existing_executable();
        let mut client = CodexClient {
            exe_path: Some(cached.clone()),
        };
        let spawned = client
            .spawn_with_recovery(
                || panic!("valid cached path should not trigger discovery"),
                |path| Ok(path.to_path_buf()),
            )
            .unwrap();
        assert_eq!(spawned, cached);
    }

    #[test]
    fn absent_or_missing_cached_executable_is_rediscovered() {
        let missing = existing_executable().join("missing-codex.exe");
        for cache in [None, Some(missing)] {
            let replacement = replacement_executable();
            let mut client = CodexClient { exe_path: cache };
            let mut lookups = 0;
            let spawned = client
                .spawn_with_recovery(
                    || {
                        lookups += 1;
                        Some(replacement.clone())
                    },
                    |path| Ok(path.to_path_buf()),
                )
                .unwrap();
            assert_eq!(lookups, 1);
            assert_eq!(spawned, replacement);
            assert_eq!(client.exe_path, Some(replacement));
        }
    }

    #[test]
    fn missing_replacement_leaves_cache_empty_for_later_polls() {
        let mut client = CodexClient {
            exe_path: Some(existing_executable().join("missing-codex.exe")),
        };
        let result: Result<()> = client.spawn_with_recovery(
            || None,
            |_| panic!("spawn should not run when discovery fails"),
        );
        assert!(result.is_err());
        assert!(client.exe_path.is_none());

        let replacement = replacement_executable();
        let spawned = client
            .spawn_with_recovery(|| Some(replacement.clone()), |path| Ok(path.to_path_buf()))
            .unwrap();
        assert_eq!(spawned, replacement);
    }

    #[test]
    fn executable_disappearing_during_spawn_is_retried_with_replacement() {
        let cached = existing_executable();
        let replacement = replacement_executable();
        let mut client = CodexClient {
            exe_path: Some(cached.clone()),
        };
        let mut lookups = 0;
        let mut attempts = 0;
        let spawned = client
            .spawn_with_recovery(
                || {
                    lookups += 1;
                    Some(replacement.clone())
                },
                |path| {
                    attempts += 1;
                    if attempts == 1 {
                        assert_eq!(path, cached);
                        Err(io::Error::from(io::ErrorKind::NotFound))
                    } else {
                        Ok(path.to_path_buf())
                    }
                },
            )
            .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(lookups, 1);
        assert_eq!(spawned, replacement);
        assert_eq!(client.exe_path, Some(replacement));
    }

    #[test]
    fn spawn_time_disappearance_without_replacement_clears_cache() {
        let mut client = CodexClient {
            exe_path: Some(existing_executable()),
        };
        let mut attempts = 0;
        let result: Result<()> = client.spawn_with_recovery(
            || None,
            |_| {
                attempts += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
        );
        assert!(result.is_err());
        assert_eq!(attempts, 1);
        assert!(client.exe_path.is_none());
    }

    #[test]
    fn spawn_retries_once_and_clears_a_second_missing_path() {
        let mut client = CodexClient {
            exe_path: Some(existing_executable()),
        };
        let mut attempts = 0;
        let mut lookups = 0;
        let error = client
            .spawn_with_recovery::<()>(
                || {
                    lookups += 1;
                    Some(replacement_executable())
                },
                |_| {
                    attempts += 1;
                    Err(io::Error::from(io::ErrorKind::NotFound))
                },
            )
            .unwrap_err();
        assert_eq!(attempts, 2);
        assert_eq!(lookups, 1);
        assert!(client.exe_path.is_none());
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn unrelated_spawn_errors_are_not_retried() {
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::InvalidInput] {
            let cached = existing_executable();
            let mut client = CodexClient {
                exe_path: Some(cached.clone()),
            };
            let mut attempts = 0;
            let error = client
                .spawn_with_recovery::<()>(
                    || panic!("unrelated errors should not trigger discovery"),
                    |_| {
                        attempts += 1;
                        Err(io::Error::from(kind))
                    },
                )
                .unwrap_err();
            assert_eq!(attempts, 1);
            assert_eq!(client.exe_path, Some(cached));
            assert_eq!(error.downcast_ref::<io::Error>().unwrap().kind(), kind);
        }
    }

    #[test]
    fn parsing_preserves_raw_precision_metadata_and_account_identity() {
        let mock_result = json!({
            "account": { "id": "acct-123" },
            "rateLimitsByLimitId": {
                "codex": {
                    "primary": {
                        "usedPercent": 35.25,
                        "resetsAt": 1790536584,
                        "windowDurationMins": 300
                    },
                    "secondary": {
                        "usedPercent": 80,
                        "resetsAt": 1791077732,
                        "windowDurationMins": 10080
                    },
                    "credits": {
                        "hasCredits": true,
                        "balance": "12.50",
                        "unlimited": false
                    }
                }
            }
        });

        let parsed = parse_usage_result(&mock_result).expect("Should parse successfully");
        assert_eq!(parsed.account_id.as_deref(), Some("acct-123"));
        assert_eq!(parsed.five_hour.used_percent, Some(35.25));
        assert_eq!(parsed.weekly.used_percent, Some(80.0));
        assert_eq!(parsed.five_hour.window_duration_mins, Some(300));
        assert_eq!(parsed.weekly.window_duration_mins, Some(10080));
        assert_eq!(parsed.five_hour.resets_at, Some(1790536584));
        assert_eq!(parsed.weekly.resets_at, Some(1791077732));
        let ui = parsed.to_ui(None, None);
        assert_eq!(ui.five_hour_remaining_percent, Some(65));
        assert_eq!(ui.week_remaining_percent, Some(20));
        assert_eq!(ui.credits_display, "12.50");
        assert_eq!(ui.credits_balance, Some("12.50".to_string()));
    }

    #[test]
    fn invalid_quota_values_are_rejected_independently_and_credits_survive() {
        let mock_result = json!({
            "rateLimits": {
                "primary": { "usedPercent": 101.0, "resetsAt": 1790536584 },
                "secondary": { "usedPercent": -1.0, "resetsAt": 1791077732 },
                "credits": { "hasCredits": true, "unlimited": true }
            }
        });

        let parsed = parse_usage_result(&mock_result).expect("Should parse successfully");
        assert_eq!(parsed.five_hour.used_percent, None);
        assert_eq!(parsed.weekly.used_percent, None);
        assert_eq!(parsed.credits_display, "∞");
        assert!(parsed.has_credits);
        let ui = parsed.to_ui(None, None);
        assert_eq!(ui.five_hour_remaining_percent, None);
        assert_eq!(ui.week_remaining_percent, None);
    }

    #[test]
    fn missing_quota_and_account_fields_are_safe() {
        let parsed = parse_usage_result(&json!({
            "rateLimits": {
                "credits": { "hasCredits": false, "balance": "0" }
            }
        }))
        .unwrap();
        assert_eq!(parsed.account_id, None);
        assert_eq!(parsed.five_hour, QuotaSnapshot::default());
        assert_eq!(parsed.weekly, QuotaSnapshot::default());
        assert_eq!(parsed.credits_display, "0");
    }

    #[test]
    fn null_multi_bucket_entry_falls_back_to_valid_legacy_codex_bucket() {
        let parsed = parse_usage_result(&json!({
            "rateLimitsByLimitId": { "codex": null },
            "rateLimits": { "limitId": "codex", "primary": { "usedPercent": 97.5 } }
        }))
        .unwrap();
        assert_eq!(parsed.five_hour.used_percent, Some(97.5));
    }

    #[test]
    fn malformed_or_unrelated_buckets_are_not_successful_codex_reads() {
        for result in [
            json!({ "rateLimits": null }),
            json!({ "rateLimits": [] }),
            json!({ "rateLimitsByLimitId": { "codex": false } }),
            json!({ "rateLimitsByLimitId": { "codex_other": {} },
                "rateLimits": { "limitId": "codex_other", "primary": { "usedPercent": 1 } } }),
        ] {
            assert!(parse_usage_result(&result).is_err(), "{result}");
        }
    }

    #[test]
    fn explicit_codex_bucket_takes_priority_over_other_legacy_bucket() {
        let parsed = parse_usage_result(&json!({
            "rateLimitsByLimitId": { "codex": { "primary": { "usedPercent": 99 } } },
            "rateLimits": { "limitId": "codex_other", "primary": { "usedPercent": 1 } }
        }))
        .unwrap();
        assert_eq!(parsed.five_hour.used_percent, Some(99.0));
    }

    #[test]
    fn quota_windows_are_identified_by_duration_even_when_swapped() {
        let parsed = parse_usage_result(&json!({ "rateLimits": {
            "primary": { "usedPercent": 99.75, "windowDurationMins": 10080, "resetsAt": 2000 },
            "secondary": { "usedPercent": 95.25, "windowDurationMins": 300, "resetsAt": 1000 }
        }}))
        .unwrap();
        assert_eq!(parsed.five_hour.used_percent, Some(95.25));
        assert_eq!(parsed.five_hour.resets_at, Some(1000));
        assert_eq!(parsed.weekly.used_percent, Some(99.75));
        assert_eq!(parsed.weekly.resets_at, Some(2000));
    }

    #[test]
    fn unsupported_or_invalid_durations_do_not_masquerade_as_five_hour_quota() {
        for duration in [json!(15), json!(0), json!(-300), json!("300"), json!(300.5)] {
            let parsed = parse_usage_result(&json!({ "rateLimits": {
                "primary": { "usedPercent": 99, "windowDurationMins": duration, "resetsAt": 1000 },
                "secondary": { "usedPercent": 98, "windowDurationMins": 10080 },
                "credits": { "hasCredits": true, "balance": "12.50" }
            }}))
            .unwrap();
            assert_eq!(parsed.five_hour, QuotaSnapshot::default());
            assert_eq!(parsed.weekly.used_percent, Some(98.0));
            assert_eq!(parsed.credits_display, "12.50");
        }
    }

    #[test]
    fn null_duration_keeps_backward_compatible_positional_mapping() {
        let parsed = parse_usage_result(&json!({ "rateLimits": {
            "primary": { "usedPercent": 99.5, "windowDurationMins": null },
            "secondary": { "usedPercent": 98.5 }
        }}))
        .unwrap();
        assert_eq!(parsed.five_hour.used_percent, Some(99.5));
        assert_eq!(parsed.weekly.used_percent, Some(98.5));
    }
}
