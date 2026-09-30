import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import i18n from '../i18n';
import { DEFAULT_SETTINGS } from '../types';
import { TrayMenuView, trayMenuWidthForText } from './TrayMenuView';

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

// jsdom has no layout engine: neither `getBoundingClientRect` nor `scrollWidth`
// report anything real. `injectedTextWidth` stands in for the *net* text width of
// the longest menu label, which is what the popup width contract consumes. The
// container-bound `scrollWidth` is deliberately NOT part of the measurement — the
// spec forbids falling back to it — so the tests mock `Range.getBoundingClientRect`
// (the net-text probe) instead.
let injectedTextWidth = 260;
let originalRangeRect: PropertyDescriptor | undefined;

beforeEach(async () => {
  await i18n.changeLanguage('en-US');
  tauri.invoke.mockReset();
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_tray_menu_generation') return 3;
    if (command === 'layout_tray_menu') return 300;
    return null;
  });
  injectedTextWidth = 260;
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const radios = this.querySelectorAll('[role="menuitemradio"]').length;
    return { height: 100 + radios * 30 } as DOMRect;
  });
  // jsdom implements `Range` but not its layout methods, so the net-text probe has
  // no `getBoundingClientRect`. This stand-in reports `injectedTextWidth` for every
  // measured text node — the deterministic substitute for a real layout engine.
  originalRangeRect = Object.getOwnPropertyDescriptor(Range.prototype, 'getBoundingClientRect');
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', {
    configurable: true,
    writable: true,
    value: () => ({ width: injectedTextWidth } as DOMRect),
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  if (originalRangeRect) {
    Object.defineProperty(Range.prototype, 'getBoundingClientRect', originalRangeRect);
  } else {
    Reflect.deleteProperty(Range.prototype, 'getBoundingClientRect');
  }
});

/** The `widthLogical` of the nth `layout_tray_menu` invocation. */
const laidOutWidth = (call: number) => {
  const layouts = tauri.invoke.mock.calls.filter(([command]) => command === 'layout_tray_menu');
  expect(layouts.length).toBeGreaterThan(call - 1);
  return (layouts[call - 1][1] as { widthLogical: number }).widthLogical;
};

it('uses the unique clamp(280, 500, ceil(textW) + 40) width formula', () => {
  // Precise numbers chosen so a `+20` padding bug cannot pass: 260 + 20 = 280 and
  // 340 + 20 = 360, both of which differ from the values asserted below.
  expect(trayMenuWidthForText(220)).toBe(280);
  expect(trayMenuWidthForText(260)).toBe(300);
  expect(trayMenuWidthForText(300)).toBe(340);
  expect(trayMenuWidthForText(340)).toBe(380);
  expect(trayMenuWidthForText(460)).toBe(500);
  expect(trayMenuWidthForText(600)).toBe(500);
  // Fractional text widths round up, never down, so the label always fits.
  expect(trayMenuWidthForText(260.4)).toBe(301);
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

it('widens the tray popup to the measured net text width', async () => {
  // 340 + 40 = 380 logical px. A `+20` implementation would report 360 instead.
  injectedTextWidth = 340;
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  await waitFor(() => expect(laidOutWidth(1)).toBe(380));
  expect(tauri.invoke).toHaveBeenCalledWith('layout_tray_menu', {
    generation: 3,
    revision: 1,
    heightLogical: 120,
    widthLogical: 380,
  });
});

it('clamps the tray popup width to the configured maximum', async () => {
  // 600 + 40 exceeds the 500px cap, so the reported width must be clamped.
  injectedTextWidth = 600;
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  await waitFor(() => expect(laidOutWidth(1)).toBe(500));
});

it('keeps the tray popup at the 280px floor for narrow content', async () => {
  // 220 + 40 = 260 is below the floor, so the reported width is raised to it.
  injectedTextWidth = 220;
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  await waitFor(() => expect(laidOutWidth(1)).toBe(280));
});

it('shrinks the popup back to the floor when the longest label gets shorter', async () => {
  // A long label first widens the popup...
  injectedTextWidth = 340;
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  await waitFor(() => expect(laidOutWidth(1)).toBe(380));

  // ...then a shorter one must contract it again instead of latching at the widest
  // value ever seen (the non-contractive `scrollWidth` fixed point).
  injectedTextWidth = 196;
  fireEvent.click(screen.getByRole('menuitem', { name: /Language \/ 语言/ }));
  await waitFor(() => expect(laidOutWidth(2)).toBe(280));

  injectedTextWidth = 220;
  fireEvent.click(screen.getByRole('menuitem', { name: /Language \/ 语言/ }));
  await waitFor(() => expect(laidOutWidth(3)).toBe(280));
});

it('never sizes the popup from the container-bound scrollWidth', async () => {
  // The deprecated v1.1.3 metric: the items are `width: 100%`, so their scrollWidth
  // is bounded below by the popup's own clientWidth. Feeding it into the width is
  // what made the popup only ever grow. A huge scrollWidth must change nothing.
  injectedTextWidth = 220;
  vi.spyOn(Element.prototype, 'scrollWidth', 'get').mockReturnValue(900);
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  await waitFor(() => expect(laidOutWidth(1)).toBe(280));
});

it('shows the client version in the check-for-updates menu item', async () => {
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  expect(screen.getByRole('menuitem', { name: 'Check for Updates (v1.2.1)' })).toBeTruthy();
  cleanup();
  await i18n.changeLanguage('zh-CN');
  render(<TrayMenuView settings={DEFAULT_SETTINGS} onPatchSettings={() => {}} />);
  expect(screen.getByRole('menuitem', { name: '检查更新 (当前版本: v1.2.1)' })).toBeTruthy();
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
  const update = { version: '1.2.1', currentVersion: '1.2.0', notes: null };
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
  fireEvent.click(await screen.findByRole('menuitem', { name: 'Version 1.2.1 available — click to install' }));
  await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Retry installation' })).toBeTruthy());
  fireEvent.click(screen.getByRole('menuitem', { name: 'Retry installation' }));
  await waitFor(() => expect(installAttempts).toBe(2));
});

it('shows a prepared update in the status line with an immediate install action', async () => {
  const update = { version: '1.2.1', currentVersion: '1.2.0', notes: null };
  tauri.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_tray_menu_generation') return 3;
    if (command === 'layout_tray_menu') return 300;
    if (command === 'get_available_update') return update;
    if (command === 'get_update_ready') return true;
    return null;
  });
  render(<TrayMenuView settings={{ ...DEFAULT_SETTINGS, autoInstallUpdates: true }} onPatchSettings={() => {}} />);
  fireEvent.click(await screen.findByRole('menuitem', { name: 'Version 1.2.1 ready — install now' }));
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith('install_update'));
});
