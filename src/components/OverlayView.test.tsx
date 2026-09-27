import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OverlayView } from './OverlayView';
import { CodexUsage, DEFAULT_SETTINGS } from '../types';
import i18n from '../i18n';

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(async () => {}),
  setSize: vi.fn(async () => {}),
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

const now = 1_800_000_000;
const usage: CodexUsage = {
  fiveHourRemainingPercent: 69, weekRemainingPercent: 62,
  creditsDisplay: '12.5', creditsBalance: '12.5', hasCredits: true,
  fiveHourResetsAt: now + 4 * 3600 + 29 * 60,
  weekResetsAt: now + 6 * 86400 + 5 * 3600, fetchedAt: now,
};
let originalFonts: PropertyDescriptor | undefined;

beforeEach(async () => {
  vi.clearAllMocks();
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
    const weekly = within(screen.getByRole('group', { name: 'WEEKLY' }));
    expect(weekly.getByText('62%')).toBeTruthy();
    expect(weekly.getByText('6d 05h')).toBeTruthy();
    expect(within(screen.getByRole('group', { name: 'CREDITS' })).getByText('12.50')).toBeTruthy();
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
    expect(screen.getByText('—')).toBeTruthy();
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
