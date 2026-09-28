import React, { useEffect, useLayoutEffect, useState, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { CodexUsage, DockStateInfo, OverlaySettings } from '../types';
import { useTranslation } from 'react-i18next';

interface OverlayViewProps {
  settings: OverlaySettings;
  usage: CodexUsage | null;
  isLoading: boolean;
  dockState?: DockStateInfo;
}

function formatResetCountdown(resetsAt: number | null | undefined, weekly = false): string {
  if (resetsAt === null || resetsAt === undefined) return '--h --m';
  const remainingSeconds = Math.max(0, resetsAt - Math.floor(Date.now() / 1000));
  const totalMinutes = Math.ceil(remainingSeconds / 60);
  const hours = Math.floor(totalMinutes / 60);
  if (weekly && hours >= 24) {
    return `${Math.floor(hours / 24)}d ${(hours % 24).toString().padStart(2, '0')}h`;
  }
  return `${hours}h ${(totalMinutes % 60).toString().padStart(2, '0')}m`;
}

function usageTone(percent: number | null | undefined): string {
  if (percent === null || percent === undefined) return 'unknown';
  return percent >= 50 ? 'healthy' : percent >= 20 ? 'warning' : 'low';
}

export const OverlayView: React.FC<OverlayViewProps> = ({ settings, usage, isLoading, dockState }) => {
  const { t } = useTranslation();
  const [, setTick] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const lastSizeRef = useRef({ width: 0, height: 0 });
  const [dprComp, setDprComp] = useState(1);
  const scale = settings.scalePercent / 100;
  const layout = settings.overlayLayout === 'stacks' ? 'stacks' : 'grouped';
  const collapsed = Boolean(dockState?.docked && !dockState.expanded);

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
    void update();
    currentWindow.onScaleChanged(() => { void update(); })
      .then((fn) => { unlisten = fn; })
      .catch(() => {});
    return () => { if (unlisten) unlisten(); };
  }, []);

  useEffect(() => {
    const timer = setInterval(() => setTick((tick) => tick + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  const rawCredit = usage?.creditsDisplay ?? '—';
  const numericCredit = Number(rawCredit);
  const formattedCredit = rawCredit.trim() !== '' && Number.isFinite(numericCredit)
    ? numericCredit.toFixed(2) : rawCredit;

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
      if (dockState?.docked && dockState.expanded) {
        void invoke('dock_window_resized').catch((err) => console.error('Failed to reposition docked overlay:', err));
      }
    } catch (err) {
      lastSizeRef.current = { width: 0, height: 0 };
      console.error('Failed to fit overlay size:', err);
    }
  }, [collapsed, dockState?.docked, dockState?.expanded, dprComp]);

  useLayoutEffect(() => {
    void fitCapsuleSize();
  }, [scale, layout, settings.showCredits, settings.language, usage, fitCapsuleSize, dprComp]);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => { void fitCapsuleSize(); });
    observer.observe(el);
    return () => observer.disconnect();
  }, [fitCapsuleSize]);

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
    if (dockState?.docked) void invoke('dock_mouse_enter').catch((err) => console.error(err));
  };

  const handleMouseLeave = () => {
    if (dockState?.docked) void invoke('dock_mouse_leave').catch((err) => console.error(err));
  };

  const quotas = [
    { key: 'five', label: layout === 'stacks' ? t('fiveHourLabel') : '5H',
      percent: usage?.fiveHourRemainingPercent,
      countdown: formatResetCountdown(usage?.fiveHourResetsAt) },
    { key: 'week', label: layout === 'stacks' ? t('weeklyLabel') : 'WK',
      percent: usage?.weekRemainingPercent,
      countdown: formatResetCountdown(usage?.weekResetsAt, true) },
  ];

  if (collapsed) {
    const edge = dockState?.edge ?? 'left';
    return (
      <div className="overlay-shell overlay-shell-pill">
        <div
          ref={rootRef}
          className="overlay-surface overlay-pill-host"
          data-dock-edge={edge}
          role="group"
          aria-label={`${t('fiveHourLabel')} and ${t('weeklyLabel')}`}
          onMouseEnter={handleMouseEnter}
          onMouseLeave={handleMouseLeave}
          onContextMenu={handleContextMenu}
          style={{
            '--overlay-alpha': (100 - settings.backgroundTransparencyPercent) / 100,
            transform: `scale(${scale / dprComp})`,
          } as React.CSSProperties}
        >
          <div className={`overlay-pill-bars ${edge === 'top' || edge === 'bottom' ? 'overlay-pill-rotated' : ''}`}>
            {quotas.map((quota) => {
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
        onMouseDown={handleDragStart}
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
        onDoubleClick={(event) => { event.preventDefault(); event.stopPropagation(); }}
        onContextMenu={handleContextMenu}
        title={usage ? t('tooltipUpdated', { time: new Date(usage.fetchedAt * 1000).toLocaleTimeString() }) : t('loadingTooltip')}
        style={{
          '--overlay-alpha': (100 - settings.backgroundTransparencyPercent) / 100,
          transform: `scale(${scale / dprComp})`,
          transformOrigin: 'top left',
        } as React.CSSProperties}
        className={`overlay-surface overlay-capsule overlay-${layout}`}
      >
        {quotas.map((quota, index) => (
          <React.Fragment key={quota.key}>
            {index > 0 && layout === 'grouped' && <span aria-hidden="true" className="overlay-divider" />}
            <div className="overlay-quota" role="group" aria-label={quota.key === 'five' ? t('fiveHourLabel') : t('weeklyLabel')}>
              <div className="overlay-reading">
                <span className="overlay-label">{quota.label}</span>
                <span className={`overlay-value overlay-${usageTone(quota.percent)}`}>
                  {quota.percent === null || quota.percent === undefined ? '--%' : `${quota.percent}%`}
                </span>
              </div>
              <span className="overlay-countdown">{quota.countdown}</span>
            </div>
          </React.Fragment>
        ))}
        {settings.showCredits && (
          <>
            {layout === 'grouped' && <span aria-hidden="true" className="overlay-divider" />}
            <div className="overlay-credit" role="group" aria-label={t('creditsLabel')}>
              <div className="overlay-reading">
                <span className="overlay-label">{layout === 'stacks' ? t('creditsLabel') : 'CR'}</span>
                <span className="overlay-value">{formattedCredit}</span>
              </div>
              {layout === 'stacks' && <span className="overlay-countdown">{t('balanceLabel')}</span>}
            </div>
          </>
        )}
        {isLoading && <span role="status" aria-label={t('refreshing')} className="overlay-loading" />}
      </div>
    </div>
  );
};
