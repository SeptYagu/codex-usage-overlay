//! Opt-in live reading of Claude subscription limits.
//!
//! The statusline bridge only receives limits from the terminal `claude` CLI;
//! the desktop app and IDE extensions never run a statusline. Live reading
//! asks the usage endpoint behind Claude Code's `/usage` command with the
//! OAuth token Claude Code stores in `.credentials.json`. The token is read
//! for each request and never stored, logged or sent anywhere but that
//! endpoint. CLI logins expire after a few hours; the overlay then runs a
//! Claude Code command that renews the login and never writes the credentials.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::shared::{self, Snapshot, WindowSnapshot};
use crate::codex::reader::{FailureKind, ReadFailure, ReadStage};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const TIMEOUT: Duration = Duration::from_secs(20);
/// The endpoint is asked at most this often, also while protection polls
/// every 15 seconds near a threshold.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_RETRY_AFTER_SECS: u64 = 120;
/// Claude Code is asked to renew its login at most this often, so a signed
/// out CLI is not started every minute.
const RENEW_INTERVAL: Duration = Duration::from_secs(10 * 60);
const RENEW_TIMEOUT: Duration = Duration::from_secs(30);
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// What live reading remembers between polls.
#[derive(Debug, Default)]
pub struct LiveState {
    /// The last answer and when it arrived.
    pub answer: Option<(Instant, Snapshot)>,
    /// The last failed request, kept until a request succeeds.
    pub failure: Option<ReadFailure>,
    /// No request is made before this instant: the minimum interval after an
    /// answer, the server's `Retry-After` after a refusal.
    next_request: Option<Instant>,
    last_renewal: Option<Instant>,
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
                failure.retry_after_seconds.map_or(MIN_INTERVAL, Duration::from_secs).max(MIN_INTERVAL)
            }
        };
        self.next_request = Some(now + wait);
    }

    /// Takes the renewal slot when an expired or refused login may be renewed.
    pub fn take_renewal(&mut self, now: Instant, failure: &ReadFailure) -> bool {
        let due = failure.kind == FailureKind::Authentication
            && self.last_renewal.is_none_or(|at| now.saturating_duration_since(at) >= RENEW_INTERVAL);
        if due {
            self.last_renewal = Some(now);
        }
        due
    }
}

/// The `claude` CLI: the native installer's `claude.exe` on PATH or in
/// `~/.local/bin`, else the batch shim an npm install puts on PATH.
fn claude_cli() -> Option<PathBuf> {
    let on_path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    let native = dirs::home_dir().map(|home| home.join(".local").join("bin"));
    on_path.iter().chain(native.iter())
        .map(|dir| dir.join("claude.exe"))
        .chain(on_path.iter().map(|dir| dir.join("claude.cmd")))
        .find(|candidate| candidate.is_file())
}

/// Claude Code renews an expired login when a command needs the account, not
/// on every start: `claude auth status` only reports the stored login. Listing
/// plugins syncs the claude.ai marketplaces and the doctor checks the account;
/// neither starts a session or uses quota. They are tried in turn until the
/// stored login is valid again, so one CLI change does not stop renewal.
const RENEW_COMMANDS: [&[&str]; 2] = [&["plugin", "list"], &["doctor"]];

/// Asks Claude Code to renew its login. Returns whether the stored login is
/// valid afterwards.
pub async fn renew_login() -> bool {
    let Some(cli) = claude_cli() else { return false };
    let Some(credentials) = credentials_path() else { return false };
    let home = dirs::home_dir();
    renew_with(&credentials, |args| {
        let mut command = tokio::process::Command::new(&cli);
        command
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // The doctor reads project settings in its working directory.
        if let Some(home) = &home {
            command.current_dir(home);
        }
        async move {
            let _ = tokio::time::timeout(RENEW_TIMEOUT, command.status()).await;
        }
    })
    .await
}

async fn renew_with<F, Fut>(credentials: &Path, mut run: F) -> bool
where
    F: FnMut(&'static [&'static str]) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    for args in RENEW_COMMANDS {
        run(args).await;
        if access_token(credentials, crate::codex::reader::unix_now()).is_ok() {
            return true;
        }
    }
    false
}

/// Claude Code's credentials file, honoring `CLAUDE_CONFIG_DIR`.
pub fn credentials_path() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
        .map(|dir| dir.join(".credentials.json"))
}

fn sign_in_required(stage: ReadStage) -> ReadFailure {
    ReadFailure::new(stage, FailureKind::Authentication)
}

/// The access token, if Claude Code is signed in with a subscription.
/// An `expiresAt` of zero means no expiry was recorded; the endpoint decides.
pub fn access_token(path: &Path, now: i64) -> Result<String, ReadFailure> {
    let raw = std::fs::read_to_string(path).map_err(|_| sign_in_required(ReadStage::Discovery))?;
    let credentials: Value = serde_json::from_str(&raw).map_err(|_| sign_in_required(ReadStage::Discovery))?;
    let oauth = credentials.get("claudeAiOauth").ok_or_else(|| sign_in_required(ReadStage::Discovery))?;
    let expires_at = oauth.get("expiresAt").and_then(Value::as_i64).unwrap_or(0);
    let expires_at = if expires_at > 100_000_000_000 { expires_at / 1000 } else { expires_at };
    if expires_at > 0 && expires_at <= now {
        return Err(sign_in_required(ReadStage::Discovery));
    }
    oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| sign_in_required(ReadStage::Discovery))
}

