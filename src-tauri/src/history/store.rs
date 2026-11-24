//! History files: `<series>-YYYY-MM.jsonl` in the runtime directory's
//! `history` folder, one JSON [`Record`] per line, months in UTC.
//!
//! Lines are only ever appended. A crash mid-write leaves at most one broken
//! last line, which readers skip. Each series is parsed once and then kept in
//! memory, so queries never touch the disk again.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Datelike, NaiveDate};

use super::{should_record, valid_series, Record, RETENTION_SECS};

/// Files are pruned at most this often while the app runs.
const PRUNE_INTERVAL_SECS: i64 = 24 * 60 * 60;
/// A history file this large is not something the recorder wrote; skip it.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub struct Store {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    series: HashMap<String, Vec<Record>>,
    last_prune: i64,
}

/// A history file's series key and month, from its name.
fn parse_file_name(name: &str) -> Option<(&str, NaiveDate)> {
    let stem = name.strip_suffix(".jsonl")?;
    if stem.len() < 9 || !stem.is_char_boundary(stem.len() - 8) {
        return None;
    }
    let (series, month) = stem.split_at(stem.len() - 8);
    let month = month.strip_prefix('-')?;
    let (year, month) = month.split_once('-')?;
    if year.len() != 4 || month.len() != 2 {
        return None;
    }
    let first = NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, 1)?;
    valid_series(series).then_some((series, first))
}

fn month_label(t: i64) -> String {
    DateTime::from_timestamp(t, 0).unwrap_or_default().format("%Y-%m").to_string()
}

/// The first second after the month that starts on `first`.
fn month_end(first: NaiveDate) -> i64 {
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    next.and_then(|d| d.and_hms_opt(0, 0, 0)).map_or(i64::MAX, |d| d.and_utc().timestamp())
}

