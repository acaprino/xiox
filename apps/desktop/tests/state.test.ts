import { test } from 'node:test';
import assert from 'node:assert/strict';
import { acceptView, createGate, createUi, currentFocus, moveFocus, SERVICES_POLL_MS, shouldPollServices } from '../src/state.ts';
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
    id: 'confirm', title: 'Disable 3 services?', text: [], summary: [], rows: [], warningsTitle: null, warnings: [], cautionTitle: null,
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
  assert.equal(SERVICES_POLL_MS, 2000, 'the RAM meter is read every 2 s, as egui repainted');
});
