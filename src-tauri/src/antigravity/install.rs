//! Connects the `antigravity-usage-bridge` helper to Antigravity CLI and
//! mirrors the guard preferences it reads.
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use super::shared::{self, BridgeConfig, GuardConfig, GuardState, PoolKind};
use crate::config::OverlaySettings;

pub const BRIDGE_EXE: &str = "antigravity-usage-bridge.exe";
const BACKUP_SUFFIX: &str = "codex-overlay.bak";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityConnectionStatus {
    pub status_line: bool,
    pub hooks: bool,
    pub bridge_available: bool,
    pub cli_available: bool,
    pub settings_path: Option<String>,
    pub hooks_path: Option<String>,
    pub snapshot_captured_at: Option<i64>,
    pub antigravity_version: Option<String>,
    pub active_model_id: Option<String>,
    pub active_model_name: Option<String>,
    pub detected_pool: Option<PoolKind>,
    pub effective_pool: Option<PoolKind>,
    pub blocked_count: u64,
    pub last_blocked_at: Option<i64>,
    pub error: Option<String>,
    pub read_failure: Option<crate::codex::reader::FailureKind>,
    pub read_http_status: Option<u16>,
}

/// Antigravity CLI settings file: `~/.gemini/antigravity-cli/settings.json`.
pub fn cli_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".gemini").join("antigravity-cli").join("settings.json"))
}

/// Antigravity lifecycle hooks file: `~/.gemini/config/hooks.json`.
pub fn hooks_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".gemini").join("config").join("hooks.json"))
}

/// The helper ships next to the overlay executable, in both installed and
/// portable layouts.
pub fn bridge_path() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|dir| dir.join(BRIDGE_EXE))
}

