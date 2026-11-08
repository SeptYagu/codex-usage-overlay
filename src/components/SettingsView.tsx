import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { OverlaySettings, Provider, ProviderPreference, SettingsPatch, DisplayState } from '../types';
import { providerPreference, settingsForProvider } from '../providerSettings';
import { useProviderMonitors } from '../hooks/useProviderMonitors';
import { APP_VERSION } from '../version';
import { useTranslation } from 'react-i18next';
import { CodexOptions } from './CodexOptions';
import { BurnRateOptions } from './BurnRateOptions';
import { SoundPicker } from './SoundPicker';
import { ProtectionStatusView } from './ProtectionStatus';
import { ClaudeConnectionView } from './ClaudeConnection';
import { DshConnectionView } from './DshConnection';

interface SettingsViewProps {
  settings: OverlaySettings;
  display?: DisplayState;
  saveErrors?: string[];
  onPatchSettings: (patch: SettingsPatch, debounce?: boolean) => void;
}

export const SettingsView = ({ settings, onPatchSettings, saveErrors = [], display }: SettingsViewProps) => {
  const { t } = useTranslation();
  const monitors = useProviderMonitors();
  const [isInstalled, setIsInstalled] = useState(false);
  const [notificationStatus, setNotificationStatus] = useState<string>('loading');
  useEffect(() => {
    invoke<boolean>('is_installed_version').then(setIsInstalled).catch(() => {});
    invoke<string>('get_notification_status').then(setNotificationStatus).catch(() => setNotificationStatus('unavailable'));
  }, []);
  useEffect(() => { getCurrentWindow().setTitle(t('windowSettings')).catch(console.error); }, [t, settings.language]);
  const shared = { savedSettings: settings, savePatch: onPatchSettings, monitors, isInstalled, notificationStatus };
  return <div className="settings-shell bg-[#F5F6F8] dark:bg-slate-900 text-slate-800 dark:text-slate-100 font-sans select-none">
    <header className="flex shrink-0 justify-between items-center border-b border-slate-200 dark:border-slate-800 pb-3">
      <h2 className="text-base font-bold">{t('windowSettings')}</h2><span className="text-xs font-mono">v{APP_VERSION}</span>
    </header>
    <details className="settings-common shrink-0 rounded-lg border border-slate-200 dark:border-slate-700 p-3">
      <summary className="cursor-pointer font-semibold text-sm">{t('commonSettings')}</summary>
      <div className="space-y-2 mt-3" data-carousel-settings>
        <label className="flex items-center justify-between text-sm font-semibold">{t('quotaCarouselEnabled')}<input type="checkbox" className="accent-cyan-500" checked={settings.quotaCarouselEnabled??false} onChange={e=>onPatchSettings({quotaCarouselEnabled:e.target.checked})}/></label>
        <div className="flex justify-between text-sm"><label htmlFor="carousel-interval">{t('quotaCarouselInterval')}</label><span className="font-mono text-cyan-600">{settings.quotaCarouselIntervalSeconds??8} {t('seconds')}</span></div>
        <input id="carousel-interval" type="range" min="3" max="30" step="1" value={settings.quotaCarouselIntervalSeconds??8} onChange={e=>onPatchSettings({quotaCarouselIntervalSeconds:Number(e.target.value)},true)} className="w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500" />
        <div className="flex justify-between text-[11px] text-slate-400"><span>3 {t('seconds')}</span><span>{t('carouselDefault')}</span><span>30 {t('seconds')}</span></div>
        <p className="text-xs text-slate-500">{t('carouselHint')}</p>
        {display?.carouselEnabled && <p role="status" className="text-xs text-cyan-600">{t(`carouselStatus_${display.status}`)} · {display.candidates.length} {t('carouselSources')}</p>}
      </div>
      <SettingsPanel {...shared} common configurationProvider={settings.provider} />
    </details>
    <main className="settings-provider-grid">
      {(['codex', 'claude', 'dsh'] as Provider[]).map(provider => <section key={provider} data-provider-settings={provider} aria-label={t(provider === 'codex' ? 'providerCodex' : provider === 'claude' ? 'providerClaude' : 'providerDsh')} className="settings-provider-column rounded-xl border border-slate-200 dark:border-slate-700 bg-white/60 dark:bg-slate-800/30">
        <header className="flex items-center justify-between gap-2 p-3 border-b border-slate-200 dark:border-slate-700">
          <h3 className="font-semibold text-sm">{t(provider === 'codex' ? 'providerCodex' : provider === 'claude' ? 'providerClaude' : 'providerDsh')}</h3>
          <button type="button" aria-pressed={(display?.effectiveProvider ?? settings.provider) === provider} disabled={!monitors[provider]?.available} onClick={() => onPatchSettings({provider})} className="text-xs rounded border px-2 py-1 disabled:opacity-50">{t((display?.effectiveProvider ?? settings.provider) === provider ? 'currentlyDisplayed' : 'displayThisProvider')}</button>
        </header>
        <label className="flex items-center justify-between gap-2 p-3 text-xs">{t('carouselIncluded')}<input type="checkbox" className="accent-cyan-500" checked={providerPreference(settings,provider).carouselIncluded??true} onChange={e=>onPatchSettings({providerPreferences:{[provider]:{carouselIncluded:e.target.checked}}})}/></label>
        <SettingsPanel {...shared} configurationProvider={provider} />
      </section>)}
    </main>
    {saveErrors.length>0 && <div role="alert" className="text-xs text-rose-600 shrink-0">{saveErrors.join(' · ')}</div>}
    <footer className="shrink-0 text-center text-xs text-slate-400">{t('feedback')} <span className="select-text">septwind@agent.qq.com</span></footer>
  </div>;
};

