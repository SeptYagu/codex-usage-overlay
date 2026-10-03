import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import i18n, { matchSupportedLocale } from './i18n';
import { CodexUsage, DEFAULT_SETTINGS, OverlaySettings, SettingsEnvelope } from './types';

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
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((finish) => { resolve = finish; });
  return { promise, resolve };
}

let serverSettings: OverlaySettings;
let serverRevision: number;

beforeEach(async () => {
  tauri.windowLabel = 'settings';
  tauri.invoke.mockReset();
  tauri.listeners.clear();
  serverSettings = { ...savedSettings };
  serverRevision = 0;
  tauri.invoke.mockImplementation(async (command: string, args?: { patch?: Partial<OverlaySettings> }) => {
    if (command === 'get_settings') return { revision: serverRevision, settings: { ...serverSettings } };
    if (command === 'patch_settings') {
      serverSettings = { ...serverSettings, ...args?.patch };
      return { revision: ++serverRevision, settings: { ...serverSettings } };
    }
    if (command === 'pick_sound_file') return 'C:\\sounds\\alert.wav';
    if (command === 'is_installed_version') return true;
    return null;
  });
  await i18n.changeLanguage('en-US');
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('settings synchronization', () => {
  it('loads preferences and saves only the changed field', async () => {
    render(<App />);
    const layout = await screen.findByRole('combobox', { name: 'Overlay layout' }) as HTMLSelectElement;
    expect(layout.value).toBe('stacks');
    fireEvent.change(layout, { target: { value: 'grouped' } });
    expect(layout.value).toBe('grouped');
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', { patch: { overlayLayout: 'grouped' } }));
    expect(serverSettings.scalePercent).toBe(220);
    expect(serverSettings.weeklyResetNotification).toBe(true);
  });

  it('keeps quota notifications independent and patches sound mode by field', async () => {
    render(<App />);
    const fiveHour = await screen.findByRole('checkbox', { name: '5-hour quota reset notification Enable notification' });
    fireEvent.click(fiveHour);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { fiveHourResetNotification: false },
    }));
    expect(serverSettings.weeklyResetNotification).toBe(true);

    fireEvent.change(screen.getByRole('combobox', { name: 'Weekly quota reset notification Alert sound' }), {
      target: { value: 'custom' },
    });
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { weeklySoundMode: 'custom' },
    }));
    expect(serverSettings.fiveHourResetNotification).toBe(false);
    expect(serverSettings.weeklySoundMode).toBe('custom');
  });

  it('saves the auto edge hide preference from settings', async () => {
    render(<App />);
    const toggle = await screen.findByRole('checkbox', { name: 'Auto hide at screen edge' });
    fireEvent.click(toggle);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { autoEdgeHide: true },
    }));
  });

  it('saves mouse click-through independently from the settings window', async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole('checkbox', { name: 'Mouse click-through' }));
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { mousePassthrough: true },
    }));
    expect(serverSettings.autoEdgeHide).toBe(false);
  });

  it('enables automatic checks alongside automatic installation', async () => {
    serverSettings.autoCheckUpdates = false;
    render(<App />);
    fireEvent.click(await screen.findByRole('checkbox', { name: 'Allow automatic update installation' }));
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { autoInstallUpdates: true, autoCheckUpdates: true },
    }));
    expect(serverSettings.autoInstallUpdates).toBe(true);
    expect(serverSettings.autoCheckUpdates).toBe(true);
  });

  it('selects, previews, stops, and clears a custom sound path', async () => {
    render(<App />);
    const mode = await screen.findByRole('combobox', { name: 'Weekly quota reset notification Alert sound' });
    fireEvent.change(mode, { target: { value: 'custom' } });
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { weeklySoundMode: 'custom' },
    }));

    fireEvent.click(screen.getByRole('button', { name: 'Choose file…' }));
    expect(await screen.findByText('C:\\sounds\\alert.wav')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    await screen.findByRole('button', { name: 'Stop preview' });
    expect(tauri.invoke).toHaveBeenCalledWith('preview_sound', { kind: 'week' });
    fireEvent.click(screen.getByRole('button', { name: 'Stop preview' }));
    expect(tauri.invoke).toHaveBeenCalledWith('stop_preview_sound');

    fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { weeklySoundPath: null },
    }));
  });

  it('keeps a newer broadcast when the initial read completes late', async () => {
    const initial = deferred<SettingsEnvelope>();
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return initial.promise;
      return null;
    });
    render(<App />);
    await waitFor(() => expect(tauri.listeners.has('settings_updated')).toBe(true));
    await act(async () => {
      tauri.listeners.get('settings_updated')!({ payload: { revision: 2, settings: { ...savedSettings, scalePercent: 150 } } });
    });
    expect((screen.getByRole('slider', { name: 'Overlay Size' }) as HTMLInputElement).value).toBe('150');
    await act(async () => initial.resolve({ revision: 1, settings: savedSettings }));
    expect((screen.getByRole('slider', { name: 'Overlay Size' }) as HTMLInputElement).value).toBe('150');
  });

  it('preserves the latest local edit through an older broadcast and serializes patches', async () => {
    const firstSave = deferred<SettingsEnvelope>();
    let calls = 0;
    tauri.invoke.mockImplementation(async (command: string, args?: { patch?: Partial<OverlaySettings> }) => {
      if (command === 'get_settings') return { revision: 0, settings: savedSettings };
      if (command === 'patch_settings') {
        calls++;
        if (calls === 1) return firstSave.promise;
        return { revision: 2, settings: { ...savedSettings, overlayLayout: 'grouped', showCredits: true, ...args?.patch } };
      }
      if (command === 'is_installed_version') return true;
      return null;
    });
    render(<App />);
    const layout = await screen.findByRole('combobox', { name: 'Overlay layout' });
    fireEvent.change(layout, { target: { value: 'grouped' } });
    fireEvent.click(screen.getByRole('checkbox', { name: 'Show Credit Balance' }));
    await waitFor(() => expect(calls).toBe(1));
    await act(async () => {
      tauri.listeners.get('settings_updated')!({ payload: { revision: 1, settings: savedSettings } });
    });
    expect((screen.getByRole('checkbox', { name: 'Show Credit Balance' }) as HTMLInputElement).checked).toBe(true);
    await act(async () => firstSave.resolve({ revision: 1, settings: { ...savedSettings, overlayLayout: 'grouped' } }));
    await waitFor(() => expect(calls).toBe(2));
    expect((screen.getByRole('combobox', { name: 'Overlay layout' }) as HTMLSelectElement).value).toBe('grouped');
    expect((screen.getByRole('checkbox', { name: 'Show Credit Balance' }) as HTMLInputElement).checked).toBe(true);
  });

  it('coalesces fast slider edits and flushes on pagehide', async () => {
    render(<App />);
    const slider = await screen.findByRole('slider', { name: 'Overlay Size' });
    vi.useFakeTimers();
    fireEvent.change(slider, { target: { value: '200' } });
    fireEvent.change(slider, { target: { value: '205' } });
    expect(tauri.invoke).not.toHaveBeenCalledWith('patch_settings', expect.anything());
    await act(async () => window.dispatchEvent(new Event('pagehide')));
    expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', { patch: { scalePercent: 205 } });
  });

  it('keeps controls unavailable after a failed read and supports retry', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const retry = deferred<SettingsEnvelope>();
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
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await act(async () => retry.resolve({ revision: 0, settings: savedSettings }));
    expect((screen.getByRole('slider', { name: 'Overlay Size' }) as HTMLInputElement).value).toBe('220');
  });

  it('hides autostart in portable builds', async () => {
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return { revision: 0, settings: savedSettings };
      if (command === 'is_installed_version') return false;
      return null;
    });
    render(<App />);
    await screen.findByRole('slider', { name: 'Overlay Size' });
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('is_installed_version'));
    expect(screen.queryByRole('checkbox', { name: 'Start automatically on boot' })).toBeNull();
  });
});

