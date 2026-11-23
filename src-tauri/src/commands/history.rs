use std::sync::Arc;

use tauri::{AppHandle, State};

use super::AppState;
use crate::codex::reader::unix_now;
use crate::history::{query, valid_series, RETENTION_SECS};

/// Shortest range the History window asks for.
const MIN_RANGE_SECS: i64 = 60 * 60;

#[tauri::command]
pub fn get_usage_history_series(state: State<'_, Arc<AppState>>) -> Vec<query::SeriesInfo> {
    let now = unix_now();
    state
        .history
        .series()
        .into_iter()
        .filter_map(|series| query::series_info(&series, &state.history.records(&series, now)))
        .filter(|info| info.records > 0)
        .collect()
}

#[tauri::command]
pub fn get_usage_history(
    state: State<'_, Arc<AppState>>,
    series: String,
    range_seconds: i64,
) -> Result<query::HistoryView, String> {
    if !valid_series(&series) {
        return Err("Unknown usage history series".into());
    }
    let now = unix_now();
    let range = range_seconds.clamp(MIN_RANGE_SECS, RETENTION_SECS);
    let records = state.history.records(&series, now);
    query::view(&series, &records, now - range, now, now, &chrono::Local)
        .ok_or_else(|| "Unknown usage history series".into())
}

#[tauri::command]
pub fn clear_usage_history(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.history.clear().map_err(|error| format!("Could not clear usage history: {error}"))
}

#[tauri::command]
pub fn open_history_window(app: AppHandle) {
    crate::tray::open_history_window(&app);
}
