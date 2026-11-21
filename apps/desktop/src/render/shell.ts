// The frame of the window: title bar, status row, the two sidebars, the list header, the footer.
// Everything here comes from the SectionView (Rust writes the sentences); the page adds only
// the static headings, key hints and number formatting. The list, the detail panel and bar 2
// are drawn by their own modules.

import { noteHtml } from '@acaprino/imago/ui';
import { button, fill, fromHtml, h, keyCaps } from '../dom.ts';
import { ramText } from '../format.ts';
import { actionIntent } from '../intents.ts';
import { footerHint } from '../keys.ts';
import { disabledReason } from './policy.ts';
import { currentFocus, noticeKey, SECTION_ORDER, shownRows, type UiState } from '../state.ts';
import type { ActionView, AnyIntent, AppInfo, NoticeView, SectionId, SectionView } from '../types.ts';
import { ABOUT_KEY } from './about.ts';
import { renderSidebar } from './sidebar.ts';
import { renderDetail } from './detail.ts';
import { renderDialog } from './dialog.ts';
import { renderList } from './list.ts';
import { renderReport } from './report.ts';

/** What a click or a keystroke can ask of the page; main.ts implements it. */
export interface Handlers {
  openSection(id: SectionId): void;
  /** The Settings window, with the look and what About said (the button after the tabs; `?` opens it too). */
  openAbout(): void;
  /** Sends an intent to a section's command; a null intent (an id the section does not send) is ignored. */
  send(section: SectionId, intent: AnyIntent | null): void;
  focusRow(id: string): void;
  /** A click on a check cell: the row takes the focus and its tick is asked for (a group header's id ticks the whole category). */
  toggleRow(id: string, section: SectionId): void;
  toggleExpanded(id: string): void;
  /** Folds or opens a category of the list; `foldAll` does it for every category. */
  toggleFold(id: string): void;
  foldAll(fold: boolean): void;
  /** A click on a column's header: the list is ordered by that column, or the order turns round. */
  sortBy(column: string): void;
  /** Opens or closes the details of the focused row. */
  toggleDetails(): void;
  /** The menu of the categories: scrolls the list to one of them (opening it if it was folded). */
  goToGroup(id: string): void;
  /** The folder picker of Developers: it opens in Rust and the page never sees a path. */
  addRoot(): void;
  /** `Save as text…` of a report: the save dialog opens in Rust and the answer is the section's view with `report.saved`. */
  saveReport(section: SectionId): void;
}

/** What one render needs. */
export interface Screen {
  app: AppInfo;
  view: SectionView;
  ui: UiState;
  /** The `data-key` of the control that had the keyboard focus before this render (the redraw replaces it): where a dialog returns the focus to. */
  focused: string | null;
  /** The message of the last failed command, shown above the list until the next one succeeds. */
  error: string | null;
}

const SECTION_NAMES: Record<SectionId, string> = {
  cleanup: 'Cleanup', registry: 'Registry', developers: 'Developers',
};

const KEY_LABELS: Record<string, string> = { Enter: '↵', Escape: 'Esc' };

type NoteTone = 'ok' | 'warn' | 'err' | 'info';

function byId<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`#${id} is missing from index.html`);
  return node as T;
}

/** Free text that may be long: one line, cut with an ellipsis, the full text in the tooltip. */
function ellipsis(text: string, tag: 'span' | 'div' = 'span'): HTMLElement {
  return h(tag, { class: 'ellipsis', title: text }, text);
}

/**
 * A disabled action says why: in its tooltip and, for a screen reader, in a hidden sentence its `aria-describedby` points at.
 * The sentence is a sibling of the button, not inside it: inside, it would become part of the button's name.
 */
function actionButton(view: SectionView, action: ActionView, on: Handlers): (HTMLElement | null)[] {
  const reason = disabledReason(view, action);
  const reasonId = `why-${action.id}`;
  return [
    button(
      [h('span', { 'data-fit': '' }, action.label), action.key ? keyCaps(KEY_LABELS[action.key] ?? action.key) : null],
      {
        class: action.primary ? 'im-btn im-btn--primary' : 'im-btn', 'data-action': action.id, 'data-key': `action:${action.id}`, disabled: !action.enabled,
        title: reason, 'aria-describedby': reason === null ? null : reasonId,
        onClick: () => on.send(view.section, actionIntent(view.section, action.id)),
      },
    ),
    reason === null ? null : h('span', { class: 'sr-only', id: reasonId }, reason),
  ];
}

