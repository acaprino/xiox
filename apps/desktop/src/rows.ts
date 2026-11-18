// Row order and visibility. The engine sends rows in its own order; the page draws them in the order the user chose
// (a column and a direction; by default the size, largest first), keeps children under their parent and a header with
// the rows after it, and hides the children of a collapsed parent. Pure functions: no DOM, so node can test them.

import type { ColumnKind, ColumnView, LevelView, RowView } from './types.ts';

/** What a row is worth in the column the list is ordered by: a number, a text, or nothing (it goes last either way). */
export type SortValue = number | string | null;

/** The order the user chose: a column, by its title (unique within a list), and the direction. */
export interface SortKey { column: string; desc: boolean }

/** How a list is ordered: the kind of the column, a row's value in it, and the direction. */
export interface Order { kind: ColumnKind; value: (row: RowView) => SortValue; desc: boolean }

/** The engine's order of old, kept as the default: by size, largest first (rows without a size keep their place, last). */
export const BY_SIZE: Order = { kind: 'size', value: (r) => r.bytes, desc: true };

/** Names as a person reads them: «file10» after «file9», case and accents aside. */
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/** The level by risk: a list ordered by level, from the top, starts with what needs care. */
const LEVEL_RANK: Record<LevelView, number> = { recommended: 0, optional: 1, caution: 2 };

/** A count the engine writes with grouping commas (18,204) or a plain decimal (3.5). */
const NUMBER = /^-?(\d{1,3}(,\d{3})+|\d+)(\.\d+)?$/;

/** A cell as a value: a count is a number, a dash or an empty cell is nothing, anything else is its text. */
function cellValue(text: string | undefined): SortValue {
  const t = (text ?? '').trim();
  if (t === '' || t === '—' || t === '-') return null;
  return NUMBER.test(t) ? Number(t.replaceAll(',', '')) : t;
}

/** Nothing goes last in both directions; numbers compare as numbers, texts as a person reads them. */
function compare(a: SortValue, b: SortValue, desc: boolean): number {
  if (a === null || b === null) return a === b ? 0 : a === null ? 1 : -1;
  const d = typeof a === 'number' && typeof b === 'number' ? a - b : collator.compare(String(a), String(b));
  return desc ? -d : d;
}

/** Stable: rows of equal value keep the engine's order. */
function byValue<T>(items: T[], value: (item: T) => SortValue, desc: boolean): T[] {
  return items
    .map((item, index) => ({ item, index, v: value(item) }))
    .sort((a, b) => compare(a.v, b.v, desc) || a.index - b.index)
    .map((entry) => entry.item);
}

const sum = (values: SortValue[]): number | null => {
  const sized = values.filter((v): v is number => typeof v === 'number');
  return sized.length === 0 ? null : sized.reduce((a, b) => a + b, 0);
};

/** The direction a column takes on its first click: sizes and levels from the top, texts and dates from A and from the oldest. */
const naturalDesc = (kind: ColumnKind): boolean => kind === 'size' || kind === 'level';

/** The order of `key` over these columns; the default when there is no key or the list has no such column (another category of Developers). */
export function orderFor(columns: ColumnView[], key: SortKey | null): Order {
  const at = key === null ? -1 : columns.findIndex((c) => c.title === key.column);
  const column = columns[at];
  if (key === null || column === undefined) return BY_SIZE;
  const desc = key.desc;
  switch (column.kind) {
    case 'name': return { kind: 'name', value: (r) => r.name, desc };
    case 'size': return { ...BY_SIZE, desc };
    case 'level': return { kind: 'level', value: (r) => (r.level === null ? null : LEVEL_RANK[r.level]), desc };
    case 'cell': {
      // The cells of a row follow its `cell` columns in order.
      const i = columns.slice(0, at).filter((c) => c.kind === 'cell').length;
      return { kind: 'cell', value: (r) => cellValue(r.cells[i]), desc };
    }
  }
}

/** The column and direction the header marks: the user's, else the size column of the default order; null when the list keeps the engine's order. */
export function activeSort(columns: ColumnView[], key: SortKey | null): SortKey | null {
  if (key !== null && columns.some((c) => c.title === key.column)) return key;
  const size = columns.find((c) => c.kind === 'size');
  return size === undefined ? null : { column: size.title, desc: true };
}

