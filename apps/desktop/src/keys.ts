// The keyboard as a table: which key means what, given whether a text field or a dialog has the
// keyboard. No DOM here, so node can test it; main.ts reads the event, asks `keyAction` and acts.
// The page only presses what the view already offers (`actionForKey` hands back an ENABLED action):
// whether the intent is allowed is still Rust's decision.

import { SECTION_ORDER } from './state.ts';
import type { ActionView, SectionId, SectionView } from './types.ts';

export interface KeyLike { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean; repeat?: boolean }

/** The keys an action can carry (`ActionView.key`): `a` is Analyze and Find issues (Registry); Enter is Clean / Fix; Esc cancels a job. */
export type ActionKey = 'a' | 'Enter' | 'Esc';

export type KeyAction =
  | { kind: 'section'; section: SectionId } | { kind: 'move'; delta: number } | { kind: 'toggle' }
  | { kind: 'expand'; open: boolean } | { kind: 'action'; key: ActionKey } | { kind: 'closeDialog' } | { kind: 'about' }
  | { kind: 'sort'; reverse: boolean } | { kind: 'details' };

const ACTION_KEYS: ReadonlySet<string> = new Set(['a', 'Enter']);

/**
 * What a key press means. `inField`: a text field has the focus; `dialog`: a confirmation is open.
 * A chord (Ctrl, Alt, Meta) is never ours. Escape is the only key a field or a dialog leaves to us:
 * it closes the dialog (the About window counts as one), else it is the `Esc` action (main.ts turns it
 * into a blur inside a field). `?` opens About. `s` orders the list by the next column, `S` turns the order round, `i` opens
 * or closes the details of the selected row.
 * A key that is held down (`repeat`) never presses an action: one Enter is one press, so a held Enter cannot run
 * on past the dialog it just opened. Arrows keep repeating (that is how a long list is crossed).
 * In a dialog the browser does the rest: Enter / Space press the focused button (guarded by `dialogKeySwallowed`), Tab is trapped.
 */
export function keyAction(e: KeyLike, ctx: { inField: boolean; dialog: boolean }): KeyAction | null {
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  if (e.key === 'Escape') return ctx.dialog ? { kind: 'closeDialog' } : { kind: 'action', key: 'Esc' };
  if (ctx.inField || ctx.dialog) return null;
  const digit = /^[1-3]$/.test(e.key) ? SECTION_ORDER[Number(e.key) - 1] : undefined;
  if (digit !== undefined) return { kind: 'section', section: digit };
  switch (e.key) {
    case 'ArrowDown': return { kind: 'move', delta: 1 };
    case 'ArrowUp': return { kind: 'move', delta: -1 };
    case 'PageDown': return { kind: 'move', delta: 10 };
    case 'PageUp': return { kind: 'move', delta: -10 };
    case 'Home': return { kind: 'move', delta: -Infinity };
    case 'End': return { kind: 'move', delta: Infinity };
    case ' ': return { kind: 'toggle' };
    case '?': return { kind: 'about' };
    case 's': return { kind: 'sort', reverse: false };
    case 'S': return { kind: 'sort', reverse: true };
    case 'i': return { kind: 'details' };
    case 'ArrowRight': return { kind: 'expand', open: true };
    case 'ArrowLeft': return { kind: 'expand', open: false };
    default: return ACTION_KEYS.has(e.key) && e.repeat !== true ? { kind: 'action', key: e.key as ActionKey } : null;
  }
}

/**
 * The chord that is the app's own: Ctrl+A, which selects the copyable text of the place the focus is in. It holds in dialogs too,
 * before `keyAction` (which leaves every chord alone). In a field it is the field's: it gives null there, so the browser selects
 * the field's text. The zoom chords are Imago's (`mountZoom`), not the app's.
 */
export type ChordAction = { kind: 'selectAll' };

export function chordAction(e: KeyLike, ctx: { inField: boolean }): ChordAction | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return null;
  return (e.key === 'a' || e.key === 'A') && !e.shiftKey && !ctx.inField ? { kind: 'selectAll' } : null;
}

/**
 * Where Ctrl+A selects, by where the focus is: `[within, select]`, the region the focus is in and its copyable
 * part. tokens.css makes the same `select` places selectable (tests/native.test.ts holds the two together);
 * anywhere else Ctrl+A selects nothing, as in a desktop list: the chrome is not text.
 */
export const SELECTABLE: readonly (readonly [within: string, select: string])[] = [
  ['#detail', '#detail'], ['.report', '.rp-body'], ['[role=dialog]', '.dlg-body'], ['#notices', '#notices'],
];

/** The Esc key as an action carries it: Rust writes `Esc`; `Escape` is accepted too (the key's own name). */
const isEsc = (key: string | null): boolean => key === 'Esc' || key === 'Escape';

/** The view's enabled action that carries `key`. */
export function actionForKey(view: SectionView, key: ActionKey): ActionView | null {
  return view.actions.find((a) => a.enabled && (key === 'Esc' ? isEsc(a.key) : a.key === key)) ?? null;
}

const KEY_WORDS: Record<string, string> = { Enter: '↵' };
/** The word after a key in the footer: the action's id, except where the id is not what the button says (`scan` is "Find issues"). */
const HINT_WORDS: Record<string, string> = { scan: 'find issues' };

/**
 * The footer's key hints for the section on screen: movement, Space and the sort key, then every action that has a
 * key (`a analyze · ↵ clean`, Registry `s scan · ↵ fix`). Esc is left out so the
 * hint does not change while a job runs. Derived from the actions, so the hint and the keys agree.
 */
export function footerHint(view: SectionView): string {
  const keyed = view.actions.filter((a) => a.key !== null && !isEsc(a.key)).map((a) => `${KEY_WORDS[a.key as string] ?? a.key} ${HINT_WORDS[a.id] ?? a.id}`);
  return ['↑↓ move', 'space toggle', 's sort', ...keyed].join(' · ');
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
