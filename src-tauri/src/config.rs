use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayLayout {
    Stacks,
    #[default]
    #[serde(other)]
    Grouped,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundMode {
    Custom,
    #[default]
    #[serde(other)]
    Windows,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum FiveHourBurnRateAlgorithm { #[default] Legacy, Weighted, Personalized }
fn recover_algorithm<'de,D:serde::Deserializer<'de>>(d:D)->Result<FiveHourBurnRateAlgorithm,D::Error>{
    Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).unwrap_or_default())
}
fn recover_message<'de,D:serde::Deserializer<'de>>(d:D)->Result<String,D::Error>{
    let v=serde_json::Value::deserialize(d)?;Ok(v.as_str().filter(|s|crate::pause_message::valid_message(s)).unwrap_or("").into())
}

pub use crate::antigravity::shared::PoolKind as AntigravityPool;

/// Whose limits the overlay reads. One provider is active at a time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Dsh,
    Antigravity,
    #[default]
    Codex,
}

impl Provider {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::Dsh, Self::Antigravity];
    pub fn index(self) -> u8 {
        match self {
            Self::Codex => 0,
            Self::Claude => 1,
            Self::Dsh => 2,
            Self::Antigravity => 3,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Dsh => "dsh",
            Self::Antigravity => "antigravity",
        }
    }
}

pub const CURRENT_SCHEMA_VERSION: u32 = 5;

fn recover_refresh_interval<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_u64()
        .filter(|n| (15..=3600).contains(n))
        .map(|n| n as u32)
        .unwrap_or_else(default_refresh_interval))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreference {
    #[serde(default = "default_true")]
    pub monitor_enabled: bool,
    #[serde(default = "default_true")]
    pub carousel_included: bool,
    #[serde(default, deserialize_with = "recover_algorithm")]
    pub five_hour_burn_rate_algorithm: FiveHourBurnRateAlgorithm,
    #[serde(default)]
    pub protection_enabled: bool,
    #[serde(default = "default_auto_pause_five_hour_threshold")]
    pub five_hour_threshold_percent: u32,
    #[serde(default = "default_auto_pause_weekly_threshold")]
    pub weekly_threshold_percent: u32,
    #[serde(
        default = "default_refresh_interval",
        deserialize_with = "recover_refresh_interval"
    )]
    pub refresh_interval_seconds: u32,
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
}

impl ProviderPreference {
    pub fn default_for(provider: Provider) -> Self {
        Self {
            monitor_enabled: true,
            carousel_included: true,
            five_hour_burn_rate_algorithm: FiveHourBurnRateAlgorithm::Legacy,
            protection_enabled: false,
            five_hour_threshold_percent: if provider == Provider::Dsh {
                0
            } else {
                default_auto_pause_five_hour_threshold()
            },
            weekly_threshold_percent: if provider == Provider::Dsh {
                0
            } else {
                default_auto_pause_weekly_threshold()
            },
            refresh_interval_seconds: default_refresh_interval(),
            five_hour_reset_notification: true,
            weekly_reset_notification: true,
            five_hour_sound_mode: SoundMode::Windows,
            weekly_sound_mode: SoundMode::Windows,
            five_hour_sound_path: None,
            weekly_sound_path: None,
        }
    }
}

impl Default for ProviderPreference {
    fn default() -> Self {
        Self::default_for(Provider::Codex)
    }
}

/// A non-secret identity used by persistence and UI delivery. Never include a
/// launch URL/cookie in this key. The explicit provider prevents a DSH account
/// from aliasing a local Codex account with the same display name or id.
pub fn source_key_for(provider: Provider, settings: &OverlaySettings) -> String {
    match provider {
        Provider::Codex => "codex".into(),
        Provider::Claude => "claude".into(),
        Provider::Antigravity => format!("antigravity-{}", settings.antigravity_pool.key()),
        Provider::Dsh => {
            let identity = serde_json::to_string(&(
                settings.dsh_base_url.trim().trim_end_matches('/'),
                settings.dsh_instance_id.as_str(),
                settings.dsh_account_id.as_str(),
            ))
            .unwrap_or_default();
            // Stable FNV-1a, for namespace separation only, not authentication.
            let hash = identity.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
            format!("dsh-{hash:016x}")
        }
    }
}

pub fn source_key(settings: &OverlaySettings) -> String {
    source_key_for(settings.provider, settings)
}

/// A DSH transport address can change on reconnect without changing the
/// subscription account. Estimator history follows the verified instance and
/// account, while UI delivery keeps using the connection-specific source key.
pub fn estimator_source_key_for(provider: Provider, settings: &OverlaySettings) -> String {
    if provider != Provider::Dsh
        || settings.dsh_instance_id.is_empty()
        || settings.dsh_account_id.is_empty()
    {
        return source_key_for(provider, settings);
    }
    let identity = serde_json::to_string(&(
        settings.dsh_instance_id.as_str(),
        settings.dsh_account_id.as_str(),
    ))
    .unwrap_or_default();
    let hash = identity.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("dsh-burn-{hash:016x}")
}

pub fn same_source_for(
    provider: Provider,
    left: &OverlaySettings,
    right: &OverlaySettings,
) -> bool {
    match provider {
        Provider::Codex | Provider::Claude => true,
        Provider::Dsh => {
            left.dsh_base_url == right.dsh_base_url
                && left.dsh_instance_id == right.dsh_instance_id
                && left.dsh_account_id == right.dsh_account_id
                && left.dsh_session_ids == right.dsh_session_ids
        }
        Provider::Antigravity => left.antigravity_pool == right.antigravity_pool,
    }
}

pub fn same_source(left: &OverlaySettings, right: &OverlaySettings) -> bool {
    left.provider == right.provider && same_source_for(left.provider, left, right)
}

