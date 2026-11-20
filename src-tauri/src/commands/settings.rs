use std::sync::atomic::Ordering;
use std::sync::Arc;
use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::{OverlaySettings, Provider};
use crate::tray::TrayPresentationSink;

use super::{
    check_and_notify_auto_update, dsh_monitor_enabled, resolve_locale, set_main_mouse_passthrough,
    AppState, SettingsEnvelope,
};

pub(crate) fn overlay_geometry_changed(previous: &OverlaySettings, next: &OverlaySettings) -> bool {
    previous.scale_percent != next.scale_percent
        || previous.show_provider_logo != next.show_provider_logo
        || previous.provider != next.provider
        || previous.show_credits != next.show_credits
        || previous.show_five_hour_quota != next.show_five_hour_quota
        || previous.show_burn_rate != next.show_burn_rate
        || previous.overlay_layout != next.overlay_layout
        || previous.language != next.language
}


#[tauri::command]
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<SettingsEnvelope, String> {
    let _source_commit = state.settings_patch.lock().await;
    let guard = state.settings.lock().await;
    let settings = guard.clone();
    let revision = state.settings_revision.load(Ordering::SeqCst);
    Ok(SettingsEnvelope::new(&state, revision, settings))
}


#[tauri::command]
pub async fn patch_settings(
    patch: Value,
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<SettingsEnvelope, String> {
    apply_settings_patch(&app, state.inner(), patch, false).await
}

fn deserialize_strict_layout<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::config::OverlayLayout>, D::Error> {
    let raw = String::deserialize(deserializer)?;
    match raw.as_str() {
        "grouped" => Ok(Some(crate::config::OverlayLayout::Grouped)),
        "stacks" => Ok(Some(crate::config::OverlayLayout::Stacks)),
        _ => Err(serde::de::Error::custom("Invalid overlay layout")),
    }
}

fn deserialize_strict_sound_mode<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::config::SoundMode>, D::Error> {
    let raw = String::deserialize(deserializer)?;
    match raw.as_str() {
        "windows" => Ok(Some(crate::config::SoundMode::Windows)),
        "custom" => Ok(Some(crate::config::SoundMode::Custom)),
        _ => Err(serde::de::Error::custom("Invalid sound mode")),
    }
}

fn deserialize_strict_pool<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::config::AntigravityPool>, D::Error> {
    let raw = String::deserialize(deserializer)?;
    match raw.as_str() {
        "auto" => Ok(Some(crate::config::AntigravityPool::Auto)),
        "gemini" => Ok(Some(crate::config::AntigravityPool::Gemini)),
        "argon" => Ok(Some(crate::config::AntigravityPool::Argon)),
        "3p" => Ok(Some(crate::config::AntigravityPool::ThirdParty)),
        _ => Err(serde::de::Error::custom("Invalid Antigravity pool")),
    }
}

fn deserialize_sound_path_patch<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    match Option::<String>::deserialize(deserializer)? {
        None => Ok(Some(None)),
        Some(path) if !path.is_empty() && path.len() <= 4096 => Ok(Some(Some(path))),
        Some(_) => Err(serde::de::Error::custom("Invalid sound path")),
    }
}

fn deserialize_present_some<'de, T: serde::Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPreferencePatch {
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub monitor_enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub carousel_included: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub five_hour_burn_rate_algorithm: Option<crate::config::FiveHourBurnRateAlgorithm>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub protection_enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub five_hour_threshold_percent: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub weekly_threshold_percent: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub refresh_interval_seconds: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub five_hour_reset_notification: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub weekly_reset_notification: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_strict_sound_mode")]
    pub five_hour_sound_mode: Option<crate::config::SoundMode>,
    #[serde(default, deserialize_with = "deserialize_strict_sound_mode")]
    pub weekly_sound_mode: Option<crate::config::SoundMode>,
    #[serde(default, deserialize_with = "deserialize_sound_path_patch")]
    pub five_hour_sound_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_sound_path_patch")]
    pub weekly_sound_path: Option<Option<String>>,
}

