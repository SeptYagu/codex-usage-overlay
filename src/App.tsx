import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { useTranslation } from 'react-i18next';
import i18n, { updateLanguage } from './i18n';
import { CodexUsage, OverlaySettings, DEFAULT_SETTINGS } from './types';
import { OverlayView } from './components/OverlayView';
import { SettingsView } from './components/SettingsView';
import { TrayMenuView } from './components/TrayMenuView';

export const App: React.FC = () => {
  const { t } = useTranslation();
  const [windowLabel] = useState<string>(() => getCurrentWindow().label);
  const [settings, setSettings] = useState<OverlaySettings>(DEFAULT_SETTINGS);
  const [settingsLoadState, setSettingsLoadState] = useState<'loading' | 'ready' | 'error'>('loading');
  const settingsRevision = useRef(0);
  const [usage, setUsage] = useState<CodexUsage | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);

  const loadSettings = useCallback(async () => {
    const revision = ++settingsRevision.current;
    setSettingsLoadState('loading');
    try {
      const cfg = await invoke<OverlaySettings>('get_settings');
      if (revision !== settingsRevision.current) return;
      setSettings(cfg);
      updateLanguage(cfg.language);
      setSettingsLoadState('ready');
    } catch (err) {
      if (revision !== settingsRevision.current) return;
      console.error('Failed to get settings:', err);
      setSettingsLoadState('error');
    }
  }, []);

  useEffect(() => {
    loadSettings();

    // Listen to settings update from other windows or tray
    const unlistenSettings = listen<OverlaySettings>('settings_updated', (event) => {
      // A broadcast carries a complete snapshot and supersedes an older read.
      settingsRevision.current++;
      setSettings(event.payload);
      updateLanguage(event.payload.language);
      setSettingsLoadState('ready');
    });

    // Listen to usage data from backend
    const unlistenUsage = listen<CodexUsage>('usage_updated', (event) => {
      setUsage(event.payload);
      setIsLoading(false);
    });

    const unlistenUpdate = listen<{ version: string }>('update_available', async (event) => {
      if (windowLabel !== 'main') return;
      try {
        let granted = await isPermissionGranted();
        if (!granted) granted = (await requestPermission()) === 'granted';
        if (granted) {
          sendNotification({
            title: i18n.t('updateNotificationTitle'),
            body: i18n.t('updateAvailable', { version: event.payload.version }),
          });
        }
      } catch (error) {
        console.error('Could not show update notification:', error);
      }
    });

    // Initial usage fetch
    if (windowLabel === 'main') handleRefresh();

    return () => {
      settingsRevision.current++;
      unlistenSettings.then((f) => f());
      unlistenUsage.then((f) => f());
      unlistenUpdate.then((f) => f());
    };
  }, [loadSettings, windowLabel]);

  const handleRefresh = async () => {
    setIsLoading(true);
    try {
      const data = await invoke<CodexUsage>('fetch_usage');
      if (data) {
        setUsage(data);
      }
    } catch (err) {
      console.error('Fetch usage error:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleUpdateSettings = async (newSettings: OverlaySettings) => {
    setSettings(newSettings);
    updateLanguage(newSettings.language);
    try {
      await invoke('save_settings', { newSettings });
    } catch (err) {
      console.error('Save settings error:', err);
    }
  };

  if (windowLabel === 'settings' || window.location.hash === '#settings') {
    if (settingsLoadState !== 'ready') {
      return (
        <div className="h-screen bg-[#F5F6F8] dark:bg-slate-900 text-slate-800 dark:text-slate-100 p-5 flex flex-col items-center justify-center gap-4">
          <p role={settingsLoadState === 'error' ? 'alert' : 'status'}>
            {t(settingsLoadState === 'error' ? 'settingsLoadFailed' : 'settingsLoading')}
          </p>
          {settingsLoadState === 'error' && (
            <button
              onClick={loadSettings}
              className="rounded-md bg-cyan-600 px-4 py-2 text-white hover:bg-cyan-700"
            >
              {t('retry')}
            </button>
          )}
        </div>
      );
    }
    return (
      <SettingsView
        settings={settings}
        onUpdateSettings={handleUpdateSettings}
      />
    );
  }

  if (windowLabel === 'tray-menu' || window.location.hash === '#tray-menu') {
    if (settingsLoadState !== 'ready') return null;
    return <TrayMenuView settings={settings} onUpdateSettings={handleUpdateSettings} />;
  }

  return (
    <OverlayView
      settings={settings}
      usage={usage}
      isLoading={isLoading}
    />
  );
};

export default App;
