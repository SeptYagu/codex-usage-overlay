import { describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS, MonitorSnapshot } from './types';
import { mergeSettingsPatch, mergePendingPatches, providerPreference } from './providerSettings';
import { mergeMonitorSnapshot } from './hooks/useProviderMonitors';

describe('independent provider settings', () => {
  it('keeps each provider burn-rate algorithm isolated in providerPreferences', () => {
    const settings = {
      ...DEFAULT_SETTINGS,
      providerPreferences: {
        ...DEFAULT_SETTINGS.providerPreferences,
        codex: { ...DEFAULT_SETTINGS.providerPreferences.codex, fiveHourBurnRateAlgorithm: 'weighted' as const },
      },
    };
    expect(providerPreference(settings, 'codex').fiveHourBurnRateAlgorithm).toBe('weighted');
    expect(providerPreference(settings, 'claude').fiveHourBurnRateAlgorithm).toBe('legacy');
    expect(providerPreference(settings, 'dsh').fiveHourBurnRateAlgorithm).toBe('legacy');
    const edited = mergeSettingsPatch(settings, {
      providerPreferences: {
        claude: { fiveHourBurnRateAlgorithm: 'personalized' },
        dsh: { fiveHourBurnRateAlgorithm: 'weighted' },
      },
    });
    expect(providerPreference(edited, 'codex').fiveHourBurnRateAlgorithm).toBe('weighted');
    expect(providerPreference(edited, 'claude').fiveHourBurnRateAlgorithm).toBe('personalized');
    expect(providerPreference(edited, 'dsh').fiveHourBurnRateAlgorithm).toBe('weighted');
  });
  it('merges partial changes without dropping other providers or their thresholds', () => {
    const settings = {
      ...DEFAULT_SETTINGS,
      providerPreferences: {
        ...DEFAULT_SETTINGS.providerPreferences,
        codex: { ...DEFAULT_SETTINGS.providerPreferences.codex, protectionEnabled: true, fiveHourThresholdPercent: 7, weeklyThresholdPercent: 3 },
        claude: { ...DEFAULT_SETTINGS.providerPreferences.claude, protectionEnabled: false, fiveHourThresholdPercent: 4, weeklyThresholdPercent: 2 },
      },
    };
    const next = mergeSettingsPatch(settings, { providerPreferences: { claude: { protectionEnabled: true } } });
    expect(next.providerPreferences.codex).toEqual(settings.providerPreferences.codex);
    expect(next.providerPreferences.claude).toEqual({ ...settings.providerPreferences.claude, protectionEnabled: true });
    const displayedElsewhere = { ...next, provider: 'dsh' as const };
    expect(providerPreference(displayedElsewhere, 'codex').protectionEnabled).toBe(true);
    expect(providerPreference(displayedElsewhere, 'claude').protectionEnabled).toBe(true);
    const pending = mergePendingPatches({ providerPreferences: { codex: { fiveHourThresholdPercent: 6 } } },
      { providerPreferences: { claude: { weeklyThresholdPercent: 3 }, codex: { weeklyThresholdPercent: 4 } } });
    expect(pending.providerPreferences).toEqual({ codex: { fiveHourThresholdPercent: 6, weeklyThresholdPercent: 4 }, claude: { weeklyThresholdPercent: 3 } });
  });
});

describe('provider snapshot ordering', () => {
  const snapshot: MonitorSnapshot = { provider: 'claude', sourceKey: 'claude', sourceGeneration: 2,
    revision: 5, monitorEnabled: true, available: true, health: 'current', error: null, usage: null };
  it('retains background caches and rejects late bootstrap or invalidated-source updates', () => {
    const first = mergeMonitorSnapshot({}, snapshot);
    const background = mergeMonitorSnapshot(first, { ...snapshot, provider: 'codex', sourceKey: 'codex', revision: 6 });
    const antigravity = mergeMonitorSnapshot(background, { ...snapshot, provider: 'antigravity', sourceKey: 'antigravity-auto', revision: 7 });
    expect(antigravity.antigravity?.sourceKey).toBe('antigravity-auto');
    expect(background.claude).toBe(snapshot);
    expect(background.codex?.sourceKey).toBe('codex');
    expect(mergeMonitorSnapshot(background, { ...snapshot, revision: 4 })).toBe(background);
    expect(mergeMonitorSnapshot(background, { ...snapshot, revision: 99, sourceGeneration: 1 })).toBe(background);
    expect(mergeMonitorSnapshot(background, { ...snapshot, revision: 7, sourceGeneration: 3 }).claude?.sourceGeneration).toBe(3);
  });
});
