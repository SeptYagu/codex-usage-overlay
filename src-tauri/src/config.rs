use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettings {
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
}

fn default_scale() -> u32 { 175 }
fn default_transparency() -> u32 { 23 }
fn default_true() -> bool { true }
fn default_refresh_interval() -> u32 { 60 }
fn default_language() -> String { "auto".to_string() }

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            scale_percent: default_scale(),
            background_transparency_percent: default_transparency(),
            show_credits: default_true(),
            refresh_interval_seconds: default_refresh_interval(),
            language: default_language(),
            auto_start: default_true(),
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

    pub fn save_settings(&self, settings: &OverlaySettings) {
        let path = self.settings_path();
        let tmp = path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string_pretty(settings) {
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(tmp, path);
            }
        }
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