function SettingsPanel({ savedSettings, savePatch, configurationProvider, monitors, isInstalled, notificationStatus, common = false }: {
  savedSettings: OverlaySettings;
  savePatch: SettingsViewProps['onPatchSettings'];
  configurationProvider: Provider;
  monitors: ReturnType<typeof useProviderMonitors>;
  isInstalled: boolean;
  notificationStatus: string;
  common?: boolean;
}) {
  const { t } = useTranslation();
  const settings = settingsForProvider(savedSettings, configurationProvider);
  const preference = providerPreference(savedSettings, configurationProvider);
  const onPatchSettings = (patch: SettingsPatch, debounce = false) => {
    const fields: Record<string, keyof ProviderPreference> = {
      fiveHourBurnRateAlgorithm: 'fiveHourBurnRateAlgorithm',
      refreshIntervalSeconds: 'refreshIntervalSeconds', fiveHourResetNotification: 'fiveHourResetNotification', weeklyResetNotification: 'weeklyResetNotification',
      fiveHourSoundMode: 'fiveHourSoundMode', weeklySoundMode: 'weeklySoundMode', fiveHourSoundPath: 'fiveHourSoundPath', weeklySoundPath: 'weeklySoundPath',
      autoPauseOnLowQuota: 'protectionEnabled', autoPauseFiveHourThresholdPercent: 'fiveHourThresholdPercent',
      autoPauseWeeklyThresholdPercent: 'weeklyThresholdPercent', dshFiveHourThresholdPercent: 'fiveHourThresholdPercent',
      dshWeeklyThresholdPercent: 'weeklyThresholdPercent',
    };
    const global: Record<string, unknown> = {};
    const scoped: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(patch)) {
      if (fields[key]) scoped[fields[key]] = value; else global[key] = value;
    }
    if (Object.keys(scoped).length) {
      global.providerPreferences = { [configurationProvider]: scoped };
    }
    savePatch(global as SettingsPatch, debounce);
  };
  const updateField = <K extends keyof OverlaySettings>(key: K, value: OverlaySettings[K]) => {
    onPatchSettings({ [key]: value }, key === 'scalePercent' || key === 'backgroundTransparencyPercent');
  };

  const toggleRow = 'flex items-center justify-between p-1.5 rounded-lg bg-slate-100 dark:bg-slate-800/60 hover:bg-slate-200/60 dark:hover:bg-slate-800 cursor-pointer transition-colors';
  const checkbox = 'w-4 h-4 rounded text-cyan-600 focus:ring-cyan-500 accent-cyan-500 cursor-pointer';

  // The smart-pause thresholds only apply while the master switch is on.
  const autoPauseDisabled = !settings.autoPauseOnLowQuota || !preference.monitorEnabled;
  // Claude has no credit balance and is protected by Claude Code hooks instead
  // of a Codex pause request.
  const claude = settings.provider === 'claude';
  const dsh = settings.provider === 'dsh';
  const fiveHourThreshold = dsh ? settings.dshFiveHourThresholdPercent : settings.autoPauseFiveHourThresholdPercent;
  const weeklyThreshold = dsh ? settings.dshWeeklyThresholdPercent : settings.autoPauseWeeklyThresholdPercent;

  if (common) return <div data-common-settings className="grid gap-4 md:grid-cols-2 max-h-72 overflow-y-auto pt-4">
    <div className="space-y-2">          <div className="flex items-center justify-between p-2 rounded-lg bg-slate-100 dark:bg-slate-800/60">
            <label htmlFor="overlay-layout" className="text-sm font-medium">{t('overlayLayout')}</label>
            <select
              id="overlay-layout"
              aria-describedby="overlay-layout-hint"
              value={settings.overlayLayout}
              onChange={(e) => updateField('overlayLayout', e.target.value as OverlaySettings['overlayLayout'])}
              className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500"
            >
              <option value="grouped">{t('groupedLayout')}</option>
              <option value="stacks">{t('stacksLayout')}</option>
            </select>
          </div>

          <p id="overlay-layout-hint" className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('overlayLayoutHint')}</p>

          {/* Scale slider */}
          <div className="space-y-1.5">
            <div className="flex justify-between items-center text-sm font-semibold">
              <span>{t('scale')}</span>
              <span className="font-mono text-cyan-600 dark:text-cyan-400 text-sm">
                {settings.scalePercent}%
              </span>
            </div>
            <input
              aria-label={t('scale')}
              type="range"
              min="100"
              max="250"
              step="5"
              value={settings.scalePercent}
              onChange={(e) => updateField('scalePercent', parseInt(e.target.value, 10))}
              className="w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500"
            />
            <div className="flex justify-between text-[11px] text-slate-400">
              <span>100%</span>
              <span>{t('defaultScale')}</span>
              <span>250%</span>
            </div>
          </div>

          {/* Transparency slider */}
          <div className="space-y-1.5">
            <div className="flex justify-between items-center text-sm font-semibold">
              <span>{t('transparency')}</span>
              <span className="font-mono text-cyan-600 dark:text-cyan-400 text-sm">
                {settings.backgroundTransparencyPercent}%
              </span>
            </div>
            <input
              aria-label={t('transparency')}
              aria-describedby="transparency-hint"
              type="range"
              min="0"
              max="80"
              step="5"
              value={settings.backgroundTransparencyPercent}
              onChange={(e) => updateField('backgroundTransparencyPercent', parseInt(e.target.value, 10))}
              className="w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500"
            />
            <div className="flex justify-between text-[11px] text-slate-400">
              <span>{t('opaque')}</span>
              <span>{t('defaultAlpha')}</span>
              <span>{t('transparent')}</span>
            </div>
          </div>

          <p id="transparency-hint" className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('transparencyHint')}</p>

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('showFiveHourQuota')}</span>
            <input
              type="checkbox"
              checked={settings.showFiveHourQuota}
              aria-describedby="five-hour-quota-hint"
              onChange={(e) => updateField('showFiveHourQuota', e.target.checked)}
              className={checkbox}
            />
          </label>

          <p id="five-hour-quota-hint" className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('showFiveHourQuotaHint')}</p>

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('showProviderLogo')}</span>
            <input
              type="checkbox"
              checked={settings.showProviderLogo}
              onChange={(e) => updateField('showProviderLogo', e.target.checked)}
              className={checkbox}
            />
          </label>

          {savedSettings.provider !== 'claude' && (
            <label className={toggleRow}>
              <span className="text-sm font-medium">{t('showCredits')}</span>
              <input
                type="checkbox"
                checked={settings.showCredits}
                onChange={(e) => updateField('showCredits', e.target.checked)}
                className={checkbox}
              />
            </label>
          )}

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('showBurnRate')}</span>
            <input
              type="checkbox"
              checked={settings.showBurnRate}
              aria-describedby="burn-rate-hint"
              onChange={(e) => updateField('showBurnRate', e.target.checked)}
              className={checkbox}
            />
          </label>

          <p id="burn-rate-hint" className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('showBurnRateHint')}</p>

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('autoEdgeHide')}</span>
            <input
              type="checkbox"
              checked={settings.autoEdgeHide}
              aria-describedby="edge-hide-hint"
              onChange={(e) => updateField('autoEdgeHide', e.target.checked)}
              className={checkbox}
            />
          </label>

          <p id="edge-hide-hint" className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('autoEdgeHideHint')}</p>

          {/* Grid preferences apply only to the collapsed edge bars. */}
          {settings.autoEdgeHide && (
            <>
              <label className={toggleRow}>
                <span className="text-sm font-medium">{t('showPercentageGrid')}</span>
                <input
                  type="checkbox"
                  checked={settings.showPercentageGrid}
                  aria-describedby="percentage-grid-hint"
                  onChange={(e) => updateField('showPercentageGrid', e.target.checked)}
                  className={checkbox}
                />
              </label>
              <p id="percentage-grid-hint" className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('showPercentageGridHint')}</p>
            </>
          )}

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('mousePassthrough')}</span>
            <input
              type="checkbox"
              checked={settings.mousePassthrough}
              aria-describedby="mouse-passthrough-hint"
              onChange={(e) => updateField('mousePassthrough', e.target.checked)}
              className={checkbox}
            />
          </label>
          <p id="mouse-passthrough-hint" className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('mousePassthroughHint')}</p>

          <p className="px-1 text-[11px] text-slate-500 dark:text-slate-400 leading-relaxed">
            {t('hint')}
          </p>
