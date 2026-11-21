// The keys of the window, as commands of Imago's registry and the list's own keys (src/commands.ts). Every binding of
// docs/xiox-desktop.md "Keyboard" goes through `registry.match` with the key event a keyboard sends (the same on a US and an Italian layout for these keys), in a field and out
// of one, with a confirmation up and without. Not run locally when written (the owner's rule: CI runs it).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRegistry } from '@acaprino/imago/ui';
import {
  actionCommandId, actionCommands, actionForKey, actionKeys, COMMAND, DIALOG_ARM_MS, dialogKeySwallowed, footerEntries, listKey,
  registerCommands, type CommandApp,
} from '../src/commands.ts';
import type { ActionView, AnyIntent, SectionId, SectionView } from '../src/types.ts';

const action = (id: string, key: string | null, enabled: boolean, label = id): ActionView => ({ id, label, key, enabled, primary: false });
function viewWith(section: SectionId, ...actions: ActionView[]): SectionView {
  return {
    section, rev: 1, job: null, status: { badge: 'Ready', text: '' }, nav: [], options: [], title: '', subtitle: '',
    columns: [], rows: [], bulk: false, empty: null, notices: [], actions, search: null, dialog: null, report: null, footer: null,
  };
}
const fixture = JSON.parse(readFileSync(new URL('../fixtures/views.json', import.meta.url), 'utf8')) as { views: Record<SectionId, Record<string, SectionView>> };

/** What the commands did, and the state they ask about. */
function harness(state: { idle?: boolean; listed?: boolean; typing?: boolean } = {}) {
  const done: string[] = [];
  const sent: Array<[SectionId, AnyIntent | null]> = [];
  const flags = { idle: true, listed: true, typing: false, ...state };
  const app: CommandApp = {
    view: () => undefined,
    idle: () => flags.idle,
    listed: () => flags.idle && flags.listed,
    inTextField: () => flags.typing,
    openSection: (id) => done.push(`section:${id}`),
    openSettings: () => done.push('settings'),
    send: (section, intent) => { sent.push([section, intent]); },
    sort: (reverse) => done.push(reverse ? 'sort-reverse' : 'sort'),
    toggleInspector: (kind) => done.push(kind),
    selectAll: () => done.push('select-all'),
    escape: () => done.push('escape'),
  };
  const registry = createRegistry();
  registerCommands(registry, app);
  const actions = actionCommands(registry, app);
  return { app, registry, actions, flags, done, sent };
}

/** A key event as a keyboard types it. `target` stands for the focused element: an input is a text field. */
const press = (key: string, extra: { ctrlKey?: boolean; shiftKey?: boolean; altKey?: boolean; metaKey?: boolean; target?: unknown } = {}) =>
  ({ key, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...extra });
const field = { tagName: 'INPUT' };

test('digits 1 to 4 switch views, and not in a field or behind a confirmation', () => {
  const { registry, flags } = harness();
  assert.equal(registry.match(press('1'))?.id, 'xiox.section.cleanup');
  assert.equal(registry.match(press('2'))?.id, 'xiox.section.registry');
  assert.equal(registry.match(press('3'))?.id, 'xiox.section.developers');
  assert.equal(registry.match(press('4'))?.id, COMMAND.settingsTab);
  assert.equal(registry.match(press('0')), undefined);
  assert.equal(registry.match(press('1', { target: field })), undefined, 'typing a 1 in a field is typing');
  assert.equal(registry.match(press('1', { ctrlKey: true })), undefined, 'a chord is the browser\'s or the system\'s');
  assert.equal(registry.match(press('1', { altKey: true })), undefined);
  flags.idle = false;
  assert.equal(registry.match(press('1')), undefined, 'a confirmation keeps the keyboard');
});

test('the question mark opens Settings, with Shift as a keyboard types it, never in a field', () => {
  const { registry } = harness();
  assert.equal(registry.match(press('?', { shiftKey: true }))?.id, COMMAND.settings);
  assert.equal(registry.match(press('?'))?.id, undefined, 'a binding of ? without Shift never matches: the key is bound as Shift+?');
  assert.equal(registry.match(press('?', { shiftKey: true, target: field })), undefined);
  assert.equal(registry.match(press('/', { shiftKey: true })), undefined, 'the slash key is not it');
});

