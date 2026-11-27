import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';
import { DEFAULT_SETTINGS, OverlaySettings, SettingsPatch } from '../types';
import type { HistorySeriesInfo, HistoryViewData } from '../history';
import { HistoryView } from './HistoryView';
import { TimelineChart } from './history/TimelineChart';

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  setTitle: vi.fn(async () => {}),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ setTitle: tauri.setTitle }) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
    tauri.listeners.set(name, callback);
    return () => { tauri.listeners.delete(name); };
  },
}));

const DAY0 = 1_794_700_800;
const H = 3600;

const SERIES: HistorySeriesInfo[] = [
  { series: 'claude', provider: 'claude', firstAt: DAY0, lastAt: DAY0 + 6 * H, records: 3 },
  { series: 'codex', provider: 'codex', firstAt: DAY0, lastAt: DAY0 + 6 * H, records: 4 },
];

function viewFor(series: string): HistoryViewData {
  return {
    series,
    provider: series === 'claude' ? 'claude' : 'codex',
    from: DAY0 - 18 * H,
    to: DAY0 + 6 * H,
    recordingSince: DAY0,
    points: [
      { t: DAY0, seg: 0, fiveHour: 10, weekly: 40, argon: null },
      { t: DAY0 + H, seg: 0, fiveHour: 60, weekly: 45, argon: null },
      { t: DAY0 + 5 * H, seg: 1, fiveHour: 5, weekly: 46, argon: null },
      { t: DAY0 + 6 * H, seg: 1, fiveHour: 20, weekly: 50, argon: null },
    ],
    resets: [{ t: DAY0 + 4 * H, kind: 'fiveHour' }],
    gaps: [{ from: DAY0 + H, to: DAY0 + 5 * H }],
    days: [
      { day: '2026-11-14', start: DAY0 - 24 * H, fiveHour: 0, weekly: 0, argon: 0, approximate: false },
      { day: '2026-11-15', start: DAY0, fiveHour: 70, weekly: 10, argon: 0, approximate: true },
    ],
    windows: [
      { kind: 'fiveHour', start: DAY0 + 4 * H, resetsAt: DAY0 + 9 * H, firstSeen: DAY0 + 5 * H, lastSeen: DAY0 + 6 * H, peak: 20, hitLimit: false, limitAt: null, timeToLimitSecs: null, unusedAtReset: null, complete: false },
      { kind: 'weekly', start: DAY0 - 2 * 24 * H, resetsAt: DAY0 + 5 * 24 * H, firstSeen: DAY0, lastSeen: DAY0 + 6 * H, peak: 50, hitLimit: false, limitAt: null, timeToLimitSecs: null, unusedAtReset: null, complete: false },
      { kind: 'fiveHour', start: DAY0 - H, resetsAt: DAY0 + 4 * H, firstSeen: DAY0, lastSeen: DAY0 + H, peak: 100, hitLimit: true, limitAt: DAY0 + H, timeToLimitSecs: 2 * H, unusedAtReset: 0, complete: true },
    ],
    hasArgon: false,
  };
}

function mockHistory(series: HistorySeriesInfo[] = SERIES) {
  tauri.invoke.mockImplementation(async (command: string, args?: { series: string; rangeSeconds: number }) => {
    if (command === 'get_usage_history_series') return series;
    if (command === 'get_usage_history') return viewFor(args!.series);
    return null;
  });
}

async function mount(settings: Partial<OverlaySettings> = {}, onPatchSettings?: (patch: SettingsPatch) => void) {
  const view = render(<HistoryView settings={{ ...DEFAULT_SETTINGS, ...settings }} onPatchSettings={onPatchSettings} />);
  await act(async () => {});
  await act(async () => {});
  return view;
}

beforeEach(async () => {
  tauri.invoke.mockReset();
  tauri.listeners.clear();
  await i18n.changeLanguage('en-US');
});

afterEach(() => cleanup());

