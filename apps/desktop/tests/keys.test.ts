import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { actionForKey, cycleFocus, footerHint, keyAction } from '../src/keys.ts';
import type { ActionView, SectionId, SectionView } from '../src/types.ts';

const free = { inField: false, dialog: false };
const typing = { inField: true, dialog: false };
const modal = { inField: false, dialog: true };

const action = (id: string, key: string | null, enabled: boolean): ActionView => ({ id, label: id, key, enabled, primary: false });
function viewWith(...actions: ActionView[]): SectionView {
  return {
    section: 'cleanup', rev: 1, job: null, status: { badge: 'Ready', text: '' }, nav: [], options: [], title: '', subtitle: '',
    columns: [], rows: [], bulk: false, empty: null, notices: [], actions, search: null, dialog: null, report: null, footer: null,
  };
}

const fixture = JSON.parse(readFileSync(new URL('../fixtures/views.json', import.meta.url), 'utf8')) as { views: Record<SectionId, Record<string, SectionView>> };

test('digits switch sections', () => {
  assert.deepEqual(keyAction({ key: '1' }, free), { kind: 'section', section: 'cleanup' });
  assert.deepEqual(keyAction({ key: '2' }, free), { kind: 'section', section: 'registry' });
  assert.deepEqual(keyAction({ key: '3' }, free), { kind: 'section', section: 'services' });
  assert.deepEqual(keyAction({ key: '4' }, free), { kind: 'section', section: 'developers' });
  assert.equal(keyAction({ key: '5' }, free), null);
  assert.equal(keyAction({ key: '0' }, free), null);
  // A chord is the browser's or the system's, never ours.
  assert.equal(keyAction({ key: '1', ctrlKey: true }, free), null);
  assert.equal(keyAction({ key: '2', altKey: true }, free), null);
  assert.equal(keyAction({ key: '3', metaKey: true }, free), null);
  assert.equal(keyAction({ key: 'a', ctrlKey: true }, free), null);
  assert.equal(keyAction({ key: 'Escape', ctrlKey: true }, free), null);
});

test('space toggles and arrows move outside fields and dialogs', () => {
  assert.deepEqual(keyAction({ key: ' ' }, free), { kind: 'toggle' });
  assert.deepEqual(keyAction({ key: 'ArrowDown' }, free), { kind: 'move', delta: 1 });
  assert.deepEqual(keyAction({ key: 'ArrowUp' }, free), { kind: 'move', delta: -1 });
  assert.deepEqual(keyAction({ key: 'PageDown' }, free), { kind: 'move', delta: 10 });
  assert.deepEqual(keyAction({ key: 'PageUp' }, free), { kind: 'move', delta: -10 });
  assert.deepEqual(keyAction({ key: 'Home' }, free), { kind: 'move', delta: -Infinity });
  assert.deepEqual(keyAction({ key: 'End' }, free), { kind: 'move', delta: Infinity });
  assert.deepEqual(keyAction({ key: 'ArrowRight' }, free), { kind: 'expand', open: true });
  assert.deepEqual(keyAction({ key: 'ArrowLeft' }, free), { kind: 'expand', open: false });
  assert.equal(keyAction({ key: 'x' }, free), null);
  assert.equal(keyAction({ key: 'Tab' }, free), null);
  assert.equal(keyAction({ key: 'A' }, free), null, 'only the lower-case letter is the key');
});

test("Enter and a press the view's action with that key, and nothing when it is disabled", () => {
  assert.deepEqual(keyAction({ key: 'a' }, free), { kind: 'action', key: 'a' });
  assert.deepEqual(keyAction({ key: 'Enter' }, free), { kind: 'action', key: 'Enter' });
  assert.deepEqual(keyAction({ key: 's' }, free), { kind: 'action', key: 's' });
  assert.deepEqual(keyAction({ key: 'r' }, free), { kind: 'action', key: 'r' });
  const view = viewWith(action('analyze', 'a', true), action('clean', 'Enter', false), action('cancel', 'Escape', true), action('scan', 's', true));
  assert.equal(actionForKey(view, 'a')?.id, 'analyze');
  assert.equal(actionForKey(view, 'Enter'), null, 'Clean is disabled: Enter does nothing');
  assert.equal(actionForKey(view, 'Esc')?.id, 'cancel', 'the fixtures write the Esc key as "Escape"');
  assert.equal(actionForKey(view, 's')?.id, 'scan');
  assert.equal(actionForKey(view, 'r'), null, 'no action has that key');
  assert.equal(actionForKey(viewWith(action('analyze', 'a', false)), 'a'), null);
  assert.equal(actionForKey(viewWith(action('x', null, true)), 'a'), null, 'an action without a key never answers a key');
});

test('Escape closes a dialog before it cancels a job', () => {
  assert.deepEqual(keyAction({ key: 'Escape' }, modal), { kind: 'closeDialog' });
  assert.deepEqual(keyAction({ key: 'Escape' }, free), { kind: 'action', key: 'Esc' });
});

test('keys are ignored while typing, except Escape', () => {
  for (const key of ['1', '4', ' ', 'a', 's', 'r', 'Enter', 'ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight', 'PageDown', 'Home', 'End']) {
    assert.equal(keyAction({ key }, typing), null, `${JSON.stringify(key)} in a field`);
  }
  assert.deepEqual(keyAction({ key: 'Escape' }, typing), { kind: 'action', key: 'Esc' });
});

test('inside a dialog only Escape is ours', () => {
  for (const key of ['1', '2', ' ', 'a', 's', 'r', 'Enter', 'Tab', 'ArrowDown', 'ArrowLeft', 'PageUp', 'End']) {
    assert.equal(keyAction({ key }, modal), null, `${JSON.stringify(key)} in a dialog`);
  }
  assert.equal(keyAction({ key: 'Escape' }, { inField: true, dialog: true })?.kind, 'closeDialog');
});

test('the footer hint follows the section, from the keys its actions carry', () => {
  const hint = (section: SectionId, state: string): string => footerHint(fixture.views[section][state] as SectionView);
  assert.equal(hint('cleanup', 'analyzed'), '↑↓ move · space toggle · a analyze · ↵ clean');
  assert.equal(hint('cleanup', 'cleaning'), '↑↓ move · space toggle · a analyze · ↵ clean', 'Esc is not in the footer, busy or not');
  assert.equal(hint('registry', 'analyzed'), '↑↓ move · space toggle · s scan · ↵ fix');
  assert.equal(hint('services', 'loaded'), '↑↓ move · space toggle · r refresh');
  assert.equal(hint('developers', 'projects'), '↑↓ move · space toggle · a analyze · ↵ clean');
  assert.equal(footerHint(viewWith()), '↑↓ move · space toggle');
});

test('Tab and Shift+Tab cycle inside a dialog and never leave it', () => {
  assert.equal(cycleFocus(3, 0, false), 1);
  assert.equal(cycleFocus(3, 2, false), 0, 'Tab on the last control wraps to the first');
  assert.equal(cycleFocus(3, 0, true), 2, 'Shift+Tab on the first control wraps to the last');
  assert.equal(cycleFocus(3, 1, true), 0);
  assert.equal(cycleFocus(3, -1, false), 0, 'focus outside the dialog comes in at the first control');
  assert.equal(cycleFocus(3, -1, true), 2);
  assert.equal(cycleFocus(1, 0, false), 0);
  assert.equal(cycleFocus(1, 0, true), 0);
  assert.equal(cycleFocus(0, -1, false), null);
});