test('s sorts and Shift+S turns the order round, i opens Details and o the Options, only while the list is the page', () => {
  const { registry, flags } = harness();
  assert.equal(registry.match(press('s'))?.id, COMMAND.sort);
  assert.equal(registry.match(press('S', { shiftKey: true }))?.id, COMMAND.sortReverse);
  assert.equal(registry.match(press('i'))?.id, COMMAND.details);
  assert.equal(registry.match(press('o'))?.id, COMMAND.options);
  assert.equal(registry.match(press('s', { target: field })), undefined);
  assert.equal(registry.match(press('s', { ctrlKey: true })), undefined);
  flags.listed = false; // a report, or Settings, is on screen
  for (const key of ['s', 'i', 'o']) assert.equal(registry.match(press(key)), undefined, `${key} with no list`);
  assert.equal(registry.match(press('1'))?.id, 'xiox.section.cleanup', 'the views stay');
});

test('Escape runs in a field and behind a confirmation; Ctrl+A is the field\'s own in a field', () => {
  const { registry, flags } = harness();
  assert.equal(registry.match(press('Escape'))?.id, COMMAND.escape);
  assert.equal(registry.match(press('Escape', { target: field }))?.id, COMMAND.escape, 'it leaves the field');
  flags.idle = false;
  assert.equal(registry.match(press('Escape'))?.id, COMMAND.escape, 'it is the confirmation\'s Cancel');
  flags.idle = true;
  assert.equal(registry.match(press('a', { ctrlKey: true }))?.id, COMMAND.selectAll);
  assert.equal(registry.match(press('A', { ctrlKey: true }))?.id, COMMAND.selectAll, 'with Caps Lock');
  flags.typing = true;
  assert.equal(registry.match(press('a', { ctrlKey: true, target: field })), undefined, 'not available: the key is not taken');
});

test('the actions of the view are commands titled with Rust\'s label and bound to its key; Enter and Esc are not bound', () => {
  const { registry, actions, sent } = harness();
  const view = viewWith('cleanup', action('analyze', 'a', true, 'Analyze'), action('clean', 'Enter', true, 'Clean 8 items'), action('cancel', 'Esc', true, 'Cancel'));
  actions.sync(view);
  const analyze = registry.match(press('a'));
  assert.equal(analyze?.id, actionCommandId(view.actions[0] as ActionView));
  assert.equal(analyze?.title, 'Analyze');
  void analyze?.run();
  assert.deepEqual(sent, [['cleanup', { kind: 'analyze' }]]);
  assert.equal(registry.match(press('Enter')), undefined, 'Enter presses the focused button, no command takes it');
  assert.equal(registry.get(actionCommandId(view.actions[1] as ActionView))?.keys, undefined, 'but Clean is in the palette');
  assert.equal(registry.get(actionCommandId(view.actions[2] as ActionView))?.keys, undefined);
  assert.equal(registry.match(press('a', { target: field })), undefined);
  assert.equal(actionKeys(action('x', 'Escape', true)), undefined);
});

test('a disabled action answers no key, and the actions follow the view when it changes', () => {
  const { registry, actions } = harness();
  actions.sync(viewWith('cleanup', action('analyze', 'a', false)));
  assert.equal(registry.match(press('a')), undefined, 'disabled: Rust would refuse it');
  actions.sync(viewWith('registry', action('scan', 'a', true, 'Find issues')));
  assert.equal(registry.get('xiox.action.analyze'), undefined, 'the old view\'s command is gone');
  assert.equal(registry.match(press('a'))?.title, 'Find issues', 'a is Find issues in Registry');
  actions.sync(undefined);
  assert.equal(registry.match(press('a')), undefined);
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
  const has = (): boolean => true;
  const entries = (section: SectionId, state: string): string => footerEntries(fixture.views[section][state] as SectionView, has)
    .map((entry) => (typeof entry === 'string' ? entry : `${entry.keys}:${entry.title}`)).join(' ');
  const analyzed = fixture.views.cleanup.analyzed as SectionView;
  const analyze = analyzed.actions.find((a) => a.key === 'a');
  const clean = analyzed.actions.find((a) => a.key === 'Enter');
  assert.ok(analyze && clean);
  assert.equal(entries('cleanup', 'analyzed'), `ArrowUp ArrowDown:Move Space:Toggle ${COMMAND.sort} ${actionCommandId(analyze)} Enter:${clean.label}`);
  assert.ok(!entries('cleanup', 'cleaning').includes('Esc'), 'Esc is not in the footer, busy or not');
  assert.equal(footerEntries(viewWith('cleanup'), () => false).length, 2, 'without a sort command only the list\'s keys');
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
