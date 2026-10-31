import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { ClaudeConnectionStatus, OverlaySettings } from '../types';
import { useProtectionStatus } from '../hooks/useProtectionStatus';

interface ClaudeConnectionViewProps {
  settings: OverlaySettings;
  onPatchSettings: (patch: Partial<OverlaySettings>, debounce?: boolean) => void;
}

const STATUS_POLL_MS = 10_000;

/**
 * Connects the bundled `claude-usage-bridge` helper to Claude Code (statusline +
 * guard hooks) and shows what it last reported. Limits arrive while the terminal
 * CLI renders its statusline or, opted in, from live reading with the CLI login.
 */
export const ClaudeConnectionView: React.FC<ClaudeConnectionViewProps> = ({ settings, onPatchSettings }) => {
  const { t } = useTranslation();
  const { status: protection } = useProtectionStatus();
  const [status, setStatus] = useState<ClaudeConnectionStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<ClaudeConnectionStatus>('claude_connection_status'));
    } catch (error) {
      setActionError(String(error));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => { void refresh(); }, STATUS_POLL_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const run = async (command: 'claude_connect' | 'claude_disconnect') => {
    setBusy(true);
    setActionError(null);
    try {
      setStatus(await invoke<ClaudeConnectionStatus>(command));
    } catch (error) {
      setActionError(String(error));
    } finally {
      setBusy(false);
    }
  };

  const connected = Boolean(status?.statusLine && status.hooks);
  const partly = Boolean(status && !connected && (status.statusLine || status.hooks));
  const stateLabel = connected ? t('claudeConnected') : partly ? t('claudePartlyConnected') : t('claudeNotConnected');
  const time = (seconds: number) => new Date(seconds * 1000).toLocaleString();
  const guardSummary = !settings.autoPauseOnLowQuota ? 'off' : protection?.summary ?? 'checking';
  const live = settings.claudeLivePolling;
  const liveFailure = live && status?.readFailure && status.readFailure !== 'notConnected'
    ? status.readFailure === 'authentication'
      ? t('claudeLiveSignIn')
      : status.readHttpStatus
        ? t('claudeLiveHttp', { status: status.readHttpStatus })
        : t('claudeLiveFailed', { reason: t(`quotaReadFailures.${status.readFailure}`, { defaultValue: t('quotaReadFailures.rpc') }) })
    : null;

  return (
    <div className="space-y-1.5 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5" aria-label={t('claudeConnection')} role="group">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">{t('claudeConnection')}</h3>
        <span role="status" className={`text-xs ${connected ? 'text-emerald-700 dark:text-emerald-300' : 'text-amber-700 dark:text-amber-300'}`}>
          {stateLabel}
        </span>
      </div>
      {status && !status.bridgeAvailable && (
        <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{t('claudeBridgeMissing')}</p>
      )}
      <div className="flex gap-2">
        <button
          type="button"
          disabled={busy || !status?.bridgeAvailable}
          onClick={() => { void run('claude_connect'); }}
          className="rounded-md bg-cyan-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-cyan-700 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {busy ? t('claudeWorking') : t('claudeConnect')}
        </button>
        {(connected || partly) && (
          <button
            type="button"
            disabled={busy}
            onClick={() => { void run('claude_disconnect'); }}
            className="rounded-md border border-slate-300 dark:border-slate-600 px-2.5 py-1 text-xs font-medium hover:bg-slate-200/60 dark:hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t('claudeDisconnect')}
          </button>
        )}
      </div>
      {status?.settingsPath && (
        <p className="text-xs text-slate-500 dark:text-slate-400 leading-relaxed break-all">
          {t('claudeConnectHint', { path: status.settingsPath })}
        </p>
      )}
      <label className="flex items-center justify-between gap-3 text-sm">
        <span className="font-medium">{t('claudeLivePolling')}</span>
        <input
          type="checkbox"
          aria-label={t('claudeLivePolling')}
          checked={live}
          onChange={(e) => onPatchSettings({ claudeLivePolling: e.target.checked })}
          className="w-4 h-4 rounded text-cyan-600 accent-cyan-500 cursor-pointer"
        />
      </label>
      <p className="text-xs text-slate-500 dark:text-slate-400 leading-relaxed">{t('claudeLivePollingHint')}</p>
      {liveFailure && <p role="alert" className="text-xs text-amber-700 dark:text-amber-300">{liveFailure}</p>}
      <p className="text-xs text-slate-600 dark:text-slate-300">
        {status?.snapshotCapturedAt
          ? status.claudeVersion
            ? t('claudeSnapshotVersion', { version: status.claudeVersion, time: time(status.snapshotCapturedAt) })
            : t('claudeSnapshot', { time: time(status.snapshotCapturedAt) })
          : t('claudeNoSnapshot')}
      </p>
      <p className={`text-xs ${guardSummary === 'needsAttention' ? 'text-amber-700 dark:text-amber-300' : 'text-slate-600 dark:text-slate-300'}`}>
        {t(`claudeProtectionSummaries.${guardSummary}`)}
      </p>
      {status && status.blockedCount > 0 && status.lastBlockedAt !== null && (
        <p className="text-xs text-slate-600 dark:text-slate-300">
          {t('claudeBlocked', { count: status.blockedCount, time: time(status.lastBlockedAt) })}
        </p>
      )}
      {(actionError ?? status?.error) && (
        <p role="alert" className="text-xs text-amber-700 dark:text-amber-300 break-all">{actionError ?? status?.error}</p>
      )}
    </div>
  );
};
