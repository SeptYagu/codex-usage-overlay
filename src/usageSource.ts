import { CodexUsage, OverlaySettings, SettingsEnvelope } from './types';

export interface UsageSourceScope {
  binding: string;
  sourceKey?: string;
  sourceGeneration?: number;
  allowLegacy: boolean;
}

export function sourceBinding(settings: OverlaySettings): string {
  return JSON.stringify(settings.provider === 'dsh'
    ? [settings.provider, settings.dshBaseUrl, settings.dshInstanceId, settings.dshAccountId, [...settings.dshSessionIds].sort()]
    : [settings.provider]);
}

export function usageSourceScope(envelope: SettingsEnvelope, settings: OverlaySettings,
  previous: UsageSourceScope | null): UsageSourceScope {
  const binding = sourceBinding(settings);
  const validIdentity = typeof envelope.sourceKey === 'string' && envelope.sourceKey.length > 0
    && Number.isSafeInteger(envelope.sourceGeneration) && envelope.sourceGeneration! >= 0;
  return {
    binding,
    sourceKey: validIdentity ? envelope.sourceKey : undefined,
    sourceGeneration: validIdentity ? envelope.sourceGeneration : undefined,
    // Only untouched legacy local-source fixtures may omit identity. Never
    // downgrade after seeing identity, switching bindings, or selecting DSH.
    allowLegacy: !validIdentity && envelope.sourceKey === undefined && envelope.sourceGeneration === undefined
      && settings.provider !== 'dsh' && (!previous || (previous.allowLegacy && previous.binding === binding)),
  };
}

export function usageMatchesSource(usage: CodexUsage, scope: UsageSourceScope | null): boolean {
  if (!scope) return false;
  if (scope.sourceKey !== undefined && scope.sourceGeneration !== undefined) {
    return usage.sourceKey === scope.sourceKey && usage.sourceGeneration === scope.sourceGeneration;
  }
  return scope.allowLegacy && usage.sourceKey === undefined && usage.sourceGeneration === undefined;
}
