import { useState, useEffect, useRef, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { OverlaySettings, Provider, ProviderPreference, SettingsPatch, DisplayState, HistoryRetention, HistoryStorageInfo } from '../types';
import { providerPreference } from '../providerSettings';
import { useProviderMonitors } from '../hooks/useProviderMonitors';
import { APP_VERSION } from '../version';
import { useTranslation } from 'react-i18next';
import { languageOptions } from '../i18n';
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

type SavePatch = SettingsViewProps['onPatchSettings'];

const appPages = [
  { id: 'general', label: 'settingsPageGeneral' },
  { id: 'overlay', label: 'settingsPageOverlay' },
  { id: 'rotation', label: 'settingsPageRotation' },
  { id: 'history', label: 'settingsPageHistory' },
] as const;
const providerPages = [
  { id: 'codex', label: 'providerCodex', provider: 'codex' },
  { id: 'claude', label: 'providerClaude', provider: 'claude' },
  { id: 'dsh', label: 'providerDsh', provider: 'dsh' },
  { id: 'antigravity', label: 'providerAntigravity', provider: 'antigravity' },
] as const;
const settingsPages = [...appPages, ...providerPages];
type SettingsPage = typeof settingsPages[number]['id'];

const toggleRow = 'flex items-center justify-between gap-3 p-1.5 rounded-lg bg-slate-100 dark:bg-slate-800/60 hover:bg-slate-200/60 dark:hover:bg-slate-800 cursor-pointer transition-colors';
const selectRow = 'flex items-center justify-between gap-3 p-1.5 rounded-lg bg-slate-100 dark:bg-slate-800/60';
const checkbox = 'w-4 h-4 shrink-0 rounded text-cyan-600 focus:ring-cyan-500 accent-cyan-500 cursor-pointer';
const select = 'bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500';
const slider = 'w-full h-1.5 bg-slate-200 dark:bg-slate-700 rounded-lg appearance-none cursor-pointer accent-cyan-500 disabled:cursor-not-allowed';
const hint = 'px-1 text-xs text-slate-500 dark:text-slate-400';
const button = 'text-xs rounded border border-slate-300 dark:border-slate-600 px-2.5 py-1';

/** A headed group of related settings on a page. */
function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return <section aria-labelledby={id} className="space-y-2">
    <h3 id={id} className="text-[11px] font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">{title}</h3>
    {children}
  </section>;
}

function Toggle({ label, checked, onChange, hintText, hintId, disabled = false }: {
  label: string; checked: boolean; onChange: (checked: boolean) => void; hintText?: string; hintId?: string; disabled?: boolean;
}) {
  return <>
    <label className={toggleRow}>
      <span className="text-sm font-medium">{label}</span>
      <input type="checkbox" className={checkbox} checked={checked} disabled={disabled} aria-describedby={hintText ? hintId : undefined} onChange={e => onChange(e.target.checked)} />
    </label>
    {hintText && <p id={hintId} className={hint}>{hintText}</p>}
  </>;
}

function Slider({ label, value, unit = '%', min, max, step, onChange, marks, hintText, hintId }: {
  label: string; value: number; unit?: string; min: number; max: number; step: number;
  onChange: (value: number) => void; marks: [string, string, string]; hintText?: string; hintId?: string;
}) {
  return <div className="space-y-1.5">
    <div className="flex justify-between items-center text-sm font-semibold">
      <span>{label}</span>
      <span className="font-mono text-cyan-600 dark:text-cyan-400 text-sm">{value}{unit}</span>
    </div>
    <input aria-label={label} aria-describedby={hintText ? hintId : undefined} type="range" min={min} max={max} step={step} value={value} onChange={e => onChange(parseInt(e.target.value, 10))} className={slider} />
    <div className="flex justify-between text-[11px] text-slate-400">{marks.map(mark => <span key={mark}>{mark}</span>)}</div>
    {hintText && <p id={hintId} className={hint}>{hintText}</p>}
  </div>;
}

