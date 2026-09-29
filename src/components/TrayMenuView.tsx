import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useTranslation } from 'react-i18next';
import { OverlaySettings } from '../types';
import { APP_VERSION } from '../version';

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
  onPatchSettings: (patch: Partial<OverlaySettings>) => void;
}

// Logical width bounds for the tray popup, mirroring `TRAY_MENU_MIN_WIDTH` /
// `TRAY_MENU_MAX_WIDTH` in `src-tauri/src/tray.rs`. The backend re-clamps
// authoritatively when it sizes the native window; clamping here keeps the width we
// request within those bounds so the adaptive logic is well defined and testable.
const TRAY_MENU_MIN_WIDTH = 300;
const TRAY_MENU_MAX_WIDTH = 500;

type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'current' }
  | { kind: 'available'; update: UpdateInfo }
  | { kind: 'ready'; update: UpdateInfo }
  | { kind: 'installing'; progress: UpdateProgress }
  | { kind: 'error'; update?: UpdateInfo };

export const TrayMenuView: React.FC<TrayMenuViewProps> = ({ settings, onPatchSettings }) => {
  const { t } = useTranslation();
  const [isInstalled, setIsInstalled] = useState(false);
  const [languageOpen, setLanguageOpen] = useState(false);
  const [updateState, setUpdateState] = useState<UpdateState>({ kind: 'idle' });
  const menuRef = useRef<HTMLDivElement>(null);
  const generationRef = useRef(0);
  const dprCompRef = useRef(1);
  const scaleReadyRef = useRef(false);
  const lastLayoutKey = useRef('');
  const layoutRevision = useRef(0);
  const [maxCssHeight, setMaxCssHeight] = useState<number | undefined>();

  const measureMenu = useCallback(async () => {
    const content = menuRef.current;
    const generation = generationRef.current;
    if (!content || !generation || !scaleReadyRef.current) return;
    // 20px covers the outer `p-2.5` padding on both axes.
    const height = Math.ceil(content.getBoundingClientRect().height + 20);
    // Menu items are `width: 100%`, so only their overflow (`scrollWidth`) reveals
    // the intrinsic width the popup needs to render every label on a single line.
    let intrinsicWidth = content.scrollWidth;
    content.querySelectorAll<HTMLElement>('.tray-menu-item').forEach((item) => {
      intrinsicWidth = Math.max(intrinsicWidth, item.scrollWidth);
    });
    const width = Math.min(
      TRAY_MENU_MAX_WIDTH,
      Math.max(TRAY_MENU_MIN_WIDTH, Math.ceil(intrinsicWidth + 20)),
    );
    if (height <= 20) return;
    const dprComp = dprCompRef.current;
    const key = `${generation}:${height}:${width}:${dprComp}`;
    if (lastLayoutKey.current === key) return;
    lastLayoutKey.current = key;
    const revision = ++layoutRevision.current;
    try {
      const actualLogicalHeight = await invoke<number>('layout_tray_menu', {
        generation,
        revision,
        heightLogical: height * dprComp,
        widthLogical: width * dprComp,
      });
      if (generation === generationRef.current && revision === layoutRevision.current) {
        setMaxCssHeight(actualLogicalHeight / dprComp);
      }
    } catch (error) {
      if (generation === generationRef.current && revision === layoutRevision.current) {
        lastLayoutKey.current = '';
        console.error('Tray menu layout failed:', error);
      }
    }
  }, []);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    let active = true;
    let unlistenOpen: (() => void) | undefined;
    let unlistenScale: (() => void) | undefined;
    const updateScale = async () => {
      try {
        const scale = await currentWindow.scaleFactor();
        dprCompRef.current = window.devicePixelRatio > 0 ? window.devicePixelRatio / scale : 1;
      } catch {
        dprCompRef.current = 1;
      }
      scaleReadyRef.current = true;
      void measureMenu();
    };
    void invoke<number>('get_tray_menu_generation').then((generation) => {
      if (!active) return;
      generationRef.current = Math.max(generationRef.current, generation);
      void measureMenu();
    }).catch(console.error);
    listen<number>('tray_menu_opened', (event) => {
      generationRef.current = Math.max(generationRef.current, event.payload);
      setMaxCssHeight(undefined);
      void measureMenu();
    }).then((fn) => { unlistenOpen = fn; }).catch(console.error);
    currentWindow.onScaleChanged(() => { void updateScale(); })
      .then((fn) => { unlistenScale = fn; }).catch(console.error);
    void updateScale();
    document.fonts?.ready.then(() => { if (active) void measureMenu(); });
    return () => {
      active = false;
      unlistenOpen?.();
      unlistenScale?.();
    };
  }, [measureMenu]);

  useEffect(() => {
    if (!menuRef.current || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => { void measureMenu(); });
    observer.observe(menuRef.current);
    return () => observer.disconnect();
  }, [measureMenu]);

  useLayoutEffect(() => { void measureMenu(); });

  useEffect(() => {
    invoke<boolean>('is_installed_version').then(setIsInstalled).catch(() => setIsInstalled(false));
    let eventSeen = false;
    const updateAvailable = listen<UpdateInfo>('update_available', (event) => {
      eventSeen = true;
      setUpdateState({ kind: 'available', update: event.payload });
    });
    const updateStarted = listen('update_install_started', () => {
      eventSeen = true;
      setUpdateState({ kind: 'installing', progress: { downloaded: 0, total: null } });
    });
    const updateReady = listen<UpdateInfo>('update_ready', (event) => {
      eventSeen = true;
      setUpdateState({ kind: 'ready', update: event.payload });
    });
    const updateFailed = listen<UpdateInfo>('update_install_failed', (event) => {
      eventSeen = true;
      setUpdateState({ kind: 'error', update: event.payload });
    });
    const updateProgress = listen<UpdateProgress>('update_progress', (event) => {
      eventSeen = true;
      setUpdateState({ kind: 'installing', progress: event.payload });
    });
    void Promise.all([updateAvailable, updateStarted, updateReady, updateFailed, updateProgress])
      .then(() => Promise.all([
        invoke<UpdateInfo | null>('get_available_update'),
        invoke<string | null>('get_update_error'),
        invoke<boolean>('get_update_installing'),
        invoke<boolean>('get_update_ready'),
      ]))
      .then(([update, error, installing, ready]) => {
        if (eventSeen) return;
        if (installing) setUpdateState({ kind: 'installing', progress: { downloaded: 0, total: null } });
        else if (error) setUpdateState({ kind: 'error', update: update ?? undefined });
        else if (ready && update) setUpdateState({ kind: 'ready', update });
        else if (update) setUpdateState({ kind: 'available', update });
      })
      .catch((error) => console.error('Could not load update status:', error));
    return () => {
      updateAvailable.then((unlisten) => unlisten());
      updateStarted.then((unlisten) => unlisten());
      updateReady.then((unlisten) => unlisten());
      updateFailed.then((unlisten) => unlisten());
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
      if (!update) setUpdateState({ kind: 'current' });
      else {
        const ready = await invoke<boolean>('get_update_ready');
        setUpdateState(ready ? { kind: 'ready', update } : { kind: 'available', update });
      }
    } catch (error) {
      console.error('Update check failed:', error);
      setUpdateState({ kind: 'error' });
    }
  };

  const installUpdate = async () => {
    const update = updateState.kind === 'available' || updateState.kind === 'ready' || updateState.kind === 'error'
      ? updateState.update : undefined;
    setUpdateState({ kind: 'installing', progress: { downloaded: 0, total: null } });
    try {
      await invoke('install_update');
    } catch (error) {
      console.error('Update installation failed:', error);
      setUpdateState({ kind: 'error', update });
    }
  };

  const chooseLanguage = (language: string) => {
    onPatchSettings({ language });
  };

  const progressText = (progress: UpdateProgress) => {
    const percent = progress.total && progress.total > 0
      ? Math.min(99, Math.floor((progress.downloaded / progress.total) * 100))
      : 0;
    return t('updateProgress', { percent });
  };

  return (
    <div className="overflow-y-auto bg-white dark:bg-slate-900 text-slate-800 dark:text-slate-100 p-2.5 font-sans select-none" style={{ maxHeight: maxCssHeight }}>
      <div ref={menuRef} role="menu" aria-label={t('menuSettings')} className="space-y-0.5">
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
          {updateState.kind === 'checking' ? t('checkingUpdates') : t('menuCheckUpdates', { version: APP_VERSION })}
        </button>

        {updateState.kind === 'current' && <p role="status" className="px-2.5 py-1 text-xs text-slate-500">{t('upToDate')}</p>}
        {updateState.kind === 'available' && (
          <button className="tray-menu-item text-cyan-700 dark:text-cyan-300" role="menuitem" onClick={installUpdate}>
            {t('clickToInstallUpdate', { version: updateState.update.version })}
          </button>
        )}
        {updateState.kind === 'ready' && (
          <button className="tray-menu-item text-cyan-700 dark:text-cyan-300" role="menuitem" onClick={installUpdate}>
            {t('updateReadyInstallNow', { version: updateState.update.version })}
          </button>
        )}
        {updateState.kind === 'installing' && <p role="status" className="px-2.5 py-1 text-xs text-cyan-700 dark:text-cyan-300">{t('installingUpdate')} {progressText(updateState.progress)}</p>}
        {updateState.kind === 'error' && (
          <div>
            <p role="alert" className="px-2.5 py-1 text-xs text-rose-600">{t('updateFailed')}</p>
            {updateState.update && <button className="tray-menu-item text-cyan-700 dark:text-cyan-300" role="menuitem" onClick={installUpdate}>{t('retryInstallUpdate')}</button>}
          </div>
        )}

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

        <button
          className="tray-menu-item justify-between"
          role="menuitemcheckbox"
          aria-checked={settings.autoEdgeHide}
          onClick={() => onPatchSettings({ autoEdgeHide: !settings.autoEdgeHide })}
        >
          <span>{t('menuAutoEdgeHide')}</span>
          <span aria-hidden="true">{settings.autoEdgeHide ? '✓' : ''}</span>
        </button>

        <button
          className="tray-menu-item justify-between"
          role="menuitemcheckbox"
          aria-checked={settings.mousePassthrough}
          onClick={() => onPatchSettings({ mousePassthrough: !settings.mousePassthrough })}
        >
          <span>{t('menuMousePassthrough')}</span>
          <span aria-hidden="true">{settings.mousePassthrough ? '✓' : ''}</span>
        </button>

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
