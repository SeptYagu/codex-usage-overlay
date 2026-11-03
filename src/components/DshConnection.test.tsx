import { StrictMode } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DshConnectionView } from './DshConnection';
import { DEFAULT_SETTINGS, DshConnectionStatus, OverlaySettings } from '../types';
import i18n from '../i18n';

const tauri = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
const disconnected: DshConnectionStatus = {
  connected: false, origin: null, instanceId: null, activeAccountId: null,
  connectionRevision: 0, autoPhase: 'discovering', autoMessage: null, autoRevision: 1,
  accounts: [], sessions: [], error: null,
};
// The public DSH projection preserves modelSelection.lastUsed.provider exactly:
// openai-codex, not the overlay's local provider key "codex" (parser fixtures).
const firstSession = 'session-f5409f91-ec39-462d-9964-596e18e2a36c';
const secondSession = 'session-9505417c-f90b-4933-a568-4dff342b54ad';
const connected: DshConnectionStatus = {
  connected: true, origin: 'http://127.0.0.1:3210', instanceId: 'public-instance', activeAccountId: 'a',
  connectionRevision: 4, autoPhase: 'ready', autoMessage: null, autoRevision: 5, error: null,
  accounts: [{ id: 'a', label: 'A', active: true }, { id: 'b', label: 'B', active: false }],
  sessions: [
    { id: firstSession, title: 'Fixture session', provider: 'openai-codex', busy: true },
    { id: secondSession, title: 'Second task', provider: 'openai-codex', busy: false },
    { id: 'wrong-codex', title: 'Wrong local provider', provider: 'codex', busy: true },
    { id: 'claude-session', title: 'Claude task', provider: 'anthropic', busy: true },
  ],
};
const settings: OverlaySettings = { ...DEFAULT_SETTINGS, provider: 'dsh' };
const boundAccount: OverlaySettings = { ...settings, dshInstanceId: 'public-instance', dshAccountId: 'a' };
let cached: DshConnectionStatus;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
async function settle() { await act(async () => {}); }
async function poll() { await act(async () => { await vi.advanceTimersByTimeAsync(3_000); }); }
async function openAdvanced() {
  fireEvent.click(screen.getByText('Advanced: manual connection link (CLI Web)'));
  return screen.findByLabelText('DSH connection link') as Promise<HTMLInputElement>;
}
function sessionInput() { return screen.getByRole('checkbox', { name: 'Fixture session · Busy' }) as HTMLInputElement; }

beforeEach(async () => {
  cached = disconnected;
  tauri.invoke.mockReset();
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'dsh_connection_status') return cached;
    if (command === 'dsh_connect' || command === 'dsh_disconnect') return cached;
    if (command === 'dsh_bind_targets') return { revision: 2, settings: boundAccount };
    if (command === 'dsh_auto_retry' || command === 'dsh_auto_setup') return undefined;
    throw new Error(`Unexpected command: ${command}`);
  });
  await i18n.changeLanguage('en-US');
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it('mounts passively and polls only cached status every three seconds, with no manual prerequisites', async () => {
  vi.useFakeTimers();
  const patch = vi.fn();
  render(<DshConnectionView settings={settings} onPatchSettings={patch} />);
  await settle();
  expect(screen.getByRole('status').textContent).toBe('Looking for / connecting to DSH…');
  expect(tauri.invoke.mock.calls).toEqual([['dsh_connection_status']]);
  expect(screen.queryByLabelText('DSH bridge instance')).toBeNull();
  expect(screen.queryByLabelText('DSH connection link')).toBeNull();
  expect(screen.queryByText(/Desktop: install and enable/)).toBeNull();
  expect(screen.getByText(/No manual plugin installation or pairing is required/)).toBeTruthy();
  await act(async () => { await vi.advanceTimersByTimeAsync(2_999); });
  expect(tauri.invoke).toHaveBeenCalledTimes(1);
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  expect(tauri.invoke.mock.calls).toEqual([['dsh_connection_status'], ['dsh_connection_status']]);
  expect(patch).not.toHaveBeenCalled();
});

