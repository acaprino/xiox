// What the page remembers: the open section and, per section, the view with the highest `rev`.

import type { AppInfo, SectionId, SectionView } from './types.ts';

export const SECTION_ORDER: readonly SectionId[] = ['cleanup', 'registry', 'services', 'developers'];

export const state: { app: AppInfo | null; open: SectionId; views: Partial<Record<SectionId, SectionView>> } = {
  app: null,
  open: 'cleanup',
  views: {},
};

/** Keeps `view` when it is newer than what is held; false for a stale or repeated revision. */
export function accept(view: SectionView): boolean {
  const held = state.views[view.section];
  if (held && held.rev >= view.rev) return false;
  state.views[view.section] = view;
  return true;
}
