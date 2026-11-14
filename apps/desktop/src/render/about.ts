// The About window: the name, the version, the Winapp2 rules in use and where they come from (the
// CC-BY-SA attribution Winapp2 asks for). A small modal over the dimmed page, like the confirmation
// (render/dialog.ts) but not a section's own dialog: it is opened by the page (the About button or
// `?`) and holds no intent. Escape or Close closes it and the focus goes back to where it was.

import { button, fill, h } from '../dom.ts';
import { cycleFocus } from '../keys.ts';
import type { AboutView } from '../types.ts';

const BACKGROUND = ['top', 'status', 'profile', 'job', 'body', 'foot'];
/** The `data-key` of the About button of the sections bar. */
export const ABOUT_KEY = 'about';
const CLOSE_KEY = 'about:close';

let opener: string | null = null;
let shown = false;
let wired = false;

function host(): HTMLElement {
  const node = document.getElementById('about');
  if (!node) throw new Error('#about is missing from index.html');
  return node;
}

function setInert(on: boolean): void {
  for (const id of BACKGROUND) document.getElementById(id)?.toggleAttribute('inert', on);
}

export const aboutOpen = (): boolean => shown;

/** Tab never leaves the window: it has one control, so Tab and Shift+Tab stay on Close. */
function wire(): void {
  if (wired) return;
  wired = true;
  host().addEventListener('keydown', (event) => {
    if (event.key !== 'Tab' || event.ctrlKey || event.altKey || event.metaKey) return;
    const list = [...host().querySelectorAll<HTMLElement>('button:not([disabled])')];
    const to = cycleFocus(list.length, list.indexOf(document.activeElement as HTMLElement), event.shiftKey);
    event.preventDefault();
    if (to !== null) list[to]?.focus();
  });
}

/** Opens the window over the page. `from`: the `data-key` of the control that had the focus, where Close gives it back. */
export function showAbout(about: AboutView, from: string | null, onClose: () => void): void {
  if (shown) return;
  wire();
  opener = from;
  shown = true;
  const root = host();
  fill(root, h('div', { class: 'scrim' },
    h('div', { class: 'dlg about frame', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'about-title', tabindex: -1 },
      h('h2', { id: 'about-title', class: 'dlg-title' }, 'Xiox'),
      h('div', { class: 'dlg-body' },
        h('p', { class: 'dlg-text' }, `Version ${about.version}`),
        h('p', { class: 'dlg-text' }, about.winapp2),
        h('p', { class: 'dlg-text' }, about.attribution)),
      h('div', { class: 'dlg-buttons' },
        button(h('span', { 'data-fit': '' }, 'Close'), { class: 'btn primary', 'data-key': CLOSE_KEY, onClick: onClose })))));
  root.hidden = false;
  setInert(true);
  root.querySelector<HTMLElement>(`[data-key="${CLOSE_KEY}"]`)?.focus();
}

/** Closes the window and puts the focus back: on the control that had it, else on the About button. */
export function closeAbout(): void {
  if (!shown) return;
  shown = false;
  const root = host();
  setInert(false);
  root.hidden = true;
  fill(root);
  const find = (key: string): HTMLElement | null => document.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`);
  const back = opener === null ? null : find(opener);
  const target = back !== null && !(back as HTMLButtonElement).disabled ? back : find(ABOUT_KEY);
  opener = null;
  target?.focus();
}