it.each([
  ['inactive', 'Not connected'],
  ['discovering', 'Looking for / connecting to DSH…'],
  ['connecting', 'Looking for / connecting to DSH…'],
  ['setting-up', 'Configuring connection automatically…'],
  ['restart-required', 'Waiting for DSH hot reload'],
  ['ambiguous', 'Multiple DSH instances found; automatic connection needs attention'],
  ['suspended', 'Automatic connection suspended'],
  ['error', 'Automatic connection failed'],
])('shows native automatic phase %s without starting work', async (autoPhase, label) => {
  cached = { ...disconnected, autoPhase };
  render(<DshConnectionView settings={settings} onPatchSettings={vi.fn()} />);
  await settle();
  expect(screen.getByRole('status').textContent).toBe(label);
  expect(tauri.invoke.mock.calls).toEqual([['dsh_connection_status']]);
  if (autoPhase === 'restart-required') {
    expect(screen.getByText(/restart DSH yourself/)).toBeTruthy();
    expect(screen.getByText(/will not restart DSH/)).toBeTruthy();
  }
});

it.each(['ready', 'connected-unbound'])('shows %s as read-only and never automatically binds sessions or patches settings', async autoPhase => {
  cached = { ...connected, autoPhase };
  const patch = vi.fn();
  const view = render(<DshConnectionView settings={settings} onPatchSettings={patch} />);
  await settle();
  expect(screen.getByRole('status').textContent).toBe('Connected (quota read-only; no pause sessions bound)');
  expect((screen.getByLabelText('DSH subscription account') as HTMLSelectElement).value).toBe('');
  // The native singleton selects the active account data-only and broadcasts
  // settings. Rendering that broadcast must not perform a second write.
  view.rerender(<DshConnectionView settings={boundAccount} onPatchSettings={patch} />);
  expect((screen.getByLabelText('DSH subscription account') as HTMLSelectElement).value).toBe('a');
  for (const input of screen.getAllByRole('checkbox') as HTMLInputElement[]) expect(input.checked).toBe(false);
  expect(screen.queryByRole('checkbox', { name: /Wrong local provider/ })).toBeNull();
  expect(screen.queryByRole('checkbox', { name: /Claude task/ })).toBeNull();
  expect(tauri.invoke.mock.calls).toEqual([['dsh_connection_status']]);
  expect(patch).not.toHaveBeenCalled();
});

it.each([
  ['Retry connection', 'dsh_auto_retry', 'connecting'],
  ['Set up connection automatically', 'dsh_auto_setup', 'setting-up'],
])('runs %s only on click, with no arguments, and then reads cached status', async (label, command, autoPhase) => {
  cached = { ...disconnected, autoPhase: 'error' };
  const pending = deferred<void>();
  tauri.invoke.mockImplementation(async (name: string) => name === command ? pending.promise : cached);
  render(<DshConnectionView settings={settings} onPatchSettings={vi.fn()} />);
  await settle();
  expect(tauri.invoke).not.toHaveBeenCalledWith(command);
  fireEvent.click(screen.getByRole('button', { name: label }));
  expect(tauri.invoke).toHaveBeenCalledWith(command);
  expect(tauri.invoke.mock.calls.find(([name]) => name === command)?.length).toBe(1);
  expect((screen.getByRole('button', { name: 'Working…' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: label === 'Retry connection' ? 'Set up connection automatically' : 'Retry connection' }));
  expect(tauri.invoke.mock.calls.filter(([name]) => name === command)).toHaveLength(1);
  cached = { ...disconnected, autoPhase, autoRevision: 2 };
  await act(async () => { pending.resolve(); });
  expect(screen.getByRole('status').textContent).toBe(i18n.t(autoPhase === 'setting-up' ? 'dshAutoSettingUp' : 'dshAutoSearching'));
  expect(tauri.invoke.mock.calls.slice(-1)[0]).toEqual(['dsh_connection_status']);
});

it('binds only explicit real openai-codex selections with atomic source guards and never replays patch_settings', async () => {
  cached = connected;
  const patch = vi.fn();
  const view = render(<DshConnectionView settings={boundAccount} onPatchSettings={patch} />);
  await settle();
  fireEvent.click(sessionInput());
  await settle();
  expect(tauri.invoke).toHaveBeenCalledWith('dsh_bind_targets', {
    accountId: 'a', sessionIds: [firstSession], expectedInstanceId: 'public-instance', expectedConnectionRevision: 4,
  });
  expect(sessionInput().checked).toBe(false); // Await the authoritative broadcast, not optimistic consent.
  view.rerender(<DshConnectionView settings={{ ...boundAccount, dshSessionIds: [firstSession] }} onPatchSettings={patch} />);
  expect(sessionInput().checked).toBe(true);
  expect(screen.getByRole('status').textContent).toBe('Connected (pause sessions explicitly bound)');
  fireEvent.change(screen.getByLabelText('DSH subscription account'), { target: { value: 'b' } });
  await settle();
  expect(tauri.invoke).toHaveBeenLastCalledWith('dsh_connection_status');
  expect(tauri.invoke).toHaveBeenCalledWith('dsh_bind_targets', {
    accountId: 'b', sessionIds: [], expectedInstanceId: 'public-instance', expectedConnectionRevision: 4,
  });
  expect(patch).not.toHaveBeenCalled();
  expect(tauri.invoke.mock.calls.some(([name]) => ['patch_settings', 'resume_chat', 'fetch_usage', 'dsh_pause', 'claude_connect'].includes(name))).toBe(false);
});

it('fails closed on rejected binding, refreshes identity and permits a deliberate guarded retry', async () => {
  cached = connected;
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'dsh_bind_targets') throw new Error('secret-binding-error');
    return cached;
  });
  const { container } = render(<DshConnectionView settings={boundAccount} onPatchSettings={vi.fn()} />);
  await settle();
  fireEvent.click(sessionInput());
  await settle();
  expect(screen.getByRole('alert').textContent).toContain('binding failed');
  expect(sessionInput().checked).toBe(false);
  expect(sessionInput().disabled).toBe(false);
  expect(container.textContent).not.toContain('secret-binding-error');
  expect(tauri.invoke.mock.calls.slice(-1)[0]).toEqual(['dsh_connection_status']);
});

