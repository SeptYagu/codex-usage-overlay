//! Automatic pause control for a running Codex session.
//!
//! When a quota window runs low, the overlay can ask the Codex desktop client
//! to pause the conversation instead of leaving the session to burn the
//! remaining quota. The request is a *user message* driven collaboration
//! request: the overlay never terminates a Codex process, never writes a
//! transcript and never starts an app-server of its own to race the desktop
//! client for session writes.
//!
//! The auto-pause ledger and in-session paused-thread tracking live only in
//! memory and are never written to disk.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use crate::codex::CodexUsage;
use crate::commands::ProviderRuntime;
use crate::config::OverlaySettings;

pub mod status;
pub mod transport;

pub use status::AutoPauseStatus;
#[cfg(test)]
pub use transport::ToolCallOutcome;
pub use transport::{
    looks_like_thread_id, send_resume_to_thread, ActiveThread, BridgeCandidate, BridgeDiscovery,
    BridgeTransport,
};

/// The localized pause message literals.
pub const PAUSE_MESSAGE_ZH_CN: &str = "\u{6682}\u{505C}";
pub const PAUSE_MESSAGE_ZH_HANT: &str = "\u{66AB}\u{505C}";
pub const PAUSE_MESSAGE_EN: &str = "Pause";

pub fn pause_message_for_locale(locale: &str) -> &'static str {
    match locale {
        "zh-Hant" => PAUSE_MESSAGE_ZH_HANT,
        "zh-CN" => PAUSE_MESSAGE_ZH_CN,
        _ => PAUSE_MESSAGE_EN,
    }
}

pub fn pause_message_for_language(language: &str) -> &'static str {
    let locale = crate::commands::resolve_locale(language);
    pause_message_for_locale(locale)
}

/// The localized manual-resume message literals.
pub const RESUME_MESSAGE_ZH_CN: &str = "\u{7EE7}\u{7EED}";
pub const RESUME_MESSAGE_ZH_HANT: &str = "\u{7E7C}\u{7E8C}";
pub const RESUME_MESSAGE_EN: &str = "Continue";

pub fn resume_message_for_locale(locale: &str) -> &'static str {
    match locale {
        "zh-Hant" => RESUME_MESSAGE_ZH_HANT,
        "zh-CN" => RESUME_MESSAGE_ZH_CN,
        _ => RESUME_MESSAGE_EN,
    }
}

pub fn resume_message_for_language(language: &str) -> &'static str {
    let locale = crate::commands::resolve_locale(language);
    resume_message_for_locale(locale)
}

/// The deadline that covers one whole delivery batch — discovery, handshake and every send.
pub const DELIVERY_DEADLINE: Duration = Duration::from_secs(5);
/// The defensive cap on each ledger collection.
const MAX_COLLECTION_ENTRIES: usize = 512;

/// The quota window a pause request belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QuotaKind {
    FiveHour,
    Weekly,
}

/// A quota window that crossed its configured threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub quota: QuotaKind,
    /// The reset timestamp of the window, when the sample carried one.
    pub resets_at: Option<i64>,
}

/// Decides which quota windows crossed their threshold.
pub fn triggered_quotas(settings: &OverlaySettings, usage: &CodexUsage) -> Vec<Trigger> {
    let pref = settings.preference(settings.provider);
    if !(pref.monitor_enabled && pref.protection_enabled) {
        return Vec::new();
    }

    let mut triggers = Vec::new();
    if let Some(remaining) = usage.five_hour_remaining_percent {
        if remaining <= pref.five_hour_threshold_percent {
            triggers.push(Trigger {
                quota: QuotaKind::FiveHour,
                resets_at: usage.five_hour_resets_at,
            });
        }
    }
    if let Some(remaining) = usage.week_remaining_percent {
        if remaining <= pref.weekly_threshold_percent {
            triggers.push(Trigger {
                quota: QuotaKind::Weekly,
                resets_at: usage.week_resets_at,
            });
        }
    }
    triggers
}

/// The overlay owns resume scheduling; a cooperative message only asks to pause.
#[cfg(test)]
pub fn pause_message(language: &str, _triggers: &[Trigger], _observed_at: i64) -> String {
    pause_message_for_language(language).to_string()
}

/// The de-duplication key of one delivered request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeliveryKey {
    account: String,
    quota: QuotaKind,
    resets_at: Option<i64>,
    thread_id: String,
}

/// The active period bookkeeping of one target session.
#[derive(Debug, Clone)]
struct ThreadPeriod {
    period_id: String,
    host_id: Option<String>,
}

/// The snapshot a delivery worker consumes.
#[derive(Debug, Clone)]
pub struct PendingSample {
    pub settings: OverlaySettings,
    pub usage: CodexUsage,
    pub account_id: String,
    generation: u64,
    received_at: tokio::time::Instant,
}

/// The outcome of one bridge lookup, as the settings window shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BridgeConnection {
    Unknown,
    Available,
    Unavailable,
}

type WorkerSpawner = Arc<dyn Fn(Arc<ProviderRuntime>) + Send + Sync>;

/// The short-lived in-memory state of the auto-pause feature.
pub struct AutoPauseController {
    telemetry: status::Telemetry,
    delivery_cursor: usize,
    sent: Vec<DeliveryKey>,
    cycle_attempts: Vec<DeliveryKey>,
    pending: Vec<DeliveryKey>,
    /// Thread IDs confirmed paused during the current session (in-memory only).
    paused_threads: Vec<String>,
    running: bool,
    queued: Option<PendingSample>,
    generation: u64,
    periods: HashMap<String, ThreadPeriod>,
    period_counter: u64,
    current_account: Option<String>,
    transport: Arc<dyn BridgeTransport>,
    discovery_override: Option<BridgeDiscovery>,
    connection: BridgeConnection,
    confirmed_count: u64,
    last_confirmed_at: Option<i64>,
    delivery_deadline: Duration,
    spawn_worker: WorkerSpawner,
    spawn_probe: WorkerSpawner,
}

