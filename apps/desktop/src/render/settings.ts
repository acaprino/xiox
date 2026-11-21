// Settings is a view, the fourth tab (opened also with `4` and `?`): the look (Smooth or ONI), the theme (Windows, light or dark) and
// About Xiox (the version, the Winapp2 rules and where they come from, the attribution Winapp2 asks for). A setting is not a decision
// about the window, so it is not a dialog. The sections views stay as they are: this draws over their sidebar, header, content and
// action bar while it is open (`ui.settings`), and the window title keeps the last section. Its sidebar rows, "Look" and "About",
// scroll to their section.

import { fill, h } from '../dom.ts';
import type { Look, ThemeChoice } from '../look.ts';
import type { AboutView } from '../types.ts';
import { initials } from './sidebar.ts';

/** What the view changes: the page applies it, keeps it and draws the window again. */
export interface Appearance { look: Look; theme: ThemeChoice }

const LOOKS: [Look, string][] = [['smooth', 'Smooth'], ['oni', 'ONI mode']];
const THEMES: [ThemeChoice, string, string][] = [['system', 'Windows', 'Follow the Windows theme'], ['light', 'Light', 'Always light'], ['dark', 'Dark', 'Always dark']];

/** One choice of a segmented control: a native radio, so the arrows and the form behave as the browser's. */
function choice<T extends string>(name: string, value: T, label: string, checked: boolean, title: string | null, choose: (value: T) => void): HTMLElement {
  return h('label', { title }, h('input', { type: 'radio', name, value, checked, 'data-key': `${name}:${value}`, onChange: () => choose(value) }), h('span', {}, label));
}

/** A sidebar row of Settings: it scrolls the page to its section. */
function sectionRow(title: string, target: string, sub: string): HTMLElement {
  return h('button', {
    type: 'button', class: 'im-row xi-row', role: 'option', 'aria-selected': 'false', 'data-key': `settings:${target}`, title: `Go to ${title}`,
    onClick: () => document.getElementById(target)?.scrollIntoView({ block: 'start', behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' }),
  }, h('span', { class: 'im-avatar im-rail-item', 'aria-hidden': 'true' }, initials(title)),
  h('span', { class: 'im-row-text' }, h('span', { class: 'im-row-title' }, title), h('span', { class: 'im-row-sub' }, sub)));
}

/** Draws Settings into the regions of the shell. `about` is what Rust says of the app (read when the view opens). */
export function renderSettings(about: AboutView, current: Appearance, set: (next: Appearance) => void): void {
  const byId = (id: string): HTMLElement => {
    const node = document.getElementById(id);
    if (!node) throw new Error(`#${id} is missing from the frame`);
    return node;
  };
  // The sidebar.
  const side = byId('sidebar');
  const heading = side.querySelector('.im-panel-title');
  if (heading) heading.textContent = 'Settings';
  const count = side.querySelector('.im-count');
  if (count) count.textContent = '2';
  side.setAttribute('aria-label', 'Settings');
  const list = byId('sidebar-list');
  list.setAttribute('aria-label', 'Settings');
  fill(list, sectionRow('Look', 'settings-look', 'Look and theme'), sectionRow('About', 'settings-about', `Version ${about.version}`));
  // The content's header and the action bar: Settings has no action of its own.
  fill(byId('list-head'), h('h4', { class: 'im-panel-title' }, 'Settings'), h('span', { class: 'im-spacer' }));
  fill(byId('actions'));
  // The page.
  const rows = byId('rows');
  rows.classList.remove('is-report');
  fill(rows, h('div', { class: 'im-panel-body xi-settings', 'data-key': 'settings:page' },
    h('div', { class: 'im-page-content im-page-content--narrow' },
      h('h2', { class: 'im-section-heading', id: 'settings-look' }, 'Look'),
      h('div', { class: 'im-field' }, h('span', { class: 'im-field-label' }, 'Look'),
        h('div', { class: 'im-seg', role: 'radiogroup', 'aria-label': 'Look' }, ...LOOKS.map(([value, label]) => choice('look', value, label, value === current.look, null, (look) => set({ ...current, look }))))),
      h('div', { class: 'im-field' }, h('span', { class: 'im-field-label' }, 'Theme'),
        h('div', { class: 'im-seg', role: 'radiogroup', 'aria-label': 'Theme' },
          ...THEMES.map(([value, label, hint]) => h('label', { title: hint }, h('input', {
            type: 'radio', name: 'theme', value, checked: value === current.theme, 'data-theme-choice': value, 'data-key': `theme:${value}`, onChange: () => set({ ...current, theme: value }),
          }), h('span', {}, label))))),
      h('h2', { class: 'im-section-heading', id: 'settings-about' }, 'About Xiox'),
      h('p', {}, `Version ${about.version}`),
      h('p', {}, about.winapp2),
      h('p', {}, about.attribution))));
}
