import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsView } from './SettingsView';
import { DEFAULT_SETTINGS, OverlaySettings } from '../types';
import { APP_VERSION } from '../version';
import i18n from '../i18n';

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  setTitle: vi.fn(async () => {}),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ setTitle: tauri.setTitle }),
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
    tauri.listeners.set(name, callback);
    return () => { tauri.listeners.delete(name); };
  },
}));

beforeEach(async () => {
  tauri.invoke.mockReset();
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'is_installed_version') return true;
    if (command === 'get_notification_status') return 'enabled';
    return null;
  });
  tauri.listeners.clear();
  await i18n.changeLanguage('en-US');
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

/**
 * Sizes the three top-level bands the way the real window does: the body is the
 * `grid-cols-2` element, its two direct `<section>` children are the columns and the
 * header/footer are the sibling bands that span both columns.
 */
async function renderSettings(overrides: Partial<OverlaySettings> = {}) {
  const onPatchSettings = vi.fn();
  const utils = render(
    <SettingsView settings={{ ...DEFAULT_SETTINGS, ...overrides }} onPatchSettings={onPatchSettings} />,
  );
  // Let the two mount-time invokes settle so their state updates stay inside act().
  await act(async () => {});

  const root = utils.container.firstElementChild as HTMLElement;
  const grid = utils.container.querySelector('.grid-cols-2') as HTMLElement;
  const [left, right] = Array.from(grid.children) as HTMLElement[];
  const header = root.children[0] as HTMLElement;
  const footer = root.children[2] as HTMLElement;
  return { ...utils, onPatchSettings, root, grid, left, right, header, footer };
}

describe('two-column settings layout', () => {
  it('spans the header and footer across a grid that splits the body into two columns', async () => {
    const { root, grid, left, right, header, footer } = await renderSettings();

    // Single screen, no vertical scrollbar: the shell clips and fills the window.
    expect(root.className).toContain('h-screen');
    expect(root.className).toContain('overflow-hidden');
    expect(root.classList.contains('flex-col')).toBe(true);

    // Exactly three bands: header, the two-column grid, footer.
    expect(root.children).toHaveLength(3);
    expect(root.children[1]).toBe(grid);

    // The grid owns exactly the two column sections.
    expect(grid.children).toHaveLength(2);
    expect(left.tagName).toBe('SECTION');
    expect(right.tagName).toBe('SECTION');

    // Both columns must have overflow-y-auto and min-h-0 so oversized content
    // scrolls within the column and never clips controls or breaks root clientHeight == scrollHeight.
    expect(left.classList.contains('overflow-y-auto')).toBe(true);
    expect(right.classList.contains('overflow-y-auto')).toBe(true);
    expect(left.classList.contains('min-h-0')).toBe(true);
    expect(right.classList.contains('min-h-0')).toBe(true);

    // Header and footer sit outside the columns, so they span the full window.
    expect(grid.contains(header)).toBe(false);
    expect(grid.contains(footer)).toBe(false);
    expect(within(header).getByRole('heading', { name: 'Settings' })).toBeTruthy();
    expect(within(header).getByText(`v${APP_VERSION}`)).toBeTruthy();
    expect(within(footer).getByText('septwind@agent.qq.com')).toBeTruthy();
  });

  it('renders compact sound pickers preserving layout budget when custom sounds are selected', async () => {
    const { right } = await renderSettings({
      fiveHourResetNotification: true,
      fiveHourSoundMode: 'custom',
      fiveHourSoundPath: 'C:\\Users\\custom\\five_hour_alert_long_path_sample.wav',
      weeklyResetNotification: true,
      weeklySoundMode: 'custom',
      weeklySoundPath: 'C:\\Users\\custom\\weekly_alert_long_path_sample.wav',
    });

    // Both sound pickers are mounted and fully rendered within the right section
    const chooseButtons = within(right).getAllByRole('button', { name: 'Choose file…' });
    expect(chooseButtons).toHaveLength(2);

    const previewButtons = within(right).getAllByRole('button', { name: 'Preview' });
    expect(previewButtons).toHaveLength(2);

    const clearButtons = within(right).getAllByRole('button', { name: 'Clear' });
    expect(clearButtons).toHaveLength(2);

    // FE-M2 sentinel: SoundPicker must maintain compact footprint (truncate, py-0.5, gap-1.5)
    // to prevent vertical overflow regression inside the fixed 486px slot.
    for (const button of [...chooseButtons, ...previewButtons, ...clearButtons]) {
      expect(button.className).toContain('py-0.5');
    }

    const pathParagraphs = [
      within(right).getByText('C:\\Users\\custom\\five_hour_alert_long_path_sample.wav'),
      within(right).getByText('C:\\Users\\custom\\weekly_alert_long_path_sample.wav'),
    ];
    for (const p of pathParagraphs) {
      expect(p.className).toContain('truncate');
      expect(p.className).toContain('font-mono');
    }

    const buttonContainers = chooseButtons.map((btn) => btn.parentElement as HTMLElement);
    for (const container of buttonContainers) {
      expect(container.className).toContain('gap-1.5');
    }
  });

  it('keeps overlay controls in the left column and system controls in the right', async () => {
    const { left, right } = await renderSettings();

    // Left column: appearance, size and interaction.
    within(left).getByRole('combobox', { name: 'Overlay layout' });
    within(left).getByRole('slider', { name: 'Overlay Size' });
    within(left).getByRole('slider', { name: 'Background Transparency' });
    within(left).getByRole('checkbox', { name: 'Show Credit Balance' });
    within(left).getByRole('checkbox', { name: 'Auto hide at screen edge' });
    within(left).getByRole('checkbox', { name: 'Show Percentage Grid' });
    within(left).getByRole('checkbox', { name: 'Mouse click-through' });

    // Right column: refresh cadence, start-up/updates and reset alerts.
    within(right).getByRole('combobox', { name: 'Refresh Interval' });
    within(right).getByRole('checkbox', { name: 'Check for updates automatically' });
    within(right).getByRole('checkbox', { name: 'Allow automatic update installation' });
    within(right).getByRole('heading', { name: 'Notifications' });

    // Neither half bleeds into the other.
    expect(within(right).queryByRole('checkbox', { name: 'Show Percentage Grid' })).toBeNull();
    expect(within(left).queryByRole('combobox', { name: 'Refresh Interval' })).toBeNull();
    expect(within(left).queryByRole('heading', { name: 'Notifications' })).toBeNull();
  });

  it('tucks the percentage-grid switch directly below the edge-hide switch', async () => {
    const { left } = await renderSettings();
    const switchLabels = Array.from(left.children).filter((child) => child.tagName === 'LABEL');
    const names = switchLabels.map((label) => label.textContent);
    const edgeHideIndex = names.indexOf('Auto hide at screen edge');
    expect(edgeHideIndex).toBeGreaterThanOrEqual(0);
    expect(names[edgeHideIndex + 1]).toBe('Show Percentage Grid');
  });
});

