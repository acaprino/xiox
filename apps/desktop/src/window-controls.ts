// The title bar of the Smooth look is the page's (the window has no system frame then): minimize, maximize or restore,
// and close, plus where the maximize button is, so that Windows 11 offers its snap layouts over it (src-tauri
// native_frame.rs, ported from Shinatama). ONI MODE has the system frame: its buttons are hidden, their place reported
// empty. Outside Tauri (the browser at `npm run dev`) the buttons are drawn and do nothing.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { inTauri } from './api.ts';

export async function initializeWindowControls(reportError: (error: unknown) => void): Promise<void> {
  const minimize = document.querySelector<HTMLButtonElement>('#window-minimize');
  const maximize = document.querySelector<HTMLButtonElement>('#window-maximize');
  const close = document.querySelector<HTMLButtonElement>('#window-close');
  if (!minimize || !maximize || !close || !inTauri) return;
  const appWindow = getCurrentWindow();
  let revision = 0;

  async function syncMaximized(): Promise<void> {
    const request = ++revision;
    const maximized = await appWindow.isMaximized();
    if (request !== revision || !maximize) return;
    maximize.setAttribute('aria-label', maximized ? 'Restore window' : 'Maximize window');
    maximize.title = maximized ? 'Restore' : 'Maximize';
    maximize.querySelector('.maximize-icon')?.toggleAttribute('hidden', maximized);
    maximize.querySelector('.restore-icon')?.toggleAttribute('hidden', !maximized);
  }

  minimize.onclick = () => { void appWindow.minimize().catch(reportError); };
  // The mouse reaches the native overlay over this button, which maximizes on click; this handler serves the keyboard.
  maximize.onclick = () => { void appWindow.toggleMaximize().then(syncMaximized).catch(reportError); };
  // The close request goes through the window's own handler (it saves the window's place).
  close.onclick = () => { void appWindow.close().catch(reportError); };
  await listen<{ hover: boolean; pressed: boolean }>('caption-maximize', ({ payload }) => {
    maximize.classList.toggle('os-hover', payload.hover);
    maximize.classList.toggle('os-pressed', payload.pressed);
  });

  // Where the button is, in physical pixels; hidden (ONI MODE) it measures nothing and the overlay goes away.
  let reported = '';
  let frame = 0;
  const report = (): void => {
    frame = 0;
    const box = maximize.getBoundingClientRect();
    const ratio = window.devicePixelRatio;
    const rect = { x: Math.round(box.left * ratio), y: Math.round(box.top * ratio), width: Math.round(box.width * ratio), height: Math.round(box.height * ratio) };
    const key = JSON.stringify(rect);
    if (key === reported) return;
    reported = key;
    void invoke('window_caption_maximize', rect).catch(() => { reported = ''; });
  };
  const schedule = (): void => { if (!frame) frame = requestAnimationFrame(report); };
  new ResizeObserver(schedule).observe(maximize);
  // The look switches the bar on and off without resizing the window.
  new MutationObserver(schedule).observe(document.documentElement, { attributes: true, attributeFilter: ['data-skin'] });
  window.addEventListener('resize', schedule);
  const watchRatio = (): void => matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`).addEventListener('change', () => { schedule(); watchRatio(); }, { once: true });
  watchRatio();
  // Maximized, the overlay also covers the resize border: placed again on every resize.
  await appWindow.onResized(() => { reported = ''; schedule(); void syncMaximized().catch(reportError); });
  schedule();
  await syncMaximized();
}