/** A click on a column's header: the active column turns round, another one becomes active in its natural direction. */
export function nextSort(columns: ColumnView[], key: SortKey | null, title: string): SortKey | null {
  const active = activeSort(columns, key);
  if (active?.column === title) return { column: title, desc: !active.desc };
  const column = columns.find((c) => c.title === title);
  return column === undefined ? key : { column: title, desc: naturalDesc(column.kind) };
}

/** The sort key of the keyboard: the next column in its natural direction (`reverse`: the same column turned round). */
export function cycleSort(columns: ColumnView[], key: SortKey | null, reverse: boolean): SortKey | null {
  if (columns.length === 0) return key;
  const active = activeSort(columns, key);
  if (reverse) return active === null ? null : { column: active.column, desc: !active.desc };
  const at = active === null ? -1 : columns.findIndex((c) => c.title === active.column);
  const next = columns[(at + 1) % columns.length]!;
  return { column: next.title, desc: naturalDesc(next.kind) };
}

/** A top-level row with the rows that hang under it. */
interface Entry {
  row: RowView;
  children: RowView[];
  /** Its value in the order: for sizes its own bytes or what the children add up to, else its own. */
  value: SortValue;
}

function entries(group: RowView[], order: Order): Entry[] {
  const present = new Set(group.map((r) => r.id));
  const kids = new Map<string, RowView[]>();
  const tops: RowView[] = [];
  for (const row of group) {
    if (row.parent !== null && present.has(row.parent)) kids.set(row.parent, [...(kids.get(row.parent) ?? []), row]);
    else tops.push(row);
  }
  return tops.map((row) => {
    const children = byValue(kids.get(row.id) ?? [], order.value, order.desc);
    const own = order.value(row);
    return { row, children, value: order.kind === 'size' ? own ?? sum(children.map(order.value)) : own };
  });
}

function flatten(group: RowView[], order: Order): { rows: RowView[]; values: SortValue[] } {
  const sorted = byValue(entries(group, order), (e) => e.value, order.desc);
  return { rows: sorted.flatMap((e) => [e.row, ...e.children]), values: sorted.map((e) => e.value) };
}

/**
 * One block of rows in order, children under their parent. Rows before the first header stay first; each header moves
 * with the rows after it up to the next header. The headers go by what their rows add up to (sizes), by their own name,
 * or, for any other column, by the value of their first row.
 */
function sortBlock(rows: RowView[], order: Order): RowView[] {
  const lead: RowView[] = [];
  const groups: { header: RowView; rows: RowView[] }[] = [];
  for (const row of rows) {
    if (row.kind === 'header') groups.push({ header: row, rows: [] });
    else (groups.length === 0 ? lead : groups[groups.length - 1]!.rows).push(row);
  }
  const built = groups.map((g) => {
    const { rows: inside, values } = flatten(g.rows, order);
    const value = order.kind === 'size' ? sum(values) : order.kind === 'name' ? g.header.name : values[0] ?? null;
    return { header: g.header, rows: inside, value };
  });
  return [...flatten(lead, order).rows, ...byValue(built, (g) => g.value, order.desc).flatMap((g) => [g.header, ...g.rows])];
}

/**
 * The list in the order it is drawn. A list of categories (`group` rows) keeps the categories in the engine's order (the
 * menu beside it is in that order too) and sorts only inside each one; any other list is one block.
 */
export function sortRows(rows: RowView[], order: Order = BY_SIZE): RowView[] {
  if (!rows.some((r) => r.kind === 'group')) return sortBlock(rows, order);
  const out: RowView[] = [];
  let group: RowView | null = null;
  let block: RowView[] = [];
  const flush = (): void => { out.push(...(group ? [group] : []), ...sortBlock(block, order)); block = []; };
  for (const row of rows) {
    if (row.kind === 'group') { flush(); group = row; } else block.push(row);
  }
  flush();
  return out;
}

/**
 * The rows on screen: a child shows only while every parent above it is expanded, and the rows of a folded category
 * (`folded` holds the ids of its `group` rows) are not shown at all, its own row stays.
 */
