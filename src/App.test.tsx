import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import i18n from './i18n';
import { DEFAULT_SETTINGS, OverlaySettings } from './types';

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  setTitle: vi.fn(async () => {}),
  setSize: vi.fn(async () => {}),
  windowLabel: 'settings',
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    label: tauri.windowLabel,
    setTitle: tauri.setTitle,
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

const savedSettings: OverlaySettings = {
  ...DEFAULT_SETTINGS,
  scalePercent: 220,
  backgroundTransparencyPercent: 50,
  showCredits: false,
  refreshIntervalSeconds: 300,
  language: 'en-US',
  overlayLayout: 'stacks',
  autoStart: false,
  autoCheckUpdates: true,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((finish) => { resolve = finish; });
  return { promise, resolve };
}

function slider(name: string) {
  return screen.getByRole('slider', { name }) as HTMLInputElement;
}

beforeEach(async () => {
  tauri.windowLabel = 'settings';
  tauri.invoke.mockReset();
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_settings') return savedSettings;
    if (command === 'is_installed_version') return true;
    return null;
  });
  tauri.listeners.clear();
  await i18n.changeLanguage('en-US');
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('settings window', () => {
  it('updates the overlay layout when another window broadcasts settings', async () => {
    tauri.windowLabel = 'main';
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    const { container } = render(<App />);
    await waitFor(() => expect(container.querySelector('[data-layout="stacks"]')).not.toBeNull());
    await act(async () => {
      tauri.listeners.get('settings_updated')!({ payload: { ...savedSettings, overlayLayout: 'grouped', showCredits: true } });
    });
    expect(container.querySelector('[data-layout="grouped"]')).not.toBeNull();
    expect(screen.getByRole('group', { name: 'CREDITS' })).toBeTruthy();
    expect(tauri.invoke).not.toHaveBeenCalledWith('save_settings', expect.anything());
  });
  it('switches layouts immediately without changing other preferences', async () => {
    render(<App />);
    const selector = await screen.findByRole('combobox', { name: 'Overlay layout' }) as HTMLSelectElement;
    fireEvent.change(selector, { target: { value: 'grouped' } });
    expect(selector.value).toBe('grouped');
    expect(tauri.invoke).toHaveBeenCalledWith('save_settings', {
      newSettings: { ...savedSettings, overlayLayout: 'grouped' },
    });
    fireEvent.change(selector, { target: { value: 'stacks' } });
    expect(selector.value).toBe('stacks');
    expect(tauri.invoke).toHaveBeenLastCalledWith('save_settings', { newSettings: savedSettings });
  });
  it('waits for saved settings and preserves other preferences when editing', async () => {
    const loading = deferred<OverlaySettings>();
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return loading.promise;
      if (command === 'is_installed_version') return true;
      return null;
    });

    render(<App />);
    expect(screen.getByRole('status').textContent).toBe('Loading settings...');
    expect(screen.queryByRole('slider')).toBeNull();
    expect(tauri.invoke).not.toHaveBeenCalledWith('save_settings', expect.anything());

    await act(async () => { loading.resolve(savedSettings); });
    expect(slider('Overlay Size').value).toBe('220');
    expect(slider('Background Transparency').value).toBe('50');
    expect((screen.getByRole('combobox', { name: 'Overlay layout' }) as HTMLSelectElement).value).toBe('stacks');
    expect((screen.getByRole('checkbox', { name: 'Show Credit Balance' }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole('combobox', { name: 'Refresh Interval' }) as HTMLSelectElement).value).toBe('300');

    fireEvent.change(slider('Overlay Size'), { target: { value: '200' } });
    expect(slider('Overlay Size').value).toBe('200');
    expect(tauri.invoke).toHaveBeenCalledWith('save_settings', {
      newSettings: { ...savedSettings, scalePercent: 200 },
    });

    fireEvent.change(slider('Background Transparency'), { target: { value: '40' } });
    expect(tauri.invoke).toHaveBeenLastCalledWith('save_settings', {
      newSettings: { ...savedSettings, scalePercent: 200, backgroundTransparencyPercent: 40 },
    });
  });

  it('reflects external settings updates and uses them for subsequent edits', async () => {
    render(<App />);
    await screen.findByRole('slider', { name: 'Overlay Size' });
    await screen.findByRole('checkbox', { name: 'Start automatically on boot' });

    const updated: OverlaySettings = {
      ...savedSettings,
      scalePercent: 150,
      backgroundTransparencyPercent: 10,
      showCredits: true,
      autoStart: true,
      language: 'zh-CN',
      refreshIntervalSeconds: 120,
      overlayLayout: 'grouped',
    };
    await act(async () => {
      tauri.listeners.get('settings_updated')!({ payload: updated });
    });

    expect(slider('浮窗大小').value).toBe('150');
    expect(slider('背景透明度').value).toBe('10');
    expect((screen.getByRole('combobox', { name: '浮窗布局' }) as HTMLSelectElement).value).toBe('grouped');
    expect((screen.getByRole('checkbox', { name: '开机时自动启动' }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole('combobox', { name: '刷新频率' }) as HTMLSelectElement).value).toBe('120');
    fireEvent.click(screen.getByRole('checkbox', { name: '显示 Credit 余额' }));
    expect(tauri.invoke).toHaveBeenCalledWith('save_settings', {
      newSettings: { ...updated, showCredits: false },
    });
  });

  it('keeps controls unavailable after a load failure and supports retry', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const retry = deferred<OverlaySettings>();
    let attempts = 0;
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') {
        if (++attempts === 1) throw new Error('Read failed');
        return retry.promise;
      }
      return null;
    });

    render(<App />);
    expect((await screen.findByRole('alert')).textContent).toBe('Unable to load settings. Please try again.');
    expect(screen.queryByRole('slider')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(screen.getByRole('status').textContent).toBe('Loading settings...');
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
    expect(screen.queryByRole('slider')).toBeNull();

    await act(async () => { retry.resolve(savedSettings); });
    expect(slider('Overlay Size').value).toBe('220');
    expect(attempts).toBe(2);
    expect(tauri.invoke).not.toHaveBeenCalledWith('save_settings', expect.anything());
  });

  it('does not replace a newer broadcast with a delayed initial read', async () => {
    const loading = deferred<OverlaySettings>();
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return loading.promise;
      return null;
    });
    render(<App />);
    const updated = { ...savedSettings, scalePercent: 150 };
    await act(async () => {
      tauri.listeners.get('settings_updated')!({ payload: updated });
    });
    expect(slider('Overlay Size').value).toBe('150');
    await act(async () => { loading.resolve(savedSettings); });
    expect(slider('Overlay Size').value).toBe('150');
    fireEvent.change(slider('Background Transparency'), { target: { value: '40' } });
    expect(tauri.invoke).toHaveBeenCalledWith('save_settings', {
      newSettings: { ...updated, backgroundTransparencyPercent: 40 },
    });
  });

  it('localizes the loading error and retry action', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    await i18n.changeLanguage('zh-CN');
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') throw new Error('Read failed');
      return null;
    });

    render(<App />);
    expect(screen.getByRole('status').textContent).toBe('正在加载设置…');
    expect((await screen.findByRole('alert')).textContent).toBe('无法加载设置，请重试。');
    expect(screen.getByRole('button', { name: '重试' })).toBeTruthy();
  });

  it('does not offer autostart for portable builds', async () => {
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return savedSettings;
      if (command === 'is_installed_version') return false;
      return null;
    });
    render(<App />);
    await screen.findByRole('slider', { name: 'Overlay Size' });
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('is_installed_version'));
    expect(screen.queryByRole('checkbox', { name: 'Start automatically on boot' })).toBeNull();
    expect(tauri.invoke).not.toHaveBeenCalledWith('set_autostart', expect.anything());
  });

  it('localizes the loading error and retry action in Traditional Chinese (zh-Hant)', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    await i18n.changeLanguage('zh-Hant');
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') throw new Error('Read failed');
      return null;
    });

    render(<App />);
    expect(screen.getByRole('status').textContent).toBe('正在載入設定…');
    expect((await screen.findByRole('alert')).textContent).toBe('無法載入設定，請重試。');
    expect(screen.getByRole('button', { name: '重試' })).toBeTruthy();
  });
});