impl Default for AutoPauseController {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoPauseController {
    pub fn transport(&self) -> Arc<dyn BridgeTransport> {
        self.transport.clone()
    }

    pub fn new() -> Self {
        Self {
            telemetry: status::Telemetry::default(),
            delivery_cursor: 0,
            sent: Vec::new(),
            cycle_attempts: Vec::new(),
            pending: Vec::new(),
            paused_threads: Vec::new(),
            running: false,
            queued: None,
            generation: 0,
            periods: HashMap::new(),
            period_counter: 0,
            current_account: None,
            transport: transport::default_transport(),
            discovery_override: None,
            connection: BridgeConnection::Unknown,
            confirmed_count: 0,
            last_confirmed_at: None,
            delivery_deadline: DELIVERY_DEADLINE,
            spawn_worker: Arc::new(|state| {
                tauri::async_runtime::spawn(async move {
                    run_delivery_worker(state).await;
                });
            }),
            spawn_probe: Arc::new(|state| {
                tauri::async_runtime::spawn(async move {
                    probe_bridge(&state).await;
                });
            }),
        }
    }

    pub fn status(&self, enabled: bool) -> AutoPauseStatus {
        AutoPauseStatus::from_controller(self, enabled)
    }

    pub fn paused_threads(&self) -> Vec<String> {
        self.paused_threads.clone()
    }

    pub fn record_paused_thread(&mut self, thread_id: &str) {
        if thread_id.is_empty() {
            return;
        }
        self.paused_threads.retain(|id| id != thread_id);
        self.paused_threads.push(thread_id.to_owned());
        while self.paused_threads.len() > MAX_COLLECTION_ENTRIES {
            self.paused_threads.remove(0);
        }
    }

    pub fn remove_paused_thread(&mut self, thread_id: &str) {
        self.paused_threads.retain(|id| id != thread_id);
    }

    pub fn clear_paused_threads(&mut self) {
        self.paused_threads.clear();
    }

    #[cfg(test)]
    pub(crate) fn set_transport(&mut self, transport: Arc<dyn BridgeTransport>) {
        self.transport = transport;
    }

    #[cfg(test)]
    pub(crate) fn set_discovery(&mut self, discovery: BridgeDiscovery) {
        self.discovery_override = Some(discovery);
    }

    #[cfg(test)]
    pub(crate) fn set_worker_spawner(&mut self, spawner: WorkerSpawner) {
        self.spawn_worker = spawner;
    }

    #[cfg(test)]
    pub(crate) fn set_probe_spawner(&mut self, spawner: WorkerSpawner) {
        self.spawn_probe = spawner;
    }

    #[cfg(test)]
    pub(crate) fn set_delivery_deadline(&mut self, deadline: Duration) {
        self.delivery_deadline = deadline;
    }

    #[cfg(test)]
    pub(crate) fn running_flag(&self) -> bool {
        self.running
    }

    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub(crate) fn sent_len(&self) -> usize {
        self.sent.len()
    }

    fn candidates(discovery: &BridgeDiscovery) -> Vec<BridgeCandidate> {
        transport::build_candidates(
            discovery.explicit_pipe_path.as_deref(),
            &discovery.pipe_names,
            &discovery.caller_thread_ids,
        )
    }

    fn prune(&mut self, account: &str, observed_at: i64) {
        if self.current_account.as_deref() != Some(account) {
            self.telemetry.targets.clear();
            self.sent.clear();
            self.pending.clear();
            self.periods.clear();
            self.paused_threads.clear();
            self.current_account = Some(account.to_owned());
        }
        let still_relevant = |key: &DeliveryKey| {
            key.resets_at
                .map(|resets_at| resets_at > observed_at)
                .unwrap_or(true)
        };
        self.cycle_attempts.retain(still_relevant);
        self.sent.retain(still_relevant);
        self.pending.retain(still_relevant);
    }

    fn clear_active_periods(&mut self) {
        self.periods.clear();
        self.invalidate_pending();
    }

    pub(crate) fn invalidate_pending(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.queued = None;
    }

    fn refresh_periods(&mut self, threads: &[ActiveThread]) {
        let active: HashSet<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
        self.periods.retain(|id, _| active.contains(id.as_str()));

        for thread in transport::session_targets(threads) {
            if self
                .periods
                .get(&thread.id)
                .is_none_or(|period| period.host_id != thread.host_id)
            {
                let period_id = self.next_period_id(thread);
                self.periods.insert(
                    thread.id.clone(),
                    ThreadPeriod {
                        period_id,
                        host_id: thread.host_id.clone(),
                    },
                );
            } else if let Some(turn) = thread
                .turn_id
                .as_deref()
                .map(str::trim)
                .filter(|turn| !turn.is_empty())
            {
                let period_id = format!("{}#t{}", period_stem(thread), turn);
                if let Some(period) = self.periods.get_mut(&thread.id) {
                    period.period_id = period_id;
                }
            }
        }
    }

    fn next_period_id(&mut self, thread: &ActiveThread) -> String {
        match thread
            .turn_id
            .as_deref()
            .map(str::trim)
            .filter(|turn| !turn.is_empty())
        {
            Some(turn) => format!("{}#t{}", period_stem(thread), turn),
            None => {
                self.period_counter += 1;
                format!("{}#p{}", period_stem(thread), self.period_counter)
            }
        }
    }