/** The badge of the status row: Imago's chip, with its tone when it has one. */
function setBadge(text: string, tone: NoteTone | null): void {
  const badge = byId('badge');
  badge.textContent = text;
  if (tone === null) badge.removeAttribute('data-im-tone'); else badge.setAttribute('data-im-tone', tone);
}

/**
 * The status row is one live region (index.html: role=status, atomic): the badge is read in front of the sentence, and the
 * region is rewritten only when the phrase changes, so a redraw that says the same thing is not announced again.
 */
function setStatus(badge: string, text: string): void {
  const node = byId('status-text');
  const phrase = `${badge}: ${text}`;
  if (node.dataset.phrase === phrase) return;
  node.dataset.phrase = phrase;
  fill(node, h('span', { class: 'sr-only' }, `${badge}: `), ellipsis(text));
}

/** What a notice needs to open its extra lines: which are open, and how to flip one (the click goes to the page, which redraws). */
export interface NoticeCtx { expanded: ReadonlySet<string>; on: Handlers }

/** Rust's tones are the page's four: its `error` is Imago's `err`. */
export const noteTone = (tone: NoticeView['tone']): NoteTone => (tone === 'error' ? 'err' : tone);

/**
 * A notice, in the panel it is about: Imago's note, one line in its tone. The extra lines (`more`: the backup path of a Registry
 * fix, say) sit behind a button that opens them under the line, in the block form of the note: a real control, so the keyboard
 * and a screen reader reach them (a tooltip would not). Without a `ctx` the lines are simply shown. Errors and warnings are
 * announced (the note does it by its tone).
 */
let noticeSeq = 0;
export function noticeNode(notice: NoticeView, ctx?: NoticeCtx): HTMLElement {
  const key = noticeKey(notice.text);
  const extra = notice.more.length;
  const tone = noteTone(notice.tone);
  if (extra === 0) {
    const note = fromHtml(document, noteHtml({ message: notice.text, tone })).firstElementChild as HTMLElement;
    const line = note.querySelector('span');
    line?.setAttribute('title', notice.text);
    line?.classList.add('ellipsis');
    return note;
  }
  const open = ctx === undefined || ctx.expanded.has(key);
  const listId = `notice-more-${(noticeSeq += 1)}`;
  // The block note: the line, the extra lines when they are open, and the button that opens or closes them.
  const note = fromHtml(document, noteHtml({ title: notice.text, tone, actionsHtml: ctx === undefined ? undefined : '<span data-slot></span>' })).firstElementChild as HTMLElement;
  note.querySelector('b')?.setAttribute('title', notice.text);
  const lines = h('ul', { class: 'xi-more', id: listId }, ...notice.more.map((line) => h('li', {}, line)));
  const slot = note.querySelector('[data-slot]');
  if (open) (slot?.parentElement ?? note).before(lines);
  if (ctx !== undefined && slot) {
    slot.replaceWith(h('button', {
      type: 'button', class: 'im-btn im-btn--sm im-btn--ghost', 'aria-expanded': String(open), 'aria-controls': listId, 'data-key': `more:${key}`,
      'aria-label': `${notice.text}: ${open ? 'hide' : 'show'} ${extra} more ${extra === 1 ? 'line' : 'lines'}`,
      onClick: () => ctx.on.toggleExpanded(key),
    }, open ? 'Hide' : `Show ${extra} more ${extra === 1 ? 'line' : 'lines'}`));
  }
  return note;
}

/** Whose profile is cleaned, and whose session it is: both halves matter, so the note wraps instead of cutting. */
function profileNote(text: string): HTMLElement {
  const note = fromHtml(document, noteHtml({ message: text, tone: 'warn' })).firstElementChild as HTMLElement;
  note.title = text;
  return note;
}

