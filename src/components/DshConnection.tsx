import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { DshBridgeInstance, DshConnectionStatus, OverlaySettings } from '../types';

interface DshConnectionViewProps {
  settings: OverlaySettings;
  onPatchSettings: (patch: Partial<OverlaySettings>, debounce?: boolean) => void;
}

// Show only a public origin even if a malformed/older backend sends a full URL.
function publicOrigin(value: string): string {
  try {
    const url = new URL(value);
    return ['http:', 'https:'].includes(url.protocol) ? url.origin : '';
  } catch { return ''; }
}

export function DshConnectionView({ settings, onPatchSettings }: DshConnectionViewProps) {
  const { t } = useTranslation();
  const [connectionUrl, setConnectionUrl] = useState('');
  const [status, setStatus] = useState<DshConnectionStatus | null>(null);
  const [bridges, setBridges] = useState<DshBridgeInstance[]>([]);
  const [bridgeId, setBridgeId] = useState('');
  const [discovering, setDiscovering] = useState(false);
  const [discoveryFailed, setDiscoveryFailed] = useState(false);
  const discoveryRequest = useRef(0);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const request = useRef(0);
  const mounted = useRef(false);
  const actionInFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (actionInFlight.current) return;
    const token = ++request.current;
    try {
      const next = await invoke<DshConnectionStatus>('dsh_connection_status');
      if (mounted.current && token === request.current) {
        setStatus(next);
        setFailed(false);
      }
    } catch {
      if (mounted.current && token === request.current) { setStatus(null); setFailed(true); }
    }
  }, []);

  const discover = useCallback(async () => {
    const token = ++discoveryRequest.current;
    setDiscovering(true);
    setDiscoveryFailed(false);
    try {
      const next = await invoke<DshBridgeInstance[]>('dsh_discover');
      if (mounted.current && token === discoveryRequest.current) {
        const instances = next ?? [];
        setBridges(instances);
        setBridgeId(current => instances.some(instance => instance.id === current) ? current : '');
      }
    } catch {
      if (mounted.current && token === discoveryRequest.current) {
        setBridges([]);
        setBridgeId('');
        setDiscoveryFailed(true);
      }
    } finally {
      if (mounted.current && token === discoveryRequest.current) setDiscovering(false);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    void discover();
    const timer = setInterval(() => { void refresh(); }, 10_000);
    return () => { mounted.current = false; request.current++; discoveryRequest.current++; clearInterval(timer); };
  }, [refresh, discover]);

  const run = async (mode: 'bridge' | 'manual' | 'disconnect') => {
    const connect = mode !== 'disconnect';
    if ((mode === 'bridge' && !bridgeId) || (mode === 'manual' && !connectionUrl.trim())) return;
    if (actionInFlight.current) return;
    actionInFlight.current = true;
    const token = ++request.current;
    setBusy(true);
    setFailed(false);
    try {
      // Pairing links are ephemeral secrets: not settings, logs, storage or URLs.
      const next = connect
        ? await invoke<DshConnectionStatus>('dsh_connect', mode === 'bridge' ? { bridgeId } : { connectionUrl: connectionUrl.trim() })
        : await invoke<DshConnectionStatus>('dsh_disconnect');
      if (mounted.current && token === request.current) {
        setStatus(next);
        setConnectionUrl('');
        setFailed(connect && !next?.connected);
      }
    } catch {
      // Backend errors can include the submitted URL. Never display or log them.
      if (mounted.current && token === request.current) { setStatus(null); setFailed(true); }
    } finally {
      actionInFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  };

  const connected = !!status?.connected;
  const accounts = status?.accounts ?? [];
  const sessions = (status?.sessions ?? []).filter(session => session.provider === 'codex');
  const missingSessions = settings.dshSessionIds.filter(id => !sessions.some(session => session.id === id));
  const origin = publicOrigin(settings.dshBaseUrl);
  const selectSession = (id: string, selected: boolean) => {
    onPatchSettings({ dshSessionIds: selected
      ? [...new Set([...settings.dshSessionIds, id])]
      : settings.dshSessionIds.filter(existing => existing !== id) });
  };

  return <section role="group" aria-label={t('dshConnection')} className="space-y-2 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5">
    <div className="flex items-center justify-between gap-2">
      <h3 className="text-sm font-semibold">{t('dshConnection')}</h3>
      <span role="status" className="text-xs">{t(connected ? 'dshConnected' : 'dshNotConnected')}</span>
    </div>
    {origin && <p className="text-xs break-all text-slate-500 dark:text-slate-400">{origin}</p>}
    {status?.instanceId && <p className="text-xs break-all text-slate-500 dark:text-slate-400">{status.instanceId}</p>}
    <p className="text-xs text-slate-500 dark:text-slate-400">{t('dshBridgeHint')}</p>
    <label htmlFor="dsh-bridge" className="block text-xs font-medium">{t('dshBridgeInstance')}</label>
    <select id="dsh-bridge" value={bridgeId} disabled={busy || discovering} onChange={event => setBridgeId(event.target.value)}
      className="w-full rounded-md border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 px-2 py-1 text-xs disabled:opacity-50">
      <option value="">{t('dshChooseInstance')}</option>
      {bridges.map(bridge => <option key={bridge.id} value={bridge.id}>{bridge.label || bridge.id} · {publicOrigin(bridge.origin)}</option>)}
    </select>
    {!discovering && !bridges.length && <p className="text-xs">{t('dshNoBridges')}</p>}
    {discoveryFailed && <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{t('dshDiscoveryFailed')}</p>}
    <div className="flex gap-2">
      <button type="button" disabled={busy || discovering} onClick={() => { void discover(); }} className="rounded-md border border-slate-300 dark:border-slate-600 px-2 py-1 text-xs disabled:opacity-50">{t(discovering ? 'dshWorking' : 'dshDetect')}</button>
      <button type="button" disabled={busy || discovering || !bridgeId} onClick={() => { void run('bridge'); }} className="rounded-md bg-cyan-600 px-2 py-1 text-xs text-white disabled:opacity-50">{t('dshConnect')}</button>
      {connected && <button type="button" disabled={busy} onClick={() => { void run('disconnect'); }} className="px-1 text-xs underline">{t('dshDisconnect')}</button>}
      <button type="button" disabled={busy} onClick={() => { void refresh(); }} className="px-1 text-xs underline">{t('dshRefresh')}</button>
    </div>
    <details className="space-y-1.5 text-xs">
    <summary className="cursor-pointer">{t('dshManualConnection')}</summary>
    <form onSubmit={event => { event.preventDefault(); void run('manual'); }} className="space-y-1.5">
      <label htmlFor="dsh-connection-url" className="block text-xs font-medium">{t('dshConnectionUrl')}</label>
      <input id="dsh-connection-url" type="password" value={connectionUrl} autoComplete="off" spellCheck={false}
        autoCapitalize="none" disabled={busy} onChange={event => setConnectionUrl(event.target.value)}
        className="w-full rounded-md border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 px-2 py-1 text-xs select-text" />
      <p className="text-xs text-slate-500 dark:text-slate-400">{t('dshConnectionHint')}</p>
      <div className="flex gap-2">
        <button type="submit" disabled={busy || !connectionUrl.trim()}
          className="rounded-md bg-cyan-600 px-2.5 py-1 text-xs text-white disabled:opacity-50">{t(busy ? 'dshWorking' : 'dshConnectManual')}</button>
      </div>
    </form>
    </details>
    {(failed || status?.error) && <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{t('dshConnectionFailed')}</p>}
    <label htmlFor="dsh-account" className="block text-xs font-medium">{t('dshAccount')}</label>
    <select id="dsh-account" value={settings.dshAccountId} disabled={busy || !connected}
      onChange={event => onPatchSettings({ dshAccountId: event.target.value, dshSessionIds: [] })}
      className="w-full rounded-md border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 px-2 py-1 text-xs disabled:opacity-50">
      <option value="">{t('dshChooseAccount')}</option>
      {settings.dshAccountId && !accounts.some(account => account.id === settings.dshAccountId)
        && <option value={settings.dshAccountId}>{t('dshUnavailableAccount')}</option>}
      {accounts.map(account => <option key={account.id} value={account.id}>
        {account.label || account.id}{account.active ? ` · ${t('dshAccountActive')}` : ''}
      </option>)}
    </select>
    <fieldset disabled={busy || !connected || !settings.dshAccountId} className="space-y-1 text-xs disabled:opacity-50">
      <legend className="font-medium">{t('dshSessions')}</legend>
      {sessions.map(session => <label key={session.id} className="flex items-center gap-2 py-0.5">
        <input type="checkbox" checked={settings.dshSessionIds.includes(session.id)}
          onChange={event => selectSession(session.id, event.target.checked)} className="accent-cyan-500" />
        <span className="break-words">{session.title || session.id} · {t(session.busy ? 'dshSessionBusy' : 'dshSessionIdle')}</span>
      </label>)}
      {missingSessions.map(id => <label key={id} className="flex items-center gap-2 py-0.5">
        <input type="checkbox" checked onChange={() => selectSession(id, false)} className="accent-cyan-500" />
        <span>{t('dshUnavailableSession', { id })}</span>
      </label>)}
      {!sessions.length && <p>{t('dshNoSessions')}</p>}
    </fieldset>
    <p className="text-xs text-slate-500 dark:text-slate-400">{t('dshBindingHint')}</p>
  </section>;
}
