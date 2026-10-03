import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OverlayView } from './OverlayView';
import { CodexUsage, DEFAULT_SETTINGS } from '../types';
import i18n from '../i18n';

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(async () => {}),
  setSize: vi.fn(async () => {}),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    setSize: tauri.setSize,
    scaleFactor: async () => 1,
    onScaleChanged: async () => () => {},
  }),
  LogicalSize: class { constructor(public width: number, public height: number) {} },
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
    tauri.listeners.set(name, callback);
    return () => { tauri.listeners.delete(name); };
  },
}));

const now = 1_800_000_000;
const usage: CodexUsage = {
  fiveHourRemainingPercent: 69, fiveHourBurnRatePerHour: 16.8,
  weekRemainingPercent: 62, weekBurnRatePerHour: 0.6,
  creditsDisplay: '12.5', creditsBalance: '12.5', hasCredits: true,
  fiveHourResetsAt: now + 4 * 3600 + 29 * 60,
  weekResetsAt: now + 6 * 86400 + 5 * 3600, fetchedAt: now,
};
let originalFonts: PropertyDescriptor | undefined;

beforeEach(async () => {
  vi.clearAllMocks();
  tauri.listeners.clear();
  await i18n.changeLanguage('en-US');
  vi.spyOn(Date, 'now').mockReturnValue(now * 1000);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  originalFonts = Object.getOwnPropertyDescriptor(document, 'fonts');
  Object.defineProperty(document, 'fonts', {
    configurable: true, value: { ready: Promise.resolve() },
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
    width: 270, height: 90,
  } as DOMRect);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  if (originalFonts) Object.defineProperty(document, 'fonts', originalFonts);
  else Reflect.deleteProperty(document, 'fonts');
});

describe.each(['grouped', 'stacks'] as const)('%s layout', (overlayLayout) => {
  const settings = { ...DEFAULT_SETTINGS, overlayLayout };

  it('keeps each countdown with its quota and renders the credit balance', () => {
    const { container } = render(<OverlayView settings={settings} usage={usage} isLoading={false} />);
    const five = within(screen.getByRole('group', { name: '5 HOUR' }));
    expect(five.getByText('69%')).toBeTruthy();
    expect(five.getByText('4h 29m')).toBeTruthy();
    expect(five.getByText('16.8%/h')).toBeTruthy();
    expect(five.getByText('16.8%/h').parentElement?.classList.contains('overlay-meta-row')).toBe(true);
    const weekly = within(screen.getByRole('group', { name: 'WEEKLY' }));
    expect(weekly.getByText('62%')).toBeTruthy();
    expect(weekly.getByText('6d 05h')).toBeTruthy();
    expect(weekly.getByText('0.6%/h')).toBeTruthy();
    expect(within(screen.getByRole('group', { name: 'CREDITS' })).getByText('12.50')).toBeTruthy();
    expect(within(screen.getByRole('group', { name: 'CREDITS' })).queryByText(/%\/h$/)).toBeNull();
    expect(container.querySelectorAll('.overlay-divider').length).toBe(overlayLayout === 'grouped' ? 2 : 0);
    expect(screen.queryByText('Balance') !== null).toBe(overlayLayout === 'stacks');
  });

  it('removes the entire credit group and its separator when hidden', () => {
    const { rerender, container } = render(<OverlayView settings={settings} usage={usage} isLoading={false} />);
    rerender(<OverlayView settings={{ ...settings, showCredits: false }} usage={usage} isLoading={false} />);
    expect(screen.queryByRole('group', { name: 'CREDITS' })).toBeNull();
    expect(container.querySelectorAll('.overlay-divider').length).toBe(overlayLayout === 'grouped' ? 1 : 0);
  });

  it.each(['∞', '—', '0'])('preserves credit state %s', (creditsDisplay) => {
    render(<OverlayView settings={settings} usage={{ ...usage, creditsDisplay }} isLoading={false} />);
    expect(within(screen.getByRole('group', { name: 'CREDITS' }))
      .getByText(creditsDisplay === '0' ? '0.00' : creditsDisplay)).toBeTruthy();
  });

  it('renders missing quotas, reset times and balance without fabricating values', () => {
    render(<OverlayView settings={settings} usage={null} isLoading={true} />);
    expect(screen.getAllByText('--%').length).toBe(2);
    expect(screen.getAllByText('--h --m').length).toBe(2);
    expect(screen.getAllByText('—')).toHaveLength(3);
    expect(screen.getByRole('status', { name: 'Refreshing usage' })).toBeTruthy();
  });

  it('opens the native menu once without expanding the overlay window', async () => {
    const { container } = render(<OverlayView settings={settings} usage={usage} isLoading={false} />);
    await waitFor(() => expect(tauri.setSize).toHaveBeenCalledTimes(1));
    fireEvent.contextMenu(container.querySelector('.overlay-capsule')!);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('show_overlay_menu'));
    expect(tauri.invoke).toHaveBeenCalledTimes(1);
    expect(tauri.setSize).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('button')).toBeNull();
  });
});


