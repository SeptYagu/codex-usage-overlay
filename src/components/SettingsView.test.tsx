import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsView } from './SettingsView';
import { DEFAULT_SETTINGS, OverlaySettings } from '../types';
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

    // Header and footer sit outside the columns, so they span the full window.
    expect(grid.contains(header)).toBe(false);
    expect(grid.contains(footer)).toBe(false);
    expect(within(header).getByRole('heading', { name: 'Settings' })).toBeTruthy();
    expect(within(header).getByText('v1.2.0')).toBeTruthy();
    expect(within(footer).getByText('septwind@agent.qq.com')).toBeTruthy();
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
