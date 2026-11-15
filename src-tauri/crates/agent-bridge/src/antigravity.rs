//! Antigravity CLI rate-limit snapshot, low-quota guard, and settings/hooks wiring.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub use crate::{
    disconnect_status_line, evaluate_guard_windows, parse_timestamp, runtime_dir, safe_version,
    write_json_atomic, BridgeConfig, Connection, GuardDecision, GuardState, WindowSnapshot,
    REWRITE_SECS, SCHEMA_VERSION, SNAPSHOT_FRESH_SECS,
};

pub const SNAPSHOT_FILE: &str = "antigravity-rate-limits.json";
pub const GUARD_FILE: &str = "antigravity-guard.json";
pub const GUARD_STATE_FILE: &str = "antigravity-guard-state.json";
pub const BRIDGE_FILE: &str = "antigravity-bridge.json";
pub const BRIDGE_SHIM_FILE: &str = "antigravity-usage-bridge.cmd";
pub const BRIDGE_MARKER: &str = "antigravity-usage-bridge";
pub const HOOK_NAME: &str = "codex-usage-overlay";

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
        } else if ["claude", "gpt", "sonnet", "opus", "haiku", "anthropic"]
            .iter()
            .any(|needle| haystack.contains(needle))
        {
            PoolKind::ThirdParty
        } else {
            PoolKind::Gemini
        }
    }

    /// Argon falls back to the shared `gemini-5h` window when no Argon 5H bucket exists.
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

    pub fn apply_hook_model(&mut self, input: &Value) -> bool {
        let (id, name) = parse_hook_model(input);
        let mut changed = false;
        if let Some(id) = id {
            if self.active_model_id.as_deref() != Some(id.as_str()) {
                self.active_model_id = Some(id);
                changed = true;
            }
            if let Some(name) = name {
                if self.active_model_name.as_deref() != Some(name.as_str()) {
                    self.active_model_name = Some(name);
                    changed = true;
                }
            }
        } else if let Some(name) = name {
            if self.active_model_name.as_deref() != Some(name.as_str()) || self.active_model_id.is_some() {
                self.active_model_id = None;
                self.active_model_name = Some(name);
                changed = true;
            }
        }
        changed
    }
}

fn first_nonempty_str(candidates: &[Option<&Value>]) -> Option<String> {
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

fn assign_bucket_list(
    items: &[Value],
    now: i64,
    gemini: &mut PoolWindows,
    argon: &mut PoolWindows,
    third_party: &mut PoolWindows,
    generic: &mut PoolWindows,
) {
    for item in items {
        let Some(obj) = item.as_object() else { continue };
        let id = ["id", "bucket", "name", "quota_id", "quotaId", "pool"]
            .iter()
            .filter_map(|k| obj.get(*k))
            .find_map(Value::as_str);
        if let (Some(id), Some(window)) = (id, parse_window(item, now)) {
            assign_bucket(id, window, gemini, argon, third_party, generic);
        }
    }
}

fn assign_nested_pool(obj: &Map<String, Value>, now: i64, target: &mut PoolWindows) {
    if let Some(w) = ["five_hour", "fiveHour", "5h"]
        .iter()
        .find_map(|k| obj.get(*k))
        .and_then(|v| parse_window(v, now))
    {
        target.five_hour = Some(w);
    }
    if let Some(w) = ["weekly", "seven_day", "sevenDay", "7d", "wtu_weekly", "wtuWeekly"]
        .iter()
        .find_map(|k| obj.get(*k))
        .and_then(|v| parse_window(v, now))
    {
        target.weekly = Some(w);
    }
}

fn format_credit_number(value: f64) -> Option<String> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    if (value - value.round()).abs() < 1e-6 {
        Some(format!("{}", value.round() as i64))
    } else {
        Some(format!("{value:.1}"))
    }
}

