//! Antigravity CLI rate-limit snapshot, low-quota guard and settings/hooks wiring.
//!
//! Shared by the overlay and the `antigravity-usage-bridge` helper, which includes
//! this file directly. It depends on `serde`, `serde_json` and `dirs` only, so
//! the helper stays a small console program without Tauri.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const SNAPSHOT_FILE: &str = "antigravity-rate-limits.json";
pub const GUARD_FILE: &str = "antigravity-guard.json";
pub const GUARD_STATE_FILE: &str = "antigravity-guard-state.json";
pub const BRIDGE_FILE: &str = "antigravity-bridge.json";
/// Every command the overlay writes into Antigravity's configs contains this
/// marker, so disconnecting removes only the entries it added.
pub const BRIDGE_MARKER: &str = "antigravity-usage-bridge";
pub const HOOK_NAME: &str = "codex-usage-overlay";
/// A snapshot without a reset time is only trusted for this long.
pub const SNAPSHOT_FRESH_SECS: i64 = 15 * 60;
/// Identical values are rewritten at most this often.
const REWRITE_SECS: i64 = 30;
pub const SCHEMA_VERSION: u32 = 1;

/// Which Antigravity quota pool to project and guard against.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PoolKind {
    Gemini,
    Argon,
    #[serde(rename = "3p")]
    ThirdParty,
    #[default]
    #[serde(other)]
    Auto,
}

impl PoolKind {
    pub const ALL: [Self; 4] = [Self::Auto, Self::Gemini, Self::Argon, Self::ThirdParty];

    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Gemini => "gemini",
            Self::Argon => "argon",
            Self::ThirdParty => "3p",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "gemini" => Some(Self::Gemini),
            "argon" => Some(Self::Argon),
            "3p" | "third_party" | "thirdParty" => Some(Self::ThirdParty),
            _ => None,
        }
    }
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
    /// over, so nothing of it is used any more.
    pub fn at(self, now: i64) -> WindowSnapshot {
        match self.resets_at {
            Some(resets_at) if resets_at <= now => WindowSnapshot { used_percent: 0.0, resets_at: None },
            _ => self,
        }
    }

    pub fn remaining_percent(self) -> u32 {
        (100.0 - self.used_percent).clamp(0.0, 100.0).round() as u32
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolWindows {
    pub five_hour: Option<WindowSnapshot>,
    pub weekly: Option<WindowSnapshot>,
}

impl PoolWindows {
    pub fn is_empty(&self) -> bool {
        self.five_hour.is_none() && self.weekly.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema_version: u32,
    #[serde(default)]
    pub gemini: PoolWindows,
    #[serde(default)]
    pub argon: PoolWindows,
    #[serde(default, rename = "thirdParty")]
    pub third_party: PoolWindows,
    #[serde(default)]
    pub active_model_id: Option<String>,
    #[serde(default)]
    pub active_model_name: Option<String>,
    #[serde(default)]
    pub credits_display: Option<String>,
    #[serde(default)]
    pub credits_balance: Option<String>,
    #[serde(default)]
    pub has_credits: bool,
    pub captured_at: i64,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub cli_version: Option<String>,
}

impl Snapshot {
    pub fn age(&self, now: i64) -> i64 {
        now.saturating_sub(self.captured_at).max(0)
    }

    /// Resolves `Auto` from the last observed active model id/name.
    pub fn resolve_pool(&self, selection: PoolKind) -> PoolKind {
        if selection != PoolKind::Auto {
            return selection;
        }
        let haystack = format!(
            "{} {}",
            self.active_model_id.as_deref().unwrap_or(""),
            self.active_model_name.as_deref().unwrap_or("")
        )
        .to_ascii_lowercase();
        if haystack.contains("argon") || haystack.contains("orchestrator") {
            PoolKind::Argon
        } else if ["claude", "gpt", "sonnet", "opus", "haiku"]
            .iter()
            .any(|needle| haystack.contains(needle))
        {
            PoolKind::ThirdParty
        } else {
            PoolKind::Gemini
        }
    }

    /// Projects the selected (or resolved) pool's 5H and weekly windows.
    /// Argon falls back to the shared `gemini-5h` window when no Argon-specific
    /// 5H bucket is present.
    pub fn windows_for(&self, pool: PoolKind) -> PoolWindows {
        match self.resolve_pool(pool) {
            PoolKind::Gemini => self.gemini.clone(),
            PoolKind::Argon => PoolWindows {
                five_hour: self.argon.five_hour.or(self.gemini.five_hour),
                weekly: self.argon.weekly,
            },
            PoolKind::ThirdParty => self.third_party.clone(),
            PoolKind::Auto => unreachable!("resolve_pool never returns Auto"),
        }
    }
}

fn first_nonempty_str<'a>(candidates: &[Option<&'a Value>]) -> Option<String> {
    candidates
        .iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .find(|s| !s.is_empty() && s.len() <= 160)
        .map(str::to_string)
}

