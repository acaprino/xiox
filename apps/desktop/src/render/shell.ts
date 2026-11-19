// The frame of the window: top bar, status row, the two sidebars, the list header, the footer.
// Everything here comes from the SectionView (Rust writes the sentences); the page adds only
// the static headings, key hints and number formatting. The list, the detail panel and bar 2
// are drawn by their own modules.

import { button, fill, h, kbd } from '../dom.ts';
import { ramText } from '../format.ts';
import { actionIntent } from '../intents.ts';
import { footerHint } from '../keys.ts';
import { disabledReason } from './policy.ts';
import { currentFocus, noticeKey, SECTION_ORDER, shownRows, type UiState } from '../state.ts';
import type { ActionView, AnyIntent, AppInfo, NoticeView, SectionId, SectionView } from '../types.ts';
import { ABOUT_KEY } from './about.ts';
import { renderBar2 } from './bar2.ts';
import { renderDetail } from './detail.ts';
import { renderDialog } from './dialog.ts';
import { renderList } from './list.ts';
import { renderReport } from './report.ts';

/** What a click or a keystroke can ask of the page; main.ts implements it. */
export interface Handlers {
  openSection(id: SectionId): void;
  /** The Settings window, with the look and what About said (the button of the sections bar; `?` opens it too). */
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
  /** Opens or closes the details of the focused row (Smooth; ONI MODE always shows them). */
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
      [h('span', { 'data-fit': '' }, action.label), action.key ? kbd(KEY_LABELS[action.key] ?? action.key) : null],
      {
        class: action.primary ? 'btn primary' : 'btn', 'data-action': action.id, 'data-key': `action:${action.id}`, disabled: !action.enabled,
        title: reason, 'aria-describedby': reason === null ? null : reasonId,
        onClick: () => on.send(view.section, actionIntent(view.section, action.id)),
      },
    ),
    reason === null ? null : h('span', { class: 'sr-only', id: reasonId }, reason),
  ];
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

/**
 * A notice strip: one line in its tone colour. The extra lines (`more`: the backup path of a Registry fix, say) sit
 * behind a button that opens them under the strip: a real control, so the keyboard and a screen reader reach them (a
 * tooltip would not). Without a `ctx` the lines are simply shown. Errors are announced.
 */
let noticeSeq = 0;
export function noticeNode(notice: NoticeView, ctx?: NoticeCtx): HTMLElement {
  const key = noticeKey(notice.text);
  const extra = notice.more.length;
  const open = extra > 0 && (ctx === undefined || ctx.expanded.has(key));
  const listId = `notice-more-${(noticeSeq += 1)}`;
  const toggle = extra === 0 || ctx === undefined ? null : h('button', {
    type: 'button', class: 'more', 'aria-expanded': String(open), 'aria-controls': listId, 'data-key': `more:${key}`,
    'aria-label': `${notice.text}: ${open ? 'hide' : 'show'} ${extra} more ${extra === 1 ? 'line' : 'lines'}`,
    onClick: () => ctx.on.toggleExpanded(key),
  }, open ? 'hide' : `+${extra} more`);
  return h('div', { class: `notice tone-${notice.tone}`, role: notice.tone === 'error' ? 'alert' : null },
    h('div', { class: 'notice-line' }, h('span', { class: 'ellipsis', title: notice.text }, notice.text), toggle),
    open ? h('ul', { class: 'notice-more', id: listId }, ...notice.more.map((line) => h('li', {}, line))) : null);
}