fn parse_credit_value(raw: &Value) -> (bool, Option<String>, Option<String>) {
    match raw {
        Value::Number(n) => {
            if let Some(text) = n.as_f64().and_then(format_credit_number) {
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
            if let Some(num) = [
                "remaining_credits",
                "remainingCredits",
                "remaining",
                "balance",
                "available",
                "amount",
                "value",
            ]
            .iter()
            .filter_map(|k| map.get(*k))
            .find_map(Value::as_f64)
            .and_then(format_credit_number)
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

fn parse_credits(input: &Value, limits: Option<&Value>) -> (bool, Option<String>, Option<String>) {
    let raw = ["ai_credits", "aiCredits", "credits"]
        .iter()
        .filter_map(|key| input.get(*key).or_else(|| limits.and_then(|l| l.get(*key))))
        .next();
    let Some(raw) = raw else {
        return (false, None, None);
    };
    parse_credit_value(raw)
}

fn extract_cli_version(input: &Value) -> Option<String> {
    ["version", "cli_version", "cliVersion"]
        .iter()
        .filter_map(|key| input.get(*key))
        .find_map(Value::as_str)
        .filter(|v| safe_version(v))
        .map(str::to_string)
}

fn extract_account_id(input: &Value) -> Option<String> {
    first_nonempty_str(&[
        input.get("account_id"),
        input.get("accountId"),
        input.get("email"),
        input.pointer("/account/id"),
        input.pointer("/account/email"),
        input.get("user_email"),
    ])
}

fn is_placeholder_model(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.is_empty() || lower == "auto" || lower == "default"
}

pub fn parse_hook_model(input: &Value) -> (Option<String>, Option<String>) {
    let id = first_nonempty_str(&[
        input.get("active_model_id"),
        input.get("activeModelId"),
        input.get("model_id"),
        input.get("modelId"),
        input.pointer("/model/id"),
    ])
    .filter(|value| !is_placeholder_model(value));
    let name = first_nonempty_str(&[
        input.get("active_model_name"),
        input.get("activeModelName"),
        input.get("model_name"),
        input.get("modelName"),
        input.pointer("/model/display_name"),
        input.pointer("/model/displayName"),
        input.pointer("/model/name"),
        input.get("model").filter(|v| v.is_string()),
    ])
    .filter(|value| !is_placeholder_model(value));
    (id, name)
}

pub fn parse_statusline(input: &Value, now: i64) -> Option<Snapshot> {
    let limits = ["rate_limits", "rateLimits", "quota", "quotas", "limits", "buckets"]
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
                    if let Some(window) = parse_window(value, now) {
                        assign_bucket(key, window, &mut gemini, &mut argon, &mut third_party, &mut generic);
                    } else if let Some(nested) = value.as_object() {
                        match normalize_bucket_key(key).as_str() {
                            "gemini" => assign_nested_pool(nested, now, &mut gemini),
                            "argon" | "gemini-argon" => assign_nested_pool(nested, now, &mut argon),
                            "3p" | "third-party" | "thirdparty" => assign_nested_pool(nested, now, &mut third_party),
                            _ => {}
                        }
                    }
                }
            }
            Value::Array(items) => {
                assign_bucket_list(items, now, &mut gemini, &mut argon, &mut third_party, &mut generic);
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
        input.get("modelId"),
        input.pointer("/model/id"),
    ]);
    let active_model_name = first_nonempty_str(&[
        input.get("active_model_name"),
        input.get("activeModelName"),
        input.get("model_name"),
        input.get("modelName"),
        input.pointer("/model/display_name"),
        input.pointer("/model/displayName"),
        input.pointer("/model/name"),
    ]);
    let cli_version = extract_cli_version(input);
    let account_id = extract_account_id(input);

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

pub fn parse_usage_command(input: &Value, now: i64) -> Option<Snapshot> {
    if input.get("num_turns").and_then(Value::as_u64) != Some(0) {
        return None;
    }
    if input.pointer("/command/name").and_then(Value::as_str) != Some("usage") {
        return None;
    }
    let groups = input.pointer("/command/data/groups").and_then(Value::as_array)?;

    let mut gemini = PoolWindows::default();
    let mut argon = PoolWindows::default();
    let mut third_party = PoolWindows::default();
    let mut generic = PoolWindows::default();

    for group in groups {
        if let Some(buckets) = group.get("buckets").and_then(Value::as_array) {
            assign_bucket_list(buckets, now, &mut gemini, &mut argon, &mut third_party, &mut generic);
        }
    }

    if gemini.five_hour.is_none() {
        gemini.five_hour = generic.five_hour;
    }
    if gemini.weekly.is_none() {
        gemini.weekly = generic.weekly;
    }

    if gemini.is_empty() && argon.is_empty() && third_party.is_empty() {
        return None;
    }

    let data = input.pointer("/command/data");
    let account_id = data.and_then(extract_account_id).or_else(|| extract_account_id(input));
    let cli_version = data.and_then(extract_cli_version).or_else(|| extract_cli_version(input));

    Some(Snapshot {
        schema_version: SCHEMA_VERSION,
        gemini,
        argon,
        third_party,
        active_model_id: None,
        active_model_name: None,
        credits_display: None,
        credits_balance: None,
        has_credits: false,
        captured_at: now,
        account_id,
        cli_version,
    })
}

pub fn parse_credits_command(input: &Value) -> Option<(Option<String>, Option<String>)> {
    if input.get("num_turns").and_then(Value::as_u64) != Some(0) {
        return None;
    }
    if input.pointer("/command/name").and_then(Value::as_str) != Some("credits") {
        return None;
    }
    let data = input.pointer("/command/data")?;
    let (has_credits, display, balance) = parse_credit_value(data);
    has_credits.then_some((display, balance))
}

fn parse_window(value: &Value, now: i64) -> Option<WindowSnapshot> {
    if value.get("disabled").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let used_percent = ["used_percentage", "usedPercentage", "used_percent", "usedPercent"]
        .iter()
        .filter_map(|key| value.get(*key))
        .find_map(Value::as_f64)
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
        .or_else(|| {
            ["used_fraction", "usedFraction"]
                .iter()
                .filter_map(|key| value.get(*key))
                .find_map(Value::as_f64)
                .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                .map(|used| (used * 100.0).clamp(0.0, 100.0))
        })
        .or_else(|| {
            ["remaining_percentage", "remainingPercentage", "remaining_percent", "remainingPercent"]
                .iter()
                .filter_map(|key| value.get(*key))
                .find_map(Value::as_f64)
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .map(|rem| (100.0 - rem).clamp(0.0, 100.0))
        })
        .or_else(|| {
            ["remaining_fraction", "remainingFraction"]
                .iter()
                .filter_map(|key| value.get(*key))
                .find_map(Value::as_f64)
                .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                .map(|rem| ((1.0 - rem) * 100.0).clamp(0.0, 100.0))
        })?;
    let resets_at = ["resets_at", "resetsAt", "reset_at", "resetAt", "reset_time", "resetTime"]
        .iter()
        .filter_map(|key| value.get(*key))
        .find_map(parse_timestamp)
        .or_else(|| {
            ["reset_in_seconds", "resetInSeconds"]
                .iter()
                .filter_map(|key| value.get(*key))
                .find_map(Value::as_f64)
                .filter(|v| v.is_finite() && (0.0..1e10).contains(v))
                .map(|secs| now.saturating_add(secs.round() as i64))
        });
    Some(WindowSnapshot { used_percent, resets_at })
}

pub fn read_snapshot(dir: &Path) -> Option<Snapshot> {
    let raw = fs::read_to_string(dir.join(SNAPSHOT_FILE)).ok()?;
    serde_json::from_str::<Snapshot>(&raw)
        .ok()
        .filter(|s| s.schema_version == SCHEMA_VERSION)
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

pub fn guard_decision(config: Option<&GuardConfig>, snapshot: Option<&Snapshot>, now: i64) -> GuardDecision {
    let (Some(config), Some(snapshot)) = (config, snapshot) else {
        return GuardDecision::Allow;
    };
    let windows = snapshot.windows_for(config.pool);
    evaluate_guard_windows(
        "Antigravity",
        config.enabled,
        windows.five_hour,
        windows.weekly,
        config.five_hour_threshold_percent,
        config.weekly_threshold_percent,
        snapshot.age(now) <= SNAPSHOT_FRESH_SECS,
        &config.locale,
        now,
    )
}

#[cfg(windows)]
mod win32 {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};

    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(lpsz_long_path: *const u16, lpsz_short_path: *mut u16, cch_buffer: u32) -> u32;
        fn GetLongPathNameW(lpsz_short_path: *const u16, lpsz_long_path: *mut u16, cch_buffer: u32) -> u32;
    }

    unsafe fn transform_wide(
        path: &Path,
        func: unsafe extern "system" fn(*const u16, *mut u16, u32) -> u32,
    ) -> Option<PathBuf> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let len = func(wide.as_ptr(), std::ptr::null_mut(), 0);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        let written = func(wide.as_ptr(), buf.as_mut_ptr(), len);
        if written == 0 || written >= len {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&buf[..written as usize])))
    }

    pub fn short_path(path: &Path) -> PathBuf {
        unsafe { transform_wide(path, GetShortPathNameW) }.unwrap_or_else(|| path.to_path_buf())
    }

    pub fn long_path(path: &Path) -> Option<PathBuf> {
        unsafe { transform_wide(path, GetLongPathNameW) }
    }
}

