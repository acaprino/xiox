import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { disabledReason, emptyAction, flatToggle, layoutChoice, panelOptions } from '../src/policy.ts';
import type { ActionView, DialogView, OptionView, SectionView } from '../src/types.ts';

const action = (id: string, enabled: boolean, label = id): ActionView => ({ id, label, key: null, enabled, primary: false });
const dialog = (extra: Partial<DialogView> = {}): DialogView => ({
  id: 'confirm', title: 'Start cleaning?', text: [], summary: [], permanent: false, rows: [], warningsTitle: null, warnings: [], cautionTitle: null, cautions: [],
  ack: null, error: null, buttons: [{ id: 'continue', label: 'Continue', enabled: true, primary: true }, { id: 'cancel', label: 'Cancel', enabled: true, primary: false }], ...extra,
});
const view = (extra: Partial<SectionView> = {}): SectionView => ({
  section: 'cleanup', actions: [], dialog: null, job: null, status: { badge: 'Analysis done', text: '13.48 GB to delete (46,999 files)' }, ...extra,
}) as SectionView;

test('the views say what a confirmation is: its numbers and whether it deletes for good (from data, not from its words)', () => {
  const states = JSON.parse(readFileSync(new URL('../fixtures/views.json', import.meta.url), 'utf8')).views as Record<string, Record<string, SectionView>>;
  const confirm = (section: string, state: string): DialogView => (states[section]?.[state]?.dialog ?? assert.fail(`${section}:${state} has no dialog`));
  const cleanup = confirm('cleanup', 'confirm');
  assert.equal(cleanup.permanent, true);
  assert.deepEqual(cleanup.summary, ['Clean 8 items', '13.48 GB to delete (46,999 files)']);
  assert.equal(confirm('registry', 'backup').permanent, false);
  assert.equal(confirm('developers', 'confirm').permanent, false);
  assert.equal(confirm('developers', 'shortfall').permanent, true);
});

test('a disabled action says why', () => {
  const analyze = action('analyze', true, 'Analyze');
  const clean = action('clean', false, 'Clean 8 items');
  assert.equal(disabledReason(view({ actions: [analyze, clean] }), analyze), null);
  assert.equal(disabledReason(view({ actions: [analyze, clean] }), clean), 'Run “Analyze” and tick at least one item first.');
  assert.equal(disabledReason(view({ actions: [clean] }), clean), 'Tick at least one item first.');
  assert.match(disabledReason(view({ actions: [clean], job: { label: 'Analyzing', fraction: 0.5, detail: null } }), clean) ?? '', /Analyzing/);
  assert.equal(disabledReason(view({ actions: [clean], dialog: dialog() }), clean), null);
});

test('an empty list offers the enabled action that starts the work', () => {
  assert.equal(emptyAction(view({ actions: [action('scan', true, 'Find issues'), action('fix', false)] }))?.label, 'Find issues');
  assert.equal(emptyAction(view({ actions: [action('analyze', false)] })), null);
});

test('the layout of the projects is a choice of the header, not of the Options panel', () => {
  const layout: OptionView = { kind: 'choice', id: 'layout', label: 'Layout', value: 'tree', enabled: true, choices: [{ id: 'tree', label: 'Tree' }, { id: 'flat', label: 'Flat' }] };
  const days: OptionView = { kind: 'number', id: 'inactiveDays', label: 'Days of inactivity', value: 14, min: 1, max: 365, enabled: true };
  const nav = (active: string) => ['projects', 'worktrees', 'sessions'].map((id) => ({ id, name: id, note: null, bytes: null, active: id === active }));
  const dev = (active: string): SectionView => view({ section: 'developers', nav: nav(active), options: [layout, days] });
  assert.deepEqual(layoutChoice(dev('projects')), layout);
  assert.equal(layoutChoice(dev('worktrees')), null, 'only the list of the projects has a layout');
  assert.deepEqual(panelOptions(dev('projects')), [days]);
  assert.deepEqual(panelOptions(dev('sessions')), [days]);
  // Registry keeps its rule: its kinds of issue are its sidebar, not options.
  const kind: OptionView = { kind: 'toggle', id: 'cat:0', label: 'Fonts', on: true, enabled: true, hint: null };
  assert.deepEqual(panelOptions(view({ section: 'registry', nav: [], options: [kind, days] })), [days]);
});

test('the layout is one small button, pressed for the flat list, that asks for the other one', () => {
  const layout = (value: string): Extract<OptionView, { kind: 'choice' }> => ({ kind: 'choice', id: 'layout', label: 'Layout', value, enabled: true, choices: [{ id: 'tree', label: 'Tree' }, { id: 'flat', label: 'Flat' }] });
  assert.deepEqual(flatToggle(layout('tree')), { label: 'Flat', pressed: false, next: 'flat' });
  assert.deepEqual(flatToggle(layout('flat')), { label: 'Flat', pressed: true, next: 'tree' });
});
