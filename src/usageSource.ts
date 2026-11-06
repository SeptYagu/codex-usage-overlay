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

export function usageIsOlder(next: CodexUsage, current: CodexUsage): boolean {
  if (next.sourceKey === current.sourceKey && next.sourceGeneration === current.sourceGeneration
      && next.presentationRevision !== undefined && current.presentationRevision !== undefined
      && next.presentationRevision !== current.presentationRevision) return next.presentationRevision < current.presentationRevision;
  return next.fetchedAt < current.fetchedAt;
}

// Generations are ordered only inside a source identity, never across providers.
export function bufferUsage(inbox:Map<string,CodexUsage>,sample:CodexUsage) {
  const key=JSON.stringify([sample.sourceKey??null,sample.sourceGeneration??null]);
  const old=inbox.get(key);
  if(old&&usageIsOlder(sample,old))return;
  inbox.delete(key);inbox.set(key,sample);
  // Three live sources plus recent replaced generations; never unbounded history.
  while(inbox.size>12)inbox.delete(inbox.keys().next().value!);
}
export function takeMatchingUsage(inbox:Map<string,CodexUsage>,matches:(sample:CodexUsage)=>boolean):CodexUsage|null {
  let selected:CodexUsage|null=null;
  for(const [key,sample] of inbox){
    if(!matches(sample))continue;
    inbox.delete(key);
    if(!selected||!usageIsOlder(sample,selected))selected=sample;
  }
  return selected;
}
