import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useTranslation } from 'react-i18next';
import i18n, { updateLanguage } from './i18n';
import { CodexUsage, DockStateInfo, OverlaySettings, SettingsEnvelope, DEFAULT_SETTINGS, SettingsPatch } from './types';
import { OverlayView } from './components/OverlayView';
import { SettingsView } from './components/SettingsView';
import { TrayMenuView } from './components/TrayMenuView';
import { sourceBinding, UsageSourceScope, usageMatchesSource, usageSourceScope, usageIsOlder, bufferUsage, takeMatchingUsage } from './usageSource';
import { useDisplayState } from './hooks/useDisplayState';
import { useProviderMonitors } from './hooks/useProviderMonitors';
import { mergeSettingsPatch, mergePendingPatches, settingsForProvider } from './providerSettings';

function errorTargetsForPatch(patch: SettingsPatch): string[] {
  const targets = Object.keys(patch.providerPreferences ?? {});
  if (!targets.length) {
    if (patch.antigravityPool !== undefined) {
      targets.push('antigravity');
    } else if (
      patch.autoPauseFiveHourMessage !== undefined
      || patch.autoPauseWeeklyMessage !== undefined
      || patch.fiveHourBurnRateAlgorithm !== undefined
    ) {
      targets.push('codex');
    } else {
      targets.push('common');
    }
  }
  return targets;
}

