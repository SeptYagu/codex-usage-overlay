//! Local quota usage history: what each provider's limits read over time.
//!
//! Every successful read passes through [`record_reading`], which keeps a line
//! only when a reading changed or a heartbeat is due. Lines are appended to
//! one JSONL file per series and calendar month and kept for 90 days.
//! [`query`] turns a series into the timeline, daily consumption and window
//! summaries the History window draws.

pub mod query;
pub mod store;

use serde::{Deserialize, Serialize};

use crate::codex::{QuotaSnapshot, RawCodexUsage};
use crate::config::{OverlaySettings, Provider};

pub use store::Store;

/// How long recorded readings are kept, and the widest range a query covers.
pub const RETENTION_SECS: i64 = 90 * 24 * 60 * 60;
/// An unchanged reading is still written this often, so the charts can tell
/// an idle stretch from one where the app was off or reads were failing.
pub const HEARTBEAT_SECS: i64 = 30 * 60;
/// A reading captured longer ago than this is a stale snapshot (an idle
/// statusline or bridge file), not a fresh observation, and is not recorded.
pub const MAX_READING_AGE_SECS: i64 = 10 * 60;

/// One quota window as recorded: used percent and, when known, its reset time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowReading {
    pub u: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<i64>,
}

/// One line of a history file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// When the provider reported the reading (`fetched_at`), Unix seconds.
    pub t: i64,
    /// A short hash of the account identity, so an account switch starts a
    /// new series segment without storing the identifier itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acct: Option<String>,
    /// Antigravity's effective quota pool for this reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    /// Five-hour window.
    #[serde(default)]
    pub fh: Option<WindowReading>,
    /// Weekly window. For Antigravity it is always the pool's weekly limit
    /// shown as WK, never Argon's.
    #[serde(default)]
    pub wk: Option<WindowReading>,
    /// Antigravity's Argon weekly (WTU) window, while agy still reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ar: Option<WindowReading>,
}

/// Reset times this close are one reset time read twice.
const RESET_JITTER_SECS: i64 = 60;

impl WindowReading {
    fn same_as(self, other: WindowReading) -> bool {
        self.u == other.u
            && match (self.r, other.r) {
                (Some(a), Some(b)) => (a - b).abs() <= RESET_JITTER_SECS,
                (a, b) => a == b,
            }
    }

    /// The reading as the history compares it. A 0% window has no reset: no
    /// window is in use, and the reset providers report for it means
    /// nothing. Codex reports one a full window from every read, so it
    /// slides forward each poll; agy reports one, then none, after a reset.
    /// Claude publishes whole-minute resets but reads them back a second
    /// early now and then.
    fn normalized(self, provider: Provider) -> WindowReading {
        let r = self.r.filter(|_| self.u > 0.0);
        let r = r.map(|r| if provider == Provider::Claude { (r + 30).div_euclid(60) * 60 } else { r });
        WindowReading { u: self.u, r }
    }
}

impl Record {
    /// True when nothing a chart would draw differs from `other`.
    pub fn same_reading(&self, other: &Record) -> bool {
        let same = |a: Option<WindowReading>, b: Option<WindowReading>| match (a, b) {
            (Some(a), Some(b)) => a.same_as(b),
            (a, b) => a.is_none() && b.is_none(),
        };
        self.acct == other.acct
            && self.pool == other.pool
            && same(self.fh, other.fh)
            && same(self.wk, other.wk)
            && same(self.ar, other.ar)
    }

    /// The record with each window [`WindowReading::normalized`]; lines
    /// written before normalization existed read the same as new ones.
    pub fn normalized(&self, provider: Provider) -> Record {
        Record {
            fh: self.fh.map(|w| w.normalized(provider)),
            wk: self.wk.map(|w| w.normalized(provider)),
            ar: self.ar.map(|w| w.normalized(provider)),
            ..self.clone()
        }
    }

    pub fn window(&self, kind: WindowKind) -> Option<WindowReading> {
        match kind {
            WindowKind::FiveHour => self.fh,
            WindowKind::Weekly => self.wk,
            WindowKind::Argon => self.ar,
        }
    }

