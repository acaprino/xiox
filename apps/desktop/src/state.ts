// What the page remembers: the open section, per section the view with the highest `rev`, the focused
// row and the expanded parents. No DOM here, so node can test it.

import { sortRows, visibleRows } from './rows.ts';
import type { RowView, SectionId, SectionView } from './types.ts';

export const SECTION_ORDER: readonly SectionId[] = ['cleanup', 'registry', 'services', 'developers'];

export interface UiState {
  section: SectionId;
  views: Partial<Record<SectionId, SectionView>>;
  /** The focused row of each section (what the detail panel describes). */
  focus: Partial<Record<SectionId, string>>;
  /** Parents whose children are shown; every parent starts collapsed. */
  expanded: Set<string>;
}

export function createUi(): UiState {
  return { section: 'cleanup', views: {}, focus: {}, expanded: new Set() };
}

/** Keeps `v` when it is newer than what is held; false for a stale or repeated revision. */
export function acceptView(ui: UiState, v: SectionView): boolean {
  const held = ui.views[v.section];
  if (held && held.rev >= v.rev) return false;
  ui.views[v.section] = v;
  return true;
}

/**
 * The row after moving `delta` rows from `current` over the rows on screen: header rows are skipped
 * and the ends clamp. Without a current row (or one no longer listed) it is the first row going down
 * and the last going up. Null when there is no row to land on.
 */
export function moveFocus(rows: RowView[], current: string | null, delta: number): string | null {
  const ids = rows.filter((r) => r.kind !== 'header').map((r) => r.id);
  if (ids.length === 0) return null;
  const at = current === null ? -1 : ids.indexOf(current);
  if (at < 0) return (delta < 0 ? ids[ids.length - 1] : ids[0]) ?? null;
  return ids[Math.min(ids.length - 1, Math.max(0, at + delta))] ?? null;
}

/** The focused row of the section as the list shows it: the chosen one if it is on screen, else the first. */
export function currentFocus(ui: UiState, view: SectionView): string | null {
  const shown = visibleRows(sortRows(view.rows), ui.expanded);
  const chosen = ui.focus[view.section];
  if (chosen !== undefined && shown.some((r) => r.id === chosen && r.kind !== 'header')) return chosen;
  return moveFocus(shown, null, 1);
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
