// The safety rules of the mock (Review Focus 1: a removal without the confirmation) and the shared
// intent script. The Rust FakeBackend must obey the same rules and replay the same script.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { MockModel, type Fixture } from '../src/mockmodel.ts';
import type { AnyIntent, SectionId, SectionView } from '../src/types.ts';

const raw = readFileSync(new URL('../fixtures/views.json', import.meta.url), 'utf8');
const fixture = JSON.parse(raw) as Fixture;
const fresh = (): MockModel => new MockModel(JSON.parse(raw) as Fixture);
/** A model over a copy of the fixture that `edit` may loosen: the model must refuse on its own rules, not on the fixture's flags. */
function loose(edit: (views: Fixture['views']) => void): MockModel {
  const copy = JSON.parse(raw) as Fixture;
  edit(copy.views);
  return new MockModel(copy);
}
const checks = (view: SectionView): Record<string, string> => Object.fromEntries(view.rows.map((r) => [r.id, r.check]));
const everyView = (views: Fixture['views']): SectionView[] => Object.values(views).flatMap((states) => Object.values(states));
const enableAll = (views: Fixture['views']): void => {
  for (const v of everyView(views)) { v.rows.forEach((r) => { r.enabled = true; }); v.actions.forEach((a) => { a.enabled = true; }); }
};
const HOME: Record<SectionId, string> = { cleanup: 'analyzed', registry: 'analyzed', developers: 'projects' };

const FROZEN: [SectionId, string][] = [
  ['cleanup', 'analyzing'], ['cleanup', 'confirm'], ['cleanup', 'cleaning'], ['cleanup', 'report'], ['cleanup', 'cancelled'],
  ['registry', 'scanning'], ['registry', 'backup'], ['registry', 'fixing'],
  ['developers', 'analyzing'], ['developers', 'confirm'], ['developers', 'shortfall'], ['developers', 'cleaning'], ['developers', 'report'], ['developers', 'cancelled'],
];

test('a state with a job offers nothing but Cancel, and a report that dropped its analysis lists no row', () => {
  // Rust never greys a row because of a job (the page does, and Rust refuses a toggle anyway): the fixture
  // says what Rust says, so it is the MODEL below that must refuse. Actions are Rust's own: busy = only Cancel.
  for (const [section, name] of FROZEN) {
    const view = fixture.views[section][name];
    assert.ok(view, `${section}:${name}`);
    if (view.job !== null) {
      assert.ok(view.actions.every((a) => !a.enabled || a.id === 'cancel'), `${section}:${name} has an enabled action under a job`);
    }
    if (name === 'cancelled') assert.equal(view.rows.length, 0, `${section}:${name} still lists the dropped analysis`);
  }
  // Developers: a scan cannot be stopped, so its view has no Cancel; a clean can.
  assert.ok(!fixture.views.developers.analyzing?.actions.some((a) => a.id === 'cancel'));
  assert.ok(fixture.views.developers.cleaning?.actions.some((a) => a.id === 'cancel' && a.enabled));
});

test('toggle, select and analyze are refused while a job runs, a dialog is open or a report is on screen', () => {
  // Even when the fixture says a row is enabled, the model itself refuses.
  const model = loose(enableAll);
  for (const [section, name] of FROZEN) {
    model.go(section, name);
    const before = model.view(section);
    const row = before.rows[0];
    if (row) assert.deepEqual(checks(model.intent(section, { kind: 'toggle', row: row.id })), checks(before), `toggle in ${section}:${name}`);
    assert.equal(model.stateOf(section), name, `toggle moved ${section}:${name}`);
    const other = before.nav.find((n) => !n.active);
    if (other) {
      const after = model.intent(section, { kind: 'select', category: other.id });
      assert.deepEqual(after.nav.map((n) => n.active), before.nav.map((n) => n.active), `select in ${section}:${name}`);
      assert.equal(model.stateOf(section), name, `select moved ${section}:${name}`);
    }
    if (before.report !== null) continue; // a report accepts Analyze: see the test below
    const start = { cleanup: 'analyze', registry: 'scan', developers: 'analyze' }[section] as 'analyze';
    const rev = model.view(section).rev;
    const answered = model.intent(section, { kind: start });
    assert.equal(model.stateOf(section), name, `${start} moved ${section}:${name}`);
    assert.equal(answered.rev, rev + 1, 'a refused intent still bumps the revision');
  }
});

