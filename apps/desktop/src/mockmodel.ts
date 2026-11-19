// The scripted stand-in for the Rust core: serves the fixture views and their transitions.
// The Rust `FakeBackend` (Task 11) must implement exactly this machine over the same fixture
// (fixtures/views.json) and replay fixtures/intent-script.json, which tests/mockrules.test.ts
// replays here. No DOM and no Vite in this file, so `node --experimental-strip-types --test` loads it.
//
// A VIEW is a fixture state (`cleanup:analyzed`...). The model holds, per section, the name of the
// current state, a revision counter, and a few memories (below). Fixture states are fixed copies:
// a toggle or an ack changes the view it was made in and nothing else (the confirm / cleaning /
// report copies of a list do not follow the toggles made in the analysed one); the list does not
// change with anything but the ticks (`toggle`, `selectAll`, below).
//
// REVISIONS. `rev` starts at 0 and grows by one on EVERY view the model returns or pushes: `show`,
// `view`, `go`, each `intent` (also an ignored or refused one: the same state with a new rev) and
// each view `tick` pushes. Revisions are per section.
//
// STATES. Initial: cleanup:idle, registry:idle, developers:idle. Busy states (a job
// runs) and what they land on after their ticks:
//   cleanup    analyzing -> analyzed      cleaning -> report
//   registry   scanning  -> analyzed      fixing   -> fixed
//   developers analyzing -> projects      cleaning -> report
// A cleaning that is CANCELLED does not land on the report but on `cancelled` (see `cancel`). A Cleanup
// ANALYSIS that is cancelled lands on `interrupted`: the partial list, an error notice and Clean enabled.
// "Home" (where cancel / closeDialog fall back when nothing was recorded, e.g. after `go`):
//   cleanup analyzed, registry analyzed, developers projects.
// "Empty" (where closeDialog goes from a report, whose analysis was dropped): cleanup idle, developers idle.
// "Analyzed-like" (where a dialog can be opened): cleanup analyzed / interrupted,
//   registry analyzed, developers projects / worktrees / sessions.
// The Cleanup list holds ALL its categories one after another (contract v2): each starts with a `group` row (its own
// check box, count and size), Winapp2 keeps its `header` sub-headings, and the menu of bar 2 only scrolls (the page's
// business: Cleanup has no `select` intent any more).
// FROZEN = a job runs (busy state) OR the view has a dialog OR the view has a report.
// CANCELLED (cleanup:cancelled, developers:cancelled) = the report of a clean that was cancelled
//   (`report.cancelled` true). It is FROZEN like any report, with the differences the engine has: the
//   analysis is DROPPED (no rows, nothing selectable, `clean` disabled) and Analyze is enabled and
//   accepted (the one intent the frozen rule lets through: see `analyze`).
//
// INTENTS (any section unless named). `kind` is the intent's `kind`.
//   analyze (cleanup, developers) | scan (registry)
//       refused while a job runs or a dialog is open; accepted over a report (finished or CANCELLED: the
//       button is on when nothing runs, as in Rust). Otherwise records the current state as `back`, goes to
//       the busy state (analyzing / scanning). That first view has job.fraction 0.
//   tick (the timer: not an intent; returns the views pushed, in section order cleanup, registry,
//       developers, at most one per section)
//       a section that is not busy pushes nothing. A busy section counts its ticks: tick 1, 2, 3 push the
//       busy view with job.fraction 1/3, 2/3, 1 (same state); tick 4 goes to the landing state and
//       pushes that view (no job). Every pushed view bumps the rev.
//   select{category} (developers)    only from projects / worktrees / sessions to one of the three
//       (so never when FROZEN); goes to that state. No memory is needed: each state's nav marks its own.
//   toggle{row}                      refused when FROZEN, or when the row is missing, disabled
//       (enabled false: blocked rows) or has check 'none'. Otherwise that row's check goes on<->off
//       (mixed -> on) in the current fixture state. A box row ticks (or, when it was on, clears) its ELIGIBLE rows:
//       a `group` row the rows after it up to the next group row, a `header` row (a sub-heading with a box) the rows
//       after it up to the next header or group row; eligible are the items or children, enabled, with a check and not
//       level `caution`. Afterwards the check of every box row is recounted from its rows: `on` when every eligible
//       row is on, `off` when none is, else `mixed` (`none` when there is no eligible row); nothing else (parent
//       rows, nav notes, subtitle, status) is recounted.
//   selectAll{on}                    refused when FROZEN or when the view's `bulk` is false. Otherwise every eligible
//       row (above) of the view goes on / off, then the checks are recounted as above. A `caution` row and a blocked row
//       keep what they had, in every state.
//   openConfirm (cleanup, developers) | openFix (registry)
//       only from an analyzed-like state AND only when the action is enabled in the current view
//       (cleanup / developers action `clean`; registry `fix`). Records the current state as `back`, goes
//       to the dialog state (confirm; registry: backup). A dialog's ack box is reset to false and its
//       primary button to disabled each time it opens.
//   setAck{on} (developers only)     only when the current view has a dialog with an ack box (ack not
//       null): dialog.ack = on and the primary button enabled = on. Anything else ignores it.
//   confirm (cleanup, developers) | fix{backup} (registry) | permanent (developers)
//       run only from the right dialog state AND when the dialog's button for that intent is enabled AND
//       the dialog's ack is null or true:
//         cleanup confirm: state confirm, button `continue`.
//         developers confirm: state confirm, button `clean`.   developers permanent: state shortfall,
//         button `permanent`.   registry fix{backup}: state backup, button `backup` (true) / `nobackup`.
//       Then: cleanup / developers go to `cleaning`, registry to `fixing` (busy, job.fraction 0, `back`
//       unchanged: the state recorded when the dialog opened; `go` clears it).
//   closeDialog                      only when the view has a dialog or a report.
//       A dialog: goes to `back` (the state the dialog was opened from), else to Home.
//       A report (done or CANCELLED): RELEASES the section, as Rust's `release()` does: the analysis and
//       the cleaned data are dropped and the section goes to its Empty state (cleanup idle, developers
//       idle), whatever `back` held; `back` is forgotten. A report never goes back to the analysis.
//   cancel                           only in a busy state.
//       cleaning (cleanup, developers): goes to `cancelled`, NOT back to the analysis nor to the dialog:
//         the cancelled clean ends in a report of what was done (`report.cancelled` true, no rows, Clean
//         disabled, Analyze enabled) and the analysis is dropped; `back` is forgotten.
//       analyzing (cleanup): goes to `interrupted`, NOT back: Rust keeps what was analysed so far, shows an
//         error notice and keeps Clean enabled; `back` is forgotten.
//       analyzing (developers): REFUSED. A scan cannot be stopped: its view has no `cancel` action and
//         Rust answers "Nothing can be cancelled right now.". The scan runs on to `projects`.
//       any other busy state (cleanup analyzing, registry scanning / fixing): goes to
//         `back` (the state before the job), else Home.
//       A cancelled job stops ticking.
//   anything else (search, setOnlyOld, setInactiveDays, ...)   the same state.
// An intent that is refused or does not apply is IGNORED: same state, new rev, nothing else changes.
//
// WHAT NO INTENT DOES (the FakeBackend must not invent it either).
//   developers:shortfall is reachable only through `go`: no intent leads to it (confirm from the confirm
//       dialog goes to `cleaning`; in Rust the shortfall comes from a real disk check the fixture lacks).
//   registry:partial (a change that half failed: a `warn` notice) is likewise reachable only through `go`; the smoke script
//       draws it, the mock never moves to or from it by an intent (the real engine does both: the fixture only shows what it
//       looks like).
//   `cancel` is never sent for Registry: its intent type (RegistryIntent)
//       has no `cancel`, and `actionIntent` returns null for it there; the only "Cancel" in Registry is
//       the backup dialog's button, which is `closeDialog`. This mock does not look at the section when
//       it gets a `cancel` (a busy Registry would go back), because the page can never send one; the
//       Rust FakeBackend rejects it at the type level (no such variant).
//   cancel of a developers scan, closeDialog with no dialog and no report, cancel when nothing is busy, select of a category that is
//       not in the nav, toggle of a row whose check is `none` (a header without a box: the OTHER WORKTREES row of
//       developers:worktrees) or that does not exist, openConfirm from a state that is not analyzed-like: all refused, as above
//       (steps at the end of fixtures/intent-script.json).
//
// REPORTS. `saveReport(s)` (the `save_report` command; not an intent, the dialog is Rust's) puts the saved
// outcome on the report on screen when it can be saved: `report.saved = { text: SAVED, ok: true }`. Anything
// else is returned as it is. A new analysis or a closed report drops it with the report.
//
// `go(s, state)` jumps to a named state (smoke only): resets the tick counter and `back`.

