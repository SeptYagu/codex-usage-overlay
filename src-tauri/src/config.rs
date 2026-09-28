use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayLayout {
    Stacks,
    #[default]
    #[serde(other)]
    Grouped,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundMode {
    #[default]
    Windows,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettings {
    #[serde(default)]
    pub overlay_layout: OverlayLayout,
    #[serde(default = "default_scale")]
    pub scale_percent: u32,
    #[serde(default = "default_transparency")]
    pub background_transparency_percent: u32,
    #[serde(default = "default_true")]
    pub show_credits: bool,
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_seconds: u32,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_true")]
    pub auto_start: bool,
    #[serde(default = "default_true")]
    pub auto_check_updates: bool,
    #[serde(default = "default_true")]
    pub five_hour_reset_notification: bool,
    #[serde(default = "default_true")]
    pub weekly_reset_notification: bool,
    #[serde(default)]
    pub five_hour_sound_mode: SoundMode,
    #[serde(default)]
    pub weekly_sound_mode: SoundMode,
    #[serde(default)]
    pub five_hour_sound_path: Option<String>,
    #[serde(default)]
    pub weekly_sound_path: Option<String>,
    #[serde(default)]
    pub auto_edge_hide: bool,
}

fn default_scale() -> u32 { 175 }
fn default_transparency() -> u32 { 23 }
fn default_true() -> bool { true }
fn default_refresh_interval() -> u32 { 60 }
fn default_language() -> String { "auto".to_string() }

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            overlay_layout: OverlayLayout::default(),
            scale_percent: default_scale(),
            background_transparency_percent: default_transparency(),
            show_credits: default_true(),
            refresh_interval_seconds: default_refresh_interval(),
            language: default_language(),
            auto_start: default_true(),
            auto_check_updates: default_true(),
            five_hour_reset_notification: default_true(),
            weekly_reset_notification: default_true(),
            five_hour_sound_mode: SoundMode::default(),
            weekly_sound_mode: SoundMode::default(),
            five_hour_sound_path: None,
            weekly_sound_path: None,
            auto_edge_hide: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowPosition {
    pub left: f64,
    pub top: f64,
}

#[derive(Clone)]
pub struct ConfigManager {
    runtime_dir: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Self {
        let runtime_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("CodexUsageOverlay");
        let _ = fs::create_dir_all(&runtime_dir);
        Self { runtime_dir }
    }

    pub fn settings_path(&self) -> PathBuf {
        self.runtime_dir.join("settings.json")
    }

    pub fn position_path(&self) -> PathBuf {
        self.runtime_dir.join("window-position.json")
    }

    pub fn status_path(&self) -> PathBuf {
        self.runtime_dir.join("usage-status.json")
    }

    pub fn reset_state_path(&self) -> PathBuf {
        self.runtime_dir.join("reset-state.json")
    }

    pub fn load_settings(&self) -> OverlaySettings {
        let path = self.settings_path();
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(settings) = serde_json::from_str::<OverlaySettings>(&content) {
                    return settings;
                }
            }
        }
        OverlaySettings::default()
    }

    pub fn save_settings_checked(&self, settings: &OverlaySettings) -> Result<(), String> {
        let path = self.settings_path();
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        fs::write(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    pub fn load_position(&self) -> Option<WindowPosition> {
        let path = self.position_path();
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(pos) = serde_json::from_str::<WindowPosition>(&content) {
                    return Some(pos);
                }
            }
        }
        None
    }

    pub fn save_position(&self, pos: &WindowPosition) {
        let path = self.position_path();
        let tmp = path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string(pos) {
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(tmp, path);
            }
        }
    }

    pub fn write_status(&self, state: &str, error_msg: Option<&str>) {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct StatusInfo<'a> {
            state: &'a str,
            updated_at: String,
            error: &'a str,
        }

        let info = StatusInfo {
            state,
            updated_at: chrono_like_now(),
            error: error_msg.unwrap_or(""),
        };

        if let Ok(json) = serde_json::to_string(&info) {
            let _ = fs::write(self.status_path(), json);
        }
    }
}

fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("timestamp:{}", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn legacy_settings() -> serde_json::Value {
        json!({"scalePercent":220,"backgroundTransparencyPercent":50,
            "showCredits":false,"refreshIntervalSeconds":300,
            "language":"zh-CN","autoStart":false})
    }

    fn assert_preferences(settings: &OverlaySettings) {
        assert_eq!(settings.scale_percent, 220);
        assert_eq!(settings.background_transparency_percent, 50);
        assert!(!settings.show_credits);
        assert_eq!(settings.refresh_interval_seconds, 300);
        assert_eq!(settings.language, "zh-CN");
        assert!(!settings.auto_start);
        assert!(settings.auto_check_updates);
    }

    #[test]
    fn legacy_settings_default_to_grouped_without_resetting_preferences() {
        let settings: OverlaySettings = serde_json::from_value(legacy_settings()).unwrap();
        assert_eq!(settings.overlay_layout, OverlayLayout::Grouped);
        assert_preferences(&settings);
        assert_eq!(OverlaySettings::default().overlay_layout, OverlayLayout::Grouped);
        assert!(settings.five_hour_reset_notification);
        assert!(settings.weekly_reset_notification);
        assert_eq!(settings.five_hour_sound_mode, SoundMode::Windows);
        assert_eq!(settings.weekly_sound_mode, SoundMode::Windows);
        assert_eq!(settings.five_hour_sound_path, None);
        assert_eq!(settings.weekly_sound_path, None);
        assert!(!settings.auto_edge_hide);
    }

    #[test]
    fn both_layouts_round_trip_with_existing_preferences() {
        for (wire, layout) in [("grouped", OverlayLayout::Grouped), ("stacks", OverlayLayout::Stacks)] {
            let mut value = legacy_settings();
            value["overlayLayout"] = json!(wire);
            value["autoCheckUpdates"] = json!(true);
            let settings: OverlaySettings = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(settings.overlay_layout, layout);
            assert_preferences(&settings);
            let serialized = serde_json::to_value(&settings).unwrap();
            for key in value.as_object().unwrap().keys() {
                assert_eq!(serialized.get(key), value.get(key));
            }
            let restored: OverlaySettings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(restored.overlay_layout, layout);
            assert_preferences(&restored);
        }
    }

    #[test]
    fn unknown_layout_defaults_without_resetting_preferences() {
        let mut value = legacy_settings();
        value["overlayLayout"] = json!("future-layout");
        let settings: OverlaySettings = serde_json::from_value(value).unwrap();
        assert_eq!(settings.overlay_layout, OverlayLayout::Grouped);
        assert_preferences(&settings);
    }

    #[test]
    fn independent_reset_settings_round_trip() {
        let mut settings = OverlaySettings::default();
        settings.five_hour_reset_notification = false;
        settings.weekly_sound_mode = SoundMode::Custom;
        settings.weekly_sound_path = Some("C:\\sound.m4a".into());
        settings.auto_edge_hide = true;
        let restored: OverlaySettings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!restored.five_hour_reset_notification);
        assert!(restored.weekly_reset_notification);
        assert_eq!(restored.five_hour_sound_mode, SoundMode::Windows);
        assert_eq!(restored.weekly_sound_mode, SoundMode::Custom);
        assert_eq!(restored.five_hour_sound_path, None);
        assert_eq!(restored.weekly_sound_path, Some("C:\\sound.m4a".into()));
        assert!(restored.auto_edge_hide);
    }
}

/// Detects whether the current executable is running from an installed environment
/// (e.g. NSIS or MSI installer) vs standalone portable executable.
pub fn is_installed_environment() -> bool {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };

    let exe_dir = match exe.parent() {
        Some(d) => d,
        None => return false,
    };

    // 1. NSIS installer creates uninstall.exe next to the main binary
    if exe_dir.join("uninstall.exe").exists() {
        return true;
    }

    // 2. Installed in standard Windows program directory locations
    let exe_lower = exe.to_string_lossy().to_lowercase();
    if exe_lower.contains(r"\appdata\local\programs\codex-usage-overlay")
        || exe_lower.contains(r"\program files\codex-usage-overlay")
        || exe_lower.contains(r"\program files (x86)\codex-usage-overlay")
    {
        return true;
    }

    false
}

pub fn updater_target() -> &'static str {
    if !is_installed_environment() {
        return "windows-x86_64-portable";
    }

    let is_nsis = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("uninstall.exe")))
        .is_some_and(|uninstaller| uninstaller.exists());

    if is_nsis {
        "windows-x86_64-nsis"
    } else {
        "windows-x86_64-msi"
    }
}

