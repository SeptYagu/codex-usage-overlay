import { defaultProviderPreference, MonitorSnapshot, OverlaySettings, Provider, ProviderPreference, SettingsPatch } from './types';

export const providerLabelKey = {
  codex: 'providerCodex',
  claude: 'providerClaude',
  dsh: 'providerDsh',
  antigravity: 'providerAntigravity',
} as const satisfies Record<Provider, string>;

const providerOrder: Provider[] = ['codex', 'claude', 'dsh', 'antigravity'];

export function nextDisplayProvider(current: Provider, monitors: Partial<Record<Provider, MonitorSnapshot>>): Provider | undefined {
  const nextIndex = providerOrder.indexOf(current) + 1;
  const otherProviders = [...providerOrder.slice(nextIndex), ...providerOrder.slice(0, nextIndex - 1)];
  return otherProviders.find(provider => monitors[provider]?.available && monitors[provider]?.monitorEnabled);
}

export function providerPreference(settings: OverlaySettings, provider: Provider): ProviderPreference {
  return settings.providerPreferences?.[provider] ?? defaultProviderPreference(provider);
}

export function mergeSettingsPatch(settings: OverlaySettings, patch: SettingsPatch): OverlaySettings {
  const next = { ...settings, ...patch } as OverlaySettings;
  if (patch.providerPreferences) {
    next.providerPreferences = { ...settings.providerPreferences };
    for (const provider of Object.keys(patch.providerPreferences) as Provider[]) {
      next.providerPreferences[provider] = { ...providerPreference(settings, provider), ...patch.providerPreferences[provider] };
    }
  }
  return next;
}

export function mergePendingPatches(previous: SettingsPatch, next: SettingsPatch): SettingsPatch {
  const merged = { ...previous, ...next };
  if (next.providerPreferences) {
    merged.providerPreferences = { ...previous.providerPreferences };
    for (const provider of Object.keys(next.providerPreferences) as Provider[]) {
      merged.providerPreferences[provider] = { ...previous.providerPreferences?.[provider], ...next.providerPreferences[provider] };
    }
  }
  return merged;
}
