// The keys of the app's own (src/commands.ts), decided apart from the screens: which key an action puts on a command, the action a
// key asks of a view, the list's own keys, what the foot names, and the hold a confirmation keeps on Enter and Space. The commands
// themselves are Sigrid's `useCommand` (src/screens/commands.tsx); how Sigrid runs a key is its own test, and the browser proves the
// two together (smoke.mjs).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { actionCommandId, actionForKey, actionKeys, COMMAND, DIALOG_ARM_MS, dialogKeySwallowed, footerEntries, listKey } from '../src/commands.ts';
import type { ActionView, SectionId, SectionView } from '../src/types.ts';

const action = (id: string, key: string | null, enabled: boolean, label = id): ActionView => ({ id, label, key, enabled, primary: false });
function viewWith(section: SectionId, ...actions: ActionView[]): SectionView {
  return {
    section, rev: 1, job: null, status: { badge: 'Ready', text: '' }, nav: [], options: [], title: '', subtitle: '',
    columns: [], rows: [], bulk: false, empty: null, notices: [], actions, search: null, dialog: null, report: null, footer: null,
  };
}
const fixture = JSON.parse(readFileSync(new URL('../fixtures/views.json', import.meta.url), 'utf8')) as { views: Record<SectionId, Record<string, SectionView>> };

test('an action puts its key on a command: Enter (the focused button\'s own) and Esc (the Escape command) are not bound', () => {
  assert.equal(actionKeys(action('analyze', 'a', true)), 'a');
  assert.equal(actionKeys(action('scan', 'a', true)), 'a');
  assert.equal(actionKeys(action('clean', 'Enter', true)), undefined);
  assert.equal(actionKeys(action('cancel', 'Esc', true)), undefined);
  assert.equal(actionKeys(action('x', 'Escape', true)), undefined);
  assert.equal(actionKeys(action('x', null, true)), undefined);
  assert.equal(actionCommandId(action('analyze', 'a', true)), 'xiox.action.analyze');
});

test('actionForKey finds the enabled action with that key; Rust writes the Cancel key as Esc', () => {
  const view = viewWith('cleanup', action('analyze', 'a', true), action('clean', 'Enter', false), action('cancel', 'Esc', true));
  assert.equal(actionForKey(view, 'a')?.id, 'analyze');
  assert.equal(actionForKey(view, 'Enter'), null, 'Clean is disabled: Enter does nothing');
  assert.equal(actionForKey(view, 'Esc')?.id, 'cancel');
  assert.equal(actionForKey(viewWith('cleanup', action('cancel', 'Escape', true)), 'Esc')?.id, 'cancel', 'the key\'s own name is accepted too');
  assert.equal(actionForKey(viewWith('cleanup', action('cancel', 'Esc', false)), 'Esc'), null);
});

test('the list\'s own keys: arrows, Page, Home, End, Left and Right, Space and Enter; a held key presses nothing but arrows repeat', () => {
  assert.deepEqual(listKey({ key: 'ArrowDown' }), { kind: 'move', delta: 1 });
  assert.deepEqual(listKey({ key: 'ArrowUp' }), { kind: 'move', delta: -1 });
  assert.deepEqual(listKey({ key: 'PageDown' }), { kind: 'move', delta: 10 });
  assert.deepEqual(listKey({ key: 'PageUp' }), { kind: 'move', delta: -10 });
  assert.deepEqual(listKey({ key: 'Home' }), { kind: 'move', delta: -Infinity });
  assert.deepEqual(listKey({ key: 'End' }), { kind: 'move', delta: Infinity });
  assert.deepEqual(listKey({ key: 'ArrowRight' }), { kind: 'expand', open: true });
  assert.deepEqual(listKey({ key: 'ArrowLeft' }), { kind: 'expand', open: false });
  assert.deepEqual(listKey({ key: ' ' }), { kind: 'toggle' });
  assert.deepEqual(listKey({ key: 'Enter' }), { kind: 'primary' });
  assert.equal(listKey({ key: 'Enter', repeat: true }), null, 'one Enter is one press');
  assert.equal(listKey({ key: ' ', repeat: true }), null);
  assert.deepEqual(listKey({ key: 'ArrowDown', repeat: true }), { kind: 'move', delta: 1 });
  assert.equal(listKey({ key: 'ArrowDown', ctrlKey: true }), null, 'a chord is never the list\'s');
  assert.equal(listKey({ key: 'a' }), null);
  assert.equal(listKey({ key: 'Tab' }), null);
});

test('the footer lists the list\'s keys, the sort and each action with its key, from the view', () => {
  const entries = (section: SectionId, state: string): string => footerEntries(fixture.views[section][state] as SectionView)
    .map((entry) => (typeof entry === 'string' ? entry : `${entry.keys}:${entry.label}`)).join(' ');
  const analyzed = fixture.views.cleanup.analyzed as SectionView;
  const analyze = analyzed.actions.find((a) => a.key === 'a');
  const clean = analyzed.actions.find((a) => a.key === 'Enter');
  assert.ok(analyze && clean);
  assert.equal(entries('cleanup', 'analyzed'), `↑ ↓:Move Space:Tick ← →:Fold ${COMMAND.sort} ${actionCommandId(analyze)} Enter:${clean.label}`);
  assert.ok(!entries('cleanup', 'idle').includes('Enter:'), 'Enter is not named while Clean is disabled');
  assert.ok(!entries('cleanup', 'cleaning').includes('Esc'), 'Esc is not in the footer, busy or not');
  assert.equal(footerEntries(viewWith('cleanup')).length, 4, 'with no action: the list\'s keys and the sort');
});

test('a dialog button ignores held keys and keys within 300 ms of the dialog opening, never Escape or Tab', () => {
  assert.equal(DIALOG_ARM_MS, 300);
  assert.equal(dialogKeySwallowed({ key: 'Enter', repeat: true }, 5000), true);
  assert.equal(dialogKeySwallowed({ key: ' ', repeat: true }, 5000), true);
  assert.equal(dialogKeySwallowed({ key: 'Enter' }, 0), true);
  assert.equal(dialogKeySwallowed({ key: 'Enter', repeat: false }, 299), true);
  assert.equal(dialogKeySwallowed({ key: 'Enter' }, 300), false);
  assert.equal(dialogKeySwallowed({ key: ' ' }, 300), false);
  for (const age of [0, 100, 299, 300, 5000]) {
    assert.equal(dialogKeySwallowed({ key: 'Escape' }, age), false, `Escape at ${age} ms`);
    assert.equal(dialogKeySwallowed({ key: 'Tab' }, age), false, `Tab at ${age} ms`);
  }
});