fn normalize_bucket_key(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn assign_bucket(
    key: &str,
    window: WindowSnapshot,
    gemini: &mut PoolWindows,
    argon: &mut PoolWindows,
    third_party: &mut PoolWindows,
    generic: &mut PoolWindows,
) {
    match normalize_bucket_key(key).as_str() {
        "gemini-5h" | "gemini5h" | "gemini-five-hour" => gemini.five_hour = Some(window),
        "gemini-weekly" | "gemini-7d" | "gemini-seven-day" => gemini.weekly = Some(window),
        "gemini-argon-5h" | "argon-5h" | "gemini-argon-five-hour" | "argon-five-hour" => {
            argon.five_hour = Some(window);
        }
        "gemini-argon-wtu-weekly" | "gemini-argon-weekly" | "argon-weekly" | "argon-wtu-weekly"
        | "gemini-argon-7d" | "argon-7d" => {
            argon.weekly = Some(window);
        }
        "3p-5h" | "third-party-5h" | "thirdparty-5h" | "3p-five-hour" => {
            third_party.five_hour = Some(window);
        }
        "3p-weekly" | "3p-7d" | "third-party-weekly" | "third-party-7d" | "thirdparty-weekly" => {
            third_party.weekly = Some(window);
        }
        "five-hour" | "5h" => generic.five_hour = Some(window),
        "weekly" | "seven-day" | "7d" => generic.weekly = Some(window),
        _ => {}
    }
}

fn assign_nested_pool(obj: &Map<String, Value>, target: &mut PoolWindows) {
    if let Some(w) = ["five_hour", "fiveHour", "5h"]
        .iter()
        .find_map(|k| obj.get(*k))
        .and_then(parse_window)
    {
        target.five_hour = Some(w);
    }
    if let Some(w) = ["weekly", "seven_day", "sevenDay", "7d", "wtu_weekly", "wtuWeekly"]
        .iter()
        .find_map(|k| obj.get(*k))
        .and_then(parse_window)
    {
        target.weekly = Some(w);
    }
}

fn parse_credits(input: &Value, limits: Option<&Value>) -> (bool, Option<String>, Option<String>) {
    let raw = ["ai_credits", "aiCredits", "credits"]
        .iter()
        .filter_map(|key| input.get(*key).or_else(|| limits.and_then(|l| l.get(*key))))
        .next();
    let Some(raw) = raw else {
        return (false, None, None);
    };
    let format_num = |value: f64| -> Option<String> {
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        if (value - value.round()).abs() < 1e-6 {
            Some(format!("{}", value.round() as i64))
        } else {
            Some(format!("{value:.1}"))
        }
    };
    match raw {
        Value::Number(n) => {
            if let Some(text) = n.as_f64().and_then(format_num) {
                return (true, Some(text.clone()), Some(text));
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if !trimmed.is_empty() && trimmed != "—" && trimmed != "-" && trimmed.len() <= 32 {
                let text = trimmed.to_string();
                return (true, Some(text.clone()), Some(text));
            }
        }
        Value::Object(map) => {
            if let Some(num) = ["remaining", "balance", "available", "amount", "value"]
                .iter()
                .filter_map(|k| map.get(*k))
                .find_map(Value::as_f64)
                .and_then(format_num)
            {
                return (true, Some(num.clone()), Some(num));
            }
            if let Some(text) = ["display", "creditsDisplay", "credits_display", "balance", "remaining"]
                .iter()
                .filter_map(|k| map.get(*k))
                .find_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "—" && *s != "-" && s.len() <= 32)
            {
                let owned = text.to_string();
                return (true, Some(owned.clone()), Some(owned));
            }
        }
        _ => {}
    }
    (false, None, None)
}

/// Extracts all Antigravity quota buckets, active model identity, and optional
/// AI Credits from the statusline JSON payload. Returns `None` when the input
/// contains neither usable quota windows nor credits.
pub fn parse_statusline(input: &Value, now: i64) -> Option<Snapshot> {
    let limits = ["rate_limits", "rateLimits", "quotas", "limits", "buckets"]
        .iter()
        .filter_map(|key| input.get(*key))
        .next();

    let mut gemini = PoolWindows::default();
    let mut argon = PoolWindows::default();
    let mut third_party = PoolWindows::default();
    let mut generic = PoolWindows::default();

    if let Some(limits) = limits {
        match limits {
            Value::Object(map) => {
                for (key, value) in map {
                    if let Some(window) = parse_window(value) {
                        assign_bucket(key, window, &mut gemini, &mut argon, &mut third_party, &mut generic);
                    } else if let Some(nested) = value.as_object() {
                        match normalize_bucket_key(key).as_str() {
                            "gemini" => assign_nested_pool(nested, &mut gemini),
                            "argon" | "gemini-argon" => assign_nested_pool(nested, &mut argon),
                            "3p" | "third-party" | "thirdparty" => assign_nested_pool(nested, &mut third_party),
                            _ => {}
                        }
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    let Some(obj) = item.as_object() else { continue };
                    let id = ["id", "bucket", "name", "quota_id", "quotaId", "pool"]
                        .iter()
                        .filter_map(|k| obj.get(*k))
                        .find_map(Value::as_str);
                    if let (Some(id), Some(window)) = (id, parse_window(item)) {
                        assign_bucket(id, window, &mut gemini, &mut argon, &mut third_party, &mut generic);
                    }
                }
            }
            _ => {}
        }
    }

    if gemini.five_hour.is_none() {
        gemini.five_hour = generic.five_hour;
    }
    if gemini.weekly.is_none() {
        gemini.weekly = generic.weekly;
    }

    let (has_credits, credits_display, credits_balance) = parse_credits(input, limits);
    if gemini.is_empty() && argon.is_empty() && third_party.is_empty() && !has_credits {
        return None;
    }

    let active_model_id = first_nonempty_str(&[
        input.get("active_model_id"),
        input.get("activeModelId"),
        input.get("model_id"),
        input.pointer("/model/id"),
    ]);
    let active_model_name = first_nonempty_str(&[
        input.get("active_model_name"),
        input.get("activeModelName"),
        input.pointer("/model/display_name"),
        input.pointer("/model/displayName"),
        input.pointer("/model/name"),
    ]);
    let cli_version = ["version", "cli_version", "cliVersion"]
        .iter()
        .filter_map(|key| input.get(*key))
        .find_map(Value::as_str)
        .filter(|v| safe_version(v))
        .map(str::to_string);
    let account_id = first_nonempty_str(&[
        input.get("account_id"),
        input.get("accountId"),
        input.pointer("/account/id"),
        input.pointer("/account/email"),
        input.get("user_email"),
    ]);

    Some(Snapshot {
        schema_version: SCHEMA_VERSION,
        gemini,
        argon,
        third_party,
        active_model_id,
        active_model_name,
        credits_display,
        credits_balance,
        has_credits,
        captured_at: now,
        account_id,
        cli_version,
    })
}

fn parse_window(value: &Value) -> Option<WindowSnapshot> {
    let used_percent = ["used_percentage", "usedPercentage", "used_percent", "usedPercent"]
        .iter()
        .filter_map(|key| value.get(*key))
        .find_map(Value::as_f64)
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
        .or_else(|| {
            ["remaining_percentage", "remainingPercentage", "remaining_percent", "remainingPercent"]
                .iter()
                .filter_map(|key| value.get(*key))
                .find_map(Value::as_f64)
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .map(|rem| (100.0 - rem).clamp(0.0, 100.0))
        })?;
    let resets_at = ["resets_at", "resetsAt", "reset_at", "resetAt", "reset_time", "resetTime"]
        .iter()
        .filter_map(|key| value.get(*key))
        .find_map(parse_timestamp);
    Some(WindowSnapshot { used_percent, resets_at })
}

/// Accepts epoch seconds, epoch milliseconds, or ISO 8601 strings.
pub fn parse_timestamp(value: &Value) -> Option<i64> {
    let seconds = match value {
        Value::Number(n) => {
            let raw = n.as_f64()?;
            if raw > 1e11 { raw / 1000.0 } else { raw }
        }
        Value::String(s) => match s.trim().parse::<f64>() {
            Ok(raw) => if raw > 1e11 { raw / 1000.0 } else { raw },
            Err(_) => parse_iso8601(s)? as f64,
        },
        _ => return None,
    };
    (seconds.is_finite() && seconds > 0.0 && seconds < 1e11).then_some(seconds as i64)
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`.
fn parse_iso8601(value: &str) -> Option<i64> {
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

fn safe_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
}

pub fn read_snapshot(dir: &Path) -> Option<Snapshot> {
    let raw = fs::read_to_string(dir.join(SNAPSHOT_FILE)).ok()?;
    serde_json::from_str::<Snapshot>(&raw).ok().filter(|s| s.schema_version == SCHEMA_VERSION)
}

pub fn should_write(previous: Option<&Snapshot>, next: &Snapshot) -> bool {
    previous.is_none_or(|previous| {
        previous.gemini != next.gemini
            || previous.argon != next.argon
            || previous.third_party != next.third_party
            || previous.active_model_id != next.active_model_id
            || previous.active_model_name != next.active_model_name
            || previous.credits_display != next.credits_display
            || previous.has_credits != next.has_credits
            || previous.cli_version != next.cli_version
            || previous.account_id != next.account_id
            || next.captured_at.saturating_sub(previous.captured_at) >= REWRITE_SECS
    })
}

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

/// The guard preferences mirrored from the overlay's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardConfig {
    pub enabled: bool,
    #[serde(default)]
    pub pool: PoolKind,
    pub five_hour_threshold_percent: u32,
    pub weekly_threshold_percent: u32,
    pub locale: String,
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

/// Evaluates whether a `PreInvocation` hook should stop a new model request
/// based on the selected (or auto-detected) Antigravity quota pool.
pub fn guard_decision(config: Option<&GuardConfig>, snapshot: Option<&Snapshot>, now: i64) -> GuardDecision {
    let (Some(config), Some(snapshot)) = (config, snapshot) else { return GuardDecision::Allow };
    if !config.enabled {
        return GuardDecision::Allow;
    }
    let fresh = snapshot.age(now) <= SNAPSHOT_FRESH_SECS;
    let windows = snapshot.windows_for(config.pool);
    let low = |window: Option<WindowSnapshot>, threshold: u32| {
        window
            .filter(|w| w.resets_at.is_some_and(|at| at > now) || fresh)
            .map(|w| w.at(now))
            .filter(|w| w.remaining_percent() <= threshold)
    };
    let (kind, window) = match (
        low(windows.five_hour, config.five_hour_threshold_percent),
        low(windows.weekly, config.weekly_threshold_percent),
    ) {
        (Some(window), _) => (WindowKind::FiveHour, window),
        (None, Some(window)) => (WindowKind::Weekly, window),
        (None, None) => return GuardDecision::Allow,
    };
    GuardDecision::Block(block_reason(&config.locale, kind, window, now))
}

#[derive(Clone, Copy)]
enum WindowKind {
    FiveHour,
    Weekly,
}

fn block_reason(locale: &str, kind: WindowKind, window: WindowSnapshot, now: i64) -> String {
    let remaining = window.remaining_percent();
    let countdown = window.resets_at.map(|at| format_countdown(at.saturating_sub(now)));
    match (locale, kind) {
        ("zh-CN", WindowKind::FiveHour) => format!(
            "Antigravity 5 小时额度仅剩 {remaining}%{}。Codex Usage Overlay 已拦截此请求；可在其设置中关闭额度保护。",
            countdown.map(|c| format!("，约 {c} 后重置")).unwrap_or_default()
        ),
        ("zh-CN", WindowKind::Weekly) => format!(
            "Antigravity 每周额度仅剩 {remaining}%{}。Codex Usage Overlay 已拦截此请求；可在其设置中关闭额度保护。",
            countdown.map(|c| format!("，约 {c} 后重置")).unwrap_or_default()
        ),
        ("zh-Hant", WindowKind::FiveHour) => format!(
            "Antigravity 5 小時額度僅剩 {remaining}%{}。Codex Usage Overlay 已攔截此請求；可在其設定中關閉額度保護。",
            countdown.map(|c| format!("，約 {c} 後重置")).unwrap_or_default()
        ),
        ("zh-Hant", WindowKind::Weekly) => format!(
            "Antigravity 每週額度僅剩 {remaining}%{}。Codex Usage Overlay 已攔截此請求；可在其設定中關閉額度保護。",
            countdown.map(|c| format!("，約 {c} 後重置")).unwrap_or_default()
        ),
        (_, kind) => format!(
            "Antigravity {} limit has {remaining}% left{}. Codex Usage Overlay blocked this request; turn off the quota guard in its settings to continue.",
            match kind { WindowKind::FiveHour => "5-hour", WindowKind::Weekly => "weekly" },
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

pub fn bridge_command(bridge: &Path, args: &str) -> String {
    format!("\"{}\" {args}", bridge.to_string_lossy().replace('\\', "/"))
}

fn is_ours(command: Option<&Value>) -> bool {
    command.and_then(Value::as_str).is_some_and(|c| c.contains(BRIDGE_MARKER))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub status_line: bool,
    pub hooks: bool,
}

pub fn connection(settings: &Value, hooks_doc: &Value) -> Connection {
    let pre_invocation_connected = hooks_doc
        .get(HOOK_NAME)
        .and_then(Value::as_object)
        .filter(|entry| entry.get("enabled").and_then(Value::as_bool) != Some(false))
        .and_then(|entry| entry.get("PreInvocation"))
        .and_then(Value::as_array)
        .is_some_and(|handlers| handlers.iter().any(|handler| is_ours(handler.get("command"))));

    Connection {
        status_line: is_ours(settings.pointer("/statusLine/command")),
        hooks: pre_invocation_connected,
    }
}

/// Connects `~/.gemini/antigravity-cli/settings.json` (`statusLine`), returning
/// the previous non-overlay statusline config if any.
pub fn connect_settings(settings: &mut Value, bridge: &Path, preserve_chained: bool) -> Result<Option<Value>, String> {
    if settings.is_null() {
        *settings = Value::Object(Map::new());
    }
    let root = settings
        .as_object_mut()
        .ok_or("Antigravity CLI settings are not a JSON object")?;
    let previous = root.get("statusLine").cloned().filter(|line| !is_ours(line.get("command")));
    let has_chained = previous.is_some() || preserve_chained;
    let entry = if has_chained {
        json!({ "type": "command", "command": bridge_command(bridge, "statusline"), "padding": 0 })
    } else {
        json!({
            "type": "command",
            "command": bridge_command(bridge, "statusline"),
            "stack_with_default": true,
            "padding": 0
        })
    };
    root.insert("statusLine".into(), entry);
    Ok(previous)
}

/// Restores any previous `statusLine` entry in `~/.gemini/antigravity-cli/settings.json`.
pub fn disconnect_settings(settings: &mut Value, chained: Option<Value>) {
    let Some(root) = settings.as_object_mut() else { return };
    if is_ours(root.get("statusLine").and_then(|line| line.get("command"))) {
        match chained {
            Some(line) => { root.insert("statusLine".into(), line); }
            None => { root.remove("statusLine"); }
        }
    }
}

/// Registers only `PreInvocation` under top-level `"codex-usage-overlay"` in
/// `~/.gemini/config/hooks.json`. Never sets `PreToolUse` so tool approval prompts
/// are completely unaffected.
pub fn connect_hooks(hooks_doc: &mut Value, bridge: &Path) -> Result<(), String> {
    if hooks_doc.is_null() {
        *hooks_doc = Value::Object(Map::new());
    }
    let root = hooks_doc
        .as_object_mut()
        .ok_or("Antigravity hooks configuration is not a JSON object")?;
    root.insert(
        HOOK_NAME.into(),
        json!({
            "PreInvocation": [
                {
                    "type": "command",
                    "command": bridge_command(bridge, "hook invocation"),
                    "timeout": 10
                }
            ]
        }),
    );
    Ok(())
}

/// Removes only the `"codex-usage-overlay"` key from `hooks.json`.
pub fn disconnect_hooks(hooks_doc: &mut Value) {
    let Some(root) = hooks_doc.as_object_mut() else { return };
    root.remove(HOOK_NAME);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_793_000_000;

    fn sample_input() -> Value {
        json!({
            "version": "0.9.4",
            "active_model_id": "gemini-3.1-pro",
            "active_model_name": "Gemini 3.1 Pro",
            "ai_credits": 240,
            "rate_limits": {
                "gemini-5h": { "used_percentage": 22.5, "resets_at": NOW + 3600 },
                "gemini-weekly": { "used_percentage": 40.0, "resets_at": NOW + 86_400 },
                "gemini-argon-wtu-weekly": { "used_percentage": 75.0, "resets_at": NOW + 172_800 },
                "3p-5h": { "used_percentage": 10.0, "resets_at": NOW + 1800 },
                "3p-weekly": { "used_percentage": 92.0, "resets_at": NOW + 259_200 }
            }
        })
    }

    #[test]
    fn parses_all_buckets_model_and_credits() {
        let snap = parse_statusline(&sample_input(), NOW).unwrap();
        assert_eq!(snap.gemini.five_hour, Some(WindowSnapshot { used_percent: 22.5, resets_at: Some(NOW + 3600) }));
        assert_eq!(snap.gemini.weekly, Some(WindowSnapshot { used_percent: 40.0, resets_at: Some(NOW + 86_400) }));
        assert_eq!(snap.argon.five_hour, None);
        assert_eq!(snap.argon.weekly, Some(WindowSnapshot { used_percent: 75.0, resets_at: Some(NOW + 172_800) }));
        assert_eq!(snap.third_party.five_hour, Some(WindowSnapshot { used_percent: 10.0, resets_at: Some(NOW + 1800) }));
        assert_eq!(snap.third_party.weekly, Some(WindowSnapshot { used_percent: 92.0, resets_at: Some(NOW + 259_200) }));
        assert!(snap.has_credits);
        assert_eq!(snap.credits_display.as_deref(), Some("240"));
        assert_eq!(snap.cli_version.as_deref(), Some("0.9.4"));
    }

    #[test]
    fn argon_aliases_and_shared_5h_fallback_work() {
        for key in ["gemini_argon_wtu_weekly", "gemini-argon-weekly", "gemini_argon_weekly", "argon-weekly", "argon_weekly"] {
            let input = json!({
                "active_model_id": "argon-orchestrator",
                "rate_limits": {
                    "gemini-5h": { "used_percentage": 35.0, "resets_at": NOW + 600 },
                    key: { "used_percentage": 60.0, "resets_at": NOW + 7200 }
                }
            });
            let snap = parse_statusline(&input, NOW).unwrap();
            assert_eq!(snap.resolve_pool(PoolKind::Auto), PoolKind::Argon);
            let windows = snap.windows_for(PoolKind::Auto);
            assert_eq!(windows.five_hour.unwrap().used_percent, 35.0);
            assert_eq!(windows.weekly.unwrap().used_percent, 60.0);
        }
    }

    #[test]
    fn auto_pool_routes_third_party_models_and_defaults_to_gemini() {
        let mut snap = parse_statusline(&sample_input(), NOW).unwrap();
        assert_eq!(snap.resolve_pool(PoolKind::Auto), PoolKind::Gemini);
        snap.active_model_id = Some("claude-sonnet-4.6".into());
        assert_eq!(snap.resolve_pool(PoolKind::Auto), PoolKind::ThirdParty);
        assert_eq!(snap.windows_for(PoolKind::Auto).weekly.unwrap().used_percent, 92.0);
        snap.active_model_id = Some("gpt-5.4".into());
        assert_eq!(snap.resolve_pool(PoolKind::Auto), PoolKind::ThirdParty);
        // Explicit override wins over auto detection.
        assert_eq!(snap.resolve_pool(PoolKind::Gemini), PoolKind::Gemini);
        assert_eq!(snap.windows_for(PoolKind::Gemini).weekly.unwrap().used_percent, 40.0);
    }

    #[test]
    fn guard_evaluates_selected_pool_and_respects_thresholds() {
        let snap = parse_statusline(&sample_input(), NOW).unwrap();
        // Gemini has 77.5% left 5H and 60% left WK -> Allow.
        let gemini_guard = GuardConfig {
            enabled: true,
            pool: PoolKind::Gemini,
            five_hour_threshold_percent: 5,
            weekly_threshold_percent: 10,
            locale: "en-US".into(),
        };
        assert_eq!(guard_decision(Some(&gemini_guard), Some(&snap), NOW), GuardDecision::Allow);

        // Switching guard pool to 3P sees 3p-weekly = 92% used (8% left) <= 10% -> Block.
        let tp_guard = GuardConfig { pool: PoolKind::ThirdParty, ..gemini_guard };
        let GuardDecision::Block(reason) = guard_decision(Some(&tp_guard), Some(&snap), NOW) else { panic!() };
        assert!(reason.contains("Antigravity weekly limit has 8% left"), "{reason}");
    }

    #[test]
    fn connect_and_disconnect_preserve_user_settings_and_hooks_and_skip_pretooluse() {
        let bridge = Path::new(r"C:\Program Files\Codex Usage Overlay\antigravity-usage-bridge.exe");
        let original_settings = json!({ "theme": "dark" });
        let original_hooks = json!({
            "lint-checker": {
                "PostToolUse": [{ "matcher": "run_command", "hooks": [{ "type": "command", "command": "lint.sh" }] }]
            }
        });
        let mut settings = original_settings.clone();
        let mut hooks = original_hooks.clone();

        let prev = connect_settings(&mut settings, bridge, false).unwrap();
        assert_eq!(prev, None);
        assert_eq!(settings.pointer("/statusLine/stack_with_default"), Some(&json!(true)));
        connect_hooks(&mut hooks, bridge).unwrap();

        assert_eq!(connection(&settings, &hooks), Connection { status_line: true, hooks: true });
        assert!(hooks["codex-usage-overlay"].get("PreToolUse").is_none());
        assert_eq!(hooks["lint-checker"], original_hooks["lint-checker"]);

        disconnect_settings(&mut settings, prev);
        disconnect_hooks(&mut hooks);
        assert_eq!(settings, original_settings);
        assert_eq!(hooks, original_hooks);
    }

    #[test]
    fn chaining_previous_statusline_omits_stack_with_default() {
        let bridge = Path::new(r"C:\overlay\antigravity-usage-bridge.exe");
        let mut settings = json!({ "statusLine": { "type": "command", "command": "my-line.sh" } });
        let prev = connect_settings(&mut settings, bridge, false).unwrap();
        assert_eq!(prev, Some(json!({ "type": "command", "command": "my-line.sh" })));
        assert!(settings.pointer("/statusLine/stack_with_default").is_none());
        // Second connect keeps omitting stack_with_default while chained config exists.
        assert_eq!(connect_settings(&mut settings, bridge, true).unwrap(), None);
        assert!(settings.pointer("/statusLine/stack_with_default").is_none());
        disconnect_settings(&mut settings, prev);
        assert_eq!(settings["statusLine"], json!({ "type": "command", "command": "my-line.sh" }));
    }
}
