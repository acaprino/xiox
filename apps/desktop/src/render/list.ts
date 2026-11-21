// The list of the section: a table of Imago's (`im-table`, a treegrid) with the check box that ticks everything in its column
// header, the rows (a section with several categories draws them one after another, each under its own heading row with its own
// check box), and an optional search box above. The rows are drawn in the order of `sortRows`, children hidden under a collapsed
// parent, nothing under a folded category. The page sends clicks as intents and draws what comes back; it never flips a check itself.

import { iconHtml } from '@acaprino/imago/ui';
import { fill, fromHtml, h } from '../dom.ts';
import { formatBytes } from '../format.ts';
import { bulkIntent, searchIntent } from '../intents.ts';
import { activeSort, headerBytes, rowDepths, rowParents, type SortKey } from '../rows.ts';
import { bulkCheck, bulkLocked, currentFocus, focusable, foldedGroups, searching, shownRows, type UiState } from '../state.ts';
import type { Check, ColumnView, RowView, SectionView } from '../types.ts';
import type { Handlers } from './shell.ts';

const SEARCH_DELAY_MS = 150;

/** Free text that may be long: one line, cut with an ellipsis by the cell, the full text in the tooltip. */
function ellipsis(text: string, cls: string): HTMLElement {
  return h('span', { class: `ellipsis ${cls}`, title: text }, text);
}

const ariaChecked = (check: Check): string => (check === 'on' ? 'true' : check === 'mixed' ? 'mixed' : 'false');

/**
 * The check cell: the whole cell is the pointer target (a 16 px box is a hard target) and the `im-check` inside is the drawing.
 * A job running freezes the list: Rust refuses a tick then (and never greys a row for it), so the page does not offer one.
 */
function checkCell(attrs: { label: string; check: Check; disabled: boolean; title: string | null; key: string }, click: () => void, header = false): HTMLElement {
  const cell = h(header ? 'th' : 'td', { class: 'im-col-check xi-check' }, h('span', {
    class: 'im-check', role: 'checkbox', tabindex: -1, 'aria-checked': ariaChecked(attrs.check),
    'aria-label': attrs.label, 'aria-disabled': attrs.disabled ? 'true' : null, title: attrs.title, 'data-key': attrs.key,
  }));
  cell.addEventListener('click', (event) => { event.stopPropagation(); if (!attrs.disabled) click(); });
  // A box that has the focus (a click gave it) answers Space as the box it is; the list's own Space ticks the row the selection is on.
  cell.addEventListener('keydown', (event) => {
    if (event.key !== ' ' || event.repeat) return;
    event.preventDefault();
    event.stopPropagation();
    if (!attrs.disabled) click();
  });
  return cell;
}

function checkbox(row: RowView, on: Handlers, section: SectionView['section'], busy: boolean): HTMLElement {
  if (row.check === 'none') return h('td', { class: 'im-col-check xi-check' });
  const group = row.kind === 'group' || row.kind === 'header';
  return checkCell({
    label: group ? `${row.name}: tick or clear every item that can be ticked` : row.name,
    check: row.check, disabled: !row.enabled || busy, title: row.why, key: `ck:${row.id}`,
  }, () => on.toggleRow(row.id, section));
}

/** The check box at the top of the list: ticks, or clears, everything Rust lets it. */
function masterCheck(view: SectionView, on: Handlers): HTMLElement {
  const check = bulkCheck(view);
  return checkCell({
    label: 'Tick or clear everything that can be ticked', check, disabled: check === 'none' || bulkLocked(view),
    title: check === 'on' ? 'Clear everything' : 'Tick everything that can be ticked (items marked caution stay as they are)', key: 'bulk:master',
  }, () => on.send(view.section, bulkIntent(view.section, check !== 'on')), true);
}

/** The line under the name: the path or reason, or, for a blocked row (muted and disabled), why it cannot be chosen. */
function subLine(row: RowView): HTMLElement | null {
  if (row.muted && !row.enabled && row.why) return h('span', { class: 'im-cell-sub im-text-warn', title: row.why, 'data-blocked': '' }, row.why);
  return row.sub ? h('span', { class: 'im-cell-sub xi-sub', title: row.sub }, row.sub) : null;
}

/** The id of a row's element: what the table's `aria-activedescendant` names. */
const rowDomId = (id: string): string => `row-${encodeURIComponent(id)}`;

function sizeText(row: RowView): string {
  if (row.bytes !== null) return formatBytes(row.bytes);
  return row.sizeText ?? '—';
}