pub fn resolve_execution_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        win32::short_path(path)
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

pub fn is_unquoted_safe(path: &Path) -> bool {
    let text = path.to_string_lossy();
    !text.chars().any(char::is_whitespace) && !text.contains('"')
}

pub fn bridge_command(bridge: &Path, args: &str) -> String {
    let resolved = resolve_execution_path(bridge);
    let display = resolved.to_string_lossy();
    if is_unquoted_safe(&resolved) {
        format!("{display} {args}")
    } else {
        format!("\"{display}\" {args}")
    }
}

fn split_command(command: &str) -> Option<(&str, &str)> {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix('"') {
        let (exe, tail) = rest.split_once('"')?;
        Some((exe, tail.trim()))
    } else {
        let (exe, tail) = trimmed.split_once(char::is_whitespace)?;
        Some((exe, tail.trim()))
    }
}

fn is_ours(command: Option<&Value>) -> bool {
    let Some(cmd) = command.and_then(Value::as_str) else {
        return false;
    };
    if cmd.to_ascii_lowercase().contains(BRIDGE_MARKER) {
        return true;
    }
    let Some((exe, args)) = split_command(cmd) else {
        return false;
    };
    if args != "statusline" && args != "hook invocation" {
        return false;
    }
    #[cfg(windows)]
    if let Some(expanded) = win32::long_path(Path::new(exe)) {
        if expanded.to_string_lossy().to_ascii_lowercase().contains(BRIDGE_MARKER) {
            return true;
        }
    }
    Path::new(exe)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            lower.starts_with("antigr") && lower.contains('~') && lower.ends_with(".exe")
        })
}

