import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useTranslation } from 'react-i18next';
import { OverlaySettings } from '../types';

interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes?: string | null;
}

interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

interface TrayMenuViewProps {
  settings: OverlaySettings;
  onUpdateSettings: (newSettings: OverlaySettings) => void;
}

type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'current' }
  | { kind: 'available'; update: UpdateInfo }
  | { kind: 'installing'; progress: UpdateProgress }
  | { kind: 'error' };

export const TrayMenuView: React.FC<TrayMenuViewProps> = ({ settings, onUpdateSettings }) => {
  const { t } = useTranslation();
  const [isInstalled, setIsInstalled] = useState(false);
  const [languageOpen, setLanguageOpen] = useState(false);
  const [updateState, setUpdateState] = useState<UpdateState>({ kind: 'idle' });

  useEffect(() => {
    invoke<boolean>('is_installed_version').then(setIsInstalled).catch(() => setIsInstalled(false));
    invoke<UpdateInfo | null>('get_available_update')
      .then((update) => {
        if (update) setUpdateState({ kind: 'available', update });
      })
      .catch(() => {});

    const updateAvailable = listen<UpdateInfo>('update_available', (event) => {
      setUpdateState({ kind: 'available', update: event.payload });
    });
    const updateProgress = listen<UpdateProgress>('update_progress', (event) => {
      setUpdateState({ kind: 'installing', progress: event.payload });
    });
    return () => {
      updateAvailable.then((unlisten) => unlisten());
      updateProgress.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') getCurrentWindow().hide().catch(console.error);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  const closeMenu = () => getCurrentWindow().hide().catch(console.error);

  const runMenuAction = async (action: () => Promise<unknown>) => {
    closeMenu();
    try {
      await action();
    } catch (error) {
      console.error(error);
    }
  };

  const checkForUpdates = async () => {
    setUpdateState({ kind: 'checking' });
    try {
      const update = await invoke<UpdateInfo | null>('check_for_updates');
      setUpdateState(update ? { kind: 'available', update } : { kind: 'current' });
    } catch (error) {
      console.error('Update check failed:', error);
      setUpdateState({ kind: 'error' });
    }
  };

  const installUpdate = async () => {
    setUpdateState({ kind: 'installing', progress: { downloaded: 0, total: null } });
    try {
      await invoke('install_update');
    } catch (error) {
      console.error('Update installation failed:', error);
      setUpdateState({ kind: 'error' });
    }
  };

  const chooseLanguage = (language: string) => {
    onUpdateSettings({ ...settings, language });
  };

  const progressText = (progress: UpdateProgress) => {
    const percent = progress.total && progress.total > 0
      ? Math.min(99, Math.floor((progress.downloaded / progress.total) * 100))
      : 0;
    return t('updateProgress', { percent });
  };

  return (
    <div className="h-screen overflow-y-auto bg-white dark:bg-slate-900 text-slate-800 dark:text-slate-100 p-2.5 font-sans select-none">
      <div role="menu" aria-label={t('menuSettings')} className="space-y-0.5">
        <button className="tray-menu-item" role="menuitem" onClick={() => runMenuAction(() => invoke('toggle_overlay_window'))}>
          {t('menuToggleOverlay')}
        </button>
        <button className="tray-menu-item" role="menuitem" onClick={() => runMenuAction(() => invoke('fetch_usage'))}>
          {t('menuRefreshUsage')}
        </button>
        <div className="tray-menu-separator" />
        <button className="tray-menu-item" role="menuitem" onClick={() => runMenuAction(() => invoke('open_settings_window'))}>
          {t('menuSettings')}
        </button>
        <button className="tray-menu-item" role="menuitem" onClick={checkForUpdates} disabled={updateState.kind === 'checking' || updateState.kind === 'installing'}>
          {updateState.kind === 'checking' ? t('checkingUpdates') : t('menuCheckUpdates')}
        </button>

        {updateState.kind === 'current' && <p role="status" className="px-2.5 py-1 text-xs text-slate-500">{t('upToDate')}</p>}
        {updateState.kind === 'available' && (
          <div className="mx-1 my-1 rounded-md bg-cyan-50 dark:bg-cyan-950/50 p-2 text-xs">
            <p role="status" className="mb-1.5">{t('updateAvailable', { version: updateState.update.version })}</p>
            <button className="w-full rounded bg-cyan-600 px-2 py-1.5 font-medium text-white hover:bg-cyan-700" onClick={installUpdate}>
              {t('installUpdate')}
            </button>
          </div>
        )}
        {updateState.kind === 'installing' && <p role="status" className="px-2.5 py-1 text-xs text-cyan-700 dark:text-cyan-300">{t('installingUpdate')} {progressText(updateState.progress)}</p>}
        {updateState.kind === 'error' && <p role="alert" className="px-2.5 py-1 text-xs text-rose-600">{t('updateFailed')}</p>}

        <div className="tray-menu-separator" />
        <button
          className="tray-menu-item justify-between"
          role="menuitem"
          aria-expanded={languageOpen}
          onClick={() => setLanguageOpen((open) => !open)}
        >
          <span>{t('menuLanguage')}</span><span className="text-slate-400">{languageOpen ? '⌄' : '›'}</span>
        </button>
        {languageOpen && (
          <div role="group" aria-label={t('menuLanguage')} className="ml-3 border-l border-slate-200 dark:border-slate-700 pl-2">
            {[
              ['auto', t('autoSystem')],
              ['en-US', t('languageEnglish')],
              ['zh-CN', t('languageSimplifiedChinese')],
              ['zh-Hant', t('languageTraditionalChinese')],
            ].map(([language, label]) => (
              <button
                key={language}
                role="menuitemradio"
                aria-checked={settings.language === language}
                className="tray-menu-item justify-between"
                onClick={() => chooseLanguage(language)}
              >
                <span>{label}</span><span>{settings.language === language ? '✓' : ''}</span>
              </button>
            ))}
          </div>
        )}

        {isInstalled && (
          <label className="tray-menu-item cursor-pointer justify-between">
            <span>{t('menuAutoStart')}</span>
            <input
              type="checkbox"
              checked={settings.autoStart}
              onChange={(event) => {
                invoke('set_autostart', { enable: event.target.checked }).catch((error) => console.error('Autostart update failed:', error));
              }}
              className="h-4 w-4 accent-cyan-600"
            />
          </label>
        )}

        <div className="tray-menu-separator" />
        <button className="tray-menu-item" role="menuitem" onClick={() => runMenuAction(() => invoke('exit_app'))}>
          {t('menuExit')}
        </button>
      </div>
    </div>
  );
};