export function visibleRows(rows: RowView[], expanded: ReadonlySet<string>, folded: ReadonlySet<string> = new Set()): RowView[] {
  const byId = new Map(rows.map((r) => [r.id, r]));
  const shown = (row: RowView, seen: Set<string>): boolean => {
    if (row.parent === null) return true;
    const parent = byId.get(row.parent);
    if (!parent) return true;
    if (!expanded.has(parent.id) || seen.has(parent.id)) return false;
    return shown(parent, seen.add(row.id));
  };
  let hidden = false;
  return rows.filter((row) => {
    if (row.kind === 'group') { hidden = folded.has(row.id); return true; }
    return !hidden && shown(row, new Set());
  });
}

/**
 * How deep each row sits in the tree the list draws: a category, and a sub-heading outside one, at 0; a sub-heading inside a
 * category at 1; a row one deeper than the heading above it (0 when there is none); a child one deeper than its parent.
 * Computed on the engine's order, where a heading precedes its rows and a parent its children.
 */
export function rowDepths(rows: RowView[]): Map<string, number> {
  const depth = new Map<string, number>();
  let inGroup = false;
  let base = 0;
  for (const row of rows) {
    if (row.kind === 'group') {
      depth.set(row.id, 0);
      inGroup = true;
      base = 1;
    } else if (row.kind === 'header') {
      const d = inGroup ? 1 : 0;
      depth.set(row.id, d);
      base = d + 1;
    } else {
      const parent = row.parent === null ? undefined : depth.get(row.parent);
      depth.set(row.id, parent === undefined ? base : parent + 1);
    }
  }
  return depth;
}

/** What the rows of a sub-heading add up to (its own line shows it): the top rows after it, up to the next sub-heading or category; null when none has a size. */
export function headerBytes(rows: RowView[], id: string): number | null {
  const sizes: number[] = [];
  for (const r of rows.slice(rows.findIndex((x) => x.id === id) + 1)) {
    if (r.kind === 'header' || r.kind === 'group') break;
    if (r.parent === null && r.bytes !== null) sizes.push(r.bytes);
  }
  return sizes.length === 0 ? null : sizes.reduce((a, b) => a + b, 0);
}

/** How many rows a category holds (its items and their children, not its sub-headings): the key of «long categories start folded». */
export function groupSizes(rows: RowView[]): Map<string, number> {
  const sizes = new Map<string, number>();
  let current: string | null = null;
  for (const row of rows) {
    if (row.kind === 'group') { current = row.id; sizes.set(current, 0); } else if (current !== null && row.kind !== 'header') sizes.set(current, (sizes.get(current) ?? 0) + 1);
  }
  return sizes;
}

/**
 * A cell column is as wide as its widest cell up to 260 px (a registry key is longer than the window: it is cut with an
 * ellipsis and the full text is in the tooltip) and the size column grows for a size written in words («database row only»).
 */
const COLUMN_WIDTH: Record<ColumnView['kind'], string> = {
  name: 'minmax(12ch,1fr)', cell: 'fit-content(260px)', level: '96px', size: 'minmax(74px,max-content)',
};

/** The check column is a 34 px target, not the 14 px box: a click anywhere in it ticks the row. */
const CHECK_COLUMN = '34px';

/**
 * The `grid-template-columns` of the header and every row: a check column first when rows have checks. `drop` leaves kinds of
 * column out (the narrow layouts of list.css hide the same ones, so the columns that stay keep the room).
 */
export function gridTemplate(columns: ColumnView[], checks: boolean, drop: ReadonlyArray<ColumnView['kind']> = []): string {
  return [...(checks ? [CHECK_COLUMN] : []), ...columns.filter((c) => !drop.includes(c.kind)).map((c) => COLUMN_WIDTH[c.kind])].join(' ');
}

/** The three templates the list switches between as its pane narrows: all columns, without the level, without level and cells. */
export function gridVars(columns: ColumnView[], checks: boolean): Record<'--cols' | '--cols-l' | '--cols-ls', string> {
  return {
    '--cols': gridTemplate(columns, checks),
    '--cols-l': gridTemplate(columns, checks, ['level']),
    '--cols-ls': gridTemplate(columns, checks, ['level', 'cell']),
  };
}

/** The ids of the rows that have children (they get a caret). */
export function rowParents(rows: RowView[]): Set<string> {
  return new Set(rows.flatMap((r) => (r.parent === null ? [] : [r.parent])));
}