it('does not mix old bindings with another instance even when it reuses session IDs', async () => {
  vi.useFakeTimers();
  cached = connected;
  const patch = vi.fn();
  const view = render(<DshConnectionView settings={{ ...boundAccount, dshSessionIds: [firstSession] }} onPatchSettings={patch} />);
  await settle();
  expect(sessionInput().checked).toBe(true);
  cached = { ...connected, instanceId: 'replacement-instance', origin: 'http://127.0.0.1:4321', connectionRevision: 5, autoRevision: 6 };
  await poll();
  expect(sessionInput().checked).toBe(false);
  expect((screen.getByLabelText('DSH subscription account') as HTMLSelectElement).value).toBe('');
  expect(screen.getByRole('status').textContent).toContain('no pause sessions bound');
  expect(screen.queryByText('http://127.0.0.1:3210')).toBeNull();
  // Native reconnect settings broadcast also contains no pause targets.
  view.rerender(<DshConnectionView settings={{ ...boundAccount, dshInstanceId: 'replacement-instance', dshSessionIds: [] }} onPatchSettings={patch} />);
  fireEvent.click(sessionInput());
  await settle();
  expect(tauri.invoke).toHaveBeenCalledWith('dsh_bind_targets', {
    accountId: 'a', sessionIds: [firstSession], expectedInstanceId: 'replacement-instance', expectedConnectionRevision: 5,
  });
  expect(patch).not.toHaveBeenCalled();
});

it('does not resurrect old selections after reconnecting to the same instance with a new connection revision', async () => {
  vi.useFakeTimers();
  cached = connected;
  render(<DshConnectionView settings={{ ...boundAccount, dshSessionIds: [firstSession] }} onPatchSettings={vi.fn()} />);
  await settle();
  expect(sessionInput().checked).toBe(true);
  cached = { ...connected, connectionRevision: 5, autoRevision: 6 };
  await poll();
  expect(sessionInput().checked).toBe(false);
  fireEvent.click(sessionInput());
  await settle();
  expect(tauri.invoke).toHaveBeenCalledWith('dsh_bind_targets', {
    accountId: 'a', sessionIds: [firstSession], expectedInstanceId: 'public-instance', expectedConnectionRevision: 5,
  });
  expect(sessionInput().checked).toBe(true); // Only the guarded explicit rebind can restore identical IDs.
});

it('ignores a late binding result after the native source changes while binding is pending', async () => {
  vi.useFakeTimers();
  cached = connected;
  const pending = deferred<unknown>();
  tauri.invoke.mockImplementation(async (command: string) => command === 'dsh_bind_targets' ? pending.promise : cached);
  const patch = vi.fn();
  render(<DshConnectionView settings={boundAccount} onPatchSettings={patch} />);
  await settle();
  fireEvent.click(sessionInput());
  cached = { ...connected, instanceId: 'replacement-instance', connectionRevision: 5, autoRevision: 6 };
  await poll();
  await act(async () => { pending.resolve({ revision: 9, settings: { ...boundAccount, dshSessionIds: [firstSession] } }); });
  expect(sessionInput().checked).toBe(false);
  expect((screen.getByLabelText('DSH subscription account') as HTMLSelectElement).value).toBe('');
  expect(screen.getByRole('status').textContent).toContain('no pause sessions bound');
  expect(patch).not.toHaveBeenCalled();
  expect(tauri.invoke.mock.calls.filter(([name]) => name === 'dsh_bind_targets')).toHaveLength(1);
});

