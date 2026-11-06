import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { SoundPicker } from './SoundPicker';
import i18n from '../i18n';

const api = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, Set<(e: {payload: unknown}) => void>>() }));
vi.mock('@tauri-apps/api/core', () => ({invoke: api.invoke}));
vi.mock('@tauri-apps/api/event', () => ({listen: async (name: string, callback: (e: {payload: unknown}) => void) => {
  if (!api.listeners.has(name)) api.listeners.set(name, new Set());
  api.listeners.get(name)!.add(callback);
  return () => api.listeners.get(name)?.delete(callback);
}}));
beforeEach(async () => { api.invoke.mockReset(); api.invoke.mockResolvedValue(null); api.listeners.clear(); await i18n.changeLanguage('en-US'); });
afterEach(cleanup);

it('does not let a stopped request failure clear its replacement preview', async () => {
  let rejectOld!: (error: Error) => void;
  const old = new Promise((_, reject) => {rejectOld = reject;});
  let calls = 0;
  api.invoke.mockImplementation(async (command: string) => command === 'preview_sound' && ++calls === 1 ? old : null);
  const view = render(<SoundPicker provider="claude" kind="week" path="C:/sound.wav" onChangePath={() => {}} />);
  await act(async () => { fireEvent.click(view.getByRole('button', {name:'Preview'})); });
  await act(async () => { fireEvent.click(view.getByRole('button', {name:'Stop preview'})); });
  await act(async () => { fireEvent.click(view.getByRole('button', {name:'Preview'})); });
  await act(async () => { rejectOld(new Error('old request failed')); });
  expect(view.getByRole('button', {name:'Stop preview'})).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
});

it('routes preview completion to its own provider component and playback ID', async () => {
  const view = render(<><div data-testid="claude"><SoundPicker provider="claude" kind="week" path="C:/one.wav" onChangePath={() => {}} /></div>
    <div data-testid="dsh"><SoundPicker provider="dsh" kind="week" path="C:/two.wav" onChangePath={() => {}} /></div></>);
  const claude = within(view.getByTestId('claude')), dsh = within(view.getByTestId('dsh'));
  await act(async () => { fireEvent.click(claude.getByRole('button', {name:'Preview'})); });
  await act(async () => { fireEvent.click(dsh.getByRole('button', {name:'Preview'})); });
  const id = api.invoke.mock.calls.find(([,args]) => args?.provider === 'claude')![1].playbackId;
  await act(async () => { for (const callback of api.listeners.get('sound_preview_finished')!) callback({payload:{provider:'claude',playbackId:id}}); });
  expect(claude.getByRole('button', {name:'Preview'})).toBeTruthy();
  expect(dsh.getByRole('button', {name:'Stop preview'})).toBeTruthy();
});
