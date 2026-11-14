// The list of the open category: a column header and the rows, with an optional search box above.
// The rows are drawn in the order of `sortRows`, children hidden under a collapsed parent. The page
// sends clicks as intents and draws what comes back; it never flips a check itself.

import { fill, h } from '../dom.ts';
import { formatBytes } from '../format.ts';
import { searchIntent, toggleIntent } from '../intents.ts';
import { gridTemplate, rowParents, sortRows, visibleRows } from '../rows.ts';
import { currentFocus, type UiState } from '../state.ts';
import type { RowView, SectionView } from '../types.ts';
import type { Handlers } from './shell.ts';

const SEARCH_DELAY_MS = 150;

function ellipsis(text: string, cls: string): HTMLElement {
  return h('span', { class: `ellipsis ${cls}`, title: text }, text);
}

/** A job running freezes the list: Rust refuses a tick then (and never greys a row for it), so the page does not offer one. */
function checkbox(row: RowView, on: Handlers, section: SectionView['section'], busy: boolean): HTMLElement {
  if (row.check === 'none') return h('span');
  return h('button', {
    type: 'button', class: 'ck', role: 'checkbox', tabindex: -1,
    'aria-checked': row.check === 'on' ? 'true' : row.check === 'mixed' ? 'mixed' : 'false',
    'aria-label': row.name, disabled: !row.enabled || busy, title: row.enabled ? null : row.why, 'data-key': `ck:${row.id}`,
    onClick: (event: Event) => {
      event.stopPropagation();
      on.focusRow(row.id);
      on.send(section, toggleIntent(section, row.id));
    },
  });
}

/** The line under the name: the path or reason, or, for a blocked row (muted and disabled), why it cannot be chosen. */
function subLine(row: RowView): HTMLElement | null {
  if (row.muted && !row.enabled && row.why) return ellipsis(row.why, 's blocked');
  return row.sub ? ellipsis(row.sub, 's') : null;
}

function sizeText(row: RowView): string {
  if (row.bytes !== null) return formatBytes(row.bytes);
  return row.sizeText ?? '—';
}

function rowNode(row: RowView, view: SectionView, ui: UiState, checks: boolean, parents: ReadonlySet<string>, anyParent: boolean, focused: boolean, on: Handlers): HTMLElement {
  if (row.kind === 'header') return h('div', { class: 'r sec', 'data-row': row.id }, ellipsis(row.name, 't'));
  const cells = [...row.cells];
  const parts: HTMLElement[] = [];
  if (checks) parts.push(checkbox(row, on, view.section, view.job !== null));
  for (const column of view.columns) {
    if (column.kind === 'name') {
      const caret = parents.has(row.id)
        ? h('button', {
          type: 'button', class: 'caret', tabindex: -1, 'aria-expanded': String(ui.expanded.has(row.id)), 'aria-label': `${row.name}: show or hide the items inside`,
          'data-key': `caret:${row.id}`, onClick: (event: Event) => { event.stopPropagation(); on.toggleExpanded(row.id); },
        })
        : anyParent ? h('span', { class: 'caret-gap' }) : null;
      parts.push(h('div', { class: 'nm' }, caret,
        h('div', { class: 'nm-text' }, ellipsis(row.name, 't'), subLine(row))));
    } else if (column.kind === 'cell') {
      const text = cells.shift() ?? '';
      parts.push(h('div', { class: 'cell num ellipsis', title: text }, text));
    } else if (column.kind === 'level') {
      parts.push(row.level ? h('div', { class: `lv ${row.level}`, 'data-fit': '' }, row.level) : h('div'));
    } else {
      parts.push(h('div', { class: 'sz num', 'data-fit': '' }, sizeText(row)));
    }
  }
  const cls = ['r', row.kind === 'child' ? 'child' : '', row.muted ? 'muted' : '', focused ? 'sel' : ''].filter(Boolean).join(' ');
  return h('div', {
    class: cls, 'data-row': row.id, 'aria-current': focused ? 'true' : null,
    onClick: () => on.focusRow(row.id),
  }, ...parts);
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
    tools.append(input);
  }
  input.placeholder = view.search.placeholder;
  input.setAttribute('aria-label', view.search.placeholder);
  // Another section's text must not leak in, but the text being typed here is the user's until the view agrees.
  if (input.dataset.section !== s || document.activeElement !== input) input.value = view.search.value;
  input.dataset.section = s;
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

  const rows = visibleRows(sortRows(view.rows), ui.expanded);
  // The grid is ONE Tab stop: the arrows move inside it, Space ticks, Left / Right fold (main.ts). The check boxes and
  // carets of its rows are for the mouse and for the arrows' sake, not Tab stops of their own. It is also where the
  // focus goes back to when a dialog closes.
  grid.tabIndex = 0;
  grid.setAttribute('aria-label', view.title);
  if (view.rows.length === 0 || view.empty !== null) {
    grid.style.gridTemplateColumns = '';
    fill(grid, h('div', { class: 'empty' },
      view.empty ? h('b', { class: 'empty-title' }, view.empty.title) : null,
      view.empty ? h('span', {}, view.empty.text) : null));
    return;
  }
  const checks = view.rows.some((r) => r.check !== 'none');
  const parents = rowParents(view.rows);
  const focus = currentFocus(ui, view);
  grid.style.gridTemplateColumns = gridTemplate(view.columns, checks);
  const head = h('div', { class: 'r h' },
    checks ? h('span') : null,
    ...view.columns.map((c) => h('span', { class: `col-${c.kind}`, 'data-fit': '' }, c.title)));
  fill(grid, head, ...rows.map((row) => rowNode(row, view, ui, checks, parents, parents.size > 0, row.id === focus, on)));
}
