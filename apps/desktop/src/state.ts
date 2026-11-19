// What the page remembers: the open section, per section the view with the highest `rev`, the focused
// row and the expanded parents. No DOM here, so node can test it.

import { groupSizes, orderFor, rowParents, sortRows, visibleRows, type SortKey } from './rows.ts';
import type { Check, RowView, SectionId, SectionView } from './types.ts';

export const SECTION_ORDER: readonly SectionId[] = ['cleanup', 'registry', 'developers'];

export interface UiState {
  section: SectionId;
  views: Partial<Record<SectionId, SectionView>>;
  /** The focused row of each section (what the detail panel describes). */
  focus: Partial<Record<SectionId, string>>;
  /** Parents whose children are shown; every parent starts collapsed. */
  expanded: Set<string>;
  /** What the user chose for a category (`group` row id) or a folder (a header that is a parent): true = folded, false = open. A category without an entry follows `LONG_GROUP`, a folder starts open. */
  fold: Map<string, boolean>;
  /** The category the list is scrolled to, per section (the menu marks it); set by the scroll and by a click on the menu. */
  spy: Partial<Record<SectionId, string>>;
  /** The column and direction each section's list is ordered by; none: the default, by size from the largest. */
  sort: Partial<Record<SectionId, SortKey>>;
  /** Whether the details of the focused row are open (Smooth: closed until asked for; ONI MODE always shows them). */
  details: boolean;
}

export function createUi(): UiState {
  return { section: 'cleanup', views: {}, focus: {}, expanded: new Set(), fold: new Map(), spy: {}, sort: {}, details: false };
}

/** Keeps `v` when it is newer than what is held; false for a stale or repeated revision. */
export function acceptView(ui: UiState, v: SectionView): boolean {
  const held = ui.views[v.section];
  if (held && held.rev >= v.rev) return false;
  ui.views[v.section] = v;
  return true;
}

/**
 * A row the selection can rest on: any row but a plain sub-heading (one with a box is a row of its own: Space ticks it;
 * a folder, in `parents` with the rows that have rows under them, too: the arrows fold it).
 */
export const focusable = (row: RowView, parents: ReadonlySet<string> = new Set()): boolean =>
  row.kind !== 'header' || row.check !== 'none' || parents.has(row.id);

/**
 * The row after moving `delta` rows from `current` over the rows on screen: plain sub-headings are skipped
 * and the ends clamp. Without a current row (or one no longer listed) it is the first row going down
 * and the last going up. Null when there is no row to land on.
 */
export function moveFocus(rows: RowView[], current: string | null, delta: number, parents: ReadonlySet<string> = new Set()): string | null {
  const ids = rows.filter((r) => focusable(r, parents)).map((r) => r.id);
  if (ids.length === 0) return null;
  const at = current === null ? -1 : ids.indexOf(current);
  if (at < 0) return (delta < 0 ? ids[ids.length - 1] : ids[0]) ?? null;
  return ids[Math.min(ids.length - 1, Math.max(0, at + delta))] ?? null;
}

/** A category with more rows than this starts folded (Winapp2 has thousands): its header says what is inside, one click opens it. */
export const LONG_GROUP = 40;

/** Whether the search box has text: a search shows what it found, so no category stays folded over a match. */
export const searching = (view: SectionView): boolean => (view.search?.value.trim() ?? '') !== '';

/** The ids of the categories and folders whose rows are not drawn. */
export function foldedGroups(ui: UiState, view: SectionView): Set<string> {
  const folded = new Set<string>();
  if (searching(view)) return folded;
  for (const [id, size] of groupSizes(view.rows)) {
    if (ui.fold.get(id) ?? size > LONG_GROUP) folded.add(id);
  }
  const parents = rowParents(view.rows);
  for (const row of view.rows) if (row.kind === 'header' && parents.has(row.id) && ui.fold.get(row.id) === true) folded.add(row.id);
  return folded;
}

/** The rows on screen, in drawing order: in the section's order, children under an open parent, nothing inside a folded category. */
export const shownRows = (ui: UiState, view: SectionView): RowView[] =>
  visibleRows(sortRows(view.rows, orderFor(view.columns, ui.sort[view.section] ?? null)), ui.expanded, foldedGroups(ui, view));

/** The focused row of the section as the list shows it: the chosen one if it is on screen, else the first. */
export function currentFocus(ui: UiState, view: SectionView): string | null {
  const shown = shownRows(ui, view);
  const parents = rowParents(view.rows);
  const chosen = ui.focus[view.section];
  if (chosen !== undefined && shown.some((r) => r.id === chosen && focusable(r, parents))) return chosen;
  return moveFocus(shown, null, 1, parents);
}

/**
 * The state of the check box at the top of the list: «all» when every category (or, in a plain list, every row that can be
 * ticked) is on, «none» when nothing can be ticked. It only shows what the rows say; the click sends `selectAll`.
 */
export function bulkCheck(view: SectionView): Check {
  const grouped = view.rows.some((r) => r.kind === 'group');
  // An item is a top row, or a project in its folder; a child (a project's folder to clean) is not one of the boxes.
  const boxes = view.rows.filter((r) => (grouped ? r.kind === 'group' : r.kind === 'item') && r.check !== 'none' && r.enabled);
  if (boxes.length === 0) return 'none';
  if (boxes.every((r) => r.check === 'on')) return 'on';
  return boxes.every((r) => r.check === 'off') ? 'off' : 'mixed';
}

export interface Gate {
  /**
   * Runs `load` and hands its result to `apply`, unless another `run` started meanwhile: a slow answer
   * to an earlier call (rapid clicks on the section bar) is dropped, and so is its failure. True when
   * `apply` ran.
   */
  run<T>(load: () => Promise<T>, apply: (value: T) => void, fail?: (error: unknown) => void): Promise<boolean>;
}

export function createGate(): Gate {
  let latest = 0;
  return {
    async run<T>(load: () => Promise<T>, apply: (value: T) => void, fail?: (error: unknown) => void): Promise<boolean> {
      latest += 1;
      const mine = latest;
      try {
        const value = await load();
        if (mine !== latest) return false;
        apply(value);
        return true;
      } catch (error) {
        if (mine === latest) fail?.(error);
        return false;
      }
    },
  };
}

/** Intents on their way to Rust: a second identical one (a double click on a dialog button) is dropped, not sent. */
export interface Inflight {
  /** True when the intent may go; false when the same one is already waiting for its answer. */
  begin(section: SectionId, intent: unknown): boolean;
  /** The answer (or the failure) came back: the same intent may be sent again. */
  end(section: SectionId, intent: unknown): void;
}

export function createInflight(): Inflight {
  const sent = new Set<string>();
  const key = (section: SectionId, intent: unknown): string => `${section}:${JSON.stringify(intent)}`;
  return {
    begin(section, intent) {
      const k = key(section, intent);
      if (sent.has(k)) return false;
      sent.add(k);
      return true;
    },
    end(section, intent) { sent.delete(key(section, intent)); },
  };
}

/** All / None follow the check boxes: while a job runs the list is frozen and Rust refuses a bulk change. */
export const bulkLocked = (view: SectionView): boolean => view.job !== null;

/** The key under which the page remembers that a notice's extra lines are open (`UiState.expanded` holds it with the rows' keys). */
export const noticeKey = (text: string): string => `notice:${text}`;
