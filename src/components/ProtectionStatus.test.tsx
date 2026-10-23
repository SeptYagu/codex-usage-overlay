import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ProtectionStatusView } from './ProtectionStatus';
import { DEFAULT_SETTINGS, ProtectionStatus } from '../types';
import i18n from '../i18n';

const tauri = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(),
  listeners: new Map<string, (event: { payload: ProtectionStatus }) => void>() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: tauri.listen }));

function snapshot(overrides: Partial<ProtectionStatus> = {}): ProtectionStatus {
  return { revision: 1, enabled: true, autoResumeEnabled: false, connection: 'available',
    capabilities: { cooperativeDelivery: true, directInterrupt: false, completionObservation: false,
      userActivityObservation: false, continuationControl: false },
    mode: 'cooperative', summary: 'pauseRequested', reason: 'stoppingUnconfirmed',
    confirmedCount: 1, acceptedRequestCount: 1, confirmedInterruptionCount: 0,
    lastConfirmedAt: 100, lastProbeAt: 100, lastSuccessfulSampleAt: Math.floor(Date.now() / 1000),
    effectiveIntervalSeconds: 15, nextPollAt: null, quotaHealth: 'success', targets: [],
    resumePlans: [], resumeStateError: null, ...overrides };
}
const settings = { ...DEFAULT_SETTINGS, autoPauseOnLowQuota: true };
beforeEach(async () => {
  tauri.listeners.clear(); tauri.invoke.mockReset(); tauri.listen.mockReset();
  tauri.invoke.mockResolvedValue(snapshot());
  tauri.listen.mockImplementation(async (name, callback) => {
    tauri.listeners.set(name, callback);
    return () => tauri.listeners.delete(name);
  });
  await i18n.changeLanguage('en-US');
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('protection status', () => {
  it('shows acceptance separately from verified stopping and gates resume', async () => {
    const patch = vi.fn();
    render(<ProtectionStatusView settings={settings} onPatchSettings={patch} />);
    await screen.findByText('Pause requested — stopping unconfirmed');
    fireEvent.click(screen.getByText('Protection details'));
    expect(screen.getByText('Requests accepted: 1 · Interruptions verified: 0')).toBeTruthy();
    const resume = screen.getByRole('checkbox', { name: 'Resume automatically after quota reset' }) as HTMLInputElement;
    expect(resume.disabled).toBe(true);
    fireEvent.click(resume);
    expect(patch).not.toHaveBeenCalled();
    expect(tauri.invoke.mock.calls.every(([name]) => name === 'get_auto_pause_status')).toBe(true);
  });
  it('keeps a live event when an older hydration response arrives late', async () => {
    let resolve!: (value: ProtectionStatus) => void;
    tauri.invoke.mockImplementation(() => new Promise<ProtectionStatus>(done => { resolve = done; }));
    render(<ProtectionStatusView settings={settings} />);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalled());
    await act(async () => {
      tauri.listeners.get('auto_pause_status_changed')!({ payload: snapshot({ revision: 5, summary: 'needsAttention', reason: 'bridgeUnavailable' }) });
      resolve(snapshot({ revision: 4, summary: 'armed', reason: null }));
    });
    expect(screen.getByText('Protection needs attention')).toBeTruthy();
    expect(screen.queryByText('Protection armed')).toBeNull();
  });
  it('does not let a late failed hydration hide a newer live status', async () => {
    let reject!: (value: Error) => void;
    tauri.invoke.mockImplementation(() => new Promise((_, fail) => { reject = fail; }));
    render(<ProtectionStatusView settings={settings} compact />);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalled());
    await act(async () => {
      tauri.listeners.get('auto_pause_status_changed')!({ payload: snapshot({ revision: 2, summary: 'armed', reason: null }) });
      reject(new Error('late failure'));
    });
    expect(screen.getByText('Protection armed')).toBeTruthy();
  });
  it('unsubscribes a listener registered after unmount without hydrating', async () => {
    let resolve!: (value: () => void) => void;
    const stop = vi.fn();
    tauri.listen.mockImplementation(() => new Promise<() => void>(done => { resolve = done; }));
    const view = render(<ProtectionStatusView settings={settings} />);
    view.unmount();
    await act(async () => resolve(stop));
    expect(stop).toHaveBeenCalledOnce();
    expect(tauri.invoke).not.toHaveBeenCalled();
  });
  it('shows off immediately when the local setting is disabled', async () => {
    const view = render(<ProtectionStatusView settings={settings} compact />);
    await screen.findByText('Pause requested — stopping unconfirmed');
    view.rerender(<ProtectionStatusView settings={{ ...settings, autoPauseOnLowQuota: false }} compact />);
    expect(screen.getByText('Protection off')).toBeTruthy();
  });
  it('reports status read failures instead of claiming protection is armed', async () => {
    tauri.invoke.mockRejectedValue(new Error('unavailable'));
    render(<ProtectionStatusView settings={settings} />);
    await screen.findByText('Protection needs attention');
    expect(screen.getByText('Protection status is unavailable.')).toBeTruthy();
  });
  it('cancels saved plans only after an explicit button click', async () => {
    tauri.invoke.mockResolvedValue(snapshot({ resumePlans: [
      { id: 'plan', threadId: 'chat', phase: 'needsReview', eligibleAfter: 100, reason: null }
    ] }));
    render(<ProtectionStatusView settings={settings} />);
    fireEvent.click(await screen.findByText('Cancel pending resume plans'));
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('cancel_resume_plans'));
  });
  it.each([
    ['en-US', 'Pause requested — stopping unconfirmed'],
    ['zh-CN', '已请求暂停，尚未确认停止'],
    ['zh-Hant', '已請求暫停，尚未確認停止'],
  ])('renders localized status in %s', async (locale, label) => {
    await i18n.changeLanguage(locale);
    render(<ProtectionStatusView settings={{ ...settings, language: locale }} compact />);
    await screen.findByText(label);
  });
});