import type { AboutView, AnyIntent, AppInfo, Check, DialogView, RowView, SectionId, SectionView } from './types.ts';

export interface Fixture {
  app: AppInfo;
  about: AboutView;
  views: Record<SectionId, Record<string, SectionView>>;
}

const SECTIONS: readonly SectionId[] = ['cleanup', 'registry', 'developers'];

const INITIAL: Record<SectionId, string> = { cleanup: 'idle', registry: 'idle', developers: 'idle' };
const HOME: Record<SectionId, string> = { cleanup: 'analyzed', registry: 'analyzed', developers: 'projects' };
const ANALYZED: Record<SectionId, readonly string[]> = {
  cleanup: ['analyzed', 'interrupted'], registry: ['analyzed'],
  developers: ['projects', 'worktrees', 'sessions'],
};
/** What the Save as text… command writes (Rust names the real file; the page never sees a path). */
const SAVED = 'Report saved.';
const LANDS: Record<SectionId, Record<string, string>> = {
  cleanup: { analyzing: 'analyzed', cleaning: 'report' },
  registry: { scanning: 'analyzed', fixing: 'fixed' },
  developers: { analyzing: 'projects', cleaning: 'report' },
};
const ANALYZE_BUSY: Record<SectionId, string> = { cleanup: 'analyzing', registry: 'scanning', developers: 'analyzing' };
const ANALYZE_KIND: Record<SectionId, string> = { cleanup: 'analyze', registry: 'scan', developers: 'analyze' };
/** The state a report is closed to (Rust `release()`: the section is empty again). */
const EMPTY: Partial<Record<SectionId, string>> = { cleanup: 'idle', developers: 'idle' };
/** Where `cancel` goes from a busy state: a state name, or null when that job cannot be cancelled. Missing = `back`, else Home. */
const CANCEL_TO: Partial<Record<SectionId, Record<string, string | null>>> = {
  cleanup: { analyzing: 'interrupted', cleaning: 'cancelled' },
  developers: { analyzing: null, cleaning: 'cancelled' },
};
const OPEN_KIND: Record<SectionId, string> = { cleanup: 'openConfirm', registry: 'openFix', developers: 'openConfirm' };
const DIALOG: Record<SectionId, string> = { cleanup: 'confirm', registry: 'backup', developers: 'confirm' };
const DEV_CATEGORIES = ['projects', 'worktrees', 'sessions'];
const TICKS = 3;

