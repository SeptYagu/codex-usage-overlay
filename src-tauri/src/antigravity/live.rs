//! Live reading of Antigravity quota pools via the official `agy` CLI.
//!
//! Runs `agy -p "/usage" --output-format json` (and `/credits` when credits are
//! displayed), which executes locally with zero turns and zero model tokens
//! while sharing the logged-in Antigravity Desktop / CLI account.
use serde_json::Value;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::shared::{self, Snapshot};
use crate::codex::reader::{FailureKind, ReadFailure, ReadStage};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
/// Caps `agy` invocations when protection or UI polls more frequently.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Cached outcome between background polls.
#[derive(Debug, Default)]
pub struct LiveState {
    pub answer: Option<(Instant, Snapshot)>,
    pub failure: Option<ReadFailure>,
    next_request: Option<Instant>,
}

#[derive(Debug, PartialEq)]
pub enum LiveStep {
    Answer(Snapshot),
    Failure(ReadFailure),
    Request,
}

impl LiveState {
    pub fn step(&self, now: Instant) -> LiveStep {
        if self.next_request.is_none_or(|at| now >= at) {
            return LiveStep::Request;
        }
        match (&self.failure, &self.answer) {
            (Some(failure), _) => LiveStep::Failure(failure.clone()),
            (None, Some((_, snapshot))) => LiveStep::Answer(snapshot.clone()),
            (None, None) => LiveStep::Request,
        }
    }

    pub fn record(&mut self, now: Instant, result: &Result<Snapshot, ReadFailure>) {
        let wait = match result {
            Ok(snapshot) => {
                self.answer = Some((now, snapshot.clone()));
                self.failure = None;
                MIN_INTERVAL
            }
            Err(failure) => {
                self.failure = Some(failure.clone());
                failure
                    .retry_after_seconds
                    .map_or(MIN_INTERVAL, Duration::from_secs)
                    .max(MIN_INTERVAL)
            }
        };
        self.next_request = Some(now + wait);
    }
}

/// Locates the `agy` CLI executable on `PATH` or standard Windows/user install paths.
pub fn cli_path() -> Option<PathBuf> {
    let on_path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();

    #[cfg(windows)]
    let candidates = {
        let mut extra = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            extra.push(local.join("Microsoft").join("WinGet").join("Links").join("agy.exe"));
            extra.push(local.join("Programs").join("Antigravity").join("bin").join("agy.exe"));
            extra.push(local.join("Programs").join("Antigravity").join("bin").join("agy.cmd"));
        }
        if let Some(home) = dirs::home_dir() {
            extra.push(home.join(".local").join("bin").join("agy.exe"));
            extra.push(home.join(".local").join("bin").join("agy.cmd"));
        }
        on_path
            .iter()
            .flat_map(|dir| [dir.join("agy.exe"), dir.join("agy.cmd")])
            .chain(extra)
            .collect::<Vec<_>>()
    };

    #[cfg(not(windows))]
    let candidates = {
        let home_bin = dirs::home_dir().map(|home| home.join(".local").join("bin").join("agy"));
        on_path
            .iter()
            .map(|dir| dir.join("agy"))
            .chain(home_bin)
            .collect::<Vec<_>>()
    };

    candidates.into_iter().find(|path| path.is_file())
}

pub fn cli_available() -> bool {
    cli_path().is_some()
}

fn looks_like_auth_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "not logged in",
        "login required",
        "sign in",
        "signed out",
        "unauthenticated",
        "unauthorized",
        "authentication required",
        "please authenticate",
        "run `agy`",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn parse_json_output(bytes: &[u8]) -> Option<Value> {
    let raw = std::str::from_utf8(bytes).ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return Some(value);
    }
    // Tolerate leading log lines emitted before the final JSON payload.
    raw.lines()
        .rev()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
}

