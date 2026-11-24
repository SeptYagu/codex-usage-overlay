//! Turns a series' records into what the History window draws: a timeline,
//! daily consumption and a summary of each quota window.

use std::collections::BTreeSet;

use chrono::{Days, NaiveDate, TimeZone};
use serde::Serialize;

use super::{provider_of, Record, WindowKind, WindowReading, HEARTBEAT_SECS};
use crate::config::Provider;

/// Two lines further apart than this mean the app was off or reads failed.
pub const GAP_SECS: i64 = 2 * HEARTBEAT_SECS + 5 * 60;
/// Timelines longer than this are thinned, keeping extremes and transitions.
pub const MAX_POINTS: usize = 1_200;
/// A window at or above this used percent has reached its limit.
const LIMIT_PERCENT: f64 = 99.95;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesInfo {
    pub series: String,
    pub provider: Provider,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
    pub records: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub t: i64,
    /// Lines are broken where this changes: a gap, an account or pool switch.
    pub seg: u32,
    pub five_hour: Option<f64>,
    pub weekly: Option<f64>,
    pub argon: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    /// A window reset between the previous reading and this one.
    #[serde(skip)]
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResetMarker {
    pub t: i64,
    pub kind: WindowKind,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DayUsage {
    /// Local calendar date, `YYYY-MM-DD`.
    pub day: String,
    /// Local midnight that starts the day, Unix seconds.
    pub start: i64,
    /// Percent of the five-hour limit consumed that day; several windows fit
    /// in a day, so this can exceed 100.
    pub five_hour: f64,
    /// Percent of the weekly limit consumed that day.
    pub weekly: f64,
    pub argon: f64,
    /// Part of the day's total spans a gap, so when it was used is uncertain.
    pub approximate: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowSummary {
    pub kind: WindowKind,
    pub start: i64,
    pub resets_at: i64,
    pub first_seen: i64,
    pub last_seen: i64,
    pub peak: f64,
    pub hit_limit: bool,
    pub limit_at: Option<i64>,
    pub time_to_limit_secs: Option<i64>,
    /// Percent never used before the reset; only for windows that have ended.
    pub unused_at_reset: Option<f64>,
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryView {
    pub series: String,
    pub provider: Provider,
    pub from: i64,
    pub to: i64,
    pub recording_since: Option<i64>,
    pub points: Vec<Point>,
    pub resets: Vec<ResetMarker>,
    pub gaps: Vec<Gap>,
    pub days: Vec<DayUsage>,
    pub windows: Vec<WindowSummary>,
    pub has_argon: bool,
}

pub fn series_info(series: &str, records: &[Record]) -> Option<SeriesInfo> {
    Some(SeriesInfo {
        series: series.to_string(),
        provider: provider_of(series)?,
        first_at: records.first().map(|r| r.t),
        last_at: records.last().map(|r| r.t),
        records: records.len(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Continuity {
    /// The same quota window as the previous reading.
    Same,
    /// The previous window reset and this is a new one.
    New,
    /// Another account or pool: the readings are not comparable.
    Break,
}

fn continuity(kind: WindowKind, prev: &Record, prev_w: WindowReading, cur: &Record, cur_w: WindowReading) -> Continuity {
    if prev.acct != cur.acct || prev.pool != cur.pool {
        return Continuity::Break;
    }
    match (prev_w.r, cur_w.r) {
        (Some(a), Some(b)) if (a - b).abs() <= kind.reset_tolerance_secs() => Continuity::Same,
        (Some(_), Some(_)) => Continuity::New,
        // The window ended and no new one has started yet. A reading that
        // only lost its reset time, still in use before it, is the same one.
        (Some(a), None) if cur_w.u <= 0.0 || cur.t >= a => Continuity::New,
        // A window starting from unused is not a reset, unless usage dropped.
        (None, Some(_)) if cur_w.u < prev_w.u => Continuity::New,
        _ => Continuity::Same,
    }
}

/// When the window before `cur` ended: its reported reset time, kept between
/// the two readings since providers reset a little early or late.
fn reset_time(prev: &Record, prev_w: WindowReading, cur: &Record) -> i64 {
    prev_w.r.map_or(cur.t, |r| r.clamp(prev.t, cur.t))
}

/// Readings of one window kind, oldest first, with their record.
fn readings(records: &[Record], kind: WindowKind) -> impl Iterator<Item = (&Record, WindowReading)> {
    records.iter().filter_map(move |record| record.window(kind).map(|w| (record, w)))
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn local_date<Tz: TimeZone>(tz: &Tz, t: i64) -> Option<NaiveDate> {
    tz.timestamp_opt(t, 0).earliest().map(|dt| dt.date_naive())
}

fn local_midnight<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> i64 {
    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight exists");
    tz.from_local_datetime(&midnight)
        .earliest()
        .map(|dt| dt.timestamp())
        // A DST jump at midnight: the day starts an hour later.
        .or_else(|| tz.from_local_datetime(&(midnight + chrono::Duration::hours(1))).earliest().map(|dt| dt.timestamp()))
        .unwrap_or_else(|| midnight.and_utc().timestamp())
}

fn daily<Tz: TimeZone>(records: &[Record], from: i64, to: i64, tz: &Tz) -> Vec<DayUsage> {
    let (Some(first_day), Some(last_day)) = (local_date(tz, from), local_date(tz, to)) else { return Vec::new() };
    let mut days = Vec::new();
    let mut date = first_day;
    while date <= last_day && days.len() <= 400 {
        days.push(DayUsage {
            day: date.format("%Y-%m-%d").to_string(),
            start: local_midnight(tz, date),
            five_hour: 0.0,
            weekly: 0.0,
            argon: 0.0,
            approximate: false,
        });
        let Some(next) = date.checked_add_days(Days::new(1)) else { break };
        date = next;
    }
    for kind in WindowKind::ALL {
        let mut prev: Option<(&Record, WindowReading)> = None;
        for (record, reading) in readings(records, kind) {
            if let Some((prev_record, prev_reading)) = prev {
                let used = match continuity(kind, prev_record, prev_reading, record, reading) {
                    Continuity::Same => (reading.u - prev_reading.u).max(0.0),
                    // A new window starts from zero, after the previous reading.
                    Continuity::New => reading.u,
                    Continuity::Break => 0.0,
                };
                if used > 0.0 && record.t >= from && record.t <= to {
                    let day = local_date(tz, record.t).map(|d| d.format("%Y-%m-%d").to_string());
                    if let Some(entry) = days.iter_mut().find(|entry| Some(&entry.day) == day.as_ref()) {
                        match kind {
                            WindowKind::FiveHour => entry.five_hour += used,
                            WindowKind::Weekly => entry.weekly += used,
                            WindowKind::Argon => entry.argon += used,
                        }
                        if record.t - prev_record.t > GAP_SECS {
                            entry.approximate = true;
                        }
                    }
                }
            }
            prev = Some((record, reading));
        }
    }
    for entry in &mut days {
        entry.five_hour = round2(entry.five_hour);
        entry.weekly = round2(entry.weekly);
        entry.argon = round2(entry.argon);
    }
    days
}

fn summarize(kind: WindowKind, group: &[(&Record, WindowReading)], now: i64) -> Option<WindowSummary> {
    let resets_at = group.iter().rev().find_map(|(_, w)| w.r)?;
    let start = resets_at - kind.duration_secs();
    let peak = group.iter().map(|(_, w)| w.u).fold(0.0_f64, f64::max);
    let limit_at = group.iter().find(|(_, w)| w.u >= LIMIT_PERCENT).map(|(record, _)| record.t);
    let complete = now >= resets_at;
    Some(WindowSummary {
        kind,
        start,
        resets_at,
        first_seen: group.first()?.0.t,
        last_seen: group.last()?.0.t,
        peak: round2(peak),
        hit_limit: limit_at.is_some(),
        limit_at,
        time_to_limit_secs: limit_at.map(|t| (t - start).max(0)),
        unused_at_reset: complete.then(|| round2((100.0 - peak).max(0.0))),
        complete,
        pool: group.first()?.0.pool.clone(),
    })
}

fn windows(records: &[Record], from: i64, to: i64, now: i64) -> (Vec<WindowSummary>, Vec<ResetMarker>) {
    let mut summaries = Vec::new();
    let mut resets = Vec::new();
    for kind in WindowKind::ALL {
        let mut group: Vec<(&Record, WindowReading)> = Vec::new();
        for (record, reading) in readings(records, kind) {
            if let Some(&(prev_record, prev_reading)) = group.last() {
                let link = continuity(kind, prev_record, prev_reading, record, reading);
                if link != Continuity::Same {
                    summaries.extend(summarize(kind, &group, now));
                    group.clear();
                    if link == Continuity::New {
                        let t = reset_time(prev_record, prev_reading, record);
                        if t >= from && t <= to {
                            resets.push(ResetMarker { t, kind });
                        }
                    }
                }
            }
            group.push((record, reading));
        }
        summaries.extend(summarize(kind, &group, now));
    }
    summaries.retain(|w| w.resets_at >= from && w.start <= to);
    summaries.sort_by(|a, b| b.resets_at.cmp(&a.resets_at).then(a.kind_order().cmp(&b.kind_order())));
    resets.sort_by_key(|marker| marker.t);
    (summaries, resets)
}

impl WindowSummary {
    fn kind_order(&self) -> u8 {
        match self.kind {
            WindowKind::FiveHour => 0,
            WindowKind::Weekly => 1,
            WindowKind::Argon => 2,
        }
    }
}

fn breaks_line(prev: &Record, cur: &Record) -> bool {
    cur.t - prev.t > GAP_SECS || prev.acct != cur.acct || prev.pool != cur.pool
}

/// Keeps every point of a short timeline. A long one keeps, per time bucket,
/// the first, last, lowest and highest reading of each window, plus both
/// sides of every line break and reset.
fn downsample(points: Vec<Point>, from: i64, to: i64) -> Vec<Point> {
    if points.len() <= MAX_POINTS {
        return points;
    }
    let getters: [fn(&Point) -> Option<f64>; 3] = [|p| p.five_hour, |p| p.weekly, |p| p.argon];
    let kinds = getters.iter().filter(|get| points.iter().any(|p| get(p).is_some())).count() as i64;
    // Each bucket keeps at most two points plus a low and a high per window.
    let buckets = ((MAX_POINTS as i64 - 100) / (2 + 2 * kinds)).max(1);
    let span = (to - from).max(1);
    let bucket_of = |t: i64| (t - from).clamp(0, span) * buckets / span;
    let mut keep = BTreeSet::new();
    let mut start = 0;
    while start < points.len() {
        let bucket = bucket_of(points[start].t);
        let mut end = start;
        while end + 1 < points.len() && bucket_of(points[end + 1].t) == bucket {
            end += 1;
        }
        keep.insert(start);
        keep.insert(end);
        for get in getters {
            let with_value = || (start..=end).filter_map(|i| get(&points[i]).map(|v| (i, v)));
            if let Some((i, _)) = with_value().min_by(|a, b| a.1.total_cmp(&b.1)) {
                keep.insert(i);
            }
            if let Some((i, _)) = with_value().max_by(|a, b| a.1.total_cmp(&b.1)) {
                keep.insert(i);
            }
        }
        start = end + 1;
    }
    for i in 1..points.len() {
        if points[i].reset || points[i].seg != points[i - 1].seg {
            keep.insert(i - 1);
            keep.insert(i);
        }
    }
    let mut points: Vec<Option<Point>> = points.into_iter().map(Some).collect();
    keep.into_iter().filter_map(|i| points[i].take()).collect()
}

fn starts_new_window(prev: &Record, cur: &Record) -> bool {
    WindowKind::ALL.into_iter().any(|kind| match (prev.window(kind), cur.window(kind)) {
        (Some(a), Some(b)) => continuity(kind, prev, a, cur, b) == Continuity::New,
        _ => false,
    })
}

/// The History window's data for `series` between `from` and `to`, with
/// calendar days in `tz`. `records` is the series' whole retained history,
/// so changes just before `from` still count.
pub fn view<Tz: TimeZone>(series: &str, records: &[Record], from: i64, to: i64, now: i64, tz: &Tz) -> Option<HistoryView> {
    let provider = provider_of(series)?;
    let records: Vec<Record> = records.iter().map(|record| record.normalized(provider)).collect();
    let records = records.as_slice();
    let mut points = Vec::new();
    let mut gaps = Vec::new();
    let mut seg = 0u32;
    let mut prev: Option<&Record> = None;
    for record in records {
        let reset = prev.is_some_and(|prev| starts_new_window(prev, record));
        if let Some(prev) = prev {
            if breaks_line(prev, record) {
                seg += 1;
            }
            if record.t - prev.t > GAP_SECS && record.t >= from && prev.t <= to {
                gaps.push(Gap { from: prev.t.max(from), to: record.t.min(to) });
            }
        }
        if record.t >= from && record.t <= to {
            points.push(Point {
                t: record.t,
                seg,
                five_hour: record.fh.map(|w| w.u),
                weekly: record.wk.map(|w| w.u),
                argon: record.ar.map(|w| w.u),
                pool: record.pool.clone(),
                reset,
            });
        }
        prev = Some(record);
    }
    if let Some(last) = records.last() {
        if now - last.t > GAP_SECS && now >= from && last.t <= to {
            gaps.push(Gap { from: last.t.max(from), to: now.min(to) });
        }
    }
    let (windows, resets) = windows(records, from, to, now);
    Some(HistoryView {
        series: series.to_string(),
        provider,
        from,
        to,
        recording_since: records.first().map(|r| r.t),
        has_argon: records.iter().any(|r| r.ar.is_some() && r.t >= from && r.t <= to),
        points: downsample(points, from, to),
        resets,
        gaps,
        days: daily(records, from, to, tz),
        windows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    // 2026-11-15T00:00:00Z
    const DAY0: i64 = 1_794_700_800;
    const H: i64 = 3_600;

    fn rec(t: i64, five: Option<(f64, i64)>, week: Option<(f64, i64)>) -> Record {
        Record {
            t,
            acct: Some("a".into()),
            pool: None,
            fh: five.map(|(u, r)| WindowReading { u, r: Some(r) }),
            wk: week.map(|(u, r)| WindowReading { u, r: Some(r) }),
            ar: None,
        }
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    #[test]
    fn reset_jitter_stays_in_one_window_and_a_real_reset_starts_another() {
        let reset1 = DAY0 + 5 * H;
        let reset2 = DAY0 + 11 * H;
        let records = vec![
            rec(DAY0, Some((10.0, reset1)), None),
            rec(DAY0 + 600, Some((40.0, reset1 + 7)), None),
            rec(DAY0 + 1_200, Some((100.0, reset1 - 4)), None),
            rec(DAY0 + 6 * H, Some((5.0, reset2)), None),
            rec(DAY0 + 6 * H + 600, Some((30.0, reset2 + 2)), None),
        ];
        let view = view("codex", &records, DAY0, DAY0 + 7 * H, DAY0 + 7 * H, &utc()).unwrap();
        let five: Vec<_> = view.windows.iter().filter(|w| w.kind == WindowKind::FiveHour).collect();
        assert_eq!(five.len(), 2);
        let (current, first) = (five[0], five[1]);
        assert_eq!(first.start, reset1 - 4 - 5 * H);
        assert!(first.hit_limit && first.complete);
        assert_eq!(first.limit_at, Some(DAY0 + 1_200));
        assert_eq!(first.time_to_limit_secs, Some(1_204));
        assert_eq!(first.unused_at_reset, Some(0.0));
        assert_eq!(current.peak, 30.0);
        assert!(!current.complete && !current.hit_limit);
        assert_eq!(current.unused_at_reset, None);
        // One reset, at the old window's reset time.
        assert_eq!(view.resets, vec![ResetMarker { t: reset1 - 4, kind: WindowKind::FiveHour }]);
    }

    fn five(t: i64, u: f64, r: Option<i64>) -> Record {
        Record { t, acct: Some("a".into()), pool: None, fh: Some(WindowReading { u, r }), wk: None, ar: None }
    }

    #[test]
    fn a_claude_reset_through_an_unused_window_is_one_reset_at_the_reset_time() {
        // The installed client's readings around a Claude reset: resets read
        // back a second early on alternate polls, then one 0% reading with no
        // reset before the next window's first use.
        let old = DAY0 + 5 * H;
        let new = old + 5 * H;
        let records = vec![
            five(old - 1_930, 75.0, Some(old)),
            five(old - 590, 75.0, Some(old - 1)),
            five(old - 470, 75.0, Some(old)),
            five(old - 364, 75.0, Some(old - 1)),
            five(old + 90, 0.0, None),
            five(old + 330, 1.0, Some(new)),
            five(old + 1_395, 1.0, Some(new - 1)),
            five(old + 2_475, 6.0, Some(new)),
        ];
        let now = old + 2_500;
        let view = view("claude", &records, DAY0, now, now, &utc()).unwrap();
        assert_eq!(view.resets, vec![ResetMarker { t: old, kind: WindowKind::FiveHour }]);
        let windows: Vec<_> = view.windows.iter().map(|w| (w.start, w.resets_at, w.peak, w.complete)).collect();
        assert_eq!(windows, vec![(new - 5 * H, new, 6.0, false), (old - 5 * H, old, 75.0, true)]);
        assert_eq!(view.days[0].five_hour, 6.0);
    }

    #[test]
    fn an_unused_codex_window_does_not_slide_its_window_or_reset() {
        // Codex reports an unused window's reset a full window from each read.
        let start = DAY0 + 2 * H;
        let mut records: Vec<Record> = (0..120).map(|i| five(DAY0 + i * 60, 0.0, Some(DAY0 + i * 60 + 5 * H - 30))).collect();
        records.push(five(start, 3.0, Some(start + 5 * H)));
        records.push(five(start + 600, 9.0, Some(start + 5 * H)));
        let view = view("codex", &records, DAY0, start + H, start + H, &utc()).unwrap();
        assert!(view.resets.is_empty(), "{:?}", view.resets);
        assert_eq!(view.windows.len(), 1);
        assert_eq!((view.windows[0].start, view.windows[0].resets_at, view.windows[0].peak), (start, start + 5 * H, 9.0));
    }

    #[test]
    fn a_reading_that_only_loses_its_reset_time_is_not_a_reset() {
        let reset = DAY0 + 5 * H;
        let records = vec![five(DAY0, 30.0, Some(reset)), five(DAY0 + 600, 35.0, None), five(DAY0 + 1_200, 40.0, Some(reset))];
        let view = view("antigravity", &records, DAY0, DAY0 + H, DAY0 + H, &utc()).unwrap();
        assert!(view.resets.is_empty());
        assert_eq!(view.windows.len(), 1);
        assert_eq!(view.days[0].five_hour, 10.0);
    }

    #[test]
    fn an_early_reset_is_marked_when_it_was_seen() {
        // Antigravity reset 16 s before the time it had reported.
        let reset = DAY0 + 5 * H;
        let records = vec![five(reset - 600, 48.0, Some(reset)), five(reset - 16, 0.0, None), five(reset + 90, 1.0, Some(reset + 5 * H))];
        let view = view("antigravity", &records, DAY0, reset + H, reset + H, &utc()).unwrap();
        assert_eq!(view.resets, vec![ResetMarker { t: reset - 16, kind: WindowKind::FiveHour }]);
    }

    #[test]
    fn daily_consumption_counts_increases_and_new_windows_but_not_resets() {
        let reset1 = DAY0 + 5 * H;
        let reset2 = DAY0 + 11 * H;
        let week = DAY0 + 6 * 24 * H;
        let records = vec![
            rec(DAY0 - 60, Some((10.0, reset1)), Some((50.0, week))),
            // 10 → 40 on day 0, and the weekly 50 → 55
            rec(DAY0 + H, Some((40.0, reset1)), Some((55.0, week))),
            // reset: the new window's 20% counts, the drop does not
            rec(DAY0 + 6 * H, Some((20.0, reset2)), Some((60.0, week))),
            // the next day: 20 → 25
            rec(DAY0 + 24 * H + 60, Some((25.0, reset2)), Some((61.0, week))),
        ];
        let view = view("codex", &records, DAY0, DAY0 + 25 * H, DAY0 + 25 * H, &utc()).unwrap();
        assert_eq!(view.days.len(), 2);
        assert_eq!(view.days[0].day, "2026-11-15");
        assert_eq!(view.days[0].start, DAY0);
        assert_eq!(view.days[0].five_hour, 50.0);
        assert_eq!(view.days[0].weekly, 10.0);
        assert_eq!(view.days[1].five_hour, 5.0);
        assert_eq!(view.days[1].weekly, 1.0);
        // Both later steps span hours without a line: gaps, so approximate.
        assert!(view.days[0].approximate && view.days[1].approximate);
        assert_eq!(view.gaps, vec![Gap { from: DAY0 + H, to: DAY0 + 6 * H }, Gap { from: DAY0 + 6 * H, to: DAY0 + 24 * H + 60 }]);
    }

    #[test]
    fn days_follow_the_local_time_zone() {
        // UTC−5: 2026-11-15T02:00Z is still the 14th locally.
        let tz = FixedOffset::west_opt(5 * H as i32).unwrap();
        let reset = DAY0 + 5 * H;
        let records = vec![rec(DAY0 - H, Some((0.0, reset)), None), rec(DAY0 + 2 * H, Some((30.0, reset)), None)];
        let view = view("codex", &records, DAY0 - 6 * H, DAY0 + 6 * H, DAY0 + 6 * H, &tz).unwrap();
        assert_eq!(view.days.iter().map(|d| d.day.as_str()).collect::<Vec<_>>(), vec!["2026-11-14", "2026-11-15"]);
        assert_eq!(view.days[0].five_hour, 30.0);
        assert_eq!(view.days[1].start, DAY0 + 5 * H);
    }

    #[test]
    fn an_account_switch_breaks_the_line_and_counts_nothing() {
        let reset = DAY0 + 5 * H;
        let mut other = rec(DAY0 + 600, Some((70.0, reset)), None);
        other.acct = Some("b".into());
        let records = vec![rec(DAY0, Some((10.0, reset)), None), other];
        let view = view("claude", &records, DAY0, DAY0 + H, DAY0 + H, &utc()).unwrap();
        assert_eq!(view.points.iter().map(|p| p.seg).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(view.days[0].five_hour, 0.0);
        assert!(view.resets.is_empty());
        assert_eq!(view.windows.len(), 2);
    }

    #[test]
    fn a_silent_stretch_up_to_now_is_a_trailing_gap() {
        let records = vec![rec(DAY0, Some((10.0, DAY0 + 5 * H)), None)];
        let view = view("codex", &records, DAY0 - H, DAY0 + 3 * H, DAY0 + 3 * H, &utc()).unwrap();
        assert_eq!(view.gaps, vec![Gap { from: DAY0, to: DAY0 + 3 * H }]);
        assert_eq!(view.recording_since, Some(DAY0));
    }

    #[test]
    fn downsampling_keeps_extremes_and_reset_edges() {
        let mut records = Vec::new();
        let reset1 = DAY0 + 5 * H;
        let reset2 = DAY0 + 10 * H;
        for i in 0..3_000 {
            let t = DAY0 + i * 10;
            let (u, r) = if t < DAY0 + 5 * H { ((i % 100) as f64, reset1) } else { (((i % 7) as f64) / 10.0, reset2) };
            records.push(rec(t, Some((u, r)), None));
        }
        let view = view("codex", &records, DAY0, DAY0 + 9 * H, DAY0 + 9 * H, &utc()).unwrap();
        assert!(view.points.len() <= MAX_POINTS, "{}", view.points.len());
        assert!(view.points.iter().any(|p| p.five_hour == Some(99.0)));
        assert!(view.points.iter().any(|p| p.five_hour == Some(0.0)));
        // Both sides of the reset survive.
        let edge = records.iter().position(|r| r.t >= DAY0 + 5 * H).unwrap();
        for i in [edge - 1, edge] {
            assert!(view.points.iter().any(|p| p.t == records[i].t), "point {i}");
        }
    }

    #[test]
    fn windows_outside_the_range_are_left_out() {
        let old_reset = DAY0 - 20 * H;
        let records = vec![rec(DAY0 - 22 * H, Some((50.0, old_reset)), None), rec(DAY0 + H, Some((5.0, DAY0 + 5 * H)), None)];
        let view = view("codex", &records, DAY0, DAY0 + 2 * H, DAY0 + 2 * H, &utc()).unwrap();
        assert_eq!(view.windows.len(), 1);
        assert_eq!(view.windows[0].resets_at, DAY0 + 5 * H);
        assert!(super::view("not-a-series", &records, DAY0, DAY0 + H, DAY0 + H, &utc()).is_none());
    }
}