test('Analyze is accepted over a report, finished or cancelled, and over nothing else that is frozen', () => {
  const model = loose(enableAll);
  for (const section of ['cleanup', 'developers'] as const) {
    for (const report of ['cancelled', 'report']) {
      model.go(section, report);
      model.intent(section, { kind: 'analyze' });
      assert.equal(model.stateOf(section), 'analyzing', `${section}: Analyze over the ${report}`);
    }
    for (const name of ['confirm', 'cleaning', 'analyzing']) {
      model.go(section, name);
      model.intent(section, { kind: 'analyze' });
      assert.equal(model.stateOf(section), name, `${section}:${name} accepted Analyze`);
    }
  }
});

test('closeDialog over a report releases the section, whatever was recorded; a dialog still goes back', () => {
  const model = fresh();
  for (const section of ['cleanup', 'developers'] as const) {
    for (const name of ['report', 'cancelled']) {
      // From the analysis, through a dialog and a clean, a report is closed to the empty state.
      model.go(section, HOME[section]);
      model.intent(section, { kind: 'openConfirm' });
      model.go(section, name);
      const view = model.intent(section, { kind: 'closeDialog' });
      assert.equal(model.stateOf(section), 'idle', `${section}:${name}`);
      assert.equal(view.report, null);
      assert.equal(view.rows.length === 0 || section === 'cleanup', true, 'the empty Developers list has no rows');
      model.intent(section, { kind: 'closeDialog' });
      assert.equal(model.stateOf(section), 'idle', 'nothing to close any more');
    }
  }
  // A dialog goes back to where it was opened from (unchanged).
  model.go('developers', 'worktrees');
  model.intent('developers', { kind: 'openConfirm' });
  model.intent('developers', { kind: 'closeDialog' });
  assert.equal(model.stateOf('developers'), 'worktrees');
});

test('cancel: a clean goes to the cancelled report, a Developers scan is refused, other jobs go back', () => {
  const model = fresh();
  for (const section of ['cleanup', 'developers'] as const) {
    model.go(section, 'cleaning');
    model.intent(section, { kind: 'cancel' });
    assert.equal(model.stateOf(section), 'cancelled', `${section}: the dialog and the analysis are not coming back`);
  }
  model.go('developers', 'analyzing');
  model.intent('developers', { kind: 'cancel' });
  assert.equal(model.stateOf('developers'), 'analyzing', 'a scan cannot be cancelled');
  model.go('cleanup', 'analyzing');
  model.intent('cleanup', { kind: 'cancel' });
  assert.equal(model.stateOf('cleanup'), 'interrupted', 'a Cleanup analysis can, and keeps its partial list');
});

test('confirm needs an open dialog whose button is enabled and whose ack box, where there is one, is on', () => {
  // Cleanup: the dialog's `continue` button is disabled -> refused.
  const cleanup = loose((v) => { v.cleanup.confirm?.dialog?.buttons.forEach((b) => { if (b.id === 'continue') b.enabled = false; }); });
  cleanup.go('cleanup', 'analyzed');
  cleanup.intent('cleanup', { kind: 'openConfirm' });
  cleanup.intent('cleanup', { kind: 'confirm' });
  assert.equal(cleanup.stateOf('cleanup'), 'confirm');
  // Developers: the button is enabled in the fixture but the ack box is off -> refused until it is on.
  const dev = loose((v) => { v.developers.confirm?.dialog?.buttons.forEach((b) => { b.enabled = true; }); });
  dev.go('developers', 'projects');
  dev.intent('developers', { kind: 'openConfirm' });
  assert.equal(dev.view('developers').dialog?.ack, false);
  dev.intent('developers', { kind: 'confirm' });
  assert.equal(dev.stateOf('developers'), 'confirm');
  dev.intent('developers', { kind: 'permanent' });
  assert.equal(dev.stateOf('developers'), 'confirm', 'permanent belongs to the shortfall dialog');
  dev.intent('developers', { kind: 'setAck', on: true });
  dev.intent('developers', { kind: 'confirm' });
  assert.equal(dev.stateOf('developers'), 'cleaning');
  // The shortfall dialog: `permanent` only.
  const model = fresh();
  model.go('developers', 'shortfall');
  model.intent('developers', { kind: 'confirm' });
  assert.equal(model.stateOf('developers'), 'shortfall');
  model.intent('developers', { kind: 'permanent' });
  assert.equal(model.stateOf('developers'), 'cleaning');
  // Registry: each choice goes through its own button.
  const reg = loose((v) => { v.registry.backup?.dialog?.buttons.forEach((b) => { if (b.id === 'nobackup') b.enabled = false; }); });
  reg.go('registry', 'analyzed');
  reg.intent('registry', { kind: 'openFix' });
  reg.intent('registry', { kind: 'fix', backup: false });
  assert.equal(reg.stateOf('registry'), 'backup');
  reg.intent('registry', { kind: 'fix', backup: true });
  assert.equal(reg.stateOf('registry'), 'fixing');
  // Nothing runs without a dialog.
  const bare: [SectionId, AnyIntent][] = [['cleanup', { kind: 'confirm' }], ['registry', { kind: 'fix', backup: true }], ['developers', { kind: 'confirm' }]];
  for (const [s, intent] of bare) {
    const m = fresh();
    m.go(s, HOME[s]);
    m.intent(s, intent);
    assert.equal(m.stateOf(s), HOME[s], `${s} ran without a dialog`);
  }
});

