import type { ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { languageOptions } from '../i18n';
import type { DisplayState, OverlaySettings, SettingsPatch } from '../types';
import { OverlayPreview } from './OverlayView';

interface CommonSettingsProps {
  settings: OverlaySettings;
  display?: DisplayState;
  isInstalled: boolean;
  notificationStatus: string;
  onPatchSettings: (patch: SettingsPatch, debounce?: boolean) => void;
}

const hintClass = 'text-xs leading-relaxed text-slate-600 dark:text-slate-400';
const selectClass = 'max-w-full rounded-md border border-slate-300 bg-white px-2 py-1 text-xs dark:border-slate-600 dark:bg-slate-700';
const rowClass = 'flex flex-wrap items-center justify-between gap-2 rounded-lg bg-slate-100 px-2 py-1.5 text-sm font-medium dark:bg-slate-800/70';

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="common-settings-section">
      <h3 id={id} className="mb-3 text-sm font-semibold">{title}</h3>
      <div className="space-y-2.5">{children}</div>
    </section>
  );
}

function Toggle({ label, checked, onChange, hintId, hint }: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  hintId?: string;
  hint?: string;
}) {
  return (
    <div>
      <label className={`${rowClass} cursor-pointer hover:bg-slate-200/70 dark:hover:bg-slate-700`}>
        <span>{label}</span>
        <input type="checkbox" className="h-4 w-4 shrink-0 accent-cyan-600"
          checked={checked} aria-describedby={hintId} onChange={event => onChange(event.target.checked)} />
      </label>
      {hint && <p id={hintId} className={`mt-1 px-2 ${hintClass}`}>{hint}</p>}
    </div>
  );
}

function Slider({ id, label, value, valueLabel, min, max, step, marks, hint, hintId, disabled, onChange }: {
  id: string;
  label: string;
  value: number;
  valueLabel: string;
  min: number;
  max: number;
  step: number;
  marks: [string, string, string];
  hint?: string;
  hintId?: string;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <div className="space-y-1 px-2">
      <div className="flex flex-wrap items-baseline justify-between gap-x-2">
        <label htmlFor={id} className="text-sm font-medium">{label}</label>
        <span className="font-mono text-xs font-medium text-cyan-800 dark:text-cyan-300">{valueLabel}</span>
      </div>
      <input id={id} type="range" min={min} max={max} step={step} value={value} disabled={disabled}
        aria-describedby={hintId} aria-valuetext={valueLabel} onChange={event => onChange(event.target.valueAsNumber)}
        className="block h-5 w-full cursor-pointer accent-cyan-600 disabled:cursor-default disabled:opacity-50" />
      <div className="flex flex-wrap justify-between gap-x-2 text-[11px] text-slate-600 dark:text-slate-300">
        {marks.map((mark, index) => <span key={index}>{mark}</span>)}
      </div>
      {hint && <p id={hintId} className={hintClass}>{hint}</p>}
    </div>
  );
}

