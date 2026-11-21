// The Settings window: the look of the window (Smooth, or ONI MODE, the look of the approved mock-up), then
// the name, the version, the Winapp2 rules in use and where they come from (the CC-BY-SA attribution
// Winapp2 asks for). A small modal over the dimmed page, like the confirmation (render/dialog.ts) but
// not a section's own dialog: it is opened by the page (the Settings button or `?`) and holds no
// intent; the look is the page's (main.ts keeps it). Escape or Close closes it and the focus goes back
// to where it was.

import { button, fill, h } from '../dom.ts';
import { cycleFocus } from '../keys.ts';
import type { Look } from '../look.ts';
import type { AboutView } from '../types.ts';

/** The look on screen, and what to do when another one is chosen. */
export interface LookChoice { skin: Look; choose(skin: Look): void }

const LOOKS: [Look, string][] = [['smooth', 'Smooth'], ['oni', 'ONI mode (the original look)']];

const BACKGROUND = ['top', 'status', 'profile', 'job', 'body', 'foot'];
/** The `data-key` of the Settings button of the sections bar. */
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

/** Tab never leaves the window: it goes from the chosen look to Close and back (the arrows move between the looks). */
function wire(): void {
  if (wired) return;
  wired = true;
  host().addEventListener('keydown', (event) => {
    if (event.key !== 'Tab' || event.ctrlKey || event.altKey || event.metaKey) return;
    const list = [...host().querySelectorAll<HTMLElement>('input[type="radio"]:checked, button:not([disabled])')];
    const to = cycleFocus(list.length, list.indexOf(document.activeElement as HTMLElement), event.shiftKey);
    event.preventDefault();
    if (to !== null) list[to]?.focus();
  });
}

/** Opens the window over the page. `from`: the `data-key` of the control that had the focus, where Close gives it back. */
export function showAbout(about: AboutView, from: string | null, look: LookChoice, onClose: () => void): void {
  if (shown) return;
  wire();
  opener = from;
  shown = true;
  const root = host();
  const choice = ([skin, label]: [Look, string]): HTMLElement => h('label', { class: 'radio' },
    h('input', { type: 'radio', name: 'look', value: skin, checked: skin === look.skin, 'data-key': `look:${skin}`, onChange: () => look.choose(skin) }),
    h('span', {}, label));
  fill(root, h('div', { class: 'scrim' },
    h('div', { class: 'dlg about frame', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'about-title', tabindex: -1 },
      h('h2', { id: 'about-title', class: 'dlg-title' }, 'Settings'),
      h('div', { class: 'dlg-body' },
        h('div', { class: 'ctl ctl-choice', role: 'radiogroup', 'aria-label': 'Look' },
          h('span', { class: 'ctl-title' }, 'Look'), h('div', { class: 'radios' }, ...LOOKS.map(choice))),
        h('b', { class: 'dlg-sub' }, 'About Xiox'),
        h('p', { class: 'dlg-text' }, `Version ${about.version}`),
        h('p', { class: 'dlg-text' }, about.winapp2),
        h('p', { class: 'dlg-text' }, about.attribution)),
      h('div', { class: 'dlg-buttons' },
        button(h('span', { 'data-fit': '' }, 'Close'), { class: 'btn primary', 'data-key': CLOSE_KEY, onClick: onClose })))));
  root.hidden = false;
  setInert(true);
  root.querySelector<HTMLElement>(`[data-key="${CLOSE_KEY}"]`)?.focus();
}

/** Closes the window and puts the focus back: on the control that had it, else on the Settings button. */
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
