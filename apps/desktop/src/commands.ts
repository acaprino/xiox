// The commands of the app, in Imago's command registry (docs/using-imago.md, section 6), and the keys of the list. The registry
// answers every key of the window but the list's own: the views `1` to `4`, Settings `?`, sort `s` / `Shift+S`, Details `i`, Options
// `o`, Escape, Ctrl+A, and the actions of the open view (`a` Analyze, Find issues...), which the palette (Ctrl+K) lists with the
// rest. Enter is never bound: it presses the focused button, and on the list it presses the primary action (`listKey`). A key that
// types belongs to a text field, and a confirmation that is open owns the keyboard: no command runs behind it.
// No DOM here, so node can test it; main.ts gives the commands what they act on (`CommandApp`).

import type { Command, Registry } from '@acaprino/imago/ui';
import { actionIntent } from './intents.ts';
import { SECTION_ORDER } from './state.ts';
import type { ActionView, AnyIntent, SectionId, SectionView } from './types.ts';

export interface KeyLike { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean; repeat?: boolean }

/** The keys an action can carry (`ActionView.key`): `a` is Analyze and Find issues (Registry); Enter is Clean / Fix; Esc cancels a job. */
export type ActionKey = 'a' | 'Enter' | 'Esc';

/** What the commands act on. */
export interface CommandApp {
  /** The open section's view, once there is one. */
  view(): SectionView | undefined;
  /** Nothing is open that owns the keyboard: a view is drawn and no confirmation is up. */
  idle(): boolean;
  /** The list is what the keys are for: idle, no report in its place, not Settings. */
  listed(): boolean;
  /** A text field has the focus (a key that types is the field's). */
  inTextField(): boolean;
  openSection(id: SectionId): void;
  openSettings(): void;
  send(section: SectionId, intent: AnyIntent | null): void;
  sort(reverse: boolean): void;
  toggleInspector(kind: 'details' | 'options'): void;
  selectAll(): void;
  /** Escape: blur a field, else close the report, else the view's Esc action (a running job is cancelled), a confirmation's Cancel. */
  escape(): void;
}

const SECTION_TITLES: Record<SectionId, string> = { cleanup: 'Cleanup', registry: 'Registry', developers: 'Developers' };

/** The ids of the commands that are always there. */
export const COMMAND = {
  settings: 'xiox.settings',
  settingsTab: 'xiox.settings.tab',
  sort: 'xiox.sort',
  sortReverse: 'xiox.sort.reverse',
  details: 'xiox.details',
  options: 'xiox.options',
  escape: 'xiox.escape',
  selectAll: 'xiox.selectAll',
} as const;

/** The id of the command of a view's action. */
export const actionCommandId = (action: ActionView): string => `xiox.action.${action.id}`;

/** The commands that do not depend on the view; returns the function that unregisters them. */
export function registerCommands(registry: Registry, app: CommandApp): () => void {
  const idle = (): boolean => app.idle();
  const listed = (): boolean => app.listed();
  const commands: Command[] = [
    ...SECTION_ORDER.map((id, index): Command => ({ id: `xiox.section.${id}`, title: `Go to ${SECTION_TITLES[id]}`, keys: String(index + 1), group: 'View', when: idle, run: () => app.openSection(id) })),
    { id: COMMAND.settings, title: 'Open Settings', keys: 'Shift+?', group: 'View', when: idle, run: () => app.openSettings() },
    { id: COMMAND.settingsTab, title: 'Open Settings (4)', keys: '4', group: 'View', when: idle, run: () => app.openSettings() },
    { id: COMMAND.sort, title: 'Order by the next column', keys: 's', group: 'List', when: listed, run: () => app.sort(false) },
    { id: COMMAND.sortReverse, title: 'Turn the order round', keys: 'Shift+S', group: 'List', when: listed, run: () => app.sort(true) },
    { id: COMMAND.details, title: 'Show the details of the row', keys: 'i', group: 'List', when: listed, run: () => app.toggleInspector('details') },
    { id: COMMAND.options, title: 'Show the options of the section', keys: 'o', group: 'List', when: listed, run: () => app.toggleInspector('options') },
    // Escape runs in a field and behind a confirmation (it is its Cancel).
    { id: COMMAND.escape, title: 'Cancel or close', keys: 'Escape', run: () => app.escape() },
    // Ctrl+A is the field's own in a text field: the command is not available there, so the key is not taken.
    { id: COMMAND.selectAll, title: 'Select the text of the place', keys: 'Ctrl+A', when: () => !app.inTextField(), run: () => app.selectAll() },
  ];
  const undo = commands.map((command) => registry.register(command));
  return () => { for (const off of undo.reverse()) off(); };
}