describe('usage history window', () => {
  it('opens on the displayed provider and draws all three views', async () => {
    mockHistory();
    const view = await mount({ provider: 'codex' });
    expect(view.getByRole('tab', { name: 'Local Codex' }).getAttribute('aria-selected')).toBe('true');
    expect(tauri.invoke).toHaveBeenCalledWith('get_usage_history', { series: 'codex', rangeSeconds: 7 * 24 * H });
    expect(tauri.setTitle).toHaveBeenCalledWith('Usage history');

    // Timeline: one line per window kind, broken at the segment change.
    const fiveHour = view.container.querySelector('[data-series="fiveHour"]')!;
    expect(fiveHour.getAttribute('d')!.match(/M/g)).toHaveLength(2);
    const remainingYs = [...fiveHour.getAttribute('d')!.matchAll(/[ML][\d.]+,([\d.]+)/g)].map(match => Number(match[1]));
    expect(remainingYs).toEqual([34, 144, 23, 56]); // used 10 → 60, reset to 5 → 20
    expect(view.container.querySelector('[data-series="weekly"]')).not.toBeNull();
    expect(view.container.querySelector('[data-series="argon"]')).toBeNull();
    expect(view.container.querySelectorAll('[data-reset="fiveHour"]')).toHaveLength(1);
    expect(view.container.querySelectorAll('[data-gap]')).toHaveLength(1);
    expect(view.container.querySelector('[data-threshold]')).toBeNull();

    // Daily: a day with no use draws no bars; the approximate day is dimmed.
    expect(view.container.querySelectorAll('[data-day="2026-11-14"] [data-bar]')).toHaveLength(0);
    const bars = view.container.querySelectorAll('[data-day="2026-11-15"] [data-bar]');
    expect(bars).toHaveLength(2);
    expect(bars[0].getAttribute('fill-opacity')).toBe('0.55');

    // Windows: newest first; a finished window that hit the limit says so.
    const rows = view.container.querySelectorAll('tr[data-window]');
    expect(rows).toHaveLength(3);
    expect(rows[0].textContent).toContain('In progress');
    expect(rows[2].textContent).toContain('Yes');
    expect(rows[2].textContent).toContain('2h 0m');
  });

  it('switches provider and range, and filters the window table', async () => {
    mockHistory();
    const view = await mount({ provider: 'codex' });
    await act(async () => { fireEvent.click(view.getByRole('tab', { name: 'Claude' })); });
    expect(tauri.invoke).toHaveBeenLastCalledWith('get_usage_history', { series: 'claude', rangeSeconds: 7 * 24 * H });
    await act(async () => { fireEvent.click(view.getByRole('button', { name: '90 days' })); });
    expect(tauri.invoke).toHaveBeenLastCalledWith('get_usage_history', { series: 'claude', rangeSeconds: 90 * 24 * H });

    const filters = within(view.getByRole('group', { name: 'Window' }));
    fireEvent.click(filters.getByRole('button', { name: 'Weekly' }));
    expect(view.container.querySelectorAll('tr[data-window]')).toHaveLength(1);
    expect(view.container.querySelector('tr[data-window]')!.getAttribute('data-window')).toBe('weekly');
  });

  it('shows the crosshair readout on hover and from the keyboard', async () => {
    mockHistory();
    const view = await mount();
    const chart = view.getByRole('img', { name: 'Remaining quota over time' });
    fireEvent.keyDown(chart, { key: 'End' });
    const tooltip = view.getByRole('tooltip');
    expect(tooltip.textContent).toContain('80%');
    expect(tooltip.textContent).toContain('50%');
    fireEvent.keyDown(chart, { key: 'Home' });
    expect(tooltip.textContent).toContain('90%');
    expect(tooltip.textContent).toContain('60%');
    fireEvent(chart, new MouseEvent('pointermove', { bubbles: true, clientX: 44 + 840 }));
    expect(tooltip.textContent).toContain('80%');
    fireEvent.keyDown(chart, { key: 'Escape' });
    expect(view.queryByRole('tooltip')).toBeNull();
  });

  it('draws protection thresholds at their remaining percentages', async () => {
    mockHistory();
    const prefs = { ...DEFAULT_SETTINGS.providerPreferences, codex: { ...DEFAULT_SETTINGS.providerPreferences.codex, protectionEnabled: true } };
    const view = await mount({ providerPreferences: prefs });
    expect(view.container.querySelector('[data-threshold="fiveHour"] line')!.getAttribute('y1')).toBe('221'); // 5% remaining
    expect(view.container.querySelector('[data-threshold="weekly"] line')!.getAttribute('y1')).toBe('227.6'); // 2% remaining
    const axis = view.getByRole('img', { name: 'Remaining quota over time' });
    expect(within(axis).getByText('100%').getAttribute('y')).toBe('12');
    expect(within(axis).getByText('0%').getAttribute('y')).toBe('232');
  });

  it('switches the timeline between remaining and used quota', async () => {
    mockHistory();
    const onPatchSettings = vi.fn();
    const prefs = { ...DEFAULT_SETTINGS.providerPreferences, codex: { ...DEFAULT_SETTINGS.providerPreferences.codex, protectionEnabled: true } };
    const remaining = await mount({ providerPreferences: prefs }, onPatchSettings);
    const mode = within(remaining.getByRole('group', { name: 'Show' }));
    expect(mode.getByRole('button', { name: 'Remaining' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(mode.getByRole('button', { name: 'Remaining' }));
    expect(onPatchSettings).not.toHaveBeenCalled();
    fireEvent.click(mode.getByRole('button', { name: 'Used' }));
    expect(onPatchSettings).toHaveBeenCalledWith({ historyShowUsed: true });
    cleanup();

    const used = await mount({ providerPreferences: prefs, historyShowUsed: true }, onPatchSettings);
    expect(within(used.getByRole('group', { name: 'Show' })).getByRole('button', { name: 'Used' }).getAttribute('aria-pressed')).toBe('true');
    expect(used.getByRole('heading', { name: 'Usage over time' })).toBeTruthy();
    const fiveHour = used.container.querySelector('[data-series="fiveHour"]')!;
    const usedYs = [...fiveHour.getAttribute('d')!.matchAll(/[ML][\d.]+,([\d.]+)/g)].map(match => Number(match[1]));
    expect(usedYs).toEqual([210, 100, 221, 188]); // used 10 → 60, reset to 5 → 20
    expect(Number(used.container.querySelector('[data-threshold="fiveHour"] line')!.getAttribute('y1'))).toBeCloseTo(23); // 95% used
    const chart = used.getByRole('img', { name: 'Usage over time' });
    fireEvent.keyDown(chart, { key: 'Home' });
    expect(used.getByRole('tooltip').textContent).toContain('10%');
  });

  it('has no mode switch without a way to save it', async () => {
    mockHistory();
    const view = await mount();
    expect(view.queryByRole('group', { name: 'Show' })).toBeNull();
  });

  it('keeps missing Argon readings absent and maps an isolated point to remaining quota', () => {
    const data = viewFor('codex');
    data.points = [
      { t: DAY0, seg: 0, fiveHour: null, weekly: null, argon: null },
      { t: DAY0 + H, seg: 0, fiveHour: null, weekly: null, argon: 20 },
      { t: DAY0 + 2 * H, seg: 0, fiveHour: null, weekly: null, argon: null },
    ];
    const view = render(<TimelineChart view={data} kinds={['argon']} thresholds={{}} />);
    const dot = view.container.querySelector('[data-isolated="argon"]')!;
    expect(Number(dot.getAttribute('cy'))).toBeCloseTo(56); // 20% used = 80% remaining
    expect(view.container.querySelector('[data-series="argon"]')!.getAttribute('d')!.match(/[ML]/g)).toHaveLength(1);
    fireEvent.keyDown(view.getByRole('img', { name: 'Remaining quota over time' }), { key: 'Home' });
    expect(view.getByRole('tooltip').textContent).toContain('—');
  });

  it('offers the daily numbers as a table', async () => {
    mockHistory();
    const view = await mount();
    fireEvent.click(view.getByRole('button', { name: 'Show table' }));
    expect(view.getByText('Includes time without readings')).toBeTruthy();
  });

  it('explains an empty history and a disabled recorder', async () => {
    mockHistory([]);
    const view = await mount({ recordUsageHistory: false });
    expect(view.getByText(/No usage history yet/)).toBeTruthy();
    expect(view.getByText(/Recording is off/)).toBeTruthy();
    expect(tauri.invoke).not.toHaveBeenCalledWith('get_usage_history', expect.anything());
  });

  it('reports a failed load with a retry', async () => {
    tauri.invoke.mockRejectedValue(new Error('boom'));
    const view = await mount();
    expect(view.getByRole('alert').textContent).toContain('Could not load usage history.');
  });

  it('refreshes quietly when a monitor publishes a new reading', async () => {
    mockHistory();
    await mount();
    const calls = tauri.invoke.mock.calls.filter(([command]) => command === 'get_usage_history').length;
    await act(async () => { tauri.listeners.get('monitor_updated')!({ payload: {} }); });
    expect(tauri.invoke.mock.calls.filter(([command]) => command === 'get_usage_history').length).toBe(calls + 1);
  });
});
