import { test } from 'node:test';
import assert from 'node:assert/strict';
import { gridTemplate, rowParents, sortRows, visibleRows } from '../src/rows.ts';
import type { ColumnView, RowView } from '../src/types.ts';

function row(id: string, bytes: number | null, extra: Partial<RowView> = {}): RowView {
  return {
    id, kind: 'item', parent: null, check: 'off', enabled: true, name: id, sub: null, level: null, cells: [],
    bytes, sizeText: null, muted: false, why: null, detail: [], lines: [], ...extra,
  };
}
const child = (id: string, parent: string, bytes: number | null): RowView => row(id, bytes, { kind: 'child', parent });
const header = (id: string): RowView => row(id, null, { kind: 'header', check: 'none' });
const ids = (rows: RowView[]): string[] => rows.map((r) => r.id);

test('sortRows puts the largest group first and keeps children with their parent', () => {
  const rows = [row('p1', 100), child('c1', 'p1', 10), child('c2', 'p1', 90), row('p2', 300)];
  assert.deepEqual(ids(sortRows(rows)), ['p2', 'p1', 'c2', 'c1']);
});

test('sortRows keeps the engine order without sizes', () => {
  const rows = [row('b', null), child('b1', 'b', null), row('a', null), row('c', null)];
  assert.deepEqual(ids(sortRows(rows)), ['b', 'b1', 'a', 'c']);
});

test('sortRows does not touch its input and a parent without bytes counts its children', () => {
  const rows = [row('x', 50), row('p', null), child('p1', 'p', 70)];
  const sorted = sortRows(rows);
  assert.deepEqual(ids(rows), ['x', 'p', 'p1']);
  assert.deepEqual(ids(sorted), ['p', 'p1', 'x']);
});

test('headers keep their rows', () => {
  const rows = [
    row('lead', 5),
    header('h:small'), row('s1', 10), row('s2', 20),
    header('h:big'), row('b1', 100), row('b2', 400),
  ];
  // The rows before the first header stay first; header groups go by summed bytes, rows inside by size.
  assert.deepEqual(ids(sortRows(rows)), ['lead', 'h:big', 'b2', 'b1', 'h:small', 's2', 's1']);
});

test('rows without a size follow the sized ones in engine order', () => {
  const rows = [row('n1', null), row('s', 7), row('n2', null)];
  assert.deepEqual(ids(sortRows(rows)), ['s', 'n1', 'n2']);
});

test('gridTemplate adds the check column only when rows have checks', () => {
  const columns: ColumnView[] = [
    { title: 'Item', kind: 'name' }, { title: 'Level', kind: 'level' }, { title: 'Size', kind: 'size' },
  ];
  assert.equal(gridTemplate(columns, true), '16px minmax(0,1fr) 96px minmax(74px,max-content)');
  assert.equal(gridTemplate(columns, false), 'minmax(0,1fr) 96px minmax(74px,max-content)');
  const cells: ColumnView[] = [{ title: 'Service', kind: 'name' }, { title: 'Startup', kind: 'cell' }, { title: 'State', kind: 'cell' }];
  assert.equal(gridTemplate(cells, false), 'minmax(0,1fr) fit-content(260px) fit-content(260px)');
});

test('children of a collapsed parent are not visible', () => {
  const rows = [row('p1', 100), child('c1', 'p1', 10), row('p2', 50), child('c2', 'p2', 5)];
  assert.deepEqual(ids(visibleRows(rows, new Set())), ['p1', 'p2']);
  assert.deepEqual(ids(visibleRows(rows, new Set(['p2']))), ['p1', 'p2', 'c2']);
  assert.deepEqual(ids(visibleRows(rows, new Set(['p1', 'p2']))), ['p1', 'c1', 'p2', 'c2']);
});

test('rowParents lists the rows that have children', () => {
  const rows = [row('p1', 100), child('c1', 'p1', 10), child('c2', 'p1', 90), row('p2', 300)];
  assert.deepEqual([...rowParents(rows)], ['p1']);
  assert.equal(rowParents([row('a', 1)]).size, 0);
});
