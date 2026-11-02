import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DshConnectionView } from './DshConnection';
import { DEFAULT_SETTINGS, DshConnectionStatus } from '../types';
import i18n from '../i18n';

const tauri = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
const disconnected: DshConnectionStatus = { connected: false, instanceId: null, accounts: [], sessions: [], error: null };
const connected: DshConnectionStatus = { connected: true, instanceId: 'public-instance', error: null,
  accounts: [{ id: 'account-a', label: 'Subscription A', active: true }, { id: 'account-b', label: 'Subscription B', active: false }],
  sessions: [{ id: 's1', title: 'First task', provider: 'codex', busy: true }, { id: 's2', title: 'Second task', provider: 'codex', busy: false },
    { id: 'claude-session', title: 'Claude task', provider: 'claude', busy: true }] };
const settings = { ...DEFAULT_SETTINGS, provider: 'dsh' as const };

beforeEach(async () => {
  tauri.invoke.mockReset();
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'dsh_connection_status' || command === 'dsh_disconnect') return disconnected;
    if (command === 'dsh_connect') return connected;
    if (command === 'dsh_discover') return [{ id: 'one', label: 'DSH One', origin: 'http://one.test' }, { id: 'two', label: 'DSH Two', origin: 'http://two.test' }];
    return null;
  });
  await i18n.changeLanguage('en-US');
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('discovers without selecting an instance or account and pairs only the explicitly selected bridge', async () => {
  const patch = vi.fn();
  render(<DshConnectionView settings={settings} onPatchSettings={patch} />);
  await screen.findByRole('option', { name: 'DSH Two · http://two.test' });
  expect((screen.getByLabelText('DSH bridge instance') as HTMLSelectElement).value).toBe('');
  expect((screen.getByRole('button', { name: 'Connect DSH' }) as HTMLButtonElement).disabled).toBe(true);
  expect(tauri.invoke).not.toHaveBeenCalledWith('dsh_connect', expect.anything());
  fireEvent.change(screen.getByLabelText('DSH bridge instance'), { target: { value: 'two' } });
  fireEvent.click(screen.getByRole('button', { name: 'Connect DSH' }));
  await screen.findByText('Connected');
  expect(tauri.invoke).toHaveBeenCalledWith('dsh_connect', { bridgeId: 'two' });
  expect((screen.getByLabelText('DSH subscription account') as HTMLSelectElement).value).toBe('');
  expect(patch).not.toHaveBeenCalled();
  expect(screen.queryByLabelText(/Claude task/)).toBeNull();
});

it('keeps manual pairing secret local, uses password and clears it after success', async () => {
  const setItem = vi.fn();
  vi.stubGlobal('localStorage', { setItem });
  const patch = vi.fn();
  render(<DshConnectionView settings={settings} onPatchSettings={patch} />);
  await act(async () => {});
  fireEvent.click(screen.getByText('Advanced: manual connection link (CLI Web)'));
  const input = screen.getByLabelText('DSH connection link') as HTMLInputElement;
  expect(input.type).toBe('password');
  expect(input.value).toBe('');
  const connectionUrl = 'http://dsh.test/connect?token=test-pairing-secret';
  fireEvent.change(input, { target: { value: connectionUrl } });
  fireEvent.click(screen.getByRole('button', { name: 'Pair with link' }));
  await screen.findByText('Connected');
  expect(tauri.invoke).toHaveBeenCalledWith('dsh_connect', { connectionUrl });
  expect(input.value).toBe('');
  expect(patch).not.toHaveBeenCalled();
  expect(setItem).not.toHaveBeenCalled();
});

it('does not render or log backend errors containing the submitted secret', async () => {
  const error = vi.spyOn(console, 'error').mockImplementation(() => {});
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'dsh_discover') return [];
    if (command === 'dsh_connect') throw new Error('secret-token-must-not-appear');
    return disconnected;
  });
  const { container } = render(<DshConnectionView settings={settings} onPatchSettings={vi.fn()} />);
  await act(async () => {});
  fireEvent.click(screen.getByText('Advanced: manual connection link (CLI Web)'));
  fireEvent.change(screen.getByLabelText('DSH connection link'), { target: { value: 'http://dsh.test/connect?token=secret-token-must-not-appear' } });
  fireEvent.click(screen.getByRole('button', { name: 'Pair with link' }));
  await screen.findByRole('alert');
  expect(container.textContent).not.toContain('secret-token-must-not-appear');
  expect(error).not.toHaveBeenCalled();
});

it('binds accounts and sessions explicitly, clears old sessions on account change, and disconnects without local actions', async () => {
  tauri.invoke.mockImplementation(async (command: string) => command === 'dsh_discover' ? [] : command === 'dsh_disconnect' ? disconnected : connected);
  const patch = vi.fn();
  const view = render(<DshConnectionView settings={settings} onPatchSettings={patch} />);
  await screen.findByText('Connected');
  for (const input of screen.getAllByRole('checkbox') as HTMLInputElement[]) expect(input.checked).toBe(false);
  expect(patch).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText('DSH subscription account'), { target: { value: 'account-b' } });
  expect(patch).toHaveBeenCalledWith({ dshAccountId: 'account-b', dshSessionIds: [] });
  view.rerender(<DshConnectionView settings={{ ...settings, dshAccountId: 'account-b' }} onPatchSettings={patch} />);
  fireEvent.click(screen.getByRole('checkbox', { name: 'First task · Busy' }));
  expect(patch).toHaveBeenCalledWith({ dshSessionIds: ['s1'] });
  fireEvent.click(screen.getByRole('button', { name: 'Disconnect DSH' }));
  await screen.findByText('Not connected');
  expect(tauri.invoke).toHaveBeenCalledWith('dsh_disconnect');
  expect(tauri.invoke.mock.calls.some(([command]) => ['resume_chat', 'fetch_usage', 'claude_connect'].includes(command))).toBe(false);
});

it('disables stale bindings after a failed status refresh', async () => {
  tauri.invoke.mockImplementation(async (command: string) => command === 'dsh_discover' ? [] : connected);
  render(<DshConnectionView settings={{ ...settings, dshAccountId: 'account-a' }} onPatchSettings={vi.fn()} />);
  await screen.findByText('Connected');
  tauri.invoke.mockRejectedValue(new Error('offline'));
  fireEvent.click(screen.getByRole('button', { name: 'Refresh connection' }));
  await screen.findByRole('alert');
  expect((screen.getByLabelText('DSH subscription account') as HTMLSelectElement).disabled).toBe(true);
});

describe.each(['en-US', 'zh-CN', 'zh-Hant'])('DSH translations in %s', locale => {
  it('has all DSH labels without falling back to English or key names', async () => {
    await i18n.changeLanguage(locale);
    render(<DshConnectionView settings={settings} onPatchSettings={vi.fn()} />);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('dsh_discover'));
    const resources = i18n.getResourceBundle(locale, 'translation');
    const english = i18n.getResourceBundle('en-US', 'translation');
    for (const key of Object.keys(english).filter(key => key.startsWith('dsh') || ['providerDsh', 'quotaThresholdZeroHint'].includes(key))) {
      expect(resources[key], key).toBeTruthy();
      expect(i18n.t(key)).not.toBe(key);
    }
    expect(screen.getByRole('heading', { name: i18n.t('dshConnection') })).toBeTruthy();
    expect(screen.getByLabelText(i18n.t('dshConnectionUrl')).getAttribute('type')).toBe('password');
  });
});
