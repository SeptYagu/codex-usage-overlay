export type FiveHourBurnRateAlgorithm = 'legacy' | 'weighted' | 'personalized';
export interface CodexUsage {
  presentationRevision?: number;
  settingsRevision?: number;
  requestedAlgorithm?: FiveHourBurnRateAlgorithm | null;
  effectiveAlgorithm?: FiveHourBurnRateAlgorithm | null;
  fallbackReason?: string | null;
  personalLearningStatus?: string | null;
  fiveHourRemainingPercent: number | null;
  fiveHourBurnRatePerHour: number | null;
  weekRemainingPercent: number | null;
  weekBurnRatePerHour: number | null;
  creditsDisplay: string;
  creditsBalance: string | null;
  hasCredits: boolean;
  fiveHourResetsAt: number | null;
  weekResetsAt: number | null;
  fetchedAt: number;
  /** Antigravity only: the Argon weekly (WTU) limit shown next to 5H and WK. */
  argonWeekRemainingPercent?: number | null;
  argonWeekResetsAt?: number | null;
  /**
   * Antigravity only: Argon is the projected pool, so `week*` is Argon's limit
   * (what protection acts on) and the WK reading shows `geminiWeek*` instead.
   */
  weeklyIsArgon?: boolean;
  geminiWeekRemainingPercent?: number | null;
  geminiWeekResetsAt?: number | null;
  // Optional only for legacy fixtures; production samples identify their source.
  sourceKey?: string;
  sourceGeneration?: number;
}

export type AntigravityPool = 'auto' | 'gemini' | 'argon' | '3p';
export type Provider = 'codex' | 'dsh' | 'claude' | 'antigravity';
export type HistoryRetention = '3m' | '6m' | '1y' | 'all';
export interface HistoryStorageInfo {
  totalBytes: number;
  firstAt: number | null;
  lastAt: number | null;
  deleteBytesEstimate: number;
  deleteRecords: number;
  deleteBefore: number | null;
  cleanupError: string | null;
}

export interface ProviderPreference {
  fiveHourBurnRateAlgorithm: FiveHourBurnRateAlgorithm;
  carouselIncluded: boolean;
  monitorEnabled: boolean;
  protectionEnabled: boolean;
  fiveHourThresholdPercent: number;
  weeklyThresholdPercent: number;
  refreshIntervalSeconds: number;
  fiveHourResetNotification: boolean;
  weeklyResetNotification: boolean;
  fiveHourSoundMode: 'windows' | 'custom';
  weeklySoundMode: 'windows' | 'custom';
  fiveHourSoundPath: string | null;
  weeklySoundPath: string | null;
}

export function defaultProviderPreference(provider: Provider): ProviderPreference {
  return {
    monitorEnabled: true,
    carouselIncluded: true,
    fiveHourBurnRateAlgorithm: 'legacy',
    protectionEnabled: false,
    fiveHourThresholdPercent: provider === 'dsh' ? 0 : 5,
    weeklyThresholdPercent: provider === 'dsh' ? 0 : 2,
    refreshIntervalSeconds: 60,
    fiveHourResetNotification: true,
    weeklyResetNotification: true,
    fiveHourSoundMode: 'windows',
    weeklySoundMode: 'windows',
    fiveHourSoundPath: null,
    weeklySoundPath: null,
  };
}

export type SettingsPatch = Partial<Omit<OverlaySettings, 'providerPreferences'>> & {
  providerPreferences?: Partial<Record<Provider, Partial<ProviderPreference>>>;
};

export interface MonitorSnapshot {
  provider: Provider;
  sourceKey: string;
  sourceGeneration: number;
  revision: number;
  monitorEnabled: boolean;
  available: boolean;
  health: 'disabled' | 'checking' | 'current' | 'stale' | 'failed' | 'needsSetup';
  error: string | null;
  usage: CodexUsage | null;
}

export interface OverlaySettings {
  quotaCarouselEnabled?: boolean;
  quotaCarouselIntervalSeconds?: number;
  autoPauseFiveHourMessage?: string;
  autoPauseWeeklyMessage?: string;
  schemaVersion?: number;
  providerPreferences: Record<Provider, ProviderPreference>;
  provider: Provider;
  // Public instance origin only; pairing tokens never enter settings.
  dshBaseUrl: string;
  dshInstanceId: string;
  dshAccountId: string;
  dshSessionIds: string[];
  claudeLivePolling: boolean;
  antigravityLivePolling?: boolean;
  antigravityPool?: AntigravityPool;
  showProviderLogo: boolean;
  overlayLayout: 'grouped' | 'stacks';
  scalePercent: number;
  backgroundTransparencyPercent: number;
  showCredits: boolean;
  showFiveHourQuota: boolean;
  showBurnRate: boolean;
  language: string;
  autoStart: boolean;
  autoCheckUpdates: boolean;
  autoInstallUpdates: boolean;
  includePrereleaseUpdates: boolean;
  autoEdgeHide: boolean;
  mousePassthrough: boolean;
  showPercentageGrid: boolean;
  recordUsageHistory: boolean;
  historyRetention: HistoryRetention;
  /** The History timeline shows used percent instead of remaining. */
  historyShowUsed: boolean;
}

export interface SettingsEnvelope {
  revision: number;
  settings: OverlaySettings;
  sourceKey?: string;
  sourceGeneration?: number;
}

export type QuotaKind = 'fiveHour' | 'week';
export type DockEdge = 'left' | 'right' | 'top' | 'bottom';

