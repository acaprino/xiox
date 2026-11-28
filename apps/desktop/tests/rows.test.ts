import { test } from 'node:test';
import assert from 'node:assert/strict';
import { activeSort, cycleSort, groupSizes, headerBytes, nextSort, orderFor, rowDepths, rowParents, sortRows, visibleRows } from '../src/rows.ts';
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

test('children of a collapsed parent are not visible', () => {
  const rows = [row('p1', 100), child('c1', 'p1', 10), row('p2', 50), child('c2', 'p2', 5)];
  assert.deepEqual(ids(visibleRows(rows, new Set())), ['p1', 'p2']);
  assert.deepEqual(ids(visibleRows(rows, new Set(['p2']))), ['p1', 'p2', 'c2']);
  assert.deepEqual(ids(visibleRows(rows, new Set(['p1', 'p2']))), ['p1', 'c1', 'p2', 'c2']);
});

test('a project inside a project is always in sight, after the folders to clean', () => {
  const rows = [
    header('H'),
    row('P', 40, { parent: 'H' }), child('A', 'P', 5), row('S', 20, { parent: 'P' }), child('B', 'S', 9),
    row('Q', 30, { parent: 'H' }),
  ];
  assert.deepEqual([...rowDepths(rows)], [['H', 0], ['P', 1], ['A', 2], ['S', 2], ['B', 3], ['Q', 1]]);
  const sorted = sortRows(rows);
  assert.deepEqual(ids(sorted), ['H', 'P', 'A', 'S', 'B', 'Q'], 'the small folder to clean still comes before the larger sub-project');
  assert.deepEqual(ids(visibleRows(sorted, new Set())), ['H', 'P', 'S', 'Q']);
  assert.deepEqual(ids(visibleRows(sorted, new Set(['P']))), ['H', 'P', 'A', 'S', 'Q']);
  assert.deepEqual(ids(visibleRows(sorted, new Set(), new Set(['H']))), ['H'], 'a folded folder hides them all');
  assert.equal(headerBytes(rows, 'H'), 70, 'the sub-project is in its parent, not counted twice');
});

test('rowParents lists the rows that have children', () => {
  const rows = [row('p1', 100), child('c1', 'p1', 10), child('c2', 'p1', 90), row('p2', 300)];
  assert.deepEqual([...rowParents(rows)], ['p1']);
  assert.equal(rowParents([row('a', 1)]).size, 0);
});

const group = (id: string): RowView => row(id, null, { kind: 'group' });

test('categories keep the engine order and are sorted only inside', () => {
  const rows = [
    group('g:a'), row('a1', 10), row('a2', 90),
    group('g:b'), row('b1', 500),
    group('g:c'), header('h:small'), row('c1', 1), header('h:big'), row('c2', 7), row('c3', 70),
  ];
  assert.deepEqual(ids(sortRows(rows)), ['g:a', 'a2', 'a1', 'g:b', 'b1', 'g:c', 'h:big', 'c3', 'c2', 'h:small', 'c1']);
});

test('a folded category shows its own row and none of its rows', () => {
  const rows = [group('g:a'), row('a1', 1), group('g:b'), row('b1', 2), child('b2', 'b1', 1)];
  assert.deepEqual(ids(visibleRows(rows, new Set(), new Set(['g:a']))), ['g:a', 'g:b', 'b1']);
  assert.deepEqual(ids(visibleRows(rows, new Set(['b1']), new Set(['g:b']))), ['g:a', 'a1', 'g:b']);
  assert.deepEqual(ids(visibleRows(rows, new Set(), new Set())), ['g:a', 'a1', 'g:b', 'b1']);
});

test('groupSizes counts the items of a category, not its sub-headings', () => {
  const rows = [group('g:a'), header('h'), row('a1', 1), row('a2', 2), group('g:b'), row('b1', 3), child('b2', 'b1', 1), group('g:c')];
  assert.deepEqual([...groupSizes(rows)], [['g:a', 2], ['g:b', 2], ['g:c', 0]]);
  assert.equal(groupSizes([row('x', 1)]).size, 0);
});

