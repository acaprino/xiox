// The keys of the app's own, decided apart from the screens so that node can test them: the keys of the list, the action that a key
// asks of a view, the keys the foot names, and the hold a confirmation keeps on Enter and Space for a moment after it opens.
// The commands themselves are Sigrid's `useCommand` (src/screens/commands.tsx): they give the keys of the window (`a` Analyze, `s` the
// order, `i` Details, `o` Options, Escape) their names in the palette (Ctrl+K), and the shell's views and look come with the shell.
// Enter is never a command: it presses the focused button, and on the list it presses the primary action (`listKey`).

import type { ActionView, SectionView } from './types.ts';

export interface KeyLike { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean; repeat?: boolean }

/** The keys an action can carry (`ActionView.key`): `a` is Analyze and Find issues (Registry); Enter is Clean / Fix; Esc cancels a job. */
export type ActionKey = 'a' | 'Enter' | 'Esc';

/** A key of the foot that is not a command of the registry: its key as the page writes it, and its name. */
export interface Hint { keys: string; label: string }

/** The ids of the commands that are always there. */
export const COMMAND = {
  sort: 'xiox.sort',
  sortReverse: 'xiox.sort.reverse',
  details: 'xiox.details',
  options: 'xiox.options',
  escape: 'xiox.escape',
} as const;

/** The id of the command of a view's action. */
export const actionCommandId = (action: ActionView): string => `xiox.action.${action.id}`;

/** The key an action puts on a command: Enter (the focused button's own) and Esc (the Escape command) are not bound by the action. */
export function actionKeys(action: ActionView): string | undefined {
  return action.key === null || action.key === 'Enter' || action.key === 'Esc' || action.key === 'Escape' ? undefined : action.key;
}

const isEsc = (key: string | null): boolean => key === 'Esc' || key === 'Escape';

/** The view's enabled action that carries `key`. */
export function actionForKey(view: SectionView, key: ActionKey): ActionView | null {
  return view.actions.find((a) => a.enabled && (key === 'Esc' ? isEsc(a.key) : a.key === key)) ?? null;
}

/**
 * What the foot lists after the shell's own line, in order: the list's keys, the sort, then each action with its key (Enter as its
 * own key, not a command). A command is named by its id: the foot shows it only while it has a key and can be done.
 */
export function footerEntries(view: SectionView): Array<string | Hint> {
  const entries: Array<string | Hint> = [{ keys: '↑ ↓', label: 'Move' }, { keys: 'Space', label: 'Tick' }, { keys: '← →', label: 'Fold' }, COMMAND.sort];
  for (const action of view.actions) {
    // Enter is named while it can press something: a disabled Clean answers nothing.
    if (action.key === 'Enter') {
      if (action.enabled) entries.push({ keys: 'Enter', label: action.label });
    } else if (actionKeys(action) !== undefined) entries.push(actionCommandId(action));
  }
  return entries;
}

export type ListKey =
  | { kind: 'move'; delta: number } | { kind: 'toggle' } | { kind: 'expand'; open: boolean } | { kind: 'primary' };

/**
 * The list's own keys, on its `keydown`: the arrows, PageUp and PageDown, Home and End move the selection, Left and Right fold or open,
 * Space ticks the row, Enter presses the primary action. A chord is never the list's. A key held down (`repeat`) presses nothing
 * (one Enter is one press, so a held Enter cannot run on past the dialog it opened); the arrows keep repeating, that is how a long
 * list is crossed. The table moves and folds by itself; the page takes only `toggle` and `primary`.
 */
export function listKey(e: KeyLike): ListKey | null {
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  switch (e.key) {
    case 'ArrowDown': return { kind: 'move', delta: 1 };
    case 'ArrowUp': return { kind: 'move', delta: -1 };
    case 'PageDown': return { kind: 'move', delta: 10 };
    case 'PageUp': return { kind: 'move', delta: -10 };
    case 'Home': return { kind: 'move', delta: -Infinity };
    case 'End': return { kind: 'move', delta: Infinity };
    case 'ArrowRight': return { kind: 'expand', open: true };
    case 'ArrowLeft': return { kind: 'expand', open: false };
    case ' ': return e.repeat === true ? null : { kind: 'toggle' };
    case 'Enter': return e.repeat === true ? null : { kind: 'primary' };
    default: return null;
  }
}

/** How long after a confirmation opens its buttons ignore Enter and Space (a mouse click is never held back). */
export const DIALOG_ARM_MS = 300;

/**
 * Whether a key press on a control of an open dialog must be swallowed before the browser turns it into a click:
 * Enter or Space that is held down (`repeat`), or that comes within `DIALOG_ARM_MS` of the dialog opening (the
 * Enter that opened it, repeating or pressed twice in a hurry, must not also confirm it). Escape and Tab are
 * never swallowed. `ageMs`: time since the dialog opened.
 */
export function dialogKeySwallowed(e: { key: string; repeat?: boolean }, ageMs: number): boolean {
  if (e.key !== 'Enter' && e.key !== ' ') return false;
  return e.repeat === true || ageMs < DIALOG_ARM_MS;
}
