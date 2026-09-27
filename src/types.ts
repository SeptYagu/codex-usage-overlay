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
  scalePercent: number;
  backgroundTransparencyPercent: number;
  showCredits: boolean;
  refreshIntervalSeconds: number;
  language: string;
  autoStart: boolean;
}

export const DEFAULT_SETTINGS: OverlaySettings = {
  scalePercent: 175,
  backgroundTransparencyPercent: 23,
  showCredits: true,
  refreshIntervalSeconds: 60,
  language: 'auto',
  autoStart: true,
};

export interface UsageStatus {
  state: 'reading' | 'ok' | 'error';
  updatedAt: string;
  error?: string;
}
