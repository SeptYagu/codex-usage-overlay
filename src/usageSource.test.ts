import { describe, expect, it } from 'vitest';
import { CodexUsage, DEFAULT_SETTINGS, SettingsEnvelope } from './types';
import { sourceBinding, usageMatchesSource, usageSourceScope } from './usageSource';

const sample = { fetchedAt: 123, sourceKey: 'dsh:one', sourceGeneration: 2 } as CodexUsage;
const settings = { ...DEFAULT_SETTINGS, provider: 'dsh' as const, dshBaseUrl: 'http://example.test', dshInstanceId: 'instance-1', dshAccountId: 'account-1', dshSessionIds: ['session-1'] };
const envelope: SettingsEnvelope = { revision: 1, settings, sourceKey: 'dsh:one', sourceGeneration: 2 };

describe('usage source isolation', () => {
  it('requires both exact source and generation regardless of timestamp', () => {
    const scope = usageSourceScope(envelope, settings, null);
    expect(usageMatchesSource(sample, scope)).toBe(true);
    for (const invalid of [
      { ...sample, sourceKey: 'local:one', fetchedAt: 999999 },
      { ...sample, sourceGeneration: 1, fetchedAt: 999999 },
      { ...sample, sourceGeneration: 3 },
      { ...sample, sourceKey: undefined, sourceGeneration: undefined },
    ]) expect(usageMatchesSource(invalid, scope)).toBe(false);
    expect(usageMatchesSource(sample, null)).toBe(false);
  });
  it('changes binding for instance, origin, account and sessions but not display or session ordering', () => {
    for (const patch of [{ dshBaseUrl: 'http://other.test' }, { dshInstanceId: 'other' }, { dshAccountId: 'other' }, { dshSessionIds: ['other'] }, { provider: 'codex' as const }]) {
      expect(sourceBinding({ ...settings, ...patch })).not.toBe(sourceBinding(settings));
    }
    expect(sourceBinding({ ...settings, showCredits: false })).toBe(sourceBinding(settings));
    expect(sourceBinding({ ...settings, dshSessionIds: ['b', 'a'] })).toBe(sourceBinding({ ...settings, dshSessionIds: ['a', 'b'] }));
  });
  it('permits untouched legacy local fixtures, never DSH or downgraded identity', () => {
    const legacy = { revision: 0, settings: DEFAULT_SETTINGS };
    const scope = usageSourceScope(legacy, DEFAULT_SETTINGS, null);
    expect(usageMatchesSource({ fetchedAt: 1 } as CodexUsage, scope)).toBe(true);
    expect(usageSourceScope({ revision: 0, settings }, settings, null).allowLegacy).toBe(false);
    expect(usageSourceScope(legacy, DEFAULT_SETTINGS, usageSourceScope(envelope, settings, null)).allowLegacy).toBe(false);
    expect(usageSourceScope({ ...legacy, sourceKey: 'incomplete' }, DEFAULT_SETTINGS, null).allowLegacy).toBe(false);
  });
});

import {usageIsOlder} from './usageSource';
it('orders same-sample algorithm projections by presentation revision rather than timestamp',()=>{
 const current={sourceKey:'codex',sourceGeneration:1,presentationRevision:10,fetchedAt:1000} as Parameters<typeof usageIsOlder>[0];
 expect(usageIsOlder({...current,presentationRevision:9},current)).toBe(true);
 expect(usageIsOlder({...current,presentationRevision:11},current)).toBe(false);
 expect(usageIsOlder({...current,presentationRevision:10,fetchedAt:999},current)).toBe(true);
});
