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
    fireEvent.click(view.getByRole('tab', { name: i18n.t(provider === 'codex' ? 'providerCodex' : provider === 'claude' ? 'providerClaude' : 'providerDsh') }));
    return within(view.container.querySelector(`[data-provider-settings="${provider}"]`)! as HTMLElement);
  };
  return { ...view, panel, onPatchSettings };
}

describe('settings tabs', () => {
  it('shows one settings page at a time with unique control ids', async () => {
    const view = await mount();
    expect(view.getAllByRole('tab')).toHaveLength(4);
    expect(view.container.querySelectorAll('[data-provider-settings]')).toHaveLength(0);
    expect(view.queryByLabelText('Provider settings')).toBeNull();
    expect(view.container.querySelectorAll('[data-common-settings]')).toHaveLength(1);
    const ids = [...view.container.querySelectorAll('[id]')].map(el => el.id);
    expect(new Set(ids).size).toBe(ids.length);
    view.panel('dsh');
    expect(view.container.querySelectorAll('[data-common-settings]')).toHaveLength(0);
    expect(view.container.querySelectorAll('[data-provider-settings]')).toHaveLength(1);
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
    const { panel, onPatchSettings } = await mount({autoPauseOnLowQuota:true});
    const threshold = panel('codex').getByRole('slider', {name:'5-hour quota threshold'}) as HTMLInputElement;
    expect(threshold.min).toBe('1');
    fireEvent.change(threshold, {target:{value:'9'}});
    expect(onPatchSettings).toHaveBeenCalledWith({providerPreferences:{codex:{fiveHourThresholdPercent:9}}}, true);
    expect((panel('dsh').getByRole('slider', {name:'5-hour quota threshold'}) as HTMLInputElement).min).toBe('0');
  });
  it('disables protection controls for a disabled source without hiding other tabs', async () => {
    const {panel} = await mount({providerPreferences:{codex:{monitorEnabled:false,protectionEnabled:false,fiveHourThresholdPercent:5,weeklyThresholdPercent:2}}});
    expect((panel('codex').getByRole('checkbox',{name:'Pause Codex when quota runs low'}) as HTMLInputElement).disabled).toBe(true);
    expect(panel('claude').getByRole('checkbox',{name:'Monitor this provider'})).toBeTruthy();
  });
  it('preserves a legal nonstandard refresh value instead of selecting a different default', async () => {
    const {panel} = await mount({refreshIntervalSeconds:47});
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
    for (const key of ['commonSettings','providerCodex','providerClaude','providerDsh']) expect(view.getByRole('tab',{name:i18n.t(key)})).toBeTruthy();
  });
});

it('keeps carousel participation separate and saves every slider second through debounce',async()=>{
  const {panel,getByRole,onPatchSettings}=await mount({quotaCarouselEnabled:true,quotaCarouselIntervalSeconds:9});
  const slider=getByRole('slider',{name:'Rotation interval'}) as HTMLInputElement;
  expect([slider.min,slider.max,slider.step,slider.value]).toEqual(['3','30','1','9']);
  fireEvent.change(slider,{target:{value:'11'}});expect(onPatchSettings).toHaveBeenLastCalledWith({quotaCarouselIntervalSeconds:11},true);
  fireEvent.click(getByRole('checkbox',{name:'Automatically rotate quotas'}));expect(onPatchSettings).toHaveBeenLastCalledWith({quotaCarouselEnabled:false});
  fireEvent.click(panel('claude').getByRole('checkbox',{name:'Include in quota rotation'}));
  expect(onPatchSettings).toHaveBeenLastCalledWith({providerPreferences:{claude:{carouselIncluded:false}}});
});