/** The job's progress, in the status row: the job's label, Imago's bar, its percentage and the detail. No animation. */
function renderJob(root: HTMLElement, view: SectionView): void {
  const job = view.job;
  root.hidden = job === null;
  if (job === null) { fill(root); return; }
  const percent = Math.round(Math.min(1, Math.max(0, job.fraction)) * 100);
  const bar = h('div', { class: 'im-progress', role: 'progressbar', 'aria-label': job.label, 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': percent });
  bar.style.setProperty('--im-progress', String(percent));
  fill(root,
    h('span', { class: 'xi-job-label', 'data-fit': '' }, job.label),
    bar,
    h('span', { class: 'im-progress-value', 'data-fit': '' }, `${percent}%`),
    job.detail === null ? null : h('span', { class: 'xi-job-detail im-num' }, ellipsis(job.detail)));
}

/** The window when nothing can be drawn (the first command failed): the message, in the notice area. */
export function renderFailure(message: string): void {
  setBadge('Error', 'err');
  setStatus('Error', message);
  fill(byId('notices'), noticeNode({ tone: 'error', text: message, more: [] }));
}

export function renderShell(screen: Screen, on: Handlers): void {
  const { app, view, ui } = screen;

  // Action bar.
  fill(byId('actions'), ...view.actions.flatMap((a) => actionButton(view, a, on)));

  // Status row.
  setBadge(view.status.badge, null);
  setStatus(view.status.badge, view.status.text);
  byId('status-right').textContent = `${app.admin ? 'administrator' : 'standard user'} · ${ramText(app.ramTotalBytes)}`;

  // The job's progress bar.
  renderJob(byId('job'), view);

  // The tabs of the sections: built once and then only marked. Settings, which is no view yet, is a button after them.
  const sections = byId('sections');
  if (sections.childElementCount === 0) {
    fill(sections, ...SECTION_ORDER.map((id) => h('button', {
      type: 'button', role: 'tab', class: 'im-tab', 'aria-selected': 'false', 'data-section': id, 'data-key': `section:${id}`, onClick: () => on.openSection(id),
    }, SECTION_NAMES[id])));
    sections.after(h('button', {
      type: 'button', class: 'im-btn im-btn--ghost im-btn--sm', id: 'settings', 'data-key': ABOUT_KEY, onClick: () => on.openAbout(),
    }, h('span', { 'data-fit': '' }, 'Settings'), keyCaps('?')));
  }
  for (const tab of sections.querySelectorAll<HTMLElement>('.im-tab[data-section]')) {
    const here = tab.dataset.section === view.section;
    tab.removeAttribute('aria-busy');
    tab.setAttribute('aria-selected', String(here));
    if (here) tab.setAttribute('aria-current', 'page'); else tab.removeAttribute('aria-current');
  }

  // The sidebar: categories and options.
  renderSidebar(byId('sidebar'), view, ui, on);

  // List header: title, subtitle and the switch of the details (the master check box is the first cell of the list's own column header).
  fill(byId('list-head'),
    h('h4', { class: 'im-panel-title', 'data-fit': '' }, view.title),
    h('span', { class: 'im-panel-sub' }, ellipsis(view.subtitle)),
    h('span', { class: 'im-spacer' }),
    view.report === null ? h('button', {
      type: 'button', class: 'im-btn im-btn--sm', 'data-key': 'details', 'aria-expanded': String(ui.details), 'aria-controls': 'detail',
      title: ui.details ? 'Hide the details of the selected row' : 'Show the details of the selected row: paths, files in use, why',
      onClick: () => on.toggleDetails(),
    }, h('span', { 'data-fit': '' }, ui.details ? 'Hide details' : 'Details'), keyCaps('i')) : null);
  byId('main').classList.toggle('details-open', ui.details);

  // Notices, then the list and the detail of the focused row.
  const noticeCtx: NoticeCtx = { expanded: ui.expanded, on };
  fill(byId('notices'),
    ...(app.profileMismatch === null ? [] : [profileNote(app.profileMismatch)]),
    ...(screen.error === null ? [] : [noticeNode({ tone: 'error', text: screen.error, more: [] })]),
    ...view.notices.map((notice) => noticeNode(notice, noticeCtx)));
  const rowsArea = byId('rows');
  if (view.report !== null) {
    // The report replaces the list (and its search box and detail) until it is closed.
    rowsArea.classList.add('is-report');
    renderReport(rowsArea, view.section, view.report, ui.expanded, on);
    renderDetail(byId('detail'), null);
  } else {
    if (rowsArea.classList.contains('is-report')) { rowsArea.classList.remove('is-report'); fill(rowsArea); }
    renderList(rowsArea, view, ui, on);
    const focus = currentFocus(ui, view);
    const focused = shownRows(ui, view).find((r) => r.id === focus) ?? null;
    // A category or a sub-heading has nothing to add to its own line (its note is on it).
    renderDetail(byId('detail'), focused?.kind === 'group' || focused?.kind === 'header' ? null : focused);
  }

  // Footer.
  fill(byId('foot'),
    h('span', { class: 'im-dot', 'data-im-tone': 'ok' }),
    h('span', { class: 'im-footer-text foot-left' }, ellipsis(view.footer ?? '')),
    h('span', { class: 'im-spacer' }),
    h('span', { class: 'im-footer-keys foot-right', 'data-fit': '' }, footerHint(view)));

  // The confirmation, last: it takes the focus when it opens and gives it back when it closes.
  renderDialog(view, on, screen.focused);
}
