import React, { useEffect, useLayoutEffect, useState, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { CodexUsage, OverlaySettings } from '../types';
import { useTranslation } from 'react-i18next';

interface OverlayViewProps {
  settings: OverlaySettings;
  usage: CodexUsage | null;
  isLoading: boolean;
  onRefresh: () => void;
  onOpenSettings: () => void;
}

function formatResetCountdown(resetsAt: number | null | undefined): string {
  if (!resetsAt) return '--H --min';
  const nowSeconds = Math.floor(Date.now() / 1000);
  const remainingSeconds = Math.max(0, resetsAt - nowSeconds);
  const totalMinutes = Math.ceil(remainingSeconds / 60);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return `${hours}H ${minutes.toString().padStart(2, '0')}min`;
}

function formatWeekResetCountdown(resetsAt: number | null | undefined): string {
  if (!resetsAt) return '--H --min';
  const nowSeconds = Math.floor(Date.now() / 1000);
  const remainingSeconds = Math.max(0, resetsAt - nowSeconds);
  const totalMinutes = Math.ceil(remainingSeconds / 60);
  const totalHours = Math.floor(totalMinutes / 60);

  if (totalHours >= 24) {
    const days = Math.floor(totalHours / 24);
    const hours = totalHours % 24;
    return `${days}D ${hours.toString().padStart(2, '0')}H`;
  } else {
    const hours = totalHours;
    const minutes = totalMinutes % 60;
    return `${hours.toString().padStart(2, '0')}H ${minutes.toString().padStart(2, '0')}min`;
  }
}

function getUsageColor(percent: number | null | undefined): string {
  if (percent === null || percent === undefined) return '#FFFFFF';
  if (percent >= 50) return '#52D273';
  if (percent >= 20) return '#FFC857';
  return '#FF5C5C';
}

export const OverlayView: React.FC<OverlayViewProps> = ({
  settings,
  usage,
  isLoading,
  onRefresh,
  onOpenSettings,
}) => {
  const { t } = useTranslation();
  const [, setTick] = useState(0);
  const [contextMenuPos, setContextMenuPos] = useState<{ x: number; y: number } | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const lastSizeRef = useRef({ width: 0, height: 0 });

  // Update countdown every second
  useEffect(() => {
    const timer = setInterval(() => {
      setTick((t) => t + 1);
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  // Format Credits value
  const formattedCredit = React.useMemo(() => {
    if (!usage) return '—';
    const raw = usage.creditsDisplay ?? '—';
    const num = parseFloat(raw);
    if (!isNaN(num) && raw !== '∞' && raw !== '—') {
      return num.toFixed(2);
    }
    return raw;
  }, [usage]);

  const fiveHourCountdown = formatResetCountdown(usage?.fiveHourResetsAt);
  const weekCountdown = formatWeekResetCountdown(usage?.weekResetsAt);

  const fiveColor = getUsageColor(usage?.fiveHourRemainingPercent);
  const weekColor = getUsageColor(usage?.weekRemainingPercent);

  const backgroundAlpha = (100 - settings.backgroundTransparencyPercent) / 100;
  const scale = settings.scalePercent / 100;

  // Shrink window to exact capsule dimensions with change threshold
  const fitCapsuleSize = useCallback(async () => {
    const el = rootRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const targetWidth = Math.ceil(rect.width);
    const targetHeight = Math.ceil(rect.height);
    if (targetWidth > 0 && targetHeight > 0) {
      if (
        Math.abs(targetWidth - lastSizeRef.current.width) > 1 ||
        Math.abs(targetHeight - lastSizeRef.current.height) > 1
      ) {
        lastSizeRef.current = { width: targetWidth, height: targetHeight };
        try {
          const win = getCurrentWindow();
          await win.setSize(new LogicalSize(targetWidth, targetHeight));
        } catch (err) {
          console.error('Failed to fit capsule size:', err);
        }
      }
    }
  }, []);

  // Adjust window size whenever scale, showCredits, or usage changes
  useLayoutEffect(() => {
    if (!contextMenuPos) {
      fitCapsuleSize();
    }
  }, [scale, settings.showCredits, usage, fitCapsuleSize, contextMenuPos]);

  // Observe DOM size changes (e.g. font loaded or text length change)
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      if (!contextMenuPos) {
        fitCapsuleSize();
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [fitCapsuleSize, contextMenuPos]);

  // Adjust when custom fonts are ready
  useEffect(() => {
    document.fonts.ready.then(() => {
      if (!contextMenuPos) {
        fitCapsuleSize();
      }
    });
  }, [fitCapsuleSize, contextMenuPos]);

  const closeContextMenu = useCallback(async () => {
    setContextMenuPos(null);
    await fitCapsuleSize();
  }, [fitCapsuleSize]);

  // Dismiss context menu on window blur (clicking elsewhere on desktop) or Escape
  useEffect(() => {
    if (!contextMenuPos) return;

    const handleBlur = () => {
      closeContextMenu();
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        closeContextMenu();
      }
    };

    window.addEventListener('blur', handleBlur);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('blur', handleBlur);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [contextMenuPos, closeContextMenu]);

  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    if (contextMenuPos) {
      closeContextMenu();
    }
  };

  const handleDoubleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
  };

  const handleContextMenu = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Position menu directly below or near the click
    const menuX = Math.max(0, Math.min(e.clientX, 60));
    const menuY = Math.ceil(44 * scale) + 4;

    setContextMenuPos({ x: menuX, y: menuY });

    // Expand window so custom context menu is fully visible
    try {
      const win = getCurrentWindow();
      const el = rootRef.current;
      const rect = el ? el.getBoundingClientRect() : { width: 170, height: 48 };
      const expW = Math.max(Math.ceil(rect.width), 175);
      const expH = Math.ceil(rect.height) + 165;
      lastSizeRef.current = { width: expW, height: expH };
      await win.setSize(new LogicalSize(expW, expH));
    } catch (err) {
      console.error('Failed to expand window for menu:', err);
    }
  };

  const handleHideWindow = async () => {
    await closeContextMenu();
    const win = getCurrentWindow();
    await win.hide();
  };

  const handleExit = async () => {
    await closeContextMenu();
    await invoke('exit_app');
  };

  const tooltipText = usage
    ? t('tooltipUpdated', { time: new Date(usage.fetchedAt * 1000).toLocaleTimeString() })
    : t('loadingTooltip');

  return (
    <div className="relative inline-block m-0 p-0 select-none">
      {/* Draggable Capsule */}
      <div
        ref={rootRef}
        data-tauri-drag-region
        onMouseDown={handleMouseDown}
        onDoubleClick={handleDoubleClick}
        onContextMenu={handleContextMenu}
        title={tooltipText}
        style={{
          backgroundColor: `rgba(27, 29, 36, ${backgroundAlpha})`,
          transform: `scale(${scale})`,
          transformOrigin: 'top left',
        }}
        className="inline-block rounded-xl border border-[#4C5362]/40 px-2 py-1 shadow-lg backdrop-blur-md cursor-grab active:cursor-grabbing transition-colors duration-150 select-none"
      >
        {/* Row 1: Usage Numbers */}
        <div data-tauri-drag-region className="flex items-center text-sm font-semibold tracking-tight whitespace-nowrap pointer-events-none">
          {/* 5H */}
          <span className="text-white text-xs mr-1 opacity-90">5H</span>
          <span
            style={{ color: fiveColor }}
            className="text-base min-w-[38px] text-left tabular-nums"
          >
            {usage?.fiveHourRemainingPercent !== null && usage?.fiveHourRemainingPercent !== undefined
              ? `${usage.fiveHourRemainingPercent}%`
              : '--%'}
          </span>

          <span className="mx-1 text-white/30 text-xs">|</span>

          {/* WK */}
          <span className="text-white text-xs mr-1 opacity-90">WK</span>
          <span
            style={{ color: weekColor }}
            className="text-base min-w-[38px] text-left tabular-nums"
          >
            {usage?.weekRemainingPercent !== null && usage?.weekRemainingPercent !== undefined
              ? `${usage.weekRemainingPercent}%`
              : '--%'}
          </span>

          {/* Credits */}
          {settings.showCredits && (
            <>
              <span className="mx-1 text-white/30 text-xs">|</span>
              <span className="text-white text-xs mr-1 opacity-90">CR</span>
              <span className="text-white text-base tabular-nums">
                {formattedCredit}
              </span>
            </>
          )}

          {isLoading && (
            <span className="ml-1.5 inline-block w-1.5 h-1.5 rounded-full bg-cyan-400 animate-ping" />
          )}
        </div>

        {/* Row 2: Reset Countdowns */}
        <div data-tauri-drag-region className="flex items-center text-[11px] font-bold text-[#C4CCD8] mt-0.5 tracking-tight whitespace-nowrap pointer-events-none">
          <span className="tabular-nums">{fiveHourCountdown}</span>
          <span className="mx-2 opacity-50">&bull;</span>
          <span className="tabular-nums">{weekCountdown}</span>
        </div>
      </div>

      {/* Custom Context Menu & Dismissal Backdrop */}
      {contextMenuPos && (
        <>
          {/* Transparent Backdrop to dismiss menu on outside click */}
          <div
            className="fixed inset-0 z-40 bg-transparent"
            onClick={closeContextMenu}
            onContextMenu={(e) => {
              e.preventDefault();
              closeContextMenu();
            }}
          />

          {/* Styled Context Menu */}
          <div
            style={{
              top: contextMenuPos.y,
              left: contextMenuPos.x,
            }}
            className="fixed z-50 min-w-[145px] rounded-lg border border-slate-700/80 bg-slate-900/95 py-1 text-xs text-slate-200 shadow-2xl backdrop-blur-md select-none animate-in fade-in zoom-in-95 duration-100"
            onClick={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
          >
            <button
              onClick={() => {
                closeContextMenu();
                onRefresh();
              }}
              className="w-full text-left px-3 py-1.5 hover:bg-slate-800/80 transition-colors flex items-center gap-2"
            >
              <span>{t('refreshNow')}</span>
            </button>
            <button
              onClick={() => {
                closeContextMenu();
                onOpenSettings();
              }}
              className="w-full text-left px-3 py-1.5 hover:bg-slate-800/80 transition-colors flex items-center gap-2"
            >
              <span>{t('settings')}</span>
            </button>
            <div className="my-1 border-t border-slate-700/50" />
            <button
              onClick={handleHideWindow}
              className="w-full text-left px-3 py-1.5 hover:bg-slate-800/80 transition-colors flex items-center gap-2"
            >
              <span>{t('hide')}</span>
            </button>
            <div className="my-1 border-t border-slate-700/50" />
            <button
              onClick={handleExit}
              className="w-full text-left px-3 py-1.5 hover:bg-red-500/20 text-red-300 transition-colors flex items-center gap-2"
            >
              <span>{t('exit')}</span>
            </button>
          </div>
        </>
      )}
    </div>
  );
};