/** The progress bar under the status row: the job's label, the bar, its percentage and the detail. No animation. */
function renderJob(root: HTMLElement, view: SectionView): void {
  const job = view.job;
  root.hidden = job === null;
  if (job === null) { fill(root); return; }
  const percent = Math.round(Math.min(1, Math.max(0, job.fraction)) * 100);
  const fillBar = h('i', {});
  fillBar.style.width = `${percent}%`;
  fill(root,
    h('span', { class: 'job-label', 'data-fit': '' }, job.label),
    h('div', { class: 'track', role: 'progressbar', 'aria-label': job.label, 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': percent }, fillBar),
    h('span', { class: 'job-pct num', 'data-fit': '' }, `${percent}%`),
    job.detail === null ? null : h('span', { class: 'job-detail num' }, ellipsis(job.detail)));
}

/** The window when nothing can be drawn (the first command failed): the message, in the notice area. */
export function renderFailure(message: string): void {
  byId('badge').textContent = 'Error';
  setStatus('Error', message);
  fill(byId('notices'), noticeNode({ tone: 'error', text: message, more: [] }));
}

export function renderShell(screen: Screen, on: Handlers): void {
  const { app, view, ui } = screen;

  // Top bar.
  byId('version').textContent = app.version;
  fill(byId('actions'), ...view.actions.flatMap((a) => actionButton(view, a, on)));

  // Status row.
  byId('badge').textContent = view.status.badge;
  setStatus(view.status.badge, view.status.text);
  byId('status-right').textContent = `${app.admin ? 'administrator' : 'standard user'} · ${ramText(app.ramTotalBytes)}`;
  const profile = byId('profile');
  profile.hidden = app.profileMismatch === null;
  // Both halves of the sentence matter (whose profile is cleaned, and whose session it is): it wraps instead of cutting.
  if (app.profileMismatch !== null) { profile.title = app.profileMismatch; fill(profile, app.profileMismatch); }

  // The job's progress bar.
  renderJob(byId('job'), view);

  // Sections bar: built once and then only marked, so that the bar under the open tab (Smooth) can slide to the next one.
  const sections = byId('sections');
  if (sections.childElementCount === 0) {
    fill(sections,
      h('div', { class: 'hd' }, h('h2', { class: 'hd-title', 'data-fit': '' }, 'Sections'), h('span', { class: 'count' }, SECTION_ORDER.length)),
      ...SECTION_ORDER.map((id, index) => h('button', {
        type: 'button', class: 'nv', 'data-section': id, 'data-key': `section:${id}`, onClick: () => on.openSection(id),
      }, h('span', { 'data-fit': '' }, SECTION_NAMES[id]), kbd(String(index + 1)))),
      h('button', { type: 'button', class: 'nv about', 'data-key': ABOUT_KEY, onClick: () => on.openAbout() }, h('span', { 'data-fit': '' }, 'Settings'), kbd('?')));
  }
  for (const tab of sections.querySelectorAll<HTMLElement>('.nv[data-section]')) {
    const here = tab.dataset.section === view.section;
    tab.classList.toggle('on', here);
    tab.removeAttribute('aria-busy');
    if (here) tab.setAttribute('aria-current', 'page'); else tab.removeAttribute('aria-current');
  }

  // Bar 2: categories and options.
  renderBar2(byId('bar2'), view, ui, on);

  // List header: title, subtitle and the switch of the details (the master check box is the first cell of the list's own column header).
  fill(byId('list-head'),
    h('h1', { class: 'hd-title', 'data-fit': '' }, view.title),
    h('span', { class: 'hd-right' }, h('span', { class: 'sub' }, ellipsis(view.subtitle)),
      view.report === null ? h('button', {
        type: 'button', class: 'btn sm details-toggle', 'data-key': 'details', 'aria-expanded': String(ui.details), 'aria-controls': 'detail',
        title: ui.details ? 'Hide the details of the selected row' : 'Show the details of the selected row: paths, files in use, why',
        onClick: () => on.toggleDetails(),
      }, h('span', { 'data-fit': '' }, ui.details ? 'Hide details' : 'Details'), kbd('i')) : null));
  byId('main').classList.toggle('details-open', ui.details);

  // Notices, then the list and the detail of the focused row.
  const noticeCtx: NoticeCtx = { expanded: ui.expanded, on };
  fill(byId('notices'),
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
    h('span', { class: 'dot' }),
    h('span', { class: 'foot-left' }, ellipsis(view.footer ?? '')),
    h('span', { class: 'foot-right', 'data-fit': '' }, footerHint(view)));

  // The confirmation, last: it takes the focus when it opens and gives it back when it closes.
  renderDialog(view, on, screen.focused);
}
