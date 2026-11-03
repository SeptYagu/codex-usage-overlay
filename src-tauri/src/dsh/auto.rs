//! One native lifecycle for automatic, read-only DSH attachment.
//! Discovery/authentication never runs in a webview or while holding the source
//! commit mutex. A short epoch gate protects persistence and connection install.
use super::{bridge, ConnectionStatus, Manager, PreparedConnection};
use crate::{
    commands::{AppState, SettingsEnvelope},
    config::{OverlaySettings, Provider},
    tray::TrayPresentationSink,
};
use futures_util::{stream, StreamExt};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::Emitter;
use tokio::sync::{watch, Notify};

const RESCAN: Duration = Duration::from_secs(5);

struct Control {
    epoch: u64,
    suspended: bool,
    setting_up: bool,
    setup_attempted: bool,
    reload_deadline: Option<std::time::Instant>,
    phase: String,
    message: Option<String>,
    revision: u64,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            epoch: 0,
            suspended: false,
            setting_up: false,
            setup_attempted: false,
            reload_deadline: None,
            phase: "inactive".into(),
            message: None,
            revision: 0,
        }
    }
}

pub(crate) struct Controller {
    control: Mutex<Control>,
    changed: watch::Sender<u64>,
    wake: Notify,
    started: AtomicBool,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            control: Mutex::new(Control::default()),
            changed: watch::channel(0).0,
            wake: Notify::new(),
            started: AtomicBool::new(false),
        }
    }
}
impl Controller {
    fn advance(&self, control: &mut Control) {
        control.epoch = control.epoch.wrapping_add(1);
        self.changed.send_replace(control.epoch);
        self.wake.notify_one();
    }
    pub(crate) fn suspend(&self, manager: &Manager) -> u64 {
        let mut control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        self.advance(&mut control);
        control.suspended = true;
        control.setting_up = false;
        control.phase = "suspended".into();
        control.message = None;
        control.revision += 1;
        manager.revoke();
        control.epoch
    }
    pub(crate) fn with_operation<T>(
        &self,
        epoch: u64,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        if control.epoch != epoch {
            return Err("DSH operation was cancelled".into());
        }
        operation()
    }
    pub(crate) fn manual_phase(&self, epoch: u64, phase: &str) {
        let mut control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        if control.epoch == epoch {
            control.phase = phase.into();
            control.message = None;
            control.revision += 1;
        }
    }
    pub(crate) fn provider_selected(&self, manager: &Manager, provider: Provider) {
        let mut control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        self.advance(&mut control);
        control.suspended = false;
        control.setting_up = false;
        control.phase = if provider == Provider::Dsh {
            "discovering"
        } else {
            "inactive"
        }
        .into();
        control.message = None;
        control.revision += 1;
        manager.revoke();
    }
    pub(crate) fn retry(&self, manager: &Manager) {
        let mut control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        self.advance(&mut control);
        control.suspended = false;
        control.setting_up = false;
        control.setup_attempted = false;
        control.reload_deadline = None;
        control.phase = "discovering".into();
        control.message = None;
        control.revision += 1;
        manager.revoke();
    }
    pub(crate) fn source_changed(&self) {
        let mut control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        self.advance(&mut control);
    }
    fn ticket(&self) -> Option<u64> {
        let control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        (!control.suspended && !control.setting_up).then_some(control.epoch)
    }
    fn current(&self, epoch: u64) -> bool {
        let control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        control.epoch == epoch && !control.suspended && !control.setting_up
    }
    async fn cancelled(&self, epoch: u64) {
        let mut changed = self.changed.subscribe();
        loop {
            if *changed.borrow_and_update() != epoch {
                return;
            }
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
    fn phase(&self, epoch: u64, phase: &str, message: Option<&str>) {
        let mut control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        if control.epoch != epoch || control.suspended {
            return;
        }
        let message = message.map(str::to_owned);
        if control.phase != phase || control.message != message {
            control.phase = phase.into();
            control.message = message;
            control.revision += 1;
        }
    }
    pub(crate) fn status(&self, manager: &Manager) -> ConnectionStatus {
        let mut status = manager.cached_status();
        let control = self.control.lock().unwrap_or_else(|p| p.into_inner());
        status.auto_phase = control.phase.clone();
        status.auto_message = control.message.clone();
        status.auto_revision = control.revision;
        status
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    id: String,
    origin: String,
    profile_id: Option<String>,
    path: PathBuf,
}

async fn discover(directory: &Path) -> Result<Vec<Candidate>, String> {
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("Could not inspect DSH companion discovery".into()),
    };
    let mut candidates = Vec::new();
    for index in 0..=64 {
        let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|_| "Could not inspect DSH companion discovery")?
        else {
            break;
        };
        if index == 64 {
            return Err("DSH discovery exceeds the bounded candidate limit".into());
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if !entry
            .file_type()
            .await
            .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
        {
            continue;
        }
        let Ok(record) = bridge::read_record_in_directory(&path, directory).await else {
            continue;
        };
        candidates.push(Candidate {
            id: record.instance_id,
            origin: record.origin.origin().ascii_serialization(),
            profile_id: record.profile_id,
            path,
        });
    }
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(candidates)
}

enum Selection<'a> {
    Missing,
    One(&'a Candidate),
    Ambiguous,
}
fn select<'a>(candidates: &'a [Candidate], settings: &OverlaySettings) -> Selection<'a> {
    let matching: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            if !settings.dsh_profile_id.is_empty() {
                candidate.profile_id.as_deref() == Some(settings.dsh_profile_id.as_str())
            } else {
                candidate.id == settings.dsh_instance_id
                    && candidate.origin == settings.dsh_base_url
            }
        })
        .collect();
    if matching.len() == 1 {
        return Selection::One(matching[0]);
    }
    if matching.len() > 1 {
        return Selection::Ambiguous;
    }
    // Never jump from a known profile to a different profile because it is first.
    if !settings.dsh_profile_id.is_empty() {
        return Selection::Missing;
    }
    match candidates {
        [] => Selection::Missing,
        [one] => Selection::One(one),
        _ => Selection::Ambiguous,
    }
}

