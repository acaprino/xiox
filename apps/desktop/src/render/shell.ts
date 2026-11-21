// The frame of the window: title bar, status row, the two sidebars, the list header, the footer.
// Everything here comes from the SectionView (Rust writes the sentences); the page adds only
// the static headings, key hints and number formatting. The list, the detail panel and bar 2
// are drawn by their own modules.

import { footerKeys, LOADING_DELAY_MS, noteHtml, setPanelState, showAfter, type Registry } from '@acaprino/imago/ui';
import { button, fill, fromHtml, h, keyCaps } from '../dom.ts';
import { ramText } from '../format.ts';
import { actionIntent } from '../intents.ts';
import { footerEntries } from '../commands.ts';
import { disabledReason, emptyAction } from './policy.ts';
import { currentFocus, noticeKey, SECTION_ORDER, shownRows, type UiState } from '../state.ts';
import type { ActionView, AnyIntent, AppInfo, NoticeView, RowView, SectionId, SectionView } from '../types.ts';
import { renderSidebar } from './sidebar.ts';
import { panelOptions, renderInspector } from './inspector.ts';
import { renderDialog } from './dialog.ts';
import { renderList } from './list.ts';
import { renderReport } from './report.ts';

/** What a click or a keystroke can ask of the page; main.ts implements it. */
export interface Handlers {
  openSection(id: SectionId): void;
  /** Settings, the fourth view (its tab; `4` and `?` open it too). */
  openSettings(): void;
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
  /** Opens the inspector on the details of the focused row, or on the section's options; the same one again closes it. */
  toggleInspector(kind: 'details' | 'options'): void;
  /** A key pressed on the list: it moves, ticks, folds, or presses the primary action (commands.ts `listKey`). */
  listKey(event: KeyboardEvent): void;
  /** Closes the inspector (its own button). */
  closeInspector(): void;
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
  /** The commands, for the footer's key list. */
  registry: Registry;
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

/** The panel state on screen (`ready`, `empty`, `loading` or `error`, with what it says), so that a render that changes nothing leaves it alone. */
let panelState = 'ready';
/** Cancels a loading state that is waiting for its delay. */
let cancelLoading: (() => void) | null = null;

/**
 * The content panel's state: a list with nothing in it is the panel's empty state (with the section's start action), a job with no row
 * yet is its loading state, shown only after `LOADING_DELAY_MS` so a quick answer never flashes; a list or a report is `ready`. The panel
 * keeps its header and its frame, and `setPanelState` hides the list under the state's layer.
 */
function syncPanelState(view: SectionView, ui: UiState, on: Handlers): void {
  const content = byId('main');
  // Settings draws its own page in the panel: the section's empty or loading layer must not hide it.
  const empty = !ui.settings && view.report === null && (view.rows.length === 0 || view.empty !== null);
  const kind = !empty ? 'ready' : view.job !== null ? 'loading' : 'empty';
  const start = kind === 'empty' ? emptyAction(view) : null;
  const key = `${view.section}|${kind}|${view.empty?.title ?? ''}|${view.empty?.text ?? ''}|${start?.id ?? ''}|${start?.label ?? ''}`;
  if (key === panelState) return;
  panelState = key;
  cancelLoading?.();
  cancelLoading = null;
  if (kind === 'ready') {
    setPanelState(content, { kind: 'ready' });
  } else if (kind === 'loading') {
    cancelLoading = showAfter(LOADING_DELAY_MS, () => {
      setPanelState(content, { kind: 'loading', rows: 3 });
      content.setAttribute('aria-busy', 'true');
    });
  } else {
    setPanelState(content, {
      kind: 'empty', title: view.empty?.title, text: view.empty?.text ?? 'Nothing to show.',
      action: start === null ? undefined : { label: start.label, run: () => on.send(view.section, actionIntent(view.section, start.id)) },
    });
  }
}

/** The window when nothing can be drawn (the first command failed): the content panel's error state, with a Retry that asks again. */
export function renderFailure(message: string, retry: () => void): void {
  setBadge('Error', 'err');
  setStatus('Error', message);
  cancelLoading?.();
  cancelLoading = null;
  panelState = 'error';
  setPanelState(byId('main'), { kind: 'error', text: message, retry: { label: 'Retry', run: retry } });
}

/** A button of the content header that opens the inspector on `kind`, or closes it when it is open on it. */
function inspectorButton(kind: 'details' | 'options', label: string, key: string, ui: UiState, hint: string, on: Handlers, disabledReason: string | null = null): HTMLElement {
  const open = ui.inspector === kind;
  return h('button', {
    type: 'button', class: 'im-btn im-btn--ghost im-btn--sm', 'data-key': `inspect:${kind}`, 'aria-expanded': String(open), disabled: disabledReason !== null,
    title: disabledReason ?? (open ? `Close the ${label.toLowerCase()}` : hint), onClick: () => on.toggleInspector(kind),
  }, h('span', { 'data-fit': '' }, label), keyCaps(key));
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
    // The views of the app, and Settings last.
    fill(sections, ...[...SECTION_ORDER, 'settings' as const].map((id) => h('button', {
      type: 'button', role: 'tab', class: 'im-tab', 'aria-selected': 'false', 'data-section': id, 'data-key': `section:${id}`,
      onClick: () => (id === 'settings' ? on.openSettings() : on.openSection(id)),
    }, id === 'settings' ? 'Settings' : SECTION_NAMES[id])));
  }
  for (const tab of sections.querySelectorAll<HTMLElement>('.im-tab[data-section]')) {
    const here = tab.dataset.section === (ui.settings ? 'settings' : view.section);
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
    view.report === null ? inspectorButton('details', 'Details', 'i', ui, 'The details of the selected row: paths, files in use, why', on) : null,
    view.report === null ? inspectorButton('options', 'Options', 'o', ui, 'The options of this section', on, panelOptions(view).length === 0 ? 'This section has no options' : null) : null);

  // Notices, then the list and the detail of the focused row.
  const noticeCtx: NoticeCtx = { expanded: ui.expanded, on };
  fill(byId('notices'),
    ...(app.profileMismatch === null ? [] : [profileNote(app.profileMismatch)]),
    ...view.notices.map((notice) => noticeNode(notice, noticeCtx)));
  const rowsArea = byId('rows');
  let detailRow: RowView | null = null;
  if (view.report !== null) {
    // The report replaces the list (and its search box and detail) until it is closed.
    rowsArea.classList.add('is-report');
    renderReport(rowsArea, view.section, view.report, ui.expanded, on);
  } else {
    if (rowsArea.classList.contains('is-report')) { rowsArea.classList.remove('is-report'); fill(rowsArea); }
    renderList(rowsArea, view, ui, on);
    const focus = currentFocus(ui, view);
    const focused = shownRows(ui, view).find((r) => r.id === focus) ?? null;
    // A category or a sub-heading has nothing to add to its own line (its note is on it).
    detailRow = focused?.kind === 'group' || focused?.kind === 'header' ? null : focused;
  }
  syncPanelState(view, ui, on);
  renderInspector(byId('app'), view, ui, detailRow, on);

  // Footer.
  fill(byId('foot'),
    h('span', { class: 'im-dot', 'data-im-tone': 'ok' }),
    h('span', { class: 'im-footer-text foot-left' }, ellipsis(view.footer ?? '')),
    h('span', { class: 'im-spacer' }),
    footerKeys(document, screen.registry, footerEntries(view, (id) => screen.registry.get(id) !== undefined)));

  // The confirmation, last: it takes the focus when it opens and gives it back when it closes.
  renderDialog(view, on);
}
