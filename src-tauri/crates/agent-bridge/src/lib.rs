//! Shared primitives for the standalone `claude-usage-bridge` and
//! `antigravity-usage-bridge` CLI helpers and their overlay installers.
//!
//! Intentionally depends only on `serde`, `serde_json`, and `dirs` so helper
//! binaries start fast without pulling in Tauri or Tokio.

pub mod antigravity;
pub mod claude;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

pub const SCHEMA_VERSION: u32 = 1;
/// A snapshot without a reset time is only trusted for this long.
pub const SNAPSHOT_FRESH_SECS: i64 = 15 * 60;
/// Identical values are rewritten at most this often.
pub const REWRITE_SECS: i64 = 30;
pub const CHAINED_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_INPUT_BYTES: u64 = 1024 * 1024;

/// How Windows executes a chained statusline command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMode {
    /// Always prefer Git Bash when available (Claude Code behavior).
    PreferGitBash,
    /// Use `cmd /C` unless the command uses bash syntax (`.sh`, `bash `, `~/`).
    CmdUnlessBashSyntax,
}

/// The overlay's runtime directory, matching `ConfigManager::new`.
pub fn runtime_dir() -> PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CodexUsageOverlay");
    #[cfg(debug_assertions)]
    if let Some(override_dir) = std::env::var_os("CODEX_OVERLAY_TEST_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return override_dir;
    }
    dir
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSnapshot {
    pub used_percent: f64,
    pub resets_at: Option<i64>,
}

impl WindowSnapshot {
    /// The window as of `now`. A window whose reset time has passed has rolled
    /// over, so nothing of it is used anymore.
    pub fn at(self, now: i64) -> WindowSnapshot {
        match self.resets_at {
            Some(resets_at) if resets_at <= now => WindowSnapshot {
                used_percent: 0.0,
                resets_at: None,
            },
            _ => self,
        }
    }

