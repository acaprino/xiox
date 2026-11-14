// Row order and visibility. The engine sends rows in its own order; the page shows rows with sizes
// largest first (children stay under their parent, a header keeps the rows after it) and hides the
// children of a collapsed parent. Pure functions: no DOM, so node can test them.

import type { ColumnView, RowView } from './types.ts';

/** A top-level row with the rows that hang under it. */
interface Entry {
  row: RowView;
  children: RowView[];
  /** Own bytes, or what the children add up to; null when nothing has a size. */
  bytes: number | null;
}

const sum = (values: (number | null)[]): number | null => {
  const sized = values.filter((v): v is number => v !== null);
  return sized.length === 0 ? null : sized.reduce((a, b) => a + b, 0);
};

/** Stable sort, largest first; rows without a size follow the sized ones in their original order. */
function bySize<T>(items: T[], bytes: (item: T) => number | null): T[] {
  return items
    .map((item, index) => ({ item, index, size: bytes(item) ?? -1 }))
    .sort((a, b) => b.size - a.size || a.index - b.index)
    .map((entry) => entry.item);
}

function entries(group: RowView[]): Entry[] {
  const present = new Set(group.map((r) => r.id));
  const kids = new Map<string, RowView[]>();
  const tops: RowView[] = [];
  for (const row of group) {
    if (row.parent !== null && present.has(row.parent)) kids.set(row.parent, [...(kids.get(row.parent) ?? []), row]);
    else tops.push(row);
  }
  return tops.map((row) => {
    const children = bySize(kids.get(row.id) ?? [], (c) => c.bytes);
    return { row, children, bytes: row.bytes ?? sum(children.map((c) => c.bytes)) };
  });
}

function flatten(group: RowView[]): { rows: RowView[]; bytes: number | null } {
  const sorted = bySize(entries(group), (e) => e.bytes);
  return { rows: sorted.flatMap((e) => [e.row, ...e.children]), bytes: sum(sorted.map((e) => e.bytes)) };
}

/**
 * Largest group first, children under their parent. Rows before the first header stay first; each
 * header moves with the rows after it up to the next header, and the header groups go by summed bytes.
 */
function sortBlock(rows: RowView[]): RowView[] {
  const lead: RowView[] = [];
  const groups: { header: RowView; rows: RowView[] }[] = [];
  for (const row of rows) {
    if (row.kind === 'header') groups.push({ header: row, rows: [] });
    else (groups.length === 0 ? lead : groups[groups.length - 1]!.rows).push(row);
  }
  const built = groups.map((g) => ({ header: g.header, ...flatten(g.rows) }));
  return [...flatten(lead).rows, ...bySize(built, (g) => g.bytes).flatMap((g) => [g.header, ...g.rows])];
}

/**
 * The list in the order it is drawn. A list of categories (`group` rows) keeps the categories in the engine's order (the
 * menu beside it is in that order too) and sorts only inside each one; any other list is one block, largest first.
 */
export function sortRows(rows: RowView[]): RowView[] {
  if (!rows.some((r) => r.kind === 'group')) return sortBlock(rows);
  const out: RowView[] = [];
  let group: RowView | null = null;
  let block: RowView[] = [];
  const flush = (): void => { out.push(...(group ? [group] : []), ...sortBlock(block)); block = []; };
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
  name: 'minmax(0,1fr)', cell: 'fit-content(260px)', level: '96px', size: 'minmax(74px,max-content)',
};

/** The check column is a 34 px target, not the 14 px box: a click anywhere in it ticks the row. */
const CHECK_COLUMN = '34px';

/** The `grid-template-columns` of the header and every row: a check column first when rows have checks. */
export function gridTemplate(columns: ColumnView[], checks: boolean): string {
  return [...(checks ? [CHECK_COLUMN] : []), ...columns.map((c) => COLUMN_WIDTH[c.kind])].join(' ');
}

/** The ids of the rows that have children (they get a caret). */
export function rowParents(rows: RowView[]): Set<string> {
  return new Set(rows.flatMap((r) => (r.parent === null ? [] : [r.parent])));
}