impl ProviderPreferencePatch {
    fn is_empty(&self) -> bool {
        self.monitor_enabled.is_none()
            && self.carousel_included.is_none()
            && self.five_hour_burn_rate_algorithm.is_none()
            && self.protection_enabled.is_none()
            && self.five_hour_threshold_percent.is_none()
            && self.weekly_threshold_percent.is_none()
            && self.refresh_interval_seconds.is_none()
            && self.five_hour_reset_notification.is_none()
            && self.weekly_reset_notification.is_none()
            && self.five_hour_sound_mode.is_none()
            && self.weekly_sound_mode.is_none()
            && self.five_hour_sound_path.is_none()
            && self.weekly_sound_path.is_none()
    }

    fn validate(&self, provider: Provider) -> Result<(), String> {
        if self.is_empty() {
            return Err("Provider preference patch is empty".into());
        }
        let min_threshold = if provider == Provider::Dsh { 0 } else { 1 };
        if self
            .five_hour_threshold_percent
            .is_some_and(|v| !(min_threshold..=15).contains(&v))
        {
            return Err("Invalid settings field: fiveHourThresholdPercent".into());
        }
        if self
            .weekly_threshold_percent
            .is_some_and(|v| !(min_threshold..=10).contains(&v))
        {
            return Err("Invalid settings field: weeklyThresholdPercent".into());
        }
        if self
            .refresh_interval_seconds
            .is_some_and(|v| !(15..=3600).contains(&v))
        {
            return Err("Invalid settings field: refreshIntervalSeconds".into());
        }
        Ok(())
    }

