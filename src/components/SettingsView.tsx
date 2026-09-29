import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { OverlaySettings } from '../types';
import { APP_VERSION } from '../version';
import { useTranslation } from 'react-i18next';
import { SoundPicker } from './SoundPicker';

interface SettingsViewProps {
  settings: OverlaySettings;
  onPatchSettings: (patch: Partial<OverlaySettings>, debounce?: boolean) => void;
}

/**
 * The settings window is a fixed two-column layout sized to fit 960x620 without a
 * vertical scrollbar: header and footer span both columns, the left column holds the
 * overlay's appearance/size/interaction controls and the right one everything that
 * integrates with the system (refresh cadence, start-up, updates, reset alerts).
 */
export const SettingsView: React.FC<SettingsViewProps> = ({
  settings,
  onPatchSettings,
}) => {
  const { t } = useTranslation();
  const [isInstalled, setIsInstalled] = useState<boolean>(false);
  const [notificationStatus, setNotificationStatus] = useState<'enabled' | 'disabled' | 'unavailable' | 'loading'>('loading');

  useEffect(() => {
    invoke<boolean>('is_installed_version')
      .then((installed) => setIsInstalled(installed))
      .catch(() => setIsInstalled(false));
    invoke<'enabled' | 'disabled' | 'unavailable'>('get_notification_status')
      .then(setNotificationStatus)
      .catch(() => setNotificationStatus('unavailable'));
  }, []);

  useEffect(() => {
    getCurrentWindow().setTitle(t('windowSettings')).catch(console.error);
  }, [t, settings.language]);

  const updateField = <K extends keyof OverlaySettings>(key: K, value: OverlaySettings[K]) => {
    onPatchSettings({ [key]: value }, key === 'scalePercent' || key === 'backgroundTransparencyPercent');
  };

  const toggleRow = 'flex items-center justify-between p-2 rounded-lg bg-slate-100 dark:bg-slate-800/60 hover:bg-slate-200/60 dark:hover:bg-slate-800 cursor-pointer transition-colors';
  const checkbox = 'w-4 h-4 rounded text-cyan-600 focus:ring-cyan-500 accent-cyan-500 cursor-pointer';

  return (
    <div className="h-screen overflow-hidden bg-[#F5F6F8] dark:bg-slate-900 text-slate-800 dark:text-slate-100 p-5 flex flex-col font-sans select-none">
      {/* Header — full width */}
      <div className="flex shrink-0 items-center justify-between pb-3 border-b border-slate-200 dark:border-slate-800">
        <h2 className="text-base font-bold tracking-tight">{t('windowSettings')}</h2>
        <span className="text-xs px-2 py-0.5 rounded-full bg-cyan-100 dark:bg-cyan-950 text-cyan-700 dark:text-cyan-300 font-mono">
          v{APP_VERSION}
        </span>
      </div>

      {/* Two columns */}
      <div className="grid flex-1 min-h-0 grid-cols-2 gap-5 pt-4">
        {/* Left column: overlay appearance, size and interaction */}
        <section aria-label={t('windowSettings')} className="min-h-0 space-y-2.5">
          <div className="flex items-center justify-between p-2 rounded-lg bg-slate-100 dark:bg-slate-800/60">
            <label htmlFor="overlay-layout" className="text-sm font-medium">{t('overlayLayout')}</label>
            <select
              id="overlay-layout"
              value={settings.overlayLayout}
              onChange={(e) => updateField('overlayLayout', e.target.value as OverlaySettings['overlayLayout'])}
              className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500"
            >
              <option value="grouped">{t('groupedLayout')}</option>
              <option value="stacks">{t('stacksLayout')}</option>
            </select>
          </div>

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

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('showCredits')}</span>
            <input
              type="checkbox"
              checked={settings.showCredits}
              onChange={(e) => updateField('showCredits', e.target.checked)}
              className={checkbox}
            />
          </label>

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('autoEdgeHide')}</span>
            <input
              type="checkbox"
              checked={settings.autoEdgeHide}
              onChange={(e) => updateField('autoEdgeHide', e.target.checked)}
              className={checkbox}
            />
          </label>

          {/* Directly below the edge-hide switch: the docked pill's percentage grid */}
          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('showPercentageGrid')}</span>
            <input
              type="checkbox"
              checked={settings.showPercentageGrid}
              onChange={(e) => updateField('showPercentageGrid', e.target.checked)}
              className={checkbox}
            />
          </label>

          <label className={toggleRow}>
            <span className="text-sm font-medium">{t('mousePassthrough')}</span>
            <input
              type="checkbox"
              checked={settings.mousePassthrough}
              onChange={(e) => updateField('mousePassthrough', e.target.checked)}
              className={checkbox}
            />
          </label>
          <p className="px-1 text-xs text-slate-500 dark:text-slate-400">{t('mousePassthroughHint')}</p>

          <p className="px-1 text-[11px] text-slate-500 dark:text-slate-400 leading-relaxed">
            {t('hint')}
          </p>
        </section>

        {/* Right column: system integration, updates and reset alerts */}
        <section aria-label={t('notificationSection')} className="min-h-0 space-y-2.5">
          <div className="flex items-center justify-between p-2 rounded-lg bg-slate-100 dark:bg-slate-800/60">
            <span className="text-sm font-medium">{t('refreshInterval')}</span>
            <select
              aria-label={t('refreshInterval')}
              value={settings.refreshIntervalSeconds}
              onChange={(e) => updateField('refreshIntervalSeconds', parseInt(e.target.value, 10))}
              className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500 font-mono"
            >
              <option value="30">{t('sec30')}</option>
              <option value="60">{t('sec60')}</option>
              <option value="120">{t('min2')}</option>
              <option value="300">{t('min5')}</option>
            </select>
          </div>

          {/* Autostart (only meaningful for installed builds) */}
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

          <section aria-labelledby="reset-notifications-title" className="space-y-2.5">
            <h3 id="reset-notifications-title" className="text-sm font-semibold">{t('notificationSection')}</h3>
            {notificationStatus === 'disabled' && (
              <p role="status" className="text-xs text-amber-700 dark:text-amber-300">{t('notifyPermissionDenied')}</p>
            )}
            {notificationStatus === 'unavailable' && (
              <p role="status" className="text-xs text-slate-500 dark:text-slate-400">{t('notificationStatusUnavailable')}</p>
            )}
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
                    kind={enabled === 'fiveHourResetNotification' ? 'fiveHour' : 'week'}
                    path={settings[path]}
                    disabled={!settings[enabled]}
                    onChangePath={(value) => updateField(path, value)}
                  />
                )}
              </div>
            ))}
          </section>
        </section>
      </div>

      {/* Footer — full width */}
      <div className="shrink-0 pt-3 mt-3 border-t border-slate-200 dark:border-slate-800 text-center text-xs text-slate-400">
        {t('feedback')} <span className="font-semibold text-slate-600 dark:text-slate-300 select-text">septwind@agent.qq.com</span>
      </div>
    </div>
  );
};