    pub fn window_mut(&mut self, kind: WindowKind) -> &mut Option<WindowReading> {
        match kind {
            WindowKind::FiveHour => &mut self.fh,
            WindowKind::Weekly => &mut self.wk,
            WindowKind::Argon => &mut self.ar,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowKind {
    FiveHour,
    Weekly,
    Argon,
}

impl WindowKind {
    pub const ALL: [Self; 3] = [Self::FiveHour, Self::Weekly, Self::Argon];

    pub fn duration_secs(self) -> i64 {
        match self {
            Self::FiveHour => 5 * 60 * 60,
            Self::Weekly | Self::Argon => 7 * 24 * 60 * 60,
        }
    }

    /// Providers move `resets_at` by a few seconds between reads. A new window
    /// starts at the first message after the old one ended, so its reset is
    /// at least a full window later; anything closer is the same window.
    pub fn reset_tolerance_secs(self) -> i64 {
        match self {
            Self::FiveHour => 30 * 60,
            Self::Weekly | Self::Argon => 6 * 60 * 60,
        }
    }
}

/// The history series a provider runtime writes to. DSH follows the verified
/// instance and account (like the burn-rate history), so a reconnect with a
/// new address keeps one series. Antigravity keeps one series for every pool
/// and tags each line with the pool instead.
pub fn series_key(provider: Provider, settings: &OverlaySettings) -> String {
    match provider {
        Provider::Codex => "codex".into(),
        Provider::Claude => "claude".into(),
        Provider::Antigravity => "antigravity".into(),
        Provider::Dsh => crate::config::estimator_source_key_for(Provider::Dsh, settings),
    }
}

/// The provider a series key belongs to.
pub fn provider_of(series: &str) -> Option<Provider> {
    match series {
        "codex" => Some(Provider::Codex),
        "claude" => Some(Provider::Claude),
        "antigravity" => Some(Provider::Antigravity),
        s if s.starts_with("dsh-") => Some(Provider::Dsh),
        _ => None,
    }
}

/// A series key is used in file names, so only a plain token is accepted.
pub fn valid_series(series: &str) -> bool {
    !series.is_empty()
        && series.len() <= 64
        && series.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && provider_of(series).is_some()
}

fn account_key(account_id: &str) -> String {
    // Stable FNV-1a: tells accounts apart without keeping the identifier.
    let hash = account_id.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

fn window_reading(snapshot: Option<&QuotaSnapshot>) -> Option<WindowReading> {
    let snapshot = snapshot?;
    let used = snapshot.used_percent?;
    if !used.is_finite() {
        return None;
    }
    Some(WindowReading {
        u: (used.clamp(0.0, 100.0) * 100.0).round() / 100.0,
        r: snapshot.resets_at,
    })
}

/// Builds the line a successful read would record, or `None` when the reading
/// is too old to count as an observation or carries no window at all.
pub fn record_for(provider: Provider, raw: &RawCodexUsage, now: i64) -> Option<Record> {
    if raw.fetched_at <= 0 || now.saturating_sub(raw.fetched_at) > MAX_READING_AGE_SECS {
        return None;
    }
    // Claude's five-hour window is projected to 0% once its reset passes; the
    // captured window is the real observation.
    let five_hour = match provider {
        Provider::Claude => raw.notification_five_hour.as_ref().or(Some(&raw.five_hour)),
        _ => Some(&raw.five_hour),
    };
    // Antigravity's `weekly` is the projected pool's own window. When that is
    // Argon, the WK reading is Gemini's, carried alongside.
    let (weekly, argon) = match raw.antigravity_weekly.as_ref() {
        Some(extra) if extra.weekly_is_argon => (extra.gemini.as_ref(), Some(&raw.weekly)),
        Some(extra) => (Some(&raw.weekly), extra.argon.as_ref()),
        None => (Some(&raw.weekly), None),
    };
    let record = Record {
        t: raw.fetched_at,
        acct: raw.account_id.as_deref().filter(|id| !id.is_empty()).map(account_key),
        pool: raw.quota_pool.map(str::to_string),
        fh: window_reading(five_hour),
        wk: window_reading(weekly),
        ar: window_reading(argon),
    };
    let record = record.normalized(provider);
    (record.fh.is_some() || record.wk.is_some() || record.ar.is_some()).then_some(record)
}

/// Whether `next` should be written after `last`, the series' latest line.
/// Readings that are not newer are dropped: repeated reads of one snapshot,
/// and clock changes that would put a line out of order.
pub fn should_record(last: Option<&Record>, next: &Record) -> bool {
    match last {
        None => true,
        Some(last) if next.t <= last.t => false,
        Some(last) => !next.same_reading(last) || next.t - last.t >= HEARTBEAT_SECS,
    }
}

/// Records a successful read when history is on. Failures are logged and never
/// affect the read itself.
pub fn record_reading(store: &Store, provider: Provider, settings: &OverlaySettings, raw: &RawCodexUsage, now: i64) {
    if !settings.record_usage_history {
        return;
    }
    let Some(record) = record_for(provider, raw, now) else { return };
    let series = series_key(provider, settings);
    if let Err(error) = store.append(&series, record, now) {
        eprintln!("Could not record usage history for {series}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::AntigravityWeeklies;

    const NOW: i64 = 1_795_000_000;

    fn snapshot(used: f64, resets_at: i64) -> QuotaSnapshot {
        QuotaSnapshot { used_percent: Some(used), resets_at: Some(resets_at), window_duration_mins: None }
    }

    pub(crate) fn raw(fetched_at: i64, five: f64, week: f64) -> RawCodexUsage {
        RawCodexUsage {
            account_id: Some("account-1".into()),
            five_hour: snapshot(five, NOW + 3_600),
            weekly: snapshot(week, NOW + 86_400),
            credits_display: "—".into(),
            credits_balance: None,
            has_credits: false,
            fetched_at,
            antigravity_weekly: None,
            notification_five_hour: None,
            quota_pool: None,
        }
    }

    #[test]
    fn a_fresh_reading_becomes_a_record_and_a_stale_one_does_not() {
        let record = record_for(Provider::Codex, &raw(NOW - 5, 12.345, 40.0), NOW).unwrap();
        assert_eq!(record.t, NOW - 5);
        assert_eq!(record.fh, Some(WindowReading { u: 12.35, r: Some(NOW + 3_600) }));
        assert_eq!(record.wk, Some(WindowReading { u: 40.0, r: Some(NOW + 86_400) }));
        assert_eq!(record.ar, None);
        let acct = record.acct.unwrap();
        assert_eq!(acct.len(), 16);
        assert!(!acct.contains("account"));
        assert!(record_for(Provider::Codex, &raw(NOW - MAX_READING_AGE_SECS - 1, 1.0, 1.0), NOW).is_none());
    }

    #[test]
    fn a_reading_without_any_window_is_not_recorded() {
        let mut empty = raw(NOW, 0.0, 0.0);
        empty.five_hour = QuotaSnapshot::default();
        empty.weekly = QuotaSnapshot::default();
        assert!(record_for(Provider::Codex, &empty, NOW).is_none());
    }

    #[test]
    fn claude_records_its_captured_window_not_the_projection() {
        let mut claude = raw(NOW, 0.0, 10.0);
        claude.five_hour = QuotaSnapshot { used_percent: Some(0.0), resets_at: None, window_duration_mins: None };
        // NOW is 40 s past a minute; Claude's whole-minute reset is read as
        // a second early, and recorded as the minute.
        claude.notification_five_hour = Some(snapshot(73.0, NOW + 19));
        let record = record_for(Provider::Claude, &claude, NOW).unwrap();
        assert_eq!(record.fh, Some(WindowReading { u: 73.0, r: Some(NOW + 20) }));
        // Other providers never read the Claude-only field.
        assert_eq!(record_for(Provider::Codex, &claude, NOW).unwrap().fh, Some(WindowReading { u: 0.0, r: None }));
    }

    #[test]
    fn antigravity_keeps_wk_as_the_gemini_limit_and_argon_apart() {
        let mut argon_pool = raw(NOW, 20.0, 80.0);
        argon_pool.quota_pool = Some("argon");
        argon_pool.antigravity_weekly = Some(AntigravityWeeklies {
            argon: Some(snapshot(80.0, NOW + 86_400)),
            weekly_is_argon: true,
            gemini: Some(snapshot(45.0, NOW + 50_000)),
        });
        let record = record_for(Provider::Antigravity, &argon_pool, NOW).unwrap();
        assert_eq!(record.pool.as_deref(), Some("argon"));
        assert_eq!(record.wk.unwrap().u, 45.0);
        assert_eq!(record.ar.unwrap().u, 80.0);

        let mut gemini_pool = raw(NOW, 20.0, 45.0);
        gemini_pool.quota_pool = Some("gemini");
        gemini_pool.antigravity_weekly = Some(AntigravityWeeklies {
            argon: Some(snapshot(80.0, NOW + 86_400)),
            weekly_is_argon: false,
            gemini: None,
        });
        let record = record_for(Provider::Antigravity, &gemini_pool, NOW).unwrap();
        assert_eq!(record.wk.unwrap().u, 45.0);
        assert_eq!(record.ar.unwrap().u, 80.0);
    }

    #[test]
    fn only_changes_and_heartbeats_are_written() {
        let first = record_for(Provider::Codex, &raw(NOW, 10.0, 40.0), NOW).unwrap();
        assert!(should_record(None, &first));
        let same = Record { t: NOW + 60, ..first.clone() };
        assert!(!should_record(Some(&first), &same));
        let heartbeat = Record { t: NOW + HEARTBEAT_SECS, ..first.clone() };
        assert!(should_record(Some(&first), &heartbeat));
        let changed = Record { t: NOW + 60, fh: Some(WindowReading { u: 11.0, r: Some(NOW + 3_600) }), ..first.clone() };
        assert!(should_record(Some(&first), &changed));
        let other_account = Record { t: NOW + 60, acct: Some("other".into()), ..first.clone() };
        assert!(should_record(Some(&first), &other_account));
        // A repeated snapshot or a clock step backwards is never written.
        assert!(!should_record(Some(&changed), &Record { t: NOW + 60, ..changed.clone() }));
        assert!(!should_record(Some(&changed), &Record { t: NOW, fh: None, ..changed.clone() }));
    }

    #[test]
    fn an_unused_window_records_no_reset_and_jitter_is_no_change() {
        // Codex projects an unused window's reset from every read.
        let mut idle = raw(NOW, 0.0, 40.0);
        idle.five_hour = snapshot(0.0, NOW + 5 * 3_600 - 30);
        let first = record_for(Provider::Codex, &idle, NOW).unwrap();
        assert_eq!(first.fh, Some(WindowReading { u: 0.0, r: None }));
        idle.fetched_at = NOW + 60;
        idle.five_hour = snapshot(0.0, NOW + 60 + 5 * 3_600 - 30);
        assert!(!should_record(Some(&first), &record_for(Provider::Codex, &idle, NOW + 60).unwrap()));
        // A used window keeps its reset, and a second's jitter is not a change.
        let used = record_for(Provider::Codex, &raw(NOW, 10.0, 40.0), NOW).unwrap();
        let jitter = Record { t: NOW + 60, fh: Some(WindowReading { u: 10.0, r: Some(NOW + 3_599) }), ..used.clone() };
        assert!(!should_record(Some(&used), &jitter));
    }

    #[test]
    fn series_keys_are_safe_file_tokens() {
        let settings = OverlaySettings::default();
        for provider in Provider::ALL {
            let key = series_key(provider, &settings);
            assert!(valid_series(&key), "{key}");
            assert_eq!(provider_of(&key), Some(provider));
        }
        assert!(!valid_series("../codex"));
        assert!(!valid_series("codex.jsonl"));
        assert!(!valid_series("unknown"));
    }
}