    fn period_id(&self, thread_id: &str) -> Option<String> {
        self.periods
            .get(thread_id)
            .map(|period| period.period_id.clone())
    }

    fn reserve(
        &mut self,
        account: &str,
        thread_id: &str,
        triggers: &[Trigger],
    ) -> Reservation {
        let keys: Vec<DeliveryKey> = triggers
            .iter()
            .map(|trigger| DeliveryKey {
                account: account.to_owned(),
                quota: trigger.quota,
                resets_at: trigger.resets_at,
                thread_id: thread_id.to_owned(),
            })
            .collect();

        let uncovered: Vec<DeliveryKey> = keys
            .iter()
            .filter(|key| !self.sent.contains(key) && !self.pending.contains(key))
            .cloned()
            .collect();
        if uncovered.is_empty() {
            return Reservation::Nothing;
        }

        if self
            .sent
            .iter()
            .any(|key| key.account == account && key.thread_id == thread_id)
        {
            for key in uncovered {
                self.push_sent(key);
            }
            return Reservation::Backfilled;
        }

        for key in &uncovered {
            self.push_pending(key.clone());
        }
        Reservation::Send { keys: uncovered }
    }

    fn cycle_keys(account: &str, thread: &str, triggers: &[Trigger]) -> Vec<DeliveryKey> {
        triggers
            .iter()
            .filter(|t| t.resets_at.is_some())
            .map(|t| DeliveryKey {
                account: account.into(),
                thread_id: thread.into(),
                quota: t.quota,
                resets_at: t.resets_at,
            })
            .collect()
    }

    fn reserve_target(
        &mut self,
        account: &str,
        thread: &str,
        period: &str,
        triggers: &[Trigger],
    ) -> Reservation {
        let keys = Self::cycle_keys(account, thread, triggers);
        let every_trigger_has_a_cycle = keys.len() == triggers.len();
        if every_trigger_has_a_cycle && keys.iter().all(|k| self.cycle_attempts.contains(k)) {
            return Reservation::Nothing;
        }
        if self.cycle_attempts.len()
            + keys
                .iter()
                .filter(|k| !self.cycle_attempts.contains(k))
                .count()
            > 512
        {
            return Reservation::Capacity;
        }
        let result = self.reserve(account, period, triggers);
        if matches!(result, Reservation::Backfilled) {
            for key in keys {
                if !self.cycle_attempts.contains(&key) {
                    self.cycle_attempts.push(key);
                }
            }
        }
        result
    }

    fn confirm(&mut self, keys: &[DeliveryKey]) {
        self.pending.retain(|key| !keys.contains(key));
        for key in keys {
            self.push_sent(key.clone());
        }
    }

    fn release(&mut self, keys: &[DeliveryKey]) {
        self.pending.retain(|key| !keys.contains(key));
    }

    fn push_sent(&mut self, key: DeliveryKey) {
        if self.sent.contains(&key) {
            return;
        }
        self.sent.push(key);
        while self.sent.len() > MAX_COLLECTION_ENTRIES {
            self.sent.remove(0);
        }
    }

