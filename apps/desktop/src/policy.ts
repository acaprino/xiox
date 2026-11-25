// Small pure decisions of the views, apart from the screens so that tests/policy.test.ts can state them: why an action is
// disabled, which action an empty list offers. (Whether a confirmation deletes for good is the view's `permanent`, not a reading of its words.)

import type { ActionView, DialogView, SectionView } from './types.ts';

/** The actions that start the work of a section: an empty list offers the one that is enabled. */
const START_ACTIONS: ReadonlySet<string> = new Set(['analyze', 'scan']);

/** Why an action cannot be pressed now, in a sentence; null when it can (or when a dialog covers the window anyway). */
export function disabledReason(view: SectionView, action: ActionView): string | null {
  if (action.enabled || view.dialog !== null) return null;
  if (view.job !== null) return `Wait for “${view.job.label}” to finish, or cancel it.`;
  const start = view.actions.find((a) => START_ACTIONS.has(a.id) && a.enabled);
  return start ? `Run “${start.label}” and tick at least one item first.` : 'Tick at least one item first.';
}

/** The action an empty list offers as a button: the enabled one that starts the section's work, if any. */
export function emptyAction(view: SectionView): ActionView | null {
  return view.actions.find((a) => START_ACTIONS.has(a.id) && a.enabled) ?? null;
}
