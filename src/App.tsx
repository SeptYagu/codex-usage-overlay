import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { CodexUsage, OverlaySettings, DEFAULT_SETTINGS } from './types';
import { OverlayView } from './components/OverlayView';
import { SettingsView } from './components/SettingsView';

export const App: React.FC = () => {
  const [windowLabel, setWindowLabel] = useState<string>('main');
  const [settings, setSettings] = useState<OverlaySettings>(DEFAULT_SETTINGS);
  const [usage, setUsage] = useState<CodexUsage | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);

  useEffect(() => {
    // Determine window mode from Tauri window label or location hash
    const win = getCurrentWindow();
    setWindowLabel(win.label);

    const hash = window.location.hash;
    if (hash === '#settings' || win.label === 'settings') {
      setWindowLabel('settings');
    }

    // Load initial settings
    invoke<OverlaySettings>('get_settings')
      .then((cfg) => {
        if (cfg) {
          setSettings(cfg);
          import('./i18n').then(({ updateLanguage }) => updateLanguage(cfg.language));
        }
      })
      .catch((err) => console.error('Failed to get settings:', err));

    // Listen to settings update from other windows or tray
    const unlistenSettings = listen<OverlaySettings>('settings_updated', (event) => {
      setSettings(event.payload);
      import('./i18n').then(({ updateLanguage }) => updateLanguage(event.payload.language));
    });

    // Listen to usage data from backend
    const unlistenUsage = listen<CodexUsage>('usage_updated', (event) => {
      setUsage(event.payload);
      setIsLoading(false);
    });

    // Initial usage fetch
    handleRefresh();

    return () => {
      unlistenSettings.then((f) => f());
      unlistenUsage.then((f) => f());
    };
  }, []);

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

  const handleOpenSettings = async () => {
    try {
      await invoke('open_settings_window');
    } catch (err) {
      console.error('Open settings error:', err);
    }
  };

  const handleUpdateSettings = async (newSettings: OverlaySettings) => {
    setSettings(newSettings);
    try {
      await invoke('save_settings', { newSettings });
    } catch (err) {
      console.error('Save settings error:', err);
    }
  };

  if (windowLabel === 'settings' || window.location.hash === '#settings') {
    return (
      <SettingsView
        settings={settings}
        onUpdateSettings={handleUpdateSettings}
      />
    );
  }

  return (
    <OverlayView
      settings={settings}
      usage={usage}
      isLoading={isLoading}
      onRefresh={handleRefresh}
      onOpenSettings={handleOpenSettings}
    />
  );
};

export default App;