/** The attributes of a row's place in the tree: its level for a screen reader, and its depth for the indent (xiox.css). */
const treeAttrs = (depth: number): Record<string, string | number | null> => ({ 'aria-level': depth + 1, style: depth > 0 ? `--xi-depth: ${depth}` : null });

/** A caret: the mouse's handle on a row that folds or opens. It is no Tab stop (the arrows fold), and says what it does in its tooltip. */
function caret(row: RowView, what: string, key: string, click: () => void): HTMLElement {
  return h('button', {
    type: 'button', class: 'xi-caret', tabindex: -1, 'aria-hidden': 'true', title: `${row.name}: ${what}`, 'data-key': key,
    onClick: (event: Event) => { event.stopPropagation(); click(); },
  });
}

/**
 * A sub-heading. Plain, it is a line of text the selection skips; with a box (the rows under it can be ticked in one go)
 * it is a row of its own: the box in the check column, its folder under the name, what its rows add up to in the size
 * column; the selection can rest on it and Space ticks it. A folder of the projects' tree is a sub-heading with rows
 * hanging under it (`open`: not folded; null: not a folder): a row of its own even without a box, with a caret, and a
 * click folds it or opens it, as a category's does.
 */
function headerNode(row: RowView, view: SectionView, checks: boolean, open: boolean | null, depth: number, focused: boolean, columns: number, on: Handlers): HTMLElement {
  const folder = open !== null;
  if (!folder && (!checks || row.check === 'none')) {
    return h('tr', { class: 'xi-section', role: 'row', 'data-row': row.id }, h('td', { colspan: columns }, ellipsis(row.name, 'xi-section-name')));
  }
  const sized = view.columns[view.columns.length - 1]?.kind === 'size';
  const bytes = sized ? headerBytes(view.rows, row.id) : null;
  const text = h('span', { class: 'xi-name-text' }, h('span', { class: 'im-cell-title xi-title', title: row.name }, row.name), row.sub ? h('span', { class: 'im-cell-sub xi-sub', title: row.sub }, row.sub) : null);
  const name = h('td', { class: 'xi-name', colspan: sized ? columns - (checks ? 2 : 1) : columns - (checks ? 1 : 0) },
    h('div', { class: 'xi-nm' }, folder ? caret(row, 'show or hide the folders and projects inside', `fold:${row.id}`, () => on.toggleFold(row.id)) : null, text));
  return h('tr', {
    class: ['xi-heading', folder ? 'xi-folder' : ''].filter(Boolean).join(' '),
    id: rowDomId(row.id), role: 'row', 'data-row': row.id, 'aria-selected': String(focused), 'aria-expanded': folder ? String(open) : null, ...treeAttrs(depth),
    onClick: () => { if (folder) on.toggleFold(row.id); else on.focusRow(row.id); },
  },
  checks ? checkbox(row, on, view.section, view.job !== null) : null,
  name,
  sized ? h('td', { class: 'im-num' }, bytes === null ? '' : formatBytes(bytes)) : null);
}