export const SettingsView = ({ settings, onPatchSettings, saveErrors = [], display }: SettingsViewProps) => {
  const { t } = useTranslation();
  const monitors = useProviderMonitors();
  const [isInstalled, setIsInstalled] = useState(false);
  const [notificationStatus, setNotificationStatus] = useState<string>('loading');
  const [activePage, setActivePage] = useState<SettingsPage>('general');
  const content = useRef<HTMLElement>(null);
  // Each page opens at its top, not at the previous page's scroll position.
  useEffect(() => { content.current?.scrollTo?.({ top: 0 }); }, [activePage]);
  useEffect(() => {
    invoke<boolean>('is_installed_version').then(setIsInstalled).catch(() => {});
    invoke<string>('get_notification_status').then(setNotificationStatus).catch(() => setNotificationStatus('unavailable'));
  }, []);
  useEffect(() => { getCurrentWindow().setTitle(t('windowSettings')).catch(console.error); }, [t, settings.language]);
  const displayed = display?.effectiveProvider ?? settings.provider;
  const page = settingsPages.find(item => item.id === activePage)!;

  const navButton = (item: typeof settingsPages[number], index: number) => {
    const selected = activePage === item.id;
    const provider = 'provider' in item ? item.provider : null;
    return <button key={item.id} type="button" role="tab" id={`settings-tab-${item.id}`} aria-controls="settings-tab-panel" aria-selected={selected} tabIndex={selected ? 0 : -1}
      onClick={() => setActivePage(item.id)}
      onKeyDown={event => {
        const last = settingsPages.length - 1;
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? last
          : event.key === 'ArrowDown' ? (index + 1) % settingsPages.length
            : event.key === 'ArrowUp' ? (index - 1 + settingsPages.length) % settingsPages.length : -1;
        if (next >= 0) { event.preventDefault(); setActivePage(settingsPages[next].id); document.getElementById(`settings-tab-${settingsPages[next].id}`)?.focus(); }
      }}
      className={`flex w-full items-center justify-between gap-2 rounded-lg px-3 py-1.5 text-left text-sm font-medium ${selected ? 'bg-white dark:bg-slate-800 text-cyan-700 dark:text-cyan-300 border border-slate-200 dark:border-slate-700' : 'border border-transparent text-slate-600 dark:text-slate-300 hover:bg-slate-200/60 dark:hover:bg-slate-800/60'}`}>
      <span className="min-w-0 truncate">{t(item.label)}</span>
      {provider === displayed && <span aria-hidden title={t('currentlyDisplayed')} className="h-1.5 w-1.5 shrink-0 rounded-full bg-cyan-500" />}
    </button>;
  };

  return <div className="settings-shell bg-[#F5F6F8] dark:bg-slate-900 text-slate-800 dark:text-slate-100 font-sans select-none">
    <header className="flex shrink-0 justify-between items-center border-b border-slate-200 dark:border-slate-800 pb-3">
      <h2 className="text-base font-bold">{t('windowSettings')}</h2><span className="text-xs font-mono">v{APP_VERSION}</span>
    </header>
    <div className="settings-body">
      <nav role="tablist" aria-orientation="vertical" aria-label={t('windowSettings')} className="settings-nav">
        {appPages.map((item, index) => navButton(item, index))}
        <p className="px-3 pt-3 pb-1 text-[11px] font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400" aria-hidden>{t('settingsProviders')}</p>
        {providerPages.map((item, index) => navButton(item, appPages.length + index))}
      </nav>
      <main ref={content} id="settings-tab-panel" role="tabpanel" aria-labelledby={`settings-tab-${activePage}`} tabIndex={0} className="settings-content rounded-xl border border-slate-200 dark:border-slate-700 bg-white/60 dark:bg-slate-800/30">
        {'provider' in page
          ? <ProviderPage key={page.provider} provider={page.provider} label={t(page.label)} settings={settings} savePatch={onPatchSettings} monitors={monitors} displayed={displayed} />
          : <div className="space-y-5 p-4" data-app-settings={page.id}>
            <h3 className="text-sm font-semibold">{t(page.label)}</h3>
            {page.id === 'general' && <GeneralPage settings={settings} savePatch={onPatchSettings} isInstalled={isInstalled} notificationStatus={notificationStatus} />}
            {page.id === 'overlay' && <OverlayPage settings={settings} savePatch={onPatchSettings} />}
            {page.id === 'rotation' && <RotationPage settings={settings} savePatch={onPatchSettings} display={display} />}
            {page.id === 'history' && <HistoryPage settings={settings} savePatch={onPatchSettings} />}
          </div>}
      </main>
    </div>
    {saveErrors.length>0 && <div role="alert" className="text-xs text-rose-600 shrink-0">{saveErrors.join(' · ')}</div>}
    <footer className="shrink-0 text-center text-xs text-slate-400">{t('feedback')} <span className="select-text">septwind@agent.qq.com</span></footer>
  </div>;
};