fn default_carousel_interval() -> u32 { 8 }
fn recover_carousel_interval<'de,D:serde::Deserializer<'de>>(d:D)->Result<u32,D::Error>{
    let v=serde_json::Value::deserialize(d)?;
    Ok(v.as_u64().filter(|v|(3..=30).contains(v)).map(|v|v as u32).unwrap_or(8))
}
fn recover_carousel_enabled<'de,D:serde::Deserializer<'de>>(d:D)->Result<bool,D::Error>{
    Ok(serde_json::Value::deserialize(d)?.as_bool().unwrap_or(false))
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

fn default_provider_preferences() -> std::collections::BTreeMap<String, ProviderPreference> {
    Provider::ALL
        .into_iter()
        .map(|provider| (provider.key().into(), ProviderPreference::default_for(provider)))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettings {
    #[serde(default, deserialize_with = "recover_carousel_enabled")]
    pub quota_carousel_enabled: bool,
    #[serde(default = "default_carousel_interval", deserialize_with = "recover_carousel_interval")]
    pub quota_carousel_interval_seconds: u32,
    #[serde(default, deserialize_with = "recover_message")]
    pub auto_pause_five_hour_message: String,
    #[serde(default, deserialize_with = "recover_message")]
    pub auto_pause_weekly_message: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_provider_preferences")]
    pub provider_preferences: std::collections::BTreeMap<String, ProviderPreference>,
    /// Codex by default, so an upgraded profile keeps reading Codex.
    #[serde(default)]
    pub provider: Provider,
    /// Public origin only. Launch tokens and API cookies are memory-only in the
    /// DSH manager, and never travel in settings or usage events.
    #[serde(default)]
    pub dsh_base_url: String,
    #[serde(default)]
    pub dsh_instance_id: String,
    /// Public companion profile fingerprint used only to prefer the same profile.
    #[serde(default)]
    pub dsh_profile_id: String,
    #[serde(default)]
    pub dsh_account_id: String,
    #[serde(default)]
    pub dsh_session_ids: Vec<String>,
    /// Whether the Claude provider reads limits from the usage endpoint with
    /// Claude Code's login token, instead of only from the statusline bridge.
    /// Off by default: it reads a credential, so it is never armed silently.
    #[serde(default)]
    pub claude_live_polling: bool,
    /// Whether the Antigravity provider queries `agy -p "/usage"` periodically
    /// (zero turns / zero tokens) so desktop and IDE sessions see live quotas.
    #[serde(default = "default_true")]
    pub antigravity_live_polling: bool,
    /// Which Antigravity quota pool (`auto`, `gemini`, `argon`, `3p`) to display and guard.
    #[serde(default)]
    pub antigravity_pool: AntigravityPool,
    /// Whether the capsule shows the active provider's logo.
    #[serde(default = "default_true")]
    pub show_provider_logo: bool,
    #[serde(default)]
    pub overlay_layout: OverlayLayout,
    #[serde(default = "default_scale")]
    pub scale_percent: u32,
    #[serde(default = "default_transparency")]
    pub background_transparency_percent: u32,
    #[serde(default = "default_true")]
    pub show_credits: bool,
    #[serde(default = "default_true")]
    pub show_five_hour_quota: bool,
    #[serde(default = "default_true")]
    pub show_burn_rate: bool,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_true")]
    pub auto_start: bool,
    #[serde(default = "default_true")]
    pub auto_check_updates: bool,
    #[serde(default)]
    pub auto_install_updates: bool,
    /// Whether update checks also offer preview releases. Off by default, so
    /// an upgraded profile stays on the stable channel.
    #[serde(default)]
    pub include_prerelease_updates: bool,
    #[serde(default)]
    pub auto_edge_hide: bool,
    #[serde(default)]
    pub mouse_passthrough: bool,
    /// Whether the docked pill draws the 10% grid on its quota bars. Off by
    /// default, so an upgraded profile keeps the plain bars.
    #[serde(default)]
    pub show_percentage_grid: bool,
    /// Whether successful reads are kept as local usage history for the
    /// History window. On by default; turning it off keeps existing files.
    #[serde(default = "default_true")]
    pub record_usage_history: bool,
    /// Whether the History window's timeline shows used percent instead of
    /// remaining percent. Off by default: remaining, like the overlay.
    #[serde(default)]
    pub history_show_used: bool,
}

fn default_scale() -> u32 {
    175
}
fn default_transparency() -> u32 {
    23
}
fn default_true() -> bool {
    true
}
fn default_refresh_interval() -> u32 {
    60
}
fn default_language() -> String {
    "auto".to_string()
}
fn default_auto_pause_five_hour_threshold() -> u32 {
    5
}
fn default_auto_pause_weekly_threshold() -> u32 {
    2
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            quota_carousel_enabled: false,
            quota_carousel_interval_seconds: 8,
            // Left empty so `migrate` fills them from the resolved language
            // setting, not from the OS locale at construction time.
            auto_pause_five_hour_message: String::new(),
            auto_pause_weekly_message: String::new(),
            schema_version: CURRENT_SCHEMA_VERSION,
            provider_preferences: default_provider_preferences(),
            provider: Provider::default(),
            dsh_base_url: String::new(),
            dsh_instance_id: String::new(),
            dsh_profile_id: String::new(),
            dsh_account_id: String::new(),
            dsh_session_ids: Vec::new(),
            claude_live_polling: false,
            antigravity_live_polling: default_true(),
            antigravity_pool: AntigravityPool::default(),
            show_provider_logo: default_true(),
            overlay_layout: OverlayLayout::default(),
            scale_percent: default_scale(),
            background_transparency_percent: default_transparency(),
            show_credits: default_true(),
            show_five_hour_quota: default_true(),
            show_burn_rate: default_true(),
            language: default_language(),
            auto_start: default_true(),
            auto_check_updates: default_true(),
            auto_install_updates: false,
            include_prerelease_updates: false,
            auto_edge_hide: false,
            mouse_passthrough: false,
            show_percentage_grid: false,
            record_usage_history: true,
            history_show_used: false,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LegacyTopLevelFields {
    #[serde(deserialize_with = "recover_algorithm")]
    five_hour_burn_rate_algorithm: FiveHourBurnRateAlgorithm,
    refresh_interval_seconds: Option<u64>,
    five_hour_reset_notification: Option<bool>,
    weekly_reset_notification: Option<bool>,
    five_hour_sound_mode: SoundMode,
    weekly_sound_mode: SoundMode,
    five_hour_sound_path: serde_json::Value,
    weekly_sound_path: serde_json::Value,
    auto_pause_on_low_quota: bool,
    auto_pause_five_hour_threshold_percent: Option<u32>,
    auto_pause_weekly_threshold_percent: Option<u32>,
    dsh_five_hour_threshold_percent: Option<u32>,
    dsh_weekly_threshold_percent: Option<u32>,
}

fn opt_string_field(value: &serde_json::Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty() && s.len() <= 4096).map(str::to_string)
}

/// Drops wrong-typed values inside `providerPreferences` so one bad field falls
/// back to its inherited or default value instead of failing the whole file.
fn sanitize_provider_preferences(raw: &serde_json::Value) -> serde_json::Value {
    let mut value = raw.clone();
    let Some(root) = value.as_object_mut() else {
        return value;
    };
    let Some(prefs) = root.get_mut("providerPreferences") else {
        return value;
    };
    let Some(entries) = prefs.as_object_mut() else {
        root.remove("providerPreferences");
        return value;
    };
    entries.retain(|_, entry| entry.is_object());
    for entry in entries.values_mut().filter_map(|entry| entry.as_object_mut()) {
        entry.retain(|field, value| match field.as_str() {
            "monitorEnabled" | "carouselIncluded" | "protectionEnabled"
            | "fiveHourResetNotification" | "weeklyResetNotification" => value.is_boolean(),
            "fiveHourThresholdPercent" | "weeklyThresholdPercent" => {
                value.as_u64().is_some_and(|n| u32::try_from(n).is_ok())
            }
            "fiveHourSoundMode" | "weeklySoundMode" => value.is_string(),
            "fiveHourSoundPath" | "weeklySoundPath" => value.is_null() || value.is_string(),
            _ => true,
        });
    }
    value
}

impl OverlaySettings {
    /// Parses a persisted `settings.json` JSON value, losslessly migrating schema v0..=v4
    /// settings into canonical v5 `providerPreferences`.
    pub fn from_json_migrated(raw: &serde_json::Value) -> Result<Self, String> {
        let raw_version = raw
            .get("schemaVersion")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(u64::from(u32::MAX)) as u32)
            .unwrap_or(0);
        let sanitized = sanitize_provider_preferences(raw);
        let raw = &sanitized;
        let mut settings: Self =
            serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
        if !settings.auto_check_updates {
            settings.auto_install_updates = false;
        }
        if raw_version > CURRENT_SCHEMA_VERSION {
            settings.schema_version = raw_version;
            for provider in Provider::ALL {
                settings
                    .provider_preferences
                    .entry(provider.key().into())
                    .or_insert_with(|| ProviderPreference::default_for(provider));
            }
            return Ok(settings);
        }

        let legacy: LegacyTopLevelFields =
            serde_json::from_value(raw.clone()).unwrap_or_default();
        let raw_prefs = raw.get("providerPreferences").and_then(|v| v.as_object());
        let empty_prefs = raw_prefs.is_none_or(|map| map.is_empty());
        let inherit = raw_version < 3 || empty_prefs;

        for provider in Provider::ALL {
            let key = provider.key();
            let raw_entry = raw_prefs
                .and_then(|map| map.get(key))
                .and_then(|v| v.as_object());
            let has_key = |field: &str| raw_entry.is_some_and(|m| m.contains_key(field));
            let has_valid_sound_mode = |field: &str| {
                raw_entry
                    .and_then(|m| m.get(field))
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| matches!(s, "windows" | "custom"))
            };
            let has_valid_refresh = raw_entry
                .and_then(|m| m.get("refreshIntervalSeconds"))
                .and_then(|v| v.as_u64())
                .is_some_and(|n| (15..=3600).contains(&n));

            let pref = settings
                .provider_preferences
                .entry(key.into())
                .or_insert_with(|| ProviderPreference::default_for(provider));

            // An explicit nested switch always wins over the legacy top-level one.
            if !has_key("protectionEnabled") && (raw_version < 4 || empty_prefs) {
                pref.protection_enabled =
                    provider == settings.provider && legacy.auto_pause_on_low_quota;
            }
            if !has_key("fiveHourThresholdPercent")
                && (raw_version < 4 || empty_prefs || raw_entry.is_none())
            {
                pref.five_hour_threshold_percent = if provider == Provider::Dsh {
                    legacy.dsh_five_hour_threshold_percent.unwrap_or(0)
                } else {
                    legacy
                        .auto_pause_five_hour_threshold_percent
                        .unwrap_or_else(default_auto_pause_five_hour_threshold)
                };
            }
            if !has_key("weeklyThresholdPercent")
                && (raw_version < 4 || empty_prefs || raw_entry.is_none())
            {
                pref.weekly_threshold_percent = if provider == Provider::Dsh {
                    legacy.dsh_weekly_threshold_percent.unwrap_or(0)
                } else {
                    legacy
                        .auto_pause_weekly_threshold_percent
                        .unwrap_or_else(default_auto_pause_weekly_threshold)
                };
            }
            if raw_version < 4
                && provider == Provider::Codex
                && !raw_entry.is_some_and(|m| m.contains_key("fiveHourBurnRateAlgorithm"))
            {
                pref.five_hour_burn_rate_algorithm = legacy.five_hour_burn_rate_algorithm;
            }
            if inherit {
                if !has_valid_refresh {
                    pref.refresh_interval_seconds = legacy
                        .refresh_interval_seconds
                        .filter(|n| (15..=3600).contains(n))
                        .map(|n| n as u32)
                        .unwrap_or_else(default_refresh_interval);
                }
                if !raw_entry.is_some_and(|m| m.contains_key("fiveHourResetNotification")) {
                    pref.five_hour_reset_notification =
                        legacy.five_hour_reset_notification.unwrap_or(true);
                }
                if !raw_entry.is_some_and(|m| m.contains_key("weeklyResetNotification")) {
                    pref.weekly_reset_notification =
                        legacy.weekly_reset_notification.unwrap_or(true);
                }
                if !has_valid_sound_mode("fiveHourSoundMode") {
                    pref.five_hour_sound_mode = legacy.five_hour_sound_mode;
                }
                if !has_valid_sound_mode("weeklySoundMode") {
                    pref.weekly_sound_mode = legacy.weekly_sound_mode;
                }
                if !raw_entry.is_some_and(|m| m.contains_key("fiveHourSoundPath")) {
                    pref.five_hour_sound_path = opt_string_field(&legacy.five_hour_sound_path);
                }
                if !raw_entry.is_some_and(|m| m.contains_key("weeklySoundPath")) {
                    pref.weekly_sound_path = opt_string_field(&legacy.weekly_sound_path);
                }
            }
        }
        settings.migrate();
        Ok(settings)
    }

    pub fn migrate(&mut self) {
        if self.schema_version > CURRENT_SCHEMA_VERSION {
            return;
        }
        if !(3..=30).contains(&self.quota_carousel_interval_seconds) {
            self.quota_carousel_interval_seconds = 8;
        }
        if !crate::pause_message::valid_message(&self.auto_pause_five_hour_message) {
            self.auto_pause_five_hour_message = crate::pause_message::default_pause(&self.language);
        }
        if !crate::pause_message::valid_message(&self.auto_pause_weekly_message) {
            self.auto_pause_weekly_message = crate::pause_message::default_pause(&self.language);
        }
        for provider in Provider::ALL {
            let preference = self
                .provider_preferences
                .entry(provider.key().into())
                .or_insert_with(|| ProviderPreference::default_for(provider));
            if !(15..=3600).contains(&preference.refresh_interval_seconds) {
                preference.refresh_interval_seconds = default_refresh_interval();
            }
            for path in [
                &mut preference.five_hour_sound_path,
                &mut preference.weekly_sound_path,
            ] {
                if path.as_ref().is_some_and(|s| s.is_empty() || s.len() > 4096) {
                    *path = None;
                }
            }
            if !preference.monitor_enabled {
                preference.protection_enabled = false;
            }
            preference.five_hour_threshold_percent = preference
                .five_hour_threshold_percent
                .clamp(if provider == Provider::Dsh { 0 } else { 1 }, 15);
            preference.weekly_threshold_percent = preference
                .weekly_threshold_percent
                .clamp(if provider == Provider::Dsh { 0 } else { 1 }, 10);
        }
        self.schema_version = CURRENT_SCHEMA_VERSION;
    }

    pub fn preference(&self, provider: Provider) -> ProviderPreference {
        self.provider_preferences
            .get(provider.key())
            .cloned()
            .unwrap_or_else(|| ProviderPreference::default_for(provider))
    }

    pub fn preference_mut(&mut self, provider: Provider) -> &mut ProviderPreference {
        self.provider_preferences
            .entry(provider.key().into())
            .or_insert_with(|| ProviderPreference::default_for(provider))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowPosition {
    pub left: f64,
    pub top: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsWindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Scale factor of the monitor the window was on when it was saved. It is the
    /// anchor that lets the restore path convert the logical rectangle back into
    /// the physical space it was measured in; records written by older versions
    /// have no such field (`None`).
    #[serde(default)]
    pub scale_factor: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DockPersistence {
    #[serde(default)]
    pub hidden: bool,
    pub docked: bool,
    pub edge: Option<crate::dock::Edge>,
}

#[derive(Clone)]
pub struct ConfigManager {
    runtime_dir: PathBuf,
    /// Shared by every clone. DSH uses a source/connection/account namespace,
    /// never the original Codex files, even before it has been configured.
    namespace: Arc<Mutex<String>>,
    /// Provider runtimes bind this once; cloning or changing a legacy root
    /// manager cannot redirect an in-flight provider write.
    bound_namespace: Option<String>,
    bound_estimator_namespace: Option<String>,
}

impl ConfigManager {
    pub fn new() -> Self {
        let runtime_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("CodexUsageOverlay");
        // Isolated native smoke sessions never touch the installed app's files.
        #[cfg(debug_assertions)]
        let runtime_dir = std::env::var_os("CODEX_OVERLAY_TEST_RUNTIME_DIR")
            .filter(|value| !value.is_empty()).map(PathBuf::from).unwrap_or(runtime_dir);
        let _ = fs::create_dir_all(&runtime_dir);
        Self { runtime_dir, namespace: Arc::default(), bound_namespace: None, bound_estimator_namespace: None }
    }

    #[cfg(test)]
    pub fn with_runtime_dir(runtime_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&runtime_dir);
        Self { runtime_dir, namespace: Arc::default(), bound_namespace: None, bound_estimator_namespace: None }
    }

    pub fn for_provider(&self, provider: Provider, settings: &OverlaySettings) -> Self {
        let key = if provider == Provider::Codex { String::new() } else { source_key_for(provider, settings) };
        let estimator_key = if provider == Provider::Codex { String::new() } else { estimator_source_key_for(provider, settings) };
        Self { runtime_dir: self.runtime_dir.clone(), namespace: Arc::default(), bound_namespace: Some(key), bound_estimator_namespace: Some(estimator_key) }
    }

    pub fn for_source(&self, settings: &OverlaySettings) -> Self {
        self.for_provider(settings.provider, settings)
    }

    pub fn is_local_codex(&self) -> bool {
        self.bound_namespace.clone().unwrap_or_else(||self.namespace.lock().unwrap_or_else(|p|p.into_inner()).clone()).is_empty()
    }
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn set_source(&self, settings: &OverlaySettings) {
        let key = source_key(settings);
        if let Some(bound) = &self.bound_namespace {
            debug_assert_eq!(bound, &if settings.provider == Provider::Codex { String::new() } else { key });
            return;
        }
        *self.namespace.lock().unwrap_or_else(|p| p.into_inner()) =
            if settings.provider == Provider::Codex {
                String::new()
            } else {
                key
            };
    }

    /// Local Codex retains its original names for backwards compatibility.
    fn provider_path(&self, name: &str) -> PathBuf {
        let namespace = self.bound_namespace.clone().unwrap_or_else(|| self.namespace.lock().unwrap_or_else(|p| p.into_inner()).clone());
        if namespace.is_empty() { self.runtime_dir.join(name) }
        else { self.runtime_dir.join(format!("{namespace}-{name}")) }
    }

    fn estimator_path(&self, name: &str) -> PathBuf {
        match &self.bound_estimator_namespace {
            Some(namespace) if namespace.is_empty() => self.runtime_dir.join(name),
            Some(namespace) => self.runtime_dir.join(format!("{namespace}-{name}")),
            None => self.provider_path(name),
        }
    }

    pub fn settings_path(&self) -> PathBuf {
        self.runtime_dir.join("settings.json")
    }

    pub fn position_path(&self) -> PathBuf {
        self.runtime_dir.join("window-position.json")
    }

    pub fn settings_geometry_path(&self) -> PathBuf {
        self.runtime_dir.join("settings-window-geometry.json")
    }

    pub fn dock_state_path(&self) -> PathBuf {
        self.runtime_dir.join("dock-state.json")
    }

    pub fn status_path(&self) -> PathBuf {
        self.provider_path("usage-status.json")
    }

    pub fn quota_history_path(&self) -> PathBuf {
        self.provider_path("quota-read-history.json")
    }

    pub fn reset_state_path(&self) -> PathBuf {
        self.provider_path("reset-state.json")
    }

    pub fn burn_rate_state_path(&self) -> PathBuf {
        self.estimator_path("burn-rate-state.json")
    }

    pub fn legacy_burn_rate_state_path(&self) -> PathBuf {
        self.provider_path("burn-rate-state.json")
    }

    pub fn load_settings(&self) -> OverlaySettings {
        let path = self.settings_path();
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Ok(settings) = OverlaySettings::from_json_migrated(&value) {
                        return settings;
                    }
                }
            }
        }
        let mut settings = OverlaySettings::default();
        settings.migrate();
        settings
    }

    pub fn save_settings_checked(&self, settings: &OverlaySettings) -> Result<(), String> {
        let path = self.settings_path();
        let disk_version = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|v| v.get("schemaVersion").and_then(|v| v.as_u64()));
        if settings.schema_version > CURRENT_SCHEMA_VERSION
            || disk_version.is_some_and(|v| v > u64::from(CURRENT_SCHEMA_VERSION))
        {
            return Err("Settings were created by a newer application; refusing to overwrite them".into());
        }
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

    pub fn load_settings_geometry(&self) -> Option<SettingsWindowGeometry> {
        let path = self.settings_geometry_path();
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(mut geometry) = serde_json::from_str::<SettingsWindowGeometry>(&content) {
                    if geometry.x.is_finite()
                        && geometry.y.is_finite()
                        && geometry.width.is_finite()
                        && geometry.height.is_finite()
                        && geometry.width > 0.0
                        && geometry.height > 0.0
                    {
                        // A corrupt scale factor is dropped rather than rejecting the
                        // whole record, so a bad field can never strand the window.
                        geometry.scale_factor = geometry
                            .scale_factor
                            .filter(|scale| scale.is_finite() && *scale > 0.0);
                        return Some(geometry);
                    }
                }
            }
        }
        None
    }

    pub fn save_settings_geometry(&self, geometry: &SettingsWindowGeometry) {
        let path = self.settings_geometry_path();
        let tmp = path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string(geometry) {
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(tmp, path);
            }
        }
    }

    pub fn load_dock_state(&self) -> Option<DockPersistence> {
        let content = fs::read_to_string(self.dock_state_path()).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn save_dock_state(&self, state: &DockPersistence) {
        let path = self.dock_state_path();
        let tmp = path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string_pretty(state) {
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
            updated_at: chrono::Utc::now().to_rfc3339(),
            error: error_msg.unwrap_or(""),
        };

        if let Ok(json) = serde_json::to_string(&info) {
            let _ = fs::write(self.status_path(), json);
        }
    }
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
        assert_eq!(
            settings.preference(Provider::Codex).refresh_interval_seconds,
            300
        );
        assert_eq!(settings.language, "zh-CN");
        assert!(!settings.auto_start);
        assert!(settings.auto_check_updates);
        assert!(!settings.auto_install_updates);
    }

    #[test]
    fn legacy_settings_default_to_grouped_without_resetting_preferences() {
        let settings = OverlaySettings::from_json_migrated(&legacy_settings()).unwrap();
        assert_eq!(settings.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(settings.overlay_layout, OverlayLayout::Grouped);
        assert_preferences(&settings);
        assert!(settings.show_five_hour_quota);
        assert!(settings.show_burn_rate);
        assert!(OverlaySettings::default().show_five_hour_quota);
        assert!(OverlaySettings::default().show_burn_rate);
        assert_eq!(
            OverlaySettings::default().overlay_layout,
            OverlayLayout::Grouped
        );
        let codex = settings.preference(Provider::Codex);
        assert!(codex.five_hour_reset_notification);
        assert!(codex.weekly_reset_notification);
        assert_eq!(codex.five_hour_sound_mode, SoundMode::Windows);
        assert_eq!(codex.weekly_sound_mode, SoundMode::Windows);
        assert_eq!(codex.five_hour_sound_path, None);
        assert_eq!(codex.weekly_sound_path, None);
        assert!(!settings.auto_edge_hide);
        assert!(!settings.mouse_passthrough);
    }

    #[test]
    fn both_layouts_round_trip_with_existing_preferences() {
        for (wire, layout) in [
            ("grouped", OverlayLayout::Grouped),
            ("stacks", OverlayLayout::Stacks),
        ] {
            let mut value = legacy_settings();
            value["overlayLayout"] = json!(wire);
            value["autoCheckUpdates"] = json!(true);
            let settings = OverlaySettings::from_json_migrated(&value).unwrap();
            assert_eq!(settings.overlay_layout, layout);
            assert_preferences(&settings);
            let serialized = serde_json::to_value(&settings).unwrap();
            assert_eq!(serialized.get("overlayLayout"), Some(&json!(wire)));
            assert_eq!(serialized.get("autoCheckUpdates"), Some(&json!(true)));
            assert_eq!(serialized.get("refreshIntervalSeconds"), None);
            assert_eq!(
                serialized
                    .get("providerPreferences")
                    .and_then(|p| p.get("codex"))
                    .and_then(|c| c.get("refreshIntervalSeconds")),
                Some(&json!(300))
            );
            let restored: OverlaySettings =
                serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(restored.overlay_layout, layout);
            assert_preferences(&restored);
        }
    }

    #[test]
    fn unknown_layout_defaults_without_resetting_preferences() {
        let mut value = legacy_settings();
        value["overlayLayout"] = json!("future-layout");
        let settings = OverlaySettings::from_json_migrated(&value).unwrap();
        assert_eq!(settings.overlay_layout, OverlayLayout::Grouped);
        assert_preferences(&settings);
    }

    #[test]
    fn unknown_sound_mode_defaults_without_resetting_preferences() {
        let mut value = legacy_settings();
        value["fiveHourSoundMode"] = json!("alarm");
        value["weeklySoundMode"] = json!("future-mode");
        let settings = OverlaySettings::from_json_migrated(&value).unwrap();
        let codex = settings.preference(Provider::Codex);
        assert_eq!(codex.five_hour_sound_mode, SoundMode::Windows);
        assert_eq!(codex.weekly_sound_mode, SoundMode::Windows);
        assert_preferences(&settings);
    }

    #[test]
    fn independent_reset_settings_round_trip() {
        let mut settings = OverlaySettings::default();
        let codex = settings.preference_mut(Provider::Codex);
        codex.five_hour_reset_notification = false;
        codex.weekly_sound_mode = SoundMode::Custom;
        codex.weekly_sound_path = Some("C:\\sound.m4a".into());
        settings.auto_edge_hide = true;
        settings.mouse_passthrough = true;
        settings.auto_install_updates = true;
        let restored: OverlaySettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        let codex = restored.preference(Provider::Codex);
        assert!(!codex.five_hour_reset_notification);
        assert!(codex.weekly_reset_notification);
        assert_eq!(codex.five_hour_sound_mode, SoundMode::Windows);
        assert_eq!(codex.weekly_sound_mode, SoundMode::Custom);
        assert_eq!(codex.five_hour_sound_path, None);
        assert_eq!(codex.weekly_sound_path, Some("C:\\sound.m4a".into()));
        assert!(restored.auto_edge_hide);
        assert!(restored.mouse_passthrough);
        assert!(restored.auto_install_updates);
    }

    /// The `showPercentageGrid` contract: absent -> `false`, present -> the value
    /// given, and the wire key is the camelCase one the frontend patches with. The
    /// assertions check the *value* rather than merely "deserialization succeeded",
    /// so renaming the Rust field (without a matching `serde` rename) turns them red.
    #[test]
    fn show_percentage_grid_defaults_off_and_round_trips_when_present() {
        let absent = OverlaySettings::from_json_migrated(&legacy_settings()).unwrap();
        assert!(!absent.show_percentage_grid);
        assert!(!OverlaySettings::default().show_percentage_grid);

        let mut enabled = legacy_settings();
        enabled["showPercentageGrid"] = json!(true);
        assert!(
            OverlaySettings::from_json_migrated(&enabled)
                .unwrap()
                .show_percentage_grid
        );

        let mut disabled = legacy_settings();
        disabled["showPercentageGrid"] = json!(false);
        assert!(
            !OverlaySettings::from_json_migrated(&disabled)
                .unwrap()
                .show_percentage_grid
        );

        let mut settings = OverlaySettings::default();
        settings.show_percentage_grid = true;
        let wire = serde_json::to_value(&settings).unwrap();
        assert_eq!(wire.get("showPercentageGrid"), Some(&json!(true)));
        let restored: OverlaySettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.show_percentage_grid);
    }

    /// The auto-pause contract: the protection switch is off by default, the two
    /// thresholds carry their own defaults, and an old profile that knows
    /// nothing about the feature is migrated without enabling it.
    #[test]
    fn auto_pause_defaults_to_off_with_its_own_thresholds() {
        let absent = OverlaySettings::from_json_migrated(&legacy_settings()).unwrap();
        let codex = absent.preference(Provider::Codex);
        assert!(!codex.protection_enabled);
        assert_eq!(codex.five_hour_threshold_percent, 5);
        assert_eq!(codex.weekly_threshold_percent, 2);
        let default_codex = OverlaySettings::default().preference(Provider::Codex);
        assert!(!default_codex.protection_enabled);
        assert_eq!(default_codex.five_hour_threshold_percent, 5);
        assert_eq!(default_codex.weekly_threshold_percent, 2);

        // A partially migrated profile keeps its explicit values.
        let mut partial = legacy_settings();
        partial["autoPauseOnLowQuota"] = json!(true);
        partial["autoPauseWeeklyThresholdPercent"] = json!(7);
        let migrated = OverlaySettings::from_json_migrated(&partial).unwrap();
        let migrated_codex = migrated.preference(Provider::Codex);
        assert!(migrated_codex.protection_enabled);
        assert_eq!(migrated_codex.weekly_threshold_percent, 7);
        // The field the old profile did not carry still gets its default.
        assert_eq!(migrated_codex.five_hour_threshold_percent, 5);

        let mut settings = OverlaySettings::default();
        let pref = settings.preference_mut(Provider::Codex);
        pref.protection_enabled = true;
        pref.five_hour_threshold_percent = 9;
        pref.weekly_threshold_percent = 4;
        let wire = serde_json::to_value(&settings).unwrap();
        assert_eq!(wire.get("autoPauseOnLowQuota"), None);
        assert_eq!(
            wire.get("providerPreferences")
                .and_then(|p| p.get("codex"))
                .and_then(|c| c.get("protectionEnabled")),
            Some(&json!(true))
        );

        let restored: OverlaySettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        let restored_codex = restored.preference(Provider::Codex);
        assert!(restored_codex.protection_enabled);
        assert_eq!(restored_codex.five_hour_threshold_percent, 9);
        assert_eq!(restored_codex.weekly_threshold_percent, 4);
    }

    #[test]
    fn settings_geometry_round_trips_and_falls_back_when_corrupt() {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-geometry-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let manager = ConfigManager::with_runtime_dir(dir.clone());

        assert!(manager.load_settings_geometry().is_none());

        let geometry = SettingsWindowGeometry {
            x: 120.0,
            y: 80.0,
            width: 480.0,
            height: 660.0,
            scale_factor: Some(1.5),
        };
        manager.save_settings_geometry(&geometry);
        let loaded = manager
            .load_settings_geometry()
            .expect("saved geometry should load");
        assert_eq!(
            (loaded.x, loaded.y, loaded.width, loaded.height),
            (120.0, 80.0, 480.0, 660.0)
        );
        assert_eq!(loaded.scale_factor, Some(1.5));

        let path = manager.settings_geometry_path();
        fs::write(&path, "{ not valid json").unwrap();
        assert!(manager.load_settings_geometry().is_none());

        fs::write(&path, r#"{"x":10.0,"y":20.0,"width":0.0,"height":660.0}"#).unwrap();
        assert!(manager.load_settings_geometry().is_none());

        fs::write(&path, r#"{"x":10.0,"y":20.0,"width":480.0}"#).unwrap();
        assert!(manager.load_settings_geometry().is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_geometry_without_scale_factor_still_loads() {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-geometry-legacy-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let manager = ConfigManager::with_runtime_dir(dir.clone());

        let path = manager.settings_geometry_path();
        fs::write(&path, r#"{"x":10.0,"y":20.0,"width":480.0,"height":660.0}"#).unwrap();
        let loaded = manager
            .load_settings_geometry()
            .expect("legacy geometry should load");
        assert_eq!(loaded.scale_factor, None);

        // An unusable scale factor is discarded instead of poisoning the record.
        fs::write(
            &path,
            r#"{"x":10.0,"y":20.0,"width":480.0,"height":660.0,"scale_factor":0.0}"#,
        )
        .unwrap();
        assert_eq!(manager.load_settings_geometry().unwrap().scale_factor, None);

        fs::write(
            &path,
            r#"{"x":10.0,"y":20.0,"width":480.0,"height":660.0,"scale_factor":-2.0}"#,
        )
        .unwrap();
        assert_eq!(manager.load_settings_geometry().unwrap().scale_factor, None);

        let _ = fs::remove_dir_all(&dir);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    Portable,
    Nsis,
    Msi,
}

/// The `latest.json` platform entry for a package of this build's own
/// architecture. An x64 build stays on x64 even under emulation on ARM64
/// Windows; moving to the native build is a fresh install.
pub fn updater_target_for(arch: &str, package: PackageKind) -> &'static str {
    match (arch, package) {
        ("aarch64", PackageKind::Portable) => "windows-aarch64-portable",
        ("aarch64", PackageKind::Nsis) => "windows-aarch64-nsis",
        ("aarch64", PackageKind::Msi) => "windows-aarch64-msi",
        (_, PackageKind::Portable) => "windows-x86_64-portable",
        (_, PackageKind::Nsis) => "windows-x86_64-nsis",
        (_, PackageKind::Msi) => "windows-x86_64-msi",
    }
}

pub fn updater_target() -> &'static str {
    let package = if !is_installed_environment() {
        PackageKind::Portable
    } else if std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("uninstall.exe")))
        .is_some_and(|uninstaller| uninstaller.exists())
    {
        PackageKind::Nsis
    } else {
        PackageKind::Msi
    };
    updater_target_for(std::env::consts::ARCH, package)
}

#[cfg(test)]
mod updater_target_tests {
    use super::*;

    #[test]
    fn each_architecture_updates_from_its_own_packages() {
        assert_eq!(updater_target_for("x86_64", PackageKind::Nsis), "windows-x86_64-nsis");
        assert_eq!(updater_target_for("x86_64", PackageKind::Msi), "windows-x86_64-msi");
        assert_eq!(updater_target_for("x86_64", PackageKind::Portable), "windows-x86_64-portable");
        assert_eq!(updater_target_for("aarch64", PackageKind::Nsis), "windows-aarch64-nsis");
        assert_eq!(updater_target_for("aarch64", PackageKind::Msi), "windows-aarch64-msi");
        assert_eq!(updater_target_for("aarch64", PackageKind::Portable), "windows-aarch64-portable");
        // This build's own architecture always has an entry.
        assert!(updater_target_for(std::env::consts::ARCH, PackageKind::Nsis).starts_with("windows-"));
    }
}

#[cfg(test)]
mod independent_preferences_tests {
    use super::*;
    #[test]
    fn v2_migration_preserves_explicit_values_and_inherits_only_missing_fields() {
        let mut settings = OverlaySettings::from_json_migrated(&serde_json::json!({
            "schemaVersion":2,"refreshIntervalSeconds":300,"weeklyResetNotification":false,
            "fiveHourSoundPath":"C:/alert.wav", "providerPreferences":{
                "codex":{"refreshIntervalSeconds":47,"fiveHourSoundPath":null},
                "claude":{"weeklyResetNotification":true}
            }
        })).unwrap();
        assert_eq!(settings.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(settings.preference(Provider::Codex).refresh_interval_seconds,47);
        assert_eq!(settings.preference(Provider::Codex).five_hour_sound_path,None);
        assert!(!settings.preference(Provider::Codex).weekly_reset_notification);
        assert!(settings.preference(Provider::Claude).weekly_reset_notification);
        assert_eq!(settings.preference(Provider::Dsh).refresh_interval_seconds,300);
        let once=serde_json::to_value(&settings).unwrap();
        settings.migrate();
        assert_eq!(serde_json::to_value(&settings).unwrap(),once);
    }
    #[test]
    fn independent_settings_project_without_leaking_other_sources() {
        let mut s=OverlaySettings::default(); s.migrate();
        s.preference_mut(Provider::Claude).refresh_interval_seconds=120;
        s.preference_mut(Provider::Dsh).weekly_reset_notification=false;
        assert_eq!(s.preference(Provider::Claude).refresh_interval_seconds,120);
        assert_eq!(s.preference(Provider::Codex).refresh_interval_seconds,60);
        assert!(s.preference(Provider::Claude).weekly_reset_notification);
        assert!(!s.preference(Provider::Dsh).weekly_reset_notification);
        let mut restored:OverlaySettings=serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        restored.migrate();assert_eq!(restored.preference(Provider::Claude).refresh_interval_seconds,120);
    }
    #[test]
    fn v3_codex_algorithm_migrates_and_each_provider_keeps_its_own_choice() {
        let mut settings = OverlaySettings::from_json_migrated(&serde_json::json!({
            "schemaVersion": 3, "fiveHourBurnRateAlgorithm": "weighted",
            "providerPreferences": {"codex": {}, "claude": {}, "dsh": {}}
        })).unwrap();
        assert_eq!(settings.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(settings.preference(Provider::Codex).five_hour_burn_rate_algorithm, FiveHourBurnRateAlgorithm::Weighted);
        assert_eq!(settings.preference(Provider::Claude).five_hour_burn_rate_algorithm, FiveHourBurnRateAlgorithm::Legacy);
        settings.preference_mut(Provider::Claude).five_hour_burn_rate_algorithm = FiveHourBurnRateAlgorithm::Personalized;
        settings.preference_mut(Provider::Dsh).five_hour_burn_rate_algorithm = FiveHourBurnRateAlgorithm::Weighted;
        let restored: OverlaySettings = serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(restored.preference(Provider::Codex).five_hour_burn_rate_algorithm, FiveHourBurnRateAlgorithm::Weighted);
        assert_eq!(restored.preference(Provider::Claude).five_hour_burn_rate_algorithm, FiveHourBurnRateAlgorithm::Personalized);
        assert_eq!(restored.preference(Provider::Dsh).five_hour_burn_rate_algorithm, FiveHourBurnRateAlgorithm::Weighted);
    }
    #[test]
    fn dsh_estimator_history_follows_account_across_transport_changes() {
        let root = ConfigManager::with_runtime_dir(std::env::temp_dir().join(format!("overlay-estimator-path-{}", std::process::id())));
        let mut settings = OverlaySettings::default();
        settings.provider = Provider::Dsh;
        settings.dsh_instance_id = "instance-a".into();
        settings.dsh_account_id = "account-a".into();
        settings.dsh_base_url = "http://127.0.0.1:8000".into();
        let first = root.for_provider(Provider::Dsh, &settings);
        settings.dsh_base_url = "http://127.0.0.1:9000".into();
        let reconnected = root.for_provider(Provider::Dsh, &settings);
        assert_eq!(first.burn_rate_state_path(), reconnected.burn_rate_state_path());
        settings.dsh_account_id = "account-b".into();
        assert_ne!(first.burn_rate_state_path(), root.for_provider(Provider::Dsh, &settings).burn_rate_state_path());
    }
    #[test]
    fn default_pause_messages_follow_the_language_setting_not_the_os_locale() {
        for (language, expected) in [("zh-CN", "\u{6682}\u{505C}"), ("en-US", "Pause")] {
            let mut s = OverlaySettings { language: language.into(), ..Default::default() };
            s.migrate();
            assert_eq!(s.auto_pause_five_hour_message, expected);
            assert_eq!(s.auto_pause_weekly_message, expected);
        }
        assert_eq!(crate::pause_message::effective_message("", "zh-CN"), "\u{6682}\u{505C}");
        assert_eq!(crate::pause_message::effective_message("Custom", "zh-CN"), "Custom");
    }
    #[test]
    fn future_schema_is_never_downgraded() {
        let dir = std::env::temp_dir().join(format!("overlay-future-schema-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let manager = ConfigManager::with_runtime_dir(dir.clone());
        fs::write(manager.settings_path(), r#"{"schemaVersion":99}"#).unwrap();
        let loaded = manager.load_settings();
        assert_eq!(loaded.schema_version, 99);
        assert!(manager.save_settings_checked(&loaded).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod partial_migration_repair_tests {
    use super::*;
    #[test]
    fn wrong_typed_nested_values_fall_back_without_resetting_the_file() {
        let settings = OverlaySettings::from_json_migrated(&serde_json::json!({
            "schemaVersion": 5, "provider": "claude", "scalePercent": 150,
            "providerPreferences": {
                "claude": {"fiveHourSoundMode": null, "monitorEnabled": "yes", "weeklyThresholdPercent": -3,
                    "fiveHourSoundPath": 7, "protectionEnabled": true, "refreshIntervalSeconds": 120},
                "dsh": "broken"
            }
        })).unwrap();
        assert_eq!(settings.provider, Provider::Claude);
        assert_eq!(settings.scale_percent, 150);
        let claude = settings.preference(Provider::Claude);
        assert!(claude.monitor_enabled && claude.protection_enabled);
        assert_eq!(claude.five_hour_sound_mode, SoundMode::Windows);
        assert_eq!(claude.weekly_threshold_percent, 2);
        assert_eq!(claude.five_hour_sound_path, None);
        assert_eq!(claude.refresh_interval_seconds, 120);
        assert_eq!(settings.preference(Provider::Dsh), ProviderPreference::default_for(Provider::Dsh));
        let unusable = OverlaySettings::from_json_migrated(&serde_json::json!({"providerPreferences": 3})).unwrap();
        assert_eq!(unusable.preference(Provider::Codex), ProviderPreference::default_for(Provider::Codex));
    }
    #[test]
    fn early_schemas_keep_an_explicit_nested_protection_switch() {
        for schema in [0, 1, 2, 3] {
            let settings = OverlaySettings::from_json_migrated(&serde_json::json!({
                "schemaVersion": schema, "provider": "codex", "autoPauseOnLowQuota": true,
                "providerPreferences": {"claude": {"protectionEnabled": true}, "codex": {"protectionEnabled": false}, "dsh": {}}
            })).unwrap();
            assert!(settings.preference(Provider::Claude).protection_enabled, "schema {schema}");
            assert!(!settings.preference(Provider::Codex).protection_enabled, "schema {schema}");
            assert!(!settings.preference(Provider::Dsh).protection_enabled, "schema {schema}");
        }
    }
    #[test]
    fn earlier_partial_migrations_keep_nested_values_and_recover_bad_modes() {
        for schema in [0,1,2] {
            let settings=OverlaySettings::from_json_migrated(&serde_json::json!({
                "schemaVersion":schema,"fiveHourSoundMode":"custom","fiveHourSoundPath":"C:/sound.wav",
                "providerPreferences":{"claude":{"monitorEnabled":false,"weeklyThresholdPercent":8,"refreshIntervalSeconds":47,"fiveHourSoundMode":"bad","fiveHourSoundPath":null}}
            })).unwrap();
            let p=settings.preference(Provider::Claude);
            assert!(!p.monitor_enabled); assert_eq!(p.weekly_threshold_percent,8);
            assert_eq!(p.refresh_interval_seconds,47); assert_eq!(p.five_hour_sound_mode,SoundMode::Custom);
            assert_eq!(p.five_hour_sound_path,None);
        }
    }
}

#[cfg(test)]
mod carousel_settings_tests {
    use super::*;
    #[test]
    fn carousel_recovers_only_invalid_fields_and_preserves_every_integer_interval() {
        let s:OverlaySettings=serde_json::from_value(serde_json::json!({"quotaCarouselEnabled":"yes","quotaCarouselIntervalSeconds":2,"providerPreferences":{"claude":{"carouselIncluded":false,"refreshIntervalSeconds":47}}})).unwrap();
        assert!(!s.quota_carousel_enabled);assert_eq!(s.quota_carousel_interval_seconds,8);assert!(!s.preference(Provider::Claude).carousel_included);assert_eq!(s.preference(Provider::Claude).refresh_interval_seconds,47);
        for interval in 3..=30 {let s:OverlaySettings=serde_json::from_value(serde_json::json!({"quotaCarouselIntervalSeconds":interval})).unwrap();assert_eq!(s.quota_carousel_interval_seconds,interval);}
        for invalid in [serde_json::json!(3.5),serde_json::json!(31),serde_json::json!("8"),serde_json::Value::Null]{let s:OverlaySettings=serde_json::from_value(serde_json::json!({"quotaCarouselIntervalSeconds":invalid})).unwrap();assert_eq!(s.quota_carousel_interval_seconds,8);}
    }
}