fn publish_status<S: TrayPresentationSink>(sink: &S, state: &AppState) {
    if let Some(app) = sink.app_handle() {
        let _ = app.emit("dsh_connection_updated", state.dsh_auto.status(&state.dsh));
    }
}

pub(crate) fn start(app: tauri::AppHandle, state: Arc<AppState>) {
    if state.dsh_auto.started.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            reconcile(&app, &state, true).await;
            tokio::select! { _ = tokio::time::sleep(RESCAN) => {}, _ = state.dsh_auto.wake.notified() => {} }
        }
    });
}

/// Testable production entry point. Tests disable only installation of the real
/// companion; the actual discovery, preparation, commit and source gates run.
pub(crate) async fn reconcile<S: TrayPresentationSink>(
    sink: &S,
    state: &Arc<AppState>,
    allow_setup: bool,
) {
    let settings = state.settings.lock().await.clone();
    if !crate::commands::dsh_monitor_enabled(state, &settings) {
        return;
    }
    let Some(epoch) = state.dsh_auto.ticket() else {
        return;
    };
    let generation = crate::monitor::selected(state, Provider::Dsh).source_generation.load(Ordering::Acquire);
    let directory = state.config_manager.runtime_dir().join("dsh-bridges");
    let mut candidates = match discover(&directory).await {
        Ok(candidates) => candidates,
        Err(_) => {
            state
                .dsh_auto
                .phase(epoch, "error", Some("DSH discovery is unavailable"));
            publish_status(sink, state);
            return;
        }
    };
    // Crash leftovers must not turn a unique live profile into permanent ambiguity.
    // Authentication is read-only and bounded; never delete another Host's record.
    if candidates.len() > 1 {
        let checks = stream::iter(candidates.into_iter().map(|candidate| {
            let directory = &directory;
            let manager = &state.dsh;
            async move {
                let healthy = tokio::time::timeout(
                    Duration::from_secs(8),
                    manager.prepare_bridge_in_directory(&candidate.path, directory),
                )
                .await;
                match healthy {
                    Ok(Ok(prepared)) if prepared.status().error.is_none() => Some(candidate),
                    _ => None,
                }
            }
        }))
        .buffer_unordered(4)
        .collect::<Vec<_>>();
        let checked = tokio::select! { biased;
            _ = state.dsh_auto.cancelled(epoch) => return,
            result = tokio::time::timeout(Duration::from_secs(20), checks) => result,
        };
        match checked {
            Ok(results) => candidates = results.into_iter().flatten().collect(),
            Err(_) => {
                state.dsh_auto.phase(
                    epoch,
                    "error",
                    Some("DSH discovery health checks timed out"),
                );
                publish_status(sink, state);
                return;
            }
        }
    }
    if !state.dsh_auto.current(epoch) {
        return;
    }
    let candidate = match select(&candidates, &settings) {
        Selection::One(candidate) => candidate.clone(),
        Selection::Ambiguous => {
            state.dsh_auto.phase(
                epoch,
                "ambiguous",
                Some("Multiple DSH profiles are available; select the intended instance"),
            );
            publish_status(sink, state);
            return;
        }
        Selection::Missing => {
            if allow_setup && candidates.is_empty() {
                setup(sink, state, false).await;
            } else {
                let control = state
                    .dsh_auto
                    .control
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                let keep = matches!(
                    control.phase.as_str(),
                    "restart-required" | "setting-up" | "error"
                );
                drop(control);
                if !keep {
                    state.dsh_auto.phase(
                        epoch,
                        "discovering",
                        Some("Waiting for a running DSH companion"),
                    );
                }
                publish_status(sink, state);
            }
            return;
        }
    };
    let cached = state.dsh.cached_status();
    let existing = cached.connected
        && cached.instance_id.as_deref() == Some(candidate.id.as_str())
        && cached.origin.as_deref() == Some(candidate.origin.as_str());
    let result = if existing {
        let status = tokio::select! { biased; _ = state.dsh_auto.cancelled(epoch) => return, status = state.dsh.status() => status };
        if !status.connected {
            Err("DSH companion disconnected".to_owned())
        } else {
            commit(sink, state, epoch, generation, &candidate, status, None).await
        }
    } else {
        state.dsh_auto.phase(epoch, "connecting", None);
        publish_status(sink, state);
        let prepared = tokio::select! {
            biased; _ = state.dsh_auto.cancelled(epoch) => return,
            prepared = state.dsh.prepare_bridge_in_directory(&candidate.path, &directory) => prepared,
        };
        match prepared {
            Ok(prepared) => {
                let status = prepared.status().clone();
                commit(
                    sink,
                    state,
                    epoch,
                    generation,
                    &candidate,
                    status,
                    Some(prepared),
                )
                .await
            }
            Err(error) => Err(error),
        }
    };
    if result.is_err() && state.dsh_auto.current(epoch) {
        state.dsh_auto.phase(
            epoch,
            "error",
            Some("Could not connect to the selected DSH companion"),
        );
    }
    publish_status(sink, state);
}

