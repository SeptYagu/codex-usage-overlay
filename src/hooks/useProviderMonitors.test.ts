import { describe, expect, it } from 'vitest';
import { mergeMonitorSnapshot, usageIsOlder } from './useProviderMonitors';
import { CodexUsage, MonitorSnapshot } from '../types';

describe('usageIsOlder', () => {
  it('orders same-sample algorithm projections by presentation revision rather than timestamp', () => {
    const current = { sourceKey: 'codex', sourceGeneration: 1, presentationRevision: 10, fetchedAt: 1000 } as CodexUsage;
    expect(usageIsOlder({ ...current, presentationRevision: 9 }, current)).toBe(true);
    expect(usageIsOlder({ ...current, presentationRevision: 11 }, current)).toBe(false);
    expect(usageIsOlder({ ...current, presentationRevision: 10, fetchedAt: 999 }, current)).toBe(true);
  });
});

describe('mergeMonitorSnapshot', () => {
  const usage: CodexUsage = {
    fiveHourRemainingPercent: 60,
    fiveHourBurnRatePerHour: 10,
    weekRemainingPercent: 70,
    weekBurnRatePerHour: 1,
    creditsDisplay: '10',
    creditsBalance: '10',
    hasCredits: true,
    fiveHourResetsAt: 1_900_000_000,
    weekResetsAt: 1_900_500_000,
    fetchedAt: 1_800_000_000,
    sourceKey: 'codex',
    sourceGeneration: 1,
    presentationRevision: 5,
  };
  const snapshot: MonitorSnapshot = {
    provider: 'codex',
    sourceKey: 'codex',
    sourceGeneration: 1,
    revision: 5,
    monitorEnabled: true,
    available: true,
    health: 'current',
    error: null,
    usage,
  };

  it('rejects out-of-order revisions and older usage within the same source identity', () => {
    const base = mergeMonitorSnapshot({}, snapshot);
    expect(mergeMonitorSnapshot(base, { ...snapshot, revision: 4 })).toBe(base);
    const sameGenOlderUsage = mergeMonitorSnapshot(base, {
      ...snapshot,
      revision: 6,
      usage: { ...usage, presentationRevision: 4, fiveHourRemainingPercent: 10 },
    });
    expect(sameGenOlderUsage.codex?.usage?.fiveHourRemainingPercent).toBe(60);
    const nextGen = mergeMonitorSnapshot(base, {
      ...snapshot,
      sourceGeneration: 2,
      revision: 7,
      usage: { ...usage, sourceGeneration: 2, fiveHourRemainingPercent: 80 },
    });
    expect(nextGen.codex?.usage?.fiveHourRemainingPercent).toBe(80);
  });
});