    fn apply_to(&self, pref: &mut crate::config::ProviderPreference) {
        if let Some(v) = self.monitor_enabled {
            pref.monitor_enabled = v;
        }
        if let Some(v) = self.carousel_included {
            pref.carousel_included = v;
        }
        if let Some(v) = self.five_hour_burn_rate_algorithm {
            pref.five_hour_burn_rate_algorithm = v;
        }
        if let Some(v) = self.protection_enabled {
            pref.protection_enabled = v;
        }
        if let Some(v) = self.five_hour_threshold_percent {
            pref.five_hour_threshold_percent = v;
        }
        if let Some(v) = self.weekly_threshold_percent {
            pref.weekly_threshold_percent = v;
        }
        if let Some(v) = self.refresh_interval_seconds {
            pref.refresh_interval_seconds = v;
        }
        if let Some(v) = self.five_hour_reset_notification {
            pref.five_hour_reset_notification = v;
        }
        if let Some(v) = self.weekly_reset_notification {
            pref.weekly_reset_notification = v;
        }
        if let Some(v) = self.five_hour_sound_mode {
            pref.five_hour_sound_mode = v;
        }
        if let Some(v) = self.weekly_sound_mode {
            pref.weekly_sound_mode = v;
        }
        if let Some(v) = &self.five_hour_sound_path {
            pref.five_hour_sound_path = v.clone();
        }
        if let Some(v) = &self.weekly_sound_path {
            pref.weekly_sound_path = v.clone();
        }
        if !pref.monitor_enabled {
            pref.protection_enabled = false;
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub quota_carousel_enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub quota_carousel_interval_seconds: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub auto_pause_five_hour_message: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub auto_pause_weekly_message: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub provider: Option<Provider>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub provider_preferences: Option<std::collections::BTreeMap<Provider, ProviderPreferencePatch>>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub dsh_account_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub dsh_session_ids: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub claude_live_polling: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub antigravity_live_polling: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_strict_pool")]
    pub antigravity_pool: Option<crate::config::AntigravityPool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub show_provider_logo: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_strict_layout")]
    pub overlay_layout: Option<crate::config::OverlayLayout>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub scale_percent: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub background_transparency_percent: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub show_credits: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub show_five_hour_quota: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub show_burn_rate: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub language: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub auto_start: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub auto_check_updates: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub auto_install_updates: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub include_prerelease_updates: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub auto_edge_hide: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub mouse_passthrough: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_some")]
    pub show_percentage_grid: Option<bool>,
}

impl SettingsPatch {
    pub fn apply_to(&self, settings: &mut OverlaySettings) {
        if let Some(v) = self.quota_carousel_enabled {
            settings.quota_carousel_enabled = v;
        }
        if let Some(v) = self.quota_carousel_interval_seconds {
            settings.quota_carousel_interval_seconds = v;
        }
        if let Some(v) = &self.auto_pause_five_hour_message {
            settings.auto_pause_five_hour_message = v.clone();
        }
        if let Some(v) = &self.auto_pause_weekly_message {
            settings.auto_pause_weekly_message = v.clone();
        }
        if let Some(v) = self.provider {
            settings.provider = v;
        }
        if let Some(prefs) = &self.provider_preferences {
            for (provider, pref_patch) in prefs {
                pref_patch.apply_to(settings.preference_mut(*provider));
            }
        }
        if let Some(v) = &self.dsh_account_id {
            settings.dsh_account_id = v.clone();
        }
        if let Some(v) = &self.dsh_session_ids {
            settings.dsh_session_ids = v.clone();
        }
        if let Some(v) = self.claude_live_polling {
            settings.claude_live_polling = v;
        }
        if let Some(v) = self.antigravity_live_polling {
            settings.antigravity_live_polling = v;
        }
        if let Some(v) = self.antigravity_pool {
            settings.antigravity_pool = v;
        }
        if let Some(v) = self.show_provider_logo {
            settings.show_provider_logo = v;
        }
        if let Some(v) = &self.overlay_layout {
            settings.overlay_layout = v.clone();
        }
        if let Some(v) = self.scale_percent {
            settings.scale_percent = v;
        }
        if let Some(v) = self.background_transparency_percent {
            settings.background_transparency_percent = v;
        }
        if let Some(v) = self.show_credits {
            settings.show_credits = v;
        }
        if let Some(v) = self.show_five_hour_quota {
            settings.show_five_hour_quota = v;
        }
        if let Some(v) = self.show_burn_rate {
            settings.show_burn_rate = v;
        }
        if let Some(v) = &self.language {
            settings.language = v.clone();
        }
        if let Some(v) = self.auto_start {
            settings.auto_start = v;
        }
        if let Some(v) = self.auto_check_updates {
            settings.auto_check_updates = v;
        }
        if let Some(v) = self.auto_install_updates {
            settings.auto_install_updates = v;
        }
        if let Some(v) = self.include_prerelease_updates {
            settings.include_prerelease_updates = v;
        }
        if let Some(v) = self.auto_edge_hide {
            settings.auto_edge_hide = v;
        }
        if let Some(v) = self.mouse_passthrough {
            settings.mouse_passthrough = v;
        }
        if let Some(v) = self.show_percentage_grid {
            settings.show_percentage_grid = v;
        }
    }
}

pub(crate) fn validate_settings_patch(patch: &Value, allow_auto_start: bool) -> Result<SettingsPatch, String> {
    let map = patch.as_object().ok_or("Settings patch must be an object")?;
    if map.is_empty() {
        return Err("Settings patch is empty".into());
    }
    let parsed: SettingsPatch =
        serde_json::from_value(patch.clone()).map_err(|e| format!("Invalid settings patch: {e}"))?;
    if parsed.auto_start.is_some() && !allow_auto_start {
        return Err("Invalid settings field: autoStart".into());
    }
    if parsed
        .auto_pause_five_hour_message
        .as_deref()
        .is_some_and(|s| !crate::pause_message::valid_message(s))
    {
        return Err("Invalid settings field: autoPauseFiveHourMessage".into());
    }
    if parsed
        .auto_pause_weekly_message
        .as_deref()
        .is_some_and(|s| !crate::pause_message::valid_message(s))
    {
        return Err("Invalid settings field: autoPauseWeeklyMessage".into());
    }
    if parsed
        .scale_percent
        .is_some_and(|n| !(100..=250).contains(&n) || n % 5 != 0)
    {
        return Err("Invalid settings field: scalePercent".into());
    }
    if parsed
        .background_transparency_percent
        .is_some_and(|n| n > 80 || n % 5 != 0)
    {
        return Err("Invalid settings field: backgroundTransparencyPercent".into());
    }
    if parsed
        .quota_carousel_interval_seconds
        .is_some_and(|n| !(3..=30).contains(&n))
    {
        return Err("Invalid settings field: quotaCarouselIntervalSeconds".into());
    }
    if parsed
        .language
        .as_deref()
        .is_some_and(|s| !matches!(s, "auto" | "en-US" | "ru-RU" | "zh-CN" | "zh-Hant"))
    {
        return Err("Invalid settings field: language".into());
    }
    if let Some(prefs) = &parsed.provider_preferences {
        if prefs.is_empty() {
            return Err("Invalid settings field: providerPreferences".into());
        }
        for (provider, pref_patch) in prefs {
            pref_patch.validate(*provider)?;
        }
    }
    if parsed
        .dsh_account_id
        .as_deref()
        .is_some_and(|s| !s.is_empty() && !crate::pause_control::looks_like_thread_id(s))
    {
        return Err("Invalid settings field: dshAccountId".into());
    }
    if parsed.dsh_session_ids.as_ref().is_some_and(|items| {
        items.len() > 32
            || items
                .iter()
                .any(|id| !crate::pause_control::looks_like_thread_id(id.strip_prefix("session-").unwrap_or(id)))
    }) {
        return Err("Invalid settings field: dshSessionIds".into());
    }
    if parsed.auto_check_updates == Some(false) && parsed.auto_install_updates == Some(true) {
        return Err("Automatic installation requires automatic update checks".into());
    }
    Ok(parsed)
}

#[cfg(test)]
pub fn merge_settings_patch(settings: &mut OverlaySettings, patch: &Value) -> Result<(), String> {
    let parsed = validate_settings_patch(patch, true)?;
    parsed.apply_to(settings);
    Ok(())
}

fn reconcile_update_preferences(next: &mut OverlaySettings, changes: &SettingsPatch) {
    if next.auto_install_updates && !next.auto_check_updates {
        if changes.auto_install_updates == Some(true) {
            next.auto_check_updates = true;
        } else {
            next.auto_install_updates = false;
        }
    }
}


#[cfg(test)]
mod settings_patch_tests {
    use super::*;
    use crate::commands::{claim_update_notice, next_update_check_delay, UPDATE_CHECK_INTERVAL};
    use crate::config::ConfigManager;
    use std::time::Duration;

    #[test]
    fn display_patches_require_booleans_and_invalidate_geometry() {
        for key in ["showFiveHourQuota", "showBurnRate"] {
            for value in [serde_json::json!(true), serde_json::json!(false)] {
                assert!(validate_settings_patch(&serde_json::json!({key: value}), false).is_ok());
            }
            for value in [serde_json::json!("false"), serde_json::json!(0), Value::Null, serde_json::json!([])] {
                assert!(validate_settings_patch(&serde_json::json!({key: value}), false).is_err());
            }
            let previous = OverlaySettings::default();
            let mut next = previous.clone();
            merge_settings_patch(&mut next, &serde_json::json!({key: false})).unwrap();
            assert!(overlay_geometry_changed(&previous, &next));
            assert!(!overlay_geometry_changed(&next, &next));
        }
    }


    #[test]
    fn rejects_entire_patch_when_one_field_is_invalid() {
        let patch = serde_json::json!({"showCredits": false, "scalePercent": 999});
        assert!(validate_settings_patch(&patch, false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"language": "unknown"}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"autoStart": false}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"futureField": true}), false).is_err());
    }

    #[test]
    fn accepts_independent_notification_fields() {
        let patch = serde_json::json!({
            "providerPreferences": {
                "codex": {
                    "fiveHourResetNotification": false,
                    "weeklySoundMode": "custom",
                    "weeklySoundPath": "C:\\sound.m4a",
                    "fiveHourSoundPath": null
                }
            },
            "autoEdgeHide": true,
            "mousePassthrough": true
        });
        assert!(validate_settings_patch(&patch, false).is_ok());
        assert!(validate_settings_patch(&serde_json::json!({"fiveHourResetNotification": false}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"fiveHourBurnRateAlgorithm": "weighted"}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"refreshIntervalSeconds": 120}), false).is_err());
    }

    #[test]
    fn a_failed_update_check_retries_soon_then_falls_back_to_the_regular_interval() {
        let delays: Vec<_> = (1..=5).map(|failures| next_update_check_delay(false, failures)).collect();
        assert_eq!(delays, vec![
            Duration::from_secs(30), Duration::from_secs(120), Duration::from_secs(600),
            UPDATE_CHECK_INTERVAL, UPDATE_CHECK_INTERVAL,
        ]);
        assert_eq!(next_update_check_delay(true, 0), UPDATE_CHECK_INTERVAL);
        assert_eq!(UPDATE_CHECK_INTERVAL, Duration::from_secs(12 * 60 * 60));
    }

    #[test]
    fn each_update_version_is_announced_once_as_available_and_once_as_ready() {
        let mut last = None;
        assert!(claim_update_notice(&mut last, "1.8.6", false));
        assert!(!claim_update_notice(&mut last, "1.8.6", false), "a repeated check must stay quiet");
        assert!(claim_update_notice(&mut last, "1.8.6", true), "finishing the download is new information");
        assert!(!claim_update_notice(&mut last, "1.8.6", true));
        assert!(claim_update_notice(&mut last, "1.8.7", true), "a newer version is announced");
    }

    #[test]
    fn update_notifications_are_localized_and_say_when_a_download_installs() {
        let (title, body) = crate::notify::update_notification_copy("en-US", "1.8.6", true);
        assert_eq!(title, "Codex Usage Overlay update");
        assert_eq!(body, "Version 1.8.6 has been downloaded and installs when you quit.");
        assert_eq!(crate::notify::update_notification_copy("en-US", "1.8.6", false).1, "Version 1.8.6 is available.");
        assert_eq!(crate::notify::update_notification_copy("zh-CN", "1.8.6", false).1, "发现新版本 1.8.6。");
        assert_eq!(crate::notify::update_notification_copy("zh-Hant", "1.8.6", true).1, "1.8.6 版已下載，將在退出時安裝。");
    }

    #[test]
    fn automatic_installation_keeps_update_checks_enabled() {
        let mut settings = OverlaySettings::default();
        settings.auto_check_updates = false;
        settings.auto_install_updates = true;
        let enable = serde_json::json!({"autoInstallUpdates": true});
        reconcile_update_preferences(&mut settings, &validate_settings_patch(&enable, false).unwrap());
        assert!(settings.auto_check_updates && settings.auto_install_updates);

        settings.auto_install_updates = true;
        let disable_checks = serde_json::json!({"autoCheckUpdates": false});
        settings.auto_check_updates = false;
        reconcile_update_preferences(&mut settings, &validate_settings_patch(&disable_checks, false).unwrap());
        assert!(!settings.auto_install_updates);
        assert!(validate_settings_patch(&serde_json::json!({
            "autoCheckUpdates": false, "autoInstallUpdates": true
        }), false).is_err());
    }

    /// P2-1: the closed allowlist is the toggle's only user entry point. Without the
    /// key registered here the whole patch is rejected and the switch can never
    /// persist, while a frontend-only assertion stays green.
    #[test]
    fn show_percentage_grid_is_on_the_boolean_allowlist() {
        let patch = serde_json::json!({"showPercentageGrid": true});
        let changes = validate_settings_patch(&patch, false).unwrap();
        assert_eq!(changes.show_percentage_grid, Some(true));
        assert!(validate_settings_patch(&serde_json::json!({"showPercentageGrid": false}), false).is_ok());
        // The boolean branch stays strict, and unknown fields stay rejected.
        assert!(validate_settings_patch(&serde_json::json!({"showPercentageGrid": "yes"}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"showPercentageGrid": 1}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"showPercentageGrid": null}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"futureField": true}), false).is_err());
    }

    /// The offline half of P3-4: the same `validate -> merge -> save` chain the
    /// command runs, minus the `&AppHandle`-only side effects. The reload asserts the
    /// *value*, so a key that silently fails to map onto `OverlaySettings` (or a
    /// renamed field whose serde name no longer matches the camelCase wire key) makes
    /// this fail rather than pass with the toggle simply doing nothing.
    #[test]
    fn a_show_percentage_grid_patch_survives_the_persistence_chain() {
        let dir = std::env::temp_dir().join(format!(
            "codex-usage-overlay-grid-patch-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::for_test(ConfigManager::with_runtime_dir(dir.clone()));

        let patch = serde_json::json!({"showPercentageGrid": true});
        assert_eq!(validate_settings_patch(&patch, false).unwrap().show_percentage_grid, Some(true));

        // `apply_settings_patch` (which needs an `&AppHandle`) is replaced here by its
        // window-free core plus the same `ConfigManager` it writes through.
        let mut settings = state.config_manager.load_settings();
        assert!(!settings.show_percentage_grid, "the default must be off");
        merge_settings_patch(&mut settings, &patch).expect("the patch must merge");
        assert!(settings.show_percentage_grid, "the key must map onto the field");
        state.config_manager.save_settings_checked(&settings).expect("the write must succeed");

        // Read back through a fresh manager, i.e. from the file on disk.
        let reloaded = ConfigManager::with_runtime_dir(dir.clone()).load_settings();
        assert!(reloaded.show_percentage_grid);
        let raw = std::fs::read_to_string(state.config_manager.settings_path()).unwrap();
        assert!(raw.contains("\"showPercentageGrid\": true"), "wire key missing from {raw}");

        // The command also bumps its revision on a successful patch; the state field
        // is what a frontend assertion sees, so prove the two agree here.
        state.settings_revision.fetch_add(1, Ordering::SeqCst);
        assert_eq!(state.settings_revision.load(Ordering::SeqCst), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_update_channel_is_a_boolean_setting_that_defaults_to_stable() {
        assert!(!OverlaySettings::default().include_prerelease_updates);
        assert!(validate_settings_patch(&serde_json::json!({"includePrereleaseUpdates": true}), false).is_ok());
        assert!(validate_settings_patch(&serde_json::json!({"includePrereleaseUpdates": "yes"}), false).is_err());
        let mut settings = OverlaySettings::default();
        merge_settings_patch(&mut settings, &serde_json::json!({"includePrereleaseUpdates": true})).unwrap();
        assert!(settings.include_prerelease_updates);
    }

    #[test]
    fn live_claude_reading_is_opt_in_and_the_logo_is_shown_by_default() {
        let defaults = OverlaySettings::default();
        assert!(!defaults.claude_live_polling && defaults.antigravity_live_polling && defaults.show_provider_logo);
        // An older settings file has none of these keys.
        let upgraded: OverlaySettings = serde_json::from_str("{}").unwrap();
        assert!(!upgraded.claude_live_polling && upgraded.antigravity_live_polling && upgraded.show_provider_logo);
        let patch = serde_json::json!({"claudeLivePolling": true, "antigravityLivePolling": false, "showProviderLogo": false});
        assert!(validate_settings_patch(&patch, false).is_ok());
        assert!(validate_settings_patch(&serde_json::json!({"claudeLivePolling": 1}), false).is_err());
        assert!(validate_settings_patch(&serde_json::json!({"antigravityLivePolling": "yes"}), false).is_err());
    }

    /// Negative control: a patch that is not an object, or that carries an unknown or legacy key,
    /// must be rejected instead of mutating settings.
    #[test]
    fn merge_rejects_non_objects_and_unknown_keys() {
        let mut settings = OverlaySettings::default();
        assert!(merge_settings_patch(&mut settings, &serde_json::json!("nope")).is_err());
        assert!(merge_settings_patch(&mut settings, &serde_json::json!([1, 2])).is_err());
        assert!(merge_settings_patch(&mut settings, &serde_json::json!({"showPercentageGridTypo": true})).is_err());
        assert!(merge_settings_patch(&mut settings, &serde_json::json!({"autoPauseOnLowQuota": true})).is_err());
        assert!(!settings.show_percentage_grid);
    }
}


pub(crate) async fn apply_settings_patch<S: TrayPresentationSink>(
    sink: &S,
    state: &Arc<AppState>,
    patch: Value,
    allow_auto_start: bool,
) -> Result<SettingsEnvelope, String> {
    // Serialize native side effects with the matching persisted preference, even
    // when Settings and the tray submit changes at the same time.
    let _patch_guard = state.settings_patch.lock().await;
    let previous_dsh_enabled = dsh_monitor_enabled(&*state.settings.lock().await);
    let patch = if patch == serde_json::json!({"cycleProvider": true}) {
        let current = crate::display::effective(state);
        let next = state.monitors.next(current).ok_or("No other monitored provider is available")?;
        serde_json::json!({"provider": next})
    } else { patch };
    let mut changes = validate_settings_patch(&patch, allow_auto_start)?;
    if changes.quota_carousel_enabled == Some(false)
        && state.settings.lock().await.quota_carousel_enabled
        && changes.provider.is_none()
    {
        changes.provider = Some(crate::display::effective(state));
    }
    if changes.dsh_account_id.is_some() || changes.dsh_session_ids.is_some() {
        return Err("Use the current DSH connection revision to bind protection targets".into());
    }
    let (
        mut envelope,
        auto_check_just_enabled,
        auto_install_just_enabled,
        auto_edge_hide_changed,
        previous_auto_edge_hide,
        mouse_passthrough_changed,
        previous_mouse_passthrough,
        geometry_changed,
        update_channel_changed,
    ) = {
        let mut settings = state.settings.lock().await;
        let was_auto_edge_hide = settings.auto_edge_hide;
        let was_mouse_passthrough = settings.mouse_passthrough;
        let previous_settings = settings.clone();
        let mut next = settings.clone();
        changes.apply_to(&mut next);
        reconcile_update_preferences(&mut next, &changes);
        state.config_manager.save_settings_checked(&next)?;
        let auto_check_just_enabled = !settings.auto_check_updates && next.auto_check_updates;
        let auto_install_just_enabled = !settings.auto_install_updates && next.auto_install_updates;
        *settings = next.clone();
        let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
        (
            SettingsEnvelope::new(state, revision, next.clone()),
            auto_check_just_enabled,
            auto_install_just_enabled,
            was_auto_edge_hide != next.auto_edge_hide,
            was_auto_edge_hide,
            was_mouse_passthrough != next.mouse_passthrough,
            was_mouse_passthrough,
            overlay_geometry_changed(&previous_settings, &next),
            previous_settings.include_prerelease_updates != next.include_prerelease_updates,
        )
    };

    // An update found on the other channel must not stay offered or prepared.
    if update_channel_changed {
        *state.pending_update.lock().await = None;
        *state.available_update.lock().await = None;
        *state.update_error.lock().await = None;
        if let Ok(mut prepared) = state.prepared_update.lock() {
            *prepared = None;
        }
    }

    let next_dsh_enabled = dsh_monitor_enabled(&envelope.settings);
    if previous_dsh_enabled != next_dsh_enabled {
        state.dsh_auto.provider_selected(&state.dsh, if next_dsh_enabled { Provider::Dsh } else { Provider::Codex });
    }
    let changed_providers = state.monitors.synchronize(state, &envelope.settings).await;
    crate::display::settings_committed(sink, state, &envelope.settings, changes.provider.is_some()).await;
    let mut reprojected = Vec::new();
    if let Some(prefs) = &changes.provider_preferences {
        for (provider, pref_patch) in prefs {
            if pref_patch.five_hour_burn_rate_algorithm.is_some() {
                let runtime = state.monitors.get(*provider);
                let _commit = runtime.source_commit.lock().await;
                crate::presentation::reproject_locked(sink, &runtime).await;
                reprojected.push(*provider);
            }
        }
    }
    for provider in changed_providers {
        if !reprojected.contains(&provider) {
            state.monitors.publish_one(sink, provider).await;
        }
    }
    envelope.source_generation = state.monitors.get(envelope.settings.provider).source_generation.load(Ordering::Acquire);

    if let Some(app) = sink.app_handle() {
        if auto_edge_hide_changed {
            if let Err(error) = crate::dock::auto_hide_changed(
                app,
                state,
                envelope.settings.auto_edge_hide,
            )
            .await
            {
                let rollback = {
                    let mut settings = state.settings.lock().await;
                    let mut reverted = settings.clone();
                    reverted.auto_edge_hide = previous_auto_edge_hide;
                    let persistence_error = state
                        .config_manager
                        .save_settings_checked(&reverted)
                        .err();
                    *settings = reverted.clone();
                    let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
                    (SettingsEnvelope::new(state, revision, reverted), persistence_error)
                };
                let _ = app.emit("settings_updated", &rollback.0);
                return Err(match rollback.1 {
                    Some(write_error) => format!(
                        "Could not update docked overlay: {error}; could not persist rollback: {write_error}"
                    ),
                    None => format!("Could not update docked overlay: {error}"),
                });
            }
        }
        if mouse_passthrough_changed {
            if let Err(error) = set_main_mouse_passthrough(app, envelope.settings.mouse_passthrough) {
                let rollback = {
                    let mut settings = state.settings.lock().await;
                    let mut reverted = settings.clone();
                    reverted.mouse_passthrough = previous_mouse_passthrough;
                    let persistence_error = state.config_manager.save_settings_checked(&reverted).err();
                    *settings = reverted.clone();
                    let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
                    (SettingsEnvelope::new(state, revision, reverted), persistence_error)
                };
                let _ = app.emit("settings_updated", &rollback.0);
                return Err(match rollback.1 {
                    Some(write_error) => format!(
                        "Could not change mouse passthrough: {error}; could not persist rollback: {write_error}"
                    ),
                    None => format!("Could not change mouse passthrough: {error}"),
                });
            }
        }
        if geometry_changed {
            crate::dock::keep_in_work_area(app, state).await;
        }
    }
    let new_settings = &envelope.settings;
    if !new_settings.auto_install_updates {
        if let Ok(mut prepared) = state.prepared_update.lock() {
            *prepared = None;
        }
    }

    crate::tray::repaint_tray(sink, state).await;
    state.status_wake.notify_one();

    if let Some(app) = sink.app_handle() {
        // Update settings window title
        let displayed = state.monitors.get(Provider::ALL[state.displayed_provider.load(Ordering::Acquire) as usize]);
        crate::pause_control::status::publish(app, &displayed).await;
        if let Some(settings_win) = app.get_webview_window("settings") {
            let title = match resolve_locale(&new_settings.language) {
                "zh-CN" => "设置",
                "zh-Hant" => "設定",
                "ru-RU" => "Настройки",
                _ => "Settings",
            };
            let _ = settings_win.set_title(title);
        }

        // Broadcast updated settings to all windows
        let _ = app.emit("settings_updated", &envelope);
        if auto_check_just_enabled || auto_install_just_enabled
            || (update_channel_changed && new_settings.auto_check_updates) {
            let app_for_check = app.clone();
            let state_for_check = state.clone();
            tauri::async_runtime::spawn(async move {
                check_and_notify_auto_update(&app_for_check, &state_for_check).await;
            });
        }
    }
    Ok(envelope)
}

