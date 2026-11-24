import { burnRatePresentation } from '../burnRatePresentation';
import React, { useEffect, useLayoutEffect, useState, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { CodexUsage, DockStateInfo, OverlaySettings, MonitorSnapshot, Provider } from '../types';
import { useTranslation } from 'react-i18next';
import i18n from '../i18n';
import { ProviderLogo } from './ProviderLogo';

interface OverlayViewProps {
  settings: OverlaySettings;
  provider?: Provider;
  usage: CodexUsage | null;
  isLoading: boolean;
  dockState?: DockStateInfo;
  monitors?: Partial<Record<Provider, MonitorSnapshot>>;
  onCycleProvider?: () => Promise<void>;
}

function formatResetCountdown(resetsAt: number | null | undefined, weekly = false): string {
  const russian = i18n.resolvedLanguage === 'ru-RU';
  const [day, hour, minute] = russian ? [' д', ' ч', ' мин'] : ['d', 'h', 'm'];
  if (resetsAt === null || resetsAt === undefined) return `--${hour} --${minute}`;
  const remainingSeconds = Math.max(0, resetsAt - Math.floor(Date.now() / 1000));
  const totalMinutes = Math.ceil(remainingSeconds / 60);
  const hours = Math.floor(totalMinutes / 60);
  if (weekly && hours >= 24) {
    return `${Math.floor(hours / 24)}${day} ${(hours % 24).toString().padStart(2, '0')}${hour}`;
  }
  return `${hours}${hour} ${(totalMinutes % 60).toString().padStart(2, '0')}${minute}`;
}

function usageTone(percent: number | null | undefined): string {
  if (percent === null || percent === undefined) return 'unknown';
  return percent >= 50 ? 'healthy' : percent >= 20 ? 'warning' : 'low';
}

export function formatBurnRate(rate: number | null | undefined): string {
  if (rate === null || rate === undefined || !Number.isFinite(rate) || rate < 0) return '—';
  const russian = i18n.resolvedLanguage === 'ru-RU';
  const unit = russian ? '%/ч' : '%/h';
  if (rate > 999) return `>999${unit}`;
  if (rate > 0 && rate < 0.05) return `<${russian ? '0,1' : '0.1'}${unit}`;
  const value = rate.toFixed(1);
  return `${russian ? value.replace('.', ',') : value}${unit}`;
}

/**
 * Vertical positions of the pill's percentage grid, in percent of the bar's height.
 * Nine lines split 0~100% into ten 10% cells, so the remaining quota can be counted
 * off the pill without a numeric readout.
 */
const PERCENTAGE_GRID_TICKS = [10, 20, 30, 40, 50, 60, 70, 80, 90];

export const OverlayView: React.FC<OverlayViewProps> = ({ settings, provider, usage, isLoading, dockState, monitors = {}, onCycleProvider }) => {
  const activeProvider = provider ?? settings.provider;
  const { t } = useTranslation();
  const [, setTick] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const lastSizeRef = useRef({ width: 0, height: 0 });
  // Tracks the dock state and backend size invalidations already accounted for,
  // so a programmatic resize drops the cached measurement exactly once.
  const lastDockStateRef = useRef(dockState);
  const lastInvalidationRef = useRef(0);
  const [sizeInvalidation, setSizeInvalidation] = useState(0);
  const [dprComp, setDprComp] = useState(1);
  const scale = settings.scalePercent / 100;
  const layout = settings.overlayLayout === 'stacks' ? 'stacks' : 'grouped';
  const collapsed = Boolean(dockState?.docked && !dockState.expanded);
  // Claude has no credit balance; Antigravity shows credits only when reported.
  const claude = activeProvider === 'claude';
  const showCredits = settings.showCredits && ((activeProvider !== 'claude' && activeProvider !== 'antigravity') || Boolean(usage?.hasCredits));
  const showLogo = settings.showProviderLogo;
  const [switching, setSwitching] = useState(false);
  const order: Provider[] = ['codex', 'claude', 'dsh', 'antigravity'];
  const nextProvider = [1, 2, 3].map(offset => order[(order.indexOf(activeProvider) + offset) % 4])
    .find(candidate => monitors[candidate]?.available && monitors[candidate]?.monitorEnabled);
  const providerLabel = (target: Provider) => t(
    target === 'claude' ? 'providerClaude'
      : target === 'dsh' ? 'providerDsh'
        : target === 'antigravity' ? 'providerAntigravity'
          : 'providerCodex'
  );
  const logoHint = nextProvider ? t('switchProviderHint', { current: providerLabel(activeProvider), next: providerLabel(nextProvider) })
    : t('singleProviderHint', { current: providerLabel(activeProvider) });

  const interaction=(reason:'hover'|'focus',active:boolean)=>{void invoke('set_display_interaction',{reason,active}).catch(()=>{});};
  useEffect(()=>{
    const clear=()=>{interaction('hover',false);interaction('focus',false);};
    window.addEventListener('blur',clear);window.addEventListener('pagehide',clear);
    return()=>{window.removeEventListener('blur',clear);window.removeEventListener('pagehide',clear);clear();};
  },[]);
  // WebView2 renders content at devicePixelRatio = monitor DPI scale x system
  // text scaling (Windows "make text bigger"), while Tauri converts window
  // logical/physical sizes using the monitor DPI scale only. When the user
  // has text scaling != 100% the two diverge and the capsule overflows its
  // window. Compensate with comp = devicePixelRatio / window scaleFactor:
  // render the capsule at scale/comp and size the window at rect*comp, which
  // keeps the physical size identical on every machine.
  useEffect(() => {
    const currentWindow = getCurrentWindow();
    let unlisten: (() => void) | null = null;
    const update = async () => {
      try {
        const sf = await currentWindow.scaleFactor();
        setDprComp(window.devicePixelRatio > 0 ? window.devicePixelRatio / sf : 1);
      } catch {
        setDprComp(1);
      }
    };
    // WebView2 picks up a new monitor scale asynchronously, so devicePixelRatio
    // can still hold the old value when Tauri's scale-change event arrives (a
    // remote-desktop virtual display switches DPI exactly like this). Re-read
    // the ratio whenever it actually changes, or the compensation stays stale.
    let resolution: MediaQueryList | null = null;
    const onResolutionChange = () => { watchResolution(); void update(); };
    function watchResolution() {
      resolution?.removeEventListener('change', onResolutionChange);
      resolution = typeof window.matchMedia === 'function'
        ? window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`)
        : null;
      resolution?.addEventListener('change', onResolutionChange);
    }
    watchResolution();
    void update();
    currentWindow.onScaleChanged(() => { void update(); })
      .then((fn) => { unlisten = fn; })
      .catch(() => {});
    return () => {
      resolution?.removeEventListener('change', onResolutionChange);
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    const timer = setInterval(() => setTick((tick) => tick + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  // The backend resizes the window programmatically (provisional expand size,
  // show/hide). The cached content measurement is stale afterwards, so force a
  // fresh measurement instead of trusting the value cached before the resize.
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    listen('overlay_size_invalidated', () => {
      setSizeInvalidation((token) => token + 1);
    })
      .then((fn) => { if (active) unlisten = fn; else fn(); })
      .catch(() => {});
    return () => { active = false; if (unlisten) unlisten(); };
  }, []);

  const fitCapsuleSize = useCallback(async () => {
    if (collapsed) return;
    const el = rootRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const width = Math.ceil(rect.width);
    const height = Math.ceil(rect.height);
    if (width <= 0 || height <= 0 ||
      (width === lastSizeRef.current.width && height === lastSizeRef.current.height)) return;
    lastSizeRef.current = { width, height };
    try {
      await getCurrentWindow().setSize(new LogicalSize(width * dprComp, height * dprComp));
      // setSize grows the window right and down from its top-left corner, so the
      // backend re-places it after every size change: a docked capsule against
      // its edge, a free one at its anchor inside the work area.
      void invoke('dock_window_resized').catch((err) => console.error('Failed to reposition overlay:', err));
    } catch (err) {
      lastSizeRef.current = { width: 0, height: 0 };
      console.error('Failed to fit overlay size:', err);
    }
  }, [collapsed, dprComp]);

  useLayoutEffect(() => {
    // Collapse/expand, show/hide and backend invalidations all change the window
    // size programmatically: drop the cached measurement so the real capsule is
    // re-measured and its size written back (spec §3.4/§3.5).
    if (lastDockStateRef.current !== dockState || lastInvalidationRef.current !== sizeInvalidation) {
      lastDockStateRef.current = dockState;
      lastInvalidationRef.current = sizeInvalidation;
      lastSizeRef.current = { width: 0, height: 0 };
    }
    void fitCapsuleSize();
  }, [scale, layout, showCredits, claude, showLogo, settings.showFiveHourQuota, settings.showBurnRate, settings.language, usage, fitCapsuleSize, dprComp, dockState, sizeInvalidation]);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => { void fitCapsuleSize(); });
    observer.observe(el);
    return () => observer.disconnect();
  }, [fitCapsuleSize]);

  // The cached measurement only describes the content. Windows can resize the
  // window behind our back: after a display topology change (a remote-desktop
  // virtual display coming and going) it restores the window rectangle it
  // remembered for that topology. The content does not change, so neither the
  // ResizeObserver nor the cache would notice and the capsule would stay
  // cropped. The viewport always equals the last written size (within
  // rounding) unless someone else resized the window, so a mismatch re-fits.
  useEffect(() => {
    const onResize = () => {
      const { width, height } = lastSizeRef.current;
      const viewportWidth = window.innerWidth;
      const viewportHeight = window.innerHeight;
      // Nothing written yet (a fit is pending anyway), or minimized (0x0).
      if (collapsed || width === 0 || viewportWidth <= 0 || viewportHeight <= 0) return;
      if (Math.abs(viewportWidth - width) <= 1 && Math.abs(viewportHeight - height) <= 1) return;
      lastSizeRef.current = { width: 0, height: 0 };
      void fitCapsuleSize();
    };
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, [collapsed, fitCapsuleSize]);

  useEffect(() => {
    let active = true;
    document.fonts?.ready.then(() => { if (active) void fitCapsuleSize(); });
    return () => { active = false; };
  }, [fitCapsuleSize]);

  const handleContextMenu = async (event: React.MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    try {
      await invoke('show_overlay_menu');
    } catch (err) {
      console.error('Failed to open overlay menu:', err);
    }
  };

  const handleDragStart = (event: React.MouseEvent) => {
    if (event.button !== 0 || collapsed) return;
    event.preventDefault();
    void invoke('start_dragging').catch((err) => console.error('Failed to start overlay drag:', err));
  };

  const handleMouseEnter = () => {
    interaction('hover',true);
    if (dockState?.docked) void invoke('dock_mouse_enter').catch((err) => console.error(err));
  };

  const handleMouseLeave = () => {
    interaction('hover',false);
    if (dockState?.docked) void invoke('dock_mouse_leave').catch((err) => console.error(err));
  };

  const visibleQuotas = [
    { key: 'five', label: layout === 'stacks' ? t('fiveHourLabel') : t('fiveHourShort'),
      percent: usage?.fiveHourRemainingPercent,
      countdown: formatResetCountdown(usage?.fiveHourResetsAt),
      burnRate: formatBurnRate(usage?.fiveHourBurnRatePerHour) },
    { key: 'week', label: layout === 'stacks' ? t('weeklyLabel') : t('weeklyShort'),
      percent: usage?.weekRemainingPercent,
      countdown: formatResetCountdown(usage?.weekResetsAt, true),
      burnRate: formatBurnRate(usage?.weekBurnRatePerHour) },
  ].filter((quota) => quota.key !== 'five' || settings.showFiveHourQuota);

  if (collapsed) {
    const edge = dockState?.edge ?? 'left';
    return (
      <div className="overlay-shell overlay-shell-pill">
        <div
          ref={rootRef}
          className="overlay-surface overlay-pill-host"
          data-dock-edge={edge}
          role="group"
          aria-label={visibleQuotas.map((quota) => quota.key === 'five' ? t('fiveHourLabel') : t('weeklyLabel')).join(' / ')}
          onMouseEnter={handleMouseEnter}
          onMouseLeave={handleMouseLeave}
          onContextMenu={handleContextMenu}
          style={{
            '--overlay-alpha': (100 - settings.backgroundTransparencyPercent) / 100,
            transform: `scale(${scale / dprComp})`,
          } as React.CSSProperties}
        >
          <div className={`overlay-pill-bars ${edge === 'top' || edge === 'bottom' ? 'overlay-pill-rotated' : ''}`}>
            {visibleQuotas.map((quota) => {
              const percent = quota.percent === null || quota.percent === undefined
                ? null : Math.max(0, Math.min(100, quota.percent));
              return (
                <div
                  key={quota.key}
                  className="overlay-pill-track"
                  role="progressbar"
                  aria-label={quota.key === 'five' ? t('fiveHourLabel') : t('weeklyLabel')}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={percent ?? undefined}
                  aria-valuetext={percent === null ? t('unknown') : `${percent}%`}
                >
                  {percent !== null && (
                    <span
                      className={`overlay-pill-fill overlay-${usageTone(percent)}`}
                      style={{ height: `${percent}%` }}
                    />
                  )}
                  {settings.showPercentageGrid && PERCENTAGE_GRID_TICKS.map((bottom) => (
                    <span
                      key={bottom}
                      aria-hidden="true"
                      className="overlay-pill-tick"
                      style={{ bottom: `${bottom}%` }}
                    />
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="overlay-shell">
      <div
        ref={rootRef}
        data-layout={layout}
        onFocusCapture={()=>interaction('focus',true)}
        onBlurCapture={event=>{if(!event.currentTarget.contains(event.relatedTarget as Node|null))interaction('focus',false);}}
        onMouseDown={handleDragStart}
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
        onDoubleClick={(event) => { event.preventDefault(); event.stopPropagation(); }}
        onContextMenu={handleContextMenu}
        title={`${providerLabel(activeProvider)} · ${usage ? t('tooltipUpdated', { time: new Date(usage.fetchedAt * 1000).toLocaleTimeString() }) : t('loadingTooltip')}`}
        style={{
          '--overlay-alpha': (100 - settings.backgroundTransparencyPercent) / 100,
          transform: `scale(${scale / dprComp})`,
          transformOrigin: 'top left',
        } as React.CSSProperties}
        className={`overlay-surface overlay-capsule overlay-${layout}`}
      >
        {showLogo && (
          <>
            <button type="button" className="overlay-provider" aria-label={logoHint} title={logoHint}
              disabled={!nextProvider || !onCycleProvider || switching}
              onMouseDown={event => { event.stopPropagation(); }}
              onDoubleClick={event => { event.preventDefault(); event.stopPropagation(); }}
              onClick={async event => {
                event.stopPropagation();
                if (!nextProvider || !onCycleProvider || switching) return;
                setSwitching(true);
                try { await onCycleProvider(); } finally { setSwitching(false); }
              }}>
              <ProviderLogo provider={activeProvider} />
            </button>
            {layout === 'grouped' && <span aria-hidden="true" className="overlay-divider" />}
          </>
        )}
        <CapsuleReadings settings={settings} provider={activeProvider} usage={usage} isLoading={isLoading} />
      </div>
    </div>
  );
};

function CapsuleReadings({settings,provider,usage,isLoading}:{settings:OverlaySettings;provider:Provider;usage:CodexUsage|null;isLoading:boolean}) {
  const {t}=useTranslation();const layout=settings.overlayLayout;
  const raw=usage?.creditsDisplay??'—';const numeric=Number(raw);
  const formattedCredit = raw.trim() !== '' && Number.isFinite(numeric) ? numeric.toFixed(2) : raw;
  const credit = i18n.resolvedLanguage === 'ru-RU' ? formattedCredit.replace('.', ',') : formattedCredit;
  // Antigravity always reads 5H | Gemini WK | Argon. When Argon is the projected
  // pool, the usage's own weekly window (and its burn rate) belongs to Argon.
  const antigravity=provider==='antigravity';
  const argonPool=antigravity&&usage?.weeklyIsArgon===true;
  const ownWeekly={percent:usage?.weekRemainingPercent,countdown:formatResetCountdown(usage?.weekResetsAt,true),burn:formatBurnRate(usage?.weekBurnRatePerHour)};
  const weekly=argonPool?{percent:usage?.geminiWeekRemainingPercent,countdown:formatResetCountdown(usage?.geminiWeekResetsAt,true),burn:''}:ownWeekly;
  const argon=argonPool?ownWeekly:antigravity&&usage?.argonWeekRemainingPercent!=null
    ?{percent:usage.argonWeekRemainingPercent,countdown:formatResetCountdown(usage.argonWeekResetsAt,true),burn:''}:null;
  const quotas=[
    {key:'five',label:layout==='stacks'?t('fiveHourLabel'):t('fiveHourShort'),percent:usage?.fiveHourRemainingPercent,countdown:formatResetCountdown(usage?.fiveHourResetsAt),burn:formatBurnRate(usage?.fiveHourBurnRatePerHour)},
    {key:'week',label:layout==='stacks'?t('weeklyLabel'):t('weeklyShort'),...weekly},
    ...(argon?[{key:'argon',label:layout==='stacks'?t('argonLabel'):t('argonShort'),...argon}]:[]),
  ].filter(q=>q.key!=='five'||settings.showFiveHourQuota);
  const quotaName=(key:string)=>key==='five'?t('fiveHourLabel'):key==='argon'?t('argonLabel'):t('weeklyLabel');
  return <>
    {quotas.map((quota,index)=><React.Fragment key={quota.key}>
      {index>0&&layout==='grouped'&&<span aria-hidden="true" className="overlay-divider"/>}
      <div className="overlay-quota" role="group" aria-label={quotaName(quota.key)}>
        <div className="overlay-reading"><span className="overlay-label">{quota.label}</span><span className={`overlay-value overlay-${usageTone(quota.percent)}`}>{quota.percent==null?'--%':`${quota.percent}%`}</span></div>
        <div className="overlay-meta-row"><span className="overlay-countdown">{quota.countdown}</span>{settings.showBurnRate&&quota.burn&&<span className="overlay-burn-rate" title={quota.key==='five'?burnRatePresentation(usage,t):undefined}>{quota.burn}</span>}</div>
      </div>
    </React.Fragment>)}
    {settings.showCredits&&((provider!=='claude'&&provider!=='antigravity')||Boolean(usage?.hasCredits))&&<>
      {layout==='grouped'&&<span aria-hidden="true" className="overlay-divider"/>}
      <div className="overlay-credit" role="group" aria-label={t('creditsLabel')}><div className="overlay-reading"><span className="overlay-label">{layout==='stacks'?t('creditsLabel'):t('creditsShort')}</span><span className="overlay-value">{credit}</span></div>{layout==='stacks'&&<span className="overlay-countdown">{t('balanceLabel')}</span>}</div>
    </>}
    {isLoading&&<span role="status" aria-label={t('refreshing')} className="overlay-loading"/>}
  </>;
}
