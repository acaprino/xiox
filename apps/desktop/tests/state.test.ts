import { test } from 'node:test';
import assert from 'node:assert/strict';
import { acceptView, bulkCheck, bulkLocked, createGate, createInflight, createUi, currentFocus, foldedGroups, LONG_GROUP, moveFocus, noticeKey, SERVICES_POLL_MS, shouldPollServices, shownRows } from '../src/state.ts';
import { visibleRows } from '../src/rows.ts';
import type { DialogView, RowView, SectionId, SectionView } from '../src/types.ts';

function row(id: string, extra: Partial<RowView> = {}): RowView {
  return {
    id, kind: 'item', parent: null, check: 'off', enabled: true, name: id, sub: null, level: null, cells: [],
    bytes: null, sizeText: null, muted: false, why: null, detail: [], lines: [], ...extra,
  };
}
const header = (id: string): RowView => row(id, { kind: 'header', check: 'none' });

function view(section: SectionId, rev: number, rows: RowView[] = []): SectionView {
  return {
    section, rev, job: null, status: { badge: 'b', text: 't' }, nav: [], options: [], title: 'T', subtitle: '',
    columns: [], rows, bulk: false, empty: null, notices: [], actions: [], search: null, dialog: null, report: null, footer: null,
  };
}

test('an older view never replaces a newer one', () => {
  const ui = createUi();
  assert.equal(acceptView(ui, view('cleanup', 5)), true);
  assert.equal(acceptView(ui, view('cleanup', 4)), false);
  assert.equal(ui.views.cleanup?.rev, 5);
  assert.equal(acceptView(ui, view('cleanup', 5)), false);
  assert.equal(acceptView(ui, view('cleanup', 6)), true);
  assert.equal(ui.views.cleanup?.rev, 6);
  // Revisions are per section.
  assert.equal(acceptView(ui, view('registry', 1)), true);
});

test('moveFocus skips header rows and clamps at the ends', () => {
  const rows = [header('h1'), row('a'), row('b'), header('h2'), row('c')];
  assert.equal(moveFocus(rows, 'a', 1), 'b');
  assert.equal(moveFocus(rows, 'b', 1), 'c');
  assert.equal(moveFocus(rows, 'c', 1), 'c');
  assert.equal(moveFocus(rows, 'c', -1), 'b');
  assert.equal(moveFocus(rows, 'a', -1), 'a');
  assert.equal(moveFocus(rows, 'a', -5), 'a');
  assert.equal(moveFocus(rows, null, 1), 'a');
  assert.equal(moveFocus(rows, null, -1), 'c');
  assert.equal(moveFocus(rows, 'gone', 1), 'a');
  assert.equal(moveFocus([header('h')], null, 1), null);
  assert.equal(moveFocus([], 'a', 1), null);
});

test('children of a collapsed parent are not visible', () => {
  const rows = [row('p'), row('c', { kind: 'child', parent: 'p' })];
  const ui = createUi();
  assert.deepEqual(visibleRows(rows, ui.expanded).map((r) => r.id), ['p']);
  ui.expanded.add('p');
  assert.deepEqual(visibleRows(rows, ui.expanded).map((r) => r.id), ['p', 'c']);
});

test('the focus falls back to the first row that can be focused', () => {
  const ui = createUi();
  const v = view('cleanup', 1, [header('h'), row('a'), row('c', { kind: 'child', parent: 'a' }), row('b')]);
  acceptView(ui, v);
  assert.equal(currentFocus(ui, v), 'a');
  ui.focus.cleanup = 'b';
  assert.equal(currentFocus(ui, v), 'b');
  // A focused child that is collapsed is not on screen: back to the first row.
  ui.focus.cleanup = 'c';
  assert.equal(currentFocus(ui, v), 'a');
  ui.expanded.add('a');
  assert.equal(currentFocus(ui, v), 'c');
  assert.equal(currentFocus(ui, view('cleanup', 2, [])), null);
});

test('a stale response is dropped', async () => {
  const gate = createGate();
  const shown: string[] = [];
  const slow = Promise.withResolvers<string>();
  const fast = Promise.withResolvers<string>();
  const first = gate.run(() => slow.promise, (value) => { shown.push(value); });
  const second = gate.run(() => fast.promise, (value) => { shown.push(value); });
  fast.resolve('registry');
  slow.resolve('cleanup');
  assert.equal(await second, true);
  assert.equal(await first, false);
  assert.deepEqual(shown, ['registry']);
});

test('a stale failure is dropped too and a current one is reported', async () => {
  const gate = createGate();
  const errors: string[] = [];
  const slow = Promise.withResolvers<string>();
  const first = gate.run(() => slow.promise, () => {}, (e) => { errors.push(`first ${e}`); });
  const second = gate.run(() => Promise.reject('boom'), () => {}, (e) => { errors.push(`second ${e}`); });
  slow.reject('late');
  await Promise.all([first, second]);
  assert.deepEqual(errors, ['second boom']);
});

