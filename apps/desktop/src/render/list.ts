// The list of the section: a column header with the check box that ticks everything, the rows (a section with several
// categories draws them one after another, each under its own header with its own check box), and an optional search box
// above. The rows are drawn in the order of `sortRows`, children hidden under a collapsed parent, nothing under a folded
// category. The page sends clicks as intents and draws what comes back; it never flips a check itself.

import { fill, h } from '../dom.ts';
import { formatBytes } from '../format.ts';
import { actionIntent, bulkIntent, searchIntent } from '../intents.ts';
import { activeSort, gridVars, headerBytes, rowDepths, rowParents, type SortKey } from '../rows.ts';
import { bulkCheck, bulkLocked, currentFocus, focusable, foldedGroups, searching, shownRows, type UiState } from '../state.ts';
import type { Check, ColumnView, RowView, SectionView } from '../types.ts';
import { emptyAction } from './policy.ts';
import type { Handlers } from './shell.ts';

const SEARCH_DELAY_MS = 150;

function ellipsis(text: string, cls: string): HTMLElement {
  return h('span', { class: `ellipsis ${cls}`, title: text }, text);
}

const ariaChecked = (check: Check): string => (check === 'on' ? 'true' : check === 'mixed' ? 'mixed' : 'false');

/**
 * The check cell: the whole 34 px cell is the button (a 14 px box is a hard target) and the box is drawn inside it. A job
 * running freezes the list: Rust refuses a tick then (and never greys a row for it), so the page does not offer one.
 */
function checkCell(attrs: { label: string; check: Check; disabled: boolean; title: string | null; key: string }, click: () => void, cell: 'gridcell' | 'columnheader' = 'gridcell'): HTMLElement {
  return h('div', { class: 'ckcell', role: cell }, h('button', {
    type: 'button', class: 'ckc', role: 'checkbox', tabindex: -1, 'aria-checked': ariaChecked(attrs.check),
    'aria-label': attrs.label, disabled: attrs.disabled, title: attrs.title, 'data-key': attrs.key,
    onClick: (event: Event) => { event.stopPropagation(); click(); },
  }, h('i', { class: 'ckbox' })));
}

function checkbox(row: RowView, on: Handlers, section: SectionView['section'], busy: boolean): HTMLElement {
  if (row.check === 'none') return h('span', { role: 'gridcell' });
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
  }, () => on.send(view.section, bulkIntent(view.section, check !== 'on')), 'columnheader');
}

/** The line under the name: the path or reason, or, for a blocked row (muted and disabled), why it cannot be chosen. */
function subLine(row: RowView): HTMLElement | null {
  if (row.muted && !row.enabled && row.why) return ellipsis(row.why, 's blocked');
  return row.sub ? ellipsis(row.sub, 's') : null;
}

/** The id of a row's element: what the grid's `aria-activedescendant` names. */
const rowDomId = (id: string): string => `row-${encodeURIComponent(id)}`;

function sizeText(row: RowView): string {
  if (row.bytes !== null) return formatBytes(row.bytes);
  return row.sizeText ?? '—';
}

/** The attributes of a row's place in the tree: its level for a screen reader, and its depth for the indent (Smooth: smooth.css). */
const treeAttrs = (depth: number): Record<string, string | number | null> => ({ 'aria-level': depth + 1, style: depth > 0 ? `--depth: ${depth}` : null });

/**
 * A sub-heading. Plain, it is a line of text the selection skips; with a box (the rows under it can be ticked in one go)
 * it is a row of its own: the box in the check column, its folder under the name, what its rows add up to in the size
 * column; the selection can rest on it and Space ticks it. A folder of the projects' tree is a sub-heading with rows
 * hanging under it (`open`: not folded; null: not a folder): a row of its own even without a box, with a caret, and a
 * click folds it or opens it, as a category's does.
 */
