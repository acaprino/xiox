import { test } from 'node:test';
import assert from 'node:assert/strict';
import { actionIntent, bulkIntent, buttonIntent, optionIntent, searchIntent, selectIntent, toggleIntent } from '../src/intents.ts';
import type { OptionView } from '../src/types.ts';

const toggle = (id: string): OptionView => ({ kind: 'toggle', id, label: id, on: false, enabled: true, hint: null });
const choice = (id: string): OptionView => ({ kind: 'choice', id, label: id, value: '', choices: [], enabled: true });
const number = (id: string): OptionView => ({ kind: 'number', id, label: id, value: 30, min: 1, max: 365, enabled: true });
const list = (id: string): OptionView => ({
  kind: 'list', id, label: id, items: [], selected: null, enabled: true, addLabel: 'Add', removeLabel: 'Remove', empty: '',
});
const buttonOption = (id: string): OptionView => ({ kind: 'button', id, label: id, enabled: true, hint: null });

test('actions, buttons and options map to their section\'s intent', () => {
  // Top bar actions.
  assert.deepEqual(actionIntent('cleanup', 'analyze'), { kind: 'analyze' });
  assert.deepEqual(actionIntent('developers', 'analyze'), { kind: 'analyze' });
  assert.deepEqual(actionIntent('registry', 'scan'), { kind: 'scan' });
  assert.deepEqual(actionIntent('cleanup', 'clean'), { kind: 'openConfirm' });
  assert.deepEqual(actionIntent('developers', 'clean'), { kind: 'openConfirm' });
  assert.deepEqual(actionIntent('registry', 'fix'), { kind: 'openFix' });
  assert.deepEqual(actionIntent('cleanup', 'cancel'), { kind: 'cancel' });
  assert.deepEqual(actionIntent('developers', 'cancel'), { kind: 'cancel' });
  // An id that belongs to another section, or to none, sends nothing.
  assert.equal(actionIntent('registry', 'analyze'), null);
  assert.equal(actionIntent('cleanup', 'scan'), null);
  assert.equal(actionIntent('cleanup', 'fix'), null);
  assert.equal(actionIntent('registry', 'clean'), null);
  assert.equal(actionIntent('developers', 'nothing'), null);

  // Dialog buttons.
  for (const s of ['cleanup', 'registry', 'developers'] as const) assert.deepEqual(buttonIntent(s, 'cancel'), { kind: 'closeDialog' });
  assert.deepEqual(buttonIntent('cleanup', 'continue'), { kind: 'confirm' });
  assert.deepEqual(buttonIntent('developers', 'clean'), { kind: 'confirm' });
  assert.deepEqual(buttonIntent('developers', 'permanent'), { kind: 'permanent' });
  assert.deepEqual(buttonIntent('registry', 'backup'), { kind: 'fix', backup: true });
  assert.deepEqual(buttonIntent('registry', 'nobackup'), { kind: 'fix', backup: false });
  assert.equal(buttonIntent('developers', 'continue'), null);
  assert.equal(buttonIntent('cleanup', 'clean'), null);
  assert.equal(buttonIntent('registry', 'continue'), null);
  assert.equal(buttonIntent('cleanup', 'permanent'), null);
  assert.equal(buttonIntent('cleanup', 'nothing'), null);

  // Options.
  assert.deepEqual(optionIntent('cleanup', toggle('onlyOld'), true), { kind: 'setOnlyOld', on: true });
  assert.deepEqual(optionIntent('cleanup', toggle('onlyOld'), false), { kind: 'setOnlyOld', on: false });
  assert.deepEqual(optionIntent('cleanup', buttonOption('keepList'), null), { kind: 'openKeepList' });
  assert.deepEqual(optionIntent('registry', toggle('cat:2'), true), { kind: 'toggleCategory', index: 2 });
  assert.deepEqual(optionIntent('registry', toggle('cat:12'), false), { kind: 'toggleCategory', index: 12 });
  assert.equal(optionIntent('registry', toggle('cat:x'), true), null);
  assert.equal(optionIntent('registry', toggle('cat:'), true), null);
  assert.deepEqual(optionIntent('developers', number('inactiveDays'), 45), { kind: 'setInactiveDays', days: 45 });
  assert.equal(optionIntent('developers', number('inactiveDays'), Number.NaN), null);
  assert.equal(optionIntent('developers', number('inactiveDays'), 'x'), null);
  const removeMode = choice('removeMode');
  assert.deepEqual(optionIntent('developers', removeMode, 'permanent'), { kind: 'setRemoveMode', mode: 'permanent' });
  assert.deepEqual(optionIntent('developers', removeMode, 'recycle'), { kind: 'setRemoveMode', mode: 'recycle' });
  assert.equal(optionIntent('developers', removeMode, 'delete'), null);
  const layout = choice('layout');
  assert.deepEqual(optionIntent('developers', layout, 'flat'), { kind: 'setLayout', layout: 'flat' });
  assert.deepEqual(optionIntent('developers', layout, 'tree'), { kind: 'setLayout', layout: 'tree' });
  assert.equal(optionIntent('developers', layout, 'grid'), null);
  assert.deepEqual(optionIntent('developers', toggle('scanRoots'), true), { kind: 'setScanRoots', on: true });
  assert.deepEqual(optionIntent('developers', list('roots'), 1), { kind: 'selectRoot', index: 1 });
  assert.deepEqual(optionIntent('developers', list('roots'), null), { kind: 'selectRoot', index: null });
  assert.deepEqual(optionIntent('developers', list('roots'), 'remove'), { kind: 'removeRoot' });
  assert.equal(optionIntent('developers', list('roots'), 'add'), null);
  // Unknown ids, ids of another section and text options send nothing.
  assert.equal(optionIntent('cleanup', toggle('nothing'), true), null);
  assert.equal(optionIntent('cleanup', toggle('scanRoots'), true), null);
  assert.equal(optionIntent('registry', toggle('onlyOld'), true), null);
  assert.equal(optionIntent('developers', toggle('onlyOld'), true), null);
  assert.equal(optionIntent('cleanup', { kind: 'text', text: 'x', tone: 'info', hint: null }, null), null);
});

test('rows, categories, search and bulk selection map to their section\'s intent', () => {
  for (const s of ['cleanup', 'registry', 'developers'] as const) {
    assert.deepEqual(toggleIntent(s, 'row:1'), { kind: 'toggle', row: 'row:1' });
  }
  assert.equal(selectIntent('cleanup', 'cat:browser'), null, 'Cleanup lists every category: its menu only scrolls');
  assert.deepEqual(selectIntent('developers', 'worktrees'), { kind: 'select', category: 'worktrees' });
  assert.deepEqual(selectIntent('developers', 'projects'), { kind: 'select', category: 'projects' });
  assert.deepEqual(selectIntent('developers', 'sessions'), { kind: 'select', category: 'sessions' });
  assert.equal(selectIntent('developers', 'cat:browser'), null);
  assert.equal(selectIntent('registry', 'x'), null);

  assert.deepEqual(searchIntent('cleanup', 'temp'), { kind: 'search', text: 'temp' });
  assert.equal(searchIntent('registry', 'x'), null);
  assert.equal(searchIntent('developers', 'x'), null);

  assert.deepEqual(bulkIntent('registry', true), { kind: 'selectAll', on: true });
  assert.deepEqual(bulkIntent('developers', false), { kind: 'selectAll', on: false });
  assert.deepEqual(bulkIntent('cleanup', true), { kind: 'selectAll', on: true });
});
