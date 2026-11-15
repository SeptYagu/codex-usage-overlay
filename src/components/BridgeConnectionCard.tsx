import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { OverlaySettings, Provider } from '../types';
import { providerPreference } from '../providerSettings';
import { useProtectionStatus } from '../hooks/useProtectionStatus';

const STATUS_POLL_MS = 10_000;

export interface BridgeStatusBase {
  statusLine: boolean;
  hooks: boolean;
  bridgeAvailable: boolean;
  snapshotCapturedAt: number | null;
  blockedCount: number;
  lastBlockedAt: number | null;
  error: string | null;
}

interface BridgeConnectionCardProps<S extends BridgeStatusBase> {
  settings: OverlaySettings;
  provider: Provider;
  statusCommand: string;
  connectCommand: string;
  disconnectCommand: string;
  liveChecked: boolean;
  onToggleLive: (checked: boolean) => void;
  titleKey: string;
  connectedKey: string;
  partlyConnectedKey: string;
  notConnectedKey: string;
  bridgeMissingKey: string;
  workingKey: string;
  connectKey: string;
  disconnectKey: string;
  livePollingKey: string;
  livePollingHintKey: string;
  protectionSummariesPrefix: string;
  blockedKey: string;
  renderConnectHint: (status: S) => React.ReactNode;
  renderLiveFailure: (status: S, formatFailure: (reason: string) => string) => string | null;
  renderSnapshotText: (status: S, formatTime: (seconds: number) => string) => string;
  noSnapshotKey: string;
  children?: (status: S | null, guardSummary: string) => React.ReactNode;
}

export function BridgeConnectionCard<S extends BridgeStatusBase>({
  settings,
  provider,
  statusCommand,
  connectCommand,
  disconnectCommand,
  liveChecked,
  onToggleLive,
  titleKey,
  connectedKey,
  partlyConnectedKey,
  notConnectedKey,
  bridgeMissingKey,
  workingKey,
  connectKey,
  disconnectKey,
  livePollingKey,
  livePollingHintKey,
  protectionSummariesPrefix,
  blockedKey,
  renderConnectHint,
  renderLiveFailure,
  renderSnapshotText,
  noSnapshotKey,
  children,
}: BridgeConnectionCardProps<S>) {
  const { t } = useTranslation();
  const pref = providerPreference(settings, provider);
  const protectionEnabled = pref.monitorEnabled && pref.protectionEnabled;
  const { status: protection } = useProtectionStatus(provider);
  const [status, setStatus] = useState<S | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<S>(statusCommand));
    } catch (error) {
      setActionError(String(error));
    }
  }, [statusCommand]);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => {
      void refresh();
    }, STATUS_POLL_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const run = async (command: string) => {
    setBusy(true);
    setActionError(null);
    try {
      setStatus(await invoke<S>(command));
    } catch (error) {
      setActionError(String(error));
    } finally {
      setBusy(false);
    }
  };

  const connected = Boolean(status?.statusLine && status.hooks);
  const partly = Boolean(status && !connected && (status.statusLine || status.hooks));
  const stateLabel = connected ? t(connectedKey) : partly ? t(partlyConnectedKey) : t(notConnectedKey);
  const formatTime = (seconds: number) => new Date(seconds * 1000).toLocaleString();
  const guardSummary = !protectionEnabled ? 'off' : protection?.summary ?? 'checking';
  const liveFailure = status
    ? renderLiveFailure(status, (reason) =>
        t(`quotaReadFailures.${reason}`, { defaultValue: t('quotaReadFailures.rpc') }))
    : null;

  return (
    <div className="space-y-1.5 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5" aria-label={t(titleKey)} role="group">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">{t(titleKey)}</h3>
        <span role="status" className={`text-xs ${connected ? 'text-emerald-700 dark:text-emerald-300' : 'text-amber-700 dark:text-amber-300'}`}>
          {stateLabel}
        </span>
      </div>
      {status && !status.bridgeAvailable && (
        <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{t(bridgeMissingKey)}</p>
      )}
      <div className="flex gap-2">
        <button
          type="button"
          disabled={busy || !status?.bridgeAvailable}
          onClick={() => { void run(connectCommand); }}
          className="rounded-md bg-cyan-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-cyan-700 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {busy ? t(workingKey) : t(connectKey)}
        </button>
        {(connected || partly) && (
          <button
            type="button"
            disabled={busy}
            onClick={() => { void run(disconnectCommand); }}
            className="rounded-md border border-slate-300 dark:border-slate-600 px-2.5 py-1 text-xs font-medium hover:bg-slate-200/60 dark:hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t(disconnectKey)}
          </button>
        )}
      </div>
      {status && renderConnectHint(status)}
      <label className="flex items-center justify-between gap-3 text-sm pt-0.5">
        <span className="font-medium">{t(livePollingKey)}</span>
        <input
          type="checkbox"
          aria-label={t(livePollingKey)}
          checked={liveChecked}
          onChange={(e) => onToggleLive(e.target.checked)}
          className="w-4 h-4 rounded text-cyan-600 accent-cyan-500 cursor-pointer"
        />
      </label>
      <p className="text-xs text-slate-500 dark:text-slate-400 leading-relaxed">{t(livePollingHintKey)}</p>
      {liveFailure && <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{liveFailure}</p>}
      {children?.(status, guardSummary)}
      <p className="text-xs text-slate-600 dark:text-slate-300">
        {status ? renderSnapshotText(status, formatTime) : t(noSnapshotKey)}
      </p>
      <p className={`text-xs ${guardSummary === 'needsAttention' ? 'text-amber-700 dark:text-amber-300' : 'text-slate-600 dark:text-slate-300'}`}>
        {t(`${protectionSummariesPrefix}.${guardSummary}`)}
      </p>
      {status && status.blockedCount > 0 && status.lastBlockedAt !== null && (
        <p className="text-xs text-slate-600 dark:text-slate-300">
          {t(blockedKey, { count: status.blockedCount, time: formatTime(status.lastBlockedAt) })}
        </p>
      )}
      {(actionError ?? status?.error) && (
        <p role="alert" className="text-xs text-amber-700 dark:text-amber-300 break-all">{actionError ?? status?.error}</p>
      )}
    </div>
  );
}
