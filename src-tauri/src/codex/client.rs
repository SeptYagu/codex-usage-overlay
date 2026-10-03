use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use super::finder::find_codex_executable;

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
    pub fn to_ui(&self, five_hour_burn_rate_per_hour: Option<f64>, week_burn_rate_per_hour: Option<f64>) -> CodexUsage {
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
        Self {
            exe_path: find_codex_executable(),
        }
    }

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

    pub async fn fetch_usage(&mut self, timeout_duration: Duration) -> Result<RawCodexUsage> {
        let mut child = self.spawn_with_recovery(find_codex_executable, |exe| {
            Command::new(exe)
                .arg("app-server")
                .arg("--stdio")
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
        })?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("无法连接子进程 stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("无法连接子进程 stdout"))?;
        let mut reader = BufReader::new(stdout).lines();

        let request_future = async move {
            let init_req = r#"{"id":1,"method":"initialize","params":{"clientInfo":{"name":"codex-usage-overlay","title":"Codex Usage Overlay","version":"0.3.0"},"capabilities":{"experimentalApi":true}}}"#;
            let init_notif = r#"{"method":"initialized","params":{}}"#;
            let query_req = r#"{"id":2,"method":"account/rateLimits/read","params":{"excludeResetCreditDetails":true,"supportsLunaReserveFallback":false}}"#;

            stdin
                .write_all(format!("{}\n{}\n{}\n", init_req, init_notif, query_req).as_bytes())
                .await?;
            stdin.flush().await?;

            while let Some(line) = reader.next_line().await? {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                if let Ok(msg) = serde_json::from_str::<Value>(trimmed) {
                    if msg.get("id").and_then(|id| id.as_i64()) == Some(2) {
                        if let Some(err) = msg.get("error") {
                            return Err(anyhow!("Codex usage request failed: {}", err));
                        }
                        if let Some(result) = msg.get("result") {
                            return parse_usage_result(result);
                        }
                    }
                }
            }

            Err(anyhow!("Codex app-server 关闭且未返回有效用量数据"))
        };

        let usage_result = match timeout(timeout_duration, request_future).await {
            Ok(res) => res,
            Err(_) => Err(anyhow!("读取 Codex 用量超时")),
        };

        // Ensure child process is killed cleanly
        let _ = child.kill().await;

        usage_result
    }
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
        result.get("account").and_then(|a| a.get("id")).and_then(Value::as_str),
        result.get("account").and_then(|a| a.get("accountId")).and_then(Value::as_str),
        bucket.get("accountId").and_then(Value::as_str),
        bucket.get("account").and_then(|a| a.get("id")).and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.trim().is_empty())
    .map(ToOwned::to_owned)
}

fn parse_usage_result(result: &Value) -> Result<RawCodexUsage> {
    let bucket = if let Some(by_limit) = result.get("rateLimitsByLimitId") {
        by_limit.get("codex").or_else(|| result.get("rateLimits"))
    } else {
        result.get("rateLimits")
    };

    let bucket = bucket.ok_or_else(|| anyhow!("未在返回结果中找到 rateLimits 数据"))?;
    let five_hour = parse_quota(bucket.get("primary"));
    let weekly = parse_quota(bucket.get("secondary"));

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
}
