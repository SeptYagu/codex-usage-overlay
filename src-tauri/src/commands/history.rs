use std::sync::Arc;

use tauri::{AppHandle, State};

use super::AppState;
use crate::codex::reader::unix_now;
use crate::history::{query, valid_series, RETENTION_SECS};

/// Shortest range the History window asks for.
const MIN_RANGE_SECS: i64 = 60 * 60;

// The history commands are async so their file reads run on the async
// runtime, never on the main thread that drives every window.

#[tauri::command]
pub async fn get_usage_history_series(state: State<'_, Arc<AppState>>) -> Result<Vec<query::SeriesInfo>, String> {
    let now = unix_now();
    Ok(state
        .history
        .series()
        .into_iter()
        .filter_map(|series| query::series_info(&series, &state.history.records(&series, now)))
        .filter(|info| info.records > 0)
        .collect())
}

#[tauri::command]
pub async fn get_usage_history(
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
pub async fn clear_usage_history(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.history.clear().map_err(|error| format!("Could not clear usage history: {error}"))
}

/// Async, so it never runs on the main thread; the window itself is built on
/// a worker thread by `tray::open_history_window`.
#[tauri::command]
pub async fn open_history_window(app: AppHandle) -> Result<(), String> {
    crate::tray::open_history_window(&app);
    Ok(())
}