describe('percentage grid interaction', () => {
  it('patches showPercentageGrid on when the switch is turned on', async () => {
    const { left, onPatchSettings } = await renderSettings({ showPercentageGrid: false });
    const toggle = within(left).getByRole('checkbox', { name: 'Show Percentage Grid' }) as HTMLInputElement;
    expect(toggle.checked).toBe(false);

    fireEvent.click(toggle);

    expect(onPatchSettings).toHaveBeenCalledTimes(1);
    expect(onPatchSettings.mock.calls[0][0]).toEqual({ showPercentageGrid: true });
  });

  it('reflects an enabled grid and patches it off without touching other fields', async () => {
    const { left, onPatchSettings } = await renderSettings({ showPercentageGrid: true, autoEdgeHide: true });
    const toggle = within(left).getByRole('checkbox', { name: 'Show Percentage Grid' }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);

    fireEvent.click(toggle);

    expect(onPatchSettings).toHaveBeenCalledTimes(1);
    expect(onPatchSettings.mock.calls[0][0]).toEqual({ showPercentageGrid: false });
  });
});


describe('smart pause settings', () => {
  it('renders the auto-pause controls in the right column only', async () => {
    const { left, right } = await renderSettings();

    within(right).getByRole('checkbox', { name: 'Pause Codex when quota runs low' });
    within(right).getByRole('slider', { name: '5-hour quota threshold' });
    within(right).getByRole('slider', { name: 'Weekly quota threshold' });

    expect(within(left).queryByRole('checkbox', { name: 'Pause Codex when quota runs low' })).toBeNull();
    expect(within(left).queryByRole('slider', { name: '5-hour quota threshold' })).toBeNull();
    expect(within(left).queryByRole('slider', { name: 'Weekly quota threshold' })).toBeNull();
  });

  it('keeps the master switch off by default and saves it immediately', async () => {
    const { right, onPatchSettings } = await renderSettings();
    const toggle = within(right).getByRole('checkbox', { name: 'Pause Codex when quota runs low' }) as HTMLInputElement;
    expect(toggle.checked).toBe(false);

    fireEvent.click(toggle);

    expect(onPatchSettings).toHaveBeenCalledTimes(1);
    // Only the master switch is patched; no threshold rides along.
    expect(onPatchSettings.mock.calls[0][0]).toEqual({ autoPauseOnLowQuota: true });
    // Immediate save: the call is not routed through the debounced slider path.
    expect(onPatchSettings.mock.calls[0][1]).toBeFalsy();
  });

  it('disables both threshold sliders while the master switch is off', async () => {
    const { right, onPatchSettings, rerender } = await renderSettings();
    const five = within(right).getByRole('slider', { name: '5-hour quota threshold' }) as HTMLInputElement;
    const weekly = within(right).getByRole('slider', { name: 'Weekly quota threshold' }) as HTMLInputElement;

    expect(five.disabled).toBe(true);
    expect(weekly.disabled).toBe(true);

    rerender(<SettingsView settings={{ ...DEFAULT_SETTINGS, autoPauseOnLowQuota: true }} onPatchSettings={onPatchSettings} />);

    const enabledFive = within(right).getByRole('slider', { name: '5-hour quota threshold' }) as HTMLInputElement;
    const enabledWeekly = within(right).getByRole('slider', { name: 'Weekly quota threshold' }) as HTMLInputElement;
    expect(enabledFive.disabled).toBe(false);
    expect(enabledWeekly.disabled).toBe(false);
  });

  it('patches the 5-hour threshold through the debounced slider path', async () => {
    const { right, onPatchSettings } = await renderSettings({ autoPauseOnLowQuota: true });
    const five = within(right).getByRole('slider', { name: '5-hour quota threshold' });

    fireEvent.change(five, { target: { value: '7' } });

    expect(onPatchSettings).toHaveBeenCalledTimes(1);
    expect(onPatchSettings.mock.calls[0][0]).toEqual({ autoPauseFiveHourThresholdPercent: 7 });
    expect(onPatchSettings.mock.calls[0][1]).toBe(true);
  });

  it('patches the weekly threshold through the debounced slider path', async () => {
    const { right, onPatchSettings } = await renderSettings({ autoPauseOnLowQuota: true });
    const weekly = within(right).getByRole('slider', { name: 'Weekly quota threshold' });

    fireEvent.change(weekly, { target: { value: '4' } });

    expect(onPatchSettings).toHaveBeenCalledTimes(1);
    expect(onPatchSettings.mock.calls[0][0]).toEqual({ autoPauseWeeklyThresholdPercent: 4 });
    expect(onPatchSettings.mock.calls[0][1]).toBe(true);
  });

  it('renders the current threshold percentages and the hint', async () => {
    const { right } = await renderSettings();

    expect(within(right).getByText('5%')).toBeTruthy();
    expect(within(right).getByText('2%')).toBeTruthy();
    expect(within(right).getByText(/Sends a pause request to your active Codex session/)).toBeTruthy();
  });
});


