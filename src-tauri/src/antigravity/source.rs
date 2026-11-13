//! The Antigravity provider's quota read: a local snapshot file, no process
//! and no network credentials.
use std::path::Path;

use super::shared::{self, PoolKind};
use crate::codex::reader::{FailureKind, ReadFailure, ReadStage};
use crate::codex::{QuotaSnapshot, RawCodexUsage};

const FIVE_HOUR_MINS: u32 = 5 * 60;
const SEVEN_DAY_MINS: u32 = 7 * 24 * 60;

#[derive(Debug, Clone)]
pub struct AntigravitySample {
    pub usage: RawCodexUsage,
    pub stale: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub effective_pool: PoolKind,
}

/// Reads the captured Antigravity snapshot and projects the requested quota
/// pool (`auto`, `gemini`, `argon`, or `3p`).
pub fn read_usage(dir: &Path, pool: PoolKind, now: i64) -> Result<AntigravitySample, ReadFailure> {
    if !dir.join(shared::SNAPSHOT_FILE).is_file() {
        return Err(ReadFailure::new(ReadStage::Discovery, FailureKind::NotConnected));
    }
    let snapshot = shared::read_snapshot(dir)
        .ok_or_else(|| ReadFailure::new(ReadStage::Parse, FailureKind::InvalidResponse))?;
    Ok(sample(&snapshot, pool, now))
}

pub fn describe(failure: &ReadFailure) -> String {
    match failure.kind {
        FailureKind::NotConnected => "尚未收到 Antigravity 额度 / No Antigravity limits received yet".into(),
        FailureKind::MissingExecutable => "未找到 Antigravity CLI (agy) / Antigravity CLI (agy) not found".into(),
        FailureKind::Authentication => "需要登录 Antigravity / Antigravity login required".into(),
        kind => format!(
            "读取 Antigravity 用量失败 / Antigravity usage read failed ({kind:?}, HTTP {:?})",
            failure.http_status
        ),
    }
}

pub fn sample(snapshot: &shared::Snapshot, pool: PoolKind, now: i64) -> AntigravitySample {
    let effective_pool = snapshot.resolve_pool(pool);
    let windows = snapshot.windows_for(effective_pool);
    let map_window = |window: Option<shared::WindowSnapshot>, minutes: u32| {
        window.map(|w| w.at(now)).map_or_else(QuotaSnapshot::default, |w| QuotaSnapshot {
            used_percent: Some(w.used_percent),
            resets_at: w.resets_at,
            window_duration_mins: Some(minutes),
        })
    };
    let credits_display = snapshot
        .credits_display
        .as_deref()
        .filter(|_| snapshot.has_credits)
        .unwrap_or("—")
        .to_string();
    AntigravitySample {
        usage: RawCodexUsage {
            account_id: snapshot
                .account_id
                .clone()
                .or_else(|| Some(format!("antigravity:{}", effective_pool.key()))),
            five_hour: map_window(windows.five_hour, FIVE_HOUR_MINS),
            weekly: map_window(windows.weekly, SEVEN_DAY_MINS),
            credits_display,
            credits_balance: if snapshot.has_credits {
                snapshot.credits_balance.clone()
            } else {
                None
            },
            has_credits: snapshot.has_credits,
            fetched_at: snapshot.captured_at,
        },
        stale: snapshot.age(now) > shared::SNAPSHOT_FRESH_SECS,
        effective_pool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_snapshot_is_not_connected_and_broken_is_invalid() {
        let dir = std::env::temp_dir().join(format!("antigravity-source-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_usage(&dir, PoolKind::Auto, 0).err().unwrap().kind, FailureKind::NotConnected);
        std::fs::write(dir.join(shared::SNAPSHOT_FILE), "[]").unwrap();
        assert_eq!(read_usage(&dir, PoolKind::Auto, 0).err().unwrap().kind, FailureKind::InvalidResponse);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switching_pool_reprojects_without_new_statusline_event() {
        let dir = std::env::temp_dir().join(format!("antigravity-source-pool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = 1_793_000_000;
        let input = json!({
            "active_model_id": "gemini-3.1-pro",
            "rate_limits": {
                "gemini-5h": { "used_percentage": 20.0, "resets_at": now + 100 },
                "gemini-weekly": { "used_percentage": 45.0, "resets_at": now + 50_000 },
                "gemini-argon-wtu-weekly": { "used_percentage": 80.0, "resets_at": now + 60_000 },
                "3p-5h": { "used_percentage": 10.0, "resets_at": now + 200 },
                "3p-weekly": { "used_percentage": 30.0, "resets_at": now + 70_000 }
            }
        });
        let snapshot = shared::parse_statusline(&input, now).unwrap();
        shared::write_json_atomic(&dir.join(shared::SNAPSHOT_FILE), &snapshot).unwrap();

        let auto_sample = read_usage(&dir, PoolKind::Auto, now + 10).unwrap();
        assert_eq!(auto_sample.effective_pool, PoolKind::Gemini);
        assert_eq!(auto_sample.usage.weekly.used_percent, Some(45.0));

        let argon_sample = read_usage(&dir, PoolKind::Argon, now + 10).unwrap();
        assert_eq!(argon_sample.effective_pool, PoolKind::Argon);
        assert_eq!(argon_sample.usage.five_hour.used_percent, Some(20.0));
        assert_eq!(argon_sample.usage.weekly.used_percent, Some(80.0));

        let tp_sample = read_usage(&dir, PoolKind::ThirdParty, now + 10).unwrap();
        assert_eq!(tp_sample.effective_pool, PoolKind::ThirdParty);
        assert_eq!(tp_sample.usage.five_hour.used_percent, Some(10.0));
        assert_eq!(tp_sample.usage.weekly.used_percent, Some(30.0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn describe_uses_antigravity_specific_messages() {
        let not_connected = describe(&ReadFailure::new(ReadStage::Discovery, FailureKind::NotConnected));
        assert!(not_connected.contains("Antigravity") && !not_connected.contains("Claude"), "{not_connected}");
        let missing = describe(&ReadFailure::new(ReadStage::Discovery, FailureKind::MissingExecutable));
        assert!(missing.contains("agy"), "{missing}");
    }
}
