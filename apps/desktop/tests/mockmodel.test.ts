import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { MockModel, type Fixture } from '../src/mockmodel.ts';
import type { SectionId, SectionView } from '../src/types.ts';

const raw = readFileSync(new URL('../fixtures/views.json', import.meta.url), 'utf8');
const fixture = JSON.parse(raw) as Fixture;
const fresh = (): MockModel => new MockModel(JSON.parse(raw) as Fixture);

const STATES: Record<SectionId, string[]> = {
  cleanup: ['idle', 'analyzing', 'analyzed', 'confirm', 'cleaning', 'report'],
  registry: ['idle', 'scanning', 'analyzed', 'backup', 'fixing', 'fixed'],
  services: ['loading', 'loaded', 'confirm'],
  developers: ['idle', 'analyzing', 'projects', 'worktrees', 'sessions', 'confirm', 'shortfall', 'cleaning', 'report'],
};

function checks(view: SectionView): Record<string, string> {
  return Object.fromEntries(view.rows.map((r) => [r.id, r.check]));
}

test('the fixture holds every state the table names', () => {
  for (const [section, names] of Object.entries(STATES)) {
    for (const name of names) {
      const view = fixture.views[section as SectionId][name];
      assert.ok(view, `${section}:${name} is missing`);
      assert.equal(view.section, section, `${section}:${name} names another section`);
    }
  }
  // Busy states carry a job; dialog states a dialog; the rest none.
  for (const [section, name] of [['cleanup', 'analyzing'], ['cleanup', 'cleaning'], ['registry', 'scanning'], ['registry', 'fixing'],
    ['services', 'loading'], ['developers', 'analyzing'], ['developers', 'cleaning']] as const) {
    assert.ok(fixture.views[section][name]?.job, `${section}:${name} has no job`);
  }
  for (const [section, name] of [['cleanup', 'confirm'], ['registry', 'backup'], ['services', 'confirm'], ['developers', 'confirm'], ['developers', 'shortfall']] as const) {
    assert.ok(fixture.views[section][name]?.dialog, `${section}:${name} has no dialog`);
  }
  for (const [section, name] of [['cleanup', 'report'], ['developers', 'report']] as const) {
    assert.ok(fixture.views[section][name]?.report, `${section}:${name} has no report`);
  }
  assert.equal(fixture.views.developers.confirm?.dialog?.ack, false);
});

test('the sections start where the table says and every returned view has a newer rev', () => {
  const model = fresh();
  assert.deepEqual(
    [model.stateOf('cleanup'), model.stateOf('registry'), model.stateOf('services'), model.stateOf('developers')],
    ['idle', 'idle', 'loaded', 'idle'],
  );
  const a = model.show('cleanup');
  const b = model.view('cleanup');
  const c = model.intent('cleanup', { kind: 'search', text: 'x' });
  assert.deepEqual([b.rev - a.rev, c.rev - b.rev], [1, 1]);
  assert.equal(c.status.text, a.status.text, 'an intent outside the table leaves the state as it was');
  assert.equal(model.stateOf('cleanup'), 'idle');
  // Revisions are per section.
  assert.equal(model.show('registry').rev, 1);
});

test('analyze passes three ticks and lands on analyzed', () => {
  const model = fresh();
  const first = model.intent('cleanup', { kind: 'analyze' });
  assert.equal(model.stateOf('cleanup'), 'analyzing');
  assert.equal(first.job?.fraction, 0);
  const fractions: number[] = [];
  let lastRev = first.rev;
  for (let i = 0; i < 3; i += 1) {
    const pushed = model.tick();
    assert.equal(pushed.length, 1);
    assert.equal(pushed[0]?.section, 'cleanup');
    assert.ok((pushed[0]?.rev ?? 0) > lastRev);
    lastRev = pushed[0]?.rev ?? 0;
    fractions.push(pushed[0]?.job?.fraction ?? -1);
    assert.equal(model.stateOf('cleanup'), 'analyzing');
  }
  assert.deepEqual(fractions, [1 / 3, 2 / 3, 1]);
  const landed = model.tick();
  assert.equal(landed.length, 1);
  assert.equal(landed[0]?.job, null);
  assert.equal(model.stateOf('cleanup'), 'analyzed');
  assert.equal(landed[0]?.status.badge, fixture.views.cleanup.analyzed?.status.badge);
  assert.deepEqual(model.tick(), [], 'nothing is pushed once the section is idle');
});

