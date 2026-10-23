export interface CodexUsage {
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
}

export interface OverlaySettings {
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
