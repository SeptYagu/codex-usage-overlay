import { describe, expect, it } from 'vitest';
import i18n from './i18n';
import { DEFAULT_SETTINGS } from './types';
import {
  HistoryPoint, formatDuration, isolatedPoints, linePath, nearestIndex, niceScale, seriesLabels, sortSeries, stackLabels, timeTicks, usedThresholds,
} from './history';

const point = (t: number, seg: number, fiveHour: number | null, weekly: number | null = null): HistoryPoint =>
  ({ t, seg, fiveHour, weekly, argon: null });

describe('history helpers', () => {
  it('breaks a line at segment changes and missing readings', () => {
    const points = [point(0, 0, 10), point(1, 0, 20), point(2, 1, 30), point(3, 1, null), point(4, 1, 40)];
    expect(linePath(points, 'fiveHour', t => t * 10, u => u)).toBe('M0.0,10.0L10.0,20.0M20.0,30.0M40.0,40.0');
    expect(linePath(points, 'weekly', t => t, u => u)).toBe('');
    // t=4 is alone in its segment once t=3 has no reading.
    expect(isolatedPoints(points, 'fiveHour').map(p => p.t)).toEqual([2, 4]);
  });

  it('finds the nearest reading in time', () => {
    const points = [point(100, 0, 1), point(200, 0, 2), point(400, 0, 3)];
    expect(nearestIndex(points, 0)).toBe(0);
    expect(nearestIndex(points, 149)).toBe(0);
    expect(nearestIndex(points, 151)).toBe(1);
    expect(nearestIndex(points, 999)).toBe(2);
    expect(nearestIndex([], 5)).toBe(-1);
  });

  it('turns protection thresholds into used percent only when protection is on', () => {
    expect(usedThresholds(DEFAULT_SETTINGS, 'codex')).toEqual({});
    const on = { ...DEFAULT_SETTINGS, providerPreferences: { ...DEFAULT_SETTINGS.providerPreferences, codex: { ...DEFAULT_SETTINGS.providerPreferences.codex, protectionEnabled: true } } };
    expect(usedThresholds(on, 'codex')).toEqual({ fiveHour: 95, weekly: 98 });
  });

  it('orders providers and numbers several DSH series', async () => {
    await i18n.changeLanguage('en-US');
    const list = sortSeries([
      { series: 'antigravity', provider: 'antigravity', firstAt: 1, lastAt: 1, records: 1 },
      { series: 'dsh-burn-a', provider: 'dsh', firstAt: 1, lastAt: 5, records: 1 },
      { series: 'codex', provider: 'codex', firstAt: 1, lastAt: 1, records: 1 },
      { series: 'dsh-burn-b', provider: 'dsh', firstAt: 1, lastAt: 9, records: 1 },
    ]);
    expect(list.map(item => item.series)).toEqual(['codex', 'dsh-burn-b', 'dsh-burn-a', 'antigravity']);
    const labels = seriesLabels(list, i18n.t);
    expect(labels['dsh-burn-b']).toBe('DSH Codex subscription 1');
    expect(labels.codex).toBe('Local Codex');
  });

  it('formats durations, axis maxima and bounded time ticks', async () => {
    await i18n.changeLanguage('en-US');
    expect(formatDuration(2 * 3600 + 15 * 60, i18n.t)).toBe('2h 15m');
    expect(formatDuration(3 * 86400 + 5 * 3600, i18n.t)).toBe('3d 5h');
    expect(niceScale(0)).toEqual({ max: 20, ticks: [0, 5, 10, 15, 20] });
    expect(niceScale(62)).toEqual({ max: 80, ticks: [0, 20, 40, 60, 80] });
    expect(niceScale(130)).toEqual({ max: 150, ticks: [0, 50, 100, 150] });
    expect(stackLabels([20, 30, 100])).toEqual([16, 42, 96]);
    const from = 1_794_700_800;
    for (const span of [24 * 3600, 7 * 86400, 90 * 86400]) {
      const ticks = timeTicks(from, from + span, 'en-US', 8);
      expect(ticks.length).toBeGreaterThan(1);
      expect(ticks.length).toBeLessThanOrEqual(9);
      expect(ticks.every(tick => tick.t >= from && tick.t <= from + span)).toBe(true);
    }
  });
});