it('does not refresh or apply a pending binding result after unmount', async () => {
  cached = connected;
  const pending = deferred<unknown>();
  tauri.invoke.mockImplementation(async (command: string) => command === 'dsh_bind_targets' ? pending.promise : cached);
  const patch = vi.fn();
  const view = render(<DshConnectionView settings={boundAccount} onPatchSettings={patch} />);
  await settle();
  fireEvent.click(sessionInput());
  view.unmount();
  await act(async () => { pending.resolve({ revision: 9, settings: { ...boundAccount, dshSessionIds: [firstSession] } }); });
  expect(tauri.invoke.mock.calls.map(([name]) => name)).toEqual(['dsh_connection_status', 'dsh_bind_targets']);
  expect(patch).not.toHaveBeenCalled();
});

it.each(['connecting', 'setting-up', 'restart-required', 'error'])('does not authorize binding during transitional native phase %s even if a stale connected flag remains', async autoPhase => {
  cached = { ...connected, autoPhase };
  render(<DshConnectionView settings={boundAccount} onPatchSettings={vi.fn()} />);
  await settle();
  expect((screen.getByLabelText('DSH subscription account') as HTMLSelectElement).disabled).toBe(true);
  expect(sessionInput().closest('fieldset')?.disabled).toBe(true);
  fireEvent.click(sessionInput());
  await settle();
  expect(tauri.invoke.mock.calls).toEqual([['dsh_connection_status']]);
});

it('orders overlapping status responses by autoRevision and rejects a later request with an older cache revision', async () => {
  vi.useFakeTimers();
  const old = deferred<DshConnectionStatus>();
  tauri.invoke.mockImplementation(async () => cached);
  tauri.invoke.mockImplementationOnce(() => old.promise);
  render(<DshConnectionView settings={boundAccount} onPatchSettings={vi.fn()} />);
  cached = connected;
  await poll();
  expect(screen.getByRole('status').textContent).toContain('Connected');
  await act(async () => { old.resolve({ ...disconnected, autoRevision: 2 }); });
  expect(screen.getByRole('status').textContent).toContain('Connected');
  cached = { ...disconnected, autoRevision: 3 };
  await poll();
  expect(screen.getByRole('status').textContent).toContain('Connected');
  cached = { ...disconnected, autoRevision: 6, autoPhase: 'setting-up' };
  await poll();
  expect(screen.getByRole('status').textContent).toBe('Configuring connection automatically…');
});

it('invalidates reads started before an explicit retry, even with a misleading higher revision', async () => {
  const old = deferred<DshConnectionStatus>();
  tauri.invoke.mockImplementationOnce(() => old.promise);
  render(<DshConnectionView settings={boundAccount} onPatchSettings={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: 'Retry connection' }));
  await settle();
  await act(async () => { old.resolve({ ...connected, autoRevision: 99 }); });
  expect(screen.getByRole('status').textContent).toBe('Looking for / connecting to DSH…');
  expect(screen.queryByRole('checkbox')).toBeNull();
});

it('disables stale binding controls after status failure and recovers through polling', async () => {
  vi.useFakeTimers();
  cached = connected;
  render(<DshConnectionView settings={boundAccount} onPatchSettings={vi.fn()} />);
  await settle();
  tauri.invoke.mockRejectedValueOnce(new Error('secret-status-error'));
  await poll();
  expect(screen.getByRole('alert').textContent).not.toContain('secret-status-error');
  expect((screen.getByLabelText('DSH subscription account') as HTMLSelectElement).disabled).toBe(true);
  expect(screen.queryByRole('checkbox')).toBeNull();
  await poll();
  expect(sessionInput().checked).toBe(false);
  expect(sessionInput().disabled).toBe(false);
});