const COLUMNS: ColumnView[] = [
  { title: 'item', kind: 'name' }, { title: 'files', kind: 'cell' }, { title: 'date', kind: 'cell' },
  { title: 'level', kind: 'level' }, { title: 'size', kind: 'size' },
];
const by = (column: string, desc: boolean) => orderFor(COLUMNS, { column, desc });

test('by name, as a person reads it, both ways', () => {
  const rows = [row('file10', 1), row('File9', 2), row('alpha', 3)];
  assert.deepEqual(ids(sortRows(rows, by('item', false))), ['alpha', 'File9', 'file10']);
  assert.deepEqual(ids(sortRows(rows, by('item', true))), ['file10', 'File9', 'alpha']);
});

test('by size, both ways, rows without a size last either way', () => {
  const rows = [row('none', null), row('small', 10), row('big', 900)];
  assert.deepEqual(ids(sortRows(rows, by('size', true))), ['big', 'small', 'none']);
  assert.deepEqual(ids(sortRows(rows, by('size', false))), ['small', 'big', 'none']);
});

test('a cell with grouped digits is a number, a dash is nothing, a date is its text', () => {
  const rows = [
    row('a', 0, { cells: ['2,311', '2026-09-04'] }), row('b', 0, { cells: ['18,204', '2026-08-21'] }),
    row('c', 0, { cells: ['—', '—'] }), row('d', 0, { cells: ['96', '2026-10-15'] }),
  ];
  assert.deepEqual(ids(sortRows(rows, by('files', true))), ['b', 'a', 'd', 'c']);
  assert.deepEqual(ids(sortRows(rows, by('files', false))), ['d', 'a', 'b', 'c']);
  assert.deepEqual(ids(sortRows(rows, by('date', false))), ['b', 'a', 'd', 'c']);
});

test('by level, what needs care first', () => {
  const rows = [row('r', 0, { level: 'recommended' }), row('c', 0, { level: 'caution' }), row('n', 0), row('o', 0, { level: 'optional' })];
  assert.deepEqual(ids(sortRows(rows, by('level', true))), ['c', 'o', 'r', 'n']);
});

test('the structure holds in any order: categories in engine order, headers with their rows, children under the parent', () => {
  const rows = [
    row('g:b', null, { kind: 'group' }), row('zeta', 5), row('beta', 9),
    row('g:a', null, { kind: 'group' }), header('h:z'), row('z1', 1), header('h:a'), row('a2', 2), row('a1', 1),
    row('p', 3), child('p:y', 'p', 1), child('p:x', 'p', 2),
  ];
  assert.deepEqual(ids(sortRows(rows, by('item', false))), ['g:b', 'beta', 'zeta', 'g:a', 'h:a', 'a1', 'a2', 'p', 'p:x', 'p:y', 'h:z', 'z1']);
});

test('headers follow their first row for a column of their own rows', () => {
  const rows = [header('h:old'), row('o1', 0, { cells: ['', '2026-01-01'] }), header('h:new'), row('n1', 0, { cells: ['', '2026-10-01'] })];
  assert.deepEqual(ids(sortRows(rows, by('date', true))), ['h:new', 'n1', 'h:old', 'o1']);
});

test('the header marks the size column by default, a click turns it round or picks another in its natural way', () => {
  assert.deepEqual(activeSort(COLUMNS, null), { column: 'size', desc: true });
  assert.deepEqual(activeSort([{ title: 'issue', kind: 'name' }], null), null, 'no size column: the engine order, no arrow');
  assert.deepEqual(activeSort(COLUMNS, { column: 'gone', desc: false }), { column: 'size', desc: true }, 'a column of another category');
  assert.deepEqual(nextSort(COLUMNS, null, 'size'), { column: 'size', desc: false });
  assert.deepEqual(nextSort(COLUMNS, null, 'item'), { column: 'item', desc: false });
  assert.deepEqual(nextSort(COLUMNS, { column: 'item', desc: false }, 'item'), { column: 'item', desc: true });
  assert.deepEqual(nextSort(COLUMNS, null, 'level'), { column: 'level', desc: true });
});

