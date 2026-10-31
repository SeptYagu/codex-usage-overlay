//! The Claude provider's quota read: a local file, no process and no network.
use std::path::Path;

use super::shared;
use crate::codex::reader::{FailureKind, ReadFailure, ReadStage};
use crate::codex::{QuotaSnapshot, RawCodexUsage};

const FIVE_HOUR_MINS: u32 = 5 * 60;
const SEVEN_DAY_MINS: u32 = 7 * 24 * 60;

pub struct ClaudeSample {
    pub usage: RawCodexUsage,
    /// The snapshot is older than the freshness window: shown with its age,
    /// never acted on as a current reading.
    pub stale: bool,
}

/// Maps the stored snapshot onto the shared sample shape, so burn rate, reset
/// notifications, tray and dock handle Claude like Codex. The sample time is
/// the capture time, so an unchanged file never looks like a new observation.
pub fn read_usage(dir: &Path, now: i64) -> Result<ClaudeSample, ReadFailure> {
    if !dir.join(shared::SNAPSHOT_FILE).is_file() {
        return Err(ReadFailure::new(ReadStage::Discovery, FailureKind::NotConnected));
    }
    let snapshot = shared::read_snapshot(dir)
        .ok_or_else(|| ReadFailure::new(ReadStage::Parse, FailureKind::InvalidResponse))?;
    let window = |window: Option<shared::WindowSnapshot>, minutes: u32| {
        window.map(|w| w.at(now)).map_or_else(QuotaSnapshot::default, |w| QuotaSnapshot {
            used_percent: Some(w.used_percent),
            resets_at: w.resets_at,
            window_duration_mins: Some(minutes),
        })
    };
    Ok(ClaudeSample {
        usage: RawCodexUsage {
            account_id: None,
            five_hour: window(snapshot.five_hour, FIVE_HOUR_MINS),
            weekly: window(snapshot.seven_day, SEVEN_DAY_MINS),
            credits_display: "—".into(),
            credits_balance: None,
            has_credits: false,
            fetched_at: snapshot.captured_at,
        },
        stale: snapshot.age(now) > shared::SNAPSHOT_FRESH_SECS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_missing_snapshot_means_not_connected_and_a_broken_one_is_invalid() {
        let dir = std::env::temp_dir().join(format!("claude-source-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_usage(&dir, 0).err().unwrap().kind, FailureKind::NotConnected);
        std::fs::write(dir.join(shared::SNAPSHOT_FILE), "[]").unwrap();
        assert_eq!(read_usage(&dir, 0).err().unwrap().kind, FailureKind::InvalidResponse);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_snapshot_maps_onto_the_shared_sample() {
        let dir = std::env::temp_dir().join(format!("claude-source-map-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = 1_793_000_000;
        let input = json!({ "rate_limits": {
            "five_hour": { "used_percentage": 40.5, "resets_at": now + 100 },
            "seven_day": { "used_percentage": 12, "resets_at": now + 50_000 }
        }});
        let snapshot = shared::parse_statusline(&input, now).unwrap();
        shared::write_json_atomic(&dir.join(shared::SNAPSHOT_FILE), &snapshot).unwrap();

        let sample = read_usage(&dir, now + 10).unwrap();
        assert!(!sample.stale);
        assert_eq!(sample.usage.fetched_at, now);
        assert_eq!(sample.usage.five_hour, QuotaSnapshot { used_percent: Some(40.5), resets_at: Some(now + 100), window_duration_mins: Some(300) });
        assert_eq!(sample.usage.weekly.window_duration_mins, Some(10_080));
        let ui = sample.usage.to_ui(None, None);
        assert_eq!((ui.five_hour_remaining_percent, ui.week_remaining_percent, ui.has_credits), (Some(60), Some(88), false));

        // After the 5-hour reset the window is reported as rolled over, and the
        // old capture is display-only.
        let later = read_usage(&dir, now + shared::SNAPSHOT_FRESH_SECS + 1).unwrap();
        assert!(later.stale);
        assert_eq!(later.usage.five_hour.used_percent, Some(0.0));
        assert_eq!(later.usage.five_hour.resets_at, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
