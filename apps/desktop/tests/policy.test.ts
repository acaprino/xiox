import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { disabledReason, emptyAction } from '../src/render/policy.ts';
import type { ActionView, DialogView, SectionView } from '../src/types.ts';

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
  assert.equal(confirm('services', 'confirm').permanent, false);
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
  assert.equal(emptyAction(view({ actions: [action('refresh', true)] })), null);
});