describe('matchSupportedLocale', () => {
  it('matches exact Traditional Chinese string literals', async () => {
    const { matchSupportedLocale } = await import('./i18n');
    expect(matchSupportedLocale('zh-TW')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh_TW')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh-HK')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh-MO')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh-Hant')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh-Hant-TW')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh-Hant-HK')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh-Hant-MO')).toBe('zh-Hant');
  });

  it('matches exact Simplified Chinese string literals', async () => {
    const { matchSupportedLocale } = await import('./i18n');
    expect(matchSupportedLocale('zh')).toBe('zh-CN');
    expect(matchSupportedLocale('zh-CN')).toBe('zh-CN');
    expect(matchSupportedLocale('zh_CN')).toBe('zh-CN');
    expect(matchSupportedLocale('zh-SG')).toBe('zh-CN');
    expect(matchSupportedLocale('zh-Hans')).toBe('zh-CN');
    expect(matchSupportedLocale('zh-Hans-CN')).toBe('zh-CN');
    expect(matchSupportedLocale('zh-Hans-SG')).toBe('zh-CN');
  });

  it('falls back to en-US for other locales', async () => {
    const { matchSupportedLocale } = await import('./i18n');
    expect(matchSupportedLocale('en-US')).toBe('en-US');
    expect(matchSupportedLocale('en-GB')).toBe('en-US');
    expect(matchSupportedLocale('ja-JP')).toBe('en-US');
    expect(matchSupportedLocale('fr-FR')).toBe('en-US');
    expect(matchSupportedLocale('')).toBe('en-US');
  });
});