it('formats unavailable, tiny and bounded burn rates without semantic zero', () => {
  const { rerender } = render(<OverlayView settings={DEFAULT_SETTINGS}
    usage={{ ...usage, fiveHourBurnRatePerHour: null, weekBurnRatePerHour: 0.049 }} isLoading={false} />);
  expect(within(screen.getByRole('group', { name: '5 HOUR' })).getByText('—')).toBeTruthy();
  expect(within(screen.getByRole('group', { name: 'WEEKLY' })).getByText('<0.1%/h')).toBeTruthy();
  rerender(<OverlayView settings={DEFAULT_SETTINGS}
    usage={{ ...usage, fiveHourBurnRatePerHour: 1000, weekBurnRatePerHour: 0.05 }} isLoading={false} />);
  expect(screen.getByText('>999%/h')).toBeTruthy();
  expect(screen.getByText('0.1%/h')).toBeTruthy();
});

it('resizes after changing layout, scale, credits and translated labels', async () => {
  let measuredWidth = 270;
  vi.mocked(HTMLElement.prototype.getBoundingClientRect).mockImplementation(() => ({
    width: measuredWidth, height: 90,
  } as DOMRect));
  const { rerender } = render(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} />);
  await waitFor(() => expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 270 })));
  measuredWidth = 310.2;
  const stacks = { ...DEFAULT_SETTINGS, overlayLayout: 'stacks' as const };
  rerender(<OverlayView settings={stacks} usage={usage} isLoading={false} />);
  expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 311 }));
  measuredWidth = 443;
  rerender(<OverlayView settings={{ ...stacks, scalePercent: 250 }} usage={usage} isLoading={false} />);
  expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 443 }));
  measuredWidth = 320;
  rerender(<OverlayView settings={{ ...stacks, showCredits: false }} usage={usage} isLoading={false} />);
  expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 320 }));
  measuredWidth = 300;
  await act(async () => { await i18n.changeLanguage('zh-CN'); });
  rerender(<OverlayView settings={{ ...stacks, language: 'zh-CN' }} usage={usage} isLoading={false} />);
  expect(screen.getByText('5 小时')).toBeTruthy();
  expect(screen.getByText('每周')).toBeTruthy();
  expect(screen.getByText('余额')).toBeTruthy();
  expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 300 }));
});