test('the keyboard cycles the columns in their natural way and turns the active one round', () => {
  assert.deepEqual(cycleSort(COLUMNS, null, false), { column: 'item', desc: false }, 'after size comes the first column');
  assert.deepEqual(cycleSort(COLUMNS, { column: 'item', desc: false }, false), { column: 'files', desc: false });
  assert.deepEqual(cycleSort(COLUMNS, null, true), { column: 'size', desc: false });
  assert.equal(orderFor(COLUMNS, null).kind, 'size');
});

test('each row sits one level under its heading or parent, a sub-heading one level inside its category', () => {
  const grouped = [
    row('g:a', null, { kind: 'group' }), row('a1', 1), header('h:s'), row('s1', 1), row('p', 1), child('p:c', 'p', 1),
    row('g:b', null, { kind: 'group' }), row('b1', 1),
  ];
  assert.deepEqual([...rowDepths(grouped)], [['g:a', 0], ['a1', 1], ['h:s', 1], ['s1', 2], ['p', 2], ['p:c', 3], ['g:b', 0], ['b1', 1]]);
  const plain = [row('lead', 1), header('h:x'), row('x1', 1), row('q', 1), child('q:c', 'q', 1)];
  assert.deepEqual([...rowDepths(plain)], [['lead', 0], ['h:x', 0], ['x1', 1], ['q', 1], ['q:c', 2]]);
});

/** The projects' tree: D:\ holds the folder `work` and the project `top`; `work` holds `a` (two folders to clean) and `b`. */
const folder = (id: string, parent: string | null): RowView => row(id, null, { kind: 'header', parent });
const TREE = [
  folder('h:d', null),
  folder('h:work', 'h:d'), row('a', 30, { parent: 'h:work' }), child('a:1', 'a', 10), child('a:2', 'a', 20), row('b', 50, { parent: 'h:work' }),
  row('top', 40, { parent: 'h:d' }),
  folder('h:e', null), row('e', 5, { parent: 'h:e' }),
];

test('a folder sits one level inside the folder above it, its projects one level inside it', () => {
  assert.deepEqual([...rowDepths(TREE)], [['h:d', 0], ['h:work', 1], ['a', 2], ['a:1', 3], ['a:2', 3], ['b', 2], ['top', 1], ['h:e', 0], ['e', 1]]);
});

test('the tree of the projects sorts inside each folder and a folder goes by what it holds', () => {
  assert.deepEqual(ids(sortRows(TREE)), ['h:d', 'h:work', 'b', 'a', 'a:2', 'a:1', 'top', 'h:e', 'e'], 'work (80) before top (40), b before a');
  const byName = sortRows(TREE, by('item', false));
  assert.deepEqual(ids(byName), ['h:d', 'h:work', 'a', 'a:1', 'a:2', 'b', 'top', 'h:e', 'e'], 'h:work before top by name; folders by their own name');
});

test('a folder adds up the projects inside it and inside its folders, not their folders to clean twice', () => {
  assert.equal(headerBytes(TREE, 'h:d'), 120);
  assert.equal(headerBytes(TREE, 'h:work'), 80);
  assert.equal(headerBytes(TREE, 'h:e'), 5);
  const flat = [header('h:x'), row('x1', 10), child('x1:c', 'x1', 4), row('x2', 6), header('h:y'), row('y1', 1)];
  assert.equal(headerBytes(flat, 'h:x'), 16, 'a plain sub-heading: its top rows up to the next sub-heading');
});

test('a folder is open unless folded, and folding it hides everything inside it', () => {
  const all = new Set(['a']);
  assert.deepEqual(ids(visibleRows(TREE, all)), ids(TREE), 'folders start open; a project shows its folders when expanded');
  assert.deepEqual(ids(visibleRows(TREE, new Set())), ['h:d', 'h:work', 'a', 'b', 'top', 'h:e', 'e'], 'a project starts collapsed');
  assert.deepEqual(ids(visibleRows(TREE, all, new Set(['h:work']))), ['h:d', 'h:work', 'top', 'h:e', 'e']);
  assert.deepEqual(ids(visibleRows(TREE, all, new Set(['h:d']))), ['h:d', 'h:e', 'e']);
});