async fn commit<S: TrayPresentationSink>(
    sink: &S,
    state: &Arc<AppState>,
    epoch: u64,
    generation: u64,
    candidate: &Candidate,
    status: ConnectionStatus,
    prepared: Option<PreparedConnection>,
) -> Result<(), String> {
    let _commit = state.settings_patch.lock().await;
    let runtime = crate::monitor::selected(state, Provider::Dsh);
    let source_commit = if runtime.runtime_provider.is_some() { Some(runtime.settings_patch.lock().await) } else { None };
    let replacing_connection = prepared.is_some();
    let (next, source_changed, settings_changed) = {
        let mut settings = state.settings.lock().await;
        let mut installation = state.dsh.installation().await;
        // Nothing below this mutex guard awaits: cancel either precedes the
        // whole save/install or follows it, never slips between the two.
        let control = state
            .dsh_auto
            .control
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if control.epoch != epoch
            || control.suspended
            || control.setting_up
            || !crate::commands::dsh_monitor_enabled(state, &settings)
            || !runtime.is_current()
            || runtime.source_generation.load(Ordering::Acquire) != generation
        {
            return Err("DSH automatic connection was cancelled".into());
        }
        if let Some(prepared) = &prepared {
            if !installation.accepts(prepared) {
                return Err("DSH preparation was superseded".into());
            }
        } else {
            let current = state.dsh.cached_status();
            if !current.connected
                || current.connection_revision != status.connection_revision
                || current.instance_id != status.instance_id
                || current.origin != status.origin
            {
                return Err("DSH connection changed during discovery".into());
            }
        }
        let mut next = settings.clone();
        next.dsh_base_url = candidate.origin.clone();
        next.dsh_instance_id = candidate.id.clone();
        next.dsh_profile_id = candidate.profile_id.clone().unwrap_or_default();
        next.dsh_account_id = status.active_account_id.clone().unwrap_or_default();
        // A new automatic attachment/account is data-only. Never inherit target
        // authorization from a previous connection, profile or active account.
        if prepared.is_some() || next.dsh_account_id != settings.dsh_account_id {
            next.dsh_session_ids.clear();
        }
        let source_changed = !crate::config::same_source(&settings.for_provider(Provider::Dsh), &next.for_provider(Provider::Dsh));
        let settings_changed = source_changed || next.dsh_profile_id != settings.dsh_profile_id;
        if settings_changed {
            state.config_manager.save_settings_checked(&next)?;
        }
        if prepared.is_some() || source_changed { crate::commands::invalidate_dsh_runtime(&runtime); }
        if let Some(prepared) = prepared {
            installation.install(prepared);
        }
        if settings_changed {
            *settings = next.clone();
        }
        (next, source_changed, settings_changed)
    };
    drop(source_commit);
    if source_changed || replacing_connection {
        crate::commands::synchronize_dsh_runtime(state, &next).await;
    }
    if settings_changed {
        let revision = state.settings_revision.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(app) = sink.app_handle() {
            let _ = app.emit(
                "settings_updated",
                SettingsEnvelope::new(state, revision, next.clone()),
            );
        }
    }
    if source_changed {
        state
            .polling
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .request_now(std::time::Instant::now());
        state.poll_wake.notify_one();
        crate::tray::repaint_tray(sink, state).await;
    }
    state.dsh_auto.phase(
        epoch,
        if next.dsh_account_id.is_empty() {
            "connected-unbound"
        } else {
            "ready"
        },
        None,
    );
    Ok(())
}

