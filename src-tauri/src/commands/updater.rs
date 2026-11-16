use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

use super::{AppState, AvailableUpdate, PreparedUpdate, UpdateProgress};

#[tauri::command]
pub async fn check_for_updates(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<AvailableUpdate>, String> {
    let found = check_and_store_update(&app, &state).await?;
    if found.is_some() && state.settings.lock().await.auto_install_updates {
        let app_for_prepare = app.clone();
        let state_for_prepare = state.inner().clone();
        tauri::async_runtime::spawn(async move {
            let _ = prepare_update(&app_for_prepare, &state_for_prepare).await;
        });
    }
    Ok(found)
}

#[tauri::command]
pub async fn get_available_update(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<AvailableUpdate>, String> {
    Ok(state.available_update.lock().await.clone())
}

#[tauri::command]
pub async fn get_update_error(state: State<'_, Arc<AppState>>) -> Result<Option<String>, String> {
    Ok(state.update_error.lock().await.clone())
}

#[tauri::command]
pub fn get_update_installing(state: State<'_, Arc<AppState>>) -> bool {
    state.update_installing.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn get_update_ready(state: State<'_, Arc<AppState>>) -> bool {
    state.prepared_update.lock().is_ok_and(|prepared| prepared.is_some())
}

pub(crate) async fn check_and_store_update(
    app: &AppHandle,
    state: &AppState,
) -> Result<Option<AvailableUpdate>, String> {
    use tauri_plugin_updater::UpdaterExt;

    let _check_guard = state.update_check.lock().await;
    let include_prereleases = state.settings.lock().await.include_prerelease_updates;
    let mut builder = app.updater_builder().target(crate::config::updater_target());
    if include_prereleases {
        let endpoint = crate::update_channel::prerelease_endpoint().await?;
        builder = builder.endpoints(vec![endpoint]).map_err(|error| error.to_string())?;
    }
    let updater = builder.build().map_err(|error| error.to_string())?;
    let update = updater.check().await.map_err(|error| error.to_string())?;
    let info = update.as_ref().map(|update| AvailableUpdate {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: update.body.clone(),
    });
    *state.pending_update.lock().await = update;
    *state.available_update.lock().await = info.clone();
    *state.update_error.lock().await = None;
    if let Ok(mut prepared) = state.prepared_update.lock() {
        if prepared.as_ref().map(|package| package.update.version.as_str())
            != info.as_ref().map(|available| available.version.as_str())
        {
            *prepared = None;
        }
    }
    Ok(info)
}

/// The regular automatic check, plus quick retries after a failed one (for
/// example a launch-time check made before the network is up after sign-in).
pub(crate) const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
const UPDATE_CHECK_RETRY_DELAYS: [Duration; 3] =
    [Duration::from_secs(30), Duration::from_secs(2 * 60), Duration::from_secs(10 * 60)];

/// `failures` counts consecutive failed checks, including this one.
pub(crate) fn next_update_check_delay(checked: bool, failures: usize) -> Duration {
    if checked || failures == 0 {
        return UPDATE_CHECK_INTERVAL;
    }
    UPDATE_CHECK_RETRY_DELAYS.get(failures - 1).copied().unwrap_or(UPDATE_CHECK_INTERVAL)
}

/// One notification per version and state: "available" and later "ready"
/// (downloaded, installs on quit) are announced once each.
pub(crate) fn claim_update_notice(last: &mut Option<String>, version: &str, ready: bool) -> bool {
    let key = format!("{}:{version}", if ready { "ready" } else { "available" });
    if last.as_deref() == Some(key.as_str()) {
        return false;
    }
    *last = Some(key);
    true
}

/// Returns whether the check itself succeeded (finding no update counts).
pub(crate) async fn check_and_notify_auto_update(app: &AppHandle, state: &AppState) -> bool {
    let update = match check_and_store_update(app, state).await {
        Ok(Some(update)) => update,
        Ok(None) => return true,
        Err(error) => {
            eprintln!("Automatic update check failed: {error}");
            return false;
        }
    };
    let auto_install = state.settings.lock().await.auto_install_updates;
    // Automatic installation downloads now and installs on quit; the
    // notification then says so instead of the download staying silent.
    let ready = auto_install
        && prepare_update(app, state).await.is_ok()
        && state.prepared_update.lock().is_ok_and(|prepared| {
            prepared.as_ref().is_some_and(|package| package.update.version == update.version)
        });
    let should_notify = claim_update_notice(&mut *state.last_auto_notified_version.lock().await, &update.version, ready);
    if should_notify {
        if !auto_install {
            let _ = app.emit("update_available", &update);
        }
        let language = state.settings.lock().await.language.clone();
        crate::notify::show_update_notification(app, &language, &update.version, ready);
    }
    true
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    attempt_install_update(&app, state.inner()).await
}

async fn attempt_install_update(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if state.update_installing.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Ok(());
    }
    *state.update_error.lock().await = None;
    let _ = app.emit("update_install_started", ());
    let result = install_pending_update(app, state).await;
    state.update_installing.store(false, Ordering::SeqCst);
    if let Err(error) = &result {
        *state.update_error.lock().await = Some(error.clone());
        if let Some(update) = state.available_update.lock().await.clone() {
            let _ = app.emit("update_install_failed", update);
        }
        eprintln!("Update installation failed: {error}");
    }
    result
}

async fn install_pending_update(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let _install_guard = state.update_install.lock().await;
    let prepared = state.prepared_update.lock().map_err(|error| error.to_string())?.take();
    if let Some(package) = prepared {
        let result = install_verified_bytes(app, &package.update, &package.bytes, false);
        if result.is_err() {
            *state.prepared_update.lock().map_err(|error| error.to_string())? = Some(package);
        }
        return result;
    }
    let update = state
        .pending_update
        .lock()
        .await
        .as_ref()
        .cloned()
        .ok_or_else(|| "No checked update is available".to_string())?;

    if crate::config::is_installed_environment() {
        let progress_app = app.clone();
        let mut downloaded = 0_u64;
        update
            .download_and_install(
                move |chunk, total| {
                    downloaded += chunk as u64;
                    let _ = progress_app.emit(
                        "update_progress",
                        UpdateProgress { downloaded, total },
                    );
                },
                || {},
            )
            .await
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let progress_app = app.clone();
    let mut downloaded = 0_u64;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit(
                    "update_progress",
                    UpdateProgress { downloaded, total },
                );
            },
            || {},
        )
        .await
        .map_err(|error| error.to_string())?;

    install_verified_bytes(app, &update, &bytes, false)
}

fn install_verified_bytes(
    app: &AppHandle,
    update: &tauri_plugin_updater::Update,
    bytes: &[u8],
    exiting: bool,
) -> Result<(), String> {
    if crate::config::is_installed_environment() {
        let update = update.clone().restart_after_install(!exiting);
        return update.install(bytes).map_err(|error| error.to_string());
    }

    use std::io::Cursor;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Invalid portable update archive: {error}"))?;
    let mut executable = archive
        .by_name("CodexUsageOverlay.exe")
        .map_err(|error| format!("Portable update is missing the application: {error}"))?;
    if executable.size() > 256 * 1024 * 1024 {
        return Err("Portable update executable is too large".to_string());
    }

    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = current_exe
        .parent()
        .ok_or_else(|| "Could not locate the portable application directory".to_string())?;
    let staged_exe = directory.join("CodexUsageOverlay.exe.staged");
    let helper = directory.join("CodexUsageUpdater.exe");
    let mut staged_file = std::fs::File::create(&staged_exe).map_err(|error| error.to_string())?;
    std::io::copy(&mut executable, &mut staged_file).map_err(|error| error.to_string())?;
    drop(executable);
    drop(staged_file);

    let staged_helper = directory.join("CodexUsageUpdater.next.exe");
    let mut helper_archive = archive.by_name("CodexUsageUpdater.exe")
        .map_err(|error| format!("Portable update is missing the updater helper: {error}"))?;
    if helper_archive.size() > 64 * 1024 * 1024 {
        let _ = std::fs::remove_file(&staged_exe);
        return Err("Portable update helper is too large".to_string());
    }
    let mut helper_file = std::fs::File::create(&staged_helper).map_err(|error| error.to_string())?;
    std::io::copy(&mut helper_archive, &mut helper_file).map_err(|error| error.to_string())?;
    drop(helper_file);
    drop(helper_archive);
    // Archives before a bridge was added omit that helper; a busy bridge
    // keeps the current binary and the update continues.
    replace_optional_bridge(&mut archive, directory, crate::claude::install::BRIDGE_EXE);
    replace_optional_bridge(&mut archive, directory, crate::antigravity::install::BRIDGE_EXE);

    let mut command = std::process::Command::new(&staged_helper);
    command
        .arg(std::process::id().to_string())
        .arg(&current_exe)
        .arg(&staged_exe)
        .arg(&helper)
        .arg(if exiting { "--no-restart" } else { "--restart" })
        .args(std::env::args_os().skip(1))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command.spawn().map_err(|error| {
        let _ = std::fs::remove_file(&staged_exe);
        let _ = std::fs::remove_file(&staged_helper);
        error.to_string()
    })?;
    if !exiting {
        app.exit(0);
    }
    Ok(())
}

fn replace_optional_bridge<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    directory: &std::path::Path,
    bridge_exe: &str,
) {
    if let Ok(mut bridge_archive) = archive.by_name(bridge_exe) {
        if bridge_archive.size() <= 64 * 1024 * 1024 {
            let bridge = directory.join(bridge_exe);
            let staged_bridge = bridge.with_extension("exe.next");
            let copied = std::fs::File::create(&staged_bridge)
                .and_then(|mut file| std::io::copy(&mut bridge_archive, &mut file));
            if copied.is_err() || std::fs::rename(&staged_bridge, &bridge).is_err() {
                let _ = std::fs::remove_file(&staged_bridge);
            }
        }
    }
}