function GeneralPage({ settings, savePatch, isInstalled, notificationStatus }: {
  settings: OverlaySettings; savePatch: SavePatch; isInstalled: boolean; notificationStatus: string;
}) {
  const { t } = useTranslation();
  return <>
    <Section id="settings-language-startup" title={t('sectionLanguageStartup')}>
      <label className={selectRow}>
        <span className="text-sm font-medium">{t('language')}</span>
        <select value={settings.language} onChange={event => savePatch({ language: event.target.value }, false)} className={select}>
          {languageOptions.map(([locale, label]) => <option key={locale} value={locale}>{t(label)}</option>)}
        </select>
      </label>
      {/* Autostart is only meaningful for installed builds. */}
      {isInstalled && <Toggle label={t('autoStart')} checked={settings.autoStart} onChange={enable => { invoke('set_autostart', { enable }).catch(console.error); }} />}
    </Section>
    <Section id="settings-updates" title={t('sectionUpdates')}>
      <Toggle label={t('autoCheckUpdates')} checked={settings.autoCheckUpdates}
        onChange={checked => savePatch(checked ? { autoCheckUpdates: true } : { autoCheckUpdates: false, autoInstallUpdates: false }, false)} />
      <Toggle label={t('autoInstallUpdates')} checked={settings.autoInstallUpdates} hintText={t('autoInstallUpdatesHint')} hintId="auto-install-hint"
        onChange={checked => savePatch(checked ? { autoInstallUpdates: true, autoCheckUpdates: true } : { autoInstallUpdates: false }, false)} />
      <Toggle label={t('includePrereleaseUpdates')} checked={settings.includePrereleaseUpdates} hintText={t('includePrereleaseUpdatesHint')} hintId="prerelease-hint"
        onChange={includePrereleaseUpdates => savePatch({ includePrereleaseUpdates }, false)} />
    </Section>
    {notificationStatus === 'disabled' && <p role="status" className={hint}>{t('notifyPermissionDenied')}</p>}
    {notificationStatus === 'unavailable' && <p role="status" className={hint}>{t('notificationStatusUnavailable')}</p>}
  </>;
}

function OverlayPage({ settings, savePatch }: { settings: OverlaySettings; savePatch: SavePatch }) {
  const { t } = useTranslation();
  const update = <K extends keyof OverlaySettings>(key: K, value: OverlaySettings[K]) =>
    savePatch({ [key]: value }, key === 'scalePercent' || key === 'backgroundTransparencyPercent');
  return <>
    <Section id="settings-appearance" title={t('sectionAppearance')}>
      <div className={selectRow}>
        <label htmlFor="overlay-layout" className="text-sm font-medium">{t('overlayLayout')}</label>
        <select id="overlay-layout" aria-describedby="overlay-layout-hint" value={settings.overlayLayout}
          onChange={e => update('overlayLayout', e.target.value as OverlaySettings['overlayLayout'])} className={select}>
          <option value="grouped">{t('groupedLayout')}</option>
          <option value="stacks">{t('stacksLayout')}</option>
        </select>
      </div>
      <p id="overlay-layout-hint" className={hint}>{t('overlayLayoutHint')}</p>
      <Slider label={t('scale')} value={settings.scalePercent} min={100} max={250} step={5} marks={['100%', t('defaultScale'), '250%']}
        onChange={value => update('scalePercent', value)} />
      <Slider label={t('transparency')} value={settings.backgroundTransparencyPercent} min={0} max={80} step={5} marks={[t('opaque'), t('defaultAlpha'), t('transparent')]}
        hintText={t('transparencyHint')} hintId="transparency-hint" onChange={value => update('backgroundTransparencyPercent', value)} />
      <p className={hint}>{t('hint')}</p>
    </Section>
    <Section id="settings-show-on-overlay" title={t('sectionShowOnOverlay')}>
      <Toggle label={t('showFiveHourQuota')} checked={settings.showFiveHourQuota} hintText={t('showFiveHourQuotaHint')} hintId="five-hour-quota-hint" onChange={v => update('showFiveHourQuota', v)} />
      <Toggle label={t('showProviderLogo')} checked={settings.showProviderLogo} onChange={v => update('showProviderLogo', v)} />
      {settings.provider !== 'claude' && <Toggle label={t('showCredits')} checked={settings.showCredits} onChange={v => update('showCredits', v)} />}
      <Toggle label={t('showBurnRate')} checked={settings.showBurnRate} hintText={t('showBurnRateHint')} hintId="burn-rate-hint" onChange={v => update('showBurnRate', v)} />
    </Section>
    <Section id="settings-behavior" title={t('sectionBehavior')}>
      <Toggle label={t('autoEdgeHide')} checked={settings.autoEdgeHide} hintText={t('autoEdgeHideHint')} hintId="edge-hide-hint" onChange={v => update('autoEdgeHide', v)} />
      {/* The grid applies only to the collapsed edge bars. */}
      {settings.autoEdgeHide && <Toggle label={t('showPercentageGrid')} checked={settings.showPercentageGrid} hintText={t('showPercentageGridHint')} hintId="percentage-grid-hint" onChange={v => update('showPercentageGrid', v)} />}
      <Toggle label={t('mousePassthrough')} checked={settings.mousePassthrough} hintText={t('mousePassthroughHint')} hintId="mouse-passthrough-hint" onChange={v => update('mousePassthrough', v)} />
    </Section>
  </>;
}

