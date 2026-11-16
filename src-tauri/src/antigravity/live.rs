//! Live reading of Antigravity quota pools via the official `agy` CLI.
//!
//! Runs `agy -p "/usage" --output-format json` (and `/credits` when credits are
//! displayed), which executes locally with zero turns and zero model tokens
//! while sharing the logged-in Antigravity Desktop / CLI account.
use serde_json::Value;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use super::shared::{self, Snapshot};
use crate::codex::reader::{FailureKind, ReadFailure, ReadStage};
#[cfg(test)]
pub use crate::live_poll::MIN_INTERVAL;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub type LiveState = crate::live_poll::LiveState<Snapshot>;
pub type LiveStep = crate::live_poll::LiveStep<Snapshot>;

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

fn cli_command(cli: &Path, prompt: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(cli);
    cmd.args(["-p", prompt, "--output-format", "json"])
        .current_dir(std::env::temp_dir())
        // agy's self-updater re-launches itself with DETACHED_PROCESS and then
        // runs `agy --version` without hiding it, which escapes CREATE_NO_WINDOW
        // and opens a console. Quota polls never need to update the CLI;
        // interactive `agy` sessions keep updating normally.
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Terminates every process a CLI poll started when the poll times out or is
/// cancelled. `kill_on_drop` only reaches the direct child, which for an
/// `agy.cmd` shim is `cmd.exe` rather than `agy.exe`. A poll that finishes
/// normally disarms the guard, leaving any descendants alone.
#[cfg(windows)]
struct ProcessTreeGuard {
    job: windows::Win32::Foundation::HANDLE,
    armed: bool,
}

// A job object handle is a kernel handle owned solely by this guard; Win32
// allows it to be used and closed from any thread.
#[cfg(windows)]
unsafe impl Send for ProcessTreeGuard {}

#[cfg(windows)]
impl ProcessTreeGuard {
    fn attach(child: &tokio::process::Child) -> Option<Self> {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};

        let process = child.raw_handle()?;
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.ok()?;
        let guard = Self { job, armed: true };
        // Best effort: a failed assignment leaves kill_on_drop as the fallback.
        unsafe { AssignProcessToJobObject(job, HANDLE(process)) }.ok()?;
        Some(guard)
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(windows)]
impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::JobObjects::TerminateJobObject;
        unsafe {
            if self.armed {
                let _ = TerminateJobObject(self.job, 1);
            }
            let _ = CloseHandle(self.job);
        }
    }
}

async fn run_cli_command(cli: &Path, prompt: &str) -> Result<Value, ReadFailure> {
    let child = cli_command(cli, prompt)
        .spawn()
        .map_err(|error| ReadFailure::io(ReadStage::Spawn, &error))?;
    #[cfg(windows)]
    let mut tree = ProcessTreeGuard::attach(&child);
    let output = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ReadFailure::timeout(ReadStage::Quota))?
        .map_err(|error| ReadFailure::io(ReadStage::Quota, &error))?;
    #[cfg(windows)]
    if let Some(tree) = tree.as_mut() {
        tree.disarm();
    }

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

    #[test]
    fn cli_command_disables_agy_self_update() {
        let cmd = cli_command(Path::new("agy"), "/usage");
        let disabled = cmd.as_std().get_envs().any(|(key, value)| {
            key == "AGY_CLI_DISABLE_AUTO_UPDATE" && value == Some(std::ffi::OsStr::new("true"))
        });
        assert!(disabled);
    }
}
