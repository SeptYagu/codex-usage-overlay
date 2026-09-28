import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import i18n from '../i18n';
import { DEFAULT_SETTINGS } from '../types';
import { TrayMenuView } from './TrayMenuView';

const tauri = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    scaleFactor: async () => 1,
    onScaleChanged: async () => () => {},
    hide: async () => {},
  }),
}));

beforeEach(async () => {
  await i18n.changeLanguage('en-US');
  tauri.invoke.mockReset();
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_tray_menu_generation') return 3;
    if (command === 'layout_tray_menu') return 300;
    return null;
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const radios = this.querySelectorAll('[role="menuitemradio"]').length;
    return { height: 100 + radios * 30 } as DOMRect;
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it('remeasures tray content when the language list opens and closes', async () => {
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('layout_tray_menu', {
    generation: 3,
    revision: 1,
    heightLogical: 120,
  }));
  fireEvent.click(screen.getByRole('menuitem', { name: /Language \/ 语言/ }));
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('layout_tray_menu', {
    generation: 3,
    revision: 2,
    heightLogical: 240,
  }));
  fireEvent.click(screen.getByRole('menuitem', { name: /Language \/ 语言/ }));
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('layout_tray_menu', {
    generation: 3,
    revision: 3,
    heightLogical: 120,
  }));
});

it('exposes auto edge hide as a synchronized checkbox menu item', () => {
  const onPatchSettings = vi.fn();
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={onPatchSettings} />);
  const item = screen.getByRole('menuitemcheckbox', { name: 'Auto hide at edge' });
  expect(item.getAttribute('aria-checked')).toBe('false');
  fireEvent.click(item);
  expect(onPatchSettings).toHaveBeenCalledWith({ autoEdgeHide: true });
});