fn ends_mid_line(file: &mut fs::File) -> std::io::Result<bool> {
    if file.metadata()?.len() == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, inner: Mutex::new(Inner::default()) }
    }

    fn files(&self) -> Vec<(String, NaiveDate, PathBuf)> {
        let Ok(entries) = fs::read_dir(&self.dir) else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let (series, month) = parse_file_name(&name)?;
                Some((series.to_string(), month, entry.path()))
            })
            .collect()
    }

    fn load(&self, series: &str, now: i64) -> Vec<Record> {
        let mut files: Vec<_> = self.files().into_iter().filter(|(s, _, _)| s == series).collect();
        files.sort_by_key(|(_, month, _)| *month);
        let cutoff = now - RETENTION_SECS;
        let mut records: Vec<Record> = Vec::new();
        for (_, _, path) in files {
            if fs::metadata(&path).map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            records.extend(
                text.lines()
                    .filter_map(|line| serde_json::from_str::<Record>(line.trim()).ok())
                    .filter(|record| record.t >= cutoff),
            );
        }
        records.sort_by_key(|record| record.t);
        records.dedup_by_key(|record| record.t);
        records
    }

    fn series_mut<'a>(&self, inner: &'a mut Inner, series: &str, now: i64) -> &'a mut Vec<Record> {
        if !inner.series.contains_key(series) {
            let loaded = self.load(series, now);
            inner.series.insert(series.to_string(), loaded);
        }
        inner.series.get_mut(series).expect("series was just loaded")
    }

    fn prune_if_due(&self, inner: &mut Inner, now: i64) {
        if now - inner.last_prune < PRUNE_INTERVAL_SECS {
            return;
        }
        inner.last_prune = now;
        let cutoff = now - RETENTION_SECS;
        for (_, month, path) in self.files() {
            if month_end(month) <= cutoff {
                let _ = fs::remove_file(path);
            }
        }
        for records in inner.series.values_mut() {
            records.retain(|record| record.t >= cutoff);
        }
    }

    /// Appends `record` to `series` when it is new information. Returns whether
    /// a line was written.
    pub fn append(&self, series: &str, record: Record, now: i64) -> std::io::Result<bool> {
        if !valid_series(series) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid history series"));
        }
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        self.prune_if_due(&mut inner, now);
        let records = self.series_mut(&mut inner, series, now);
        if !should_record(records.last(), &record) {
            return Ok(false);
        }
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{series}-{}.jsonl", month_label(record.t)));
        let mut line = serde_json::to_string(&record).map_err(std::io::Error::other)?;
        line.push('\n');
        let mut file = OpenOptions::new().create(true).read(true).append(true).open(path)?;
        if ends_mid_line(&mut file)? {
            // A crash cut the last line short; start this one on its own line.
            line.insert(0, '\n');
        }
        file.write_all(line.as_bytes())?;
        records.push(record);
        Ok(true)
    }

    /// The series' records from the last 90 days, oldest first.
    pub fn records(&self, series: &str, now: i64) -> Vec<Record> {
        if !valid_series(series) {
            return Vec::new();
        }
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        self.prune_if_due(&mut inner, now);
        let cutoff = now - RETENTION_SECS;
        self.series_mut(&mut inner, series, now)
            .iter()
            .filter(|record| record.t >= cutoff)
            .cloned()
            .collect()
    }

    /// Every series that has history files or was recorded this session.
    pub fn series(&self) -> Vec<String> {
        let mut names: BTreeSet<String> = self.files().into_iter().map(|(series, _, _)| series).collect();
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        names.extend(inner.series.iter().filter(|(_, r)| !r.is_empty()).map(|(s, _)| s.clone()));
        names.into_iter().collect()
    }

    /// Deletes every history file and forgets the cached records.
    pub fn clear(&self) -> std::io::Result<()> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.series.clear();
        let mut first_error = None;
        for (_, _, path) in self.files() {
            if let Err(error) = fs::remove_file(&path) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{WindowReading, HEARTBEAT_SECS};

    // 2026-11-15T00:00:00Z
    const NOW: i64 = 1_794_700_800;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cuo-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn record(t: i64, five: f64) -> Record {
        Record {
            t,
            acct: Some("a".into()),
            pool: None,
            fh: Some(WindowReading { u: five, r: Some(NOW + 3_600) }),
            wk: None,
            ar: None,
        }
    }

    #[test]
    fn file_names_parse_only_history_files() {
        let (series, month) = parse_file_name("dsh-burn-0123456789abcdef-2026-11.jsonl").unwrap();
        assert_eq!(series, "dsh-burn-0123456789abcdef");
        assert_eq!(month, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        assert_eq!(parse_file_name("codex-2026-01.jsonl").unwrap().0, "codex");
        for bad in ["codex-2026-13.jsonl", "codex-2026-1.jsonl", "codex.jsonl", "notes-2026-11.jsonl", "codex-2026-11.json", "-2026-11.jsonl"] {
            assert!(parse_file_name(bad).is_none(), "{bad}");
        }
        assert_eq!(month_end(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()), 1_798_761_600);
    }

    #[test]
    fn appends_survive_a_reload_and_skip_a_broken_last_line() {
        let dir = temp_dir("reload");
        let store = Store::new(dir.clone());
        assert!(store.append("codex", record(NOW, 1.0), NOW).unwrap());
        assert!(!store.append("codex", record(NOW + 60, 1.0), NOW + 60).unwrap(), "unchanged reading");
        assert!(store.append("codex", record(NOW + 120, 2.0), NOW + 120).unwrap());
        assert!(store.append("codex", record(NOW + 120 + HEARTBEAT_SECS, 2.0), NOW + 120 + HEARTBEAT_SECS).unwrap());
        let path = dir.join("codex-2026-11.jsonl");
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"t\":17950").unwrap();

        let reloaded = Store::new(dir.clone());
        let records = reloaded.records("codex", NOW + 7_200);
        assert_eq!(records.iter().map(|r| r.t).collect::<Vec<_>>(), vec![NOW, NOW + 120, NOW + 120 + HEARTBEAT_SECS]);
        assert_eq!(reloaded.series(), vec!["codex".to_string()]);
        // The next line after a cut-off one is still readable.
        let next = NOW + 120 + 2 * HEARTBEAT_SECS;
        assert!(reloaded.append("codex", record(next, 3.0), next).unwrap());
        assert_eq!(Store::new(dir.clone()).records("codex", next).last().map(|r| r.t), Some(next));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn records_roll_into_monthly_files_and_old_months_are_pruned() {
        let dir = temp_dir("prune");
        let store = Store::new(dir.clone());
        let july = 1_783_382_400; // 2026-07-07
        let october = 1_791_331_200; // 2026-10-07
        store.append("claude", record(july, 1.0), july).unwrap();
        store.append("claude", record(october, 2.0), october).unwrap();
        assert!(dir.join("claude-2026-07.jsonl").is_file());
        assert!(dir.join("claude-2026-10.jsonl").is_file());

        // 2026-11-15 is more than 90 days after July ended, but not October.
        let later = Store::new(dir.clone());
        let records = later.records("claude", NOW);
        assert_eq!(records.iter().map(|r| r.t).collect::<Vec<_>>(), vec![october]);
        assert!(!dir.join("claude-2026-07.jsonl").exists());
        assert!(dir.join("claude-2026-10.jsonl").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_removes_files_and_cached_records() {
        let dir = temp_dir("clear");
        let store = Store::new(dir.clone());
        store.append("antigravity", record(NOW, 1.0), NOW).unwrap();
        fs::write(dir.join("unrelated.txt"), "keep").unwrap();
        store.clear().unwrap();
        assert!(store.records("antigravity", NOW).is_empty());
        assert!(store.series().is_empty());
        assert!(dir.join("unrelated.txt").is_file());
        // Recording starts again from scratch after a clear.
        assert!(store.append("antigravity", record(NOW + 60, 1.0), NOW + 60).unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_invalid_series_is_refused() {
        let store = Store::new(temp_dir("invalid"));
        assert!(store.append("../codex", record(NOW, 1.0), NOW).is_err());
        assert!(store.records("../codex", NOW).is_empty());
    }
}