    fn push_pending(&mut self, key: DeliveryKey) {
        if self.pending.contains(&key) {
            return;
        }
        self.pending.push(key);
        while self.pending.len() > MAX_COLLECTION_ENTRIES {
            self.pending.remove(0);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Reservation {
    Nothing,
    Capacity,
    Backfilled,
    Send { keys: Vec<DeliveryKey> },
}

fn lock_controller(state: &Arc<ProviderRuntime>) -> std::sync::MutexGuard<'_, AutoPauseController> {
    state
        .auto_pause
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Accepts a fresh sample and schedules a delivery when it qualifies.
pub fn schedule(
    state: &Arc<ProviderRuntime>,
    settings: OverlaySettings,
    usage: CodexUsage,
    account_id: Option<String>,
) {
    let generation = {
        let mut controller = lock_controller(state);
        controller.telemetry.account_known =
            account_id.as_ref().is_some_and(|id| !id.trim().is_empty());
        if controller.current_account.as_deref() != account_id.as_deref().map(str::trim) {
            controller.telemetry.targets.clear();
        }
        controller.invalidate_pending();
        controller.generation
    };
    let Some(account_id) = account_id
        .map(|account| account.trim().to_owned())
        .filter(|account| !account.is_empty())
    else {
        state.status_wake.notify_one();
        return;
    };
    let timing = state
        .reader
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .observation;
    if timing.is_some_and(|timing| !timing.action_eligible()) {
        lock_controller(state).prune(&account_id, usage.fetched_at);
        state.status_wake.notify_one();
        return;
    }
    let codex = settings.preference(crate::config::Provider::Codex);
    if !(codex.monitor_enabled && codex.protection_enabled) {
        lock_controller(state).clear_active_periods();
        state.status_wake.notify_one();
        return;
    }
    if triggered_quotas(&settings, &usage).is_empty() {
        lock_controller(state).clear_active_periods();
        probe_bridge_status(state);
        state.status_wake.notify_one();
        return;
    }

    let sample = PendingSample {
        settings,
        usage,
        account_id,
        generation,
        received_at: timing
            .map(|v| v.quota_started)
            .unwrap_or_else(tokio::time::Instant::now),
    };

    let spawner = {
        let mut controller = lock_controller(state);
        if controller.generation != generation {
            return;
        }
        controller.prune(&sample.account_id, sample.usage.fetched_at);
        controller.queued = Some(sample);
        if controller.running {
            None
        } else {
            controller.running = true;
            Some(controller.spawn_worker.clone())
        }
    };
    state.status_wake.notify_one();

    if let Some(spawner) = spawner {
        spawner(state.clone());
    }
}

pub fn clear_active_periods(state: &Arc<ProviderRuntime>) {
    lock_controller(state).clear_active_periods();
}

/// Rechecks bridge health with bounded retries while protection is enabled.
pub fn probe_bridge_status(state: &Arc<ProviderRuntime>) {
    let spawner = {
        let mut controller = lock_controller(state);
        let interval = 30i64
            .saturating_mul(1i64 << controller.telemetry.probe_failures.min(4))
            .min(300);
        if controller.running
            || controller.telemetry.probe_running
            || controller
                .telemetry
                .last_probe_at
                .is_some_and(|at| unix_now().saturating_sub(at) < interval)
        {
            return;
        }
        controller.telemetry.probe_running = true;
        controller.spawn_probe.clone()
    };
    spawner(state.clone());
}

pub(crate) async fn probe_bridge(state: &Arc<ProviderRuntime>) {
    let result = tokio::time::timeout(DELIVERY_DEADLINE, probe_bridge_inner(state)).await;
    {
        let mut controller = lock_controller(state);
        controller.telemetry.probe_running = false;
        controller.telemetry.last_probe_at = Some(unix_now());
        if result.is_err() {
            controller.connection = BridgeConnection::Unavailable;
        }
        if controller.connection == BridgeConnection::Unavailable {
            controller.telemetry.probe_failures =
                controller.telemetry.probe_failures.saturating_add(1);
        } else {
            controller.telemetry.probe_failures = 0;
        }
    }
    state.status_wake.notify_one();
}

async fn probe_bridge_inner(state: &Arc<ProviderRuntime>) {
    let generation = lock_controller(state).generation;
    let (transport, override_discovery) = {
        let controller = lock_controller(state);
        (
            controller.transport.clone(),
            controller.discovery_override.clone(),
        )
    };
    let discovery = match override_discovery {
        Some(discovery) => discovery,
        None => transport::discover_bridge().await,
    };
    let candidates = AutoPauseController::candidates(&discovery);
    let mut answered = false;
    let mut threads: Option<Vec<ActiveThread>> = None;
    for candidate in candidates {
        match transport::handshake(transport.as_ref(), &candidate).await {
            Err(_) => continue,
            Ok(transport::Handshake::NoActiveSession) => {
                answered = true;
                break;
            }
            Ok(transport::Handshake::Targets(listed)) => {
                answered = true;
                threads = Some(listed);
                break;
            }
        }
    }
    let mut controller = lock_controller(state);
    controller.connection = if answered {
        BridgeConnection::Available
    } else {
        BridgeConnection::Unavailable
    };
    if controller.generation == generation {
        if let Some(threads) = threads {
            controller.refresh_periods(&threads);
        }
    }
}

/// Consumes the queued samples until the queue is empty.
pub(crate) async fn run_delivery_worker(state: Arc<ProviderRuntime>) {
    loop {
        let sample = {
            let mut controller = lock_controller(&state);
            match controller.queued.take() {
                Some(sample) => sample,
                None => {
                    controller.running = false;
                    drop(controller);
                    state.status_wake.notify_one();
                    return;
                }
            }
        };

        let deadline = { lock_controller(&state).delivery_deadline };
        let _ = tokio::time::timeout(deadline, deliver_batch(&state, &sample)).await;
        state.status_wake.notify_one();
    }
}

async fn delivery_is_current(state: &Arc<ProviderRuntime>, sample: &PendingSample) -> bool {
    if sample.received_at.elapsed() > Duration::from_secs(30) {
        return false;
    }
    let settings = state.settings.lock().await;
    let codex = settings.preference(crate::config::Provider::Codex);
    let sample_codex = sample.settings.preference(crate::config::Provider::Codex);
    if !(codex.monitor_enabled && codex.protection_enabled)
        || codex.five_hour_threshold_percent != sample_codex.five_hour_threshold_percent
        || codex.weekly_threshold_percent != sample_codex.weekly_threshold_percent
    {
        return false;
    }
    let controller = lock_controller(state);
    controller.generation == sample.generation
        && controller.current_account.as_deref() == Some(sample.account_id.as_str())
}

/// Delivers one batch: discover, handshake, reserve and send.
async fn deliver_batch(state: &Arc<ProviderRuntime>, sample: &PendingSample) {
    if !delivery_is_current(state, sample).await {
        return;
    }
    let triggers = triggered_quotas(&sample.settings, &sample.usage);
    if triggers.is_empty() {
        return;
    }

    let (transport, override_discovery) = {
        let controller = lock_controller(state);
        (
            controller.transport.clone(),
            controller.discovery_override.clone(),
        )
    };
    let time_zone = crate::pause_message::TimeZoneContext::system();
    let discovery = match override_discovery {
        Some(discovery) => discovery,
        None => transport::discover_bridge().await,
    };
    let candidates = AutoPauseController::candidates(&discovery);
    if candidates.is_empty() {
        let mut controller = lock_controller(state);
        controller.connection = BridgeConnection::Unavailable;
        return;
    }

    let mut answered = false;
    let mut reported: Option<(BridgeCandidate, Vec<ActiveThread>)> = None;
    let mut exhausted_pipes: HashSet<String> = HashSet::new();
    for candidate in candidates {
        if !delivery_is_current(state, sample).await {
            return;
        }
        if exhausted_pipes.contains(&candidate.pipe.to_ascii_lowercase()) {
            continue;
        }
        match transport::handshake(transport.as_ref(), &candidate).await {
            Err(_) => continue,
            Ok(transport::Handshake::NoActiveSession) => {
                answered = true;
                exhausted_pipes.insert(candidate.pipe.to_ascii_lowercase());
                continue;
            }
            Ok(transport::Handshake::Targets(threads)) => {
                answered = true;
                reported = Some((candidate, threads));
                break;
            }
        }
    }

    if !delivery_is_current(state, sample).await {
        return;
    }
    let Some((candidate, threads)) = reported else {
        let mut controller = lock_controller(state);
        controller.telemetry.last_probe_at = Some(unix_now());
        controller.connection = if answered {
            BridgeConnection::Available
        } else {
            BridgeConnection::Unavailable
        };
        if answered {
            controller.refresh_periods(&[]);
        }
        return;
    };

    {
        let mut controller = lock_controller(state);
        controller.connection = BridgeConnection::Available;
        controller.telemetry.last_probe_at = Some(unix_now());
        controller.refresh_periods(&threads);
    }
    let targets = transport::session_targets(&threads);
    let start = lock_controller(state).delivery_cursor % targets.len();
    for offset in 0..targets.len() {
        let index = (start + offset) % targets.len();
        let thread = targets[index];
        lock_controller(state).delivery_cursor = (index + 1) % targets.len();
        if !delivery_is_current(state, sample).await {
            return;
        }
        let period_id = { lock_controller(state).period_id(&thread.id) };
        let Some(period_id) = period_id else {
            continue;
        };
        let reservation = {
            lock_controller(state).reserve_target(
                &sample.account_id,
                &thread.id,
                &period_id,
                &triggers,
            )
        };
        let keys = match reservation {
            Reservation::Nothing | Reservation::Backfilled => continue,
            Reservation::Capacity => {
                lock_controller(state).telemetry.record(
                    thread,
                    &period_id,
                    "failed",
                    Some("ledgerFull"),
                );
                continue;
            }
            Reservation::Send { keys } => keys,
        };

        let mut guard = ReservationGuard::new(state.clone(), keys.clone());
        guard.cycles = AutoPauseController::cycle_keys(&sample.account_id, &thread.id, &triggers);
        let language = &sample.settings.language;
        let prompt = if triggers.iter().any(|t| t.quota == QuotaKind::Weekly) {
            crate::pause_message::effective_message(
                &sample.settings.auto_pause_weekly_message,
                language,
            )
        } else {
            crate::pause_message::render_five_hour(
                &crate::pause_message::effective_message(
                    &sample.settings.auto_pause_five_hour_message,
                    language,
                ),
                sample.usage.five_hour_resets_at,
                unix_now(),
                language,
                &time_zone,
            )
        };

        lock_controller(state)
            .telemetry
            .record(thread, &period_id, "requested", None);
        let may_submit = || lock_controller(state).generation == sample.generation;
        let result = transport::send_pause(
            transport.as_ref(),
            &candidate,
            thread,
            &prompt,
            &guard.started,
            &may_submit,
        )
        .await;
        if matches!(result, Ok(false)) {
            guard.started.store(false, Ordering::Release);
        }
        let confirmed = result.unwrap_or(false);
        if confirmed {
            let mut controller = lock_controller(state);
            let carries_forward = controller.queued.as_ref().is_some_and(|next| {
                next.account_id == sample.account_id
                    && keys.iter().all(|key| {
                        controller
                            .periods
                            .values()
                            .any(|period| period.period_id == key.thread_id)
                            && triggered_quotas(&next.settings, &next.usage)
                                .iter()
                                .any(|trigger| {
                                    trigger.quota == key.quota && trigger.resets_at == key.resets_at
                                })
                    })
            });
            if controller.generation == sample.generation || carries_forward {
                controller.confirm(&keys);
                guard.mark_confirmed();
            }
            controller.confirmed_count += 1;
            controller.last_confirmed_at = Some(unix_now());
            if controller.generation == sample.generation || carries_forward {
                controller.telemetry.record(
                    thread,
                    &period_id,
                    "accepted",
                    Some("stoppingUnconfirmed"),
                );
                controller.record_paused_thread(&thread.id);
            }
        } else if lock_controller(state).generation == sample.generation {
            let unknown = guard.started.load(Ordering::Acquire);
            lock_controller(state).telemetry.record(
                thread,
                &period_id,
                if unknown { "unknown" } else { "failed" },
                Some(if unknown {
                    "submissionUnknown"
                } else {
                    "deliveryFailed"
                }),
            );
        }
    }
}

fn period_stem(thread: &ActiveThread) -> String {
    match &thread.host_id {
        Some(host) => format!("{}@{}", thread.id, host),
        None => thread.id.clone(),
    }
}

struct ReservationGuard {
    state: Arc<ProviderRuntime>,
    keys: Vec<DeliveryKey>,
    confirmed: bool,
    started: AtomicBool,
    cycles: Vec<DeliveryKey>,
}

impl ReservationGuard {
    fn new(state: Arc<ProviderRuntime>, keys: Vec<DeliveryKey>) -> Self {
        Self {
            state,
            keys,
            confirmed: false,
            started: AtomicBool::new(false),
            cycles: Vec::new(),
        }
    }

    fn mark_confirmed(&mut self) {
        self.confirmed = true;
    }
}

impl Drop for ReservationGuard {
    fn drop(&mut self) {
        let mut controller = lock_controller(&self.state);
        if self.confirmed || self.started.load(Ordering::Acquire) {
            for key in &self.cycles {
                if !controller.cycle_attempts.contains(key) {
                    controller.cycle_attempts.push(key.clone());
                }
            }
        }
        if !self.confirmed {
            if self.started.load(Ordering::Acquire) {
                for target in controller.telemetry.targets.values_mut() {
                    if self.keys.iter().any(|key| key.thread_id == target.period_id) {
                        target.phase = "unknown".into();
                        target.reason = Some("submissionUnknown".into());
                    }
                }
            }
            controller.release(&self.keys);
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::pause_control::transport::BridgeError;
    use serde_json::json;
    use serde_json::Value;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) struct RecordedCall {
        pub pipe: String,
        pub caller_thread_id: String,
        pub tool_name: String,
        pub arguments: Value,
    }

    pub(crate) struct ScriptedTransport {
        inner: Mutex<ScriptedState>,
    }

    #[derive(Default)]
    struct ScriptedState {
        calls: Vec<RecordedCall>,
        list_threads: Option<ToolCallOutcome>,
        send_success: bool,
        send_error: Option<String>,
        send_delay_ms: u64,
        handshake_gate: Option<Arc<tokio::sync::Notify>>,
        send_gate: Option<Arc<tokio::sync::Notify>>,
    }

    impl ScriptedTransport {
        pub(crate) fn new() -> Self {
            let state = ScriptedState {
                list_threads: Some(ToolCallOutcome {
                    success: true,
                    text: Some(
                        json!({
                            "pinnedThreads": [],
                            "threads": [{
                                "id": "session-1",
                                "kind": "codex",
                                "status": "active",
                                "hostId": "host-1",
                            }],
                        })
                        .to_string(),
                    ),
                }),
                send_success: true,
                ..ScriptedState::default()
            };
            Self {
                inner: Mutex::new(state),
            }
        }

        pub(crate) fn set_list_threads(&self, outcome: ToolCallOutcome) {
            self.inner.lock().unwrap().list_threads = Some(outcome);
        }

        pub(crate) fn set_send_success(&self, success: bool) {
            self.inner.lock().unwrap().send_success = success;
        }

        pub(crate) fn set_send_error(&self, error: &str) {
            self.inner.lock().unwrap().send_error = Some(error.to_owned());
        }

        pub(crate) fn set_send_delay_ms(&self, delay_ms: u64) {
            self.inner.lock().unwrap().send_delay_ms = delay_ms;
        }

        pub(crate) fn hold_handshakes(&self) -> Arc<tokio::sync::Notify> {
            let gate = Arc::new(tokio::sync::Notify::new());
            self.inner.lock().unwrap().handshake_gate = Some(gate.clone());
            gate
        }

        pub(crate) fn hold_sends(&self) -> Arc<tokio::sync::Notify> {
            let gate = Arc::new(tokio::sync::Notify::new());
            self.inner.lock().unwrap().send_gate = Some(gate.clone());
            gate
        }

        pub(crate) fn calls(&self) -> Vec<RecordedCall> {
            self.inner.lock().unwrap().calls.clone()
        }

        pub(crate) fn sent_prompts(&self) -> Vec<String> {
            self.inner
                .lock()
                .unwrap()
                .calls
                .iter()
                .filter(|call| call.tool_name == "send_message_to_thread")
                .filter_map(|call| {
                    call.arguments
                        .get("prompt")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect()
        }
    }

    impl BridgeTransport for ScriptedTransport {
        fn call_tool<'a>(
            &'a self,
            pipe: &'a str,
            caller_thread_id: &'a str,
            tool_name: &'a str,
            arguments: Value,
        ) -> Pin<Box<dyn Future<Output = Result<ToolCallOutcome, BridgeError>> + Send + 'a>> {
            Box::pin(async move {
                let (
                    list_threads,
                    send_success,
                    send_error,
                    send_delay_ms,
                    handshake_gate,
                    send_gate,
                ) = {
                    let mut state = self.inner.lock().unwrap();
                    state.calls.push(RecordedCall {
                        pipe: pipe.to_owned(),
                        caller_thread_id: caller_thread_id.to_owned(),
                        tool_name: tool_name.to_owned(),
                        arguments: arguments.clone(),
                    });
                    (
                        state.list_threads.clone(),
                        state.send_success,
                        state.send_error.clone(),
                        state.send_delay_ms,
                        state.handshake_gate.clone(),
                        state.send_gate.clone(),
                    )
                };
                if tool_name == "list_threads" {
                    if let Some(gate) = handshake_gate {
                        gate.notified().await;
                    }
                    return match list_threads {
                        Some(outcome) => Ok(outcome),
                        None => Err(BridgeError::Protocol(
                            "the list_threads call was not confirmed".into(),
                        )),
                    };
                }
                if let Some(gate) = send_gate {
                    gate.notified().await;
                }
                if send_delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(send_delay_ms)).await;
                }
                if let Some(error) = send_error {
                    return Err(BridgeError::Unavailable(error));
                }
                Ok(ToolCallOutcome {
                    success: send_success,
                    text: None,
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> OverlaySettings {
        let mut settings = OverlaySettings::default();
        settings.preference_mut(crate::config::Provider::Codex).protection_enabled = true;
        settings
    }

    fn usage(
        five_hour: Option<u32>,
        five_hour_resets_at: Option<i64>,
        week: Option<u32>,
        week_resets_at: Option<i64>,
        fetched_at: i64,
    ) -> CodexUsage {
        CodexUsage {
            five_hour_remaining_percent: five_hour,
            five_hour_burn_rate_per_hour: None,
            week_remaining_percent: week,
            week_burn_rate_per_hour: None,
            credits_display: "—".to_string(),
            credits_balance: None,
            has_credits: false,
            five_hour_resets_at,
            week_resets_at,
            fetched_at,
            antigravity: Default::default(),
        }
    }

    #[test]
    fn the_switch_gates_every_trigger() {
        let mut disabled = OverlaySettings::default();
        disabled.preference_mut(crate::config::Provider::Codex).protection_enabled = false;
        let sample = usage(Some(0), Some(1_800), Some(0), Some(1_800), 1_000);
        assert!(triggered_quotas(&disabled, &sample).is_empty());
        assert_eq!(triggered_quotas(&settings(), &sample).len(), 2);
    }

    #[test]
    fn each_threshold_triggers_at_its_own_boundary() {
        let current = settings();

        let five_hit = usage(Some(5), Some(1_800), Some(50), Some(1_800), 1_000);
        let triggers = triggered_quotas(&current, &five_hit);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].quota, QuotaKind::FiveHour);

        let five_missed = usage(Some(6), Some(1_800), Some(50), Some(1_800), 1_000);
        assert!(triggered_quotas(&current, &five_missed).is_empty());

        let weekly_hit = usage(Some(50), Some(1_800), Some(2), Some(1_800), 1_000);
        let triggers = triggered_quotas(&current, &weekly_hit);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].quota, QuotaKind::Weekly);

        let weekly_missed = usage(Some(50), Some(1_800), Some(3), Some(1_800), 1_000);
        assert!(triggered_quotas(&current, &weekly_missed).is_empty());
    }

    #[test]
    fn an_unknown_remaining_percentage_never_triggers() {
        let sample = usage(None, Some(1_800), None, Some(1_800), 1_000);
        assert!(triggered_quotas(&settings(), &sample).is_empty());
    }

    #[test]
    fn a_five_hour_pause_does_not_delegate_unverified_resume_scheduling() {
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(1_000 + 3_600 + 120),
        }];
        let message = pause_message("zh-CN", &triggers, 1_000);
        assert_eq!(message, PAUSE_MESSAGE_ZH_CN);
    }

    #[test]
    fn a_partial_minute_reset_still_requests_pause_only() {
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(1_000 + 61),
        }];
        assert_eq!(pause_message("zh-CN", &triggers, 1_000), PAUSE_MESSAGE_ZH_CN);
    }

