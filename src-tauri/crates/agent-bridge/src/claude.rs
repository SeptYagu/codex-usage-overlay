//! Claude Code rate-limit snapshot, low-quota guard, and settings wiring.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

pub use crate::{
    disconnect_status_line, evaluate_guard_windows, parse_timestamp, runtime_dir, safe_version,
    write_json_atomic, BridgeConfig, Connection, GuardDecision, GuardState, WindowSnapshot,
    REWRITE_SECS, SCHEMA_VERSION, SNAPSHOT_FRESH_SECS,
};

pub const SNAPSHOT_FILE: &str = "claude-rate-limits.json";
pub const GUARD_FILE: &str = "claude-guard.json";
pub const GUARD_STATE_FILE: &str = "claude-guard-state.json";
pub const BRIDGE_FILE: &str = "claude-bridge.json";
/// Every command written into Claude Code's settings contains this marker.
pub const BRIDGE_MARKER: &str = "claude-usage-bridge";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema_version: u32,
    pub five_hour: Option<WindowSnapshot>,
    pub seven_day: Option<WindowSnapshot>,
    pub captured_at: i64,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub claude_version: Option<String>,
}

impl Snapshot {
    pub fn age(&self, now: i64) -> i64 {
        now.saturating_sub(self.captured_at).max(0)
    }
}

/// Extracts `rate_limits` passed on Claude Code's statusline stdin.
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
    Some(Snapshot {
        schema_version: SCHEMA_VERSION,
        five_hour,
        seven_day,
        captured_at: now,
        account_id: None,
        claude_version,
    })
}

/// Reads the non-secret account UUID from Claude Code's global metadata.
pub fn account_identity() -> Option<String> {
    let path = dirs::home_dir()?.join(".claude.json");
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    account_identity_from_config(&serde_json::from_str::<Value>(&raw).ok()?)
}

fn account_identity_from_config(config: &Value) -> Option<String> {
    let account = &config["oauthAccount"];
    let id = account["accountUuid"].as_str().filter(|s| valid_uuid(s))?;
    let organization = account["organizationUuid"]
        .as_str()
        .filter(|s| valid_uuid(s));
    Some(format!("{id}:{}", organization.unwrap_or("")))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn parse_window(value: &Value) -> Option<WindowSnapshot> {
    let used_percent = value
        .get("used_percentage")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))?;
    let resets_at = value.get("resets_at").and_then(parse_timestamp);
    Some(WindowSnapshot {
        used_percent,
        resets_at,
    })
}

pub fn read_snapshot(dir: &Path) -> Option<Snapshot> {
    let raw = fs::read_to_string(dir.join(SNAPSHOT_FILE)).ok()?;
    serde_json::from_str::<Snapshot>(&raw)
        .ok()
        .filter(|s| s.schema_version == SCHEMA_VERSION)
}

pub fn should_write(previous: Option<&Snapshot>, next: &Snapshot) -> bool {
    previous.is_none_or(|previous| {
        previous.five_hour != next.five_hour
            || previous.seven_day != next.seven_day
            || previous.claude_version != next.claude_version
            || previous.account_id != next.account_id
            || next.captured_at.saturating_sub(previous.captured_at) >= REWRITE_SECS
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardConfig {
    pub enabled: bool,
    pub five_hour_threshold_percent: u32,
    pub weekly_threshold_percent: u32,
    pub locale: String,
}

pub fn guard_decision(config: Option<&GuardConfig>, snapshot: Option<&Snapshot>, now: i64) -> GuardDecision {
    let (Some(config), Some(snapshot)) = (config, snapshot) else {
        return GuardDecision::Allow;
    };
    evaluate_guard_windows(
        "Claude",
        config.enabled,
        snapshot.five_hour,
        snapshot.seven_day,
        config.five_hour_threshold_percent,
        config.weekly_threshold_percent,
        snapshot.age(now) <= SNAPSHOT_FRESH_SECS,
        &config.locale,
        now,
    )
}

pub fn bridge_command(bridge: &Path, args: &str) -> String {
    format!("\"{}\" {args}", bridge.to_string_lossy().replace('\\', "/"))
}

fn is_ours(command: Option<&Value>) -> bool {
    command
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(BRIDGE_MARKER))
}

const HOOK_EVENTS: [(&str, &str, Option<&str>); 2] = [
    ("UserPromptSubmit", "hook prompt", None),
    ("PreToolUse", "hook tool", Some("*")),
];

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

pub fn connect(settings: &mut Value, bridge: &Path) -> Result<Option<Value>, String> {
    if settings.is_null() {
        *settings = Value::Object(Map::new());
    }
    let root = settings
        .as_object_mut()
        .ok_or("Claude Code settings are not a JSON object")?;
    let previous = root
        .get("statusLine")
        .cloned()
        .filter(|line| !is_ours(line.get("command")));
    root.insert(
        "statusLine".into(),
        json!({ "type": "command", "command": bridge_command(bridge, "statusline"), "padding": 0 }),
    );
    let hooks = root.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks
        .as_object_mut()
        .ok_or("Claude Code hooks are not a JSON object")?;
    for (event, args, matcher) in HOOK_EVENTS {
        let groups = hooks.entry(event).or_insert_with(|| Value::Array(Vec::new()));
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| format!("Claude Code {event} hooks are not a list"))?;
        remove_ours(groups);
        let mut group = json!({
            "hooks": [{ "type": "command", "command": bridge_command(bridge, args), "timeout": 10 }]
        });
        if let Some(matcher) = matcher {
            group["matcher"] = json!(matcher);
        }
        groups.push(group);
    }
    Ok(previous)
}

pub fn disconnect(settings: &mut Value, chained: Option<Value>) {
    let Some(root) = settings.as_object_mut() else { return };
    disconnect_status_line(root, chained, is_ours);
    let mut hooks_empty = false;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        for (event, _, _) in HOOK_EVENTS {
            let now_empty = hooks
                .get_mut(event)
                .and_then(Value::as_array_mut)
                .is_some_and(|groups| {
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
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_none_or(|hooks| !hooks.is_empty())
    });
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
        GuardConfig {
            enabled: true,
            five_hour_threshold_percent: five,
            weekly_threshold_percent: week,
            locale: "en-US".into(),
        }
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
        assert_eq!(crate::parse_iso8601("1970-01-01T00:00:00.123Z"), Some(0));
        assert_eq!(crate::parse_iso8601("2000-03-01T00:00:00Z"), Some(951_868_800));
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
    fn account_identity_accepts_only_valid_account_uuids() {
        let config = json!({"oauthAccount": {
            "accountUuid": "12345678-1234-1234-1234-123456789abc",
            "organizationUuid": "abcdefab-cdef-abcd-efab-cdefabcdefab"
        }});
        assert_eq!(
            account_identity_from_config(&config).as_deref(),
            Some("12345678-1234-1234-1234-123456789abc:abcdefab-cdef-abcd-efab-cdefabcdefab")
        );
        assert_eq!(account_identity_from_config(&json!({"oauthAccount":{"accountUuid":"invalid"}})), None);
        assert_eq!(account_identity_from_config(&json!({"oauthAccount":{"organizationUuid":"abcdefab-cdef-abcd-efab-cdefabcdefab"}})), None);
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
        let later = NOW + SNAPSHOT_FRESH_SECS + 1;
        assert!(matches!(guard_decision(Some(&config), Some(&low), NOW + 599), GuardDecision::Block(_)));
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