describe('main usage startup', () => {
  it('registers the usage listener before reading cache and does not auto-fetch', async () => {
    tauri.windowLabel = 'main';
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    const cached: CodexUsage = {
      fiveHourRemainingPercent: 69,
      fiveHourBurnRatePerHour: 16.8,
      weekRemainingPercent: 62,
      weekBurnRatePerHour: 0.6,
      creditsDisplay: '12.5',
      creditsBalance: '12.5',
      hasCredits: true,
      fiveHourResetsAt: 1_900_000_000,
      weekResetsAt: 1_900_500_000,
      fetchedAt: 1_800_000_000,
    };
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return { revision: 0, settings: DEFAULT_SETTINGS };
      if (command === 'get_dock_state') {
        return { docked: false, edge: null, expanded: true, hidden: false };
      }
      if (command === 'get_last_usage') {
        expect(tauri.listeners.has('usage_updated')).toBe(true);
        return cached;
      }
      if (command === 'fetch_usage') throw new Error('startup must not fetch');
      return null;
    });

    render(<App />);
    expect(await screen.findByText('69%')).toBeTruthy();
    expect(tauri.invoke).toHaveBeenCalledWith('get_last_usage');
    expect(tauri.invoke).not.toHaveBeenCalledWith('fetch_usage');
  });

  it('does not let an older cache response overwrite a newer live event', async () => {
    tauri.windowLabel = 'main';
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    const cached: CodexUsage = {
      fiveHourRemainingPercent: 69, fiveHourBurnRatePerHour: 16.8,
      weekRemainingPercent: 62, weekBurnRatePerHour: 0.6,
      creditsDisplay: '12.5', creditsBalance: '12.5', hasCredits: true,
      fiveHourResetsAt: 1_900_000_000, weekResetsAt: 1_900_500_000,
      fetchedAt: 1_800_000_000,
    };
    const live = { ...cached, fiveHourRemainingPercent: 68, fetchedAt: cached.fetchedAt + 1 };
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return { revision: 0, settings: DEFAULT_SETTINGS };
      if (command === 'get_dock_state') {
        return { docked: false, edge: null, expanded: true, hidden: false };
      }
      if (command === 'get_last_usage') {
        tauri.listeners.get('usage_updated')!({ payload: live });
        return cached;
      }
      return null;
    });

    render(<App />);
    expect(await screen.findByText('68%')).toBeTruthy();
    await waitFor(() => expect(screen.queryByText('69%')).toBeNull());
  });
});

describe('locale selection', () => {
  it('maps Windows Chinese locales', () => {
    expect(matchSupportedLocale('zh_TW')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh-CN')).toBe('zh-CN');
    expect(matchSupportedLocale('en-US')).toBe('en-US');
  });
});