fn read_json_doc(path: &Path, label: &str) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => Ok(Value::Null),
        Ok(raw) => serde_json::from_str(&raw).map_err(|error| {
            format!("{label} is not valid JSON ({error}); fix {} before connecting", path.display())
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

pub fn status(
    settings_path: Option<&Path>,
    hooks_file_path: Option<&Path>,
    runtime_dir: &Path,
    bridge: Option<&Path>,
    configured_pool: PoolKind,
) -> AntigravityConnectionStatus {
    let settings_doc = settings_path.map(|p| read_json_doc(p, "Antigravity CLI settings"));
    let hooks_doc = hooks_file_path.map(|p| read_json_doc(p, "Antigravity hooks configuration"));
    let (connection, error) = match (settings_doc, hooks_doc) {
        (Some(Ok(settings)), Some(Ok(hooks))) => (shared::connection(&settings, &hooks), None),
        (Some(Err(err)), _) | (_, Some(Err(err))) => (Default::default(), Some(err)),
        (None, _) | (_, None) => (
            Default::default(),
            Some("Could not locate the Antigravity configuration folder".into()),
        ),
    };
    let snapshot = shared::read_snapshot(runtime_dir);
    let guard: GuardState = fs::read_to_string(runtime_dir.join(shared::GUARD_STATE_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let detected_pool = snapshot.as_ref().map(|s| s.resolve_pool(PoolKind::Auto));
    let effective_pool = snapshot.as_ref().map(|s| s.resolve_pool(configured_pool));
    AntigravityConnectionStatus {
        status_line: connection.status_line,
        hooks: connection.hooks,
        bridge_available: bridge.is_some_and(Path::is_file),
        cli_available: super::live::cli_available(),
        settings_path: settings_path.map(|p| p.display().to_string()),
        hooks_path: hooks_file_path.map(|p| p.display().to_string()),
        snapshot_captured_at: snapshot.as_ref().map(|s| s.captured_at),
        antigravity_version: snapshot.as_ref().and_then(|s| s.cli_version.clone()),
        active_model_id: snapshot.as_ref().and_then(|s| s.active_model_id.clone()),
        active_model_name: snapshot.and_then(|s| s.active_model_name),
        detected_pool,
        effective_pool,
        blocked_count: guard.blocked_count,
        last_blocked_at: guard.last_blocked_at,
        error,
        read_failure: None,
        read_http_status: None,
    }
}

fn resolve_command_target(runtime_dir: &Path, bridge: &Path) -> Result<PathBuf, String> {
    let resolved = shared::resolve_execution_path(bridge);
    let shim_path = runtime_dir.join(shared::BRIDGE_SHIM_FILE);
    if shared::is_unquoted_safe(&resolved) {
        let _ = fs::remove_file(&shim_path);
        return Ok(bridge.to_path_buf());
    }
    #[cfg(windows)]
    {
        let resolved_shim_dir = shared::resolve_execution_path(runtime_dir);
        let resolved_shim = resolved_shim_dir.join(shared::BRIDGE_SHIM_FILE);
        if shared::is_unquoted_safe(&resolved_shim) {
            fs::create_dir_all(runtime_dir).map_err(|e| e.to_string())?;
            let script = format!("@echo off\r\n\"{}\" %*\r\n", bridge.display());
            fs::write(&shim_path, script)
                .map_err(|error| format!("Could not write {}: {error}", shim_path.display()))?;
            return Ok(resolved_shim);
        }
    }
    Ok(bridge.to_path_buf())
}

/// Repairs already-connected bridge commands when upgrading from older versions
/// (for example switching from quoted Windows paths to unquoted short/shim paths).
pub fn repair_if_needed(
    settings_path: &Path,
    hooks_file_path: &Path,
    runtime_dir: &Path,
    bridge: &Path,
) -> Result<(), String> {
    if !bridge.is_file() {
        return Ok(());
    }
    let settings = read_json_doc(settings_path, "Antigravity CLI settings")?;
    let hooks_doc = read_json_doc(hooks_file_path, "Antigravity hooks configuration")?;
    let current = shared::connection(&settings, &hooks_doc);
    if !(current.status_line && current.hooks) {
        return Ok(());
    }
    let target = resolve_command_target(runtime_dir, bridge)?;
    if shared::needs_repair(&settings, &hooks_doc, &target) {
        connect(settings_path, hooks_file_path, runtime_dir, bridge)?;
    }
    Ok(())
}

fn backup_once(path: &Path) -> Result<(), String> {
    let backup = path.with_extension(format!("json.{BACKUP_SUFFIX}"));
    if path.is_file() && !backup.exists() {
        fs::copy(path, &backup)
            .map_err(|error| format!("Could not back up {}: {error}", path.display()))?;
    }
    Ok(())
}

/// Registers `antigravity-usage-bridge` in `~/.gemini/antigravity-cli/settings.json`
/// and registers only the `PreInvocation` guard hook in `~/.gemini/config/hooks.json`.
pub fn connect(
    settings_path: &Path,
    hooks_file_path: &Path,
    runtime_dir: &Path,
    bridge: &Path,
) -> Result<(), String> {
    if !bridge.is_file() {
        return Err(format!("{BRIDGE_EXE} is missing next to the overlay executable"));
    }
    let mut settings = read_json_doc(settings_path, "Antigravity CLI settings")?;
    let mut hooks_doc = read_json_doc(hooks_file_path, "Antigravity hooks configuration")?;
    backup_once(settings_path)?;
    backup_once(hooks_file_path)?;

    let target = resolve_command_target(runtime_dir, bridge)?;
    let existing_bridge = read_bridge_config(runtime_dir);
    let preserve_chained = existing_bridge.chained_status_line.is_some();
    let previous = shared::connect_settings(&mut settings, &target, preserve_chained)?;
    if previous.is_some() {
        let config = BridgeConfig { chained_status_line: previous };
        shared::write_json_atomic(&runtime_dir.join(shared::BRIDGE_FILE), &config)
            .map_err(|e| e.to_string())?;
    }
    shared::connect_hooks(&mut hooks_doc, &target)?;
    shared::write_json_atomic(settings_path, &settings)
        .map_err(|error| format!("Could not write {}: {error}", settings_path.display()))?;
    shared::write_json_atomic(hooks_file_path, &hooks_doc)
        .map_err(|error| format!("Could not write {}: {error}", hooks_file_path.display()))
}

/// Removes the helper's `statusLine` and `"codex-usage-overlay"` hook, restoring
/// any previously chained `statusLine`.
pub fn disconnect(settings_path: &Path, hooks_file_path: &Path, runtime_dir: &Path) -> Result<(), String> {
    let mut settings = read_json_doc(settings_path, "Antigravity CLI settings")?;
    if !settings.is_null() {
        let config = read_bridge_config(runtime_dir);
        shared::disconnect_settings(&mut settings, config.chained_status_line);
        shared::write_json_atomic(settings_path, &settings)
            .map_err(|error| format!("Could not write {}: {error}", settings_path.display()))?;
    }

    let mut hooks_doc = read_json_doc(hooks_file_path, "Antigravity hooks configuration")?;
    if !hooks_doc.is_null() {
        shared::disconnect_hooks(&mut hooks_doc);
        shared::write_json_atomic(hooks_file_path, &hooks_doc)
            .map_err(|error| format!("Could not write {}: {error}", hooks_file_path.display()))?;
    }

    for file in [shared::BRIDGE_FILE, shared::BRIDGE_SHIM_FILE] {
        if let Err(error) = fs::remove_file(runtime_dir.join(file)) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error.to_string());
            }
        }
    }
    Ok(())
}

/// Mirrors the guard switch, pool selector and thresholds for the `PreInvocation`
/// hook. Armed only when `Antigravity` is the active provider.
pub fn write_guard_config(runtime_dir: &Path, settings: &OverlaySettings) -> Result<(), String> {
    let config = GuardConfig {
        enabled: settings.provider == crate::config::Provider::Antigravity
            && settings.auto_pause_on_low_quota,
        pool: settings.antigravity_pool,
        five_hour_threshold_percent: settings.auto_pause_five_hour_threshold_percent,
        weekly_threshold_percent: settings.auto_pause_weekly_threshold_percent,
        locale: crate::commands::resolve_locale(&settings.language).to_string(),
    };
    shared::write_json_atomic(&runtime_dir.join(shared::GUARD_FILE), &config)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dirs(name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("antigravity-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let gemini = root.join(".gemini");
        let runtime = root.join("runtime");
        fs::create_dir_all(gemini.join("antigravity-cli")).unwrap();
        fs::create_dir_all(gemini.join("config")).unwrap();
        fs::create_dir_all(&runtime).unwrap();
        let bridge = root.join(BRIDGE_EXE);
        fs::write(&bridge, b"").unwrap();
        (
            gemini.join("antigravity-cli").join("settings.json"),
            gemini.join("config").join("hooks.json"),
            runtime,
            bridge,
        )
    }

    #[test]
    fn connect_backs_up_and_disconnect_restores_both_files() {
        let (settings_path, hooks_path, runtime, bridge) = dirs("roundtrip");
        let orig_settings = json!({
            "theme": "default",
            "statusLine": { "type": "command", "command": "bash ~/.gemini/line.sh" }
        });
        let orig_hooks = json!({
            "user-guard": {
                "PostToolUse": [{ "matcher": "*", "hooks": [{ "type": "command", "command": "audit.sh" }] }]
            }
        });
        fs::write(&settings_path, serde_json::to_string_pretty(&orig_settings).unwrap()).unwrap();
        fs::write(&hooks_path, serde_json::to_string_pretty(&orig_hooks).unwrap()).unwrap();

        connect(&settings_path, &hooks_path, &runtime, &bridge).unwrap();
        let current = status(Some(&settings_path), Some(&hooks_path), &runtime, Some(&bridge), PoolKind::Auto);
        assert!(current.status_line && current.hooks && current.bridge_available && current.error.is_none());
        assert_eq!(read_bridge_config(&runtime).chained_command(), Some("bash ~/.gemini/line.sh"));
        assert!(settings_path.with_extension("json.codex-overlay.bak").exists());
        assert!(hooks_path.with_extension("json.codex-overlay.bak").exists());

        disconnect(&settings_path, &hooks_path, &runtime).unwrap();
        let restored_settings: Value = serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        let restored_hooks: Value = serde_json::from_str(&fs::read_to_string(&hooks_path).unwrap()).unwrap();
        assert_eq!(restored_settings, orig_settings);
        assert_eq!(restored_hooks, orig_hooks);
        let _ = fs::remove_dir_all(runtime.parent().unwrap());
    }

    #[test]
    fn guard_config_only_arms_for_antigravity_provider() {
        let (_, _, runtime, _) = dirs("guard");
        let mut settings = OverlaySettings {
            auto_pause_on_low_quota: true,
            antigravity_pool: PoolKind::Argon,
            language: "zh-CN".into(),
            ..Default::default()
        };
        write_guard_config(&runtime, &settings).unwrap();
        let read = || serde_json::from_str::<GuardConfig>(&fs::read_to_string(runtime.join(shared::GUARD_FILE)).unwrap()).unwrap();
        assert!(!read().enabled);
        settings.provider = crate::config::Provider::Antigravity;
        write_guard_config(&runtime, &settings).unwrap();
        assert_eq!(
            read(),
            GuardConfig {
                enabled: true,
                pool: PoolKind::Argon,
                five_hour_threshold_percent: 5,
                weekly_threshold_percent: 2,
                locale: "zh-CN".into(),
            }
        );
        let _ = fs::remove_dir_all(runtime.parent().unwrap());
    }

    #[test]
    fn repair_if_needed_rewrites_legacy_quoted_commands() {
        let (settings_path, hooks_path, runtime, _) = dirs("repair");
        let spaced_dir = runtime.parent().unwrap().join("Program Filesdir");
        fs::create_dir_all(&spaced_dir).unwrap();
        let bridge = spaced_dir.join(BRIDGE_EXE);
        fs::write(&bridge, b"").unwrap();
        let legacy_settings = json!({
            "statusLine": { "type": "command", "command": r#""C:/Program Files/codex-usage-overlay/antigravity-usage-bridge.exe" statusline"#, "stack_with_default": true }
        });
        let legacy_hooks = json!({
            "codex-usage-overlay": {
                "PreInvocation": [{ "type": "command", "command": r#""C:/Program Files/codex-usage-overlay/antigravity-usage-bridge.exe" hook invocation"#, "timeout": 10 }]
            }
        });
        fs::write(&settings_path, serde_json::to_string_pretty(&legacy_settings).unwrap()).unwrap();
        fs::write(&hooks_path, serde_json::to_string_pretty(&legacy_hooks).unwrap()).unwrap();

        repair_if_needed(&settings_path, &hooks_path, &runtime, &bridge).unwrap();
        let updated_settings: Value = serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        let updated_hooks: Value = serde_json::from_str(&fs::read_to_string(&hooks_path).unwrap()).unwrap();
        let status_cmd = updated_settings.pointer("/statusLine/command").and_then(Value::as_str).unwrap();
        let hook_cmd = updated_hooks.pointer("/codex-usage-overlay/PreInvocation/0/command").and_then(Value::as_str).unwrap();
        assert!(!status_cmd.contains('"'), "unexpected quotes in statusLine: {status_cmd}");
        assert!(!hook_cmd.contains('"'), "unexpected quotes in hook: {hook_cmd}");
        assert!(status_cmd.ends_with("statusline"));
        assert!(hook_cmd.ends_with("hook invocation"));
        let _ = fs::remove_dir_all(runtime.parent().unwrap());
    }
}