function headerNode(row: RowView, view: SectionView, checks: boolean, open: boolean | null, depth: number, focused: boolean, on: Handlers): HTMLElement {
  const folder = open !== null;
  if (!folder && (!checks || row.check === 'none')) return h('div', { class: 'r sec', role: 'row', 'data-row': row.id }, h('div', { role: 'gridcell' }, ellipsis(row.name, 't')));
  const sized = view.columns[view.columns.length - 1]?.kind === 'size';
  const bytes = sized ? headerBytes(view.rows, row.id) : null;
  const text = [ellipsis(row.name, 't'), row.sub ? ellipsis(row.sub, 's') : null];
  return h('div', {
    class: ['r sec boxed', folder ? 'folder' : '', sized ? 'sized' : '', focused ? 'sel' : ''].filter(Boolean).join(' '),
    id: rowDomId(row.id), role: 'row', 'data-row': row.id, 'aria-selected': String(focused), 'aria-expanded': folder ? String(open) : null, ...treeAttrs(depth),
    onClick: () => { if (folder) on.toggleFold(row.id); else on.focusRow(row.id); },
  },
  checks ? checkbox(row, on, view.section, view.job !== null) : null,
  folder
    ? h('div', { class: 'sec-name nm', role: 'gridcell' },
      h('button', {
        type: 'button', class: 'caret', tabindex: -1, 'aria-hidden': 'true', 'aria-label': `${row.name}: show or hide the folders and projects inside`,
        'data-key': `fold:${row.id}`, onClick: (event: Event) => { event.stopPropagation(); on.toggleFold(row.id); },
      }),
      h('div', { class: 'nm-text' }, ...text))
    : h('div', { class: 'sec-name nm-text', role: 'gridcell' }, ...text),
  sized ? h('div', { class: 'sz num', role: 'gridcell', 'data-fit': '' }, bytes === null ? '' : formatBytes(bytes)) : null);
}

function rowNode(row: RowView, view: SectionView, ui: UiState, checks: boolean, parents: ReadonlySet<string>, anyParent: boolean, folded: ReadonlySet<string>, depth: number, focused: boolean, on: Handlers): HTMLElement {
  if (row.kind === 'header') return headerNode(row, view, checks, parents.has(row.id) ? !folded.has(row.id) : null, depth, focused, on);
  const group = row.kind === 'group';
  const cells = [...row.cells];
  const parts: HTMLElement[] = [];
  if (checks) parts.push(checkbox(row, on, view.section, view.job !== null));
  for (const column of view.columns) {
    if (column.kind === 'name') {
      let caret: HTMLElement | null = null;
      if (group) {
        caret = h('button', {
          type: 'button', class: 'caret', tabindex: -1, 'aria-hidden': 'true', 'aria-label': `${row.name}: show or hide the items`,
          'data-key': `fold:${row.id}`, onClick: (event: Event) => { event.stopPropagation(); on.toggleFold(row.id); },
        });
      } else if (parents.has(row.id)) {
        caret = h('button', {
          type: 'button', class: 'caret', tabindex: -1, 'aria-hidden': 'true', 'aria-label': `${row.name}: show or hide the items inside`,
          'data-key': `caret:${row.id}`, onClick: (event: Event) => { event.stopPropagation(); on.toggleExpanded(row.id); },
        });
      } else if (anyParent) {
        caret = h('span', { class: 'caret-gap' });
      } else if (depth > 0) {
        // A row under a heading keeps the place of the heading's caret, so its name sits one level in (Smooth; ONI MODE hides it).
        caret = h('span', { class: 'caret-gap tree' });
      }
      const note = group ? [view.nav.find((n) => n.id === row.id)?.note, row.why].filter(Boolean).join(' · ') : null;
      parts.push(h('div', { class: 'nm', role: 'gridcell' }, caret,
        h('div', { class: 'nm-text' }, ellipsis(row.name, 't'), note ? ellipsis(note, 's') : subLine(row))));
    } else if (column.kind === 'cell') {
      const text = cells.shift() ?? '';
      parts.push(h('div', { class: 'cell num ellipsis', role: 'gridcell', title: text }, text));
    } else if (column.kind === 'level') {
      parts.push(row.level && !group ? h('div', { class: `lv ${row.level}`, role: 'gridcell', 'data-fit': '' }, row.level) : h('div', { class: 'lv', role: 'gridcell' }));
    } else {
      parts.push(h('div', { class: 'sz num', role: 'gridcell', 'data-fit': '' }, group && row.bytes === null ? '' : sizeText(row)));
    }
  }
  const cls = ['r', group ? 'grp' : '', row.kind === 'child' ? 'child' : '', row.muted ? 'muted' : '', focused ? 'sel' : ''].filter(Boolean).join(' ');
  // Tree grid: the row is expanded or collapsed when it has rows to show, selected when it has the focus, and sits at its depth.
  const expandable = group || parents.has(row.id);
  const expanded = group ? !folded.has(row.id) : ui.expanded.has(row.id);
  return h('div', {
    class: cls, id: rowDomId(row.id), role: 'row', 'data-row': row.id, 'aria-selected': String(focused), ...treeAttrs(depth),
    'aria-expanded': expandable ? String(expanded) : null,
    // A category header folds its rows (its check box, in the first cell, ticks them); any other row takes the focus, which
    // is what the detail panel follows. The tick is the check cell's business: a 34 px target, not a 14 px box.
    onClick: () => { if (group) on.toggleFold(row.id); else on.focusRow(row.id); },
  }, ...parts);
}