it('shows separate quota bars and expands a collapsed pill on hover', async () => {
  const dockState = { docked: true, edge: 'left' as const, expanded: false, hidden: false };
  const sample = { ...usage, fiveHourRemainingPercent: 19, weekRemainingPercent: 20 };
  const { container, rerender } = render(
    <OverlayView settings={DEFAULT_SETTINGS} usage={sample} isLoading={false} dockState={dockState} />,
  );
  const bars = screen.getAllByRole('progressbar');
  expect(bars[0].getAttribute('aria-valuenow')).toBe('19');
  expect(bars[1].getAttribute('aria-valuenow')).toBe('20');
  expect(container.querySelector('.overlay-pill-fill.overlay-low')?.getAttribute('style')).toContain('19%');
  expect(container.querySelector('.overlay-pill-fill.overlay-warning')?.getAttribute('style')).toContain('20%');
  expect(tauri.setSize).not.toHaveBeenCalled();
  expect(screen.queryByText('16.8%/h')).toBeNull();
  expect(screen.queryByText('0.6%/h')).toBeNull();

  const pill = container.querySelector('.overlay-pill-host')!;
  fireEvent.mouseEnter(pill);
  fireEvent.mouseLeave(pill);
  fireEvent.mouseDown(pill, { button: 0 });
  await waitFor(() => {
    expect(tauri.invoke).toHaveBeenCalledWith('dock_mouse_enter');
    expect(tauri.invoke).toHaveBeenCalledWith('dock_mouse_leave');
  });
  expect(tauri.invoke).not.toHaveBeenCalledWith('start_dragging');

  rerender(
    <OverlayView
      settings={DEFAULT_SETTINGS}
      usage={{ ...sample, fiveHourRemainingPercent: 50, weekRemainingPercent: 100 }}
      isLoading={false}
      dockState={{ ...dockState, edge: 'top' }}
    />,
  );
  expect(container.querySelector('.overlay-pill-host')?.getAttribute('data-dock-edge')).toBe('top');
  expect(container.querySelector('.overlay-pill-bars')?.classList.contains('overlay-pill-rotated')).toBe(true);
  expect(container.querySelectorAll('.overlay-pill-fill.overlay-healthy').length).toBe(2);

  for (const [percent, tone] of [
    [0, 'low'], [19, 'low'], [20, 'warning'], [49, 'warning'], [50, 'healthy'], [100, 'healthy'],
  ] as const) {
    rerender(
      <OverlayView
        settings={DEFAULT_SETTINGS}
        usage={{ ...sample, fiveHourRemainingPercent: percent, weekRemainingPercent: null }}
        isLoading={false}
        dockState={{ ...dockState, edge: 'right' }}
      />,
    );
    const fiveHourBar = screen.getAllByRole('progressbar')[0];
    expect(fiveHourBar.querySelector(`.overlay-pill-fill.overlay-${tone}`)?.getAttribute('style'))
      .toContain(`${percent}%`);
    expect(screen.getAllByRole('progressbar')[1].getAttribute('aria-valuenow')).toBeNull();
  }
});

it('represents unknown quota data with empty neutral bars', () => {
  const { container } = render(
    <OverlayView
      settings={DEFAULT_SETTINGS}
      usage={null}
      isLoading={false}
      dockState={{ docked: true, edge: 'bottom', expanded: false, hidden: false }}
    />,
  );
  expect(screen.getAllByRole('progressbar').map((bar) => bar.getAttribute('aria-valuetext')))
    .toEqual(['Unknown', 'Unknown']);
  expect(container.querySelectorAll('.overlay-pill-fill')).toHaveLength(0);
  expect(container.querySelector('.overlay-pill-bars')?.classList.contains('overlay-pill-rotated')).toBe(true);
});

it('draws nine 10% grid lines per pill bar only when the setting is on', () => {
  const dockState = { docked: true, edge: 'left' as const, expanded: false, hidden: false };
  const { container, rerender } = render(
    <OverlayView
      settings={{ ...DEFAULT_SETTINGS, showPercentageGrid: true }}
      usage={usage}
      isLoading={false}
      dockState={dockState}
    />,
  );
  const tracks = container.querySelectorAll('.overlay-pill-track');
  expect(tracks.length).toBe(2);
  const expected = ['10%', '20%', '30%', '40%', '50%', '60%', '70%', '80%', '90%'];
  tracks.forEach((track) => {
    const ticks = track.querySelectorAll<HTMLElement>('.overlay-pill-tick');
    expect(ticks.length).toBe(9);
    expect(Array.from(ticks).map((tick) => tick.style.bottom)).toEqual(expected);
  });

  // Off again: the bars go back to being plain columns.
  rerender(
    <OverlayView
      settings={{ ...DEFAULT_SETTINGS, showPercentageGrid: false }}
      usage={usage}
      isLoading={false}
      dockState={dockState}
    />,
  );
  expect(container.querySelectorAll('.overlay-pill-tick').length).toBe(0);
});

it('re-measures the capsule after a docked collapse and expand cycle', async () => {
  const collapsed = { docked: true, edge: 'left' as const, expanded: false, hidden: false };
  const expanded = { ...collapsed, expanded: true };
  // The real capsule is wider/taller than the backend's provisional size; the
  // measured content size must be written back on every expand.
  vi.mocked(HTMLElement.prototype.getBoundingClientRect).mockImplementation(() => ({
    width: 498, height: 121,
  } as DOMRect));

  const { rerender } = render(
    <OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} dockState={expanded} />,
  );
  await waitFor(() => expect(tauri.setSize).toHaveBeenLastCalledWith(
    expect.objectContaining({ width: 498, height: 121 })));
  const callsAfterExpand = tauri.setSize.mock.calls.length;

  // Collapsed pill: no content measurement, no window resize.
  rerender(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} dockState={collapsed} />);
  expect(tauri.setSize.mock.calls.length).toBe(callsAfterExpand);

  // Re-expanding with an identical content size must still re-apply the size:
  // the cached measurement was invalidated by the collapse.
  rerender(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} dockState={expanded} />);
  await waitFor(() => expect(tauri.setSize.mock.calls.length).toBeGreaterThan(callsAfterExpand));
  expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 498, height: 121 }));
  expect(tauri.invoke).toHaveBeenCalledWith('dock_window_resized');
});