    #[test]
    fn an_unusable_reset_time_falls_back_to_the_plain_pause() {
        for resets_at in [
            None,
            Some(999),
            Some(1_000 + 6 * 3_600 + 1),
            Some(1_000 + 30 * 24 * 3_600),
            Some(i64::MIN),
            Some(i64::MAX),
        ] {
            let triggers = vec![Trigger {
                quota: QuotaKind::FiveHour,
                resets_at,
            }];
            assert_eq!(pause_message("zh-CN", &triggers, 1_000), PAUSE_MESSAGE_ZH_CN);
        }
    }

    #[test]
    fn weekly_wins_over_the_five_hour_window() {
        let triggers = vec![
            Trigger {
                quota: QuotaKind::FiveHour,
                resets_at: Some(1_000 + 3_600),
            },
            Trigger {
                quota: QuotaKind::Weekly,
                resets_at: Some(1_000 + 86_400),
            },
        ];
        assert_eq!(pause_message("zh-CN", &triggers, 1_000), PAUSE_MESSAGE_ZH_CN);
    }

    #[test]
    fn a_weekly_trigger_alone_uses_the_plain_pause() {
        let triggers = vec![Trigger {
            quota: QuotaKind::Weekly,
            resets_at: Some(1_000 + 86_400),
        }];
        assert_eq!(pause_message("zh-CN", &triggers, 1_000), PAUSE_MESSAGE_ZH_CN);
    }

