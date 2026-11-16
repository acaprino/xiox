import { test } from 'node:test';
import assert from 'node:assert/strict';
import { disabledReason, emptyAction, permanentDelete, summaryLines } from '../src/render/policy.ts';
import type { ActionView, DialogView, SectionView } from '../src/types.ts';

const action = (id: string, enabled: boolean, label = id): ActionView => ({ id, label, key: null, enabled, primary: false });
const dialog = (extra: Partial<DialogView> = {}): DialogView => ({
  id: 'confirm', title: 'Start cleaning?', text: [], summary: [], rows: [], warningsTitle: null, warnings: [], cautionTitle: null, cautions: [],
  ack: null, error: null, buttons: [{ id: 'continue', label: 'Continue', enabled: true, primary: true }, { id: 'cancel', label: 'Cancel', enabled: true, primary: false }], ...extra,
});
const view = (extra: Partial<SectionView> = {}): SectionView => ({
  section: 'cleanup', actions: [], dialog: null, job: null, status: { badge: 'Analysis done', text: '13.48 GB to delete (46,999 files)' }, ...extra,
}) as SectionView;

test('a delete that says it is permanent starts on Cancel; a Recycle Bin one does not', () => {
  assert.equal(permanentDelete(dialog({ text: ['The selected files will be deleted permanently.'] })), true);
  assert.equal(permanentDelete(dialog({ warnings: ['Emptying the Recycle Bin cannot be undone.'] })), true);
  assert.equal(permanentDelete(dialog({ buttons: [{ id: 'permanent', label: 'Permanently', enabled: true, primary: true }] })), true);
  assert.equal(permanentDelete(dialog({ text: ['The selected items go to the Recycle Bin.'] })), false);
  assert.equal(permanentDelete(dialog({ id: 'backup', text: ['permanently'] })), false);
});

test('the confirmation of Cleanup states the item count, the size and the file count', () => {
  const v = view({ actions: [action('clean', false, 'Clean 8 items')] });
  assert.deepEqual(summaryLines(v, dialog()), ['Clean 8 items', '13.48 GB to delete (46,999 files)']);
  assert.deepEqual(summaryLines(v, dialog({ summary: ['2 folders · 6.80 GB'] })), ['2 folders · 6.80 GB']);
  assert.deepEqual(summaryLines(view({ section: 'services' }), dialog()), []);
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
