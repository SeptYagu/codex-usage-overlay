import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { OverlaySettings, SettingsPatch } from '../types';
import { PauseMessageInput } from './PauseMessageInput';

export function CodexOptions({settings,disabled,onPatchSettings}:{
  settings:OverlaySettings;disabled:boolean;onPatchSettings:(patch:SettingsPatch)=>void;
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
