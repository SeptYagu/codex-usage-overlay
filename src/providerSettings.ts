import { defaultProviderPreference, OverlaySettings, Provider, ProviderPreference, SettingsPatch } from './types';

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
