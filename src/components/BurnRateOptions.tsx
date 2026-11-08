import { useTranslation } from 'react-i18next';
import { burnRatePresentation } from '../burnRatePresentation';
import { CodexUsage, FiveHourBurnRateAlgorithm, Provider, SettingsPatch } from '../types';

export function BurnRateOptions({provider,algorithm,usage,onPatchSettings}:{
  provider:Provider;algorithm:FiveHourBurnRateAlgorithm;usage?:CodexUsage|null;
  onPatchSettings:(patch:SettingsPatch)=>void;
}) {
  const {t}=useTranslation();
  return <section className="space-y-2 border border-slate-200 dark:border-slate-700 rounded-lg p-2.5" aria-label={t('burnAlgorithm')}>
    <label className="flex justify-between items-center gap-2 text-xs" htmlFor={`${provider}-burn-algorithm`}>
      <span>{t('burnAlgorithm')}</span>
      <select id={`${provider}-burn-algorithm`} value={algorithm}
        onChange={event=>onPatchSettings({fiveHourBurnRateAlgorithm:event.target.value as FiveHourBurnRateAlgorithm})}
        className="min-w-0 max-w-52 rounded border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-700 p-1">
        {(['legacy','weighted','personalized'] as const).map(value=><option key={value} value={value}>{t(`burnAlgorithm_${value}`)}</option>)}
      </select>
    </label>
    <p className="text-[11px] text-slate-500 dark:text-slate-400">{t('burnScope')}</p>
    <p role="status" className="text-[11px] text-slate-500 dark:text-slate-400">{burnRatePresentation(usage,t)}</p>
  </section>;
}