test('scan and refresh are the same job on their sections and land where the table says', () => {
  const model = fresh();
  model.intent('registry', { kind: 'scan' });
  assert.equal(model.stateOf('registry'), 'scanning');
  model.intent('services', { kind: 'refresh' });
  assert.equal(model.stateOf('services'), 'loading');
  model.intent('developers', { kind: 'analyze' });
  assert.equal(model.stateOf('developers'), 'analyzing');
  for (let i = 0; i < 4; i += 1) model.tick();
  assert.deepEqual(
    [model.stateOf('registry'), model.stateOf('services'), model.stateOf('developers')],
    ['analyzed', 'loaded', 'projects'],
  );
  // An analyze while the job runs changes nothing.
  model.intent('cleanup', { kind: 'analyze' });
  model.tick();
  const during = model.intent('cleanup', { kind: 'analyze' });
  assert.equal(during.job?.fraction, 1 / 3);
});

test('toggle flips one row only', () => {
  const model = fresh();
  model.go('cleanup', 'analyzed');
  const before = model.view('cleanup');
  const first = before.rows[0];
  assert.ok(first);
  const after = model.intent('cleanup', { kind: 'toggle', row: first.id });
  const flipped = checks(after);
  for (const row of before.rows) {
    const want = row.id === first.id ? (row.check === 'on' ? 'off' : 'on') : row.check;
    assert.equal(flipped[row.id], want, row.id);
  }
  // Nothing else is recounted.
  assert.equal(after.actions.find((a) => a.primary)?.label, before.actions.find((a) => a.primary)?.label);
  assert.deepEqual(after.nav, before.nav);
  // And back.
  assert.deepEqual(checks(model.intent('cleanup', { kind: 'toggle', row: first.id })), checks(before));
  // The change belongs to the model, not to the one view returned.
  assert.deepEqual(checks(model.view('cleanup')), checks(before));
  // `mixed` turns on, a row that does not exist changes nothing.
  model.go('developers', 'projects');
  const dev = model.view('developers');
  const mixed = dev.rows.find((r) => r.check === 'mixed');
  assert.ok(mixed, 'the Developers fixture has a mixed row');
  assert.equal(checks(model.intent('developers', { kind: 'toggle', row: mixed.id }))[mixed.id], 'on');
  assert.deepEqual(checks(model.intent('developers', { kind: 'toggle', row: 'p:nope' })), checks(model.view('developers')));
});

test('select moves the active category, or the Developers state', () => {
  const model = fresh();
  model.go('cleanup', 'analyzed');
  const after = model.intent('cleanup', { kind: 'select', category: 'cat:browser' });
  assert.deepEqual(after.nav.filter((n) => n.active).map((n) => n.id), ['cat:browser']);
  assert.equal(model.stateOf('cleanup'), 'analyzed');
  model.go('developers', 'projects');
  assert.equal(model.intent('developers', { kind: 'select', category: 'worktrees' }).title, fixture.views.developers.worktrees?.title);
  assert.equal(model.stateOf('developers'), 'worktrees');
  model.intent('developers', { kind: 'select', category: 'sessions' });
  assert.equal(model.stateOf('developers'), 'sessions');
  model.go('developers', 'idle');
  model.intent('developers', { kind: 'select', category: 'sessions' });
  assert.equal(model.stateOf('developers'), 'idle', 'select needs an analysis to show');
});

test('closeDialog returns to the state the dialog was opened from', () => {
  const model = fresh();
  model.go('developers', 'worktrees');
  model.intent('developers', { kind: 'openConfirm' });
  assert.equal(model.stateOf('developers'), 'confirm');
  model.intent('developers', { kind: 'closeDialog' });
  assert.equal(model.stateOf('developers'), 'worktrees');
  model.go('registry', 'analyzed');
  model.intent('registry', { kind: 'openFix' });
  assert.equal(model.stateOf('registry'), 'backup');
  model.intent('registry', { kind: 'closeDialog' });
  assert.equal(model.stateOf('registry'), 'analyzed');
  model.go('services', 'loaded');
  model.intent('services', { kind: 'openConfirm', action: 'disable' });
  assert.equal(model.stateOf('services'), 'confirm');
  model.intent('services', { kind: 'closeDialog' });
  assert.equal(model.stateOf('services'), 'loaded');
  // closeDialog with no dialog open changes nothing.
  model.go('cleanup', 'analyzed');
  model.intent('cleanup', { kind: 'closeDialog' });
  assert.equal(model.stateOf('cleanup'), 'analyzed');
  // openConfirm needs an analysis.
  model.go('cleanup', 'idle');
  model.intent('cleanup', { kind: 'openConfirm' });
  assert.equal(model.stateOf('cleanup'), 'idle');
});