/** The key an action puts on a command: Enter (the focused button's own) and Esc (the Escape command) are not bound by the action. */
export function actionKeys(action: ActionView): string | undefined {
  return action.key === null || action.key === 'Enter' || action.key === 'Esc' || action.key === 'Escape' ? undefined : action.key;
}

/**
 * The commands of the open view's actions, `xiox.action.<id>` titled with Rust's label and bound to its key (`a`). They change with the
 * view, so `sync` registers them again when the actions, their keys or their state changed, and only then.
 */
export function actionCommands(registry: Registry, app: CommandApp): { sync(view: SectionView | undefined): void; dispose(): void } {
  let signature = '';
  let undo: Array<() => void> = [];
  const clear = (): void => { for (const off of undo.reverse()) off(); undo = []; };
  return {
    sync(view) {
      const next = view === undefined ? '' : `${view.section}|${view.actions.map((a) => `${a.id}:${a.key}:${a.enabled}:${a.label}`).join(',')}`;
      if (next === signature) return;
      signature = next;
      clear();
      if (view === undefined) return;
      const used = new Set<string>();
      for (const action of view.actions) {
        const keys = actionKeys(action);
        const bound = keys !== undefined && !used.has(keys) ? keys : undefined;
        if (bound !== undefined) used.add(bound);
        undo.push(registry.register({
          id: actionCommandId(action), title: action.label, keys: bound, group: 'Section',
          when: () => action.enabled && app.idle(),
          run: () => app.send(view.section, actionIntent(view.section, action.id)),
        }));
      }
    },
    dispose() { signature = ''; clear(); },
  };
}

const isEsc = (key: string | null): boolean => key === 'Esc' || key === 'Escape';

/** The view's enabled action that carries `key`. */
export function actionForKey(view: SectionView, key: ActionKey): ActionView | null {
  return view.actions.find((a) => a.enabled && (key === 'Esc' ? isEsc(a.key) : a.key === key)) ?? null;
}

/** What the footer lists, in order: the list's own keys, the sort, then each action with its key (Enter as its own key, not a command). */
export function footerEntries(view: SectionView, has: (id: string) => boolean): Array<string | { keys: string; title: string }> {
  const entries: Array<string | { keys: string; title: string }> = [{ keys: 'ArrowUp ArrowDown', title: 'Move' }, { keys: 'Space', title: 'Toggle' }];
  if (has(COMMAND.sort)) entries.push(COMMAND.sort);
  for (const action of view.actions) {
    if (action.key === 'Enter') entries.push({ keys: 'Enter', title: action.label });
    else if (actionKeys(action) !== undefined) entries.push(actionCommandId(action));
  }
  return entries;
}

export type ListKey =
  | { kind: 'move'; delta: number } | { kind: 'toggle' } | { kind: 'expand'; open: boolean } | { kind: 'primary' };

/**
 * The list's own keys, on its `keydown`: the arrows, PageUp and PageDown, Home and End move the selection, Left and Right fold or open,
 * Space ticks the row, Enter presses the primary action. A chord is never the list's. A key held down (`repeat`) presses nothing
 * (one Enter is one press, so a held Enter cannot run on past the dialog it opened); the arrows keep repeating, that is how a long
 * list is crossed.
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

/**
 * Where Ctrl+A selects, by where the focus is: `[within, select]`, the region the focus is in and its copyable
 * part; anywhere else Ctrl+A selects nothing, as in a desktop list: the chrome is not text. Imago's `im-content-text` makes the same
 * `select` places selectable (tests/native.test.ts holds the two together).
 */
export const SELECTABLE: readonly (readonly [within: string, select: string])[] = [
  ['.im-inspector', '.im-content-text'], ['.report', '.rp-body'], ['[role=dialog]', '.dlg-body'], ['#notices', '#notices'],
];

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