it('re-measures the capsule when the backend invalidates the window size', async () => {
  vi.mocked(HTMLElement.prototype.getBoundingClientRect).mockImplementation(() => ({
    width: 320, height: 96,
  } as DOMRect));
  render(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} />);
  await waitFor(() => expect(tauri.setSize).toHaveBeenCalledTimes(1));

  await act(async () => {
    tauri.listeners.get('overlay_size_invalidated')?.({ payload: null });
  });
  await waitFor(() => expect(tauri.setSize).toHaveBeenCalledTimes(2));
  expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 320, height: 96 }));
});

it('starts a native drag from the expanded capsule', async () => {
  const { container } = render(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} />);
  fireEvent.mouseDown(container.querySelector('.overlay-capsule')!, { button: 0 });
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('start_dragging'));
});

it('retains threshold colors and treats unavailable quotas as unknown', () => {
  const { rerender } = render(<OverlayView settings={DEFAULT_SETTINGS}
    usage={{ ...usage, fiveHourRemainingPercent: 50, weekRemainingPercent: 20 }} isLoading={false} />);
  expect(screen.getByText('50%').classList.contains('overlay-healthy')).toBe(true);
  expect(screen.getByText('20%').classList.contains('overlay-warning')).toBe(true);
  rerender(<OverlayView settings={DEFAULT_SETTINGS}
    usage={{ ...usage, fiveHourRemainingPercent: 19, weekRemainingPercent: null }} isLoading={false} />);
  expect(screen.getByText('19%').classList.contains('overlay-low')).toBe(true);
  expect(screen.getByText('--%').classList.contains('overlay-unknown')).toBe(true);
});

it.each([
  [now + 1, now + 86400 - 1, '0h 01m', '1d 00h'],
  [now - 1, now + 86340, '0h 00m', '23h 59m'],
  [0, null, '0h 00m', '--h --m'],
] as const)('rounds reset minutes upward and clamps expired resets (%s)', (fiveHourResetsAt, weekResetsAt, five, weekly) => {
  render(<OverlayView settings={DEFAULT_SETTINGS} usage={{ ...usage, fiveHourResetsAt, weekResetsAt }} isLoading={false} />);
  expect(within(screen.getByRole('group', { name: '5 HOUR' })).getByText(five)).toBeTruthy();
  expect(within(screen.getByRole('group', { name: 'WEEKLY' })).getByText(weekly)).toBeTruthy();
});

it('updates countdowns without a new usage payload', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(now * 1000);
  render(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} />);
  expect(screen.getByText('4h 29m')).toBeTruthy();
  vi.setSystemTime((now + 60) * 1000);
  await act(async () => { vi.advanceTimersByTime(1000); });
  expect(screen.getByText('4h 28m')).toBeTruthy();
});