test('openConfirm and openFix are refused when the action is disabled', () => {
  const off = loose((v) => {
    v.cleanup.analyzed?.actions.forEach((a) => { if (a.id === 'clean') a.enabled = false; });
    v.registry.analyzed?.actions.forEach((a) => { if (a.id === 'fix') a.enabled = false; });
    v.developers.projects?.actions.forEach((a) => { if (a.id === 'clean') a.enabled = false; });
  });
  off.go('cleanup', 'analyzed'); off.intent('cleanup', { kind: 'openConfirm' });
  off.go('registry', 'analyzed'); off.intent('registry', { kind: 'openFix' });
  off.go('developers', 'projects'); off.intent('developers', { kind: 'openConfirm' });
  assert.deepEqual([off.stateOf('cleanup'), off.stateOf('registry'), off.stateOf('developers')], ['analyzed', 'analyzed', 'projects']);
});

test('openConfirm and openFix are refused from a state that is not analyzed-like, even with the action enabled', () => {
  const model = loose(enableAll);
  const NOT_ANALYZED: [SectionId, string][] = [
    ['cleanup', 'idle'], ['cleanup', 'analyzing'], ['cleanup', 'cleaning'], ['cleanup', 'report'], ['cleanup', 'cancelled'],
    ['registry', 'idle'], ['registry', 'scanning'], ['registry', 'fixed'],
    ['developers', 'idle'], ['developers', 'analyzing'], ['developers', 'cleaning'], ['developers', 'report'], ['developers', 'cancelled'],
  ];
  for (const [section, name] of NOT_ANALYZED) {
    model.go(section, name);
    const open: AnyIntent = section === 'registry' ? { kind: 'openFix' } : { kind: 'openConfirm' };
    model.intent(section, open);
    assert.equal(model.stateOf(section), name, `${section}:${name} opened a dialog`);
    assert.equal(model.view(section).dialog, null);
  }
});

test('a dialog that is reopened starts with its ack box off', () => {
  const model = fresh();
  model.go('developers', 'projects');
  model.intent('developers', { kind: 'openConfirm' });
  model.intent('developers', { kind: 'setAck', on: true });
  model.intent('developers', { kind: 'closeDialog' });
  const again = model.intent('developers', { kind: 'openConfirm' });
  assert.equal(again.dialog?.ack, false);
  assert.equal(again.dialog?.buttons.find((b) => b.primary)?.enabled, false);
  // setAck means something in the Developers dialog only.
  model.go('cleanup', 'analyzed');
  model.intent('cleanup', { kind: 'openConfirm' });
  assert.equal(model.intent('cleanup', { kind: 'setOnlyOld', on: false }).dialog?.ack, null);
});

// Rust blocks: Winapp2 / browser options of a running program, native items, a worktree with unsaved changes, a project in use,
// a session in use. It never blocks a built-in category or a registry issue.
const BLOCKED: [SectionId, string, string][] = [
  ['cleanup', 'analyzed', 'steam/downloadcache'], ['cleanup', 'analyzed', 'chrome/cache'],
  ['developers', 'worktrees', 'w:3'], ['developers', 'projects', 'p:3'], ['developers', 'projects', 'a:5'], ['developers', 'sessions', 's:claude:3a7c15'],
];

test('Rust has no protected registry issue: every row of the Registry fixture can be ticked', () => {
  for (const name of Object.keys(fixture.views.registry)) {
    assert.ok(fixture.views.registry[name]?.rows.every((r) => r.enabled && !r.muted && r.why === null), `registry:${name}`);
  }
  const model = fresh();
  model.go('registry', 'analyzed');
  assert.equal(checks(model.intent('registry', { kind: 'toggle', row: 'i:4' }))['i:4'], 'on');
});