export interface DockStateInfo {
  docked: boolean;
  edge: DockEdge | null;
  expanded: boolean;
  hidden: boolean;
}

export const DEFAULT_SETTINGS: OverlaySettings = {
  schemaVersion: 5,
  providerPreferences: {
    codex: defaultProviderPreference('codex'),
    claude: defaultProviderPreference('claude'),
    dsh: defaultProviderPreference('dsh'),
    antigravity: defaultProviderPreference('antigravity'),
  },
  quotaCarouselEnabled: false, quotaCarouselIntervalSeconds: 8,
  provider: 'codex',
  dshBaseUrl: '',
  dshInstanceId: '',
  dshAccountId: '',
  dshSessionIds: [],
  claudeLivePolling: false,
  antigravityLivePolling: true,
  antigravityPool: 'auto',
  showProviderLogo: true,
  overlayLayout: 'grouped',
  scalePercent: 175,
  backgroundTransparencyPercent: 23,
  showCredits: true,
  showFiveHourQuota: true,
  showBurnRate: true,
  language: 'auto',
  autoStart: true,
  autoCheckUpdates: true,
  autoInstallUpdates: false,
  includePrereleaseUpdates: false,
  autoEdgeHide: false,
  mousePassthrough: false,
  showPercentageGrid: false,
  recordUsageHistory: true,
  historyRetention: 'all',
  historyShowUsed: false,
};

export interface UsageStatus {
  state: 'reading' | 'ok' | 'error';
  updatedAt: string;
  error?: string;
}

export interface ProtectionStatus {
  revision: number;
  enabled: boolean;
  provider: Provider;
  connection: 'unknown' | 'available' | 'unavailable';
  mode: 'cooperative' | 'direct';
  summary: 'off' | 'checking' | 'armed' | 'quotaLow' | 'pauseRequested' | 'needsAttention';
  reason: string | null;
  confirmedCount: number;
  acceptedRequestCount: number;
  confirmedInterruptionCount: number;
  lastConfirmedAt: number | null;
  lastProbeAt: number | null;
  lastSuccessfulSampleAt: number | null;
  effectiveIntervalSeconds: number;
  nextPollAt: number | null;
  quotaHealth: 'pending' | 'success' | 'error' | 'stale';
  reader?: {
    phase: string;
    attempt: number;
    lastSuccess: QuotaReadDiagnostic | null;
    lastFailure: QuotaReadDiagnostic | null;
  } | null;
  targets: Array<{
    periodId: string; active: boolean;
    threadId: string; hostId: string | null; turnId: string | null;
    phase: string; reason: string | null; requestedAt: number;
    acceptedAt: number | null; interruptedAt: number | null;
  }>;
  pausedThreads: string[];
}

export interface QuotaReadDiagnostic {
  startedAt: number;
  attempt: number;
  elapsedMs: number;
  initializeMs: number | null;
  quotaMs: number | null;
  success: boolean;
  failure: {
    stage: string;
    kind: string;
    rpcCode: number | null;
    httpStatus: number | null;
    retryAfterSeconds: number | null;
  } | null;
  cliVersion: string | null;
  stderr: { modelTimeouts: number; pluginFailures: number; otherLines: number; oversizedLines: number };
  cleanupFailed: boolean;
}

export interface DshBridgeInstance {
  id: string;
  label: string;
  origin: string;
}

export interface DshConnectionStatus {
  connected: boolean;
  origin: string | null;
  instanceId: string | null;
  activeAccountId: string | null;
  // Binding epoch, checked atomically by dsh_bind_targets before saving.
  connectionRevision: number;
  autoPhase: string;
  autoMessage?: string | null;
  // Monotonic cached-status revision, including across automatic reconnects.
  autoRevision: number;
  accounts: Array<{ id: string; label: string; active: boolean }>;
  sessions: Array<{ id: string; title: string; provider: string; busy: boolean }>;
  sessionsError?: boolean;
  error: string | null;
}

export interface ClaudeConnectionStatus {
  statusLine: boolean;
  hooks: boolean;
  bridgeAvailable: boolean;
  settingsPath: string | null;
  snapshotCapturedAt: number | null;
  claudeVersion: string | null;
  blockedCount: number;
  lastBlockedAt: number | null;
  error: string | null;
  readFailure: string | null;
  readHttpStatus: number | null;
}

export interface AntigravityConnectionStatus {
  statusLine: boolean;
  hooks: boolean;
  bridgeAvailable: boolean;
  cliAvailable: boolean;
  settingsPath: string | null;
  hooksPath: string | null;
  snapshotCapturedAt: number | null;
  antigravityVersion: string | null;
  activeModelId: string | null;
  activeModelName: string | null;
  detectedPool: AntigravityPool | null;
  effectivePool: AntigravityPool | null;
  blockedCount: number;
  lastBlockedAt: number | null;
  error: string | null;
  readFailure: string | null;
  readHttpStatus: number | null;
}

export interface DisplayState {
  revision: number; settingsRevision: number;
  effectiveProvider: Provider; manualProvider: Provider;
  sourceKey: string; sourceGeneration: number;
  carouselEnabled: boolean; intervalSeconds: number;
  candidates: Array<{provider: Provider; sourceKey: string; sourceGeneration: number}>;
  pauseReasons: string[]; status: 'off'|'waiting'|'paused'|'manualHold'|'rotating';
}
