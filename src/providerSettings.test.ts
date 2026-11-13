import { describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS, MonitorSnapshot } from './types';
import { mergeSettingsPatch, mergePendingPatches, settingsForProvider } from './providerSettings';
import { mergeMonitorSnapshot } from './hooks/useProviderMonitors';

describe('independent provider settings', () => {
  it('keeps a legacy top-level Codex algorithm out of other providers', () => {
    const settings = { ...DEFAULT_SETTINGS, fiveHourBurnRateAlgorithm: 'weighted' as const,
      providerPreferences: {
        codex: { monitorEnabled: true, protectionEnabled: false, fiveHourThresholdPercent: 5, weeklyThresholdPercent: 2 },
        claude: { monitorEnabled: true, protectionEnabled: false, fiveHourThresholdPercent: 5, weeklyThresholdPercent: 2 },
        dsh: { monitorEnabled: true, protectionEnabled: false, fiveHourThresholdPercent: 0, weeklyThresholdPercent: 0 },
      } };
    expect(settingsForProvider(settings, 'codex').fiveHourBurnRateAlgorithm).toBe('weighted');
    expect(settingsForProvider(settings, 'claude').fiveHourBurnRateAlgorithm).toBe('legacy');
    expect(settingsForProvider(settings, 'dsh').fiveHourBurnRateAlgorithm).toBe('legacy');
    const edited = mergeSettingsPatch(settings, { providerPreferences: {
      claude: { fiveHourBurnRateAlgorithm: 'personalized' }, dsh: { fiveHourBurnRateAlgorithm: 'weighted' },
    } });
    expect(settingsForProvider(edited, 'codex').fiveHourBurnRateAlgorithm).toBe('weighted');
    expect(settingsForProvider(edited, 'claude').fiveHourBurnRateAlgorithm).toBe('personalized');
    expect(settingsForProvider(edited, 'dsh').fiveHourBurnRateAlgorithm).toBe('weighted');
  });
  it('merges partial changes without dropping other providers or their thresholds', () => {
    const settings = { ...DEFAULT_SETTINGS, providerPreferences: {
      codex: { monitorEnabled: true, protectionEnabled: true, fiveHourThresholdPercent: 7, weeklyThresholdPercent: 3 },
      claude: { monitorEnabled: true, protectionEnabled: false, fiveHourThresholdPercent: 4, weeklyThresholdPercent: 2 },
    } };
    const next = mergeSettingsPatch(settings, { providerPreferences: { claude: { protectionEnabled: true } } });
    expect(next.providerPreferences?.codex).toEqual(settings.providerPreferences.codex);
    expect(next.providerPreferences?.claude).toEqual({ ...settings.providerPreferences.claude, protectionEnabled: true });
    const displayedElsewhere = { ...next, provider: 'dsh' as const };
    expect(settingsForProvider(displayedElsewhere, 'codex').autoPauseOnLowQuota).toBe(true);
    expect(settingsForProvider(displayedElsewhere, 'claude').autoPauseOnLowQuota).toBe(true);
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
