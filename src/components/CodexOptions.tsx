import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { CodexUsage, FiveHourBurnRateAlgorithm, OverlaySettings, SettingsPatch } from '../types';
import { burnRatePresentation } from '../burnRatePresentation';
import { PauseMessageInput } from './PauseMessageInput';

export function CodexOptions({settings,usage,disabled,onPatchSettings}:{
  settings:OverlaySettings; usage?:CodexUsage|null;disabled:boolean;onPatchSettings:(patch:SettingsPatch)=>void;
}) {
  const {t}=useTranslation();
  const [templates,setTemplates]=useState<{pause:string;continuation:string}|null>(null);
  useEffect(()=>{let active=true;setTemplates(null);
    invoke<{pause:string;continuation:string}>('get_pause_templates',{language:settings.language})
      .then(value=>{if(active && value?.pause && value.continuation)setTemplates(value);}).catch(()=>{});
    return()=>{active=false;};
  },[settings.language]);
  const pause=templates?.pause??t('pauseLiteral');
  return <section className="space-y-3 border border-slate-200 dark:border-slate-700 rounded-lg p-2.5" aria-label={t('codexOptions')}>
    <label className="flex justify-between items-center gap-2 text-xs" htmlFor="codex-burn-algorithm">
      <span>{t('burnAlgorithm')}</span>
      <select id="codex-burn-algorithm" value={settings.fiveHourBurnRateAlgorithm??'legacy'}
        onChange={event=>onPatchSettings({fiveHourBurnRateAlgorithm:event.target.value as FiveHourBurnRateAlgorithm})}
        className="min-w-0 max-w-52 rounded border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-700 p-1">
        {(['legacy','weighted','personalized'] as const).map(algorithm=><option key={algorithm} value={algorithm}>{t(`burnAlgorithm_${algorithm}`)}</option>)}
      </select>
    </label>
    <p className="text-[11px] text-slate-500 dark:text-slate-400">{t('burnScope')}</p>
    <p role="status" className="text-[11px] text-slate-500 dark:text-slate-400">{burnRatePresentation(usage,t)}</p>
    <PauseMessageInput id="codex-five-hour-message" label={t('pauseFiveHourMessage')} value={settings.autoPauseFiveHourMessage??pause}
      defaultValue={pause} disabled={disabled} onSave={autoPauseFiveHourMessage=>onPatchSettings({autoPauseFiveHourMessage})} />
    <button type="button" disabled={disabled || !templates} onMouseDown={event=>event.preventDefault()}
      onClick={()=>{if(templates)onPatchSettings({autoPauseFiveHourMessage:templates.continuation});}}
      className="text-xs rounded border px-2 py-1 disabled:opacity-50">{t('pauseContinuationPreset')}</button>
    <p className="text-[11px] text-slate-500 dark:text-slate-400">{t('pauseContinuationHint')}</p>
    <PauseMessageInput id="codex-weekly-message" label={t('pauseWeeklyMessage')} value={settings.autoPauseWeeklyMessage??pause}
      defaultValue={pause} disabled={disabled} onSave={autoPauseWeeklyMessage=>onPatchSettings({autoPauseWeeklyMessage})} />
  </section>;
}
