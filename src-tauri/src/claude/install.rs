//! Connects the `claude-usage-bridge` helper to Claude Code and mirrors the
//! guard preferences it reads.
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use super::shared::{self, BridgeConfig, GuardConfig, GuardState};
use crate::config::OverlaySettings;

pub const BRIDGE_EXE: &str = "claude-usage-bridge.exe";
const BACKUP_SUFFIX: &str = "codex-overlay.bak";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeConnectionStatus {
    pub status_line: bool,
    pub hooks: bool,
    pub bridge_available: bool,
    pub settings_path: Option<String>,
    pub snapshot_captured_at: Option<i64>,
    pub claude_version: Option<String>,
    pub blocked_count: u64,
    pub last_blocked_at: Option<i64>,
    pub error: Option<String>,
}

/// Claude Code's user settings file, honoring `CLAUDE_CONFIG_DIR`.
pub fn claude_settings_path() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
        .map(|dir| dir.join("settings.json"))
}

/// The helper ships next to the overlay executable, in the installed and the
/// portable layout alike.
pub fn bridge_path() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|dir| dir.join(BRIDGE_EXE))
}

fn read_settings(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => Ok(Value::Null),
        Ok(raw) => serde_json::from_str(&raw).map_err(|error| {
            format!("Claude Code settings are not valid JSON ({error}); fix {} before connecting", path.display())
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

fn read_bridge_config(runtime_dir: &Path) -> BridgeConfig {
    fs::read_to_string(runtime_dir.join(shared::BRIDGE_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn status(settings_path: Option<&Path>, runtime_dir: &Path, bridge: Option<&Path>) -> ClaudeConnectionStatus {
    let (connection, error) = match settings_path.map(read_settings) {
        Some(Ok(settings)) => (shared::connection(&settings), None),
        Some(Err(error)) => (Default::default(), Some(error)),
        None => (Default::default(), Some("Could not locate the Claude Code settings folder".into())),
    };
    let snapshot = shared::read_snapshot(runtime_dir);
    let guard: GuardState = fs::read_to_string(runtime_dir.join(shared::GUARD_STATE_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    ClaudeConnectionStatus {
        status_line: connection.status_line,
        hooks: connection.hooks,
        bridge_available: bridge.is_some_and(Path::is_file),
        settings_path: settings_path.map(|p| p.display().to_string()),
        snapshot_captured_at: snapshot.as_ref().map(|s| s.captured_at),
        claude_version: snapshot.and_then(|s| s.claude_version),
        blocked_count: guard.blocked_count,
        last_blocked_at: guard.last_blocked_at,
        error,
    }
}

/// Registers the helper as statusline and guard hooks. The user's previous
/// statusline is kept in the helper's config and keeps rendering through it,
/// and the original settings file is backed up once before the first change.
pub fn connect(settings_path: &Path, runtime_dir: &Path, bridge: &Path) -> Result<(), String> {
    if !bridge.is_file() {
        return Err(format!("{BRIDGE_EXE} is missing next to the overlay executable"));
    }
    let mut settings = read_settings(settings_path)?;
    let backup = settings_path.with_extension(format!("json.{BACKUP_SUFFIX}"));
    if settings_path.is_file() && !backup.exists() {
        fs::copy(settings_path, &backup).map_err(|error| format!("Could not back up {}: {error}", settings_path.display()))?;
    }
    let previous = shared::connect(&mut settings, bridge)?;
    if previous.is_some() {
        let config = BridgeConfig { chained_status_line: previous };
        shared::write_json_atomic(&runtime_dir.join(shared::BRIDGE_FILE), &config).map_err(|e| e.to_string())?;
    }
    shared::write_json_atomic(settings_path, &settings).map_err(|error| format!("Could not write {}: {error}", settings_path.display()))
}

/// Removes the helper's statusline and hooks and restores the statusline it
/// replaced. Everything else in the file stays as it is.
pub fn disconnect(settings_path: &Path, runtime_dir: &Path) -> Result<(), String> {
    let mut settings = read_settings(settings_path)?;
    if settings.is_null() {
        return Ok(());
    }
    let config = read_bridge_config(runtime_dir);
    shared::disconnect(&mut settings, config.chained_status_line);
    shared::write_json_atomic(settings_path, &settings).map_err(|error| format!("Could not write {}: {error}", settings_path.display()))?;
    match fs::remove_file(runtime_dir.join(shared::BRIDGE_FILE)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
        _ => Ok(()),
    }
}

/// Mirrors the guard switch and thresholds for the hook. The guard is only
/// armed while Claude is the active provider.
pub fn write_guard_config(runtime_dir: &Path, settings: &OverlaySettings) -> Result<(), String> {
    let config = GuardConfig {
        enabled: settings.provider == crate::config::Provider::Claude && settings.auto_pause_on_low_quota,
        five_hour_threshold_percent: settings.auto_pause_five_hour_threshold_percent,
        weekly_threshold_percent: settings.auto_pause_weekly_threshold_percent,
        locale: crate::commands::resolve_locale(&settings.language).to_string(),
    };
    shared::write_json_atomic(&runtime_dir.join(shared::GUARD_FILE), &config).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dirs(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("claude-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let claude = root.join(".claude");
        let runtime = root.join("runtime");
        fs::create_dir_all(&claude).unwrap();
        fs::create_dir_all(&runtime).unwrap();
        let bridge = root.join(BRIDGE_EXE);
        fs::write(&bridge, b"").unwrap();
        (claude.join("settings.json"), runtime, bridge)
    }

    #[test]
    fn connect_backs_up_chains_the_statusline_and_disconnect_restores_it() {
        let (settings_path, runtime, bridge) = dirs("roundtrip");
        let original = json!({
            "permissions": { "allow": ["Bash(npm test)"] },
            "statusLine": { "type": "command", "command": "bash ~/.claude/line.sh" }
        });
        fs::write(&settings_path, serde_json::to_string_pretty(&original).unwrap()).unwrap();

        connect(&settings_path, &runtime, &bridge).unwrap();
        let current = status(Some(&settings_path), &runtime, Some(&bridge));
        assert!(current.status_line && current.hooks && current.bridge_available && current.error.is_none());
        assert_eq!(read_bridge_config(&runtime).chained_command(), Some("bash ~/.claude/line.sh"));
        let backup = settings_path.with_extension("json.codex-overlay.bak");
        assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&backup).unwrap()).unwrap(), original);

        // A second connect keeps the chained statusline instead of chaining the helper.
        connect(&settings_path, &runtime, &bridge).unwrap();
        assert_eq!(read_bridge_config(&runtime).chained_command(), Some("bash ~/.claude/line.sh"));

        disconnect(&settings_path, &runtime).unwrap();
        let restored: Value = serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(restored, original);
        // Key order of the user's file survives the round trip.
        assert!(fs::read_to_string(&settings_path).unwrap().find("permissions") < fs::read_to_string(&settings_path).unwrap().find("statusLine"));
        assert!(!runtime.join(shared::BRIDGE_FILE).exists());
        let _ = fs::remove_dir_all(settings_path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn invalid_settings_or_a_missing_helper_are_never_overwritten() {
        let (settings_path, runtime, bridge) = dirs("invalid");
        fs::write(&settings_path, "{ not json").unwrap();
        assert!(connect(&settings_path, &runtime, &bridge).unwrap_err().contains("not valid JSON"));
        assert_eq!(fs::read_to_string(&settings_path).unwrap(), "{ not json");
        assert!(status(Some(&settings_path), &runtime, Some(&bridge)).error.is_some());

        fs::remove_file(&settings_path).unwrap();
        assert!(connect(&settings_path, &runtime, &runtime.join("missing.exe")).is_err());
        assert!(!settings_path.exists());
        // Without a previous file there is nothing to back up, and disconnecting
        // a missing file changes nothing.
        connect(&settings_path, &runtime, &bridge).unwrap();
        assert!(!settings_path.with_extension("json.codex-overlay.bak").exists());
        disconnect(&settings_path, &runtime).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&settings_path).unwrap()).unwrap(), json!({}));
        let _ = fs::remove_dir_all(settings_path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn the_guard_is_only_armed_for_the_claude_provider() {
        let (_, runtime, _) = dirs("guard");
        let mut settings = OverlaySettings { auto_pause_on_low_quota: true, language: "zh-CN".into(), ..Default::default() };
        write_guard_config(&runtime, &settings).unwrap();
        let read = || serde_json::from_str::<GuardConfig>(&fs::read_to_string(runtime.join(shared::GUARD_FILE)).unwrap()).unwrap();
        assert!(!read().enabled);
        settings.provider = crate::config::Provider::Claude;
        write_guard_config(&runtime, &settings).unwrap();
        assert_eq!(read(), GuardConfig { enabled: true, five_hour_threshold_percent: 5, weekly_threshold_percent: 2, locale: "zh-CN".into() });
        let _ = fs::remove_dir_all(runtime.parent().unwrap());
    }
}