/**
 * A column's header: a button that orders the list by that column (a click on the active one turns the order round). The
 * arrow and `aria-sort` say which column and which way. Like the check boxes it is the mouse's: `s` / `S` are the keyboard's.
 */
function columnHeader(column: ColumnView, active: SortKey | null, on: Handlers): HTMLElement {
  const way = active?.column !== column.title ? null : active.desc ? 'descending' : 'ascending';
  return h('span', { class: `col-${column.kind}`, role: 'columnheader', 'aria-sort': way ?? 'none' },
    h('button', {
      type: 'button', class: way === null ? 'sort' : 'sort on', tabindex: -1, 'data-key': `sort:${column.title}`,
      title: way === null ? `Order by ${column.title}` : `Ordered by ${column.title}, ${way}: click to turn round`,
      onClick: () => on.sortBy(column.title),
    }, h('span', { class: 'sort-label', 'data-fit': '' }, column.title), h('i', { class: 'sort-arrow', 'aria-hidden': 'true' }, way === 'descending' ? '▼' : way === 'ascending' ? '▲' : '')));
}

/** The search box lives across renders: re-creating it would drop the text being typed and its focus. */
function syncSearch(tools: HTMLElement, view: SectionView, on: Handlers): void {
  let input = tools.querySelector<HTMLInputElement>('input.search');
  if (!view.search) { input?.remove(); return; }
  const s = view.section;
  if (!input) {
    input = h('input', { type: 'search', class: 'field search', 'data-key': 'search' });
    let timer: ReturnType<typeof setTimeout> | undefined;
    const box = input;
    box.addEventListener('input', () => {
      clearTimeout(timer);
      const section = box.dataset.section as SectionView['section'];
      timer = setTimeout(() => on.send(section, searchIntent(section, box.value)), SEARCH_DELAY_MS);
    });
    tools.prepend(input);
  }
  input.placeholder = view.search.placeholder;
  input.setAttribute('aria-label', view.search.placeholder);
  // Another section's text must not leak in, but the text being typed here is the user's until the view agrees.
  if (input.dataset.section !== s || document.activeElement !== input) input.value = view.search.value;
  input.dataset.section = s;
}