test('services refresh only while shown, visible and without a dialog', () => {
  const dialog: DialogView = {
    id: 'confirm', title: 'Disable 3 services?', text: [], summary: [], permanent: false, rows: [], warningsTitle: null, warnings: [], cautionTitle: null,
    cautions: [], ack: null, error: null, buttons: [],
  };
  const ui = createUi();
  // Not the open section: nothing to refresh, whatever else holds.
  ui.section = 'cleanup';
  assert.equal(shouldPollServices(ui, true), false, 'another section is open');
  ui.section = 'services';
  assert.equal(shouldPollServices(ui, true), true, 'the section is shown, the window visible, no view yet means no dialog');
  assert.equal(shouldPollServices(ui, false), false, 'the window is hidden');
  acceptView(ui, view('services', 1));
  assert.equal(shouldPollServices(ui, true), true, 'a view without a dialog');
  assert.equal(shouldPollServices(ui, false), false);
  // A dialog describes a selection: the list does not move under it.
  acceptView(ui, { ...view('services', 2), dialog });
  assert.equal(shouldPollServices(ui, true), false, 'a dialog is open');
  acceptView(ui, view('services', 3));
  assert.equal(shouldPollServices(ui, true), true, 'the dialog closed');
  // Another section's dialog is not ours.
  acceptView(ui, { ...view('developers', 1), dialog });
  assert.equal(shouldPollServices(ui, true), true);
  assert.equal(SERVICES_POLL_MS, 2000, 'the RAM meter is read every 2 s, as the old window repainted');
});

test('an identical intent is dropped while one is in flight, and goes again after the answer', () => {
  const flight = createInflight();
  const confirm = { kind: 'confirm' };
  assert.equal(flight.begin('cleanup', confirm), true, 'the first one goes');
  assert.equal(flight.begin('cleanup', { kind: 'confirm' }), false, 'the same intent (a double click) is dropped');
  assert.equal(flight.begin('registry', confirm), true, 'the same words to another section are another intent');
  assert.equal(flight.begin('cleanup', { kind: 'toggle', row: 'cat:1' }), true, 'a different intent is not held back');
  assert.equal(flight.begin('cleanup', { kind: 'toggle', row: 'cat:2' }), true, 'nor the same kind on another row');
  flight.end('cleanup', confirm);
  assert.equal(flight.begin('cleanup', confirm), true, 'the answer came back: it may be sent again');
  flight.end('cleanup', confirm);
  flight.end('cleanup', confirm); // a second end is harmless
  assert.equal(flight.begin('cleanup', confirm), true);
});

test('All and None are locked while a job runs', () => {
  const idle = { job: null } as unknown as SectionView;
  const busy = { job: { label: 'Scanning the registry…', fraction: 0.3, detail: null } } as unknown as SectionView;
  assert.equal(bulkLocked(idle), false);
  assert.equal(bulkLocked(busy), true);
});

test('a notice is remembered by its text, apart from the rows', () => {
  assert.equal(noticeKey('Fixed 2 issues.'), 'notice:Fixed 2 issues.');
  assert.notEqual(noticeKey('a'), noticeKey('b'));
});

const group = (id: string, check: RowView['check'] = 'off', extra: Partial<RowView> = {}): RowView => row(id, { kind: 'group', check, ...extra });

test('a long category starts folded, a short one open, and the user choice wins', () => {
  const ui = createUi();
  const many = Array.from({ length: LONG_GROUP + 1 }, (_, i) => row(`l${i}`));
  const v = view('cleanup', 1, [group('g:short'), row('s1'), group('g:long'), ...many]);
  assert.deepEqual([...foldedGroups(ui, v)], ['g:long']);
  assert.deepEqual(shownRows(ui, v).map((r) => r.id), ['g:short', 's1', 'g:long']);
  ui.fold.set('g:long', false);
  ui.fold.set('g:short', true);
  assert.deepEqual([...foldedGroups(ui, v)], ['g:short']);
  assert.equal(shownRows(ui, v).length, 2 + LONG_GROUP + 1);
});

test('a search opens every category, so that no match stays folded', () => {
  const ui = createUi();
  const many = Array.from({ length: LONG_GROUP + 1 }, (_, i) => row(`l${i}`));
  const v = { ...view('cleanup', 1, [group('g:long'), ...many]), search: { placeholder: 'Search', value: ' app ' } };
  assert.equal(foldedGroups(ui, v).size, 0);
  ui.fold.set('g:long', true);
  assert.equal(foldedGroups(ui, v).size, 0, 'even one the user folded');
});

test('the focus can rest on a category row, and moves past a folded category', () => {
  const ui = createUi();
  const v = view('cleanup', 1, [group('g:a'), row('a1'), group('g:b'), row('b1')]);
  acceptView(ui, v);
  assert.equal(currentFocus(ui, v), 'g:a');
  ui.fold.set('g:a', true);
  ui.focus.cleanup = 'a1';
  assert.equal(currentFocus(ui, v), 'g:a', 'the focused row is hidden: the first row takes over');
  assert.equal(moveFocus(shownRows(ui, v), 'g:a', 1), 'g:b');
});

test('the check box at the top of the list sums the categories, or the rows of a plain list', () => {
  assert.equal(bulkCheck(view('cleanup', 1, [group('a', 'on'), row('x'), group('b', 'on')])), 'on');
  assert.equal(bulkCheck(view('cleanup', 1, [group('a', 'on'), group('b', 'off')])), 'mixed');
  assert.equal(bulkCheck(view('cleanup', 1, [group('a', 'mixed'), group('b', 'off')])), 'mixed');
  assert.equal(bulkCheck(view('cleanup', 1, [group('a', 'off'), group('b', 'off')])), 'off');
  assert.equal(bulkCheck(view('cleanup', 1, [group('a', 'none'), group('b', 'on', { enabled: false })])), 'none', 'a category with nothing to tick does not count');
  assert.equal(bulkCheck(view('registry', 1, [row('i:0', { check: 'on' }), row('i:1', { check: 'on' })])), 'on');
  assert.equal(bulkCheck(view('registry', 1, [row('i:0', { check: 'on' }), row('i:1', { check: 'off' })])), 'mixed');
  assert.equal(bulkCheck(view('developers', 1, [row('p', { check: 'mixed' }), row('c', { kind: 'child', parent: 'p', check: 'off' })])), 'mixed', 'children do not count');
  assert.equal(bulkCheck(view('services', 1, [])), 'none');
});
