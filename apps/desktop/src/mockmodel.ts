// The scripted stand-in for the Rust core: serves the fixture views and their transitions.
// The Rust `FakeBackend` (Task 11) must implement exactly this machine over the same fixture
// (fixtures/views.json) and replay fixtures/intent-script.json, which tests/mockrules.test.ts
// replays here. No DOM and no Vite in this file, so `node --experimental-strip-types --test` loads it.
//
// A VIEW is a fixture state (`cleanup:analyzed`...). The model holds, per section, the name of the
// current state, a revision counter, and a few memories (below). Fixture states are fixed copies:
// a toggle or an ack changes the view it was made in and nothing else (the confirm / cleaning /
// report copies of a list do not follow the toggles made in the analysed one); the list does not
// change with the Cleanup category (`select` only moves `nav[].active`).
//
// REVISIONS. `rev` starts at 0 and grows by one on EVERY view the model returns or pushes: `show`,
// `view`, `go`, each `intent` (also an ignored or refused one: the same state with a new rev) and
// each view `tick` pushes. Revisions are per section.
//
// STATES. Initial: cleanup:idle, registry:idle, services:loaded, developers:idle. Busy states (a job
// runs) and what they land on after their ticks:
//   cleanup    analyzing -> analyzed      cleaning -> report
//   registry   scanning  -> analyzed      fixing   -> fixed
//   services   loading   -> loaded
//   developers analyzing -> projects      cleaning -> report
// "Home" (where cancel / closeDialog fall back when nothing was recorded, e.g. after `go`):
//   cleanup analyzed, registry analyzed, services loaded, developers projects.
// "Analyzed-like" (where a dialog can be opened): cleanup analyzed, registry analyzed, services loaded,
//   developers projects / worktrees / sessions.
// FROZEN = a job runs (busy state) OR the view has a dialog OR the view has a report.
//
// INTENTS (any section unless named). `kind` is the intent's `kind`.
//   analyze (cleanup, developers) | scan (registry) | refresh (services)
//       refused when FROZEN. Otherwise records the current state as `back`, clears the Services notice,
//       goes to the busy state (analyzing / scanning / loading). That first view has job.fraction 0.
//   tick (the timer: not an intent; returns the views pushed, in section order cleanup, registry,
//       services, developers, at most one per section)
//       a section that is not busy pushes nothing. A busy section counts its ticks: tick 1, 2, 3 push the
//       busy view with job.fraction 1/3, 2/3, 1 (same state); tick 4 goes to the landing state and
//       pushes that view (no job). Every pushed view bumps the rev.
//   select{category} (cleanup)       refused when FROZEN or when the category is not in nav; otherwise the
//       same state, and from now on every view of the section has nav[].active moved to that id (the
//       model remembers it across states and revs and rewrites nav[].active on every view it returns;
//       it applies only to views whose nav has that id).
//   select{category} (developers)    only from projects / worktrees / sessions to one of the three
//       (so never when FROZEN); goes to that state. No memory is needed: each state's nav marks its own.
//   toggle{row}                      refused when FROZEN, or when the row is missing, disabled
//       (enabled false: blocked rows) or has check 'none'. Otherwise that row's check goes on<->off
//       (mixed -> on) in the current fixture state; nothing else is recounted.
//   openConfirm (cleanup, developers) | openFix (registry) | openConfirm{action} (services)
//       only from an analyzed-like state AND only when the action is enabled in the current view
//       (cleanup / developers action `clean`; registry `fix`; services the action named by `action`:
//       disable | manual | restore). Records the current state as `back`, clears the Services notice,
//       remembers the services action, goes to the dialog state (confirm; registry: backup). A dialog's
//       ack box is reset to false and its primary button to disabled each time it opens.
//   setAck{on} (developers only)     only when the current view has a dialog with an ack box (ack not
//       null): dialog.ack = on and the primary button enabled = on. Anything else ignores it.
//   confirm (cleanup, services, developers) | fix{backup} (registry) | permanent (developers)
//       run only from the right dialog state AND when the dialog's button for that intent is enabled AND
//       the dialog's ack is null or true:
//         cleanup confirm: state confirm, button `continue`.   services confirm: state confirm, `continue`.
//         developers confirm: state confirm, button `clean`.   developers permanent: state shortfall,
//         button `permanent`.   registry fix{backup}: state backup, button `backup` (true) / `nobackup`.
//       Then: cleanup / developers go to `cleaning`, registry to `fixing` (busy, job.fraction 0, `back`
//       unchanged: the state recorded when the dialog opened; `go` clears it). Services goes straight
//       back to `loaded` with one notice {tone ok, more []} whose text is, by the action:
//         disable  "Selected services disabled."
//         manual   "Selected services set to Manual."
//         restore  "Original startup types restored."
//   closeDialog                      only when the view has a dialog or a report: goes to `back`
//       (the state the dialog was opened from; a report also returns there), else to Home.
//   cancel                           only in a busy state: goes to `back` (the state before the job; a
//       cancelled clean returns to the analysis, not to the dialog), else Home. A cancelled job stops
//       ticking.
//   anything else (search, setOnlyOld, selectAll, setAll, setInactiveDays, ...)   the same state.
// An intent that is refused or does not apply is IGNORED: same state, new rev, nothing else changes.
//
// WHAT NO INTENT DOES (the FakeBackend must not invent it either).
//   developers:shortfall is reachable only through `go`: no intent leads to it (confirm from the confirm
//       dialog goes to `cleaning`; in Rust the shortfall comes from a real disk check the fixture lacks).
//   `cancel` is never sent for Registry and Services: their intent types (RegistryIntent, ServicesIntent)
//       have no `cancel`, and `actionIntent` returns null for it there; the only "Cancel" in Registry is
//       the backup dialog's button, which is `closeDialog`. This mock does not look at the section when
//       it gets a `cancel` (a busy Registry would go back), because the page can never send one; the
//       Rust FakeBackend rejects it at the type level (no such variant).
//   closeDialog with no dialog and no report, cancel when nothing is busy, select of a category that is
//       not in the nav, toggle of a row whose check is `none` (a header: the OTHER WORKTREES row of
//       developers:worktrees), openConfirm from a state that is not analyzed-like: all refused, as above
//       (steps at the end of fixtures/intent-script.json).
//
// SERVICES NOTICE. The notice of `services:loaded` is cleared when a refresh starts, when a dialog opens
// and when `show('services')` is called (the section is shown again); it is set only by a confirm.
//
// `go(s, state)` jumps to a named state (smoke only): resets the tick counter and `back`.