    pub fn remaining_percent(self) -> u32 {
        (100.0 - self.used_percent).clamp(0.0, 100.0).round() as u32
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GuardState {
    pub blocked_count: u64,
    pub last_blocked_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardDecision {
    Allow,
    Block(String),
}

#[derive(Clone, Copy)]
pub enum WindowKind {
    FiveHour,
    Weekly,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BridgeConfig {
    pub chained_status_line: Option<Value>,
}

impl BridgeConfig {
    pub fn chained_command(&self) -> Option<&str> {
        self.chained_status_line
            .as_ref()?
            .get("command")
            .and_then(Value::as_str)
            .filter(|c| !c.trim().is_empty())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub status_line: bool,
    pub hooks: bool,
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn read_stdin() -> String {
    let mut input = String::new();
    let _ = std::io::stdin().take(MAX_INPUT_BYTES).read_to_string(&mut input);
    input
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn read_json_doc(path: &Path, label: &str) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => Ok(Value::Null),
        Ok(raw) => serde_json::from_str(&raw).map_err(|error| {
            format!("{label} is not valid JSON ({error}); fix {} before connecting", path.display())
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

pub fn read_bridge_config(runtime_dir: &Path, bridge_file: &str) -> BridgeConfig {
    read_json(&runtime_dir.join(bridge_file)).unwrap_or_default()
}

pub fn backup_once(path: &Path, backup_suffix: &str) -> Result<(), String> {
    let backup = path.with_extension(format!("json.{backup_suffix}"));
    if path.is_file() && !backup.exists() {
        fs::copy(path, &backup)
            .map_err(|error| format!("Could not back up {}: {error}", path.display()))?;
    }
    Ok(())
}

/// Writes through a per-process temporary file so concurrent CLI sessions and
/// the overlay never observe a partial file.
pub fn write_json_atomic(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

pub fn record_block(dir: &Path, guard_state_file: &str, now: i64) {
    let path = dir.join(guard_state_file);
    let mut state: GuardState = read_json(&path).unwrap_or_default();
    state.blocked_count = state.blocked_count.saturating_add(1);
    state.last_blocked_at = Some(now);
    let _ = write_json_atomic(&path, &state);
}

pub fn disconnect_status_line(
    root: &mut Map<String, Value>,
    chained: Option<Value>,
    is_ours: impl Fn(Option<&Value>) -> bool,
) {
    if is_ours(root.get("statusLine").and_then(|line| line.get("command"))) {
        match chained {
            Some(line) => {
                root.insert("statusLine".into(), line);
            }
            None => {
                root.remove("statusLine");
            }
        }
    }
}

/// Accepts epoch seconds, epoch milliseconds, or ISO 8601 strings.
pub fn parse_timestamp(value: &Value) -> Option<i64> {
    let seconds = match value {
        Value::Number(n) => {
            let raw = n.as_f64()?;
            if raw > 1e11 { raw / 1000.0 } else { raw }
        }
        Value::String(s) => match s.trim().parse::<f64>() {
            Ok(raw) => {
                if raw > 1e11 { raw / 1000.0 } else { raw }
            }
            Err(_) => parse_iso8601(s)? as f64,
        },
        _ => return None,
    };
    (seconds.is_finite() && seconds > 0.0 && seconds < 1e11).then_some(seconds as i64)
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`.
pub fn parse_iso8601(value: &str) -> Option<i64> {
    let value = value.trim();
    let (date, rest) = value.split_once(['T', ' '])?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (time, offset_seconds) = if let Some(time) = rest.strip_suffix(['Z', 'z']) {
        (time, 0)
    } else if let Some(index) = rest.rfind(['+', '-']) {
        let (time, offset) = rest.split_at(index);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let (hours, minutes) = offset[1..].split_once(':').unwrap_or((&offset[1..], "0"));
        let hours: i64 = hours.parse().ok()?;
        let minutes: i64 = minutes.parse().ok()?;
        (time, sign * (hours * 3600 + minutes * 60))
    } else {
        return None;
    };
    let time = time.split('.').next()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset_seconds)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn safe_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
}

#[allow(clippy::too_many_arguments)]
pub fn evaluate_guard_windows(
    provider_label: &str,
    enabled: bool,
    five_hour: Option<WindowSnapshot>,
    weekly: Option<WindowSnapshot>,
    five_hour_threshold_percent: u32,
    weekly_threshold_percent: u32,
    fresh: bool,
    locale: &str,
    now: i64,
) -> GuardDecision {
    if !enabled {
        return GuardDecision::Allow;
    }
    let low = |window: Option<WindowSnapshot>, threshold: u32| {
        window
            .filter(|w| w.resets_at.is_some_and(|at| at > now) || fresh)
            .map(|w| w.at(now))
            .filter(|w| w.remaining_percent() <= threshold)
    };
    let (kind, window) = match (
        low(five_hour, five_hour_threshold_percent),
        low(weekly, weekly_threshold_percent),
    ) {
        (Some(window), _) => (WindowKind::FiveHour, window),
        (None, Some(window)) => (WindowKind::Weekly, window),
        (None, None) => return GuardDecision::Allow,
    };
    GuardDecision::Block(block_reason(provider_label, locale, kind, window, now))
}

fn block_reason(
    provider_label: &str,
    locale: &str,
    kind: WindowKind,
    window: WindowSnapshot,
    now: i64,
) -> String {
    let remaining = window.remaining_percent();
    let countdown = window
        .resets_at
        .map(|at| format_countdown(at.saturating_sub(now)));
    match (locale, kind) {
        ("zh-CN", WindowKind::FiveHour) => format!(
            "{provider_label} 5 小时额度仅剩 {remaining}%{}。Codex Usage Overlay 已拦截此请求；可在其设置中关闭额度保护。",
            countdown.map(|c| format!("，约 {c} 后重置")).unwrap_or_default()
        ),
        ("zh-CN", WindowKind::Weekly) => format!(
            "{provider_label} 每周额度仅剩 {remaining}%{}。Codex Usage Overlay 已拦截此请求；可在其设置中关闭额度保护。",
            countdown.map(|c| format!("，约 {c} 后重置")).unwrap_or_default()
        ),
        ("zh-Hant", WindowKind::FiveHour) => format!(
            "{provider_label} 5 小時額度僅剩 {remaining}%{}。Codex Usage Overlay 已攔截此請求；可在其設定中關閉額度保護。",
            countdown.map(|c| format!("，約 {c} 後重置")).unwrap_or_default()
        ),
        ("zh-Hant", WindowKind::Weekly) => format!(
            "{provider_label} 每週額度僅剩 {remaining}%{}。Codex Usage Overlay 已攔截此請求；可在其設定中關閉額度保護。",
            countdown.map(|c| format!("，約 {c} 後重置")).unwrap_or_default()
        ),
        (_, kind) => format!(
            "{provider_label} {} limit has {remaining}% left{}. Codex Usage Overlay blocked this request; turn off the quota guard in its settings to continue.",
            match kind {
                WindowKind::FiveHour => "5-hour",
                WindowKind::Weekly => "weekly",
            },
            countdown.map(|c| format!(" (resets in {c})")).unwrap_or_default()
        ),
    }
}

fn format_countdown(seconds: i64) -> String {
    let minutes = (seconds.max(0) + 59) / 60;
    let hours = minutes / 60;
    if hours >= 24 {
        format!("{}d {:02}h", hours / 24, hours % 24)
    } else {
        format!("{hours}h {:02}m", minutes % 60)
    }
}

/// Runs the user's chained statusline command with `input` on stdin.
pub fn run_chained(command: &str, input: &str, mode: ShellMode) -> Option<Vec<u8>> {
    let mut child = shell(command, mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        output
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < CHAINED_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    reader.join().ok().filter(|output| !output.is_empty())
}

fn shell(command: &str, mode: ShellMode) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let use_bash = match mode {
            ShellMode::PreferGitBash => true,
            ShellMode::CmdUnlessBashSyntax => {
                command.trim_start().starts_with("bash ")
                    || command.contains(".sh")
                    || command.contains("~/")
            }
        };
        let mut shell = match (use_bash, git_bash()) {
            (true, Some(bash)) => {
                let mut shell = Command::new(bash);
                shell.arg("-c").arg(command);
                shell
            }
            _ => {
                let mut shell = Command::new("cmd");
                shell.arg("/C").arg(command);
                shell
            }
        };
        shell.creation_flags(CREATE_NO_WINDOW);
        shell
    }
    #[cfg(not(windows))]
    {
        let _ = mode;
        let mut shell = Command::new("sh");
        shell.arg("-c").arg(command);
        shell
    }
}

/// Finds Git Bash on Windows while skipping the WSL launcher in System32.
#[cfg(windows)]
fn git_bash() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>());
    on_path
        .filter(|dir| {
            let dir = dir.to_string_lossy().to_ascii_lowercase();
            !dir.contains("\\windows\\system32") && !dir.contains("windowsapps")
        })
        .map(|dir| dir.join("bash.exe"))
        .chain(
            ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"]
                .into_iter()
                .filter_map(std::env::var_os)
                .flat_map(|root| {
                    let root = PathBuf::from(root);
                    [
                        root.join("Git").join("bin").join("bash.exe"),
                        root.join("Programs").join("Git").join("bin").join("bash.exe"),
                    ]
                }),
        )
        .find(|path| path.is_file())
}
