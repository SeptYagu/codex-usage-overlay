import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { OverlaySettings } from '../types';
import { useTranslation } from 'react-i18next';
import { updateLanguage } from '../i18n';

interface SettingsViewProps {
  settings: OverlaySettings;
  onUpdateSettings: (newSettings: OverlaySettings) => void;
}

export const SettingsView: React.FC<SettingsViewProps> = ({
  settings,
  onUpdateSettings,
}) => {
  const { t } = useTranslation();
  const [localSettings, setLocalSettings] = useState<OverlaySettings>(settings);
  const [isInstalled, setIsInstalled] = useState<boolean>(false);

  useEffect(() => {
    invoke<boolean>('is_installed_version')
      .then((installed) => setIsInstalled(installed))
      .catch(() => setIsInstalled(false));
  }, []);

  useEffect(() => {
    getCurrentWindow().setTitle(t('windowSettings')).catch(console.error);
  }, [t, localSettings.language]);

  const updateField = <K extends keyof OverlaySettings>(key: K, value: OverlaySettings[K]) => {
    const next = { ...localSettings, [key]: value };
    setLocalSettings(next);
    onUpdateSettings(next);
    if (key === 'language') {
      updateLanguage(value as string);
    }
  };

  return (
    <div className="h-screen overflow-y-auto bg-[#F5F6F8] dark:bg-slate-900 text-slate-800 dark:text-slate-100 p-5 flex flex-col justify-between font-sans select-none custom-scrollbar">
      <div className="space-y-5">
        {/* Header */}
        <div className="flex items-center justify-between pb-3 border-b border-slate-200 dark:border-slate-800">
          <h2 className="text-base font-bold tracking-tight">{t('windowSettings')}</h2>
          <span className="text-xs px-2 py-0.5 rounded-full bg-cyan-100 dark:bg-cyan-950 text-cyan-700 dark:text-cyan-300 font-mono">
            Tauri v2
          </span>
        </div>

        {/* 1. Scale Slider */}
        <div className="space-y-1.5">
          <div className="flex justify-between items-center text-sm font-semibold">
            <span>{t('scale')}</span>
            <span className="font-mono text-cyan-600 dark:text-cyan-400 text-sm">
              {localSettings.scalePercent}%
            </span>
          </div>
          <input
            type="range"
            min="100"
            max="250"
            step="5"
            value={localSettings.scalePercent}
            onChange={(e) => updateField('scalePercent', parseInt(e.target.value, 10))}
            className="w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500"
          />
          <div className="flex justify-between text-[11px] text-slate-400">
            <span>100%</span>
            <span>{t('defaultScale')}</span>
            <span>250%</span>
          </div>
        </div>

        {/* 2. Transparency Slider */}
        <div className="space-y-1.5">
          <div className="flex justify-between items-center text-sm font-semibold">
            <span>{t('transparency')}</span>
            <span className="font-mono text-cyan-600 dark:text-cyan-400 text-sm">
              {localSettings.backgroundTransparencyPercent}%
            </span>
          </div>
          <input
            type="range"
            min="0"
            max="80"
            step="5"
            value={localSettings.backgroundTransparencyPercent}
            onChange={(e) => updateField('backgroundTransparencyPercent', parseInt(e.target.value, 10))}
            className="w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500"
          />
          <div className="flex justify-between text-[11px] text-slate-400">
            <span>{t('opaque')}</span>
            <span>{t('defaultAlpha')}</span>
            <span>{t('transparent')}</span>
          </div>
        </div>

        {/* 3. Toggles */}
        <div className="space-y-3 pt-2">
          {/* Show Credits */}
          <label className="flex items-center justify-between p-2.5 rounded-lg bg-slate-100 dark:bg-slate-800/60 hover:bg-slate-200/60 dark:hover:bg-slate-800 cursor-pointer transition-colors">
            <span className="text-sm font-medium">{t('showCredits')}</span>
            <input
              type="checkbox"
              checked={localSettings.showCredits}
              onChange={(e) => updateField('showCredits', e.target.checked)}
              className="w-4 h-4 rounded text-cyan-600 focus:ring-cyan-500 accent-cyan-500 cursor-pointer"
            />
          </label>

          {/* Autostart (Only shown for installed/installer versions) */}
          {isInstalled && (
            <label className="flex items-center justify-between p-2.5 rounded-lg bg-slate-100 dark:bg-slate-800/60 hover:bg-slate-200/60 dark:hover:bg-slate-800 cursor-pointer transition-colors">
              <span className="text-sm font-medium">{t('autoStart')}</span>
              <input
                type="checkbox"
                checked={localSettings.autoStart}
                onChange={(e) => {
                  const checked = e.target.checked;
                  updateField('autoStart', checked);
                  invoke('set_autostart', { enable: checked }).catch(console.error);
                }}
                className="w-4 h-4 rounded text-cyan-600 focus:ring-cyan-500 accent-cyan-500 cursor-pointer"
              />
            </label>
          )}

          {/* Language Selection */}
          <div className="flex items-center justify-between p-2.5 rounded-lg bg-slate-100 dark:bg-slate-800/60">
            <span className="text-sm font-medium">{t('language')}</span>
            <select
              value={localSettings.language}
              onChange={(e) => updateField('language', e.target.value)}
              className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500"
            >
              <option value="auto">{t('autoSystem')}</option>
              <option value="en-US">English (en-US)</option>
              <option value="zh-CN">简体中文 (zh-CN)</option>
            </select>
          </div>

          {/* Refresh Interval */}
          <div className="flex items-center justify-between p-2.5 rounded-lg bg-slate-100 dark:bg-slate-800/60">
            <span className="text-sm font-medium">{t('refreshInterval')}</span>
            <select
              value={localSettings.refreshIntervalSeconds}
              onChange={(e) => updateField('refreshIntervalSeconds', parseInt(e.target.value, 10))}
              className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500 font-mono"
            >
              <option value="30">{t('sec30')}</option>
              <option value="60">{t('sec60')}</option>
              <option value="120">{t('min2')}</option>
              <option value="300">{t('min5')}</option>
            </select>
          </div>
        </div>

        <p className="text-[11px] text-slate-500 dark:text-slate-400 leading-relaxed">
          {t('hint')}
        </p>
      </div>

      {/* Footer */}
      <div className="pt-4 mt-6 border-t border-slate-200 dark:border-slate-800 text-center text-xs text-slate-400 space-y-2">
        <div>
          {t('feedback')} <span className="font-semibold text-slate-600 dark:text-slate-300 select-text">septwind@agent.qq.com</span>
        </div>
      </div>
    </div>
  );
};
