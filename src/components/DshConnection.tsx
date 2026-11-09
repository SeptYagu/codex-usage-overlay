import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { DshConnectionStatus, OverlaySettings, SettingsEnvelope } from '../types';

interface DshConnectionViewProps {
  settings: OverlaySettings;
  onPatchSettings: (patch: Partial<OverlaySettings>, debounce?: boolean) => void;
}

type Action = 'retry' | 'setup' | 'manual' | 'disconnect' | 'bind';

// Show only a public origin even if a malformed/older backend sends a full URL.
function publicOrigin(value: string): string {
  try {
    const url = new URL(value);
    return ['http:', 'https:'].includes(url.protocol) ? url.origin : '';
  } catch { return ''; }
}

const phaseLabels: Record<string, string> = {
  inactive: 'dshNotConnected',
  discovering: 'dshAutoSearching',
  connecting: 'dshAutoSearching',
  'setting-up': 'dshAutoSettingUp',
  'restart-required': 'dshAutoWaitingReload',
  ambiguous: 'dshAutoAmbiguous',
  suspended: 'dshAutoSuspended',
  error: 'dshAutoFailed',
};

export function DshConnectionView({ settings }: DshConnectionViewProps) {
  const { t } = useTranslation();
  const [connectionUrl, setConnectionUrl] = useState('');
  const [advanced, setAdvanced] = useState(false);
  const [status, setStatus] = useState<DshConnectionStatus | null>(null);
  const [busy, setBusy] = useState<Action | null>(null);
  const [failed, setFailed] = useState<'connection' | 'binding' | null>(null);
  const [invalidBinding, setInvalidBinding] = useState<string | null>(null);
  const bindingSignature = JSON.stringify([settings.dshInstanceId, settings.dshAccountId, settings.dshSessionIds]);
  const currentBinding = useRef(bindingSignature);
  currentBinding.current = bindingSignature;
  const connectionEpoch = useRef<string | null>(null);
  const liveStatus = useRef<DshConnectionStatus | null>(null);
  const lifecycle = useRef(0);
  const operation = useRef(0);
  const request = useRef(0);
  const accepted = useRef({ revision: -1, request: -1 });
  const actionInFlight = useRef(false);

  const acceptStatus = useCallback((next: DshConnectionStatus, epoch: number, action: number, token: number) => {
    if (epoch !== lifecycle.current || action !== operation.current) return;
    // Status is a cache projection. Never resurrect an earlier source/phase from
    // a slow response, including a newer request carrying an older revision.
    if (!next || !Number.isSafeInteger(next.autoRevision)
      || next.autoRevision < accepted.current.revision
      || (next.autoRevision === accepted.current.revision && token < accepted.current.request)) return;
    accepted.current = { revision: next.autoRevision, request: token };
    const nextEpoch = next.connected ? JSON.stringify([next.instanceId, next.connectionRevision]) : null;
    if (!next.connected || (connectionEpoch.current !== null && connectionEpoch.current !== nextEpoch)) {
      // A settings broadcast can trail the new status. Old selected IDs must
      // not look authorized in a reconnected instance, even if IDs are reused.
      setInvalidBinding(currentBinding.current);
    }
    connectionEpoch.current = nextEpoch;
    liveStatus.current = next;
    setStatus(next);
    setFailed(current => current === 'binding' ? current : null);
  }, []);

  const refresh = useCallback(async () => {
    const epoch = lifecycle.current;
    const action = operation.current;
    const token = ++request.current;
    try {
      const next = await invoke<DshConnectionStatus>('dsh_connection_status');
      acceptStatus(next, epoch, action, token);
    } catch {
      if (epoch === lifecycle.current && action === operation.current
        && token === request.current && token >= accepted.current.request) {
        liveStatus.current = null;
        setStatus(null);
        setFailed('connection');
      }
    }
  }, [acceptStatus]);

  useEffect(() => {
    lifecycle.current++;
    // The Rust singleton owns discovery, setup and reconnecting. Mount only
    // reads its cache; opening settings must never start a second worker.
    void refresh();
    const timer = setInterval(() => { void refresh(); }, 3_000);
    return () => {
      lifecycle.current++;
      operation.current++;
      actionInFlight.current = false;
      clearInterval(timer);
    };
  }, [refresh]);

  const run = async (mode: Exclude<Action, 'bind'>) => {
    if (actionInFlight.current || (mode === 'manual' && !connectionUrl.trim())) return;
    actionInFlight.current = true;
    const epoch = lifecycle.current;
    const action = ++operation.current;
    const token = ++request.current;
    liveStatus.current = null;
    setStatus(null);
    setBusy(mode);
    setFailed(null);
    try {
      if (mode === 'retry' || mode === 'setup') {
        // No arguments: these commands idempotently wake the native worker.
        await invoke<void>(mode === 'retry' ? 'dsh_auto_retry' : 'dsh_auto_setup');
      } else {
        // Pairing links remain ephemeral, never settings, logs or storage.
        const next = await invoke<DshConnectionStatus>(mode === 'manual' ? 'dsh_connect' : 'dsh_disconnect',
          mode === 'manual' ? { connectionUrl: connectionUrl.trim() } : undefined);
        acceptStatus(next, epoch, action, token);
      }
      if (epoch !== lifecycle.current || action !== operation.current) return;
      setConnectionUrl('');
      await refresh();
    } catch {
      // Even native error messages can contain the submitted link. Do not show
      // raw error/autoMessage text, tokens, paths or backend diagnostics.
      if (epoch === lifecycle.current && action === operation.current) {
        liveStatus.current = null;
        setStatus(null);
        setFailed('connection');
      }
    } finally {
      if (epoch === lifecycle.current && action === operation.current) {
        actionInFlight.current = false;
        setBusy(null);
      }
    }
  };

  const connected = !!status?.connected;
  const bindingMatches = connected && !!status?.instanceId && settings.dshInstanceId === status.instanceId;
  const accountId = bindingMatches ? settings.dshAccountId : '';
  const selectedSessions = bindingMatches && accountId && invalidBinding !== bindingSignature ? settings.dshSessionIds : [];
  const accounts = status?.accounts ?? [];
  const sessions = (status?.sessions ?? []).filter(session => session.provider === 'openai-codex');
  const missingSessions = selectedSessions.filter(id => !sessions.some(session => session.id === id));
  const origin = publicOrigin(status?.origin ?? '');
  const canBind = connected && !!status?.instanceId && Number.isSafeInteger(status.connectionRevision)
    && status.connectionRevision >= 0 && ['ready', 'connected-unbound'].includes(status.autoPhase)
    && !status.error && !status.sessionsError && !busy && failed !== 'connection';

  const bind = async (nextAccount: string, sessionIds: string[]) => {
    const snapshot = status;
    if (!canBind || !snapshot || liveStatus.current !== snapshot || actionInFlight.current) return;
    actionInFlight.current = true;
    const epoch = lifecycle.current;
    const action = ++operation.current;
    setBusy('bind');
    setFailed(null);
    try {
      // Never replay IDs through patch_settings: only this command atomically
      // checks the displayed connection epoch. App consumes settings_updated.
      await invoke<SettingsEnvelope>('dsh_bind_targets', {
        accountId: nextAccount,
        sessionIds,
        expectedInstanceId: snapshot.instanceId,
        expectedConnectionRevision: snapshot.connectionRevision,
      });
      if (epoch === lifecycle.current && action === operation.current
        && liveStatus.current?.instanceId === snapshot.instanceId
        && liveStatus.current.connectionRevision === snapshot.connectionRevision) {
        // A deliberate rebind of exactly the same IDs is valid only after the
        // guarded native command confirms it, never because settings lagged.
        const confirmed = JSON.stringify([snapshot.instanceId, nextAccount, sessionIds]);
        setInvalidBinding(current => current === confirmed ? null : current);
      }
    } catch {
      if (epoch === lifecycle.current && action === operation.current) setFailed('binding');
    } finally {
      if (epoch === lifecycle.current && action === operation.current) {
        actionInFlight.current = false;
        setBusy(null);
        // Refresh even on rejection: the source/account may have changed.
        void refresh();
      }
    }
  };

  const connectionFailed = failed === 'connection' || !!status?.error || status?.autoPhase === 'error';
  const label = connectionFailed ? 'dshAutoFailed' : connected
    ? status?.sessionsError ? 'dshConnected' : selectedSessions.length ? 'dshConnectedBound' : 'dshConnectedReadOnly'
    : phaseLabels[status?.autoPhase ?? ''] ?? 'dshAutoSearching';

  return <section role="group" aria-label={t('dshConnection')} className="space-y-2 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5">
    <h3 className="text-sm font-semibold">{t('dshConnection')}</h3>
    <p role="status" className="text-xs">{t(label)}</p>
    <p className="text-xs text-slate-500 dark:text-slate-400">{t('dshAutoHint')}</p>
    {status?.autoPhase === 'restart-required' && <p className="text-xs text-amber-700 dark:text-amber-300">{t('dshRestartHint')}</p>}
    {(!connected || connectionFailed) && <div className="flex flex-wrap gap-2">
      <button type="button" disabled={!!busy} onClick={() => { void run('retry'); }} className="rounded-md border border-slate-300 dark:border-slate-600 px-2 py-1 text-xs disabled:opacity-50">{t(busy === 'retry' ? 'dshWorking' : 'dshRetry')}</button>
      <button type="button" disabled={!!busy} onClick={() => { void run('setup'); }} className="rounded-md bg-cyan-600 px-2 py-1 text-xs text-white disabled:opacity-50">{t(busy === 'setup' ? 'dshWorking' : 'dshAutoSetup')}</button>
    </div>}
    {connectionFailed && <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{t('dshConnectionFailed')}</p>}
    {connected && status?.sessionsError && <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{t('dshSessionsUnavailable')}</p>}
    {failed === 'binding' && <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{t('dshBindingFailed')}</p>}
    {origin && <p className="text-xs break-all text-slate-500 dark:text-slate-400">{origin}</p>}
    <label htmlFor="dsh-account" className="block text-xs font-medium">{t('dshAccount')}</label>
    <select id="dsh-account" value={accountId} disabled={!canBind}
      onChange={event => { void bind(event.target.value, []); }}
      className="w-full rounded-md border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 px-2 py-1 text-xs disabled:opacity-50">
      <option value="">{t('dshChooseAccount')}</option>
      {accountId && !accounts.some(account => account.id === accountId)
        && <option value={accountId}>{t('dshUnavailableAccount')}</option>}
      {accounts.map(account => <option key={account.id} value={account.id}>
        {account.label || account.id}{account.active ? ` · ${t('dshAccountActive')}` : ''}
      </option>)}
    </select>
    <fieldset disabled={!canBind || !accountId} className="space-y-1 text-xs disabled:opacity-50">
      <legend className="font-medium">{t('dshSessions')}</legend>
      {sessions.map(session => <label key={session.id} className="flex items-center gap-2 py-0.5">
        <input type="checkbox" checked={selectedSessions.includes(session.id)}
          onChange={event => { void bind(accountId, event.target.checked
            ? [...new Set([...selectedSessions, session.id])]
            : selectedSessions.filter(id => id !== session.id)); }} className="accent-cyan-500" />
        <span className="break-words">{session.title || session.id} · {t(session.busy ? 'dshSessionBusy' : 'dshSessionIdle')}</span>
      </label>)}
      {missingSessions.map(id => <label key={id} className="flex items-center gap-2 py-0.5">
        <input type="checkbox" checked onChange={() => { void bind(accountId, selectedSessions.filter(existing => existing !== id)); }} className="accent-cyan-500" />
        <span>{t('dshUnavailableSession', { id })}</span>
      </label>)}
      {!sessions.length && !status?.sessionsError && <p>{t('dshNoSessions')}</p>}
    </fieldset>
    <p className="text-xs text-slate-500 dark:text-slate-400">{t('dshBindingHint')}</p>
    <details onToggle={event => {
      setAdvanced(event.currentTarget.open);
      if (!event.currentTarget.open) setConnectionUrl('');
    }} className="space-y-1.5 text-xs">
      <summary className="cursor-pointer">{t('dshManualConnection')}</summary>
      {advanced && <>
        <form onSubmit={event => { event.preventDefault(); void run('manual'); }} className="space-y-1.5">
          <label htmlFor="dsh-connection-url" className="block text-xs font-medium">{t('dshConnectionUrl')}</label>
          <input id="dsh-connection-url" type="password" value={connectionUrl} autoComplete="off" spellCheck={false}
            autoCapitalize="none" disabled={!!busy} onChange={event => setConnectionUrl(event.target.value)}
            className="w-full rounded-md border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 px-2 py-1 text-xs select-text" />
          <p className="text-xs text-slate-500 dark:text-slate-400">{t('dshConnectionHint')}</p>
          <button type="submit" disabled={!!busy || !connectionUrl.trim()}
            className="rounded-md bg-cyan-600 px-2.5 py-1 text-xs text-white disabled:opacity-50">{t(busy === 'manual' ? 'dshWorking' : 'dshConnectManual')}</button>
        </form>
        <div className="flex gap-2">
          {connected && <button type="button" disabled={!!busy} onClick={() => { void run('disconnect'); }} className="px-1 text-xs underline">{t('dshDisconnect')}</button>}
          <button type="button" disabled={!!busy} onClick={() => { void refresh(); }} className="px-1 text-xs underline">{t('dshRefresh')}</button>
        </div>
      </>}
    </details>
  </section>;
}
