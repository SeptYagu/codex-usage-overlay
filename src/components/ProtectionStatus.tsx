import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { OverlaySettings } from '../types';
import { useProtectionStatus } from '../hooks/useProtectionStatus';

export function ProtectionStatusView({ settings, compact = false, onPatchSettings }: {
  settings: OverlaySettings;
  compact?: boolean;
  onPatchSettings?: (patch: Partial<OverlaySettings>) => void;
}) {
  const { t } = useTranslation();
  const { status, unavailable } = useProtectionStatus();
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const [actionError, setActionError] = useState(false);
  useEffect(() => {
    const timer = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => clearInterval(timer);
  }, []);
  const summary = !settings.autoPauseOnLowQuota ? 'off' : unavailable ? 'needsAttention' : status?.summary ?? 'checking';
  const label = t(`protectionSummaries.${summary}`);
  if (compact) {
    return <div role="status" aria-label={t('protectionStatus')} className="tray-menu-item text-xs opacity-80">{label}</div>;
  }
  const capable = !!status?.capabilities.directInterrupt && status.capabilities.completionObservation
    && status.capabilities.userActivityObservation && status.capabilities.continuationControl;
  const reason = unavailable ? 'statusUnavailable' : status?.reason;
  const cancel = async () => {
    setActionError(false);
    try { await invoke('cancel_resume_plans'); }
    catch { setActionError(true); }
  };
  return <div className="space-y-1 text-xs" data-protection-status>
    <div role="status" aria-label={t('protectionStatus')}
      className={summary === 'needsAttention' ? 'text-amber-700 dark:text-amber-300' : 'text-slate-600 dark:text-slate-300'}>
      {label}
      {reason && <span className="block text-[11px]">{t(`protectionReasons.${reason}`, { defaultValue: t('protectionReasons.statusUnavailable') })}</span>}
    </div>
    {settings.autoPauseOnLowQuota && status?.lastSuccessfulSampleAt != null && <p className="text-[11px] text-slate-500 dark:text-slate-400">
      {t('protectionSampleAge', { seconds: Math.max(0, now - status.lastSuccessfulSampleAt), interval: status.effectiveIntervalSeconds })}
    </p>}
    <details className="text-slate-500 dark:text-slate-400">
      <summary className="cursor-pointer">{t('protectionDetails')}</summary>
      <div className="space-y-1 pt-1 break-words">
        <p>{t('protectionCooperative')}</p>
        {!capable && <p>{t('protectionResumeUnavailable')}</p>}
        {onPatchSettings && <label className="flex items-center justify-between gap-3 py-1">
          <span>{t('autoResumeAfterQuotaReset')}</span>
          <input type="checkbox" aria-label={t('autoResumeAfterQuotaReset')}
            checked={settings.autoResumeAfterQuotaReset}
            disabled={!settings.autoResumeAfterQuotaReset && (!capable || !settings.autoPauseOnLowQuota)}
            onChange={event => {
              if (!event.target.checked || (capable && settings.autoPauseOnLowQuota)) {
                onPatchSettings({ autoResumeAfterQuotaReset: event.target.checked });
              }
            }}
            className="w-4 h-4 accent-cyan-500 disabled:cursor-not-allowed" />
        </label>}
        {status && <p>{t('protectionCounts', { accepted: status.acceptedRequestCount, stopped: status.confirmedInterruptionCount })}</p>}
        {status?.targets.map(target => <p key={target.periodId} title={target.threadId}>
          {t('protectionChat', { id: target.threadId.slice(0, 12) })}: {t(`protectionPhases.${target.phase}`, { defaultValue: target.phase })}
          {!target.active && <> · {t('protectionHistorical')}</>}
        </p>)}
        {status?.resumePlans.map(plan => <p key={plan.id}>
          {t('protectionChat', { id: plan.threadId })}: {t(`resumePhases.${plan.phase}`, { defaultValue: plan.phase })}
        </p>)}
        {status?.resumePlans.some(plan => ['needsReview', 'submissionUnknown'].includes(plan.phase))
          && <p>{t('protectionManualReview')}</p>}
        {status?.resumeStateError && <p role="alert">{t(`protectionReasons.${status.resumeStateError}`)}</p>}
        {status?.resumePlans.some(plan => ['waitingForQuota', 'ready', 'needsReview'].includes(plan.phase))
          && <button type="button" onClick={cancel} className="underline">{t('cancelResumePlans')}</button>}
        {actionError && <p role="alert">{t('protectionReasons.resumeStateWriteFailed')}</p>}
      </div>
    </details>
  </div>;
}
