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
  // Optional only for legacy fixtures; production samples identify their source.
  sourceKey?: string;
  sourceGeneration?: number;
}

export type Provider = 'codex' | 'dsh' | 'claude';

export interface ProviderPreference {
  fiveHourBurnRateAlgorithm?: FiveHourBurnRateAlgorithm;
  carouselIncluded?: boolean;
  monitorEnabled: boolean;
  protectionEnabled: boolean;
  fiveHourThresholdPercent: number;
  weeklyThresholdPercent: number;
  refreshIntervalSeconds?: number;
  fiveHourResetNotification?: boolean;
  weeklyResetNotification?: boolean;
  fiveHourSoundMode?: 'windows' | 'custom';
  weeklySoundMode?: 'windows' | 'custom';
  fiveHourSoundPath?: string | null;
  weeklySoundPath?: string | null;
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
  fiveHourBurnRateAlgorithm?: FiveHourBurnRateAlgorithm;
  schemaVersion?: number;
  providerPreferences?: Partial<Record<Provider, ProviderPreference>>;
  provider: Provider;
  // Public instance origin only; pairing tokens never enter settings.
  dshBaseUrl: string;
  dshInstanceId: string;
  dshAccountId: string;
  dshSessionIds: string[];
  dshFiveHourThresholdPercent: number;
  dshWeeklyThresholdPercent: number;
  claudeLivePolling: boolean;
  showProviderLogo: boolean;
  overlayLayout: 'grouped' | 'stacks';
  scalePercent: number;
  backgroundTransparencyPercent: number;
  showCredits: boolean;
  showFiveHourQuota: boolean;
  showBurnRate: boolean;
  refreshIntervalSeconds: number;
  language: string;
  autoStart: boolean;
  autoCheckUpdates: boolean;
  autoInstallUpdates: boolean;
  includePrereleaseUpdates: boolean;
  fiveHourResetNotification: boolean;
  weeklyResetNotification: boolean;
  fiveHourSoundMode: 'windows' | 'custom';
  weeklySoundMode: 'windows' | 'custom';
  fiveHourSoundPath: string | null;
  weeklySoundPath: string | null;
  autoEdgeHide: boolean;
  mousePassthrough: boolean;
  showPercentageGrid: boolean;
  autoPauseOnLowQuota: boolean;
  autoResumeAfterQuotaReset: boolean;
  autoPauseFiveHourThresholdPercent: number;
  autoPauseWeeklyThresholdPercent: number;
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
  quotaCarouselEnabled: false, quotaCarouselIntervalSeconds: 8,
  provider: 'codex',
  dshBaseUrl: '',
  dshInstanceId: '',
  dshAccountId: '',
  dshSessionIds: [],
  dshFiveHourThresholdPercent: 0,
  dshWeeklyThresholdPercent: 0,
  claudeLivePolling: false,
  showProviderLogo: true,
  overlayLayout: 'grouped',
  scalePercent: 175,
  backgroundTransparencyPercent: 23,
  showCredits: true,
  showFiveHourQuota: true,
  showBurnRate: true,
  refreshIntervalSeconds: 60,
  language: 'auto',
  autoStart: true,
  autoCheckUpdates: true,
  autoInstallUpdates: false,
  includePrereleaseUpdates: false,
  fiveHourResetNotification: true,
  weeklyResetNotification: true,
  fiveHourSoundMode: 'windows',
  weeklySoundMode: 'windows',
  fiveHourSoundPath: null,
  weeklySoundPath: null,
  autoEdgeHide: false,
  mousePassthrough: false,
  showPercentageGrid: false,
  // Smart pause is opt-in: an upgrade must never silently arm the feature.
  autoPauseOnLowQuota: false,
  autoResumeAfterQuotaReset: false,
  autoPauseFiveHourThresholdPercent: 5,
  autoPauseWeeklyThresholdPercent: 2,
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
  autoResumeEnabled: boolean;
  connection: 'unknown' | 'available' | 'unavailable';
  capabilities: {
    cooperativeDelivery: boolean;
    directInterrupt: boolean;
    completionObservation: boolean;
    userActivityObservation: boolean;
    continuationControl: boolean;
  };
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
  resumePlans: Array<{
    id: string; threadId: string; phase: string; eligibleAfter: number; reason: string | null;
  }>;
  resumeStateError: string | null;
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

export interface DisplayState {
  revision: number; settingsRevision: number;
  effectiveProvider: Provider; manualProvider: Provider;
  sourceKey: string; sourceGeneration: number;
  carouselEnabled: boolean; intervalSeconds: number;
  candidates: Array<{provider: Provider; sourceKey: string; sourceGeneration: number}>;
  pauseReasons: string[]; status: 'off'|'waiting'|'paused'|'manualHold'|'rotating';
}
