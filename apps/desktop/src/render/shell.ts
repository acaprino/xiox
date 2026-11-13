// The frame of the window: top bar, status row, the two sidebars and the list header, the footer.
// Everything here comes from the SectionView (Rust writes the sentences); the page adds only
// the static headings, key hints and number formatting.

import { button, fill, h, kbd } from '../dom.ts';
import { formatBytes, ramText } from '../format.ts';
import { SECTION_ORDER } from '../state.ts';
import type { ActionView, AppInfo, NavItem, OptionView, SectionId, SectionView } from '../types.ts';

const SECTION_NAMES: Record<SectionId, string> = {
  cleanup: 'Cleanup', registry: 'Registry', services: 'Services & RAM', developers: 'Developers',
};

const KEY_LABELS: Record<string, string> = { Enter: '↵', Escape: 'Esc' };

const FOOTER_HINT = '↑↓ move · space toggle · a analyze · ↵ clean';

function byId<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`#${id} is missing from index.html`);
  return node as T;
}

/** Free text that may be long: one line, cut with an ellipsis, the full text in the tooltip. */
function ellipsis(text: string, tag: 'span' | 'div' = 'span'): HTMLElement {
  return h(tag, { class: 'ellipsis', title: text }, text);
}

function actionButton(action: ActionView): HTMLButtonElement {
  const node = button(
    [h('span', { 'data-fit': '' }, action.label), action.key ? kbd(KEY_LABELS[action.key] ?? action.key) : null],
    { class: action.primary ? 'btn primary' : 'btn', 'data-action': action.id, disabled: !action.enabled },
  );
  return node;
}

function navRow(item: NavItem): HTMLElement {
  return h('button', { type: 'button', class: item.active ? 'it on' : 'it', 'data-nav': item.id },
    h('span', { class: 'it-name', 'data-fit': '' }, item.name),
    item.bytes === null ? null : h('b', { class: 'num', 'data-fit': '' }, formatBytes(item.bytes)),
    item.note ? h('small', { 'data-fit': '' }, item.note) : null);
}

function optionRow(option: OptionView): HTMLElement {
  const label = option.kind === 'text' ? option.text : option.label;
  return h('div', { class: 'it opt' }, h('span', { class: 'it-name' }, ellipsis(label)));
}

export function renderShell(app: AppInfo, view: SectionView): void {
  // Top bar.
  byId('version').textContent = app.version;
  fill(byId('actions'), ...view.actions.map(actionButton));

  // Status row.
  byId('badge').textContent = view.status.badge;
  fill(byId('status-text'), ellipsis(view.status.text));
  byId('status-right').textContent = `${app.admin ? 'administrator' : 'standard user'} · ${ramText(app.ramTotalBytes)}`;
  const profile = byId('profile');
  profile.hidden = app.profileMismatch === null;
  if (app.profileMismatch !== null) fill(profile, ellipsis(app.profileMismatch));

  // Sections bar.
  fill(byId('sections'),
    h('div', { class: 'hd' }, h('span', { class: 'hd-title', 'data-fit': '' }, 'Sections'), h('span', { class: 'count' }, SECTION_ORDER.length)),
    ...SECTION_ORDER.map((id, index) => h('button', { type: 'button', class: id === view.section ? 'nv on' : 'nv', 'data-section': id },
      h('span', { 'data-fit': '' }, SECTION_NAMES[id]), kbd(String(index + 1)))));

  // Bar 2: the section's categories, or its options.
  const categories = view.nav.length > 0;
  fill(byId('bar2'),
    h('div', { class: 'hd' },
      h('span', { class: 'hd-title', 'data-fit': '' }, categories ? 'Categories' : 'Options'),
      h('span', { class: 'count' }, categories ? view.nav.length : view.options.length)),
    h('div', { class: 'items' }, ...(categories ? view.nav.map(navRow) : view.options.map(optionRow))));

  // List header.
  fill(byId('list-head'),
    h('span', { class: 'hd-title', 'data-fit': '' }, view.title),
    h('span', { class: 'sub' }, ellipsis(view.subtitle)));

  // Footer.
  fill(byId('foot'),
    h('span', { class: 'dot' }),
    h('span', { class: 'foot-left' }, ellipsis(view.footer ?? '')),
    h('span', { class: 'foot-right', 'data-fit': '' }, FOOTER_HINT));
}