/// Maps the endpoint's `five_hour` / `seven_day` windows onto a snapshot.
pub fn parse_usage(body: &Value, now: i64) -> Option<Snapshot> {
    let window = |key: &str| {
        let window = body.get(key)?;
        let used_percent = window
            .get("utilization")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)?
            .min(100.0);
        let resets_at = window.get("resets_at").and_then(shared::parse_timestamp);
        Some(WindowSnapshot { used_percent, resets_at })
    };
    let five_hour = window("five_hour");
    let seven_day = window("seven_day");
    (five_hour.is_some() || seven_day.is_some()).then_some(Snapshot {
        schema_version: shared::SCHEMA_VERSION,
        five_hour,
        seven_day,
        captured_at: now,
        account_id: None,
        claude_version: None,
    })
}

fn status_failure(status: u16, retry_after: Option<u64>) -> ReadFailure {
    let mut failure = if matches!(status, 401 | 403) {
        sign_in_required(ReadStage::Quota)
    } else {
        ReadFailure::new(ReadStage::Quota, FailureKind::Rpc)
    };
    failure.http_status = Some(status);
    if status == 429 {
        failure.retry_after_seconds = Some(retry_after.unwrap_or(DEFAULT_RETRY_AFTER_SECS).max(MIN_INTERVAL.as_secs()));
    }
    failure
}