test('cancel returns to the state before the job', () => {
  const model = fresh();
  model.go('developers', 'sessions');
  model.intent('developers', { kind: 'analyze' });
  model.tick();
  assert.equal(model.stateOf('developers'), 'analyzing');
  model.intent('developers', { kind: 'cancel' });
  assert.equal(model.stateOf('developers'), 'sessions');
  assert.deepEqual(model.tick(), [], 'a cancelled job stops ticking');
  // A job started from idle goes back to idle.
  model.go('cleanup', 'idle');
  model.intent('cleanup', { kind: 'analyze' });
  model.intent('cleanup', { kind: 'cancel' });
  assert.equal(model.stateOf('cleanup'), 'idle');
  // Cancelling a clean returns to the analysis it started from, not to the dialog.
  model.go('cleanup', 'analyzed');
  model.intent('cleanup', { kind: 'openConfirm' });
  model.intent('cleanup', { kind: 'confirm' });
  assert.equal(model.stateOf('cleanup'), 'cleaning');
  model.intent('cleanup', { kind: 'cancel' });
  assert.equal(model.stateOf('cleanup'), 'analyzed');
});

test('a confirmed clean runs three ticks and ends on the report', () => {
  const model = fresh();
  model.go('developers', 'projects');
  model.intent('developers', { kind: 'openConfirm' });
  const acked = model.intent('developers', { kind: 'setAck', on: true });
  assert.equal(acked.dialog?.ack, true);
  assert.equal(acked.dialog?.buttons.find((b) => b.primary)?.enabled, true);
  const unacked = model.intent('developers', { kind: 'setAck', on: false });
  assert.equal(unacked.dialog?.ack, false);
  assert.equal(unacked.dialog?.buttons.find((b) => b.primary)?.enabled, false);
  // The ack box is off: the confirmation is refused, whatever the page sends.
  model.intent('developers', { kind: 'confirm' });
  assert.equal(model.stateOf('developers'), 'confirm');
  model.intent('developers', { kind: 'setAck', on: true });
  model.intent('developers', { kind: 'confirm' });
  assert.equal(model.stateOf('developers'), 'cleaning');
  for (let i = 0; i < 3; i += 1) model.tick();
  assert.equal(model.stateOf('developers'), 'cleaning');
  const done = model.tick();
  assert.equal(model.stateOf('developers'), 'report');
  assert.ok(done[0]?.report);
  // The shortfall dialog is left with `permanent`.
  model.go('developers', 'shortfall');
  model.intent('developers', { kind: 'permanent' });
  assert.equal(model.stateOf('developers'), 'cleaning');
  // confirm outside a dialog does nothing.
  model.go('developers', 'projects');
  model.intent('developers', { kind: 'confirm' });
  assert.equal(model.stateOf('developers'), 'projects');
  // The report is closed back to the analysis.
  model.go('cleanup', 'analyzed');
  model.intent('cleanup', { kind: 'openConfirm' });
  model.intent('cleanup', { kind: 'confirm' });
  for (let i = 0; i < 4; i += 1) model.tick();
  assert.equal(model.stateOf('cleanup'), 'report');
  model.intent('cleanup', { kind: 'closeDialog' });
  assert.equal(model.stateOf('cleanup'), 'analyzed');
});

test('registry fixes through the backup dialog and services confirm goes straight back to loaded', () => {
  const model = fresh();
  model.go('registry', 'analyzed');
  model.intent('registry', { kind: 'openFix' });
  model.intent('registry', { kind: 'fix', backup: true });
  assert.equal(model.stateOf('registry'), 'fixing');
  for (let i = 0; i < 4; i += 1) model.tick();
  assert.equal(model.stateOf('registry'), 'fixed');

  model.go('services', 'loaded');
  model.intent('services', { kind: 'openConfirm', action: 'manual' });
  const done = model.intent('services', { kind: 'confirm' });
  assert.equal(model.stateOf('services'), 'loaded');
  assert.equal(done.dialog, null);
  assert.deepEqual(done.notices.map((n) => n.tone), ['ok']);
  assert.deepEqual(model.tick(), []);
});
