// The keyboard as a table: which key means what, given whether a text field or a dialog has the
// keyboard. No DOM here, so node can test it; main.ts reads the event, asks `keyAction` and acts.
// The page only presses what the view already offers (`actionForKey` hands back an ENABLED action):
// whether the intent is allowed is still Rust's decision.

import { SECTION_ORDER } from './state.ts';
import type { ActionView, SectionId, SectionView } from './types.ts';

export interface KeyLike { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean }

/** The keys an action can carry (`ActionView.key`): `a` analyze, `s` scan, `r` refresh, Enter clean / fix, Esc cancel. */
export type ActionKey = 'a' | 's' | 'r' | 'Enter' | 'Esc';

export type KeyAction =
  | { kind: 'section'; section: SectionId } | { kind: 'move'; delta: number } | { kind: 'toggle' }
  | { kind: 'expand'; open: boolean } | { kind: 'action'; key: ActionKey } | { kind: 'closeDialog' };

const ACTION_KEYS: ReadonlySet<string> = new Set(['a', 's', 'r', 'Enter']);

/**
 * What a key press means. `inField`: a text field has the focus; `dialog`: a confirmation is open.
 * A chord (Ctrl, Alt, Meta) is never ours. Escape is the only key a field or a dialog leaves to us:
 * it closes the dialog, else it is the `Esc` action (main.ts turns it into a blur inside a field).
 * In a dialog the browser does the rest: Enter / Space press the focused button, Tab is trapped.
 */
export function keyAction(e: KeyLike, ctx: { inField: boolean; dialog: boolean }): KeyAction | null {
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  if (e.key === 'Escape') return ctx.dialog ? { kind: 'closeDialog' } : { kind: 'action', key: 'Esc' };
  if (ctx.inField || ctx.dialog) return null;
  const digit = /^[1-4]$/.test(e.key) ? SECTION_ORDER[Number(e.key) - 1] : undefined;
  if (digit !== undefined) return { kind: 'section', section: digit };
  switch (e.key) {
    case 'ArrowDown': return { kind: 'move', delta: 1 };
    case 'ArrowUp': return { kind: 'move', delta: -1 };
    case 'PageDown': return { kind: 'move', delta: 10 };
    case 'PageUp': return { kind: 'move', delta: -10 };
    case 'Home': return { kind: 'move', delta: -Infinity };
    case 'End': return { kind: 'move', delta: Infinity };
    case ' ': return { kind: 'toggle' };
    case 'ArrowRight': return { kind: 'expand', open: true };
    case 'ArrowLeft': return { kind: 'expand', open: false };
    default: return ACTION_KEYS.has(e.key) ? { kind: 'action', key: e.key as ActionKey } : null;
  }
}

/** The Esc key as an action carries it: Rust writes `Esc`; `Escape` is accepted too (the key's own name). */
const isEsc = (key: string | null): boolean => key === 'Esc' || key === 'Escape';

/** The view's enabled action that carries `key`. */
export function actionForKey(view: SectionView, key: ActionKey): ActionView | null {
  return view.actions.find((a) => a.enabled && (key === 'Esc' ? isEsc(a.key) : a.key === key)) ?? null;
}

const KEY_WORDS: Record<string, string> = { Enter: '↵' };

/**
 * The footer's key hints for the section on screen: movement and Space, then every action that has a
 * key (`a analyze · ↵ clean`, Registry `s scan · ↵ fix`, Services `r refresh`). Esc is left out so the
 * hint does not change while a job runs. Derived from the actions, so the hint and the keys agree.
 */
export function footerHint(view: SectionView): string {
  const keyed = view.actions.filter((a) => a.key !== null && !isEsc(a.key)).map((a) => `${KEY_WORDS[a.key as string] ?? a.key} ${a.id}`);
  return ['↑↓ move', 'space toggle', ...keyed].join(' · ');
}

/**
 * Where Tab goes inside an open dialog: the index among its `count` focusable controls, wrapping at
 * both ends. `current` is the focused one's index, or -1 when the focus is outside the dialog (then
 * Tab lands on the first control and Shift+Tab on the last). Null when there is nothing to focus.
 */
export function cycleFocus(count: number, current: number, backwards: boolean): number | null {
  if (count <= 0) return null;
  if (current < 0 || current >= count) return backwards ? count - 1 : 0;
  return (current + (backwards ? count - 1 : 1)) % count;
}