/** What it takes to run: the dialog state, the dialog button and the busy state it starts; null when the intent is not a run. */
function runRule(s: SectionId, intent: AnyIntent): { from: string; button: string; busy: string } | null {
  if (s === 'cleanup' && intent.kind === 'confirm') return { from: 'confirm', button: 'continue', busy: 'cleaning' };
  if (s === 'registry' && intent.kind === 'fix') return { from: 'backup', button: intent.backup ? 'backup' : 'nobackup', busy: 'fixing' };
  if (s === 'developers' && intent.kind === 'confirm') return { from: 'confirm', button: 'clean', busy: 'cleaning' };
  if (s === 'developers' && intent.kind === 'permanent') return { from: 'shortfall', button: 'permanent', busy: 'cleaning' };
  return null;
}

/** A row that a group box or «select all» may change: an item or a child, enabled, with a check, and not a `caution` one. */
const eligible = (r: RowView): boolean => (r.kind === 'item' || r.kind === 'child') && r.enabled && r.check !== 'none' && r.level !== 'caution';

/** A row with a box over other rows: a category (`group`) or a sub-heading (`header`). */
const isBox = (r: RowView): boolean => r.kind === 'group' || r.kind === 'header';

/**
 * The rows a box row may change: after it, up to the next group row (a sub-heading: also the next sub-heading that is
 * not inside it, as a folder of the projects' tree is inside the folder its `parent` names).
 */
function boxRows(rows: RowView[], box: RowView): RowView[] {
  const inside = new Set([box.id]);
  const out: RowView[] = [];
  for (const r of rows.slice(rows.indexOf(box) + 1)) {
    if (r.kind === 'group') break;
    if (box.kind === 'header' && r.kind === 'header') {
      if (r.parent === null || !inside.has(r.parent)) break;
      inside.add(r.id);
    }
    if (eligible(r)) out.push(r);
  }
  return out;
}

