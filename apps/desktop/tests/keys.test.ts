import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { actionForKey, cycleFocus, DIALOG_ARM_MS, dialogKeySwallowed, footerHint, keyAction } from '../src/keys.ts';
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
  // `a` is also Registry's Find issues and Services' Refresh: there is no `s` and no `r` key (the old window had them).
  assert.equal(keyAction({ key: 's' }, free), null);
  assert.equal(keyAction({ key: 'r' }, free), null);
  const view = viewWith(action('analyze', 'a', true), action('clean', 'Enter', false), action('cancel', 'Escape', true));
  assert.equal(actionForKey(view, 'a')?.id, 'analyze');
  assert.equal(actionForKey(view, 'Enter'), null, 'Clean is disabled: Enter does nothing');
  assert.equal(actionForKey(view, 'Esc')?.id, 'cancel', 'the key name "Escape" is accepted');
  assert.equal(actionForKey(viewWith(action('cancel', 'Esc', true)), 'Esc')?.id, 'cancel', 'Rust writes the key as "Esc"');
  assert.equal(actionForKey(viewWith(action('cancel', 'Esc', false)), 'Esc'), null, 'a disabled Cancel does not answer');
  assert.equal(actionForKey(viewWith(action('scan', 'a', true)), 'a')?.id, 'scan', 'Registry: a presses Find issues');
  assert.equal(actionForKey(viewWith(action('refresh', 'a', true)), 'a')?.id, 'refresh', 'Services: a presses Refresh');
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
  assert.equal(hint('cleanup', 'cleaning'), '↑↓ move · space toggle · a analyze · ↵ clean', 'Rust writes the Cancel key as "Esc": still not in the footer');
  assert.equal(hint('registry', 'analyzed'), '↑↓ move · space toggle · a find issues · ↵ fix');
  assert.equal(hint('services', 'loaded'), '↑↓ move · space toggle · a refresh · ↵ disable');
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

test('question mark opens About outside fields and dialogs', () => {
  assert.deepEqual(keyAction({ key: '?' }, free), { kind: 'about' });
  // Shift is how a keyboard types it: still ours.
  assert.deepEqual(keyAction({ key: '?', shiftKey: true }, free), { kind: 'about' });
  // Typing a question mark in a search box is typing, and an open dialog (About included) keeps the keyboard.
  assert.equal(keyAction({ key: '?' }, typing), null);
  assert.equal(keyAction({ key: '?' }, modal), null);
  assert.equal(keyAction({ key: '?', ctrlKey: true }, free), null);
  // Only the question mark itself: the slash key unshifted is not it.
  assert.equal(keyAction({ key: '/' }, free), null);
});

test('a held key presses no action, but arrows and Escape keep repeating', () => {
  for (const key of ['Enter', 'a']) {
    assert.equal(keyAction({ key, repeat: true }, free), null, `${key} held down`);
    assert.deepEqual(keyAction({ key, repeat: false }, free), { kind: 'action', key }, `${key} pressed once`);
  }
  assert.deepEqual(keyAction({ key: 'ArrowDown', repeat: true }, free), { kind: 'move', delta: 1 });
  assert.deepEqual(keyAction({ key: 'Escape', repeat: true }, free), { kind: 'action', key: 'Esc' });
  assert.deepEqual(keyAction({ key: 'Escape', repeat: true }, modal), { kind: 'closeDialog' });
});

test('a dialog button ignores held keys and keys within 300 ms of the dialog opening, never Escape or Tab', () => {
  assert.equal(DIALOG_ARM_MS, 300);
  // The Enter that opened the dialog, held: every repeat is swallowed, however late it comes.
  assert.equal(dialogKeySwallowed({ key: 'Enter', repeat: true }, 5000), true);
  assert.equal(dialogKeySwallowed({ key: ' ', repeat: true }, 5000), true);
  // A second Enter in a hurry lands on the primary button before the dialog is armed.
  assert.equal(dialogKeySwallowed({ key: 'Enter' }, 0), true);
  assert.equal(dialogKeySwallowed({ key: 'Enter', repeat: false }, 299), true);
  assert.equal(dialogKeySwallowed({ key: ' ' }, 120), true);
  // After 300 ms Enter and Space press the focused button.
  assert.equal(dialogKeySwallowed({ key: 'Enter' }, 300), false);
  assert.equal(dialogKeySwallowed({ key: 'Enter', repeat: false }, 2000), false);
  assert.equal(dialogKeySwallowed({ key: ' ' }, 300), false);
  // Escape and Tab always work.
  for (const age of [0, 100, 299, 300, 5000]) {
    assert.equal(dialogKeySwallowed({ key: 'Escape' }, age), false, `Escape at ${age} ms`);
    assert.equal(dialogKeySwallowed({ key: 'Escape', repeat: true }, age), false, `held Escape at ${age} ms`);
    assert.equal(dialogKeySwallowed({ key: 'Tab' }, age), false, `Tab at ${age} ms`);
  }
});