pub fn needs_repair(settings: &Value, hooks_doc: &Value, bridge: &Path) -> bool {
    let statusline_broken = is_ours(settings.pointer("/statusLine/command"))
        && settings.pointer("/statusLine/command").and_then(Value::as_str)
            != Some(bridge_command(bridge, "statusline").as_str());
    let hook_cmd = hooks_doc
        .pointer(&format!("/{HOOK_NAME}/PreInvocation/0/command"))
        .and_then(Value::as_str);
    let hooks_broken = hooks_doc.get(HOOK_NAME).is_some()
        && is_ours(hooks_doc.pointer(&format!("/{HOOK_NAME}/PreInvocation/0/command")))
        && hook_cmd != Some(bridge_command(bridge, "hook invocation").as_str());
    statusline_broken || hooks_broken
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

pub fn connect_settings(settings: &mut Value, bridge: &Path, preserve_chained: bool) -> Result<Option<Value>, String> {
    if settings.is_null() {
        *settings = Value::Object(Map::new());
    }
    let root = settings
        .as_object_mut()
        .ok_or("Antigravity CLI settings are not a JSON object")?;
    let previous = root
        .get("statusLine")
        .cloned()
        .filter(|line| !is_ours(line.get("command")));
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

pub fn disconnect_settings(settings: &mut Value, chained: Option<Value>) {
    let Some(root) = settings.as_object_mut() else { return };
    disconnect_status_line(root, chained, is_ours);
}

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
        assert_eq!(snap.resolve_pool(PoolKind::Gemini), PoolKind::Gemini);
        assert_eq!(snap.windows_for(PoolKind::Gemini).weekly.unwrap().used_percent, 40.0);
    }

    #[test]
    fn guard_evaluates_selected_pool_and_respects_thresholds() {
        let snap = parse_statusline(&sample_input(), NOW).unwrap();
        let gemini_guard = GuardConfig {
            enabled: true,
            pool: PoolKind::Gemini,
            five_hour_threshold_percent: 5,
            weekly_threshold_percent: 10,
            locale: "en-US".into(),
        };
        assert_eq!(guard_decision(Some(&gemini_guard), Some(&snap), NOW), GuardDecision::Allow);

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
        assert_eq!(connect_settings(&mut settings, bridge, true).unwrap(), None);
        assert!(settings.pointer("/statusLine/stack_with_default").is_none());
        disconnect_settings(&mut settings, prev);
        assert_eq!(settings["statusLine"], json!({ "type": "command", "command": "my-line.sh" }));
    }

    #[test]
    fn parses_cli_statusline_quota_remaining_fraction_and_reset_in_seconds() {
        let input = json!({
            "version": "0.10.1",
            "email": "user@example.com",
            "modelName": "gemini-3.1-pro",
            "quota": {
                "gemini-5h": { "remaining_fraction": 0.85, "reset_in_seconds": 3600 },
                "gemini-weekly": { "remaining_fraction": 0.5, "reset_in_seconds": 86_400 },
                "3p-5h": { "remaining_fraction": 0.9, "reset_in_seconds": 1800, "disabled": true }
            }
        });
        let snap = parse_statusline(&input, NOW).unwrap();
        assert_eq!(snap.account_id.as_deref(), Some("user@example.com"));
        assert_eq!(snap.active_model_name.as_deref(), Some("gemini-3.1-pro"));
        assert!((snap.gemini.five_hour.unwrap().used_percent - 15.0).abs() < 1e-6);
        assert_eq!(snap.gemini.five_hour.unwrap().resets_at, Some(NOW + 3600));
        assert!((snap.gemini.weekly.unwrap().used_percent - 50.0).abs() < 1e-6);
        assert!(snap.third_party.five_hour.is_none());
    }

    #[test]
    fn parses_usage_and_credits_commands_and_rejects_llm_fallback() {
        let usage = json!({
            "num_turns": 0,
            "status": "SUCCESS",
            "command": {
                "name": "usage",
                "data": {
                    "groups": [
                        {
                            "name": "Gemini Models",
                            "buckets": [
                                { "id": "gemini-weekly", "remaining_fraction": 0.99, "reset_time": "2026-11-20T22:55:49Z" },
                                { "id": "gemini-5h", "remaining_fraction": 0.97, "reset_time": "2026-11-14T03:55:49Z" }
                            ]
                        },
                        {
                            "name": "Claude and GPT models",
                            "buckets": [
                                { "id": "3p-weekly", "remaining_fraction": 1.0, "reset_time": "2026-11-20T23:06:43Z" },
                                { "id": "3p-5h", "remaining_fraction": 1.0, "reset_time": "2026-11-14T04:06:43Z" }
                            ]
                        },
                        {
                            "name": "Gemini 4 Argon",
                            "buckets": [
                                { "id": "gemini-argon-wtu-weekly", "remaining_fraction": 0.8, "reset_time": "2026-11-20T23:06:43Z" }
                            ]
                        }
                    ]
                }
            }
        });
        let snap = parse_usage_command(&usage, NOW).unwrap();
        assert!((snap.gemini.five_hour.unwrap().used_percent - 3.0).abs() < 1e-6);
        assert!((snap.gemini.weekly.unwrap().used_percent - 1.0).abs() < 1e-6);
        assert!((snap.argon.weekly.unwrap().used_percent - 20.0).abs() < 1e-6);
        assert_eq!(snap.third_party.five_hour.unwrap().used_percent, 0.0);

        let llm_fallback = json!({
            "num_turns": 1,
            "command": usage["command"]
        });
        assert!(parse_usage_command(&llm_fallback, NOW).is_none());

        let wrong_command = json!({
            "num_turns": 0,
            "command": { "name": "status", "data": usage["command"]["data"] }
        });
        assert!(parse_usage_command(&wrong_command, NOW).is_none());

        let credits = json!({
            "num_turns": 0,
            "command": { "name": "credits", "data": { "remaining_credits": 420 } }
        });
        assert_eq!(parse_credits_command(&credits), Some((Some("420".into()), Some("420".into()))));
        assert!(parse_credits_command(&json!({ "num_turns": 1, "command": credits["command"] })).is_none());
    }

    #[test]
    fn recognizes_unquoted_windows_short_path_and_quoted_legacy_commands() {
        assert!(is_ours(Some(&json!(r"C:\PROGRA~1\CODEX-~1\ANTIGR~1.EXE hook invocation"))));
        assert!(is_ours(Some(&json!(r"C:\PROGRA~1\CODEX-~1\ANTIGR~1.EXE statusline"))));
        assert!(is_ours(Some(&json!(r#""C:/Program Files/codex-usage-overlay/antigravity-usage-bridge.exe" hook invocation"#))));
        assert!(!is_ours(Some(&json!("python custom_hook.py"))));
        let no_spaces = Path::new(r"C:\overlay\antigravity-usage-bridge.exe");
        assert_eq!(bridge_command(no_spaces, "hook invocation"), r"C:\overlay\antigravity-usage-bridge.exe hook invocation");
    }

    #[test]
    fn parses_hook_model_and_ignores_auto_placeholder() {
        assert_eq!(parse_hook_model(&json!({ "modelName": "Claude Opus 4.6" })), (None, Some("Claude Opus 4.6".into())));
        assert_eq!(parse_hook_model(&json!({ "modelName": "auto" })), (None, None));
    }
}
