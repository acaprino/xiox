// What the page does with the views Rust sends: the open section, its view, what the user chose in it (the focused row, the folds, the
// order, the inspector), and the intents that go back. Plain TypeScript with no screen in it: the screens (src/screens) read it through
// `useController` and ask it for changes, and it tells them when something changed. The state it holds is `state.ts`'s, which node
// tests; the commands are `api.ts`'s.

import { api, errorText, onView } from './api.ts';
import { actionIntent, toggleIntent } from './intents.ts';
import { cycleSort, nextSort } from './rows.ts';
import { acceptView, createGate, createInflight, createUi, currentFocus, foldedGroups, type UiState } from './state.ts';
import type { AboutView, AnyIntent, AppInfo, RowView, SectionId, SectionView } from './types.ts';
import { actionForKey } from './commands.ts';

/** The views of the window: the sections, and Settings, which is the page's own. */
export type ViewId = SectionId | 'settings';

export class Controller {
  readonly ui: UiState = createUi();
  app: AppInfo | null = null;
  about: AboutView | null = null;
  /** Says a failure to the person (a notice that stays until it is closed); set by the window. */
  fail: (text: string) => void = () => {};
  /** The words of the first command that failed, while there is nothing to draw: the content says them, with a Retry. */
  failure: string | null = null;
  /** Counts the changes of how the list is laid out (folds, order, open parents): the screens memoize on it. */
  layout = 0;
  /** The category the next draw scrolls the list to, once (`takeScroll`). */
  private scroll: string | null = null;
  private version = 0;
  private readonly listeners = new Set<() => void>();
  /** Rapid clicks on the views: only the answer to the last one is drawn. */
  private readonly opening = createGate();
  /** The intents waiting for Rust's answer: a repeat of one of them (a double click) is not sent. */
  private readonly inflight = createInflight();
  private listening = false;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  /** What changes whenever the page's state does: the snapshot of `useSyncExternalStore`. */
  snapshot = (): number => this.version;

  private changed(): void {
    this.version += 1;
    for (const listener of [...this.listeners]) listener();
  }

  private relaid(): void {
    this.layout += 1;
    this.changed();
  }

  /** The open section's view, once there is one. */
  get view(): SectionView | undefined { return this.ui.views[this.ui.section]; }

  /** The view of the window that is open. */
  get open(): ViewId { return this.ui.settings ? 'settings' : this.ui.section; }

  /** The first views: the app's facts, then Cleanup. A failure leaves the content in its error state, with a Retry that asks again. */
  async load(): Promise<void> {
    this.failure = null;
    try {
      this.app = await api.appInfo();
      if (!this.listening) {
        this.listening = true;
        onView((view) => {
          if (acceptView(this.ui, view)) this.changed();
        });
      }
      await this.show('cleanup', true);
    } catch (failure) {
      this.failure = errorText(failure);
    }
    this.changed();
  }

  /** Opens a section. The first one's failure is thrown (`load` keeps it); any other is said as a notice, and the section stays as it was. */
  async show(section: SectionId, first = false): Promise<void> {
    let failed: unknown = null;
    await this.opening.run(
      () => api.show(section),
      (view) => {
        acceptView(this.ui, view);
        this.ui.section = section;
        this.ui.settings = false;
      },
      (error) => { failed = error; },
    );
    if (failed !== null) {
      if (first) throw failed;
      this.fail(errorText(failed));
    }
    this.changed();
  }

  /** The person chose a view of the window. */
  choose(view: ViewId): void {
    if (view === 'settings') void this.openSettings();
    else if (view !== this.ui.section || this.ui.settings) void this.show(view);
  }

  /** Settings, the fourth view: the page's own, with no intent. A confirmation that is open keeps the keyboard, so Settings waits. */
  async openSettings(): Promise<void> {
    if (this.ui.settings || this.view?.dialog != null) return;
    try {
      this.about = await api.about();
    } catch (failure) {
      this.fail(errorText(failure));
      return;
    }
    this.ui.settings = true;
    this.ui.inspector = null;
    this.changed();
  }

  /** Sends an intent to a section's command and takes the view that answers; a null intent (an id the section does not send) is ignored. */
  async send(section: SectionId, intent: AnyIntent | null): Promise<void> {
    if (intent === null || !this.inflight.begin(section, intent)) return;
    try {
      acceptView(this.ui, await api.intent(section, intent));
    } catch (failure) {
      this.fail(errorText(failure));
    } finally {
      this.inflight.end(section, intent);
    }
    this.changed();
  }

