import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { MonitorSnapshot, Provider } from '../types';

export function mergeMonitorSnapshot(current: Partial<Record<Provider, MonitorSnapshot>>, next: MonitorSnapshot) {
  if (!next || !['codex', 'claude', 'dsh'].includes(next.provider) || !Number.isSafeInteger(next.revision)
    || !Number.isSafeInteger(next.sourceGeneration)) return current;
  const old = current[next.provider];
  if (old && (next.sourceGeneration < old.sourceGeneration || next.revision <= old.revision)) return current;
  return { ...current, [next.provider]: next };
}

export function useProviderMonitors() {
  const [monitors, setMonitors] = useState<Partial<Record<Provider, MonitorSnapshot>>>({});
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    const apply = (next: MonitorSnapshot) => { if (active) setMonitors(current => mergeMonitorSnapshot(current, next)); };
    void listen<MonitorSnapshot>('monitor_updated', event => apply(event.payload)).then(async fn => {
      if (!active) { fn(); return; }
      unlisten = fn;
      try {
        const snapshots = await invoke<MonitorSnapshot[]>('get_monitor_snapshots');
        if (Array.isArray(snapshots)) snapshots.forEach(apply);
      } catch { /* Existing usage delivery remains available during startup. */ }
    }).catch(() => {});
    return () => { active = false; unlisten?.(); };
  }, []);
  return monitors;
}