export const App: React.FC = () => {
  const { t } = useTranslation();
  const [windowLabel] = useState<string>(() => getCurrentWindow().label);
  const [settings, setSettings] = useState<OverlaySettings>(DEFAULT_SETTINGS);
  const [settingsLoadState, setSettingsLoadState] = useState<'loading' | 'ready' | 'error'>('loading');
  const [saveErrors,setSaveErrors]=useState<Record<string,string>>({});
  const confirmedSettings=useRef<SettingsEnvelope|null>(null);
  const settingsRevision = useRef(-1);
  const settingsLoadToken = useRef(0);
  const pendingPatches = useRef<SettingsPatch[]>([]);
  const patchQueue = useRef<Promise<void>>(Promise.resolve());
  const sliderPatch = useRef<SettingsPatch>({});
  const sliderTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [usage, setUsage] = useState<CodexUsage | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [dockState, setDockState] = useState<DockStateInfo | null>(null);
  const dockStateRevision = useRef(0);
  const usageEventRevision = useRef(0);
  const sourceScope = useRef<UsageSourceScope | null>(null);
  const currentSettings = useRef(DEFAULT_SETTINGS);
  const pendingUsage = useRef(new Map<string,CodexUsage>());
  const monitors = useProviderMonitors();
  const display=useDisplayState();
  const currentDisplay=useRef(display);currentDisplay.current=display;
  const matchesActive=(sample:CodexUsage)=>currentDisplay.current
    ? sample.sourceKey===currentDisplay.current.sourceKey&&sample.sourceGeneration===currentDisplay.current.sourceGeneration
    : usageMatchesSource(sample,sourceScope.current);
  useEffect(()=>{
    if(!display)return;
    const pending=takeMatchingUsage(pendingUsage.current,matchesActive);
    setUsage(current=>{
      const retained=current&&matchesActive(current)?current:null;
      if(pending){return retained&&usageIsOlder(pending,retained)?retained:pending;}
      return retained;
    });
  },[display]);

  const applySavedSettings = useCallback((envelope: SettingsEnvelope) => {
    if (envelope.revision < settingsRevision.current) return;
    settingsRevision.current = envelope.revision;
    confirmedSettings.current=envelope;
    const serverSettings = { ...DEFAULT_SETTINGS, ...envelope.settings };
    const merged = [...pendingPatches.current, sliderPatch.current].reduce<OverlaySettings>(mergeSettingsPatch, serverSettings);
    // An optimistic source change has no authoritative identity until its save
    // arrives. Do not associate the old envelope's identity with that binding.
    const confirmed = sourceBinding(merged) === sourceBinding(serverSettings);
    const scope = usageSourceScope(confirmed ? envelope : { revision: envelope.revision, settings: merged }, merged, sourceScope.current);
    sourceScope.current = scope;
    currentSettings.current = merged;
    const pending = takeMatchingUsage(pendingUsage.current,matchesActive);
    setUsage(current => {
      const retained = current && matchesActive(current) ? current : null;
      if (pending) {
        return retained && usageIsOlder(pending, retained) ? retained : pending;
      }
      return retained;
    });
    setSettings(merged);
    updateLanguage(merged.language);
    setSettingsLoadState('ready');
  }, []);

  const loadSettings = useCallback(async () => {
    const token = ++settingsLoadToken.current;
    setSettingsLoadState('loading');
    try {
      const envelope = await invoke<SettingsEnvelope>('get_settings');
      if (token !== settingsLoadToken.current || envelope.revision < settingsRevision.current) return;
      applySavedSettings(envelope);
    } catch (err) {
      if (token !== settingsLoadToken.current) return;
      if (settingsRevision.current >= 0) return;
      console.error('Failed to get settings:', err);
      setSettingsLoadState('error');
    }
  }, [applySavedSettings]);

  useEffect(() => {
    let active = true;
    loadSettings();

    // Listen to settings update from other windows or tray
    const unlistenSettings = listen<SettingsEnvelope>('settings_updated', (event) => {
      if (active) applySavedSettings(event.payload);
    });

    // Register first, then hydrate only if no live event arrived during the
    // cache request. fetchedAt alone cannot order samples within the same second.
    const applyUsageIfNewer = (next: CodexUsage) => {
      if (!matchesActive(next)) {
        // A sample may arrive before its settings broadcast. Hold it without
        // displaying it; only a matching authoritative source can release it.
        bufferUsage(pendingUsage.current,next);
        return;
      }
      setUsage((current) => {
        if (!matchesActive(next)) return current;
        return current && matchesActive(current)
          && usageIsOlder(next, current) ? current : next;
      });
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

    // Update notifications come from the backend, so a check that finishes
    // before this page has loaded (right at launch) is never lost.

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
      unlistenDockState.then((f) => f());
    };
  }, [applySavedSettings, loadSettings, windowLabel]);

  const enqueuePatch = (patch: SettingsPatch, optimistic = true) => {
    pendingPatches.current.push(patch);
    if (optimistic) {
      const merged = mergeSettingsPatch(currentSettings.current, patch);
      if (!currentDisplay.current && sourceBinding(merged) !== sourceBinding(currentSettings.current)) {
        sourceScope.current = { binding: sourceBinding(merged), allowLegacy: false };
        pendingUsage.current.clear();
        setUsage(null);
      }
      currentSettings.current = merged;
      setSettings(merged);
      if (patch.language) updateLanguage(patch.language);
    }
    patchQueue.current = patchQueue.current.then(async () => {
      try {
        const envelope = await invoke<SettingsEnvelope>('patch_settings', { patch });
        pendingPatches.current = pendingPatches.current.filter((item) => item !== patch);
        applySavedSettings(envelope);
        const targets = errorTargetsForPatch(patch);
        setSaveErrors(current=>{const next={...current};for(const target of targets)delete next[target];return next;});
      } catch (err) {
        pendingPatches.current = pendingPatches.current.filter((item) => item !== patch);
        console.error('Save settings error:', err);
        const targets = errorTargetsForPatch(patch);
        setSaveErrors(current=>({...current,...Object.fromEntries(targets.map(target=>[target,i18n.t('settingsSaveFailed',{target:target==='common'?i18n.t('commonSettings'):target})]))}));
        if(confirmedSettings.current)applySavedSettings(confirmedSettings.current);
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

  const handlePatchSettings = (patch: SettingsPatch, debounce = false) => {
    if (debounce) {
      sliderPatch.current = mergePendingPatches(sliderPatch.current, patch);
      setSettings((current) => mergeSettingsPatch(current, patch));
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
        saveErrors={Object.values(saveErrors)}
        display={display ?? undefined}
      />
    );
  }

  if (windowLabel === 'tray-menu' || window.location.hash === '#tray-menu') {
    if (settingsLoadState !== 'ready') return null;
    return <TrayMenuView display={display??undefined} settings={settingsForProvider(settings, display?.effectiveProvider ?? settings.provider)} onPatchSettings={handlePatchSettings} />;
  }

  if (windowLabel === 'main' && (settingsLoadState === 'loading' || dockState === null)) return null;

  const effectiveProvider=display?.effectiveProvider??settings.provider;
  const monitor=monitors[effectiveProvider];
  const matches=(sample:CodexUsage|null|undefined)=>!!sample && (display
    ?sample.sourceKey===display.sourceKey&&sample.sourceGeneration===display.sourceGeneration
    :usageMatchesSource(sample,sourceScope.current));
  const monitorUsage=matches(monitor?.usage)?monitor!.usage:null;
  const directUsage=matches(usage)?usage:null;
  const displayedUsage=monitorUsage&&directUsage ?(usageIsOlder(monitorUsage,directUsage)?directUsage:monitorUsage):monitorUsage??directUsage;
  return (
    <OverlayView
      settings={settingsForProvider(settings,effectiveProvider)}
      usage={displayedUsage}
      isLoading={monitor ? monitor.health === 'checking' : isLoading}
      dockState={dockState ?? undefined}
      monitors={monitors}
      onCycleProvider={async () => {
        try { applySavedSettings(await invoke<SettingsEnvelope>('cycle_display_provider')); }
        catch (error) { console.error('Could not switch displayed provider:', error); }
      }}
    />
  );
};

export default App;