function rowNode(row: RowView, view: SectionView, ui: UiState, checks: boolean, parents: ReadonlySet<string>, anyParent: boolean, folded: ReadonlySet<string>, depth: number, focused: boolean, columns: number, on: Handlers): HTMLElement {
  if (row.kind === 'header') return headerNode(row, view, checks, parents.has(row.id) ? !folded.has(row.id) : null, depth, focused, columns, on);
  const group = row.kind === 'group';
  const cells = [...row.cells];
  const parts: HTMLElement[] = [];
  if (checks) parts.push(checkbox(row, on, view.section, view.job !== null));
  for (const column of view.columns) {
    if (column.kind === 'name') {
      let handle: HTMLElement | null = null;
      if (group) handle = caret(row, 'show or hide the items', `fold:${row.id}`, () => on.toggleFold(row.id));
      else if (parents.has(row.id)) handle = caret(row, 'show or hide the items inside', `caret:${row.id}`, () => on.toggleExpanded(row.id));
      else if (anyParent || depth > 0) handle = h('span', { class: 'xi-caret-gap', 'aria-hidden': 'true' });
      const note = group ? [view.nav.find((n) => n.id === row.id)?.note, row.why].filter(Boolean).join(' · ') : null;
      parts.push(h('td', { class: 'xi-name' }, h('div', { class: 'xi-nm' }, handle,
        h('span', { class: 'xi-name-text' }, h('span', { class: 'im-cell-title xi-title', title: row.name }, row.name),
          note ? h('span', { class: 'im-cell-sub xi-sub', title: note }, note) : subLine(row)))));
    } else if (column.kind === 'cell') {
      const text = cells.shift() ?? '';
      parts.push(h('td', { class: 'im-num xi-cell', title: text }, text));
    } else if (column.kind === 'level') {
      parts.push(h('td', { class: 'xi-level' }, row.level && !group ? levelChip(row.level) : null));
    } else {
      parts.push(h('td', { class: 'im-num xi-size' }, group && row.bytes === null ? '' : sizeText(row)));
    }
  }
  const cls = [group ? 'xi-group' : '', row.kind === 'child' ? 'xi-child' : ''].filter(Boolean).join(' ');
  // Tree grid: the row is expanded or collapsed when it has rows to show, selected when it has the focus, and sits at its depth.
  const expandable = group || parents.has(row.id);
  const expanded = group ? !folded.has(row.id) : ui.expanded.has(row.id);
  return h('tr', {
    class: cls || null, id: rowDomId(row.id), role: 'row', 'data-row': row.id, 'aria-selected': String(focused), ...treeAttrs(depth),
    'aria-expanded': expandable ? String(expanded) : null, 'aria-disabled': row.muted && !row.enabled ? 'true' : null,
    // A category row folds its rows (its check box, in the first cell, ticks them); any other row takes the focus, which
    // is what the detail panel follows. The tick is the check cell's business: a whole cell, not a 16 px box.
    onClick: () => { if (group) on.toggleFold(row.id); else on.focusRow(row.id); },
  }, ...parts);
}

/** The level as a chip: recommended is `ok`, caution `warn`, optional the neutral chip. */
function levelChip(level: NonNullable<RowView['level']>): HTMLElement {
  const tone = level === 'recommended' ? 'ok' : level === 'caution' ? 'warn' : null;
  return h('span', { class: 'im-chip', 'data-im-tone': tone, 'data-level': level }, level.charAt(0).toUpperCase() + level.slice(1));
}

/**
 * A column's header: a button that orders the list by that column (a click on the active one turns the order round). The
 * arrow and `aria-sort` say which column and which way. Like the check boxes it is the mouse's: `s` / `S` are the keyboard's.
 */
function columnHeader(column: ColumnView, active: SortKey | null, on: Handlers): HTMLElement {
  const way = active?.column !== column.title ? null : active.desc ? 'descending' : 'ascending';
  return h('th', { class: column.kind === 'cell' || column.kind === 'size' ? 'im-num' : null, 'aria-sort': way ?? 'none', 'data-col': column.kind },
    h('button', {
      type: 'button', class: way === null ? 'xi-sort' : 'xi-sort xi-sort-on', tabindex: -1, 'data-key': `sort:${column.title}`,
      title: way === null ? `Order by ${column.title}` : `Ordered by ${column.title}, ${way}: click to turn round`,
      onClick: () => on.sortBy(column.title),
    }, h('span', { class: 'xi-sort-label' }, column.title), h('i', { class: 'xi-sort-arrow', 'aria-hidden': 'true' }, way === 'descending' ? '▼' : way === 'ascending' ? '▲' : '')));
}

/** The search box lives across renders: re-creating it would drop the text being typed and its focus. */
function syncSearch(tools: HTMLElement, view: SectionView, on: Handlers): void {
  let box = tools.querySelector<HTMLElement>('.xi-search');
  let input = box?.querySelector<HTMLInputElement>('input') ?? null;
  if (!view.search) { box?.remove(); return; }
  const s = view.section;
  if (!box || !input) {
    input = h('input', { type: 'search', 'data-key': 'search' });
    let timer: ReturnType<typeof setTimeout> | undefined;
    const field = input;
    field.addEventListener('input', () => {
      clearTimeout(timer);
      const section = field.dataset.section as SectionView['section'];
      timer = setTimeout(() => on.send(section, searchIntent(section, field.value)), SEARCH_DELAY_MS);
    });
    box = h('label', { class: 'im-input xi-search' }, fromHtml(document, iconHtml('search')), input);
    tools.prepend(box);
  }
  input.placeholder = view.search.placeholder;
  input.setAttribute('aria-label', view.search.placeholder);
  // Another section's text must not leak in, but the text being typed here is the user's until the view agrees.
  if (input.dataset.section !== s || document.activeElement !== input) input.value = view.search.value;
  input.dataset.section = s;
}

