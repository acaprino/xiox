// The scripted stand-in for the Rust core: serves the fixture views and their transitions.
// The Rust `FakeBackend` (Task 11) implements the same transition table over the same fixture.
// No DOM and no Vite here, so `node --experimental-strip-types --test` can load it.
//
//   intent                         from                          to
//   show(s)                        -                             the current state (initial: cleanup:idle,
//                                                                registry:idle, services:loaded, developers:idle)
//   analyze / scan / refresh       not busy                      the busy state; each tick pushes it with
//                                                                job.fraction 1/3, 2/3, 1; the tick after lands
//                                                                on analyzed (Developers projects, Services loaded)
//   select{category}  Developers   projects/worktrees/sessions   that state
//   select{category}  other        any                           same state, nav[].active moved
//   toggle{row}                    any                           same state, that row on<->off (mixed -> on)
//   openConfirm / openFix          analyzed-like                 confirm (Registry: backup)
//   setAck{on}                     a dialog with an ack box      same, dialog.ack = on, primary button enabled = on
//   confirm / fix / permanent      confirm / backup / shortfall  cleaning / fixing for 3 ticks, then report
//                                                                (Registry: fixed); Services goes back to loaded
//                                                                at once with an ok notice
//   closeDialog                    a dialog or a report          the state it was opened from
//   cancel                         a busy state                  the state before the job
//   anything else                  -                             unchanged
//
// `rev` grows by one on every view the model returns.

import type { AboutView, AnyIntent, AppInfo, DialogView, SectionId, SectionView } from './types.ts';

export interface Fixture {
  app: AppInfo;
  about: AboutView;
  views: Record<SectionId, Record<string, SectionView>>;
}

const SECTIONS: readonly SectionId[] = ['cleanup', 'registry', 'services', 'developers'];

const INITIAL: Record<SectionId, string> = { cleanup: 'idle', registry: 'idle', services: 'loaded', developers: 'idle' };
/** The analysed state a job or a dialog falls back to when it was reached with `go`. */
const HOME: Record<SectionId, string> = { cleanup: 'analyzed', registry: 'analyzed', services: 'loaded', developers: 'projects' };
/** Analyzed-like states: where a confirmation can be opened. */
const ANALYZED: Record<SectionId, readonly string[]> = {
  cleanup: ['analyzed'], registry: ['analyzed'], services: ['loaded'], developers: ['projects', 'worktrees', 'sessions'],
};
/** Busy state -> the state it lands on after its three ticks. */
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
/** Dialog states a confirm / fix / permanent can run from, and the intent kinds each section accepts. */
const RUNNABLE: Record<SectionId, readonly string[]> = {
  cleanup: ['confirm'], registry: ['backup'], services: ['confirm'], developers: ['confirm', 'shortfall'],
};
const RUN_KINDS: Record<SectionId, readonly string[]> = {
  cleanup: ['confirm'], registry: ['fix'], services: ['confirm'], developers: ['confirm', 'permanent'],
};
const RUN_BUSY: Record<SectionId, string | null> = { cleanup: 'cleaning', registry: 'fixing', services: null, developers: 'cleaning' };
const DEV_CATEGORIES = ['projects', 'worktrees', 'sessions'];
const TICKS = 3;

export class MockModel {
  private readonly work: Fixture['views'];
  private readonly state: Record<SectionId, string>;
  private readonly revs: Record<SectionId, number>;
  /** Where closeDialog and cancel go back to; undefined = HOME. */
  private readonly back: Partial<Record<SectionId, string>> = {};
  private readonly ticks: Record<SectionId, number> = { cleanup: 0, registry: 0, services: 0, developers: 0 };
  private readonly active: Partial<Record<SectionId, string>> = {};

  constructor(fixture: Fixture) {
    // A working copy: toggles and acks change it, the fixture stays as loaded.
    this.work = structuredClone(fixture.views);
    this.state = { ...INITIAL };
    this.revs = { cleanup: 0, registry: 0, services: 0, developers: 0 };
    for (const s of SECTIONS) {
      if (!this.work[s][this.state[s]]) throw new Error(`the fixture has no ${s}:${this.state[s]}`);
    }
  }

  /** Opens a section: the view it was left in. */
  show(s: SectionId): SectionView {
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
    if (view.job) {
      const landing = LANDS[s][this.state[s]];
      // The fixture's own fraction is the starting point; ticks move it.
      view.job = { ...view.job, fraction: landing === undefined ? view.job.fraction : Math.min(this.ticks[s], TICKS) / TICKS };
    }
    return view;
  }

  private isBusy(s: SectionId): boolean {
    return LANDS[s][this.state[s]] !== undefined;
  }

  private goTo(s: SectionId, state: string): void {
    if (!this.work[s][state]) throw new Error(`no state ${state} for ${s}`);
    this.state[s] = state;
    this.ticks[s] = 0;
  }

  private apply(s: SectionId, intent: AnyIntent): void {
    const here = this.state[s];
    const kind = intent.kind;
    if (kind === ANALYZE_KIND[s]) {
      if (this.isBusy(s)) return;
      this.back[s] = here;
      this.goTo(s, ANALYZE_BUSY[s]);
    } else if (kind === 'select' && (s === 'cleanup' || s === 'developers')) {
      const category = (intent as { category: string }).category;
      if (s === 'developers') {
        if (DEV_CATEGORIES.includes(here) && DEV_CATEGORIES.includes(category)) this.goTo(s, category);
      } else if (this.current(s).nav.some((n) => n.id === category)) {
        this.active[s] = category;
      }
    } else if (kind === 'toggle') {
      this.toggle(s, (intent as { row: string }).row);
    } else if (kind === OPEN_KIND[s]) {
      if (!ANALYZED[s].includes(here)) return;
      this.back[s] = here;
      this.goTo(s, DIALOG[s]);
    } else if (kind === 'setAck' && s === 'developers') {
      this.setAck((intent as { on: boolean }).on);
    } else if (RUN_KINDS[s].includes(kind)) {
      if (!RUNNABLE[s].includes(here)) return;
      this.run(s);
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

  private run(s: SectionId): void {
    const busy = RUN_BUSY[s];
    if (busy === null) {
      // Services: the change is applied at once and the list comes back with a notice.
      const loaded = this.work[s][HOME[s]];
      if (loaded) loaded.notices = [{ tone: 'ok', text: 'Startup type changed for the selected services.', more: [] }];
      this.goTo(s, HOME[s]);
      return;
    }
    // `back` stays what it was when the dialog opened: cancel and the closed report go there.
    this.goTo(s, busy);
  }
}