it('cancels polling on unmount and ignores late status/action results without post-unmount reads', async () => {
  vi.useFakeTimers();
  const status = deferred<DshConnectionStatus>();
  const action = deferred<void>();
  tauri.invoke.mockImplementation((command: string) => command === 'dsh_connection_status' ? status.promise : action.promise);
  const view = render(<DshConnectionView settings={boundAccount} onPatchSettings={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: 'Set up connection automatically' }));
  view.unmount();
  await act(async () => { status.resolve(connected); action.resolve(); });
  await poll();
  expect(tauri.invoke.mock.calls).toEqual([['dsh_connection_status'], ['dsh_auto_setup']]);
});

it('rejects an obsolete mount response when StrictMode replays the effect', async () => {
  const old = deferred<DshConnectionStatus>();
  tauri.invoke.mockImplementationOnce(() => old.promise);
  render(<StrictMode><DshConnectionView settings={boundAccount} onPatchSettings={vi.fn()} /></StrictMode>);
  await settle();
  expect(tauri.invoke.mock.calls).toEqual([['dsh_connection_status'], ['dsh_connection_status']]);
  await act(async () => { old.resolve({ ...connected, autoRevision: 99 }); });
  expect(screen.getByRole('status').textContent).toBe('Looking for / connecting to DSH…');
});

it('keeps optional advanced pairing local, password-masked and cleared on success or collapse', async () => {
  const setItem = vi.fn();
  vi.stubGlobal('localStorage', { setItem });
  const patch = vi.fn();
  render(<DshConnectionView settings={settings} onPatchSettings={patch} />);
  await settle();
  expect(screen.queryByLabelText('DSH connection link')).toBeNull();
  const input = await openAdvanced();
  expect(input.type).toBe('password');
  const connectionUrl = 'http://127.0.0.1:3210/connect?token=test-pairing-secret';
  fireEvent.change(input, { target: { value: connectionUrl } });
  cached = connected;
  fireEvent.click(screen.getByRole('button', { name: 'Pair with link' }));
  await settle();
  expect(tauri.invoke).toHaveBeenCalledWith('dsh_connect', { connectionUrl });
  expect(input.value).toBe('');
  expect(patch).not.toHaveBeenCalled();
  expect(setItem).not.toHaveBeenCalled();
  fireEvent.change(input, { target: { value: connectionUrl } });
  fireEvent.click(screen.getByText('Advanced: manual connection link (CLI Web)'));
  await waitFor(() => expect(screen.queryByLabelText('DSH connection link')).toBeNull());
  expect((await openAdvanced()).value).toBe('');
});

it('never renders or logs raw errors/auto messages or secret URL components', async () => {
  cached = { ...disconnected, origin: 'http://user:secret@127.0.0.1:3210/connect?token=secret#secret', autoPhase: 'error', error: 'secret', autoMessage: 'secret' };
  const log = vi.spyOn(console, 'error').mockImplementation(() => {});
  const { container } = render(<DshConnectionView settings={settings} onPatchSettings={vi.fn()} />);
  await settle();
  expect(container.textContent).not.toContain('secret');
  expect(screen.getByText('http://127.0.0.1:3210')).toBeTruthy();
  await openAdvanced();
  fireEvent.change(screen.getByLabelText('DSH connection link'), { target: { value: 'http://dsh.test/connect?token=secret' } });
  tauri.invoke.mockRejectedValueOnce(new Error('secret'));
  fireEvent.click(screen.getByRole('button', { name: 'Pair with link' }));
  await settle();
  expect(container.textContent).not.toContain('secret');
  expect(log).not.toHaveBeenCalled();
});

describe.each(['en-US', 'zh-CN', 'zh-Hant'])('DSH translations in %s', locale => {
  it('has every DSH label translated with no manual installation step in the primary view', async () => {
    await i18n.changeLanguage(locale);
    render(<DshConnectionView settings={settings} onPatchSettings={vi.fn()} />);
    await settle();
    const resources = i18n.getResourceBundle(locale, 'translation');
    const english = i18n.getResourceBundle('en-US', 'translation');
    for (const key of Object.keys(english).filter(key => key.startsWith('dsh') || ['providerDsh', 'quotaThresholdZeroHint'].includes(key))) {
      expect(resources[key], key).toBeTruthy();
      expect(i18n.t(key)).not.toBe(key);
    }
    expect(screen.getByRole('heading', { name: i18n.t('dshConnection') })).toBeTruthy();
    expect(screen.getByRole('button', { name: i18n.t('dshRetry') })).toBeTruthy();
    expect(screen.queryByLabelText(i18n.t('dshConnectionUrl'))).toBeNull();
  });
});
