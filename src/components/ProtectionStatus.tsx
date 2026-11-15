import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { OverlaySettings, Provider } from '../types';
import { providerPreference } from '../providerSettings';
import { useProtectionStatus } from '../hooks/useProtectionStatus';

export function ProtectionStatusView({ settings, provider, compact = false }: {
  settings: OverlaySettings;
  provider?: Provider;
  compact?: boolean;
}) {
  const { t } = useTranslation();
  const targetProvider = provider ?? settings.provider;
  const pref = providerPreference(settings, targetProvider);
  const protectionEnabled = pref.monitorEnabled && pref.protectionEnabled;
  const { status: observedStatus, unavailable } = useProtectionStatus(targetProvider);
  // Never render a previous provider's local resume actions after switching.
  const status = observedStatus?.provider === targetProvider ? observedStatus : null;
  const dsh = targetProvider === 'dsh';
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const [resumingThreadId, setResumingThreadId] = useState<string | null>(null);
  const [resumeError, setResumeError] = useState<string | null>(null);
  useEffect(() => {
    const timer = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => clearInterval(timer);
  }, []);
  const summary = !protectionEnabled ? 'off' : unavailable ? 'needsAttention' : status?.summary ?? 'checking';
  const label = t(`protectionSummaries.${summary}`);
  if (compact) {
    return <div role="status" aria-label={t('protectionStatus')} className="tray-menu-item text-xs opacity-80">{label}</div>;
  }
  const reason = unavailable ? 'statusUnavailable' : status?.reason;
  const reader = status?.reader;
  const reading = reader && ['initializing', 'reading'].includes(reader.phase);
  const retrying = reader && (reader.phase === 'retrying' || (reading && reader.attempt > 1));
  const lastFailure = reader?.lastFailure;
  const cancel = async () => {
    setResumeError(null);
    try { await invoke('cancel_resume_plans', { provider: targetProvider }); }
    catch { setResumeError('generic'); }
  };
  const resume = async (threadId: string) => {
    setResumingThreadId(threadId);
    setResumeError(null);
    try {
      await invoke('resume_chat', { threadId, provider: targetProvider });
    } catch (err: unknown) {
      const errMsg = typeof err === 'string' ? err : err instanceof Error ? err.message : '';
      setResumeError(errMsg.includes('quotaStillLow') ? 'quotaStillLow' : errMsg.includes('quotaStale') ? 'quotaStale' : 'generic');
    } finally {
      setResumingThreadId(null);
    }
  };
  return <div className="space-y-1 text-xs" data-protection-status>
    <div role="status" aria-label={t('protectionStatus')}
      className={summary === 'needsAttention' ? 'text-amber-700 dark:text-amber-300' : 'text-slate-600 dark:text-slate-300'}>
      {label}
      {reason && <span className="block text-[11px]">{t(`protectionReasons.${reason}`, { defaultValue: t('protectionReasons.statusUnavailable') })}</span>}
    </div>
    {retrying ? <p>{t('quotaReadRetrying')}</p> : reading ? <p>{t('quotaReadReading')}</p>
      : reader?.phase === 'error' && status?.nextPollAt != null && <p>
        {t('quotaReadRetryCountdown', { seconds: Math.max(0, status.nextPollAt - now) })}
      </p>}
    {protectionEnabled && status?.lastSuccessfulSampleAt != null && <p className="text-[11px] text-slate-500 dark:text-slate-400">
      {t('protectionSampleAge', { seconds: Math.max(0, now - status.lastSuccessfulSampleAt), interval: status.effectiveIntervalSeconds })}
    </p>}
    <details className="text-slate-500 dark:text-slate-400">
      <summary className="cursor-pointer">{t('protectionDetails')}</summary>
      <div className="space-y-1 pt-1 break-words">
        {lastFailure?.failure && <p data-quota-last-failure>{t('quotaReadLastFailure', {
          reason: t(`quotaReadFailures.${lastFailure.failure.kind}`, { defaultValue: t('quotaReadFailures.rpc') }),
          stage: t(`quotaReadStages.${lastFailure.failure.stage}`, { defaultValue: t('quotaReadStages.refresh') }),
          time: new Date(lastFailure.startedAt * 1000).toLocaleTimeString(),
        })}</p>}
        {reader?.lastSuccess && <p>{t('quotaReadTiming', { milliseconds: reader.lastSuccess.elapsedMs, attempt: reader.lastSuccess.attempt })}</p>}
        <p>{t('protectionCooperative')}</p>
        {dsh && <p>{t('dshResumeUnavailable')}</p>}
        {status && <p>{t('protectionCounts', { accepted: status.acceptedRequestCount, stopped: dsh ? 0 : status.confirmedInterruptionCount })}</p>}
        {status?.targets.map(target => <p key={target.periodId} title={target.threadId}>
          {t('protectionChat', { id: target.threadId.slice(0, 12) })}: {t(`protectionPhases.${dsh && target.phase === 'interrupted' ? 'accepted' : target.phase}`, { defaultValue: target.phase })}
          {!target.active && <> · {t('protectionHistorical')}</>}
        </p>)}
        {!dsh && status?.pausedThreads.map(threadId => <div key={threadId} className="flex items-center justify-between gap-2 py-0.5">
          <span>{t('protectionChat', { id: threadId })}</span>
          <button
            type="button"
            onClick={() => resume(threadId)}
            disabled={resumingThreadId === threadId}
            className="px-2 py-0.5 text-[11px] font-medium bg-cyan-600 hover:bg-cyan-500 disabled:opacity-50 text-white rounded cursor-pointer transition-colors"
          >
            {resumingThreadId === threadId ? t('resumingChat') : t('resumeChat')}
          </button>
        </div>)}
        {!dsh && (status?.pausedThreads.length ?? 0) > 0
          && <button type="button" onClick={cancel} className="underline">{t('cancelResumePlans')}</button>}
        {resumeError && <p role="alert" className="text-amber-700 dark:text-amber-300">
          {resumeError === 'quotaStillLow' ? t('quotaStillLow') : resumeError === 'quotaStale' ? t('protectionReasons.quotaStale') : t('resumeFailed')}
        </p>}
      </div>
    </details>
  </div>;
}