import type { AboutView, AnyIntent, AppInfo, DialogView, SectionId, SectionView } from './types.ts';

export interface Fixture {
  app: AppInfo;
  about: AboutView;
  views: Record<SectionId, Record<string, SectionView>>;
}

const SECTIONS: readonly SectionId[] = ['cleanup', 'registry', 'services', 'developers'];

const INITIAL: Record<SectionId, string> = { cleanup: 'idle', registry: 'idle', services: 'loaded', developers: 'idle' };
const HOME: Record<SectionId, string> = { cleanup: 'analyzed', registry: 'analyzed', services: 'loaded', developers: 'projects' };
const ANALYZED: Record<SectionId, readonly string[]> = {
  cleanup: ['analyzed'], registry: ['analyzed'], services: ['loaded'], developers: ['projects', 'worktrees', 'sessions'],
};
const LANDS: Record<SectionId, Record<string, string>> = {
  cleanup: { analyzing: 'analyzed', cleaning: 'report' },
  registry: { scanning: 'analyzed', fixing: 'fixed' },
  services: { loading: 'loaded' },
  developers: { analyzing: 'projects', cleaning: 'report' },
};
const ANALYZE_BUSY: Record<SectionId, string> = { cleanup: 'analyzing', registry: 'scanning', services: 'loading', developers: 'analyzing' };
const ANALYZE_KIND: Record<SectionId, string> = { cleanup: 'analyze', registry: 'scan', services: 'refresh', developers: 'analyze' };
const OPEN_KIND: Record<SectionId, string> = { cleanup: 'openConfirm', registry: 'openFix', services: 'openConfirm', developers: 'openConfirm' };
const DIALOG: Record<SectionId, string> = { cleanup: 'confirm', registry: 'backup', services: 'confirm', developers: 'confirm' };
const DEV_CATEGORIES = ['projects', 'worktrees', 'sessions'];
const NOTICE: Record<string, string> = {
  disable: 'Selected services disabled.',
  manual: 'Selected services set to Manual.',
  restore: 'Original startup types restored.',
};
const TICKS = 3;

/** What it takes to run: the dialog state, the dialog button and the busy state it starts; null when the intent is not a run. */
function runRule(s: SectionId, intent: AnyIntent): { from: string; button: string; busy: string | null } | null {
  if (s === 'cleanup' && intent.kind === 'confirm') return { from: 'confirm', button: 'continue', busy: 'cleaning' };
  if (s === 'services' && intent.kind === 'confirm') return { from: 'confirm', button: 'continue', busy: null };
  if (s === 'registry' && intent.kind === 'fix') return { from: 'backup', button: intent.backup ? 'backup' : 'nobackup', busy: 'fixing' };
  if (s === 'developers' && intent.kind === 'confirm') return { from: 'confirm', button: 'clean', busy: 'cleaning' };
  if (s === 'developers' && intent.kind === 'permanent') return { from: 'shortfall', button: 'permanent', busy: 'cleaning' };
  return null;
}

