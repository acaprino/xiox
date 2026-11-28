// Small pure decisions of the views, apart from the screens so that tests/policy.test.ts can state them: why an action is
// disabled, which action an empty list offers. (Whether a confirmation deletes for good is the view's `permanent`, not a reading of its words.)

import type { ActionView, DialogView, OptionView, SectionView } from './types.ts';

type ChoiceOption = Extract<OptionView, { kind: 'choice' }>;

/** The layout of the list (Tree / Flat), a choice of the panel's header: Developers sends it, and only its projects have one. */
export function layoutChoice(view: Pick<SectionView, 'nav' | 'options'>): ChoiceOption | null {
  if (view.nav.find((n) => n.active)?.id !== 'projects') return null;
  return view.options.find((o): o is ChoiceOption => o.kind === 'choice' && o.id === 'layout') ?? null;
}

/** The layout as one small button beside Options, pressed for the flat list: its label, whether it is pressed, what a press asks for. */
export function flatToggle(layout: ChoiceOption): { label: string; pressed: boolean; next: string } {
  const flat = layout.value === 'flat';
  return { label: layout.choices.find((c) => c.id === 'flat')?.label ?? 'Flat', pressed: flat, next: flat ? 'tree' : 'flat' };
}

/**
 * The options the inspector shows: all of the section's, but the layout of the list (in the panel's header) and Registry's kinds
 * of issue, which are its categories in the sidebar.
 */
export function panelOptions(view: Pick<SectionView, 'nav' | 'section' | 'options'>): OptionView[] {
  const kinds = view.nav.length === 0 && view.section === 'registry';
  return view.options.filter((o) => !(kinds && o.kind === 'toggle') && !(o.kind === 'choice' && o.id === 'layout'));
}

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