/// Both the explicit command and the one-per-process background bootstrap use
/// this gate. Re-scanning alone never repeatedly invokes the official CLI.
pub(crate) async fn setup<S: TrayPresentationSink>(
    sink: &S,
    state: &Arc<AppState>,
    explicit: bool,
) {
    let settings = state.settings.lock().await.clone();
    if !crate::commands::dsh_monitor_enabled(state, &settings) { return; }
    if explicit {
        let _commit = state.settings_patch.lock().await;
        let settings = state.settings.lock().await.clone();
        if !crate::commands::dsh_monitor_enabled(state, &settings) { return; }
        let runtime = crate::monitor::selected(state, Provider::Dsh);
        let frozen = crate::commands::freeze_dsh_connection(&runtime).await;
        state.dsh_auto.retry(&state.dsh);
        drop(frozen);
        crate::commands::synchronize_dsh_runtime(state, &settings).await;
    }
    let Some(before) = state.dsh_auto.ticket() else {
        return;
    };
    {
        let mut control = state
            .dsh_auto
            .control
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if control.setup_attempted {
            if control
                .reload_deadline
                .is_some_and(|deadline| deadline <= std::time::Instant::now())
            {
                control.phase = "restart-required".into();
                control.message =
                    Some("DSH companion installed; restart DSH safely to load its plugins".into());
                control.reload_deadline = None;
                control.revision += 1;
            }
            drop(control);
            publish_status(sink, state);
            return;
        }
    }
    // DSH may start after the overlay. Absence does not consume the one real
    // installation attempt or permanently disable discovery for this process.
    let available = tokio::select! { biased;
        _ = state.dsh_auto.cancelled(before) => return,
        result = super::setup::desktop_available() => result,
    };
    match available {
        Ok(true) => {}
        Ok(false) => {
            state.dsh_auto.phase(
                before,
                "discovering",
                Some("Waiting for DSH Desktop to start"),
            );
            publish_status(sink, state);
            return;
        }
        Err(_) => {
            state.dsh_auto.phase(
                before,
                "error",
                Some("Could not identify a supported running DSH Desktop"),
            );
            publish_status(sink, state);
            return;
        }
    }
    let epoch = {
        let mut control = state
            .dsh_auto
            .control
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if control.epoch != before
            || control.setting_up
            || control.suspended
            || control.setup_attempted
        {
            return;
        }
        state.dsh_auto.advance(&mut control);
        control.setting_up = true;
        control.setup_attempted = true;
        control.phase = "setting-up".into();
        control.message = None;
        control.revision += 1;
        control.epoch
    };
    publish_status(sink, state);
    let result = tokio::select! { biased;
        _ = state.dsh_auto.cancelled(epoch) => return,
        result = super::setup::install_bridge() => result,
    };
    {
        let mut control = state
            .dsh_auto
            .control
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if control.epoch != epoch || control.suspended {
            return;
        }
        control.setting_up = false;
        match result {
            Ok(_) => {
                control.phase = "discovering".into();
                control.message =
                    Some("Waiting for the installed DSH companion to become healthy".into());
                control.reload_deadline = Some(std::time::Instant::now() + Duration::from_secs(30));
            }
            Err(_) => {
                control.phase = "error".into();
                control.message = Some(
                    "Automatic DSH companion setup did not complete; retry to try again".into(),
                );
            }
        }
        control.revision += 1;
    }
    state.dsh_auto.wake.notify_one();
    publish_status(sink, state);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(id: &str, profile: Option<&str>) -> Candidate {
        Candidate {
            id: id.into(),
            origin: "http://127.0.0.1:1234".into(),
            profile_id: profile.map(str::to_owned),
            path: PathBuf::new(),
        }
    }
    #[test]
    fn candidates_never_choose_an_arbitrary_or_different_known_profile() {
        let mut settings = OverlaySettings::default();
        let candidates = [
            candidate("one", Some("profile-a")),
            candidate("two", Some("profile-b")),
        ];
        assert!(matches!(
            select(&candidates, &settings),
            Selection::Ambiguous
        ));
        settings.dsh_profile_id = "profile-b".into();
        assert!(
            matches!(select(&candidates, &settings), Selection::One(candidate) if candidate.id == "two")
        );
        settings.dsh_profile_id = "missing".into();
        assert!(matches!(
            select(&candidates[..1], &settings),
            Selection::Missing
        ));
    }
    #[tokio::test]
    async fn suspend_cancels_old_tickets_and_only_explicit_retry_or_reselection_rearms() {
        let controller = Controller::default();
        let manager = Manager::default();
        let old = controller.ticket().unwrap();
        controller.suspend(&manager);
        controller.cancelled(old).await;
        assert!(!controller.current(old));
        assert!(controller.ticket().is_none());
        assert_eq!(controller.status(&manager).auto_phase, "suspended");
        controller.retry(&manager);
        assert!(controller.ticket().is_some());
        controller.suspend(&manager);
        controller.provider_selected(&manager, Provider::Dsh);
        assert!(controller.ticket().is_some());
    }
}