export class MockModel {
  private readonly work: Fixture['views'];
  private readonly state: Record<SectionId, string>;
  private readonly revs: Record<SectionId, number>;
  /** Where closeDialog and cancel go back to; undefined = Home. */
  private readonly back: Partial<Record<SectionId, string>> = {};
  private readonly ticks: Record<SectionId, number> = { cleanup: 0, registry: 0, services: 0, developers: 0 };
  /** The Cleanup category chosen by `select`, applied to every view whose nav has it. */
  private readonly active: Partial<Record<SectionId, string>> = {};
  /** The Services action whose dialog is open (decides the notice text). */
  private pendingAction = 'disable';

  constructor(fixture: Fixture) {
    // A working copy: toggles and acks change it, the fixture stays as loaded.
    this.work = structuredClone(fixture.views);
    this.state = { ...INITIAL };
    this.revs = { cleanup: 0, registry: 0, services: 0, developers: 0 };
    for (const s of SECTIONS) {
      if (!this.work[s][this.state[s]]) throw new Error(`the fixture has no ${s}:${this.state[s]}`);
    }
  }

  /** Opens a section: its current view (the Services notice does not outlive the section being left). */
  show(s: SectionId): SectionView {
    if (s === 'services') this.clearNotice();
    return this.stamp(s);
  }

  /** The section's current view. */
  view(s: SectionId): SectionView {
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
    this.state[s] = state;
    this.ticks[s] = 0;
    delete this.back[s];
    return this.stamp(s);
  }

  private current(s: SectionId): SectionView {
    const view = this.work[s][this.state[s]];
    if (!view) throw new Error(`no state ${this.state[s]} for ${s}`);
    return view;
  }

  private stamp(s: SectionId): SectionView {
    this.revs[s] += 1;
    const view: SectionView = { ...structuredClone(this.current(s)), rev: this.revs[s] };
    const active = this.active[s];
    if (active !== undefined && view.nav.some((n) => n.id === active)) {
      view.nav = view.nav.map((n) => ({ ...n, active: n.id === active }));
    }
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
    this.state[s] = state;
    this.ticks[s] = 0;
  }

  private clearNotice(): void {
    const loaded = this.work.services[HOME.services];
    if (loaded) loaded.notices = [];
  }

  private actionEnabled(s: SectionId, id: string): boolean {
    return this.current(s).actions.find((a) => a.id === id)?.enabled === true;
  }

  private apply(s: SectionId, intent: AnyIntent): void {
    const here = this.state[s];
    const kind = intent.kind;
    if (kind === ANALYZE_KIND[s]) {
      if (this.frozen(s)) return;
      this.back[s] = here;
      if (s === 'services') this.clearNotice();
      this.goTo(s, ANALYZE_BUSY[s]);
    } else if (kind === 'select' && (s === 'cleanup' || s === 'developers')) {
      if (this.frozen(s)) return;
      const category = (intent as { category: string }).category;
      if (s === 'developers') {
        if (DEV_CATEGORIES.includes(here) && DEV_CATEGORIES.includes(category)) this.goTo(s, category);
      } else if (this.current(s).nav.some((n) => n.id === category)) {
        this.active[s] = category;
      }
    } else if (kind === 'toggle') {
      if (!this.frozen(s)) this.toggle(s, (intent as { row: string }).row);
    } else if (kind === OPEN_KIND[s]) {
      const action = s === 'services' ? (intent as { action: string }).action : s === 'registry' ? 'fix' : 'clean';
      if (!ANALYZED[s].includes(here) || !this.actionEnabled(s, action)) return;
      this.back[s] = here;
      if (s === 'services') { this.clearNotice(); this.pendingAction = action; }
      this.goTo(s, DIALOG[s]);
      this.resetAck(s);
    } else if (kind === 'setAck' && s === 'developers') {
      this.setAck((intent as { on: boolean }).on);
    } else if (runRule(s, intent) !== null) {
      this.run(s, intent);
    } else if (kind === 'closeDialog') {
      const view = this.current(s);
      if (view.dialog === null && view.report === null) return;
      this.goTo(s, this.back[s] ?? HOME[s]);
    } else if (kind === 'cancel') {
      if (!this.isBusy(s)) return;
      this.goTo(s, this.back[s] ?? HOME[s]);
    }
  }

  private toggle(s: SectionId, id: string): void {
    const row = this.current(s).rows.find((r) => r.id === id);
    if (!row || !row.enabled || row.check === 'none') return;
    row.check = row.check === 'on' ? 'off' : 'on';
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
    if (rule.busy === null) {
      // Services: the change is applied at once and the list comes back with a notice.
      const loaded = this.work[s][HOME[s]];
      if (loaded) loaded.notices = [{ tone: 'ok', text: NOTICE[this.pendingAction] ?? NOTICE.disable ?? '', more: [] }];
      this.goTo(s, HOME[s]);
      return;
    }
    // `back` stays what it was when the dialog opened: cancel and the closed report go there.
    this.goTo(s, rule.busy);
  }
}