async fn run_cli_command(cli: &Path, prompt: &str) -> Result<Value, ReadFailure> {
    let mut cmd = tokio::process::Command::new(cli);
    cmd.args(["-p", prompt, "--output-format", "json"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let child = cmd
        .spawn()
        .map_err(|error| ReadFailure::io(ReadStage::Spawn, &error))?;
    let output = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ReadFailure::timeout(ReadStage::Quota))?
        .map_err(|error| ReadFailure::io(ReadStage::Quota, &error))?;

    let stdout_json = parse_json_output(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout_text = String::from_utf8_lossy(&output.stdout);

    if looks_like_auth_error(&stderr) || looks_like_auth_error(&stdout_text) {
        return Err(ReadFailure::new(ReadStage::Quota, FailureKind::Authentication));
    }
    if !output.status.success() {
        return Err(ReadFailure::new(ReadStage::Quota, FailureKind::Rpc));
    }
    stdout_json.ok_or_else(|| ReadFailure::new(ReadStage::Parse, FailureKind::InvalidResponse))
}

async fn fetch_with<F, Fut>(now: i64, include_credits: bool, mut run: F) -> Result<Snapshot, ReadFailure>
where
    F: FnMut(&'static str) -> Fut,
    Fut: Future<Output = Result<Value, ReadFailure>>,
{
    let usage_json = run("/usage").await?;
    let mut snapshot = shared::parse_usage_command(&usage_json, now)
        .ok_or_else(|| ReadFailure::new(ReadStage::Parse, FailureKind::InvalidResponse))?;

    if include_credits {
        if let Ok(credits_json) = run("/credits").await {
            if let Some((display, balance)) = shared::parse_credits_command(&credits_json) {
                snapshot.has_credits = true;
                snapshot.credits_display = display;
                snapshot.credits_balance = balance;
            }
        }
    }
    Ok(snapshot)
}

/// Fetches the current Antigravity quota snapshot via `agy -p "/usage"`
/// (and optionally `/credits`), using zero model tokens.
pub async fn fetch(now: i64, include_credits: bool) -> Result<Snapshot, ReadFailure> {
    let cli = cli_path().ok_or_else(|| ReadFailure::new(ReadStage::Discovery, FailureKind::MissingExecutable))?;
    fetch_with(now, include_credits, |prompt| {
        let cli = cli.clone();
        async move { run_cli_command(&cli, prompt).await }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::antigravity::shared::{PoolWindows, WindowSnapshot};
    use serde_json::json;

    #[tokio::test]
    async fn fetch_with_combines_usage_and_optional_credits_and_ignores_credits_failure() {
        let now = 1_793_000_000;
        let usage = json!({
            "num_turns": 0,
            "command": {
                "name": "usage",
                "data": {
                    "groups": [{
                        "name": "Gemini Models",
                        "buckets": [{ "id": "gemini-5h", "remaining_fraction": 0.9, "reset_time": "2026-11-14T03:55:49Z" }]
                    }]
                }
            }
        });
        let credits = json!({
            "num_turns": 0,
            "command": { "name": "credits", "data": { "remaining_credits": 150 } }
        });

        let ok = fetch_with(now, true, |prompt| {
            let usage = usage.clone();
            let credits = credits.clone();
            async move { Ok(if prompt == "/usage" { usage } else { credits }) }
        })
        .await
        .unwrap();
        assert!((ok.gemini.five_hour.unwrap().used_percent - 10.0).abs() < 1e-6);
        assert!(ok.has_credits);
        assert_eq!(ok.credits_display.as_deref(), Some("150"));

        let degraded = fetch_with(now, true, |prompt| {
            let usage = usage.clone();
            async move {
                if prompt == "/usage" {
                    Ok(usage)
                } else {
                    Err(ReadFailure::timeout(ReadStage::Quota))
                }
            }
        })
        .await
        .unwrap();
        assert!(!degraded.has_credits);
        assert!((degraded.gemini.five_hour.unwrap().used_percent - 10.0).abs() < 1e-6);
    }

    #[test]
    fn live_state_throttles_successes_and_failures() {
        let t0 = Instant::now();
        let mut state = LiveState::default();
        assert_eq!(state.step(t0), LiveStep::Request);

        let snap = Snapshot {
            schema_version: shared::SCHEMA_VERSION,
            gemini: PoolWindows {
                five_hour: Some(WindowSnapshot { used_percent: 12.0, resets_at: None }),
                weekly: None,
            },
            argon: PoolWindows::default(),
            third_party: PoolWindows::default(),
            active_model_id: None,
            active_model_name: None,
            credits_display: None,
            credits_balance: None,
            has_credits: false,
            captured_at: 100,
            account_id: None,
            cli_version: None,
        };
        state.record(t0, &Ok(snap.clone()));
        assert_eq!(state.step(t0 + Duration::from_secs(30)), LiveStep::Answer(snap));
        assert_eq!(state.step(t0 + MIN_INTERVAL), LiveStep::Request);

        let fail = ReadFailure::new(ReadStage::Quota, FailureKind::Authentication);
        state.record(t0 + MIN_INTERVAL, &Err(fail.clone()));
        assert_eq!(state.step(t0 + MIN_INTERVAL + Duration::from_secs(10)), LiveStep::Failure(fail));
        assert_eq!(state.step(t0 + MIN_INTERVAL + MIN_INTERVAL), LiveStep::Request);
    }
}