function RotationPage({ settings, savePatch, display }: { settings: OverlaySettings; savePatch: SavePatch; display?: DisplayState }) {
  const { t } = useTranslation();
  const interval = settings.quotaCarouselIntervalSeconds ?? 8;
  return <>
    <div className="space-y-2" data-carousel-settings>
      <Toggle label={t('quotaCarouselEnabled')} checked={settings.quotaCarouselEnabled ?? false} onChange={quotaCarouselEnabled => savePatch({ quotaCarouselEnabled })} />
      <Slider label={t('quotaCarouselInterval')} value={interval} unit={` ${t('seconds')}`} min={3} max={30} step={1}
        marks={[`3 ${t('seconds')}`, t('carouselDefault'), `30 ${t('seconds')}`]} onChange={quotaCarouselIntervalSeconds => savePatch({ quotaCarouselIntervalSeconds }, true)} />
      <p className={hint}>{t('carouselHint')}</p>
      {display?.carouselEnabled && <p role="status" className="px-1 text-xs text-cyan-600">{t(`carouselStatus_${display.status}`)} · {display.candidates.length} {t('carouselSources')}</p>}
    </div>
    <Section id="settings-rotation-providers" title={t('sectionRotationProviders')}>
      {providerPages.map(({ provider, label }) => (
        <Toggle key={provider} label={t(label)} checked={providerPreference(settings, provider).carouselIncluded ?? true}
          onChange={carouselIncluded => savePatch({ providerPreferences: { [provider]: { carouselIncluded } } })} />
      ))}
    </Section>
  </>;
}

