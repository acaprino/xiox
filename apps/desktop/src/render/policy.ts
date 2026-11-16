// Small pure decisions of the views, apart from the DOM so that tests/policy.test.ts can state them: what a destructive
// confirmation says and where its focus starts, why an action is disabled, which action an empty list offers.

import type { ActionView, DialogView, SectionView } from '../types.ts';

/** The words of a dialog that say the delete cannot be taken back. */
const IRREVERSIBLE = /permanent|cannot be undone/i;

/** The actions that start the work of a section: an empty list offers the one that is enabled. */
const START_ACTIONS: ReadonlySet<string> = new Set(['analyze', 'scan']);

/** A confirmation whose primary button deletes for good: its focus starts on Cancel and its button is the danger variant. */
export function permanentDelete(dialog: DialogView): boolean {
  if (dialog.id !== 'confirm') return false;
  const primary = dialog.buttons.find((b) => b.primary);
  if (primary?.id === 'permanent') return true;
  return [...dialog.text, ...dialog.warnings].some((line) => IRREVERSIBLE.test(line));
}

/**
 * What the confirmation says about the size of the job. The view's own `summary` wins; a section whose view leaves it empty
 * (Cleanup's) gets the label of its clean action ("Clean 8 items") and its status sentence ("13.48 GB to delete (46,999 files)").
 */
export function summaryLines(view: SectionView, dialog: DialogView): string[] {
  if (dialog.summary.length > 0) return dialog.summary;
  if (view.section !== 'cleanup' || dialog.id !== 'confirm') return [];
  const clean = view.actions.find((a) => a.id === 'clean');
  return [clean?.label, view.status.text].filter((line): line is string => Boolean(line));
}

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