  /** The primary action (Enter on the list), the one Rust gave the key Enter. */
  primary(): void {
    const view = this.view;
    const action = view === undefined ? null : actionForKey(view, 'Enter');
    if (view !== undefined && action !== null) void this.send(view.section, actionIntent(view.section, action.id));
  }

  /** The row the selection is on: the details follow it. */
  focusRow(id: string): void {
    if (this.ui.focus[this.ui.section] === id) return;
    this.ui.focus[this.ui.section] = id;
    this.changed();
  }

  /** The check of a row was pressed: the row takes the focus and its tick is asked for (a category's id ticks the whole category). */
  toggleRow(id: string, section: SectionId): void {
    this.ui.focus[section] = id;
    void this.send(section, toggleIntent(section, id));
  }

  /** The caret of a row, or Left and Right on it: a category and a folder fold, a parent row opens its rows. */
  setOpen(row: RowView, open: boolean): void {
    if (row.kind === 'group' || row.kind === 'header') this.ui.fold.set(row.id, !open);
    else if (open) this.ui.expanded.add(row.id);
    else this.ui.expanded.delete(row.id);
    this.ui.focus[this.ui.section] = row.id;
    this.relaid();
  }

  /** «Fold all» / «Open all» of a list of categories. */
  foldAll(fold: boolean): void {
    const view = this.view;
    if (view === undefined) return;
    for (const row of view.rows) if (row.kind === 'group') this.ui.fold.set(row.id, fold);
    this.relaid();
  }

  /** A click on a column's header: the list is ordered by that column, or the order turns round. */
  sortBy(column: string): void {
    const view = this.view;
    if (view === undefined) return;
    this.setSort(view, nextSort(view.columns, this.ui.sort[view.section] ?? null, column));
  }

  /** The order by the next column, or the same one turned round (`s`, `Shift+S`). */
  cycleSort(reverse: boolean): void {
    const view = this.view;
    if (view !== undefined) this.setSort(view, cycleSort(view.columns, this.ui.sort[view.section] ?? null, reverse));
  }

  private setSort(view: SectionView, key: ReturnType<typeof nextSort>): void {
    if (key === null) delete this.ui.sort[view.section]; else this.ui.sort[view.section] = key;
    this.relaid();
  }

  /** The inspector on the details of the focused row, or on the section's options; the same one again closes it. */
  toggleInspector(kind: 'details' | 'options'): void {
    this.ui.inspector = this.ui.inspector === kind ? null : kind;
    this.changed();
  }

  closeInspector(): void {
    if (this.ui.inspector === null) return;
    this.ui.inspector = null;
    this.changed();
  }

  /** The menu of the categories: the list is scrolled to one of them, which is opened if it was folded. */
  goToGroup(id: string): void {
    const view = this.view;
    if (view === undefined) return;
    if (foldedGroups(this.ui, view).has(id)) this.ui.fold.set(id, false);
    this.ui.focus[this.ui.section] = id;
    this.ui.spy[this.ui.section] = id;
    this.scroll = id;
    this.relaid();
  }

  /** The category the list is to be scrolled to now, once. */
  takeScroll(): string | null {
    const id = this.scroll;
    this.scroll = null;
    return id;
  }

  /** The category the list is at, as the menu marks it. */
  setSpy(id: string): void {
    const section = this.ui.section;
    if (this.ui.spy[section] === id) return;
    this.ui.spy[section] = id;
    this.changed();
  }

  /** «Save as text…» of a report: the save dialog opens in Rust and the answer is the section's view with `report.saved`. */
  async saveReport(section: SectionId): Promise<void> {
    try {
      acceptView(this.ui, await api.saveReport(section));
    } catch (failure) {
      this.fail(errorText(failure));
    }
    this.changed();
  }

  /** The folder picker of Developers: it opens in Rust and the page never sees a path. */
  async addRoot(): Promise<void> {
    try {
      acceptView(this.ui, await api.addRoot());
    } catch (failure) {
      this.fail(errorText(failure));
    }
    this.changed();
  }

  /** What the details describe: the focused row, unless it is a category or a sub-heading (their own line says it all). */
  detailRow(view: SectionView, shown: readonly RowView[]): RowView | null {
    const focus = currentFocus(this.ui, view);
    const row = shown.find((r) => r.id === focus) ?? null;
    return row === null || row.kind === 'group' || row.kind === 'header' ? null : row;
  }
}