async fn prepare_update(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let _install_guard = state.update_install.lock().await;
    if !state.settings.lock().await.auto_install_updates {
        return Ok(());
    }
    let update = state.pending_update.lock().await.as_ref().cloned()
        .ok_or_else(|| "No checked update is available".to_string())?;
    if state.prepared_update.lock().map_err(|error| error.to_string())?
        .as_ref().is_some_and(|package| package.update.version == update.version)
    {
        return Ok(());
    }
    let bytes = match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => bytes,
        Err(error) => {
            *state.update_error.lock().await = Some(error.to_string());
            if let Some(info) = state.available_update.lock().await.clone() {
                let _ = app.emit("update_install_failed", info);
            }
            eprintln!("Automatic update download failed: {error}");
            return Err(error.to_string());
        }
    };
    if !state.settings.lock().await.auto_install_updates
        || state.pending_update.lock().await.as_ref().is_none_or(|pending| pending.version != update.version)
    {
        return Ok(());
    }
    *state.prepared_update.lock().map_err(|error| error.to_string())? = Some(PreparedUpdate { update, bytes });
    *state.update_error.lock().await = None;
    if let Some(info) = state.available_update.lock().await.clone() {
        let _ = app.emit("update_ready", info);
    }
    Ok(())
}

pub(crate) fn install_prepared_update_on_exit(app: &AppHandle, state: &AppState) {
    if let Ok(mut prepared) = state.prepared_update.lock() {
        if let Some(package) = prepared.take() {
            if let Err(error) = install_verified_bytes(app, &package.update, &package.bytes, true) {
                eprintln!("Could not install prepared update on exit: {error}");
            }
        }
    }
}

