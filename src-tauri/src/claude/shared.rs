//! Claude Code rate-limit snapshot, low-quota guard and settings wiring.
//!
//! Shared by the overlay and the `claude-usage-bridge` helper, which includes
//! this file directly. It depends on `serde`, `serde_json` and `dirs` only, so
//! the helper stays a small console program without Tauri.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const SNAPSHOT_FILE: &str = "claude-rate-limits.json";
pub const GUARD_FILE: &str = "claude-guard.json";
pub const GUARD_STATE_FILE: &str = "claude-guard-state.json";
pub const BRIDGE_FILE: &str = "claude-bridge.json";
/// Every command the overlay writes into Claude Code's settings contains this
/// name, so disconnecting removes exactly the entries it added.
pub const BRIDGE_MARKER: &str = "claude-usage-bridge";
/// A snapshot without a reset time is only trusted for this long.
pub const SNAPSHOT_FRESH_SECS: i64 = 15 * 60;
/// Identical values are rewritten at most this often, so a busy statusline
/// does not rewrite the file after every message.
const REWRITE_SECS: i64 = 30;
const SCHEMA_VERSION: u32 = 1;

/// The overlay's runtime directory, the same one `ConfigManager::new` uses.
pub fn runtime_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CodexUsageOverlay")
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema_version: u32,
    pub five_hour: Option<WindowSnapshot>,
    pub seven_day: Option<WindowSnapshot>,
    pub captured_at: i64,
    #[serde(default)]
    pub claude_version: Option<String>,
}

impl Snapshot {
    pub fn age(&self, now: i64) -> i64 {
        now.saturating_sub(self.captured_at).max(0)
    }
}

/// Extracts the `rate_limits` object Claude Code passes to its statusline
/// command. `None` when the input carries no usable window, e.g. for API-key
/// accounts or before the first response of a session.
pub fn parse_statusline(input: &Value, now: i64) -> Option<Snapshot> {
    let limits = input.get("rate_limits")?;
    let five_hour = limits.get("five_hour").and_then(parse_window);
    let seven_day = limits.get("seven_day").and_then(parse_window);
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    let claude_version = input
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| safe_version(v))
        .map(str::to_string);
    Some(Snapshot { schema_version: SCHEMA_VERSION, five_hour, seven_day, captured_at: now, claude_version })
}

fn parse_window(value: &Value) -> Option<WindowSnapshot> {
    let used_percent = value
        .get("used_percentage")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))?;
    let resets_at = value.get("resets_at").and_then(parse_timestamp);
    Some(WindowSnapshot { used_percent, resets_at })
}