</div><div className="space-y-2">          {/* Autostart (only meaningful for installed builds) */}
          {isInstalled && (
            <label className={toggleRow}>
              <span className="text-sm font-medium">{t('autoStart')}</span>
              <input
                type="checkbox"
                checked={settings.autoStart}
                onChange={(e) => {
                  const checked = e.target.checked;
                  invoke('set_autostart', { enable: checked }).catch(console.error);
                }}
                className={checkbox}
              />
            </label>
          )}

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('autoCheckUpdates')}</span>
            <input
              type="checkbox"
              checked={settings.autoCheckUpdates}
              onChange={(e) => onPatchSettings(e.target.checked
                ? { autoCheckUpdates: true }
                : { autoCheckUpdates: false, autoInstallUpdates: false })}
              className={checkbox}
            />
          </label>

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('autoInstallUpdates')}</span>
            <input
              type="checkbox"
              checked={settings.autoInstallUpdates}
              onChange={(e) => onPatchSettings(e.target.checked
                ? { autoInstallUpdates: true, autoCheckUpdates: true }
                : { autoInstallUpdates: false })}
              className={checkbox}
            />
          </label>
          <p className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('autoInstallUpdatesHint')}</p>

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('includePrereleaseUpdates')}</span>
            <input
              type="checkbox"
              aria-label={t('includePrereleaseUpdates')}
              checked={settings.includePrereleaseUpdates}
              onChange={(e) => updateField('includePrereleaseUpdates', e.target.checked)}
              className={checkbox}
            />
          </label>
          <p className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('includePrereleaseUpdatesHint')}</p>


      {notificationStatus === 'disabled' && <p role="status">{t('notifyPermissionDenied')}</p>}
      {notificationStatus === 'unavailable' && <p role="status">{t('notificationStatusUnavailable')}</p>}
    </div></div>;
  return <div className="settings-provider-body space-y-3 p-3">          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('monitorProvider')}</span>
            <input type="checkbox" className={checkbox} checked={preference.monitorEnabled}
              onChange={event => savePatch({ providerPreferences: { [configurationProvider]: {
                monitorEnabled: event.target.checked, ...(!event.target.checked ? { protectionEnabled: false } : {}),
              } } })} />
          </label>
          <p role="status" className="px-1 text-xs text-slate-500 dark:text-slate-400">
            {t(`monitorHealth.${monitors[configurationProvider]?.health ?? (preference.monitorEnabled ? 'checking' : 'disabled')}`)}
          </p>

          {claude && <ClaudeConnectionView settings={settings} onPatchSettings={onPatchSettings} />}
          {dsh && <DshConnectionView settings={settings} onPatchSettings={onPatchSettings} />}

          <div className="flex items-center justify-between p-2 rounded-lg bg-slate-100 dark:bg-slate-800/60">
            <span className="text-sm font-medium">{t('refreshInterval')}</span>
            <select
              aria-label={t('refreshInterval')}
              aria-describedby={`${configurationProvider}-refresh-interval-hint`}
              value={settings.refreshIntervalSeconds}
              onChange={(e) => updateField('refreshIntervalSeconds', parseInt(e.target.value, 10))}
              className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500 font-mono"
            >
              {![30, 60, 120, 300].includes(settings.refreshIntervalSeconds) && <option value={settings.refreshIntervalSeconds}>{settings.refreshIntervalSeconds} s</option>}
              <option value="30">{t('sec30')}</option>
              <option value="60">{t('sec60')}</option>
              <option value="120">{t('min2')}</option>
              <option value="300">{t('min5')}</option>
            </select>
          </div>

          <p id={`${configurationProvider}-refresh-interval-hint`} className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('refreshIntervalHint')}</p>

          {/* Smart pause: asks the running Codex session to pause when the quota runs low,
              or, for Claude, lets the Claude Code hooks refuse new requests */}
          <div className="space-y-2.5 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5">
            <label className={toggleRow}>
              <span className="text-sm font-medium">{t(claude ? 'claudeGuardOnLowQuota' : 'autoPauseOnLowQuota')}</span>
              <input
                type="checkbox"
                aria-label={t(claude ? 'claudeGuardOnLowQuota' : 'autoPauseOnLowQuota')}
                checked={settings.autoPauseOnLowQuota}
                disabled={!preference.monitorEnabled}
                onChange={(e) => onPatchSettings({ autoPauseOnLowQuota: e.target.checked })}
                className={checkbox}
              />
            </label>

            <div className={`space-y-2.5 ${autoPauseDisabled ? 'opacity-50 cursor-not-allowed' : ''}`}>
              <div className="space-y-1">
                <div className="flex justify-between items-center text-sm font-semibold">
                  <span>{t('autoPauseFiveHourThreshold')}</span>
                  <span className="font-mono text-cyan-600 dark:text-cyan-400 text-sm">
                    {fiveHourThreshold}%
                  </span>
                </div>
                <input
                  aria-label={t('autoPauseFiveHourThreshold')}
                  type="range"
                  min={dsh ? 0 : 1}
                  max="15"
                  step="1"
                  disabled={autoPauseDisabled}
                  value={fiveHourThreshold}
                  onChange={(e) => onPatchSettings(dsh
                    ? { dshFiveHourThresholdPercent: parseInt(e.target.value, 10) }
                    : { autoPauseFiveHourThresholdPercent: parseInt(e.target.value, 10) }, true)}
                  className="w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500 disabled:cursor-not-allowed"
                />
              </div>

              <div className="space-y-1">
                <div className="flex justify-between items-center text-sm font-semibold">
                  <span>{t('autoPauseWeeklyThreshold')}</span>
                  <span className="font-mono text-cyan-600 dark:text-cyan-400 text-sm">
                    {weeklyThreshold}%
                  </span>
                </div>
                <input
                  aria-label={t('autoPauseWeeklyThreshold')}
                  type="range"
                  min={dsh ? 0 : 1}
                  max="10"
                  step="1"
                  disabled={autoPauseDisabled}
                  value={weeklyThreshold}
                  onChange={(e) => onPatchSettings(dsh
                    ? { dshWeeklyThresholdPercent: parseInt(e.target.value, 10) }
                    : { autoPauseWeeklyThresholdPercent: parseInt(e.target.value, 10) }, true)}
                  className="w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500 disabled:cursor-not-allowed"
                />
              </div>
            </div>

            <p className="px-1 text-xs text-slate-500 dark:text-slate-400 leading-relaxed">
              {t(claude ? 'claudeGuardHint' : dsh ? 'dshPauseHint' : 'autoPauseHint')}
            </p>
            {dsh && <p className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('quotaThresholdZeroHint')}</p>}
            {!claude && <ProtectionStatusView settings={settings} onPatchSettings={onPatchSettings} />}
          </div>

          <BurnRateOptions provider={configurationProvider} algorithm={settings.fiveHourBurnRateAlgorithm??'legacy'} usage={monitors[configurationProvider]?.usage} onPatchSettings={onPatchSettings} />
          {configurationProvider === 'codex' && <CodexOptions settings={settings}
            disabled={autoPauseDisabled} onPatchSettings={onPatchSettings} />}
          <section aria-labelledby={`${configurationProvider}-reset-notifications-title`} className="space-y-2.5">
            <h3 id={`${configurationProvider}-reset-notifications-title`} className="text-sm font-semibold">{t('notificationSection')}</h3>
            {([
              { title: 'fiveHourResetNotify', enabled: 'fiveHourResetNotification', mode: 'fiveHourSoundMode', path: 'fiveHourSoundPath' },
              { title: 'weeklyResetNotify', enabled: 'weeklyResetNotification', mode: 'weeklySoundMode', path: 'weeklySoundPath' },
            ] as const).map(({ title, enabled, mode, path }) => (
              <div key={enabled} className="space-y-1.5 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5">
                <h4 className="text-sm font-medium">{t(title)}</h4>
                <label className="flex items-center justify-between gap-3 text-sm">
                  <span>{t('enableNotification')}</span>
                  <input
                    type="checkbox"
                    aria-label={`${t(title)} ${t('enableNotification')}`}
                    checked={settings[enabled]}
                    onChange={(e) => updateField(enabled, e.target.checked)}
                    className="w-4 h-4 rounded text-cyan-600 accent-cyan-500 cursor-pointer"
                  />
                </label>
                <label className={`flex items-center justify-between gap-3 text-sm ${!settings[enabled] ? 'opacity-50' : ''}`}>
                  <span>{t('soundMode')}</span>
                  <select
                    aria-label={`${t(title)} ${t('soundMode')}`}
                    disabled={!settings[enabled]}
                    value={settings[mode]}
                    onChange={(e) => updateField(mode, e.target.value as 'windows' | 'custom')}
                    className="max-w-36 bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 disabled:cursor-not-allowed"
                  >
                    <option value="windows">{t('soundModeWindows')}</option>
                    <option value="custom">{t('soundModeCustom')}</option>
                  </select>
                </label>
                {settings[mode] === 'custom' && settings[enabled] && (
                  <SoundPicker
                    provider={configurationProvider}
                    kind={enabled === 'fiveHourResetNotification' ? 'fiveHour' : 'week'}
                    path={settings[path]}
                    disabled={!settings[enabled]}
                    onChangePath={(value) => updateField(path, value)}
                  />
                )}
              </div>
            ))}
          </section>
</div>;
}