describe.each(['en-US', 'zh-CN', 'zh-Hant'])('display preferences in %s', (language) => {
  it.each(['grouped', 'stacks'] as const)('covers every visibility combination in %s', async (overlayLayout) => {
    await i18n.changeLanguage(language);
    const { container, rerender } = render(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} />);
    for (const showFiveHourQuota of [true, false]) {
      for (const showBurnRate of [true, false]) {
        for (const showCredits of [true, false]) {
          rerender(<OverlayView settings={{ ...DEFAULT_SETTINGS, overlayLayout, showFiveHourQuota, showBurnRate, showCredits }} usage={usage} isLoading={false} />);
          expect(screen.queryByRole('group', { name: i18n.t('fiveHourLabel') }) !== null).toBe(showFiveHourQuota);
          const weekly = within(screen.getByRole('group', { name: i18n.t('weeklyLabel') }));
          expect(weekly.getByText('62%')).toBeTruthy();
          expect(weekly.getByText('6d 05h')).toBeTruthy();
          expect(weekly.queryByText('0.6%/h') !== null).toBe(showBurnRate);
          expect(screen.queryByRole('group', { name: i18n.t('creditsLabel') }) !== null).toBe(showCredits);
          expect(container.querySelectorAll('.overlay-burn-rate')).toHaveLength(showBurnRate ? (showFiveHourQuota ? 2 : 1) : 0);
          expect(container.querySelectorAll('.overlay-divider')).toHaveLength(overlayLayout === 'grouped' ? Number(showFiveHourQuota) + Number(showCredits) : 0);
        }
      }
    }
    // Restoring visibility uses the same cached DTO immediately.
    rerender(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} />);
    expect(screen.getByText('16.8%/h')).toBeTruthy();
    expect(usage.fiveHourRemainingPercent).toBe(69);
  });

  it.each(['left', 'right', 'top', 'bottom'] as const)('shows only the Weekly bar at the %s edge', async (edge) => {
    await i18n.changeLanguage(language);
    const dockState = { docked: true, edge, expanded: false, hidden: false };
    const settings = { ...DEFAULT_SETTINGS, showFiveHourQuota: false, showPercentageGrid: true };
    const { container, rerender } = render(<OverlayView settings={settings} usage={usage} isLoading={false} dockState={dockState} />);
    expect(screen.getAllByRole('progressbar')).toHaveLength(1);
    expect(screen.getByRole('progressbar', { name: i18n.t('weeklyLabel') }).getAttribute('aria-valuenow')).toBe('62');
    expect(screen.queryByRole('progressbar', { name: i18n.t('fiveHourLabel') })).toBeNull();
    expect(screen.getByRole('group').getAttribute('aria-label')).toBe(i18n.t('weeklyLabel'));
    expect(container.querySelectorAll('.overlay-pill-tick')).toHaveLength(9);
    expect(container.querySelector('.overlay-pill-bars')?.classList.contains('overlay-pill-rotated')).toBe(edge === 'top' || edge === 'bottom');
    fireEvent.mouseEnter(container.querySelector('.overlay-pill-host')!);
    fireEvent.mouseLeave(container.querySelector('.overlay-pill-host')!);
    expect(tauri.invoke).toHaveBeenCalledWith('dock_mouse_enter');
    expect(tauri.invoke).toHaveBeenCalledWith('dock_mouse_leave');
    rerender(<OverlayView settings={settings} usage={null} isLoading={true} dockState={dockState} />);
    expect(screen.getAllByRole('progressbar')).toHaveLength(1);
    expect(screen.getByRole('progressbar').getAttribute('aria-valuetext')).toBe(i18n.t('unknown'));
    rerender(<OverlayView settings={{ ...settings, showFiveHourQuota: true }} usage={usage} isLoading={false} dockState={dockState} />);
    expect(screen.getAllByRole('progressbar')).toHaveLength(2);
  });
});

it('hides unknown Burn Rate placeholders with the entire rate span', () => {
  const { container } = render(<OverlayView settings={{ ...DEFAULT_SETTINGS, showBurnRate: false, showFiveHourQuota: false, showCredits: false }} usage={null} isLoading={true} />);
  expect(container.querySelector('.overlay-burn-rate')).toBeNull();
  expect(screen.queryByText('—')).toBeNull();
  expect(screen.getAllByText('--%')).toHaveLength(1);
});


it('remeasures docked content immediately when either display preference changes', async () => {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    return { width: 100 + this.querySelectorAll('.overlay-quota').length * 50 + this.querySelectorAll('.overlay-burn-rate').length * 20, height: 69 } as DOMRect;
  });
  const dockState = { docked: true, edge: 'right' as const, expanded: true, hidden: false };
  const { rerender } = render(<OverlayView settings={DEFAULT_SETTINGS} usage={usage} isLoading={false} dockState={dockState} />);
  await waitFor(() => expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 240, height: 69 })));
  rerender(<OverlayView settings={{ ...DEFAULT_SETTINGS, showBurnRate: false }} usage={usage} isLoading={false} dockState={dockState} />);
  await waitFor(() => expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 200, height: 69 })));
  rerender(<OverlayView settings={{ ...DEFAULT_SETTINGS, showBurnRate: false, showFiveHourQuota: false }} usage={usage} isLoading={false} dockState={dockState} />);
  await waitFor(() => expect(tauri.setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 150, height: 69 })));
  expect(tauri.invoke).toHaveBeenCalledWith('dock_window_resized');
  expect(tauri.invoke).not.toHaveBeenCalledWith('fetch_usage');
});