/// Reset times arrive as epoch seconds in current releases and as ISO 8601
/// strings in some older ones, so both are accepted.
fn parse_timestamp(value: &Value) -> Option<i64> {
    let seconds = match value {
        Value::Number(n) => {
            let raw = n.as_f64()?;
            // Millisecond values are far beyond any plausible second value.
            if raw > 1e11 { raw / 1000.0 } else { raw }
        }
        Value::String(s) => match s.trim().parse::<f64>() {
            Ok(raw) => raw,
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

/// Days since 1970-01-01 of a proleptic Gregorian date.
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

/// Whether `next` carries news compared to the stored snapshot.
pub fn should_write(previous: Option<&Snapshot>, next: &Snapshot) -> bool {
    previous.is_none_or(|previous| {
        previous.five_hour != next.five_hour
            || previous.seven_day != next.seven_day
            || previous.claude_version != next.claude_version
            || next.captured_at.saturating_sub(previous.captured_at) >= REWRITE_SECS
    })
}

/// Writes through a per-process temporary file, so concurrent Claude Code
/// sessions and the overlay never observe a partial file.
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

/// The guard preferences the overlay mirrors from its own settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardConfig {
    pub enabled: bool,
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

/// Whether a new prompt or tool call should be stopped.
///
/// Used percentages only grow until the reset time, so an old snapshot is still
/// a valid lower bound while its reset time lies ahead. Everything uncertain —
/// no guard file, no snapshot, a passed reset, a window without reset time that
/// is no longer fresh — allows the request.
pub fn guard_decision(config: Option<&GuardConfig>, snapshot: Option<&Snapshot>, now: i64) -> GuardDecision {
    let (Some(config), Some(snapshot)) = (config, snapshot) else { return GuardDecision::Allow };
    if !config.enabled {
        return GuardDecision::Allow;
    }
    let fresh = snapshot.age(now) <= SNAPSHOT_FRESH_SECS;
    let low = |window: Option<WindowSnapshot>, threshold: u32| {
        window
            .filter(|w| w.resets_at.is_some_and(|at| at > now) || fresh)
            .map(|w| w.at(now))
            .filter(|w| w.remaining_percent() <= threshold)
    };
    let (kind, window) = match (
        low(snapshot.five_hour, config.five_hour_threshold_percent),
        low(snapshot.seven_day, config.weekly_threshold_percent),
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
            "Claude 5 小时额度仅剩 {remaining}%{}。Codex Usage Overlay 已拦截此请求；可在其设置中关闭额度保护。",
            countdown.map(|c| format!("，约 {c} 后重置")).unwrap_or_default()
        ),
        ("zh-CN", WindowKind::Weekly) => format!(
            "Claude 每周额度仅剩 {remaining}%{}。Codex Usage Overlay 已拦截此请求；可在其设置中关闭额度保护。",
            countdown.map(|c| format!("，约 {c} 后重置")).unwrap_or_default()
        ),
        ("zh-Hant", WindowKind::FiveHour) => format!(
            "Claude 5 小時額度僅剩 {remaining}%{}。Codex Usage Overlay 已攔截此請求；可在其設定中關閉額度保護。",
            countdown.map(|c| format!("，約 {c} 後重置")).unwrap_or_default()
        ),
        ("zh-Hant", WindowKind::Weekly) => format!(
            "Claude 每週額度僅剩 {remaining}%{}。Codex Usage Overlay 已攔截此請求；可在其設定中關閉額度保護。",
            countdown.map(|c| format!("，約 {c} 後重置")).unwrap_or_default()
        ),
        (_, kind) => format!(
            "Claude {} limit has {remaining}% left{}. Codex Usage Overlay blocked this request; turn off the quota guard in its settings to continue.",
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

/// The helper's own configuration: the statusline it replaced, so the user's
/// existing statusline keeps rendering through the helper.
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

/// The command line Claude Code runs for a helper subcommand. Claude Code on
/// Windows runs commands through Git Bash, which accepts forward slashes and
/// a double-quoted path.
pub fn bridge_command(bridge: &Path, args: &str) -> String {
    format!("\"{}\" {args}", bridge.to_string_lossy().replace('\\', "/"))
}

fn is_ours(command: Option<&Value>) -> bool {
    command.and_then(Value::as_str).is_some_and(|c| c.contains(BRIDGE_MARKER))
}

const HOOK_EVENTS: [(&str, &str, Option<&str>); 2] =
    [("UserPromptSubmit", "hook prompt", None), ("PreToolUse", "hook tool", Some("*"))];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub status_line: bool,
    pub hooks: bool,
}

pub fn connection(settings: &Value) -> Connection {
    Connection {
        status_line: is_ours(settings.pointer("/statusLine/command")),
        hooks: HOOK_EVENTS.iter().all(|(event, _, _)| {
            settings
                .pointer(&format!("/hooks/{event}"))
                .and_then(Value::as_array)
                .is_some_and(|groups| groups.iter().any(group_has_ours))
        }),
    }
}

fn group_has_ours(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| hooks.iter().any(|hook| is_ours(hook.get("command"))))
}

/// Points Claude Code's statusline and guard hooks at the helper. Returns the
/// statusline it replaced, unless that one already was the helper.
pub fn connect(settings: &mut Value, bridge: &Path) -> Result<Option<Value>, String> {
    if settings.is_null() {
        *settings = Value::Object(Map::new());
    }
    let root = settings.as_object_mut().ok_or("Claude Code settings are not a JSON object")?;
    let previous = root.get("statusLine").cloned().filter(|line| !is_ours(line.get("command")));
    root.insert(
        "statusLine".into(),
        json!({ "type": "command", "command": bridge_command(bridge, "statusline"), "padding": 0 }),
    );
    let hooks = root.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or("Claude Code hooks are not a JSON object")?;
    for (event, args, matcher) in HOOK_EVENTS {
        let groups = hooks.entry(event).or_insert_with(|| Value::Array(Vec::new()));
        let groups = groups.as_array_mut().ok_or_else(|| format!("Claude Code {event} hooks are not a list"))?;
        remove_ours(groups);
        let mut group = json!({ "hooks": [{ "type": "command", "command": bridge_command(bridge, args), "timeout": 10 }] });
        if let Some(matcher) = matcher {
            group["matcher"] = json!(matcher);
        }
        groups.push(group);
    }
    Ok(previous)
}

/// Removes everything `connect` added and restores the chained statusline.
/// Hooks and keys the user added stay as they were.
pub fn disconnect(settings: &mut Value, chained: Option<Value>) {
    let Some(root) = settings.as_object_mut() else { return };
    if is_ours(root.get("statusLine").and_then(|line| line.get("command"))) {
        match chained {
            Some(line) => { root.insert("statusLine".into(), line); }
            None => { root.remove("statusLine"); }
        }
    }
    let mut hooks_empty = false;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        for (event, _, _) in HOOK_EVENTS {
            let now_empty = hooks.get_mut(event).and_then(Value::as_array_mut).is_some_and(|groups| {
                remove_ours(groups);
                groups.is_empty()
            });
            if now_empty {
                hooks.remove(event);
            }
        }
        hooks_empty = hooks.is_empty();
    }
    if hooks_empty {
        root.remove("hooks");
    }
}

fn remove_ours(groups: &mut Vec<Value>) {
    for group in groups.iter_mut() {
        if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            hooks.retain(|hook| !is_ours(hook.get("command")));
        }
    }
    groups.retain(|group| group.get("hooks").and_then(Value::as_array).is_none_or(|hooks| !hooks.is_empty()));
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_793_000_000;

    fn input(five: f64, five_reset: Value, week: f64, week_reset: Value) -> Value {
        json!({
            "version": "2.1.90",
            "rate_limits": {
                "five_hour": { "used_percentage": five, "resets_at": five_reset },
                "seven_day": { "used_percentage": week, "resets_at": week_reset }
            }
        })
    }

    fn guard(five: u32, week: u32) -> GuardConfig {
        GuardConfig { enabled: true, five_hour_threshold_percent: five, weekly_threshold_percent: week, locale: "en-US".into() }
    }

    #[test]
    fn statusline_input_accepts_epoch_seconds_milliseconds_and_iso_reset_times() {
        let snapshot = parse_statusline(&input(30.0, json!(NOW + 600), 60.0, json!("2026-11-05T12:30:00Z")), NOW).unwrap();
        assert_eq!(snapshot.five_hour, Some(WindowSnapshot { used_percent: 30.0, resets_at: Some(NOW + 600) }));
        assert_eq!(snapshot.seven_day.unwrap().resets_at, Some(1_793_881_800));
        assert_eq!(snapshot.claude_version.as_deref(), Some("2.1.90"));
        assert_eq!(snapshot.captured_at, NOW);

        let millis = parse_statusline(&input(1.0, json!((NOW + 60) * 1000), 1.0, json!("2026-11-05T20:30:00+08:00")), NOW).unwrap();
        assert_eq!(millis.five_hour.unwrap().resets_at, Some(NOW + 60));
        assert_eq!(millis.seven_day.unwrap().resets_at, Some(1_793_881_800));
        assert_eq!(parse_iso8601("1970-01-01T00:00:00.123Z"), Some(0));
        assert_eq!(parse_iso8601("2000-03-01T00:00:00Z"), Some(951_868_800));
    }

    #[test]
    fn statusline_input_without_usable_limits_is_ignored() {
        assert_eq!(parse_statusline(&json!({ "model": {} }), NOW), None);
        assert_eq!(parse_statusline(&json!({ "rate_limits": {} }), NOW), None);
        assert_eq!(parse_statusline(&input(130.0, json!(NOW), -1.0, json!(NOW)), NOW), None);
        let partial = parse_statusline(&json!({ "rate_limits": { "seven_day": { "used_percentage": 5 } } }), NOW).unwrap();
        assert_eq!(partial.five_hour, None);
        assert_eq!(partial.seven_day, Some(WindowSnapshot { used_percent: 5.0, resets_at: None }));
        let unsafe_version = json!({ "version": "x; rm -rf", "rate_limits": { "five_hour": { "used_percentage": 1 } } });
        assert_eq!(parse_statusline(&unsafe_version, NOW).unwrap().claude_version, None);
    }

    #[test]
    fn a_passed_reset_rolls_the_window_over() {
        let window = WindowSnapshot { used_percent: 97.0, resets_at: Some(NOW) };
        assert_eq!(window.at(NOW - 1).remaining_percent(), 3);
        assert_eq!(window.at(NOW), WindowSnapshot { used_percent: 0.0, resets_at: None });
    }

    #[test]
    fn identical_values_are_rewritten_only_after_the_rewrite_interval() {
        let first = parse_statusline(&input(10.0, json!(NOW + 60), 20.0, json!(NOW + 600)), NOW).unwrap();
        let mut same = first.clone();
        same.captured_at = NOW + REWRITE_SECS - 1;
        assert!(should_write(None, &first));
        assert!(!should_write(Some(&first), &same));
        same.captured_at = NOW + REWRITE_SECS;
        assert!(should_write(Some(&first), &same));
        let mut changed = first.clone();
        changed.five_hour = Some(WindowSnapshot { used_percent: 11.0, resets_at: Some(NOW + 60) });
        assert!(should_write(Some(&first), &changed));
    }

    #[test]
    fn atomic_writes_round_trip_through_the_reader() {
        let dir = std::env::temp_dir().join(format!("claude-core-snapshot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let snapshot = parse_statusline(&input(10.0, json!(NOW + 60), 20.0, json!(NOW + 600)), NOW).unwrap();
        write_json_atomic(&dir.join(SNAPSHOT_FILE), &snapshot).unwrap();
        assert_eq!(read_snapshot(&dir), Some(snapshot));
        fs::write(dir.join(SNAPSHOT_FILE), "{ broken").unwrap();
        assert_eq!(read_snapshot(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_guard_blocks_at_or_below_either_threshold() {
        let snapshot = parse_statusline(&input(95.0, json!(NOW + 4800), 50.0, json!(NOW + 86_400)), NOW).unwrap();
        let GuardDecision::Block(reason) = guard_decision(Some(&guard(5, 2)), Some(&snapshot), NOW) else { panic!() };
        assert_eq!(reason, "Claude 5-hour limit has 5% left (resets in 1h 20m). Codex Usage Overlay blocked this request; turn off the quota guard in its settings to continue.");
        assert_eq!(guard_decision(Some(&guard(4, 2)), Some(&snapshot), NOW), GuardDecision::Allow);

        let weekly = parse_statusline(&input(10.0, json!(NOW + 60), 99.0, json!(NOW + 90_000)), NOW).unwrap();
        let mut zh = guard(5, 2);
        zh.locale = "zh-CN".into();
        let GuardDecision::Block(reason) = guard_decision(Some(&zh), Some(&weekly), NOW) else { panic!() };
        assert!(reason.starts_with("Claude 每周额度仅剩 1%，约 1d 01h 后重置"), "{reason}");
    }

    #[test]
    fn the_guard_fails_open_whenever_the_quota_is_uncertain() {
        let low = parse_statusline(&input(99.0, json!(NOW + 600), 99.0, Value::Null), NOW).unwrap();
        let config = guard(5, 2);
        assert_eq!(guard_decision(None, Some(&low), NOW), GuardDecision::Allow);
        assert_eq!(guard_decision(Some(&config), None, NOW), GuardDecision::Allow);
        let mut disabled = config.clone();
        disabled.enabled = false;
        assert_eq!(guard_decision(Some(&disabled), Some(&low), NOW), GuardDecision::Allow);
        // Old but still before its reset: the used share can only have grown.
        let later = NOW + SNAPSHOT_FRESH_SECS + 1;
        assert!(matches!(guard_decision(Some(&config), Some(&low), NOW + 599), GuardDecision::Block(_)));
        // The 5-hour window has reset and the weekly one has no reset time and is stale.
        assert_eq!(guard_decision(Some(&config), Some(&low), later), GuardDecision::Allow);
    }

    fn user_settings() -> Value {
        json!({
            "model": "opus",
            "statusLine": { "type": "command", "command": "~/.claude/statusline.sh", "padding": 1 },
            "hooks": {
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "lint.sh" }] }]
            }
        })
    }

    #[test]
    fn connecting_and_disconnecting_restores_the_users_settings() {
        let bridge = Path::new(r"C:\Program Files\Codex Usage Overlay\claude-usage-bridge.exe");
        let original = user_settings();
        let mut settings = original.clone();
        assert_eq!(connection(&settings), Connection::default());

        let previous = connect(&mut settings, bridge).unwrap();
        assert_eq!(previous, Some(original["statusLine"].clone()));
        assert_eq!(connection(&settings), Connection { status_line: true, hooks: true });
        assert_eq!(
            settings.pointer("/statusLine/command").unwrap(),
            "\"C:/Program Files/Codex Usage Overlay/claude-usage-bridge.exe\" statusline"
        );
        assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        assert_eq!(settings["model"], "opus");

        // Connecting again neither duplicates hooks nor chains the helper to itself.
        assert_eq!(connect(&mut settings, bridge).unwrap(), None);
        assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        assert_eq!(settings["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1);

        disconnect(&mut settings, previous);
        assert_eq!(settings, original);
    }

    #[test]
    fn disconnecting_a_fresh_install_leaves_no_empty_sections() {
        let mut settings = Value::Null;
        let previous = connect(&mut settings, Path::new("C:/bridge/claude-usage-bridge.exe")).unwrap();
        assert_eq!(previous, None);
        disconnect(&mut settings, previous);
        assert_eq!(settings, json!({}));
        assert!(connect(&mut json!([1]), Path::new("x")).is_err());
    }

    #[test]
    fn a_chained_statusline_command_is_read_from_the_bridge_config() {
        let config = BridgeConfig { chained_status_line: Some(user_settings()["statusLine"].clone()) };
        assert_eq!(config.chained_command(), Some("~/.claude/statusline.sh"));
        assert_eq!(BridgeConfig::default().chained_command(), None);
    }
}