pub async fn fetch(now: i64) -> Result<Snapshot, ReadFailure> {
    let account_before = shared::account_identity();
    let path = credentials_path().ok_or_else(|| sign_in_required(ReadStage::Discovery))?;
    let token = access_token(&path, now)?;
    let client = crate::update_channel::http_client(TIMEOUT)
        .map_err(|_| ReadFailure::new(ReadStage::Spawn, FailureKind::Transport))?;
    let response = client
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", OAUTH_BETA)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|error| {
            ReadFailure::new(ReadStage::Quota, if error.is_timeout() { FailureKind::Timeout } else { FailureKind::Transport })
        })?;
    let status = response.status();
    if !status.is_success() {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        return Err(status_failure(status.as_u16(), retry_after));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| ReadFailure::new(ReadStage::Parse, FailureKind::InvalidResponse))?;
    let mut snapshot = parse_usage(&body, now).ok_or_else(|| ReadFailure::new(ReadStage::Parse, FailureKind::InvalidQuota))?;
    let account_after = shared::account_identity();
    if account_before == account_after { snapshot.account_id = account_after; }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_793_000_000;

    fn credentials(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("claude-live-{name}-{}.json", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn the_token_is_read_only_from_a_subscription_login() {
        let ok = credentials("ok", r#"{"claudeAiOauth":{"accessToken":"t-1","expiresAt":0,"subscriptionType":"pro"}}"#);
        assert_eq!(access_token(&ok, NOW).unwrap(), "t-1");
        let future_ms = credentials("future", &format!(r#"{{"claudeAiOauth":{{"accessToken":"t-2","expiresAt":{}}}}}"#, (NOW + 60) * 1000));
        assert_eq!(access_token(&future_ms, NOW).unwrap(), "t-2");
        let expired = credentials("expired", &format!(r#"{{"claudeAiOauth":{{"accessToken":"t-3","expiresAt":{}}}}}"#, NOW - 1));
        assert_eq!(access_token(&expired, NOW).unwrap_err().kind, FailureKind::Authentication);
        let api_key = credentials("apikey", r#"{"primaryApiKey":"k"}"#);
        assert_eq!(access_token(&api_key, NOW).unwrap_err().kind, FailureKind::Authentication);
        let empty = credentials("empty", r#"{"claudeAiOauth":{"accessToken":" "}}"#);
        assert!(access_token(&empty, NOW).is_err());
        assert_eq!(access_token(Path::new("Z:/missing/.credentials.json"), NOW).unwrap_err().kind, FailureKind::Authentication);
        for path in [ok, future_ms, expired, api_key, empty] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn the_usage_response_maps_onto_a_snapshot() {
        let body = json!({
            "five_hour": { "utilization": 8.0, "resets_at": "2026-11-01T12:00:00.123456+00:00" },
            "seven_day": { "utilization": 41.5, "resets_at": null },
            "seven_day_opus": null,
            "extra_usage": { "is_enabled": false }
        });
        let snapshot = parse_usage(&body, NOW).unwrap();
        assert_eq!(snapshot.five_hour, Some(WindowSnapshot { used_percent: 8.0, resets_at: Some(1_793_534_400) }));
        assert_eq!(snapshot.seven_day, Some(WindowSnapshot { used_percent: 41.5, resets_at: None }));
        assert_eq!(snapshot.captured_at, NOW);
        // Over-limit usage is reported as fully used.
        let over = parse_usage(&json!({ "five_hour": { "utilization": 104.0 } }), NOW).unwrap();
        assert_eq!(over.five_hour.unwrap().used_percent, 100.0);
        assert_eq!(parse_usage(&json!({ "error": "x" }), NOW), None);
        assert_eq!(parse_usage(&json!({ "five_hour": { "utilization": -1 } }), NOW), None);
    }

    #[test]
    fn http_failures_map_onto_read_failures() {
        assert_eq!(status_failure(401, None).kind, FailureKind::Authentication);
        assert_eq!(status_failure(403, None).http_status, Some(403));
        let limited = status_failure(429, Some(0));
        assert!(limited.transient());
        assert_eq!(limited.retry_after_seconds, Some(60));
        assert_eq!(status_failure(429, Some(600)).retry_after_seconds, Some(600));
        assert_eq!(status_failure(429, None).retry_after_seconds, Some(120));
        let server = status_failure(503, None);
        assert!(server.transient() && server.retry_after_seconds.is_none());
    }

    fn snapshot(used: f64) -> Snapshot {
        Snapshot {
            schema_version: shared::SCHEMA_VERSION,
            five_hour: Some(WindowSnapshot { used_percent: used, resets_at: None }),
            seven_day: None,
            captured_at: NOW,
            account_id: None,
            claude_version: None,
        }
    }

    #[test]
    fn answers_are_reused_and_refusals_wait_for_their_retry_time() {
        let start = Instant::now();
        let mut state = LiveState::default();
        assert_eq!(state.step(start), LiveStep::Request);

        state.record(start, &Ok(snapshot(10.0)));
        assert_eq!(state.step(start + Duration::from_secs(59)), LiveStep::Answer(snapshot(10.0)));
        assert_eq!(state.step(start + MIN_INTERVAL), LiveStep::Request);

        let refused = status_failure(429, Some(300));
        let later = start + MIN_INTERVAL;
        state.record(later, &Err(refused.clone()));
        assert_eq!(state.step(later + Duration::from_secs(299)), LiveStep::Failure(refused));
        assert_eq!(state.step(later + Duration::from_secs(300)), LiveStep::Request);
        // The failure stays until a request succeeds.
        assert!(state.failure.is_some());
        state.record(later + Duration::from_secs(300), &Ok(snapshot(12.0)));
        assert!(state.failure.is_none());
    }

    #[test]
    fn an_expired_login_is_renewed_at_most_every_ten_minutes() {
        let start = Instant::now();
        let mut state = LiveState::default();
        let expired = sign_in_required(ReadStage::Discovery);
        assert!(!state.take_renewal(start, &status_failure(503, None)));
        assert!(state.take_renewal(start, &expired));
        assert!(!state.take_renewal(start + Duration::from_secs(599), &expired));
        assert!(state.take_renewal(start + RENEW_INTERVAL, &expired));
    }

    #[tokio::test]
    async fn renewal_tries_each_command_until_the_login_is_valid() {
        let now = crate::codex::reader::unix_now();
        let expired = format!(r#"{{"claudeAiOauth":{{"accessToken":"old","expiresAt":{}}}}}"#, (now - 60) * 1000);
        let renewed = format!(r#"{{"claudeAiOauth":{{"accessToken":"new","expiresAt":{}}}}}"#, (now + 3600) * 1000);

        // The first command leaves the login expired; the doctor renews it.
        let path = credentials("renew", &expired);
        let mut ran = Vec::new();
        let valid = renew_with(&path, |args| {
            ran.push(args.join(" "));
            if args == ["doctor"] {
                std::fs::write(&path, &renewed).unwrap();
            }
            async {}
        })
        .await;
        assert!(valid);
        assert_eq!(ran, ["plugin list", "doctor"]);

        // A login renewed by the first command needs no second one.
        std::fs::write(&path, &expired).unwrap();
        ran.clear();
        assert!(renew_with(&path, |args| {
            ran.push(args.join(" "));
            std::fs::write(&path, &renewed).unwrap();
            async {}
        })
        .await);
        assert_eq!(ran, ["plugin list"]);

        // A login nothing renews is reported as still expired.
        std::fs::write(&path, &expired).unwrap();
        assert!(!renew_with(&path, |_| async {}).await);
        let _ = std::fs::remove_file(path);
    }

    /// Reads the real endpoint with this machine's Claude Code login. Prints
    /// only the percentages. Run explicitly: `cargo test live_read -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_read() {
        let snapshot = fetch(crate::codex::reader::unix_now()).await.expect("live read");
        println!(
            "5h used {:?}% resets {:?}; 7d used {:?}% resets {:?}",
            snapshot.five_hour.map(|w| w.used_percent),
            snapshot.five_hour.and_then(|w| w.resets_at),
            snapshot.seven_day.map(|w| w.used_percent),
            snapshot.seven_day.and_then(|w| w.resets_at),
        );
    }
}
