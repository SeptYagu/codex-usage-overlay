import { StrictMode } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { mergeSettingsPatch } from './providerSettings';
import i18n, { matchSupportedLocale } from './i18n';
import { CodexUsage, DEFAULT_SETTINGS, OverlaySettings, SettingsEnvelope, DisplayState, MonitorSnapshot, Provider } from './types';

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  setTitle: vi.fn(async () => {}),
  setSize: vi.fn(async () => {}),
  windowLabel: 'settings',
  monitorRegistration: null as Promise<void> | null,
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
    if (name === 'monitor_updated' && tauri.monitorRegistration) await tauri.monitorRegistration;
    return () => { if(tauri.listeners.get(name)===callback)tauri.listeners.delete(name); };
  },
}));

const savedSettings: OverlaySettings = {
  ...DEFAULT_SETTINGS,
  scalePercent: 220,
  backgroundTransparencyPercent: 50,
  showCredits: false,
  showFiveHourQuota: true,
  showBurnRate: true,
  providerPreferences: {
    ...DEFAULT_SETTINGS.providerPreferences,
    codex: {
      ...DEFAULT_SETTINGS.providerPreferences.codex,
      refreshIntervalSeconds: 300,
    },
  },
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
  tauri.monitorRegistration = null;
  tauri.invoke.mockReset();
  tauri.listeners.clear();
  serverSettings = { ...savedSettings };
  serverRevision = 0;
  tauri.invoke.mockImplementation(async (command: string, args?: { patch?: Partial<OverlaySettings> }) => {
    if (command === 'get_settings') return { revision: serverRevision, settings: { ...serverSettings } };
    if (command === 'patch_settings') {
      serverSettings = mergeSettingsPatch(serverSettings, args?.patch ?? {});
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
    expect(serverSettings.providerPreferences.codex.weeklyResetNotification).toBe(true);
  });

  it('keeps quota notifications independent and patches sound mode by field', async () => {
    const view = render(<App />);
    fireEvent.click(await screen.findByRole('tab', {name:'Local Codex'}));
    await screen.findByRole('heading', {name:'Local Codex'});
    const codex = within(view.container.querySelector('[data-provider-settings="codex"]')! as HTMLElement);
    const fiveHour = await codex.findByRole('checkbox', { name: '5-hour quota reset notification Enable notification' });
    fireEvent.click(fiveHour);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { providerPreferences: {codex:{ fiveHourResetNotification: false }} },
    }));
    expect(serverSettings.providerPreferences.codex.weeklyResetNotification).toBe(true);

    fireEvent.change(codex.getByRole('combobox', { name: 'Weekly quota reset notification Alert sound' }), {
      target: { value: 'custom' },
    });
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { providerPreferences: {codex:{ weeklySoundMode: 'custom' }} },
    }));
    expect(serverSettings.providerPreferences?.codex?.fiveHourResetNotification).toBe(false);
    expect(serverSettings.providerPreferences?.codex?.weeklySoundMode).toBe('custom');
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
    const view = render(<App />);
    fireEvent.click(await screen.findByRole('tab', {name:'Local Codex'}));
    await screen.findByRole('heading', {name:'Local Codex'});
    const codex = within(view.container.querySelector('[data-provider-settings="codex"]')! as HTMLElement);
    const mode = await codex.findByRole('combobox', { name: 'Weekly quota reset notification Alert sound' });
    fireEvent.change(mode, { target: { value: 'custom' } });
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { providerPreferences: {codex:{ weeklySoundMode: 'custom' }} },
    }));

    fireEvent.click(codex.getByRole('button', { name: 'Choose file…' }));
    expect(await codex.findByText('C:\\sounds\\alert.wav')).toBeTruthy();
    fireEvent.click(codex.getByRole('button', { name: 'Preview' }));
    await codex.findByRole('button', { name: 'Stop preview' });
    expect(tauri.invoke).toHaveBeenCalledWith('preview_sound', { kind: 'week', provider:'codex', playbackId:expect.any(String) });
    fireEvent.click(codex.getByRole('button', { name: 'Stop preview' }));
    expect(tauri.invoke).toHaveBeenCalledWith('stop_preview_sound', {playbackId:expect.any(String)});

    fireEvent.click(codex.getByRole('button', { name: 'Clear' }));
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('patch_settings', {
      patch: { providerPreferences: {codex:{ weeklySoundPath: null }} },
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
  const cached: CodexUsage = {
    fiveHourRemainingPercent: 69, fiveHourBurnRatePerHour: 16.8,
    weekRemainingPercent: 62, weekBurnRatePerHour: 0.6,
    creditsDisplay: '12.5', creditsBalance: '12.5', hasCredits: true,
    fiveHourResetsAt: 1_900_000_000, weekResetsAt: 1_900_500_000,
    fetchedAt: 1_800_000_000,
  };

  beforeEach(() => {
    tauri.windowLabel = 'main';
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  });

  const makeSnapshot = (
    provider: Provider,
    usage: CodexUsage | null,
    revision: number,
    sourceKey: string = provider,
    sourceGeneration = 0,
  ): MonitorSnapshot => ({
    provider,
    sourceKey,
    sourceGeneration,
    revision,
    monitorEnabled: true,
    available: Boolean(usage),
    health: usage ? 'current' : 'checking',
    error: null,
    usage,
  });

  function mockSnapshots(response: MonitorSnapshot[] | Promise<MonitorSnapshot[]>) {
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return { revision: 0, settings: DEFAULT_SETTINGS };
      if (command === 'get_dock_state') {
        return { docked: false, edge: null, expanded: true, hidden: false };
      }
      if (command === 'get_monitor_snapshots') {
        expect(tauri.listeners.has('monitor_updated')).toBe(true);
        return response;
      }
      if (command === 'fetch_usage') throw new Error('startup must not fetch');
      return null;
    });
  }

  it('switches to cached background readings immediately and never displays another provider’s late update', async () => {
    const codex = { ...cached, sourceKey: 'codex', sourceGeneration: 0 };
    const claude = { ...cached, fiveHourRemainingPercent: 41, sourceKey: 'claude', sourceGeneration: 0 };
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return { revision: 0, settings: DEFAULT_SETTINGS, sourceKey: 'codex', sourceGeneration: 0 };
      if (command === 'get_dock_state') return { docked: false, edge: null, expanded: true, hidden: false };
      if (command === 'get_monitor_snapshots') return [makeSnapshot('codex', codex, 1), makeSnapshot('claude', claude, 2)];
      if (command === 'cycle_display_provider') return { revision: 1, settings: { ...DEFAULT_SETTINGS, provider: 'claude' }, sourceKey: 'claude', sourceGeneration: 0 };
      if (command === 'fetch_usage') throw new Error('Switching must use cache');
      return null;
    });
    render(<App />);
    await screen.findByText('69%');
    const logo = await screen.findByRole('button', { name: 'Showing Local Codex. Click to show Claude.' });
    fireEvent.click(logo);
    await screen.findByText('41%');
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('codex', { ...codex, fiveHourRemainingPercent: 12 }, 3) }));
    expect(screen.queryByText('12%')).toBeNull();
    expect(screen.getByText('41%')).toBeTruthy();
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('claude', { ...claude, fiveHourRemainingPercent: 40 }, 4) }));
    expect(screen.getByText('40%')).toBeTruthy();
    expect(tauri.invoke).not.toHaveBeenCalledWith('fetch_usage');
  });

  it('orders display events, rejects mismatched source generations and keeps auto pages through ordinary settings saves', async () => {
    const hydration = deferred<DisplayState>();
    const codex = { ...cached, sourceKey: 'codex', sourceGeneration: 0 };
    const claude = { ...cached, fiveHourRemainingPercent: 41, sourceKey: 'claude', sourceGeneration: 1 };
    const selected: DisplayState = { revision: 2, settingsRevision: 0, effectiveProvider: 'claude', manualProvider: 'codex', sourceKey: 'claude', sourceGeneration: 1, carouselEnabled: false, intervalSeconds: 8, candidates: [], pauseReasons: [], status: 'off' };
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return { revision: 0, settings: DEFAULT_SETTINGS, sourceKey: 'codex', sourceGeneration: 0 };
      if (command === 'get_display_state') return hydration.promise;
      if (command === 'get_dock_state') return { docked: false, edge: null, expanded: true, hidden: false };
      if (command === 'get_monitor_snapshots') return [makeSnapshot('codex', codex, 1), makeSnapshot('claude', { ...claude, sourceGeneration: 0 }, 2, 'claude', 0)];
      return null;
    });
    const view = render(<App />);
    await screen.findByText('69%');
    act(() => tauri.listeners.get('display_updated')!({ payload: selected }));
    expect(screen.queryByText('69%')).toBeNull();
    expect(screen.queryByText('41%')).toBeNull();
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('claude', claude, 3, 'claude', 1) }));
    expect(screen.getByText('41%')).toBeTruthy();
    act(() => tauri.listeners.get('settings_updated')!({ payload: { revision: 1, settings: { ...DEFAULT_SETTINGS, language: 'en-US' }, sourceKey: 'codex', sourceGeneration: 0 } }));
    expect(screen.getByText('41%')).toBeTruthy();
    await act(async () => hydration.resolve({ ...selected, revision: 1, effectiveProvider: 'codex', sourceKey: 'codex', sourceGeneration: 0 }));
    expect(screen.getByText('41%')).toBeTruthy();
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('codex', { ...codex, fetchedAt: 9999999999, fiveHourRemainingPercent: 12 }, 4) }));
    expect(screen.queryByText('12%')).toBeNull();
    expect(view.container.querySelector('.overlay-provider svg')?.getAttribute('data-provider')).toBe('claude');
    expect(tauri.invoke).not.toHaveBeenCalledWith('fetch_usage');
  });

  it.each([false, true])('keeps early readings in separate provider snapshot slots across higher foreign generations (StrictMode=%s)', async (strict) => {
    const selected: DisplayState = { revision: 1, settingsRevision: 0, effectiveProvider: 'codex', manualProvider: 'codex', sourceKey: 'codex', sourceGeneration: 0, carouselEnabled: false, intervalSeconds: 8, candidates: [], pauseReasons: [], status: 'off' };
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return { revision: 0, settings: DEFAULT_SETTINGS, sourceKey: 'codex', sourceGeneration: 0 };
      if (command === 'get_display_state') return selected;
      if (command === 'get_dock_state') return { docked: false, edge: null, expanded: true, hidden: false };
      if (command === 'get_monitor_snapshots') return [makeSnapshot('codex', { ...cached, sourceKey: 'codex', sourceGeneration: 0 }, 1)];
      return null;
    });
    render(strict ? <StrictMode><App /></StrictMode> : <App />);
    await screen.findByText('69%');
    act(() => {
      tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('dsh', { ...cached, fiveHourRemainingPercent: 22, sourceKey: 'dsh-old', sourceGeneration: 8 }, 2, 'dsh-old', 8) });
      tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('claude', { ...cached, fiveHourRemainingPercent: 41, sourceKey: 'claude', sourceGeneration: 1 }, 3, 'claude', 1) });
      tauri.listeners.get('display_updated')!({ payload: { ...selected, revision: 2, effectiveProvider: 'claude', sourceKey: 'claude', sourceGeneration: 1 } });
    });
    expect(await screen.findByText('41%')).toBeTruthy();
    expect(screen.queryByText('22%')).toBeNull();
    expect(screen.queryByText('69%')).toBeNull();
  });

  it('registers the monitor listener before reading snapshots and never calls legacy get_last_usage', async () => {
    mockSnapshots([makeSnapshot('codex', cached, 1)]);
    render(<App />);
    expect(await screen.findByText('69%')).toBeTruthy();
    expect(tauri.invoke).toHaveBeenCalledWith('get_monitor_snapshots');
    expect(tauri.invoke).not.toHaveBeenCalledWith('get_last_usage');
    expect(tauri.invoke).not.toHaveBeenCalledWith('fetch_usage');
    expect(tauri.listeners.has('usage_updated')).toBe(false);
  });

  it('keeps a live snapshot revision over delayed hydration snapshots', async () => {
    const hydration = deferred<MonitorSnapshot[]>();
    mockSnapshots(hydration.promise);
    render(<App />);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('get_monitor_snapshots'));
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('codex', { ...cached, fiveHourRemainingPercent: 68 }, 2) }));
    expect(await screen.findByText('68%')).toBeTruthy();
    await act(async () => hydration.resolve([makeSnapshot('codex', cached, 1)]));
    expect(screen.getByText('68%')).toBeTruthy();
    expect(screen.queryByText('69%')).toBeNull();
  });

  it('accepts successive live monitor snapshots in the same second and rejects older samples', async () => {
    mockSnapshots([makeSnapshot('codex', { ...cached, sourceKey: 'codex', sourceGeneration: 0, presentationRevision: 1 }, 1)]);
    render(<App />);
    await screen.findByText('69%');
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('codex', { ...cached, sourceKey: 'codex', sourceGeneration: 0, presentationRevision: 2, fiveHourRemainingPercent: 68 }, 2) }));
    expect(screen.getByText('68%')).toBeTruthy();
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('codex', { ...cached, sourceKey: 'codex', sourceGeneration: 0, presentationRevision: 3, fiveHourRemainingPercent: 67 }, 3) }));
    expect(screen.getByText('67%')).toBeTruthy();
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('codex', { ...cached, sourceKey: 'codex', sourceGeneration: 0, presentationRevision: 1, fiveHourRemainingPercent: 69 }, 4) }));
    expect(screen.getByText('67%')).toBeTruthy();
    expect(screen.queryByText('69%')).toBeNull();
  });

  it.each(['provider', 'account', 'sessions', 'instance', 'generation'] as const)('clears %s changes and rejects late source/generation snapshots even with higher revision', async change => {
    const dshSettings: OverlaySettings = { ...DEFAULT_SETTINGS, provider: 'dsh', dshBaseUrl: 'http://dsh.test', dshInstanceId: 'one', dshAccountId: 'a', dshSessionIds: ['s1'] };
    const initialSettings = change === 'provider' ? DEFAULT_SETTINGS : dshSettings;
    const provider = initialSettings.provider;
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return { revision: 0, settings: initialSettings, sourceKey: 'old-source', sourceGeneration: 1 };
      if (command === 'get_dock_state') return { docked: false, edge: null, expanded: true, hidden: false };
      if (command === 'get_monitor_snapshots') return [makeSnapshot(provider, { ...cached, sourceKey: 'old-source', sourceGeneration: 1 }, 1, 'old-source', 1)];
      return null;
    });
    render(<App />);
    await screen.findByText('69%');
    const nextSettings = {
      ...dshSettings,
      ...(change === 'account' ? { dshAccountId: 'b' } : {}),
      ...(change === 'sessions' ? { dshSessionIds: ['s2'] } : {}),
      ...(change === 'instance' ? { dshInstanceId: 'two' } : {}),
    };
    const key = change === 'generation' ? 'old-source' : 'new-source';
    act(() => tauri.listeners.get('settings_updated')!({ payload: { revision: 1, settings: nextSettings, sourceKey: key, sourceGeneration: 2 } }));
    expect(screen.queryByText('69%')).toBeNull();
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('dsh', { ...cached, fetchedAt: cached.fetchedAt + 99999, sourceKey: 'old-source', sourceGeneration: 1 }, 2, 'old-source', 1) }));
    expect(screen.queryByText('69%')).toBeNull();
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('dsh', { ...cached, fiveHourRemainingPercent: 68, fetchedAt: cached.fetchedAt - 100, sourceKey: key, sourceGeneration: 2 }, 3, key, 2) }));
    expect(screen.getByText('68%')).toBeTruthy();
  });

  it('holds early identified monitor snapshot until matching settings arrive', async () => {
    const initial = deferred<SettingsEnvelope>();
    tauri.invoke.mockImplementation(async (command: string) => {
      if (command === 'get_settings') return initial.promise;
      if (command === 'get_dock_state') return { docked: false, edge: null, expanded: true, hidden: false };
      return null;
    });
    render(<App />);
    await waitFor(() => expect(tauri.listeners.has('monitor_updated')).toBe(true));
    act(() => tauri.listeners.get('monitor_updated')!({ payload: makeSnapshot('dsh', { ...cached, sourceKey: 'dsh-now', sourceGeneration: 4 }, 2, 'dsh-now', 4) }));
    expect(screen.queryByText('69%')).toBeNull();
    await act(async () => initial.resolve({ revision: 3, settings: { ...DEFAULT_SETTINGS, provider: 'dsh' }, sourceKey: 'dsh-now', sourceGeneration: 4 }));
    expect(screen.getByText('69%')).toBeTruthy();
  });

  it('does not start snapshot hydration after cleanup while registration is pending', async () => {
    const registration = deferred<void>();
    tauri.monitorRegistration = registration.promise;
    mockSnapshots([makeSnapshot('codex', cached, 1)]);
    const view = render(<App />);
    view.unmount();
    await act(async () => registration.resolve());
    expect(tauri.invoke).not.toHaveBeenCalledWith('get_monitor_snapshots');
    expect(tauri.listeners.has('monitor_updated')).toBe(false);
  });

  it('allows a pending snapshot response to finish safely after cleanup', async () => {
    const hydration = deferred<MonitorSnapshot[]>();
    mockSnapshots(hydration.promise);
    const view = render(<App />);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('get_monitor_snapshots'));
    view.unmount();
    await act(async () => hydration.resolve([makeSnapshot('codex', cached, 1)]));
    expect(tauri.listeners.has('monitor_updated')).toBe(false);
    expect(view.container.childElementCount).toBe(0);
    expect(tauri.invoke.mock.calls.filter(([command]) => command === 'get_monitor_snapshots')).toHaveLength(1);
  });
});

describe('locale selection', () => {
  it('maps Windows Chinese locales', () => {
    expect(matchSupportedLocale('zh_TW')).toBe('zh-Hant');
    expect(matchSupportedLocale('zh-CN')).toBe('zh-CN');
    expect(matchSupportedLocale('en-US')).toBe('en-US');
  });
});

it('rolls back failed optimistic edits even when reloading settings also fails',async()=>{
  let reads=0;tauri.invoke.mockImplementation(async(command:string)=>{
    if(command==='get_settings'){if(reads++>0)throw new Error('reload failed');return {revision:0,settings:serverSettings};}
    if(command==='patch_settings')throw new Error('disk failed');
    return null;
  });
  render(<App/>);const checkbox=await screen.findByRole('checkbox',{name:'Show Credit Balance'}) as HTMLInputElement;
  expect(checkbox.checked).toBe(false);fireEvent.click(checkbox);
  await waitFor(()=>expect(checkbox.checked).toBe(false));
  expect(await screen.findByRole('alert')).toBeTruthy();
});
