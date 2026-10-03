import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { useTranslation } from 'react-i18next';
import i18n, { updateLanguage } from './i18n';
import { CodexUsage, DockStateInfo, OverlaySettings, SettingsEnvelope, DEFAULT_SETTINGS } from './types';
import { OverlayView } from './components/OverlayView';
import { SettingsView } from './components/SettingsView';
import { TrayMenuView } from './components/TrayMenuView';

export const App: React.FC = () => {
  const { t } = useTranslation();
  const [windowLabel] = useState<string>(() => getCurrentWindow().label);
  const [settings, setSettings] = useState<OverlaySettings>(DEFAULT_SETTINGS);
  const [settingsLoadState, setSettingsLoadState] = useState<'loading' | 'ready' | 'error'>('loading');
  const settingsRevision = useRef(-1);
  const settingsLoadToken = useRef(0);
  const pendingPatches = useRef<Partial<OverlaySettings>[]>([]);
  const patchQueue = useRef<Promise<void>>(Promise.resolve());
  const sliderPatch = useRef<Partial<OverlaySettings>>({});
  const sliderTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [usage, setUsage] = useState<CodexUsage | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [dockState, setDockState] = useState<DockStateInfo | null>(null);
  const dockStateRevision = useRef(0);
  const usageEventRevision = useRef(0);

  const loadSettings = useCallback(async () => {
    const token = ++settingsLoadToken.current;
    setSettingsLoadState('loading');
    try {
      const envelope = await invoke<SettingsEnvelope>('get_settings');
      if (token !== settingsLoadToken.current || envelope.revision < settingsRevision.current) return;
      settingsRevision.current = envelope.revision;
      const merged = Object.assign({}, envelope.settings, ...pendingPatches.current, sliderPatch.current);
      setSettings(merged);
      updateLanguage(merged.language);
      setSettingsLoadState('ready');
    } catch (err) {
      if (token !== settingsLoadToken.current) return;
      if (settingsRevision.current >= 0) return;
      console.error('Failed to get settings:', err);
      setSettingsLoadState('error');
    }
  }, []);

  useEffect(() => {
    let active = true;
    loadSettings();

    // Listen to settings update from other windows or tray
    const unlistenSettings = listen<SettingsEnvelope>('settings_updated', (event) => {
      if (event.payload.revision < settingsRevision.current) return;
      settingsRevision.current = event.payload.revision;
      const merged = Object.assign({}, event.payload.settings, ...pendingPatches.current, sliderPatch.current);
      setSettings(merged);
      updateLanguage(merged.language);
      setSettingsLoadState('ready');
    });

    // Register first, then hydrate only if no live event arrived during the
    // cache request. fetchedAt alone cannot order samples within the same second.
    const applyUsageIfNewer = (next: CodexUsage) => {
      setUsage((current) => (
        current && current.fetchedAt > next.fetchedAt ? current : next
      ));
      setIsLoading(false);
    };
    const unlistenUsage = listen<CodexUsage>('usage_updated', (event) => {
      if (!active) return;
      usageEventRevision.current++;
      applyUsageIfNewer(event.payload);
    });
    if (windowLabel === 'main') {
      void unlistenUsage.then(async () => {
        if (!active) return;
        const revision = usageEventRevision.current;
        try {
          const cached = await invoke<CodexUsage | null>('get_last_usage');
          if (active && usageEventRevision.current === revision && cached) {
            applyUsageIfNewer(cached);
          }
        } catch (error) {
          console.error('Failed to get cached usage:', error);
        }
      });
    }

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

    const unlistenDockState = listen<DockStateInfo>('dock_state_changed', (event) => {
      dockStateRevision.current++;
      setDockState(event.payload);
    });
    if (windowLabel === 'main') {
      const revision = dockStateRevision.current;
      invoke<DockStateInfo>('get_dock_state').then((initial) => {
        if (dockStateRevision.current === revision) setDockState(initial);
      }).catch((error) => {
        console.error('Failed to get dock state:', error);
        setDockState({ docked: false, edge: null, expanded: true, hidden: false });
      });
    }

    return () => {
      active = false;
      settingsLoadToken.current++;
      unlistenSettings.then((f) => f());
      unlistenUsage.then((f) => f());
      unlistenUpdate.then((f) => f());
      unlistenDockState.then((f) => f());
    };
  }, [loadSettings, windowLabel]);

  const applySavedSettings = (envelope: SettingsEnvelope) => {
    if (envelope.revision < settingsRevision.current) return;
    settingsRevision.current = envelope.revision;
    const merged = Object.assign({}, envelope.settings, ...pendingPatches.current, sliderPatch.current);
    setSettings(merged);
    updateLanguage(merged.language);
  };

  const enqueuePatch = (patch: Partial<OverlaySettings>, optimistic = true) => {
    pendingPatches.current.push(patch);
    if (optimistic) {
      setSettings((current) => ({ ...current, ...patch }));
      if (patch.language) updateLanguage(patch.language);
    }
    patchQueue.current = patchQueue.current.then(async () => {
      try {
        const envelope = await invoke<SettingsEnvelope>('patch_settings', { patch });
        pendingPatches.current = pendingPatches.current.filter((item) => item !== patch);
        applySavedSettings(envelope);
      } catch (err) {
        pendingPatches.current = pendingPatches.current.filter((item) => item !== patch);
        console.error('Save settings error:', err);
        try {
          applySavedSettings(await invoke<SettingsEnvelope>('get_settings'));
        } catch (reloadError) {
          console.error('Failed to reload settings:', reloadError);
        }
      }
    });
  };

  const flushSliderPatch = (optimistic = true) => {
    if (sliderTimer.current) clearTimeout(sliderTimer.current);
    sliderTimer.current = null;
    const patch = sliderPatch.current;
    sliderPatch.current = {};
    if (Object.keys(patch).length) enqueuePatch(patch, optimistic);
  };

  const handlePatchSettings = (patch: Partial<OverlaySettings>, debounce = false) => {
    if (debounce) {
      sliderPatch.current = { ...sliderPatch.current, ...patch };
      setSettings((current) => ({ ...current, ...patch }));
      if (sliderTimer.current) clearTimeout(sliderTimer.current);
      sliderTimer.current = setTimeout(flushSliderPatch, 250);
    } else {
      enqueuePatch(patch);
    }
  };

  useEffect(() => {
    const flush = () => flushSliderPatch();
    window.addEventListener('pagehide', flush);
    return () => {
      window.removeEventListener('pagehide', flush);
      flushSliderPatch(false);
    };
  }, []);

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
        onPatchSettings={handlePatchSettings}
      />
    );
  }

  if (windowLabel === 'tray-menu' || window.location.hash === '#tray-menu') {
    if (settingsLoadState !== 'ready') return null;
    return <TrayMenuView settings={settings} onPatchSettings={handlePatchSettings} />;
  }

  if (windowLabel === 'main' && (settingsLoadState === 'loading' || dockState === null)) return null;

  return (
    <OverlayView
      settings={settings}
      usage={usage}
      isLoading={isLoading}
      dockState={dockState ?? undefined}
    />
  );
};

export default App;
