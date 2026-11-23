import type { TFunction } from 'i18next';
import type { OverlaySettings, Provider } from './types';
import { providerPreference } from './providerSettings';

export type HistoryRange = '24h' | '7d' | '30d' | '90d';
export const HISTORY_RANGES: HistoryRange[] = ['24h', '7d', '30d', '90d'];
export const RANGE_SECONDS: Record<HistoryRange, number> = {
  '24h': 24 * 3600,
  '7d': 7 * 24 * 3600,
  '30d': 30 * 24 * 3600,
  '90d': 90 * 24 * 3600,
};

export type WindowKind = 'fiveHour' | 'weekly' | 'argon';
export const WINDOW_KINDS: WindowKind[] = ['fiveHour', 'weekly', 'argon'];

export interface HistorySeriesInfo {
  series: string;
  provider: Provider;
  firstAt: number | null;
  lastAt: number | null;
  records: number;
}

export interface HistoryPoint {
  t: number;
  seg: number;
  fiveHour: number | null;
  weekly: number | null;
  argon: number | null;
  pool?: string;
}

export interface HistoryResetMarker { t: number; kind: WindowKind }
export interface HistoryGap { from: number; to: number }

export interface HistoryDay {
  day: string;
  start: number;
  fiveHour: number;
  weekly: number;
  argon: number;
  approximate: boolean;
}

export interface HistoryWindow {
  kind: WindowKind;
  start: number;
  resetsAt: number;
  firstSeen: number;
  lastSeen: number;
  peak: number;
  hitLimit: boolean;
  limitAt: number | null;
  timeToLimitSecs: number | null;
  unusedAtReset: number | null;
  complete: boolean;
  pool?: string;
}

export interface HistoryViewData {
  series: string;
  provider: Provider;
  from: number;
  to: number;
  recordingSince: number | null;
  points: HistoryPoint[];
  resets: HistoryResetMarker[];
  gaps: HistoryGap[];
  days: HistoryDay[];
  windows: HistoryWindow[];
  hasArgon: boolean;
}

export const KIND_LABEL_KEY: Record<WindowKind, string> = {
  fiveHour: 'historySeriesFiveHour',
  weekly: 'historySeriesWeekly',
  argon: 'historySeriesArgon',
};

/** Categorical slot per window kind; fixed, so a kind keeps its colour whatever else is shown. */
export const KIND_COLOR: Record<WindowKind, string> = {
  fiveHour: 'var(--viz-series-1)',
  weekly: 'var(--viz-series-2)',
  argon: 'var(--viz-series-3)',
};

const PROVIDER_ORDER: Provider[] = ['codex', 'claude', 'dsh', 'antigravity'];
const PROVIDER_LABEL_KEY: Record<Provider, string> = {
  codex: 'providerCodex',
  claude: 'providerClaude',
  dsh: 'providerDsh',
  antigravity: 'providerAntigravity',
};

export function sortSeries(list: HistorySeriesInfo[]): HistorySeriesInfo[] {
  return [...list].sort((a, b) =>
    PROVIDER_ORDER.indexOf(a.provider) - PROVIDER_ORDER.indexOf(b.provider)
    || (b.lastAt ?? 0) - (a.lastAt ?? 0)
    || a.series.localeCompare(b.series));
}

/** Tab labels; several DSH series are numbered by recency. */
export function seriesLabels(list: HistorySeriesInfo[], t: TFunction): Record<string, string> {
  const labels: Record<string, string> = {};
  const dsh = list.filter(item => item.provider === 'dsh');
  for (const item of list) {
    const base = t(PROVIDER_LABEL_KEY[item.provider]);
    labels[item.series] = item.provider === 'dsh' && dsh.length > 1 ? `${base} ${dsh.indexOf(item) + 1}` : base;
  }
  return labels;
}

/** Protection thresholds as used percent, for the kinds that are protected. */
export function usedThresholds(settings: OverlaySettings, provider: Provider): Partial<Record<WindowKind, number>> {
  const pref = providerPreference(settings, provider);
  if (!pref.protectionEnabled) return {};
  const out: Partial<Record<WindowKind, number>> = {};
  if (pref.fiveHourThresholdPercent > 0) out.fiveHour = 100 - pref.fiveHourThresholdPercent;
  if (pref.weeklyThresholdPercent > 0) out.weekly = 100 - pref.weeklyThresholdPercent;
  return out;
}

export function formatPercent(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—';
  return `${value >= 10 || Number.isInteger(value) ? Math.round(value) : value.toFixed(1)}%`;
}