it.each([
  ['en-US', 'Pause Codex when quota runs low', '5-hour quota threshold', 'Weekly quota threshold'],
  ['zh-CN', '额度不足时暂停 Codex', '5 小时额度阈值', '每周额度阈值'],
  ['zh-Hant', '額度不足時暫停 Codex', '5 小時額度閾值', '每週額度閾值'],
])('exposes localized auto-pause control names in %s', async (language, toggleName, fiveName, weeklyName) => {
  await i18n.changeLanguage(language);
  const { right, onPatchSettings } = await renderSettings();

  const toggle = within(right).getByRole('checkbox', { name: toggleName }) as HTMLInputElement;
  expect(toggle.checked).toBe(false);
  fireEvent.click(toggle);
  expect(onPatchSettings).toHaveBeenCalledTimes(1);
  expect(onPatchSettings.mock.calls[0][0]).toEqual({ autoPauseOnLowQuota: true });

  const five = within(right).getByRole('slider', { name: fiveName }) as HTMLInputElement;
  const weekly = within(right).getByRole('slider', { name: weeklyName }) as HTMLInputElement;
  expect(five.disabled).toBe(true);
  expect(weekly.disabled).toBe(true);

  // The accessible name must follow the active locale instead of staying English.
  if (language !== 'en-US') {
    expect(within(right).queryByRole('checkbox', { name: 'Pause Codex when quota runs low' })).toBeNull();
  }
});


it.each([
  ['en-US', 'Show 5-hour quota', 'Show Burn Rate (%/h)'],
  ['zh-CN', '显示 5 小时额度', '显示消耗速度（%/h）'],
  ['zh-Hant', '顯示 5 小時額度', '顯示消耗速度（%/h）'],
])('sends independent display patches in %s', async (language, fiveLabel, rateLabel) => {
  await i18n.changeLanguage(language);
  const { left, right, onPatchSettings, rerender } = await renderSettings();
  const five = within(left).getByRole('checkbox', { name: fiveLabel }) as HTMLInputElement;
  const rate = within(left).getByRole('checkbox', { name: rateLabel }) as HTMLInputElement;
  expect(five.checked).toBe(true);
  expect(rate.checked).toBe(true);
  fireEvent.click(five);
  expect(onPatchSettings).toHaveBeenLastCalledWith({ showFiveHourQuota: false }, false);
  rerender(<SettingsView settings={{ ...DEFAULT_SETTINGS, showFiveHourQuota: false }} onPatchSettings={onPatchSettings} />);
  expect(rate.disabled).toBe(false);
  fireEvent.click(rate);
  expect(onPatchSettings).toHaveBeenLastCalledWith({ showBurnRate: false }, false);
  const resume = within(right).getByRole('checkbox', { name: i18n.t('autoResumeAfterQuotaReset') });
  expect((resume as HTMLInputElement).disabled).toBe(true);
  expect(within(right).getAllByRole('checkbox').filter(box => box !== resume)
    .every(box => !(box as HTMLInputElement).disabled)).toBe(true);
});
