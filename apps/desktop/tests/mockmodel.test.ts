import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { MockModel, type Fixture } from '../src/mockmodel.ts';
import type { SectionId, SectionView } from '../src/types.ts';

const raw = readFileSync(new URL('../fixtures/views.json', import.meta.url), 'utf8');
const fixture = JSON.parse(raw) as Fixture;
const fresh = (): MockModel => new MockModel(JSON.parse(raw) as Fixture);
const HOME: Record<SectionId, string> = { cleanup: 'analyzed', registry: 'analyzed', services: 'loaded', developers: 'projects' };

const STATES: Record<SectionId, string[]> = {
  cleanup: ['idle', 'analyzing', 'analyzed', 'confirm', 'cleaning', 'report', 'cancelled'],
  registry: ['idle', 'scanning', 'analyzed', 'backup', 'fixing', 'fixed'],
  services: ['loading', 'loaded', 'confirm'],
  developers: ['idle', 'analyzing', 'projects', 'worktrees', 'sessions', 'confirm', 'shortfall', 'cleaning', 'report', 'cancelled'],
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
  for (const [section, name] of [['cleanup', 'report'], ['developers', 'report'], ['cleanup', 'cancelled'], ['developers', 'cancelled']] as const) {
    assert.ok(fixture.views[section][name]?.report, `${section}:${name} has no report`);
  }
  for (const section of ['cleanup', 'developers'] as const) {
    assert.equal(fixture.views[section].report?.report?.cancelled, false, `${section}:report is a finished clean`);
    const view = fixture.views[section].cancelled;
    assert.equal(view?.report?.cancelled, true, `${section}:cancelled is the report of a cancelled clean`);
    // The analysis is dropped: nothing to select, nothing to clean, only Analyze is on.
    assert.deepEqual(view?.rows, [], `${section}:cancelled keeps rows`);
    assert.deepEqual(view?.actions.map((a) => [a.id, a.enabled]), [['analyze', true], ['clean', false]], `${section}:cancelled actions`);
    assert.equal(view?.dialog, null);
    assert.equal(view?.job, null);
  }
  // A scan cannot be cancelled: no cancel action in Developers' analyzing. A clean can.
  assert.ok(!fixture.views.developers.analyzing?.actions.some((a) => a.id === 'cancel'), 'developers:analyzing offers cancel');
  assert.ok(fixture.views.developers.cleaning?.actions.some((a) => a.id === 'cancel' && a.enabled), 'developers:cleaning has no cancel');
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
  const after = model.intent('cleanup', { kind: 'select', category: 'rules:Sviluppo e AI' });
  assert.deepEqual(after.nav.filter((n) => n.active).map((n) => n.id), ['rules:Sviluppo e AI']);
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

test('cancel of an analysis keeps the partial list; a Developers scan cannot be cancelled', () => {
  const model = fresh();
  // A Cleanup analysis that is cancelled keeps what was analysed so far (Rust), wherever it started: the
  // `interrupted` state, with its error notice and Clean enabled.
  for (const from of ['idle', 'analyzed']) {
    model.go('cleanup', from);
    model.intent('cleanup', { kind: 'analyze' });
    const view = model.intent('cleanup', { kind: 'cancel' });
    assert.equal(model.stateOf('cleanup'), 'interrupted', `started from ${from}`);
    assert.deepEqual(view.notices.map((n) => n.tone), ['error']);
    assert.match(view.notices[0]?.text ?? '', /cancelled/);
    assert.ok(view.actions.find((a) => a.id === 'clean')?.enabled, 'Clean stays enabled over the partial list');
    assert.ok(view.rows.some((r) => r.bytes !== null) && view.rows.some((r) => r.bytes === null), 'some rows were analysed, some were not');
    assert.deepEqual(model.tick().filter((v) => v.section === 'cleanup'), [], 'a cancelled job stops ticking');
  }
  // The partial analysis can be cleaned: the dialog opens, and closing it comes back to the partial list.
  model.intent('cleanup', { kind: 'openConfirm' });
  assert.equal(model.stateOf('cleanup'), 'confirm');
  model.intent('cleanup', { kind: 'closeDialog' });
  assert.equal(model.stateOf('cleanup'), 'interrupted');
  // Developers: the scan is not offered a cancel and the mock refuses it: it runs on to the projects.
  model.go('developers', 'sessions');
  model.intent('developers', { kind: 'analyze' });
  model.tick();
  assert.equal(model.stateOf('developers'), 'analyzing');
  const refused = model.intent('developers', { kind: 'cancel' });
  assert.equal(model.stateOf('developers'), 'analyzing');
  assert.ok(!refused.actions.some((a) => a.id === 'cancel'), 'the scan view has no cancel action');
  for (let i = 0; i < 3; i += 1) model.tick();
  assert.equal(model.stateOf('developers'), 'projects', 'the refused cancel did not stop the scan');
});

test('cancel of a clean ends in the cancelled report with the analysis dropped, in Cleanup and in Developers', () => {
  const model = fresh();
  for (const section of ['cleanup', 'developers'] as const) {
    model.go(section, HOME[section]);
    model.intent(section, { kind: 'openConfirm' });
    model.intent(section, { kind: 'setAck', on: true });
    model.intent(section, { kind: 'confirm' });
    assert.equal(model.stateOf(section), 'cleaning');
    model.tick();
    const view = model.intent(section, { kind: 'cancel' });
    assert.equal(model.stateOf(section), 'cancelled', `${section}: neither the analysis nor the dialog`);
    assert.equal(view.report?.cancelled, true);
    assert.equal(view.job, null);
    assert.deepEqual(view.rows, [], 'nothing is selectable');
    assert.deepEqual(view.actions.map((a) => [a.id, a.enabled]), [['analyze', true], ['clean', false]]);
    assert.deepEqual(model.tick().filter((v) => v.section === section), [], 'a cancelled job stops ticking');
    // Nothing but Analyze works over it.
    for (const refusedIntent of [{ kind: 'openConfirm' }, { kind: 'confirm' }, { kind: 'toggle', row: 'x' }, { kind: 'cancel' }] as const) {
      model.intent(section, refusedIntent);
      assert.equal(model.stateOf(section), 'cancelled', `${section}: ${refusedIntent.kind}`);
    }
    // closeDialog releases the section: empty again, not the analysis.
    const closed = model.intent(section, { kind: 'closeDialog' });
    assert.equal(model.stateOf(section), 'idle');
    assert.equal(closed.report, null);
    // Analyze over the cancelled report is accepted and, cancelled, goes back to where it was.
    model.go(section, 'cancelled');
    const again = model.intent(section, { kind: 'analyze' });
    assert.equal(model.stateOf(section), 'analyzing');
    assert.equal(again.job?.fraction, 0);
    for (let i = 0; i < 4; i += 1) model.tick();
    assert.equal(model.stateOf(section), section === 'cleanup' ? 'analyzed' : 'projects');
  }
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
  // A report is closed by releasing the section (Rust's release()): its empty state, not the analysis.
  model.go('cleanup', 'analyzed');
  model.intent('cleanup', { kind: 'openConfirm' });
  model.intent('cleanup', { kind: 'confirm' });
  for (let i = 0; i < 4; i += 1) model.tick();
  assert.equal(model.stateOf('cleanup'), 'report');
  model.intent('cleanup', { kind: 'closeDialog' });
  assert.equal(model.stateOf('cleanup'), 'idle');
  model.go('developers', 'report');
  model.intent('developers', { kind: 'closeDialog' });
  assert.equal(model.stateOf('developers'), 'idle');
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

test('selecting Applications shows its sections and the search box', () => {
  const model = fresh();
  model.go('cleanup', 'analyzed');
  const plain = model.view('cleanup');
  assert.equal(plain.search, null, 'only Applications has a search box');
  const apps = model.intent('cleanup', { kind: 'select', category: 'winapp2' });
  assert.equal(model.stateOf('cleanup'), 'applications');
  assert.deepEqual(apps.nav.filter((n) => n.active).map((n) => n.id), ['winapp2']);
  assert.deepEqual(apps.search, { placeholder: 'Search', value: '' });
  assert.ok(apps.rows.some((r) => r.kind === 'header' && r.id.startsWith('section:')), 'rows are grouped under Winapp2 section headers');
  assert.ok(apps.options.some((o) => o.kind === 'text'), 'the Winapp2 status line is an option');
  assert.ok(apps.rows.some((r) => r.muted && !r.enabled && r.why), 'a blocked row says why');
  assert.ok(apps.rows.some((r) => r.lines.length > 20), 'a row with a long list of lines');
  // A blocked row cannot be ticked, an ordinary one can.
  const blocked = apps.rows.find((r) => !r.enabled)!;
  assert.equal(checks(model.intent('cleanup', { kind: 'toggle', row: blocked.id }))[blocked.id], 'off');
  const open = apps.rows.find((r) => r.kind === 'item' && r.enabled && r.check === 'off')!;
  assert.equal(checks(model.intent('cleanup', { kind: 'toggle', row: open.id }))[open.id], 'on');
  // Another category goes back to the plain analysis; the Browser category has a state too (keep list button).
  const back = model.intent('cleanup', { kind: 'select', category: 'builtin:Sistema' });
  assert.equal(model.stateOf('cleanup'), 'analyzed');
  assert.equal(back.search, null);
  assert.deepEqual(back.nav.filter((n) => n.active).map((n) => n.id), ['builtin:Sistema']);
  const browser = model.intent('cleanup', { kind: 'select', category: 'rules:Browser' });
  assert.equal(model.stateOf('cleanup'), 'browser');
  assert.ok(browser.options.some((o) => o.kind === 'button' && o.id === 'keepList'));
  assert.ok(browser.rows.some((r) => r.lines.length > 20), 'a native row with its own list of lines');
  // Before an analysis there is nothing to show of them, and the confirmation can be opened from them.
  model.go('cleanup', 'idle');
  model.intent('cleanup', { kind: 'select', category: 'winapp2' });
  assert.equal(model.stateOf('cleanup'), 'idle');
  model.go('cleanup', 'applications');
  model.intent('cleanup', { kind: 'openConfirm' });
  assert.equal(model.stateOf('cleanup'), 'confirm');
  model.intent('cleanup', { kind: 'closeDialog' });
  assert.equal(model.stateOf('cleanup'), 'applications', 'the dialog goes back to the list it was opened from');
});

test('Analyze is on over a finished report and starts a new analysis (Rust enables it when nothing runs)', () => {
  const model = fresh();
  for (const section of ['cleanup', 'developers'] as const) {
    model.go(section, 'report');
    assert.ok(model.view(section).actions.find((a) => a.id === 'analyze')?.enabled, `${section}: Analyze is on over the report`);
    model.intent(section, { kind: 'analyze' });
    assert.equal(model.stateOf(section), 'analyzing');
  }
});

test('Save as text… marks the report saved, only a report that can be saved, and only until it is left', () => {
  const model = fresh();
  model.go('cleanup', 'report');
  assert.equal(model.view('cleanup').report?.saved, null);
  assert.deepEqual(model.saveReport('cleanup').report?.saved, { text: 'Report saved.', ok: true });
  model.go('cleanup', 'report');
  assert.equal(model.view('cleanup').report?.saved, null, 'a report arrived at again starts unsaved');
  model.go('cleanup', 'analyzed');
  assert.equal(model.saveReport('cleanup').report, null, 'nothing to save without a report');
});