export function CommonSettings({ settings, display, isInstalled, notificationStatus, onPatchSettings }: CommonSettingsProps) {
  const { t } = useTranslation();
  const interval = settings.quotaCarouselIntervalSeconds ?? 8;
  const rotationEnabled = settings.quotaCarouselEnabled ?? false;

  return (
    <div className="common-settings" data-common-settings>
      <div className="common-settings-appearance">
        <Section id="appearance-settings" title={t('settingsAppearance')}>
          <label className={rowClass}>
            <span>{t('language')}</span>
            <select value={settings.language} className={selectClass}
              onChange={event => onPatchSettings({ language: event.target.value }, false)}>
              {languageOptions.map(([locale, label]) => <option key={locale} value={locale}>{t(label)}</option>)}
            </select>
          </label>
          <div>
            <label className={rowClass}>
              <span>{t('overlayLayout')}</span>
              <select id="overlay-layout" value={settings.overlayLayout} className={selectClass} aria-describedby="overlay-layout-hint"
                onChange={event => onPatchSettings({ overlayLayout: event.target.value as OverlaySettings['overlayLayout'] })}>
                <option value="grouped">{t('groupedLayout')}</option>
                <option value="stacks">{t('stacksLayout')}</option>
              </select>
            </label>
            <p id="overlay-layout-hint" className={`mt-1 px-2 ${hintClass}`}>{t('overlayLayoutHint')}</p>
          </div>
          <Slider id="overlay-scale" label={t('scale')} value={settings.scalePercent}
            valueLabel={`${settings.scalePercent}%`} min={100} max={250} step={5}
            marks={['100%', t('defaultScale'), '250%']}
            onChange={scalePercent => onPatchSettings({ scalePercent }, true)} />
          <Slider id="overlay-transparency" label={t('transparency')} value={settings.backgroundTransparencyPercent}
            valueLabel={`${settings.backgroundTransparencyPercent}%`} min={0} max={80} step={5}
            marks={[t('opaque'), t('defaultAlpha'), t('transparent')]} hintId="transparency-hint"
            hint={t('transparencyHint')} onChange={backgroundTransparencyPercent => onPatchSettings({ backgroundTransparencyPercent }, true)} />
          <Toggle label={t('showFiveHourQuota')} checked={settings.showFiveHourQuota}
            hintId="five-hour-quota-hint" hint={t('showFiveHourQuotaHint')} onChange={showFiveHourQuota => onPatchSettings({ showFiveHourQuota })} />
          <Toggle label={t('showProviderLogo')} checked={settings.showProviderLogo} onChange={showProviderLogo => onPatchSettings({ showProviderLogo })} />
          {settings.provider !== 'claude' && <Toggle label={t('showCredits')} checked={settings.showCredits}
            onChange={showCredits => onPatchSettings({ showCredits })} />}
          <Toggle label={t('showBurnRate')} checked={settings.showBurnRate}
            hintId="burn-rate-hint" hint={t('showBurnRateHint')} onChange={showBurnRate => onPatchSettings({ showBurnRate })} />
        </Section>
        <OverlayPreview settings={settings} provider={display?.effectiveProvider ?? settings.provider} />
      </div>

      <div className="common-settings-other">
        <Section id="docking-settings" title={t('settingsDocking')}>
          <Toggle label={t('autoEdgeHide')} checked={settings.autoEdgeHide}
            hintId="edge-hide-hint" hint={t('autoEdgeHideHint')} onChange={autoEdgeHide => onPatchSettings({ autoEdgeHide })} />
          {settings.autoEdgeHide && <Toggle label={t('showPercentageGrid')} checked={settings.showPercentageGrid}
            hintId="percentage-grid-hint" hint={t('showPercentageGridHint')} onChange={showPercentageGrid => onPatchSettings({ showPercentageGrid })} />}
          <Toggle label={t('mousePassthrough')} checked={settings.mousePassthrough}
            hintId="mouse-passthrough-hint" hint={t('mousePassthroughHint')} onChange={mousePassthrough => onPatchSettings({ mousePassthrough })} />
          <p className={`px-2 ${hintClass}`}>{t('hint')}</p>
        </Section>
        <Section id="rotation-settings" title={t('settingsRotation')}>
          <div className="space-y-2.5" data-carousel-settings>
            <Toggle label={t('quotaCarouselEnabled')} checked={rotationEnabled} onChange={quotaCarouselEnabled => onPatchSettings({ quotaCarouselEnabled })} />
            <Slider id="carousel-interval" label={t('quotaCarouselInterval')} value={interval}
              valueLabel={`${interval} ${t('seconds')}`} min={3} max={30} step={1} disabled={!rotationEnabled}
              marks={[`3 ${t('seconds')}`, t('carouselDefault'), `30 ${t('seconds')}`]}
              hintId="carousel-hint" hint={t('carouselHint')}
              onChange={quotaCarouselIntervalSeconds => onPatchSettings({ quotaCarouselIntervalSeconds }, true)} />
            {display?.carouselEnabled && <p role="status" className="px-2 text-xs text-cyan-800 dark:text-cyan-300">
              {t(`carouselStatus_${display.status}`)} · {display.candidates.length} {t('carouselSources')}
            </p>}
          </div>
        </Section>
        <Section id="startup-settings" title={t('settingsStartup')}>
          {isInstalled && <Toggle label={t('autoStart')} checked={settings.autoStart}
            onChange={enable => { void invoke('set_autostart', { enable }).catch(console.error); }} />}
          <Toggle label={t('autoCheckUpdates')} checked={settings.autoCheckUpdates}
            onChange={autoCheckUpdates => onPatchSettings(autoCheckUpdates ? { autoCheckUpdates } : { autoCheckUpdates, autoInstallUpdates: false })} />
          <Toggle label={t('autoInstallUpdates')} checked={settings.autoInstallUpdates} hint={t('autoInstallUpdatesHint')}
            onChange={autoInstallUpdates => onPatchSettings(autoInstallUpdates ? { autoInstallUpdates, autoCheckUpdates: true } : { autoInstallUpdates })} />
          <Toggle label={t('includePrereleaseUpdates')} checked={settings.includePrereleaseUpdates}
            hint={t('includePrereleaseUpdatesHint')} onChange={includePrereleaseUpdates => onPatchSettings({ includePrereleaseUpdates })} />
          {notificationStatus === 'disabled' && <p role="status" className={hintClass}>{t('notifyPermissionDenied')}</p>}
          {notificationStatus === 'unavailable' && <p role="status" className={hintClass}>{t('notificationStatusUnavailable')}</p>}
        </Section>
      </div>
    </div>
  );
}