/** Local usage history: recording, display mode, open, and clear. */
function HistoryPage({ settings, savePatch }: { settings: OverlaySettings; savePatch: SavePatch }) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  const [status, setStatus] = useState<'idle' | 'cleared' | 'failed'>('idle');
  const [storage, setStorage] = useState<HistoryStorageInfo | null>(null);
  const [storageError, setStorageError] = useState(false);
  const [pendingRetention, setPendingRetention] = useState<{ value: HistoryRetention; info: HistoryStorageInfo } | null>(null);
  const [retentionBusy, setRetentionBusy] = useState(false);
  useEffect(() => {
    let active = true;
    void invoke<HistoryStorageInfo>('preview_usage_history_retention', { retention: 'all' })
      .then(info => { if (active) { setStorage(info); setStorageError(false); } })
      .catch(error => { console.error('Could not inspect usage history:', error); if (active) setStorageError(true); });
    return () => { active = false; };
  }, [settings, status]);
  const changeRetention = async (value: HistoryRetention) => {
    if (value === settings.historyRetention) return;
    setRetentionBusy(true);
    try {
      const info = await invoke<HistoryStorageInfo>('preview_usage_history_retention', { retention: value });
      const order: Record<HistoryRetention, number> = { '3m': 0, '6m': 1, '1y': 2, all: 3 };
      if (order[value] < order[settings.historyRetention]) setPendingRetention({ value, info });
      else savePatch({ historyRetention: value });
      setStorageError(false);
    } catch (error) {
      console.error('Could not inspect usage history:', error);
      setStorageError(true);
    } finally {
      setRetentionBusy(false);
    }
  };
  const clear = async () => {
    setConfirming(false);
    try {
      await invoke('clear_usage_history');
      setStatus('cleared');
    } catch (error) {
      console.error('Could not clear usage history:', error);
      setStatus('failed');
    }
  };
  return <>
    <Section id="settings-history-recording" title={t('sectionRecording')}>
      <div className="space-y-2" data-usage-history-settings>
        <Toggle label={t('recordUsageHistory')} checked={settings.recordUsageHistory ?? true} hintText={t('recordUsageHistoryHint')} hintId="record-history-hint"
          onChange={recordUsageHistory => savePatch({ recordUsageHistory })} />
        <label className={selectRow}>
          <span className="text-sm font-medium">{t('historyRetentionLabel')}</span>
          <select value={settings.historyRetention ?? 'all'} disabled={retentionBusy || pendingRetention !== null}
            onChange={event => void changeRetention(event.target.value as HistoryRetention)} className={select}>
            <option value="3m">{t('historyRetention3m')}</option>
            <option value="6m">{t('historyRetention6m')}</option>
            <option value="1y">{t('historyRetention1y')}</option>
            <option value="all">{t('historyRetentionAll')}</option>
          </select>
        </label>
        {pendingRetention && <div role="group" aria-label={t('historyRetentionConfirmTitle')} className="px-1 space-y-2 text-xs">
          <p>{t('historyRetentionConfirm', {
            choice: t(`historyRetention${pendingRetention.value === '3m' ? '3m' : pendingRetention.value === '6m' ? '6m' : pendingRetention.value === '1y' ? '1y' : 'All'}`),
            records: pendingRetention.info.deleteRecords,
            bytes: pendingRetention.info.deleteBytesEstimate.toLocaleString(),
            first: pendingRetention.info.firstAt ? new Date(pendingRetention.info.firstAt * 1000).toLocaleString() : t('historyStorageNone'),
            before: pendingRetention.info.deleteBefore ? new Date(pendingRetention.info.deleteBefore * 1000).toLocaleString() : t('historyStorageNone'),
          })}</p>
          <div className="flex gap-2">
            <button type="button" className="rounded bg-red-600 px-2.5 py-1 text-white" onClick={() => {
              savePatch({ historyRetention: pendingRetention.value });
              setPendingRetention(null);
            }}>{t('historyRetentionConfirmYes')}</button>
            <button type="button" className={button} onClick={() => setPendingRetention(null)}>{t('clearUsageHistoryCancel')}</button>
          </div>
        </div>}
        {storage && <p className={hint}>{t('historyStorageSummary', {
          bytes: storage.totalBytes.toLocaleString(),
          first: storage.firstAt ? new Date(storage.firstAt * 1000).toLocaleString() : t('historyStorageNone'),
          last: storage.lastAt ? new Date(storage.lastAt * 1000).toLocaleString() : t('historyStorageNone'),
        })}</p>}
        {storage?.cleanupError && <p role="alert" className="px-1 text-xs text-red-600">{t('historyCleanupIncomplete')}: {storage.cleanupError}</p>}
        {storageError && <p role="alert" className="px-1 text-xs text-red-600">{t('historyStorageFailed')}</p>}
        <label className={selectRow}>
          <span className="text-sm font-medium">{t('historyTimelineModeLabel')}</span>
          <select value={settings.historyShowUsed ? 'used' : 'remaining'} onChange={event => savePatch({ historyShowUsed: event.target.value === 'used' })} className={select}>
            <option value="remaining">{t('historyTimelineModeRemaining')}</option>
            <option value="used">{t('historyTimelineModeUsed')}</option>
          </select>
        </label>
        <div className="flex flex-wrap items-center gap-2 px-1">
          <button type="button" className={button} onClick={() => void invoke('open_history_window').catch(error => console.error('Could not open usage history:', error))}>
            {t('openUsageHistory')}
          </button>
          {confirming ? (
            <span role="group" aria-label={t('clearUsageHistory')} className="flex flex-wrap items-center gap-2 text-xs">
              <span>{t('clearUsageHistoryConfirm')}</span>
              <button type="button" className="rounded bg-red-600 px-2.5 py-1 text-white" onClick={() => void clear()}>{t('clearUsageHistoryYes')}</button>
              <button type="button" className={button} onClick={() => setConfirming(false)}>{t('clearUsageHistoryCancel')}</button>
            </span>
          ) : (
            <button type="button" className={button} onClick={() => { setStatus('idle'); setConfirming(true); }}>{t('clearUsageHistory')}</button>
          )}
        </div>
        <p className={hint}>{t('historyOpenHint')}</p>
        {status === 'cleared' && <p role="status" className="px-1 text-xs text-cyan-600">{t('clearUsageHistoryDone')}</p>}
        {status === 'failed' && <p role="alert" className="px-1 text-xs text-red-600">{t('clearUsageHistoryFailed')}</p>}
      </div>
    </Section>
  </>;
}

