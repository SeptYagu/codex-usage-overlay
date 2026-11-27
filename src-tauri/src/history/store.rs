//! History files: `<series>-YYYY-MM.jsonl` in the runtime directory's
//! `history` folder, one JSON [`Record`] per line, months in UTC.
//!
//! Normal recording appends lines. Retention cleanup rewrites a boundary month
//! with a recoverable backup; the in-memory query cache stays at 90 days.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Datelike, Months, NaiveDate};
use serde::Serialize;

use super::{should_record, valid_series, Record, RETENTION_SECS};
use crate::config::HistoryRetention;

/// Files are pruned at most this often while the app runs.
const PRUNE_INTERVAL_SECS: i64 = 24 * 60 * 60;

pub struct Store {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    series: HashMap<String, Vec<Record>>,
    last_prune: i64,
    retention: HistoryRetention,
    cleanup_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub as_of: i64,
    pub total_bytes: u64,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
    pub delete_bytes_estimate: u64,
    pub delete_records: u64,
    pub delete_before: Option<i64>,
    pub cleanup_error: Option<String>,
}

impl HistoryRetention {
    pub fn cutoff(self, now: i64) -> Option<i64> {
        let months = match self {
            Self::ThreeMonths => 3,
            Self::SixMonths => 6,
            Self::OneYear => 12,
            Self::All => return None,
        };
        DateTime::from_timestamp(now, 0)?.checked_sub_months(Months::new(months)).map(|date| date.timestamp())
    }
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
        Self::with_retention(dir, HistoryRetention::All)
    }

    pub fn with_retention(dir: PathBuf, retention: HistoryRetention) -> Self {
        let store = Self { dir, inner: Mutex::new(Inner { retention, ..Inner::default() }) };
        store.recover_rewrites();
        store
    }

    fn recover_rewrites(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
            let Some(original_name) = name.strip_suffix(".jsonl.backup") else { continue };
            if parse_file_name(&format!("{original_name}.jsonl")).is_none() { continue; }
            let original = path.with_file_name(format!("{original_name}.jsonl"));
            if original.exists() {
                if let Err(error) = fs::remove_file(&path) { eprintln!("Could not remove history backup {}: {error}", path.display()); }
            } else if let Err(error) = fs::rename(&path, &original) {
                eprintln!("Could not restore history backup {}: {error}", path.display());
            }
        }
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

    fn load(&self, series: &str, now: i64, retention: HistoryRetention) -> Vec<Record> {
        let mut files: Vec<_> = self.files().into_iter().filter(|(s, _, _)| s == series).collect();
        files.sort_by_key(|(_, month, _)| *month);
        let cutoff = (now - RETENTION_SECS).max(retention.cutoff(now).unwrap_or(i64::MIN));
        let mut records: Vec<Record> = Vec::new();
        for (_, month, path) in files {
            if month_end(month) <= cutoff { continue; }
            let Ok(file) = fs::File::open(&path) else {
                eprintln!("Could not open history file {}", path.display());
                continue;
            };
            for line in BufReader::new(file).lines() {
                match line {
                    Ok(line) => {
                        if let Ok(record) = serde_json::from_str::<Record>(line.trim()) {
                            if record.t >= cutoff { records.push(record); }
                        }
                    }
                    Err(error) => { eprintln!("Could not read history file {}: {error}", path.display()); break; }
                }
            }
        }
        records.sort_by_key(|record| record.t);
        records.dedup_by_key(|record| record.t);
        records
    }

    fn series_mut<'a>(&self, inner: &'a mut Inner, series: &str, now: i64) -> &'a mut Vec<Record> {
        if !inner.series.contains_key(series) {
            let loaded = self.load(series, now, inner.retention);
            inner.series.insert(series.to_string(), loaded);
        }
        inner.series.get_mut(series).expect("series was just loaded")
    }

    fn prune_if_due(&self, inner: &mut Inner, now: i64) {
        if now - inner.last_prune < PRUNE_INTERVAL_SECS {
            return;
        }
        match self.prune(inner.retention, now) {
            Ok(()) => {
                inner.last_prune = now;
                inner.cleanup_error = None;
                let view_cutoff = (now - RETENTION_SECS).max(inner.retention.cutoff(now).unwrap_or(i64::MIN));
                for records in inner.series.values_mut() {
                    records.retain(|record| record.t >= view_cutoff);
                }
            }
            Err(error) => {
                eprintln!("Could not clean usage history: {error}");
                inner.cleanup_error = Some(error.to_string());
            }
        }
    }

    fn prune(&self, retention: HistoryRetention, now: i64) -> std::io::Result<()> {
        let Some(cutoff) = retention.cutoff(now) else { return Ok(()) };
        for (_, month, path) in self.files() {
            if month_end(month) <= cutoff {
                fs::remove_file(&path)?;
                continue;
            }
            if month.and_hms_opt(0, 0, 0).is_some_and(|start| start.and_utc().timestamp() < cutoff) {
                self.rewrite_boundary(&path, cutoff)?;
            }
        }
        Ok(())
    }

    fn rewrite_boundary(&self, path: &PathBuf, cutoff: i64) -> std::io::Result<()> {
        let temporary = path.with_extension("jsonl.rewrite");
        let backup = path.with_extension("jsonl.backup");
        if backup.exists() { return Err(std::io::Error::other("history backup requires recovery")) }
        let source = fs::File::open(path)?;
        let mut file = fs::File::create(&temporary)?;
        let mut changed = false;
        for line in BufReader::new(source).lines() {
            let line = line?;
            let record: Record = serde_json::from_str(line.trim()).map_err(std::io::Error::other)?;
            if record.t >= cutoff {
                file.write_all(line.as_bytes())?;
                file.write_all(b"\n")?;
            } else {
                changed = true;
            }
        }
        if !changed {
            drop(file);
            fs::remove_file(temporary)?;
            return Ok(());
        }
        file.sync_all()?;
        drop(file);
        fs::rename(path, &backup)?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::rename(&backup, path);
            return Err(error);
        }
        fs::remove_file(backup)?;
        Ok(())
    }

    pub fn set_retention(&self, retention: HistoryRetention, now: i64) -> std::io::Result<()> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        self.recover_rewrites();
        inner.retention = retention;
        inner.last_prune = 0;
        inner.series.clear();
        self.prune_if_due(&mut inner, now);
        inner.cleanup_error.as_ref().map_or(Ok(()), |error| Err(std::io::Error::other(error.clone())))
    }

    pub fn storage_info(&self, target: HistoryRetention, now: i64) -> std::io::Result<StorageInfo> {
        let cutoff = target.cutoff(now);
        let mut info = StorageInfo {
            as_of: now,
            total_bytes: 0, first_at: None, last_at: None, delete_bytes_estimate: 0,
            delete_records: 0, delete_before: cutoff,
            cleanup_error: self.inner.lock().unwrap_or_else(|p| p.into_inner()).cleanup_error.clone(),
        };
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(info),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
            if parse_file_name(name).is_none() { continue; }
            let file = fs::File::open(&path)?;
            info.total_bytes += file.metadata()?.len();
            for line in BufReader::new(file).lines() {
                let line = line?;
                let record: Record = serde_json::from_str(line.trim()).map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {error}", path.display()))
                })?;
                info.first_at = Some(info.first_at.map_or(record.t, |first| first.min(record.t)));
                info.last_at = Some(info.last_at.map_or(record.t, |last| last.max(record.t)));
                if cutoff.is_some_and(|cutoff| record.t < cutoff) {
                    info.delete_records += 1;
                    info.delete_bytes_estimate += line.len() as u64 + 1;
                }
            }
        }
        Ok(info)
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
        let cutoff = (now - RETENTION_SECS).max(inner.retention.cutoff(now).unwrap_or(i64::MIN));
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
        if self.dir.exists() {
            for entry in fs::read_dir(&self.dir)? {
                let entry = entry?;
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
                let history_name = name.strip_suffix(".backup")
                    .or_else(|| name.strip_suffix(".rewrite"))
                    .unwrap_or(name);
                if parse_file_name(history_name).is_none() { continue; }
                if let Err(error) = fs::remove_file(&path) {
                    first_error.get_or_insert(error);
                }
            }
        }
        if first_error.is_none() { inner.cleanup_error = None; }
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
        let later = Store::with_retention(dir.clone(), HistoryRetention::ThreeMonths);
        let records = later.records("claude", NOW);
        assert_eq!(records.iter().map(|r| r.t).collect::<Vec<_>>(), vec![october]);
        assert!(!dir.join("claude-2026-07.jsonl").exists());
        assert!(dir.join("claude-2026-10.jsonl").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn all_retention_preserves_old_files_while_queries_stay_at_ninety_days() {
        let dir = temp_dir("all-retention");
        let july = 1_783_382_400;
        let october = 1_791_331_200;
        let store = Store::new(dir.clone());
        store.append("codex", record(july, 1.0), july).unwrap();
        store.append("codex", record(october, 2.0), october).unwrap();
        let upgraded = Store::with_retention(dir.clone(), HistoryRetention::All);
        upgraded.set_retention(HistoryRetention::All, NOW).unwrap();
        assert!(dir.join("codex-2026-07.jsonl").exists());
        assert_eq!(upgraded.records("codex", NOW).iter().map(|r| r.t).collect::<Vec<_>>(), vec![october]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn shorter_retention_rewrites_only_the_expired_boundary_lines() {
        let dir = temp_dir("boundary-retention");
        let store = Store::new(dir.clone());
        let cutoff = HistoryRetention::ThreeMonths.cutoff(NOW).unwrap();
        let month = month_label(cutoff);
        let path = dir.join(format!("codex-{month}.jsonl"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, format!("{}\n{}\n",
            serde_json::to_string(&record(cutoff - 60, 1.0)).unwrap(),
            serde_json::to_string(&record(cutoff + 60, 2.0)).unwrap())).unwrap();
        let before = store.storage_info(HistoryRetention::ThreeMonths, NOW).unwrap();
        assert_eq!(before.delete_records, 1);
        store.set_retention(HistoryRetention::ThreeMonths, NOW).unwrap();
        let lines = fs::read_to_string(&path).unwrap();
        assert_eq!(lines.lines().count(), 1);
        assert!(lines.contains(&(cutoff + 60).to_string()));
        assert!(!path.with_extension("jsonl.backup").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn calendar_month_cutoffs_clamp_at_the_end_of_short_months() {
        let date = chrono::NaiveDate::from_ymd_opt(2027, 5, 31).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        let cutoff = HistoryRetention::ThreeMonths.cutoff(date).unwrap();
        let expected = chrono::NaiveDate::from_ymd_opt(2027, 2, 28).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        assert_eq!(cutoff, expected);
    }

    #[test]
    fn a_broken_boundary_file_is_kept_and_reports_pending_cleanup() {
        let dir = temp_dir("broken-boundary");
        fs::create_dir_all(&dir).unwrap();
        let cutoff = HistoryRetention::ThreeMonths.cutoff(NOW).unwrap();
        let path = dir.join(format!("codex-{}.jsonl", month_label(cutoff)));
        fs::write(&path, format!("{}\n{{broken\n", serde_json::to_string(&record(cutoff - 60, 1.0)).unwrap())).unwrap();
        let store = Store::new(dir.clone());
        assert!(store.set_retention(HistoryRetention::ThreeMonths, NOW).is_err());
        assert!(fs::read_to_string(&path).unwrap().contains("{broken"));
        assert_eq!(store.storage_info(HistoryRetention::All, NOW).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_backup_from_an_interrupted_rewrite_is_restored() {
        let dir = temp_dir("restore-boundary");
        fs::create_dir_all(&dir).unwrap();
        let backup = dir.join("codex-2026-11.jsonl.backup");
        fs::write(&backup, format!("{}\n", serde_json::to_string(&record(NOW, 1.0)).unwrap())).unwrap();
        let store = Store::new(dir.clone());
        assert!(!backup.exists());
        assert_eq!(store.records("codex", NOW).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_removes_files_and_cached_records() {
        let dir = temp_dir("clear");
        let store = Store::new(dir.clone());
        store.append("antigravity", record(NOW, 1.0), NOW).unwrap();
        fs::write(dir.join("codex-2026-10.jsonl.backup"), "pending backup").unwrap();
        fs::write(dir.join("codex-2026-10.jsonl.rewrite"), "pending rewrite").unwrap();
        fs::write(dir.join("unrelated.txt"), "keep").unwrap();
        store.clear().unwrap();
        assert!(store.records("antigravity", NOW).is_empty());
        assert!(store.series().is_empty());
        assert!(!dir.join("codex-2026-10.jsonl.backup").exists());
        assert!(!dir.join("codex-2026-10.jsonl.rewrite").exists());
        assert!(dir.join("unrelated.txt").is_file());
        assert!(Store::new(dir.clone()).series().is_empty());
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
