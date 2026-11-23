import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useTranslation } from 'react-i18next';
import type { OverlaySettings } from '../types';
import {
  HISTORY_RANGES, HistoryRange, HistorySeriesInfo, HistoryViewData, RANGE_SECONDS, WINDOW_KINDS, WindowKind,
  formatDay, seriesLabels, sortSeries, usedThresholds,
} from '../history';
import { TimelineChart } from './history/TimelineChart';
import { DailyBars } from './history/DailyBars';
import { WindowTable } from './history/WindowTable';

/** New readings arrive about once a minute; refetch at most this often. */
const REFRESH_THROTTLE_MS = 15_000;

function presentKinds(view: HistoryViewData): WindowKind[] {
  return WINDOW_KINDS.filter(kind =>
    view.points.some(point => point[kind] !== null && point[kind] !== undefined)
    || view.windows.some(window => window.kind === kind)
    || view.days.some(day => day[kind] > 0));
}

export const HistoryView: React.FC<{ settings: OverlaySettings }> = ({ settings }) => {
  const { t, i18n } = useTranslation();
  const [seriesList, setSeriesList] = useState<HistorySeriesInfo[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [range, setRange] = useState<HistoryRange>('7d');
  const [view, setView] = useState<HistoryViewData | null>(null);
  const [state, setState] = useState<'loading' | 'ready' | 'error'>('loading');
  const request = useRef(0);
  const lastRefresh = useRef(0);

  useEffect(() => {
    document.title = t('historyTitle');
    void getCurrentWindow().setTitle(t('historyTitle')).catch(() => {});
  }, [t]);

  const loadSeries = useCallback(async () => {
    try {
      const list = sortSeries(await invoke<HistorySeriesInfo[]>('get_usage_history_series'));
      setSeriesList(list);
      setSelected(current => {
        if (current && list.some(item => item.series === current)) return current;
        return list.find(item => item.provider === settings.provider)?.series ?? list[0]?.series ?? null;
      });
      if (!list.length) setState('ready');
    } catch (error) {
      console.error('Could not list usage history:', error);
      setState('error');
    }
  }, [settings.provider]);

  const loadView = useCallback(async (series: string, selectedRange: HistoryRange, quiet = false) => {
    const token = ++request.current;
    if (!quiet) setState('loading');
    try {
      const next = await invoke<HistoryViewData>('get_usage_history', { series, rangeSeconds: RANGE_SECONDS[selectedRange] });
      if (token !== request.current) return;
      setView(next);
      setState('ready');
    } catch (error) {
      if (token !== request.current) return;
      console.error('Could not load usage history:', error);
      setState('error');
    }
  }, []);

  useEffect(() => { void loadSeries(); }, [loadSeries]);
  useEffect(() => { if (selected) void loadView(selected, range); }, [selected, range, loadView]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    void listen('monitor_updated', () => {
      const now = Date.now();
      if (now - lastRefresh.current < REFRESH_THROTTLE_MS) return;
      lastRefresh.current = now;
      void loadSeries();
      if (selected) void loadView(selected, range, true);
    }).then(fn => { if (active) unlisten = fn; else fn(); });
    return () => { active = false; unlisten?.(); };
  }, [selected, range, loadSeries, loadView]);

  const labels = useMemo(() => seriesLabels(seriesList ?? [], t), [seriesList, t]);
  const kinds = view ? presentKinds(view) : [];
  const thresholds = view ? usedThresholds(settings, view.provider) : {};
  const locale = i18n.resolvedLanguage;

  return (
    <div className="viz-root h-screen flex flex-col bg-[#F5F6F8] dark:bg-slate-900 text-slate-800 dark:text-slate-100 font-sans">
      <header className="shrink-0 flex flex-wrap items-center gap-3 px-5 pt-4 pb-3 border-b border-slate-200 dark:border-slate-700">
        <h1 className="text-base font-semibold mr-2">{t('historyTitle')}</h1>
        {seriesList && seriesList.length > 0 && (
          <nav role="tablist" aria-label={t('historyProviders')} className="flex gap-1">
            {seriesList.map(item => (
              <button
                key={item.series}
                type="button"
                role="tab"
                aria-selected={selected === item.series}
                onClick={() => setSelected(item.series)}
                className={`rounded-lg px-3 py-1 text-sm font-medium ${selected === item.series ? 'bg-white dark:bg-slate-800 text-cyan-700 dark:text-cyan-300 border border-slate-200 dark:border-slate-700' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-100'}`}
              >
                {labels[item.series]}
              </button>
            ))}
          </nav>
        )}
        <div role="group" aria-label={t('historyRangeLabel')} className="ml-auto flex gap-1">
          {HISTORY_RANGES.map(option => (
            <button
              key={option}
              type="button"
              aria-pressed={range === option}
              onClick={() => setRange(option)}
              className={`rounded-md px-2.5 py-1 text-xs font-medium border ${range === option ? 'border-cyan-600 bg-cyan-600 text-white' : 'border-slate-300 dark:border-slate-600'}`}
            >
              {t(`historyRange${option}`)}
            </button>
          ))}
        </div>
      </header>
      <main className="flex-1 overflow-y-auto px-5 py-4 space-y-4">
        {!settings.recordUsageHistory && <p role="status" className="text-sm rounded-lg border border-amber-300 bg-amber-50 dark:bg-amber-950/40 dark:border-amber-700 px-3 py-2">{t('historyDisabled')}</p>}
        {state === 'error' && (
          <div role="alert" className="text-sm space-y-2">
            <p>{t('historyLoadFailed')}</p>
            <button type="button" className="rounded-md bg-cyan-600 px-3 py-1 text-white" onClick={() => { void loadSeries(); if (selected) void loadView(selected, range); }}>{t('retry')}</button>
          </div>
        )}
        {state !== 'error' && seriesList !== null && seriesList.length === 0 && <p role="status" className="text-sm" style={{ color: 'var(--viz-ink-2)' }}>{t('historyEmpty')}</p>}
        {state === 'loading' && !view && <p role="status" className="text-sm" style={{ color: 'var(--viz-ink-2)' }}>{t('historyLoading')}</p>}
        {view && seriesList && seriesList.length > 0 && (
          <>
            {view.recordingSince !== null && (
              <p className="text-xs" style={{ color: 'var(--viz-ink-2)' }}>{t('historyRecordingSince', { date: formatDay(view.recordingSince, locale) })}</p>
            )}
            <section className="viz-card p-4 space-y-2" aria-labelledby="history-timeline-title">
              <h2 id="history-timeline-title" className="text-sm font-semibold">{t('historyTimelineTitle')}</h2>
              <p className="text-xs" style={{ color: 'var(--viz-ink-2)' }}>{t('historyTimelineHint')}</p>
              <TimelineChart view={view} kinds={kinds} thresholds={thresholds} />
            </section>
            <section className="viz-card p-4 space-y-2" aria-labelledby="history-daily-title">
              <h2 id="history-daily-title" className="text-sm font-semibold">{t('historyDailyTitle')}</h2>
              <p className="text-xs" style={{ color: 'var(--viz-ink-2)' }}>{t('historyDailyHint')}</p>
              <DailyBars days={view.days} kinds={kinds} />
            </section>
            <section className="viz-card p-4 space-y-2" aria-labelledby="history-windows-title">
              <h2 id="history-windows-title" className="text-sm font-semibold">{t('historyWindowsTitle')}</h2>
              <WindowTable windows={view.windows} kinds={kinds} />
            </section>
          </>
        )}
      </main>
    </div>
  );
};
