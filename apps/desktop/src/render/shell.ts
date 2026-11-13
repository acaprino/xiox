// The frame of the window: top bar, status row, the two sidebars, the list header, the footer.
// Everything here comes from the SectionView (Rust writes the sentences); the page adds only
// the static headings, key hints and number formatting. The list, the detail panel and bar 2
// are drawn by their own modules.

import { button, fill, h, kbd } from '../dom.ts';
import { ramText } from '../format.ts';
import { actionIntent, bulkIntent } from '../intents.ts';
import { sortRows, visibleRows } from '../rows.ts';
import { currentFocus, SECTION_ORDER, type UiState } from '../state.ts';
import type { ActionView, AnyIntent, AppInfo, NoticeView, SectionId, SectionView } from '../types.ts';
import { renderBar2 } from './bar2.ts';
import { renderDetail } from './detail.ts';
import { renderList } from './list.ts';

/** What a click or a keystroke can ask of the page; main.ts implements it. */
export interface Handlers {
  openSection(id: SectionId): void;
  /** Sends an intent to a section's command; a null intent (an id the section does not send) is ignored. */
  send(section: SectionId, intent: AnyIntent | null): void;
  focusRow(id: string): void;
  toggleExpanded(id: string): void;
  /** The folder picker of Developers: it opens in Rust and the page never sees a path. */
  addRoot(): void;
}

/** What one render needs. */
export interface Screen {
  app: AppInfo;
  view: SectionView;
  ui: UiState;
  /** The message of the last failed command, shown above the list until the next one succeeds. */
  error: string | null;
}

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

function actionButton(section: SectionId, action: ActionView, on: Handlers): HTMLButtonElement {
  return button(
    [h('span', { 'data-fit': '' }, action.label), action.key ? kbd(KEY_LABELS[action.key] ?? action.key) : null],
    {
      class: action.primary ? 'btn primary' : 'btn', 'data-action': action.id, 'data-key': `action:${action.id}`, disabled: !action.enabled,
      onClick: () => on.send(section, actionIntent(section, action.id)),
    },
  );
}

/** A notice line: its text, and the extra lines under it. Errors are announced. */
export function noticeNode(notice: NoticeView): HTMLElement {
  return h('div', { class: `notice tone-${notice.tone}`, role: notice.tone === 'error' ? 'alert' : null },
    h('span', {}, notice.text),
    ...notice.more.map((line) => h('small', {}, line)));
}

/** The window when nothing can be drawn (the first command failed): the message, in the notice area. */
export function renderFailure(message: string): void {
  byId('badge').textContent = 'Error';
  fill(byId('status-text'), ellipsis(message));
  fill(byId('notices'), noticeNode({ tone: 'error', text: message, more: [] }));
}

export function renderShell(screen: Screen, on: Handlers): void {
  const { app, view, ui } = screen;

  // Top bar.
  byId('version').textContent = app.version;
  fill(byId('actions'), ...view.actions.map((a) => actionButton(view.section, a, on)));

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
    ...SECTION_ORDER.map((id, index) => h('button', {
      type: 'button', class: id === view.section ? 'nv on' : 'nv', 'data-section': id, 'data-key': `section:${id}`,
      'aria-current': id === view.section ? 'page' : null, onClick: () => on.openSection(id),
    }, h('span', { 'data-fit': '' }, SECTION_NAMES[id]), kbd(String(index + 1)))));

  // Bar 2: categories and options.
  renderBar2(byId('bar2'), view, on);

  // List header: title, subtitle and, where the section has bulk selection, All / None.
  const bulk = view.bulk && bulkIntent(view.section, true) !== null;
  fill(byId('list-head'),
    h('span', { class: 'hd-title', 'data-fit': '' }, view.title),
    h('span', { class: 'hd-right' },
      h('span', { class: 'sub' }, ellipsis(view.subtitle)),
      bulk ? button(h('span', { 'data-fit': '' }, 'All'), { class: 'btn sm', 'data-key': 'bulk:all', onClick: () => on.send(view.section, bulkIntent(view.section, true)) }) : null,
      bulk ? button(h('span', { 'data-fit': '' }, 'None'), { class: 'btn sm', 'data-key': 'bulk:none', onClick: () => on.send(view.section, bulkIntent(view.section, false)) }) : null));

  // Notices, then the list and the detail of the focused row.
  fill(byId('notices'),
    ...(screen.error === null ? [] : [noticeNode({ tone: 'error', text: screen.error, more: [] })]),
    ...view.notices.map(noticeNode));
  renderList(byId('rows'), view, ui, on);
  const focus = currentFocus(ui, view);
  renderDetail(byId('detail'), visibleRows(sortRows(view.rows), ui.expanded).find((r) => r.id === focus) ?? null);

  // Footer.
  fill(byId('foot'),
    h('span', { class: 'dot' }),
    h('span', { class: 'foot-left' }, ellipsis(view.footer ?? '')),
    h('span', { class: 'foot-right', 'data-fit': '' }, FOOTER_HINT));
}
