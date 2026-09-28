export interface CodexUsage {
  fiveHourRemainingPercent: number | null;
  weekRemainingPercent: number | null;
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
  refreshIntervalSeconds: number;
  language: string;
  autoStart: boolean;
  autoCheckUpdates: boolean;
  fiveHourResetNotification: boolean;
  weeklyResetNotification: boolean;
  fiveHourSoundMode: 'windows' | 'custom';
  weeklySoundMode: 'windows' | 'custom';
  fiveHourSoundPath: string | null;
  weeklySoundPath: string | null;
  autoEdgeHide: boolean;
}

export interface SettingsEnvelope {
  revision: number;
  settings: OverlaySettings;
}

export type QuotaKind = 'fiveHour' | 'week';

export const DEFAULT_SETTINGS: OverlaySettings = {
  overlayLayout: 'grouped',
  scalePercent: 175,
  backgroundTransparencyPercent: 23,
  showCredits: true,
  refreshIntervalSeconds: 60,
  language: 'auto',
  autoStart: true,
  autoCheckUpdates: true,
  fiveHourResetNotification: true,
  weeklyResetNotification: true,
  fiveHourSoundMode: 'windows',
  weeklySoundMode: 'windows',
  fiveHourSoundPath: null,
  weeklySoundPath: null,
  autoEdgeHide: false,
};

export interface UsageStatus {
  state: 'reading' | 'ok' | 'error';
  updatedAt: string;
  error?: string;
}