/** «Fold all» / «Open all» for a list of categories: one button, whose word follows what a click would do. */
function syncFold(tools: HTMLElement, view: SectionView, ui: UiState, on: Handlers): void {
  let btn = tools.querySelector<HTMLButtonElement>('button.xi-foldall');
  const groups = view.rows.filter((r) => r.kind === 'group');
  if (groups.length < 2) { btn?.remove(); return; }
  if (!btn) {
    const node = h('button', { type: 'button', class: 'im-btn im-btn--sm xi-foldall', 'data-key': 'fold:all' });
    node.addEventListener('click', () => on.foldAll(node.dataset.mode === 'fold'));
    tools.append(node);
    btn = node;
  }
  const folded = foldedGroups(ui, view);
  const anyOpen = groups.some((g) => !folded.has(g.id));
  btn.dataset.mode = anyOpen ? 'fold' : 'open';
  btn.textContent = anyOpen ? 'Fold all' : 'Open all';
  btn.disabled = searching(view);
  btn.title = btn.disabled ? 'A search shows every category open: clear it to fold them' : anyOpen ? 'Fold every category down to its header' : 'Show the items of every category';
}

export function renderList(root: HTMLElement, view: SectionView, ui: UiState, on: Handlers): void {
  let tools = root.querySelector<HTMLElement>(':scope > .xi-tools');
  let scroller = root.querySelector<HTMLElement>(':scope > .xi-list');
  if (!tools || !scroller) {
    tools = h('div', { class: 'im-panel-tools xi-tools' });
    scroller = h('div', { class: 'im-panel-body xi-list' });
    root.replaceChildren(tools, scroller);
  }
  syncSearch(tools, view, on);
  syncFold(tools, view, ui, on);

  const rows = shownRows(ui, view);
  const empty = view.rows.length === 0 || view.empty !== null;
  scroller.classList.toggle('is-empty', empty);
  scroller.setAttribute('aria-busy', String(view.job !== null));
  if (empty) {
    // No table (a treegrid with no rows would be announced as one) and no Tab stop. The state layer (`setPanelState` on `#rows`,
    // render/shell.ts) is a child of this body and must stay: only the table goes. The search box stays above it, in the tools.
    scroller.querySelector(':scope > table')?.remove();
    return;
  }
  const checks = view.rows.some((r) => r.check !== 'none');
  const columns = view.columns.length + (checks ? 1 : 0);
  const parents = rowParents(view.rows);
  const folded = foldedGroups(ui, view);
  const focus = currentFocus(ui, view);
  const depths = rowDepths(view.rows);
  const master = view.bulk && view.report === null && bulkIntent(view.section, true) !== null;
  const head = h('tr', {},
    checks ? (master ? masterCheck(view, on) : h('th', { class: 'im-col-check' })) : null,
    ...view.columns.map((c) => columnHeader(c, activeSort(view.columns, ui.sort[view.section] ?? null), on)));
  const body = rows.flatMap((row) => {
    const node = rowNode(row, view, ui, checks, parents, parents.size > 0, folded, depths.get(row.id) ?? 0, row.id === focus, columns, on);
    // An empty row before a category's: it does not stick, so the page can tell where the category really starts.
    return row.kind === 'group' ? [h('tr', { class: 'xi-anchor', role: 'presentation', 'data-anchor': row.id }, h('td', { colspan: columns })), node] : [node];
  });
  // The table is ONE Tab stop: the arrows move inside it, Space ticks, Left / Right fold (main.ts). The check boxes and carets of
  // its rows are for the mouse and for the arrows' sake, not Tab stops of their own. It is also where the focus goes back to
  // when a dialog closes. The focused row is announced by its id: the focus itself stays on the table.
  let table = scroller.querySelector<HTMLTableElement>(':scope > table');
  if (!table) {
    table = h('table', { class: 'im-table xi-table', role: 'treegrid', tabindex: 0 });
    table.addEventListener('keydown', (event) => on.listKey(event));
    scroller.append(table);
  }
  table.setAttribute('aria-label', view.title);
  table.classList.toggle('xi-grouped', view.rows.some((r) => r.kind === 'group'));
  const current = rows.find((r) => r.id === focus && focusable(r, parents));
  if (current) table.setAttribute('aria-activedescendant', rowDomId(current.id)); else table.removeAttribute('aria-activedescendant');
  table.replaceChildren(h('thead', {}, head), h('tbody', {}, ...body));
}