/** «Fold all» / «Open all» for a list of categories: one button, whose word follows what a click would do. */
function syncFold(tools: HTMLElement, view: SectionView, ui: UiState, on: Handlers): void {
  let btn = tools.querySelector<HTMLButtonElement>('button.foldall');
  const groups = view.rows.filter((r) => r.kind === 'group');
  if (groups.length < 2) { btn?.remove(); return; }
  if (!btn) {
    const node = h('button', { type: 'button', class: 'btn sm foldall', 'data-key': 'fold:all' });
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
  let tools = root.querySelector<HTMLElement>(':scope > .tools');
  let grid = root.querySelector<HTMLElement>(':scope > .grid');
  if (!tools || !grid) {
    tools = h('div', { class: 'tools' });
    grid = h('div', { class: 'grid' });
    root.replaceChildren(tools, grid);
  }
  syncSearch(tools, view, on);
  syncFold(tools, view, ui, on);

  const rows = shownRows(ui, view);
  // The grid is ONE Tab stop: the arrows move inside it, Space ticks, Left / Right fold (main.ts). The check boxes and
  // carets of its rows are for the mouse and for the arrows' sake, not Tab stops of their own. It is also where the
  // focus goes back to when a dialog closes.
  grid.tabIndex = 0;
  grid.setAttribute('aria-label', view.title);
  grid.setAttribute('aria-busy', String(view.job !== null));
  const empty = view.rows.length === 0 || view.empty !== null;
  grid.classList.toggle('is-empty', empty);
  // An empty list is no grid: a treegrid with no rows would be announced as one.
  if (empty) grid.removeAttribute('role'); else grid.setAttribute('role', 'treegrid');
  if (empty) {
    // Nothing for the arrows to move over: the grid is no Tab stop (so no pane-wide focus ring); the action it offers is.
    grid.tabIndex = -1;
    grid.removeAttribute('aria-activedescendant');
    for (const name of ['--cols', '--cols-l', '--cols-ls']) grid.style.removeProperty(name);
    grid.classList.remove('grouped');
    const start = emptyAction(view);
    fill(grid, h('div', { class: view.job !== null ? 'empty busy' : 'empty' },
      view.empty ? h('b', { class: 'empty-title' }, view.empty.title) : null,
      view.empty ? h('span', {}, view.empty.text) : null,
      start === null ? null : h('button', {
        type: 'button', class: 'btn primary', 'data-key': `empty:${start.id}`, onClick: () => on.send(view.section, actionIntent(view.section, start.id)),
      }, h('span', { 'data-fit': '' }, start.label))));
    return;
  }
  const checks = view.rows.some((r) => r.check !== 'none');
  const parents = rowParents(view.rows);
  const folded = foldedGroups(ui, view);
  const focus = currentFocus(ui, view);
  const depths = rowDepths(view.rows);
  for (const [name, value] of Object.entries(gridVars(view.columns, checks))) grid.style.setProperty(name, value);
  // The focused row is announced by its id: the focus itself stays on the grid (one Tab stop).
  const current = rows.find((r) => r.id === focus && focusable(r, parents));
  if (current) grid.setAttribute('aria-activedescendant', rowDomId(current.id)); else grid.removeAttribute('aria-activedescendant');
  grid.classList.toggle('grouped', view.rows.some((r) => r.kind === 'group'));
  const master = view.bulk && view.report === null && bulkIntent(view.section, true) !== null;
  const head = h('div', { class: 'r h', role: 'row' },
    checks ? (master ? masterCheck(view, on) : h('span', { role: 'columnheader' })) : null,
    ...view.columns.map((c) => columnHeader(c, activeSort(view.columns, ui.sort[view.section] ?? null), on)));
  fill(grid, head, ...rows.flatMap((row) => {
    const node = rowNode(row, view, ui, checks, parents, parents.size > 0, folded, depths.get(row.id) ?? 0, row.id === focus, on);
    // An empty marker before a category header: it does not stick, so the page can tell where the category really starts.
    return row.kind === 'group' ? [h('div', { class: 'anchor', role: 'presentation', 'data-anchor': row.id }), node] : [node];
  }));
}