test('a blocked row is listed, dimmed, with its reason, and toggling it changes nothing', () => {
  for (const [section, name, id] of BLOCKED) {
    const model = fresh();
    model.go(section, name);
    const view = model.view(section);
    const row = view.rows.find((r) => r.id === id);
    assert.ok(row, `${section}:${name} has no ${id}`);
    assert.equal(row.enabled, false); assert.equal(row.muted, true);
    assert.ok(row.why && row.why.length > 10, `${id} has no reason`);
    assert.equal(row.check, 'off');
    assert.equal(row.kind === 'child' || row.kind === 'item', true);
    assert.deepEqual(checks(model.intent(section, { kind: 'toggle', row: id })), checks(view), `toggle of ${id}`);
    assert.deepEqual(checks(model.intent(section, { kind: 'toggle', row: id })), checks(view));
  }
});

// ---- The shared script: the Rust FakeBackend replays the same file.

interface Expect {
  state?: string; rev?: number; fraction?: number | null; checks?: Record<string, string>; active?: string;
  ack?: boolean | null; primaryEnabled?: boolean; notices?: string[]; dialog?: boolean; report?: boolean; saved?: boolean;
}
type Step =
  | { op: 'go' | 'show' | 'view'; section: SectionId; state?: string; expect?: Expect; note?: string }
  | { op: 'intent'; section: SectionId; intent: AnyIntent; expect?: Expect; note?: string }
  | { op: 'save'; section: SectionId; expect?: Expect; note?: string }
  | { op: 'tick'; pushed: ({ section: SectionId } & Expect)[]; note?: string };

function matches(view: SectionView, state: string, want: Expect, where: string): void {
  if (want.state !== undefined) assert.equal(state, want.state, `${where}: state`);
  if (want.rev !== undefined) assert.equal(view.rev, want.rev, `${where}: rev`);
  if (want.fraction !== undefined) {
    if (want.fraction === null) assert.equal(view.job, null, `${where}: job`);
    else assert.ok(view.job && Math.abs(view.job.fraction - want.fraction) < 1e-6, `${where}: fraction ${view.job?.fraction}`);
  }
  for (const [id, check] of Object.entries(want.checks ?? {})) assert.equal(view.rows.find((r) => r.id === id)?.check, check, `${where}: check ${id}`);
  if (want.active !== undefined) assert.deepEqual(view.nav.filter((n) => n.active).map((n) => n.id), [want.active], `${where}: active`);
  if (want.ack !== undefined) assert.equal(view.dialog?.ack, want.ack, `${where}: ack`);
  if (want.primaryEnabled !== undefined) assert.equal(view.dialog?.buttons.find((b) => b.primary)?.enabled, want.primaryEnabled, `${where}: primary`);
  if (want.notices !== undefined) assert.deepEqual(view.notices.map((n) => n.text), want.notices, `${where}: notices`);
  if (want.dialog !== undefined) assert.equal(view.dialog !== null, want.dialog, `${where}: dialog`);
  if (want.report !== undefined) assert.equal(view.report !== null, want.report, `${where}: report`);
  if (want.saved !== undefined) assert.equal(view.report?.saved != null, want.saved, `${where}: saved`);
}

test('the intent script replays on the model', () => {
  const script = JSON.parse(readFileSync(new URL('../fixtures/intent-script.json', import.meta.url), 'utf8')) as { steps: Step[] };
  assert.ok(script.steps.length > 60, 'the script is short');
  const model = fresh();
  script.steps.forEach((step, index) => {
    const where = `step ${index + 1}${step.note ? ` (${step.note})` : ''}`;
    if (step.op === 'tick') {
      const pushed = model.tick();
      assert.deepEqual(pushed.map((v) => v.section), step.pushed.map((p) => p.section), `${where}: sections pushed`);
      pushed.forEach((view, i) => matches(view, model.stateOf(view.section), step.pushed[i] as Expect, where));
      return;
    }
    const view = step.op === 'go' ? model.go(step.section, step.state as string)
      : step.op === 'show' ? model.show(step.section)
        : step.op === 'view' ? model.view(step.section)
          : step.op === 'save' ? model.saveReport(step.section)
            : model.intent(step.section, step.intent);
    matches(view, model.stateOf(step.section), step.expect ?? {}, where);
  });
});