const tally = (checks: Check[]): Check => (checks.length === 0 ? 'none' : checks.every((c) => c === 'on') ? 'on' : checks.every((c) => c === 'off') ? 'off' : 'mixed');

/** The check of every box row, counted again from the rows it covers (a parent row is not recounted: the engine's own figures stay). */
function recount(rows: RowView[]): void {
  for (const r of rows) if (isBox(r)) r.check = tally(boxRows(rows, r).map((x) => x.check));
}

export class MockModel {
  private readonly work: Fixture['views'];
  private readonly state: Record<SectionId, string>;
  private readonly revs: Record<SectionId, number>;
  /** Where closeDialog and cancel go back to; undefined = Home. */
  private readonly back: Partial<Record<SectionId, string>> = {};
  private readonly ticks: Record<SectionId, number> = { cleanup: 0, registry: 0, developers: 0 };

  constructor(fixture: Fixture) {
    // A working copy: toggles and acks change it, the fixture stays as loaded.
    this.work = structuredClone(fixture.views);
    this.state = { ...INITIAL };
    this.revs = { cleanup: 0, registry: 0, developers: 0 };
    for (const s of SECTIONS) {
      if (!this.work[s][this.state[s]]) throw new Error(`the fixture has no ${s}:${this.state[s]}`);
    }
  }

  /** Opens a section: its current view. */
  show(s: SectionId): SectionView {
    return this.stamp(s);
  }

  /** The section's current view. */
  view(s: SectionId): SectionView {
    return this.stamp(s);
  }

  /** `Save as text…`: the report on screen says it was saved (the working copy keeps it until the report goes). */
  saveReport(s: SectionId): SectionView {
    const report = this.current(s).report;
    if (report !== null && report.canSave) report.saved = { text: SAVED, ok: true };
    return this.stamp(s);
  }

  /** The name of the fixture state a section is in (tests and the smoke script read it). */
  stateOf(s: SectionId): string {
    return this.state[s];
  }

  /** Applies one intent by the transition table and returns the section's new view. */
  intent(s: SectionId, intent: AnyIntent): SectionView {
    this.apply(s, intent);
    return this.stamp(s);
  }

  /** What a worker would push on its own: one view for every section whose job moved. */
  tick(): SectionView[] {
    const pushed: SectionView[] = [];
    for (const s of SECTIONS) {
      const landing = LANDS[s][this.state[s]];
      if (landing === undefined) continue;
      this.ticks[s] += 1;
      if (this.ticks[s] > TICKS) this.goTo(s, landing);
      pushed.push(this.stamp(s));
    }
    return pushed;
  }

  /** Jumps a section to one of its fixture states (the smoke script's lever). */
  go(s: SectionId, state: string): SectionView {
    if (!this.work[s][state]) throw new Error(`no state ${state} for ${s}`);
    this.enter(s, state);
    delete this.back[s];
    return this.stamp(s);
  }

  /**
   * Changes fields of a fixture state in the working copy (the smoke script's lever, with `go`): a view the engine
   * never sends but the page must still survive (the bulk buttons with a job running).
   */
  patch(s: SectionId, state: string, fields: Partial<SectionView>): void {
    const view = this.work[s][state];
    if (!view) throw new Error(`no state ${state} for ${s}`);
    Object.assign(view, structuredClone(fields));
  }

  private current(s: SectionId): SectionView {
    const view = this.work[s][this.state[s]];
    if (!view) throw new Error(`no state ${this.state[s]} for ${s}`);
    return view;
  }

  private stamp(s: SectionId): SectionView {
    this.revs[s] += 1;
    const view: SectionView = { ...structuredClone(this.current(s)), rev: this.revs[s] };
    if (view.job && LANDS[s][this.state[s]] !== undefined) {
      view.job = { ...view.job, fraction: Math.min(this.ticks[s], TICKS) / TICKS };
    }
    return view;
  }

  private isBusy(s: SectionId): boolean {
    return LANDS[s][this.state[s]] !== undefined;
  }

  /** A job runs, a dialog is open or a report is on screen: selection and new jobs are refused. */
  private frozen(s: SectionId): boolean {
    const view = this.current(s);
    return this.isBusy(s) || view.dialog !== null || view.report !== null;
  }

  private goTo(s: SectionId, state: string): void {
    if (!this.work[s][state]) throw new Error(`no state ${state} for ${s}`);
    this.enter(s, state);
  }