export function formatDuration(seconds: number, t: TFunction): string {
  const minutes = Math.max(0, Math.round(seconds / 60));
  const hours = Math.floor(minutes / 60);
  if (hours >= 24) return t('durationDaysHours', { d: Math.floor(hours / 24), h: hours % 24 });
  return t('durationHoursMinutes', { h: hours, m: minutes % 60 });
}

export function formatDateTime(t: number, locale: string | undefined, withDate = true): string {
  return new Intl.DateTimeFormat(locale, withDate
    ? { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }
    : { hour: '2-digit', minute: '2-digit' }).format(new Date(t * 1000));
}

export function formatDay(start: number, locale: string | undefined): string {
  return new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' }).format(new Date(start * 1000));
}

export interface TimeTick { t: number; label: string }

const HOUR = 3600;
const TICK_STEPS = [HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, 24 * HOUR, 2 * 24 * HOUR, 5 * 24 * HOUR, 7 * 24 * HOUR, 15 * 24 * HOUR, 30 * 24 * HOUR];

/** Local-time ticks aligned to whole hours or local midnights, at most `max`. */
export function timeTicks(from: number, to: number, locale: string | undefined, max = 8): TimeTick[] {
  const span = Math.max(1, to - from);
  const step = TICK_STEPS.find(s => span / s <= max) ?? TICK_STEPS[TICK_STEPS.length - 1];
  const d = new Date(from * 1000);
  d.setMinutes(0, 0, 0);
  const days = step >= 24 * HOUR ? step / (24 * HOUR) : 0;
  if (days) d.setHours(0);
  else d.setHours(Math.ceil(d.getHours() / (step / HOUR)) * (step / HOUR));
  const ticks: TimeTick[] = [];
  for (let guard = 0; guard < 200; guard += 1) {
    const t = Math.floor(d.getTime() / 1000);
    if (t > to) break;
    if (t >= from) {
      const midnight = d.getHours() === 0;
      ticks.push({ t, label: days || midnight ? formatDay(t, locale) : formatDateTime(t, locale, false) });
    }
    if (days) d.setDate(d.getDate() + days);
    else d.setHours(d.getHours() + step / HOUR);
  }
  return ticks;
}

/**
 * SVG path data for one series: a new subpath wherever the line breaks
 * (a gap, an account or pool switch) or the series has no reading.
 */
export function linePath(
  points: HistoryPoint[],
  kind: WindowKind,
  x: (t: number) => number,
  y: (u: number) => number,
): string {
  let d = '';
  let prevSeg: number | null = null;
  for (const point of points) {
    const value = point[kind];
    if (value === null || value === undefined) { prevSeg = null; continue; }
    const command = prevSeg === point.seg ? 'L' : 'M';
    d += `${command}${x(point.t).toFixed(1)},${y(value).toFixed(1)}`;
    prevSeg = point.seg;
  }
  return d;
}

/** Readings with no neighbour in their line segment, which a path alone would not show. */
export function isolatedPoints(points: HistoryPoint[], kind: WindowKind): HistoryPoint[] {
  const has = (p: HistoryPoint | undefined) => p !== undefined && p[kind] !== null && p[kind] !== undefined;
  return points.filter((p, i) => has(p)
    && !(has(points[i - 1]) && points[i - 1].seg === p.seg)
    && !(has(points[i + 1]) && points[i + 1].seg === p.seg));
}

/** Index of the point closest in time to `t`; -1 when there are none. */
export function nearestIndex(points: HistoryPoint[], t: number): number {
  if (!points.length) return -1;
  let lo = 0;
  let hi = points.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid].t < t) lo = mid + 1; else hi = mid;
  }
  if (lo > 0 && Math.abs(points[lo - 1].t - t) <= Math.abs(points[lo].t - t)) return lo - 1;
  return lo;
}

const NICE_STEPS = [5, 10, 20, 25, 50, 100, 200, 250, 500];

/** A 0-based axis with round ticks (at most five intervals) covering `value`. */
export function niceScale(value: number): { max: number; ticks: number[] } {
  const target = Math.max(value, 20);
  const step = NICE_STEPS.find(s => target / s <= 5) ?? Math.ceil(target / 500) * 100;
  const max = Math.ceil(target / step) * step;
  const ticks: number[] = [];
  for (let tick = 0; tick <= max; tick += step) ticks.push(tick);
  return { max, ticks };
}

/**
 * Label positions for horizontal reference lines: above the line, unless that
 * would overlap the previous label, in which case below it.
 */
export function stackLabels(ys: number[], minGap = 12): number[] {
  const order = ys.map((y, i) => ({ y, i })).sort((a, b) => a.y - b.y);
  const out = new Array<number>(ys.length);
  let last = -Infinity;
  for (const { y, i } of order) {
    let label = y - 4;
    if (label - last < minGap) label = y + 12;
    out[i] = label;
    last = label;
  }
  return out;
}
