// The commands of the window that are Xiox's own, declared with Sigrid's `useCommand`: the keys of the list (`s` the order, `i` Details,
// `o` Options), the action of the open view (`a` Analyze, Find issues), and Escape. The palette (Ctrl+K) lists them with the shell's,
// and the foot names the ones that work now. The views, the look and the search are the shell's. A key that types belongs to a field,
// and a confirmation that is open owns the keyboard: Sigrid keeps both (`useCommand`), and `when` keeps the rest.
import { useCommand } from '@acaprino/sigrid/assemblies';
import { Icon } from '@acaprino/sigrid/primitives';
import { actionCommandId, actionForKey, actionKeys, COMMAND } from '../commands.ts';
import type { Controller } from '../controller.ts';
import { actionIntent } from '../intents.ts';
import type { ActionView, SectionView } from '../types.ts';

/** The one action of the view with this id, as a command with the key Rust gave it: a disabled action answers no key. */
function useAction(c: Controller, view: SectionView | undefined, id: string): void {
  const action: ActionView | undefined = view?.actions.find((a) => a.id === id);
  useCommand({
    id: action === undefined ? `xiox.action.${id}` : actionCommandId(action),
    label: action?.label ?? id,
    group: 'Section',
    keys: action === undefined ? undefined : actionKeys(action),
    when: action !== undefined && action.enabled && view?.dialog === null && !c.ui.settings,
    run: () => { if (view !== undefined && action !== undefined) void c.send(view.section, actionIntent(view.section, action.id)); },
  });
}

/** Escape: a field lets go, a report closes, a running job is cancelled. A confirmation's own Escape is its Cancel (the dialog's). */
function escape(c: Controller): void {
  const view = c.view;
  if (view === undefined || c.ui.settings) return;
  const active = document.activeElement;
  if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) { active.blur(); return; }
  if (view.report !== null) { void c.send(view.section, { kind: 'closeDialog' }); return; }
  const action = actionForKey(view, 'Esc');
  if (action !== null) void c.send(view.section, actionIntent(view.section, action.id));
}

/** Declares them all. Called once by the window. */
export function useXioxCommands(c: Controller): void {
  const view = c.view;
  const listed = view !== undefined && view.dialog === null && view.report === null && !c.ui.settings;
  
  useAction(c, view, 'analyze');
  useAction(c, view, 'scan');
  useAction(c, view, 'clean');
  useAction(c, view, 'fix');
  useAction(c, view, 'cancel');
  useCommand({ id: COMMAND.sort, label: 'Order by the next column', group: 'List', keys: 's', icon: <Icon name="moveDown" />, when: listed, run: () => c.cycleSort(false) });
  useCommand({ id: COMMAND.sortReverse, label: 'Turn the order round', group: 'List', keys: 'Shift+S', icon: <Icon name="moveUp" />, when: listed, run: () => c.cycleSort(true) });
  useCommand({ id: COMMAND.details, label: 'Show the details of the row', group: 'List', keys: 'i', icon: <Icon name="info" />, when: listed, checked: c.ui.inspector === 'details', run: () => c.toggleInspector('details') });
  useCommand({ id: COMMAND.options, label: 'Show the options of the section', group: 'List', keys: 'o', icon: <Icon name="settings" />, when: listed, checked: c.ui.inspector === 'options', run: () => c.toggleInspector('options') });
  useCommand({ id: COMMAND.escape, label: 'Cancel or close', group: 'List', keys: 'Escape', run: () => escape(c) });
}