    #[test]
    fn pause_message_matches_overlay_display_language() {
        assert_eq!(pause_message("zh-CN", &[], 0), PAUSE_MESSAGE_ZH_CN);
        assert_eq!(pause_message("zh-Hant", &[], 0), PAUSE_MESSAGE_ZH_HANT);
        assert_eq!(pause_message("en-US", &[], 0), PAUSE_MESSAGE_EN);
        assert_eq!(pause_message_for_locale("zh-CN"), PAUSE_MESSAGE_ZH_CN);
        assert_eq!(pause_message_for_locale("zh-Hant"), PAUSE_MESSAGE_ZH_HANT);
        assert_eq!(pause_message_for_locale("en-US"), PAUSE_MESSAGE_EN);
        assert_eq!(pause_message_for_locale("other"), PAUSE_MESSAGE_EN);
        assert_eq!(resume_message_for_locale("zh-CN"), RESUME_MESSAGE_ZH_CN);
        assert_eq!(resume_message_for_locale("zh-Hant"), RESUME_MESSAGE_ZH_HANT);
        assert_eq!(resume_message_for_locale("en-US"), RESUME_MESSAGE_EN);
        assert_eq!(resume_message_for_language("zh-CN"), "继续");
        assert_eq!(resume_message_for_language("zh-Hant"), "繼續");
        assert_eq!(resume_message_for_language("en-US"), "Continue");
    }