function ProviderPage({ provider, label, settings, savePatch, monitors, displayed }: {
  provider: Provider;
  label: string;
  settings: OverlaySettings;
  savePatch: SavePatch;
  monitors: ReturnType<typeof useProviderMonitors>;
  displayed: Provider;
}) {
  const { t } = useTranslation();
  const preference = providerPreference(settings, provider);
  const patchPreference = (patch: Partial<ProviderPreference>, debounce = false) => {
    savePatch({ providerPreferences: { [provider]: patch } }, debounce);
  };
  const onPatchSettings = (patch: SettingsPatch, debounce = false) => savePatch(patch, debounce);

  // The smart-pause thresholds only apply while the master switch is on.
  const protectionEnabled = preference.monitorEnabled && preference.protectionEnabled;
  const autoPauseDisabled = !protectionEnabled;
  // Claude has no credit balance and is protected by Claude Code hooks instead
  // of a Codex pause request.
  const claude = provider === 'claude';
  const antigravity = provider === 'antigravity';
  const dsh = provider === 'dsh';
  const guardLabel = t(claude ? 'claudeGuardOnLowQuota' : antigravity ? 'antigravityGuardOnLowQuota' : 'autoPauseOnLowQuota');
  const sectionId = (name: string) => `${provider}-${name}-title`;

  return <section data-provider-settings={provider} aria-label={label} className="settings-provider-column">
    <header className="flex items-center justify-between gap-2 p-3 border-b border-slate-200 dark:border-slate-700">
      <h3 className="font-semibold text-sm">{label}</h3>
      <button type="button" aria-pressed={displayed === provider} disabled={!monitors[provider]?.available} onClick={() => savePatch({ provider })} className="text-xs rounded border px-2 py-1 disabled:opacity-50">
        {t(displayed === provider ? 'currentlyDisplayed' : 'displayThisProvider')}
      </button>
    </header>
    <div className="settings-provider-body space-y-5 p-3">
      <Section id={sectionId('monitoring')} title={t('sectionMonitoring')}>
        <label className={toggleRow}>
          <span className="text-sm font-medium">{t('monitorProvider')}</span>
          <input type="checkbox" className={checkbox} checked={preference.monitorEnabled}
            onChange={event => savePatch({ providerPreferences: { [provider]: {
              monitorEnabled: event.target.checked, ...(!event.target.checked ? { protectionEnabled: false } : {}),
            } } })} />
        </label>
        <p role="status" className={hint}>
          {t(`monitorHealth.${monitors[provider]?.health ?? (preference.monitorEnabled ? 'checking' : 'disabled')}`)}
        </p>
        {claude && <ClaudeConnectionView settings={settings} onPatchSettings={onPatchSettings} />}
        {antigravity && <AntigravityConnectionView settings={settings} onPatchSettings={onPatchSettings} />}
        {dsh && <DshConnectionView settings={settings} onPatchSettings={onPatchSettings} />}
        <div className={selectRow}>
          <span className="text-sm font-medium">{t('refreshInterval')}</span>
          <select aria-label={t('refreshInterval')} aria-describedby={`${provider}-refresh-interval-hint`} value={preference.refreshIntervalSeconds}
            onChange={(e) => patchPreference({ refreshIntervalSeconds: parseInt(e.target.value, 10) })} className={`${select} font-mono`}>
            {![30, 60, 120, 300].includes(preference.refreshIntervalSeconds) && <option value={preference.refreshIntervalSeconds}>{preference.refreshIntervalSeconds} s</option>}
            <option value="30">{t('sec30')}</option>
            <option value="60">{t('sec60')}</option>
            <option value="120">{t('min2')}</option>
            <option value="300">{t('min5')}</option>
          </select>
        </div>
        <p id={`${provider}-refresh-interval-hint`} className={hint}>{t('refreshIntervalHint')}</p>
      </Section>

      {/* Smart pause: asks the running Codex session to pause when the quota runs low,
          or, for Claude, lets the Claude Code hooks refuse new requests */}
      <Section id={sectionId('protection')} title={t('sectionProtection')}>
        <label className={toggleRow}>
          <span className="text-sm font-medium">{guardLabel}</span>
          <input type="checkbox" aria-label={guardLabel} checked={protectionEnabled} disabled={!preference.monitorEnabled}
            onChange={(e) => patchPreference({ protectionEnabled: e.target.checked })} className={checkbox} />
        </label>
        <div className={`space-y-2.5 px-1 ${autoPauseDisabled ? 'opacity-50 cursor-not-allowed' : ''}`}>
          {([
            { label: 'autoPauseFiveHourThreshold', value: preference.fiveHourThresholdPercent, max: 15, key: 'fiveHourThresholdPercent' },
            { label: 'autoPauseWeeklyThreshold', value: preference.weeklyThresholdPercent, max: 10, key: 'weeklyThresholdPercent' },
          ] as const).map(item => (
            <div key={item.key} className="space-y-1">
              <div className="flex justify-between items-center text-sm font-semibold">
                <span>{t(item.label)}</span>
                <span className="font-mono text-cyan-600 dark:text-cyan-400 text-sm">{item.value}%</span>
              </div>
              <input aria-label={t(item.label)} type="range" min={dsh ? 0 : 1} max={item.max} step="1" disabled={autoPauseDisabled} value={item.value}
                onChange={(e) => patchPreference({ [item.key]: parseInt(e.target.value, 10) }, true)} className={slider} />
            </div>
          ))}
        </div>
        <p className={`${hint} leading-relaxed`}>{t(claude ? 'claudeGuardHint' : antigravity ? 'antigravityGuardHint' : dsh ? 'dshPauseHint' : 'autoPauseHint')}</p>
        {dsh && <p className={hint}>{t('quotaThresholdZeroHint')}</p>}
        {!claude && !antigravity && <ProtectionStatusView settings={settings} provider={provider} />}
        {provider === 'codex' && <CodexOptions settings={settings} disabled={autoPauseDisabled} onPatchSettings={onPatchSettings} />}
      </Section>

      <Section id={sectionId('display')} title={t('sectionDisplay')}>
        <BurnRateOptions provider={provider} algorithm={preference.fiveHourBurnRateAlgorithm ?? 'legacy'} usage={monitors[provider]?.usage} onPatchSettings={onPatchSettings} />
      </Section>

      <Section id={sectionId('reset-notifications')} title={t('notificationSection')}>
        {([
          { title: 'fiveHourResetNotify', enabled: 'fiveHourResetNotification', mode: 'fiveHourSoundMode', path: 'fiveHourSoundPath' },
          { title: 'weeklyResetNotify', enabled: 'weeklyResetNotification', mode: 'weeklySoundMode', path: 'weeklySoundPath' },
        ] as const).map(({ title, enabled, mode, path }) => (
          <div key={enabled} className="space-y-1.5 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5">
            <h4 className="text-sm font-medium">{t(title)}</h4>
            <label className="flex items-center justify-between gap-3 text-sm">
              <span>{t('enableNotification')}</span>
              <input type="checkbox" aria-label={`${t(title)} ${t('enableNotification')}`} checked={preference[enabled]}
                onChange={(e) => patchPreference({ [enabled]: e.target.checked })} className={checkbox} />
            </label>
            <label className={`flex items-center justify-between gap-3 text-sm ${!preference[enabled] ? 'opacity-50' : ''}`}>
              <span>{t('soundMode')}</span>
              <select aria-label={`${t(title)} ${t('soundMode')}`} disabled={!preference[enabled]} value={preference[mode]}
                onChange={(e) => patchPreference({ [mode]: e.target.value as 'windows' | 'custom' })} className={`max-w-36 ${select} disabled:cursor-not-allowed`}>
                <option value="windows">{t('soundModeWindows')}</option>
                <option value="custom">{t('soundModeCustom')}</option>
              </select>
            </label>
            {preference[mode] === 'custom' && preference[enabled] && (
              <SoundPicker provider={provider} kind={enabled === 'fiveHourResetNotification' ? 'fiveHour' : 'week'} path={preference[path]}
                disabled={!preference[enabled]} onChangePath={(value) => patchPreference({ [path]: value })} />
            )}
          </div>
        ))}
      </Section>
    </div>
  </section>;
}
