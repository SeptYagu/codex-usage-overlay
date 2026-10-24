const listeners = new Map();
let protection = null;
let revision = 0;
export const setProtectionFixture = (fixture) => {
    const now = Math.floor(Date.now() / 1000);
    const quotaRecovery = ['protection-retrying', 'protection-quota-error', 'protection-stale'].includes(fixture);
    const summary = fixture === 'protection-requested' ? 'pauseRequested' : fixture === 'protection-error' || quotaRecovery
        ? 'needsAttention' : fixture === 'protection-low' ? 'quotaLow' : 'armed';
    protection = { revision: ++revision, enabled: true, autoResumeEnabled: false, mode: 'cooperative',
        connection: fixture === 'protection-error' ? 'unavailable' : 'available',
        capabilities: { cooperativeDelivery: true, directInterrupt: false, completionObservation: false,
            userActivityObservation: false, continuationControl: false },
        summary, reason: fixture === 'protection-error' ? 'bridgeUnavailable' : fixture === 'protection-requested' ? 'stoppingUnconfirmed'
            : fixture === 'protection-retrying' ? 'quotaReadRetrying' : fixture === 'protection-quota-error' ? 'quotaReadFailed' : fixture === 'protection-stale' ? 'quotaStale' : null,
        confirmedCount: 1, acceptedRequestCount: 1, confirmedInterruptionCount: 0, lastConfirmedAt: now,
        lastProbeAt: now, lastSuccessfulSampleAt: now, effectiveIntervalSeconds: 15, nextPollAt: now + 15,
        quotaHealth: quotaRecovery ? fixture === 'protection-stale' ? 'stale' : 'error' : 'success',
        reader: quotaRecovery ? { phase: fixture === 'protection-retrying' ? 'retrying' : fixture === 'protection-stale' ? 'stale' : 'error', attempt: 1,
            lastSuccess: { elapsedMs: 35000, attempt: 1 },
            lastFailure: { startedAt: now - 60, failure: { kind: 'timeout', stage: 'quota' } } } : null,
        targets: fixture === 'protection-requested' ? [{ threadId: '0199b0d2-4444-7000-8000-000000000004', periodId: 'fixture-period', active: true,
            hostId: 'local', turnId: null, phase: 'accepted', reason: 'stoppingUnconfirmed', requestedAt: now,
            acceptedAt: now, interruptedAt: null }] : [], resumePlans: [], resumeStateError: null };
    for (const callback of listeners.get('auto_pause_status_changed') ?? []) callback({ payload: protection });
};
export const invoke = async (cmd, args) => {
    if (cmd === 'is_installed_version') return true;
    if (cmd === 'get_notification_status') return 'enabled';
    if (cmd === 'get_auto_pause_status') return protection;
    if (cmd === 'get_tray_menu_generation') return 1;
    if (cmd === 'layout_tray_menu') { window.qaTrayLayout = args; return args.heightLogical; }
    return null;
};
export const listen = async (name, callback) => {
    if (!listeners.has(name)) listeners.set(name, new Set());
    listeners.get(name).add(callback);
    return () => listeners.get(name)?.delete(callback);
};
export class LogicalSize {
    constructor(width, height) { this.width = width; this.height = height; }
}
export const getCurrentWindow = () => ({ scaleFactor: async () => window.devicePixelRatio, onScaleChanged: async () => () => { }, setSize: async () => { }, setTitle: async () => { }, close: async () => { } });
export const addPluginListener = async () => ({ unregister: async () => { } });