    #[test]
    fn a_period_identity_is_minted_per_active_period() {
        let mut controller = AutoPauseController::new();
        let thread = ActiveThread {
            id: "session-1".to_string(),
            host_id: None,
            turn_id: None,
        };
        controller.refresh_periods(std::slice::from_ref(&thread));
        let first = controller.period_id("session-1").expect("a period is minted");
        assert!(first.starts_with("session-1#p"));

        controller.refresh_periods(std::slice::from_ref(&thread));
        assert_eq!(
            controller.period_id("session-1").as_deref(),
            Some(first.as_str())
        );

        controller.refresh_periods(&[]);
        assert_eq!(controller.period_id("session-1"), None);

        controller.refresh_periods(std::slice::from_ref(&thread));
        let second = controller
            .period_id("session-1")
            .expect("a new period is minted");
        assert_ne!(first, second);
    }

    #[test]
    fn a_reported_turn_id_identifies_the_period() {
        let mut controller = AutoPauseController::new();
        let thread = ActiveThread {
            id: "session-1".to_string(),
            host_id: None,
            turn_id: Some("turn-9".to_string()),
        };
        controller.refresh_periods(std::slice::from_ref(&thread));
        assert_eq!(
            controller.period_id("session-1").as_deref(),
            Some("session-1#tturn-9")
        );
    }

