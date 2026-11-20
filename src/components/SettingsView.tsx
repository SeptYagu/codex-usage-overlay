import { useState, useEffect, useLayoutEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { OverlaySettings, Provider, ProviderPreference, SettingsPatch, DisplayState } from '../types';
import { providerPreference } from '../providerSettings';
import { useProviderMonitors } from '../hooks/useProviderMonitors';
import { APP_VERSION } from '../version';
import { useTranslation } from 'react-i18next';
import { CommonSettings } from './CommonSettings';
import { CodexOptions } from './CodexOptions';
import { BurnRateOptions } from './BurnRateOptions';
import { SoundPicker } from './SoundPicker';
import { ProtectionStatusView } from './ProtectionStatus';
import { ClaudeConnectionView } from './ClaudeConnection';
import { AntigravityConnectionView } from './AntigravityConnection';
import { DshConnectionView } from './DshConnection';

interface SettingsViewProps {
  settings: OverlaySettings;
  display?: DisplayState;
  saveErrors?: string[];
  onPatchSettings: (patch: SettingsPatch, debounce?: boolean) => void;
}

const settingsTabs = [
  { id: 'common', label: 'commonSettings' },
  { id: 'codex', label: 'providerCodex', provider: 'codex' },
  { id: 'claude', label: 'providerClaude', provider: 'claude' },
  { id: 'dsh', label: 'providerDsh', provider: 'dsh' },
  { id: 'antigravity', label: 'providerAntigravity', provider: 'antigravity' },
] as const;
type SettingsTab = typeof settingsTabs[number]['id'];

export const SettingsView = ({ settings, onPatchSettings, saveErrors = [], display }: SettingsViewProps) => {
  const { t } = useTranslation();
  const monitors = useProviderMonitors();
  const [isInstalled, setIsInstalled] = useState(false);
  const [notificationStatus, setNotificationStatus] = useState<string>('loading');
  const [activeTab, setActiveTab] = useState<SettingsTab>('common');
  const panelRef = useRef<HTMLElement>(null);
  const scrollPositions = useRef<Partial<Record<SettingsTab, number>>>({});
  const selectTab = (tab: SettingsTab) => {
    if (tab === activeTab) return;
    scrollPositions.current[activeTab] = panelRef.current?.scrollTop ?? 0;
    setActiveTab(tab);
  };
  useLayoutEffect(() => {
    if (panelRef.current) panelRef.current.scrollTop = scrollPositions.current[activeTab] ?? 0;
  }, [activeTab]);
  useEffect(() => {
    invoke<boolean>('is_installed_version').then(setIsInstalled).catch(() => {});
    invoke<string>('get_notification_status').then(setNotificationStatus).catch(() => setNotificationStatus('unavailable'));
  }, []);
  useEffect(() => { getCurrentWindow().setTitle(t('windowSettings')).catch(console.error); }, [t, settings.language]);
  const activePage = settingsTabs.find(page => page.id === activeTab)!;
  const provider = 'provider' in activePage ? activePage.provider : null;
  return <div className="settings-shell bg-[#F5F6F8] dark:bg-slate-900 text-slate-800 dark:text-slate-100 font-sans select-none">
    <header className="flex shrink-0 justify-between items-center border-b border-slate-200 dark:border-slate-800 pb-3">
      <h2 className="text-base font-bold">{t('windowSettings')}</h2><span className="text-xs font-mono">v{APP_VERSION}</span>
    </header>
    <nav role="tablist" aria-label={t('windowSettings')} className="settings-tab-list shrink-0 flex gap-2 border-b border-slate-200 dark:border-slate-700">
      {settingsTabs.map((tab, index) => <button key={tab.id} type="button" role="tab" id={`settings-tab-${tab.id}`} aria-controls="settings-tab-panel" aria-selected={activeTab === tab.id} tabIndex={activeTab === tab.id ? 0 : -1} onClick={() => selectTab(tab.id)} onKeyDown={event => {
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? settingsTabs.length - 1 : event.key === 'ArrowRight' ? (index + 1) % settingsTabs.length : event.key === 'ArrowLeft' ? (index - 1 + settingsTabs.length) % settingsTabs.length : -1;
        if (next >= 0) { event.preventDefault(); selectTab(settingsTabs[next].id); document.getElementById(`settings-tab-${settingsTabs[next].id}`)?.focus(); }
      }} className={`shrink-0 rounded-t-lg px-4 py-2 text-sm font-semibold ${activeTab === tab.id ? 'bg-white dark:bg-slate-800 text-cyan-700 dark:text-cyan-300 border border-b-0 border-slate-200 dark:border-slate-700' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-100'}`}>{t(tab.label)}</button>)}
    </nav>
    <main ref={panelRef} id="settings-tab-panel" role="tabpanel" aria-labelledby={`settings-tab-${activeTab}`} tabIndex={0} className="settings-content rounded-xl border border-slate-200 dark:border-slate-700 bg-white/60 dark:bg-slate-800/30">
    {activeTab === 'common' ? (
      <CommonSettings settings={settings} onPatchSettings={onPatchSettings} display={display}
        isInstalled={isInstalled} notificationStatus={notificationStatus} />
    ) : provider && <section key={provider} data-provider-settings={provider} aria-label={t(activePage.label)} className="settings-provider-column">
        <header className="flex items-center justify-between gap-2 p-3 border-b border-slate-200 dark:border-slate-700">
          <h3 className="font-semibold text-sm">{t(activePage.label)}</h3>
          <button type="button" aria-pressed={(display?.effectiveProvider ?? settings.provider) === provider} disabled={!monitors[provider]?.available} onClick={() => onPatchSettings({provider})} className="text-xs rounded border px-2 py-1 disabled:opacity-50">{t((display?.effectiveProvider ?? settings.provider) === provider ? 'currentlyDisplayed' : 'displayThisProvider')}</button>
        </header>
        <label className="flex items-center justify-between gap-2 p-3 text-xs">{t('carouselIncluded')}<input type="checkbox" className="accent-cyan-500" checked={providerPreference(settings,provider).carouselIncluded??true} onChange={e=>onPatchSettings({providerPreferences:{[provider]:{carouselIncluded:e.target.checked}}})}/></label>
        <ProviderSettings savedSettings={settings} savePatch={onPatchSettings} monitors={monitors} configurationProvider={provider} />
      </section>}
    </main>
    {saveErrors.length>0 && <div role="alert" className="text-xs text-rose-600 shrink-0">{saveErrors.join(' · ')}</div>}
    <footer className="shrink-0 text-center text-xs text-slate-600 dark:text-slate-400">{t('feedback')} <span className="select-text">septwind@agent.qq.com</span></footer>
  </div>;
};

function ProviderSettings({ savedSettings, savePatch, configurationProvider, monitors }: {
  savedSettings: OverlaySettings;
  savePatch: SettingsViewProps['onPatchSettings'];
  configurationProvider: Provider;
  monitors: ReturnType<typeof useProviderMonitors>;
}) {
  const { t } = useTranslation();
  const settings = savedSettings;
  const preference = providerPreference(savedSettings, configurationProvider);
  const patchPreference = (patch: Partial<ProviderPreference>, debounce = false) => {
    savePatch({ providerPreferences: { [configurationProvider]: patch } }, debounce);
  };
  const onPatchSettings = (patch: SettingsPatch, debounce = false) => {
    savePatch(patch, debounce);
  };

  const toggleRow = 'flex items-center justify-between p-1.5 rounded-lg bg-slate-100 dark:bg-slate-800/60 hover:bg-slate-200/60 dark:hover:bg-slate-800 cursor-pointer transition-colors';
  const checkbox = 'w-4 h-4 rounded text-cyan-600 focus:ring-cyan-500 accent-cyan-500 cursor-pointer';

  // The smart-pause thresholds only apply while the master switch is on.
  const protectionEnabled = preference.monitorEnabled && preference.protectionEnabled;
  const autoPauseDisabled = !protectionEnabled;
  // Claude has no credit balance and is protected by Claude Code hooks instead
  // of a Codex pause request.
  const claude = configurationProvider === 'claude';
  const antigravity = configurationProvider === 'antigravity';
  const dsh = configurationProvider === 'dsh';
  const fiveHourThreshold = preference.fiveHourThresholdPercent;
  const weeklyThreshold = preference.weeklyThresholdPercent;

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
          {antigravity && <AntigravityConnectionView settings={settings} onPatchSettings={onPatchSettings} />}
          {dsh && <DshConnectionView settings={settings} onPatchSettings={onPatchSettings} />}

          <div className="flex items-center justify-between p-2 rounded-lg bg-slate-100 dark:bg-slate-800/60">
            <span className="text-sm font-medium">{t('refreshInterval')}</span>
            <select
              aria-label={t('refreshInterval')}
              aria-describedby={`${configurationProvider}-refresh-interval-hint`}
              value={preference.refreshIntervalSeconds}
              onChange={(e) => patchPreference({ refreshIntervalSeconds: parseInt(e.target.value, 10) })}
              className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500 font-mono"
            >
              {![30, 60, 120, 300].includes(preference.refreshIntervalSeconds) && <option value={preference.refreshIntervalSeconds}>{preference.refreshIntervalSeconds} s</option>}
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
              <span className="text-sm font-medium">{t(claude ? 'claudeGuardOnLowQuota' : antigravity ? 'antigravityGuardOnLowQuota' : 'autoPauseOnLowQuota')}</span>
              <input
                type="checkbox"
                aria-label={t(claude ? 'claudeGuardOnLowQuota' : antigravity ? 'antigravityGuardOnLowQuota' : 'autoPauseOnLowQuota')}
                checked={protectionEnabled}
                disabled={!preference.monitorEnabled}
                onChange={(e) => patchPreference({ protectionEnabled: e.target.checked })}
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
                  onChange={(e) => patchPreference({ fiveHourThresholdPercent: parseInt(e.target.value, 10) }, true)}
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
                  onChange={(e) => patchPreference({ weeklyThresholdPercent: parseInt(e.target.value, 10) }, true)}
                  className="w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500 disabled:cursor-not-allowed"
                />
              </div>
            </div>

            <p className="px-1 text-xs text-slate-500 dark:text-slate-400 leading-relaxed">
              {t(claude ? 'claudeGuardHint' : antigravity ? 'antigravityGuardHint' : dsh ? 'dshPauseHint' : 'autoPauseHint')}
            </p>
            {dsh && <p className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('quotaThresholdZeroHint')}</p>}
            {!claude && !antigravity && <ProtectionStatusView settings={settings} provider={configurationProvider} />}
          </div>

          <BurnRateOptions provider={configurationProvider} algorithm={preference.fiveHourBurnRateAlgorithm ?? 'legacy'} usage={monitors[configurationProvider]?.usage} onPatchSettings={onPatchSettings} />
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
                    checked={preference[enabled]}
                    onChange={(e) => patchPreference({ [enabled]: e.target.checked })}
                    className="w-4 h-4 rounded text-cyan-600 accent-cyan-500 cursor-pointer"
                  />
                </label>
                <label className={`flex items-center justify-between gap-3 text-sm ${!preference[enabled] ? 'opacity-50' : ''}`}>
                  <span>{t('soundMode')}</span>
                  <select
                    aria-label={`${t(title)} ${t('soundMode')}`}
                    disabled={!preference[enabled]}
                    value={preference[mode]}
                    onChange={(e) => patchPreference({ [mode]: e.target.value as 'windows' | 'custom' })}
                    className="max-w-36 bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 disabled:cursor-not-allowed"
                  >
                    <option value="windows">{t('soundModeWindows')}</option>
                    <option value="custom">{t('soundModeCustom')}</option>
                  </select>
                </label>
                {preference[mode] === 'custom' && preference[enabled] && (
                  <SoundPicker
                    provider={configurationProvider}
                    kind={enabled === 'fiveHourResetNotification' ? 'fiveHour' : 'week'}
                    path={preference[path]}
                    disabled={!preference[enabled]}
                    onChangePath={(value) => patchPreference({ [path]: value })}
                  />
                )}
              </div>
            ))}
          </section>
</div>;
}