  /** Arriving on a state: the tick counter restarts and a report starts unsaved. */
  private enter(s: SectionId, state: string): void {
    this.state[s] = state;
    this.ticks[s] = 0;
    const report = this.work[s][state]?.report;
    if (report) report.saved = null;
  }

  private actionEnabled(s: SectionId, id: string): boolean {
    return this.current(s).actions.find((a) => a.id === id)?.enabled === true;
  }

  private apply(s: SectionId, intent: AnyIntent): void {
    const here = this.state[s];
    const kind = intent.kind;
    if (kind === ANALYZE_KIND[s]) {
      if (this.isBusy(s) || this.current(s).dialog !== null) return; // a report does not stop it
      this.back[s] = here;
      this.goTo(s, ANALYZE_BUSY[s]);
    } else if (kind === 'select' && s === 'developers') {
      if (this.frozen(s)) return;
      const category = (intent as { category: string }).category;
      if (DEV_CATEGORIES.includes(here) && DEV_CATEGORIES.includes(category)) this.goTo(s, category);
    } else if (kind === 'toggle') {
      if (!this.frozen(s)) this.toggle(s, (intent as { row: string }).row);
    } else if (kind === 'selectAll') {
      if (!this.frozen(s) && this.current(s).bulk) this.selectAll(s, (intent as { on: boolean }).on);
    } else if (kind === OPEN_KIND[s]) {
      const action = s === 'registry' ? 'fix' : 'clean';
      if (!ANALYZED[s].includes(here) || !this.actionEnabled(s, action)) return;
      this.back[s] = here;
      this.goTo(s, DIALOG[s]);
      this.resetAck(s);
    } else if (kind === 'setAck' && s === 'developers') {
      this.setAck((intent as { on: boolean }).on);
    } else if (runRule(s, intent) !== null) {
      this.run(s, intent);
    } else if (kind === 'closeDialog') {
      const view = this.current(s);
      if (view.dialog === null && view.report === null) return;
      if (view.report !== null) {
        // A report is closed by releasing the section: the analysis is gone, the section is empty.
        delete this.back[s];
        this.goTo(s, EMPTY[s] ?? HOME[s]);
        return;
      }
      this.goTo(s, this.back[s] ?? HOME[s]);
    } else if (kind === 'cancel') {
      if (!this.isBusy(s)) return;
      const to = CANCEL_TO[s]?.[here];
      if (to === null) return; // a scan cannot be cancelled
      if (to !== undefined) delete this.back[s]; // the analysis is dropped: nothing to go back to
      this.goTo(s, to ?? this.back[s] ?? HOME[s]);
    }
  }

  private toggle(s: SectionId, id: string): void {
    const rows = this.current(s).rows;
    const row = rows.find((r) => r.id === id);
    if (!row || !row.enabled || row.check === 'none') return;
    if (isBox(row)) {
      const to = row.check === 'on' ? 'off' : 'on';
      for (const r of boxRows(rows, row)) r.check = to;
    } else {
      row.check = row.check === 'on' ? 'off' : 'on';
    }
    recount(rows);
  }

  private selectAll(s: SectionId, on: boolean): void {
    const rows = this.current(s).rows;
    for (const r of rows) if (eligible(r)) r.check = on ? 'on' : 'off';
    recount(rows);
  }

  private setAck(on: boolean): void {
    const dialog: DialogView | null = this.current('developers').dialog;
    if (!dialog || dialog.ack === null) return;
    dialog.ack = on;
    const primary = dialog.buttons.find((b) => b.primary);
    if (primary) primary.enabled = on;
  }

  /** A dialog that opens again must not start pre-checked. */
  private resetAck(s: SectionId): void {
    const dialog = this.current(s).dialog;
    if (!dialog || dialog.ack === null) return;
    dialog.ack = false;
    const primary = dialog.buttons.find((b) => b.primary);
    if (primary) primary.enabled = false;
  }

  private run(s: SectionId, intent: AnyIntent): void {
    const rule = runRule(s, intent);
    if (!rule || this.state[s] !== rule.from) return;
    const dialog = this.current(s).dialog;
    const button = dialog?.buttons.find((b) => b.id === rule.button);
    if (!dialog || !button || !button.enabled || (dialog.ack !== null && !dialog.ack)) return;
    // `back` stays what it was when the dialog opened; a cancelled clean forgets it, a closed report ignores it.
    this.goTo(s, rule.busy);
  }
}
