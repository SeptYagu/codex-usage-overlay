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

async function mount(overrides: Partial<OverlaySettings> = {}) {
  const onPatchSettings = vi.fn();
  const view = render(<SettingsView settings={{ ...DEFAULT_SETTINGS, ...overrides }} onPatchSettings={onPatchSettings} />);
  await act(async () => {});
  const panel = (provider: string) => {
    fireEvent.click(view.getByRole('tab', { name: i18n.t(
      provider === 'codex' ? 'providerCodex'
        : provider === 'claude' ? 'providerClaude'
          : provider === 'antigravity' ? 'providerAntigravity'
            : 'providerDsh',
    ) }));
    return within(view.container.querySelector(`[data-provider-settings="${provider}"]`)! as HTMLElement);
  };
  const page = (label: string) => fireEvent.click(view.getByRole('tab', { name: i18n.t(label) }));
  return { ...view, panel, page, onPatchSettings };
}

describe('settings tabs', () => {
  it('offers Russian in settings and presents the localized settings after selection', async () => {
    const view = await mount();
    fireEvent.change(view.getByRole('combobox', { name: 'Language' }), { target: { value: 'ru-RU' } });
    expect(view.onPatchSettings).toHaveBeenCalledWith({ language: 'ru-RU' }, false);
    await act(async () => {
      await i18n.changeLanguage('ru-RU');
      view.rerender(<SettingsView settings={{ ...DEFAULT_SETTINGS, language: 'ru-RU' }} onPatchSettings={view.onPatchSettings} />);
    });
    expect((view.getByRole('combobox', { name: 'Язык' }) as HTMLSelectElement).value).toBe('ru-RU');
    expect(view.getByRole('option', { name: 'Русский' })).toBeTruthy();
    expect(view.getByRole('tab', { name: 'Общие' })).toBeTruthy();
  });
  it('shows one settings page at a time with unique control ids', async () => {
    const view = await mount();
    const tabs = view.getAllByRole('tab');
    expect(tabs.map(tab => tab.textContent)).toEqual(['General', 'Overlay', 'Quota rotation', 'Usage history', 'Local Codex', 'Claude', 'DSH Codex subscription', 'Antigravity']);
    expect(view.getByRole('tablist').getAttribute('aria-orientation')).toBe('vertical');
    expect(view.container.querySelectorAll('[data-provider-settings]')).toHaveLength(0);
    expect(view.container.querySelectorAll('[data-app-settings="general"]')).toHaveLength(1);
    for (const label of ['settingsPageGeneral', 'settingsPageOverlay', 'settingsPageRotation', 'settingsPageHistory']) {
      view.page(label);
      const ids = [...view.container.querySelectorAll('[id]')].map(el => el.id);
      expect(new Set(ids).size).toBe(ids.length);
    }
    view.panel('dsh');
    expect(view.container.querySelectorAll('[data-app-settings]')).toHaveLength(0);
    expect(view.container.querySelectorAll('[data-provider-settings]')).toHaveLength(1);
    const ids = [...view.container.querySelectorAll('[id]')].map(el => el.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('moves between pages with the arrow keys and groups each page into headed sections', async () => {
    const view = await mount();
    const general = view.getByRole('tab', { name: 'General' });
    general.focus();
    fireEvent.keyDown(general, { key: 'ArrowDown' });
    expect(view.getByRole('tab', { name: 'Overlay' }).getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(view.getByRole('tab', { name: 'Overlay' }));
    for (const name of ['Appearance', 'Show on overlay', 'Behavior']) expect(view.getByRole('heading', { name })).toBeTruthy();
    fireEvent.keyDown(document.activeElement!, { key: 'End' });
    expect(view.getByRole('tab', { name: 'Antigravity' }).getAttribute('aria-selected')).toBe('true');
    for (const name of ['Monitoring', 'Protection', 'Display', 'Notifications']) expect(view.getByRole('heading', { name })).toBeTruthy();
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(view.getByRole('tab', { name: 'General' }).getAttribute('aria-selected')).toBe('true');
  });

  it('marks the provider the overlay shows in the sidebar', async () => {
    const view = await mount({ provider: 'claude' });
    expect(view.getByRole('tab', { name: 'Claude' }).querySelector('[title="Showing"]')).not.toBeNull();
    expect(view.getByRole('tab', { name: 'Local Codex' }).querySelector('[title="Showing"]')).toBeNull();
  });
  it('writes refresh only to the initiating provider even when the overlay shows another', async () => {
    const { panel, onPatchSettings } = await mount({provider: 'dsh'});
    fireEvent.change(panel('claude').getByRole('combobox', {name: 'Refresh Interval'}), {target:{value:'120'}});
    expect(onPatchSettings).toHaveBeenCalledWith({providerPreferences:{claude:{refreshIntervalSeconds:120}}}, false);
    expect(onPatchSettings.mock.calls[0][0].provider).toBeUndefined();
  });
  it('keeps reminder preferences and explicit null paths separate', async () => {
    const { panel, onPatchSettings } = await mount();
    fireEvent.click(panel('dsh').getByRole('checkbox', {name:'Weekly quota reset notification Enable notification'}));
    expect(onPatchSettings).toHaveBeenCalledWith({providerPreferences:{dsh:{weeklyResetNotification:false}}}, false);
  });
  it('keeps protection thresholds and zero-capable DSH ranges independent', async () => {
    const { panel, onPatchSettings } = await mount({ providerPreferences: { ...DEFAULT_SETTINGS.providerPreferences, codex: { ...DEFAULT_SETTINGS.providerPreferences.codex, protectionEnabled: true } } });
    const threshold = panel('codex').getByRole('slider', {name:'5-hour quota threshold'}) as HTMLInputElement;
    expect(threshold.min).toBe('1');
    fireEvent.change(threshold, {target:{value:'9'}});
    expect(onPatchSettings).toHaveBeenCalledWith({providerPreferences:{codex:{fiveHourThresholdPercent:9}}}, true);
    expect((panel('dsh').getByRole('slider', {name:'5-hour quota threshold'}) as HTMLInputElement).min).toBe('0');
  });
  it('disables protection controls for a disabled source without hiding other tabs', async () => {
    const {panel} = await mount({ providerPreferences: { ...DEFAULT_SETTINGS.providerPreferences, codex: { ...DEFAULT_SETTINGS.providerPreferences.codex, monitorEnabled: false, protectionEnabled: false } } });
    expect((panel('codex').getByRole('checkbox',{name:'Pause Codex when quota runs low'}) as HTMLInputElement).disabled).toBe(true);
    expect(panel('claude').getByRole('checkbox',{name:'Monitor this provider'})).toBeTruthy();
  });
  it('preserves a legal nonstandard refresh value instead of selecting a different default', async () => {
    const {panel} = await mount({ providerPreferences: { ...DEFAULT_SETTINGS.providerPreferences, codex: { ...DEFAULT_SETTINGS.providerPreferences.codex, refreshIntervalSeconds: 47 } } });
    expect((panel('codex').getByRole('combobox',{name:'Refresh Interval'}) as HTMLSelectElement).value).toBe('47');
  });
  it('queries global installation and permission status only once', async () => {
    await mount();
    expect(tauri.invoke.mock.calls.filter(([name])=>name==='get_notification_status')).toHaveLength(1);
    expect(tauri.invoke.mock.calls.filter(([name])=>name==='is_installed_version')).toHaveLength(1);
  });
  it.each(['en-US','zh-CN','zh-Hant'])('keeps all provider tabs in %s', async locale => {
    await i18n.changeLanguage(locale);
    const view = await mount();
    for (const key of ['settingsPageGeneral','settingsPageOverlay','settingsPageRotation','settingsPageHistory','providerCodex','providerClaude','providerDsh','providerAntigravity']) expect(view.getByRole('tab',{name:i18n.t(key)})).toBeTruthy();
  });
  it('updates Antigravity quota pool selection and live polling from the Antigravity settings tab', async () => {
    const { panel, onPatchSettings } = await mount({ antigravityPool: 'auto', antigravityLivePolling: true });
    const antigravity = panel('antigravity');
    fireEvent.change(antigravity.getByRole('combobox', { name: 'Quota pool' }), { target: { value: 'argon' } });
    expect(onPatchSettings).toHaveBeenCalledWith({ antigravityPool: 'argon' }, false);
    fireEvent.click(antigravity.getByRole('checkbox', { name: 'Read limits live via Antigravity CLI (agy /usage)' }));
    expect(onPatchSettings).toHaveBeenCalledWith({ antigravityLivePolling: false }, false);
  });
});

it('keeps carousel participation separate and saves every slider second through debounce',async()=>{
  const {page,getByRole,onPatchSettings}=await mount({quotaCarouselEnabled:true,quotaCarouselIntervalSeconds:9});
  page('settingsPageRotation');
  const slider=getByRole('slider',{name:'Rotation interval'}) as HTMLInputElement;
  expect([slider.min,slider.max,slider.step,slider.value]).toEqual(['3','30','1','9']);
  fireEvent.change(slider,{target:{value:'11'}});expect(onPatchSettings).toHaveBeenLastCalledWith({quotaCarouselIntervalSeconds:11},true);
  fireEvent.click(getByRole('checkbox',{name:'Automatically rotate quotas'}));expect(onPatchSettings).toHaveBeenLastCalledWith({quotaCarouselEnabled:false});
  const included=within(getByRole('region',{name:'Included providers'}));
  fireEvent.click(included.getByRole('checkbox',{name:'Claude'}));
  expect(onPatchSettings).toHaveBeenLastCalledWith({providerPreferences:{claude:{carouselIncluded:false}}});
});

describe('usage history settings', () => {
  it('toggles recording, opens the history window and clears only after confirmation', async () => {
    const view = await mount();
    view.page('settingsPageHistory');
    const block = within(view.container.querySelector('[data-usage-history-settings]') as HTMLElement);
    const toggle = block.getByRole('checkbox', { name: 'Record usage history' }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    fireEvent.click(toggle);
    expect(view.onPatchSettings).toHaveBeenCalledWith({ recordUsageHistory: false });

    fireEvent.click(block.getByRole('button', { name: 'Open usage history' }));
    expect(tauri.invoke).toHaveBeenCalledWith('open_history_window');

    fireEvent.click(block.getByRole('button', { name: 'Clear history' }));
    expect(tauri.invoke).not.toHaveBeenCalledWith('clear_usage_history');
    fireEvent.click(block.getByRole('button', { name: 'Cancel' }));
    expect(tauri.invoke).not.toHaveBeenCalledWith('clear_usage_history');

    fireEvent.click(block.getByRole('button', { name: 'Clear history' }));
    await act(async () => { fireEvent.click(block.getByRole('button', { name: 'Delete' })); });
    expect(tauri.invoke).toHaveBeenCalledWith('clear_usage_history');
    expect(block.getByRole('status').textContent).toBe('Usage history cleared.');
  });

  it('chooses what the history timeline shows', async () => {
    const view = await mount();
    view.page('settingsPageHistory');
    const shows = view.getByRole('combobox', { name: 'Timeline shows' }) as HTMLSelectElement;
    expect(shows.value).toBe('remaining');
    fireEvent.change(shows, { target: { value: 'used' } });
    expect(view.onPatchSettings).toHaveBeenCalledWith({ historyShowUsed: true });
  });
});
