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
  return { revision: 1, enabled: true, provider: 'codex', autoResumeEnabled: false, connection: 'available',
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
  it.each([
    ['en-US', 'Retrying quota read…'],
    ['zh-CN', '正在重试读取额度…'],
    ['zh-Hant', '正在重試讀取額度…'],
  ])('shows localized quota recovery in %s', async (locale, label) => {
    await i18n.changeLanguage(locale);
    tauri.invoke.mockResolvedValue(snapshot({ summary: 'needsAttention', reason: 'quotaReadRetrying', quotaHealth: 'error',
      reader: { phase: 'retrying', attempt: 1, lastSuccess: null, lastFailure: null } }));
    render(<ProtectionStatusView settings={{ ...settings, language: locale }} />);
    await screen.findByText(label);
    expect(screen.queryByText('Protection armed')).toBeNull();
  });
  it('shows the next recovery and keeps failure details after successful recovery', async () => {
    const now = 1_792_800_000;
    vi.spyOn(Date, 'now').mockReturnValue(now * 1000);
    const diagnostic = { startedAt: now - 5, attempt: 1, elapsedMs: 43000, initializeMs: 200,
      quotaMs: 42800, success: false, failure: { stage: 'quota', kind: 'timeout', rpcCode: null, httpStatus: null, retryAfterSeconds: null },
      cliVersion: null, stderr: { modelTimeouts: 0, pluginFailures: 0, otherLines: 0, oversizedLines: 0 }, cleanupFailed: false };
    tauri.invoke.mockResolvedValue(snapshot({ summary: 'needsAttention', reason: 'quotaReadFailed', quotaHealth: 'error', nextPollAt: now + 15,
      reader: { phase: 'error', attempt: 1, lastSuccess: null, lastFailure: diagnostic } }));
    render(<ProtectionStatusView settings={settings} />);
    await screen.findByText('Next quota check in 15s.');
    expect(screen.getByText(/Last failure: Timed out during quota request/)).toBeTruthy();
    await act(async () => {
      tauri.listeners.get('auto_pause_status_changed')!({ payload: snapshot({ revision: 2, summary: 'armed', reason: null,
        reader: { phase: 'success', attempt: 1, lastSuccess: { ...diagnostic, success: true, failure: null, elapsedMs: 450 }, lastFailure: diagnostic } }) });
    });
    expect(screen.getByText('Protection armed')).toBeTruthy();
    expect(screen.getByText(/Last failure: Timed out during quota request/)).toBeTruthy();
    expect(screen.getByText('Last successful read: 450 ms · attempt 1.')).toBeTruthy();
    expect(screen.queryByText('Next quota check in 15s.')).toBeNull();
  });
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
  it.each(['en-US', 'zh-CN', 'zh-Hant'])('keeps DSH cooperative-only and cannot offer local resume in %s even with stale capabilities/plans', async locale => {
    await i18n.changeLanguage(locale);
    tauri.invoke.mockResolvedValue(snapshot({ provider: 'dsh', autoResumeEnabled: true,
      confirmedInterruptionCount: 9,
      capabilities: { cooperativeDelivery: true, directInterrupt: true, completionObservation: true, userActivityObservation: true, continuationControl: true },
      targets: [{ periodId: 'period', active: true, threadId: 'dsh-session', hostId: null, turnId: null, phase: 'interrupted', reason: null, requestedAt: 100, acceptedAt: 101, interruptedAt: 102 }],
      resumePlans: [{ id: 'legacy-plan', threadId: 'local-chat', phase: 'ready', eligibleAfter: 100, reason: null }] }));
    const patch = vi.fn();
    render(<ProtectionStatusView settings={{ ...settings, provider: 'dsh', autoResumeAfterQuotaReset: true }} onPatchSettings={patch} />);
    await screen.findByText(i18n.t('protectionSummaries.pauseRequested'));
    fireEvent.click(screen.getByText(i18n.t('protectionDetails')));
    expect(screen.getByText(i18n.t('protectionCounts', { accepted: 1, stopped: 0 }))).toBeTruthy();
    expect(screen.getByText(new RegExp(i18n.t('protectionPhases.accepted')))).toBeTruthy();
    expect(screen.getByText(i18n.t('dshResumeUnavailable'))).toBeTruthy();
    const resume = screen.getByRole('checkbox', { name: i18n.t('autoResumeAfterQuotaReset') }) as HTMLInputElement;
    expect(resume.disabled).toBe(true);
    expect(resume.checked).toBe(false);
    fireEvent.click(resume);
    expect(patch).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: i18n.t('resumeChat') })).toBeNull();
    expect(screen.queryByRole('button', { name: i18n.t('cancelResumePlans') })).toBeNull();
    expect(tauri.invoke.mock.calls.every(([command]) => command === 'get_auto_pause_status')).toBe(true);
  });

  it.each([
    ['en-US', 'Connect the DSH bridge and explicitly select the active subscription account and target sessions.'],
    ['zh-CN', '请连接 DSH 桥，并显式选择活动订阅账户及目标会话。'],
    ['zh-Hant', '請連線 DSH 橋接，並明確選擇啟用中的訂閱帳戶及目標工作階段。'],
  ])('renders the DSH binding requirement without a fallback in %s', async (locale, message) => {
    await i18n.changeLanguage(locale);
    tauri.invoke.mockResolvedValue(snapshot({ provider: 'dsh', summary: 'needsAttention', reason: 'dshBindingRequired',
      confirmedCount: 0, acceptedRequestCount: 0, confirmedInterruptionCount: 0, targets: [], resumePlans: [] }));
    render(<ProtectionStatusView settings={{ ...settings, provider: 'dsh' }} />);
    await screen.findByText(message);
    expect(screen.queryByText(i18n.t('protectionReasons.statusUnavailable'))).toBeNull();
    expect(i18n.getResource(locale, 'translation', 'protectionReasons.dshBindingRequired')).toBe(message);
  });

  it('drops old-provider status and resume actions immediately when selecting DSH', async () => {
    tauri.invoke.mockResolvedValue(snapshot({ resumePlans: [{ id: 'plan', threadId: 'local-chat', phase: 'ready', eligibleAfter: 100, reason: null }] }));
    const view = render(<ProtectionStatusView settings={settings} />);
    fireEvent.click(await screen.findByText('Protection details'));
    expect(screen.getByRole('button', { name: 'Resume chat' })).toBeTruthy();
    view.rerender(<ProtectionStatusView settings={{ ...settings, provider: 'dsh' }} />);
    expect(screen.queryByRole('button', { name: 'Resume chat' })).toBeNull();
    expect(screen.queryByText('Pause requested — stopping unconfirmed')).toBeNull();
    expect(screen.getByText(i18n.t('protectionSummaries.checking'))).toBeTruthy();
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
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('cancel_resume_plans', { provider: 'codex' }));
  });
  it('resumes a chat when clicking the resume button', async () => {
    tauri.invoke.mockImplementation(async (cmd) => {
      if (cmd === 'get_auto_pause_status') {
        return snapshot({ resumePlans: [
          { id: 'plan', threadId: 'chat-abc', phase: 'ready', eligibleAfter: 100, reason: null }
        ] });
      }
      return null;
    });
    render(<ProtectionStatusView settings={settings} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Resume chat' }));
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('resume_chat', { threadId: 'chat-abc', provider: 'codex' }));
  });
  it('displays error when resume fails because quota is still low', async () => {
    tauri.invoke.mockImplementation(async (cmd) => {
      if (cmd === 'get_auto_pause_status') {
        return snapshot({ resumePlans: [
          { id: 'plan', threadId: 'chat-abc', phase: 'waitingForQuota', eligibleAfter: 100, reason: null }
        ] });
      }
      if (cmd === 'resume_chat') {
        throw 'quotaStillLow';
      }
      return null;
    });
    render(<ProtectionStatusView settings={settings} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Resume chat' }));
    await screen.findByText('Quota is still below the pause threshold.');
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