    #[test]
    fn an_owner_change_creates_a_distinct_period_even_with_the_same_turn_id() {
        let mut controller = AutoPauseController::new();
        let mut thread = ActiveThread {
            id: "session-1".into(),
            host_id: Some("host-a".into()),
            turn_id: Some("turn-9".into()),
        };
        controller.refresh_periods(std::slice::from_ref(&thread));
        let first = controller.period_id(&thread.id).unwrap();
        thread.host_id = Some("host-b".into());
        controller.refresh_periods(std::slice::from_ref(&thread));
        assert_ne!(controller.period_id(&thread.id).unwrap(), first);
    }

    #[test]
    fn a_reservation_is_only_recorded_after_a_confirmation() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];

        let reservation = controller.reserve("acct", "session-1#p1", &triggers);
        let keys = match reservation {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        assert_eq!(keys.len(), 1);

        controller.release(&keys);
        let again = controller.reserve("acct", "session-1#p1", &triggers);
        assert!(matches!(again, Reservation::Send { .. }));
    }

    #[test]
    fn a_confirmed_reservation_is_not_repeated() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        let keys = match controller.reserve("acct", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &triggers),
            Reservation::Nothing
        ));
    }

    #[test]
    fn two_quota_windows_produce_one_message_and_two_keys() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![
            Trigger {
                quota: QuotaKind::FiveHour,
                resets_at: Some(2_000),
            },
            Trigger {
                quota: QuotaKind::Weekly,
                resets_at: Some(9_000),
            },
        ];
        let keys = match controller.reserve("acct", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        assert_eq!(keys.len(), 2);
        controller.confirm(&keys);

        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &triggers),
            Reservation::Nothing
        ));
    }

    #[test]
    fn a_later_second_window_in_the_same_period_is_backfilled_without_a_message() {
        let mut controller = AutoPauseController::new();
        let five_hour = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        let weekly = vec![Trigger {
            quota: QuotaKind::Weekly,
            resets_at: Some(9_000),
        }];

        let keys = match controller.reserve("acct", "session-1#p1", &five_hour) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);

        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &weekly),
            Reservation::Backfilled
        ));
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &weekly),
            Reservation::Nothing
        ));
    }

    #[test]
    fn a_new_session_may_be_prompted_on_its_own() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        let keys = match controller.reserve("acct", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);

        assert!(matches!(
            controller.reserve("acct", "session-2#p2", &triggers),
            Reservation::Send { .. }
        ));
    }

    #[test]
    fn an_account_switch_clears_the_ledger() {
        let mut controller = AutoPauseController::new();
        controller.record_paused_thread("session-1");
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        let keys = match controller.reserve("acct-1", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);

        controller.prune("acct-2", 1_000);
        assert!(controller.paused_threads().is_empty());
        assert!(matches!(
            controller.reserve("acct-2", "session-1#p1", &triggers),
            Reservation::Send { .. }
        ));
    }

    #[test]
    fn in_session_paused_threads_survive_quota_recovery_until_resumed_or_cleared() {
        let mut controller = AutoPauseController::new();
        controller.record_paused_thread("session-1");
        controller.record_paused_thread("session-2");
        controller.record_paused_thread("session-1");
        assert_eq!(controller.paused_threads(), vec!["session-2", "session-1"]);

        // Clearing active periods on quota recovery preserves paused threads for manual resume.
        controller.clear_active_periods();
        assert_eq!(controller.paused_threads(), vec!["session-2", "session-1"]);

        controller.remove_paused_thread("session-2");
        assert_eq!(controller.paused_threads(), vec!["session-1"]);

        controller.clear_paused_threads();
        assert!(controller.paused_threads().is_empty());
    }

    #[test]
    fn an_expired_quota_cycle_is_dropped() {
        let mut controller = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2_000),
        }];
        controller.prune("acct", 1_000);

        let keys = match controller.reserve("acct", "session-1#p1", &triggers) {
            Reservation::Send { keys } => keys,
            other => panic!("expected a send reservation, got {other:?}"),
        };
        controller.confirm(&keys);

        controller.prune("acct", 1_500);
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &triggers),
            Reservation::Nothing
        ));

        controller.prune("acct", 3_000);
        assert!(matches!(
            controller.reserve("acct", "session-1#p1", &triggers),
            Reservation::Send { .. }
        ));
    }

    #[test]
    fn the_ledger_is_bounded() {
        let mut controller = AutoPauseController::new();
        for index in 0..(MAX_COLLECTION_ENTRIES + 50) {
            let triggers = vec![Trigger {
                quota: QuotaKind::FiveHour,
                resets_at: Some(2_000),
            }];
            let thread_id = format!("session-{index}");
            let keys = match controller.reserve("acct", &thread_id, &triggers) {
                Reservation::Send { keys } => keys,
                other => panic!("expected a send reservation, got {other:?}"),
            };
            controller.confirm(&keys);
        }
        assert!(controller.sent.len() <= MAX_COLLECTION_ENTRIES);
    }

    #[test]
    fn a_full_cycle_ledger_refuses_new_work_without_evicting_unknown_attempts() {
        let mut c = AutoPauseController::new();
        let triggers = vec![Trigger {
            quota: QuotaKind::FiveHour,
            resets_at: Some(2000),
        }];
        for i in 0..512 {
            c.cycle_attempts
                .extend(AutoPauseController::cycle_keys("a", &i.to_string(), &triggers));
        }
        let before = c.cycle_attempts.clone();
        assert!(matches!(
            c.reserve_target("a", "new", "new#period", &triggers),
            Reservation::Capacity
        ));
        assert_eq!(before, c.cycle_attempts);
    }
}
