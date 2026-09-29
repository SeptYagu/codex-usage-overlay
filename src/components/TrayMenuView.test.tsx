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
  // jsdom reports a zero scrollWidth; the popup width is derived from it, so give
  // it a non-zero value (280 + 20 padding = 300 logical px).
  vi.spyOn(Element.prototype, 'scrollWidth', 'get').mockReturnValue(280);
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
    widthLogical: 300,
  }));
  fireEvent.click(screen.getByRole('menuitem', { name: /Language \/ 语言/ }));
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('layout_tray_menu', {
    generation: 3,
    revision: 2,
    heightLogical: 240,
    widthLogical: 300,
  }));
  fireEvent.click(screen.getByRole('menuitem', { name: /Language \/ 语言/ }));
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('layout_tray_menu', {
    generation: 3,
    revision: 3,
    heightLogical: 120,
    widthLogical: 300,
  }));
});

it('shows the client version in the check-for-updates menu item', async () => {
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  expect(screen.getByRole('menuitem', { name: 'Check for Updates (v1.1.3)' })).toBeTruthy();
  cleanup();
  await i18n.changeLanguage('zh-CN');
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  expect(screen.getByRole('menuitem', { name: '检查更新 (当前版本: v1.1.3)' })).toBeTruthy();
});

it('exposes auto edge hide as a synchronized checkbox menu item', () => {
  const onPatchSettings = vi.fn();
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={onPatchSettings} />);
  const item = screen.getByRole('menuitemcheckbox', { name: 'Auto hide at edge' });
  expect(item.getAttribute('aria-checked')).toBe('false');
  fireEvent.click(item);
  expect(onPatchSettings).toHaveBeenCalledWith({ autoEdgeHide: true });
});

it('keeps the click-through recovery switch in the tray menu', () => {
  const onPatchSettings = vi.fn();
  render(<TrayMenuView settings={{ ...DEFAULT_SETTINGS, mousePassthrough: true }} onPatchSettings={onPatchSettings} />);
  const item = screen.getByRole('menuitemcheckbox', { name: 'Mouse click-through' });
  expect(item.getAttribute('aria-checked')).toBe('true');
  fireEvent.click(item);
  expect(onPatchSettings).toHaveBeenCalledWith({ mousePassthrough: false });
});

it('uses the status line as the install action and allows retry after a failed install', async () => {
  const update = { version: '1.2.0', currentVersion: '1.1.1', notes: null };
  let installAttempts = 0;
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_tray_menu_generation') return 3;
    if (command === 'layout_tray_menu') return 300;
    if (command === 'get_available_update') return update;
    if (command === 'install_update' && ++installAttempts === 1) throw new Error('installer unavailable');
    return null;
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  fireEvent.click(await screen.findByRole('menuitem', { name: 'Version 1.2.0 available — click to install' }));
  await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Retry installation' })).toBeTruthy());
  fireEvent.click(screen.getByRole('menuitem', { name: 'Retry installation' }));
  await waitFor(() => expect(installAttempts).toBe(2));
});

it('shows a prepared update in the status line with an immediate install action', async () => {
  const update = { version: '1.2.0', currentVersion: '1.1.1', notes: null };
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_tray_menu_generation') return 3;
    if (command === 'layout_tray_menu') return 300;
    if (command === 'get_available_update') return update;
    if (command === 'get_update_ready') return true;
    return null;
  });
  render(<TrayMenuView settings={{ ...DEFAULT_SETTINGS, autoInstallUpdates: true }} onPatchSettings={() => {}} />);
  fireEvent.click(await screen.findByRole('menuitem', { name: 'Version 1.2.0 ready — install now' }));
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('install_update'));
});
