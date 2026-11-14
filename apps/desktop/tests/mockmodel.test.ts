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
  const first = before.rows.find((r) => r.kind === 'item');
  assert.ok(first);
  const after = model.intent('cleanup', { kind: 'toggle', row: first.id });
  const flipped = checks(after);
  // (The group row above it is recounted: see the group test below.)
  for (const row of before.rows.filter((r) => r.kind !== 'group')) {
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

test('select moves the Developers state (Cleanup has no select: its menu only scrolls)', () => {
  const model = fresh();
  model.go('developers', 'projects');
  assert.equal(model.intent('developers', { kind: 'select', category: 'worktrees' }).title, fixture.views.developers.worktrees?.title);
  assert.equal(model.stateOf('developers'), 'worktrees');
  model.intent('developers', { kind: 'select', category: 'sessions' });
  assert.equal(model.stateOf('developers'), 'sessions');
  model.go('developers', 'idle');
  model.intent('developers', { kind: 'select', category: 'sessions' });
  assert.equal(model.stateOf('developers'), 'idle', 'select needs an analysis to show');
  model.go('cleanup', 'analyzed');
  const before = model.view('cleanup');
  const after = model.intent('cleanup', { kind: 'select', category: 'rules:Browser' } as never);
  assert.equal(model.stateOf('cleanup'), 'analyzed');
  assert.deepEqual(after.nav, before.nav);
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

test('the Cleanup list holds every category one after another, each under its own group row', () => {
  const model = fresh();
  model.go('cleanup', 'analyzed');
  const view = model.view('cleanup');
  assert.deepEqual(view.rows.filter((r) => r.kind === 'group').map((r) => r.id), view.nav.map((n) => n.id), 'one group row per menu entry, in the menu order');
  assert.ok(view.nav.every((n) => !n.active), 'the menu only scrolls: no entry is the open one');
  assert.equal(view.title, 'Cleanup');
  assert.equal(view.bulk, true);
  assert.deepEqual(view.search, { placeholder: 'Search all categories', value: '' });
  assert.ok(view.options.some((o) => o.kind === 'button' && o.id === 'keepList'), 'the cookie keep list button is always there');
  assert.ok(view.options.some((o) => o.kind === 'text'), 'so is the Winapp2 status line');
  assert.ok(view.rows.some((r) => r.kind === 'header' && r.id.startsWith('section:')), 'Applications keeps its section headings');
  assert.ok(view.rows.some((r) => r.muted && !r.enabled && r.why), 'a blocked row says why');
  assert.ok(view.rows.some((r) => r.lines.length > 20), 'a native row with its own list of lines');
  const apps = view.rows.slice(view.rows.findIndex((r) => r.id === 'winapp2'));
  assert.ok(apps.filter((r) => r.kind === 'item').length > 40, 'Applications is long enough to start folded');
  // A group row carries the count and size of what it holds, and a note about the caution rows it leaves alone.
  const system = view.rows.find((r) => r.id === 'builtin:System')!;
  assert.equal(system.check, 'mixed');
  assert.equal(system.bytes, view.nav.find((n) => n.id === 'builtin:System')?.bytes);
  assert.equal(system.why, null);
  assert.match(view.rows.find((r) => r.id === 'rules:Browser')!.why ?? '', /Caution items are not ticked/);
  // A blocked row cannot be ticked, an ordinary one can.
  const blocked = view.rows.find((r) => !r.enabled)!;
  assert.equal(checks(model.intent('cleanup', { kind: 'toggle', row: blocked.id }))[blocked.id], 'off');
  const open = view.rows.find((r) => r.kind === 'item' && r.enabled && r.check === 'off' && r.level !== 'caution')!;
  assert.equal(checks(model.intent('cleanup', { kind: 'toggle', row: open.id }))[open.id], 'on');
  // The dialog opens from the list and goes back to it.
  model.intent('cleanup', { kind: 'openConfirm' });
  assert.equal(model.stateOf('cleanup'), 'confirm');
  model.intent('cleanup', { kind: 'closeDialog' });
  assert.equal(model.stateOf('cleanup'), 'analyzed');
});

test('a group box ticks what it may, and clears it again; select all and none leave caution and blocked rows as they are', () => {
  const model = fresh();
  model.go('cleanup', 'analyzed');
  const state = (id: string): string => checks(model.view('cleanup'))[id] ?? 'gone';
  // Mixed turns on, then off; the group row follows its rows.
  assert.equal(state('builtin:System'), 'mixed');
  let after = checks(model.intent('cleanup', { kind: 'toggle', row: 'builtin:System' }));
  assert.deepEqual(['cat:0', 'cat:4', 'cat:5', 'builtin:System'].map((id) => after[id]), ['on', 'on', 'on', 'on']);
  after = checks(model.intent('cleanup', { kind: 'toggle', row: 'builtin:System' }));
  assert.deepEqual(['cat:0', 'cat:4', 'cat:5', 'builtin:System'].map((id) => after[id]), ['off', 'off', 'off', 'off']);
  // Ticking the rows one by one turns the group row on by itself.
  for (const id of ['cat:0', 'cat:1', 'cat:2', 'cat:3', 'cat:4']) model.intent('cleanup', { kind: 'toggle', row: id });
  assert.equal(state('builtin:System'), 'mixed');
  model.intent('cleanup', { kind: 'toggle', row: 'cat:5' });
  assert.equal(state('builtin:System'), 'on');
  // Select all: caution rows keep what they had (Cookies was ticked, Windows.old was not), a blocked row stays off.
  const all = checks(model.intent('cleanup', { kind: 'selectAll', on: true }));
  assert.equal(all['native:clipboard'], 'on');
  assert.equal(all['native:winold'], 'off');
  assert.equal(all['native:cookies'], 'on');
  assert.equal(all['chrome/cache'], 'off');
  assert.equal(all['rules:Browser'], 'on');
  assert.equal(all['rules:Windows'], 'on');
  const none = checks(model.intent('cleanup', { kind: 'selectAll', on: false }));
  assert.equal(none['native:cookies'], 'on', 'a caution row that was ticked stays ticked');
  assert.equal(none['rules:Browser'], 'off');
  assert.equal(none['builtin:System'], 'off');
  // A group row with nothing to tick does nothing; select all is refused where `bulk` is off.
  model.go('cleanup', 'cancelled');
  assert.equal(model.intent('cleanup', { kind: 'selectAll', on: true }).rows.length, 0);
});

test('select all works in the Registry list too (every issue is eligible), and in Services', () => {
  const model = fresh();
  model.go('registry', 'analyzed');
  const reg = checks(model.intent('registry', { kind: 'selectAll', on: true }));
  assert.ok(Object.values(reg).every((c) => c === 'on'));
  assert.ok(Object.values(checks(model.intent('registry', { kind: 'selectAll', on: false }))).every((c) => c === 'off'));
  model.go('services', 'loaded');
  const before = model.view('services');
  const after = model.intent('services', { kind: 'selectAll', on: false });
  for (const row of before.rows) assert.equal(after.rows.find((r) => r.id === row.id)?.check, row.enabled && row.check !== 'none' ? 'off' : row.check, row.id);
  model.go('services', 'loading');
  const frozen = model.view('services');
  assert.deepEqual(checks(model.intent('services', { kind: 'selectAll', on: true })), checks(frozen), 'refused while the job runs');
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
