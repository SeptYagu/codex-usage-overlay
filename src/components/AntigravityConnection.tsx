import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { AntigravityConnectionStatus, AntigravityPool, OverlaySettings } from '../types';
import { useProtectionStatus } from '../hooks/useProtectionStatus';

interface AntigravityConnectionViewProps {
  settings: OverlaySettings;
  onPatchSettings: (patch: Partial<OverlaySettings>, debounce?: boolean) => void;
}

const STATUS_POLL_MS = 10_000;
const POOLS: AntigravityPool[] = ['auto', 'gemini', 'argon', '3p'];

/**
 * Connects the bundled `antigravity-usage-bridge` helper to Antigravity CLI
 * (statusline + `PreInvocation` guard hook) and selects which quota pool is projected.
 */
export const AntigravityConnectionView: React.FC<AntigravityConnectionViewProps> = ({ settings, onPatchSettings }) => {
  const { t } = useTranslation();
  const { status: protection } = useProtectionStatus(settings.provider);
  const [status, setStatus] = useState<AntigravityConnectionStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<AntigravityConnectionStatus>('antigravity_connection_status'));
    } catch (error) {
      setActionError(String(error));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => { void refresh(); }, STATUS_POLL_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const run = async (command: 'antigravity_connect' | 'antigravity_disconnect') => {
    setBusy(true);
    setActionError(null);
    try {
      setStatus(await invoke<AntigravityConnectionStatus>(command));
    } catch (error) {
      setActionError(String(error));
    } finally {
      setBusy(false);
    }
  };

  const connected = Boolean(status?.statusLine && status.hooks);
  const partly = Boolean(status && !connected && (status.statusLine || status.hooks));
  const stateLabel = connected ? t('antigravityConnected') : partly ? t('antigravityPartlyConnected') : t('antigravityNotConnected');
  const time = (seconds: number) => new Date(seconds * 1000).toLocaleString();
  const guardSummary = !settings.autoPauseOnLowQuota ? 'off' : protection?.summary ?? 'checking';
  const pool = settings.antigravityPool ?? 'auto';
  const live = settings.antigravityLivePolling ?? true;
  const liveFailure = live && status
    ? !status.cliAvailable
      ? t('antigravityCliMissing')
      : status.readFailure && status.readFailure !== 'notConnected'
        ? status.readFailure === 'authentication'
          ? t('antigravityLiveSignIn')
          : status.readFailure === 'missingExecutable'
            ? t('antigravityCliMissing')
            : t('antigravityLiveFailed', {
                reason: t(`quotaReadFailures.${status.readFailure}`, { defaultValue: t('quotaReadFailures.rpc') }),
              })
        : null
    : null;
  const modelDisplay = status?.activeModelName ?? status?.activeModelId;

  return (
    <div className="space-y-1.5 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5" aria-label={t('antigravityConnection')} role="group">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">{t('antigravityConnection')}</h3>
        <span role="status" className={`text-xs ${connected ? 'text-emerald-700 dark:text-emerald-300' : 'text-amber-700 dark:text-amber-300'}`}>
          {stateLabel}
        </span>
      </div>
      {status && !status.bridgeAvailable && (
        <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{t('antigravityBridgeMissing')}</p>
      )}
      <div className="flex gap-2">
        <button
          type="button"
          disabled={busy || !status?.bridgeAvailable}
          onClick={() => { void run('antigravity_connect'); }}
          className="rounded-md bg-cyan-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-cyan-700 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {busy ? t('antigravityWorking') : t('antigravityConnect')}
        </button>
        {(connected || partly) && (
          <button
            type="button"
            disabled={busy}
            onClick={() => { void run('antigravity_disconnect'); }}
            className="rounded-md border border-slate-300 dark:border-slate-600 px-2.5 py-1 text-xs font-medium hover:bg-slate-200/60 dark:hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t('antigravityDisconnect')}
          </button>
        )}
      </div>
      {status?.settingsPath && status.hooksPath && (
        <p className="text-xs text-slate-500 dark:text-slate-400 leading-relaxed break-all">
          {t('antigravityConnectHint', { settingsPath: status.settingsPath, hooksPath: status.hooksPath })}
        </p>
      )}
      <label className="flex items-center justify-between gap-3 text-sm pt-0.5">
        <span className="font-medium">{t('antigravityLivePolling')}</span>
        <input
          type="checkbox"
          aria-label={t('antigravityLivePolling')}
          checked={live}
          onChange={(e) => onPatchSettings({ antigravityLivePolling: e.target.checked })}
          className="w-4 h-4 rounded text-cyan-600 accent-cyan-500 cursor-pointer"
        />
      </label>
      <p className="text-xs text-slate-500 dark:text-slate-400 leading-relaxed">{t('antigravityLivePollingHint')}</p>
      {liveFailure && <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{liveFailure}</p>}
      <div className="flex items-center justify-between gap-2 pt-0.5">
        <label htmlFor="antigravity-pool-select" className="text-sm font-medium">{t('antigravityPoolLabel')}</label>
        <select
          id="antigravity-pool-select"
          aria-label={t('antigravityPoolLabel')}
          value={pool}
          onChange={(e) => onPatchSettings({ antigravityPool: e.target.value as AntigravityPool })}
          className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500"
        >
          {POOLS.map((item) => (
            <option key={item} value={item}>{t(`antigravityPool_${item}`)}</option>
          ))}
        </select>
      </div>
      {modelDisplay && status?.effectivePool && (
        <p className="text-xs text-slate-500 dark:text-slate-400">
          {t('antigravityDetectedModel', {
            model: modelDisplay,
            pool: t(`antigravityPool_${status.effectivePool}`),
          })}
        </p>
      )}
      <p className="text-xs text-slate-600 dark:text-slate-300">
        {status?.snapshotCapturedAt
          ? status.antigravityVersion
            ? t('antigravitySnapshotVersion', { version: status.antigravityVersion, time: time(status.snapshotCapturedAt) })
            : t('antigravitySnapshot', { time: time(status.snapshotCapturedAt) })
          : t('antigravityNoSnapshot')}
      </p>
      <p className={`text-xs ${guardSummary === 'needsAttention' ? 'text-amber-700 dark:text-amber-300' : 'text-slate-600 dark:text-slate-300'}`}>
        {t(`antigravityProtectionSummaries.${guardSummary}`)}
      </p>
      {status && status.blockedCount > 0 && status.lastBlockedAt !== null && (
        <p className="text-xs text-slate-600 dark:text-slate-300">
          {t('antigravityBlocked', { count: status.blockedCount, time: time(status.lastBlockedAt) })}
        </p>
      )}
      {(actionError ?? status?.error) && (
        <p role="alert" className="text-xs text-amber-700 dark:text-amber-300 break-all">{actionError ?? status?.error}</p>
      )}
    </div>
  );
};
