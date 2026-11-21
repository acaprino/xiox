// The confirmation: an app-owned `dialog` element opened with Imago's `openContentDialog`, so it is in the package's layer stack (the
// regions under the title bar are inert, Tab stays inside, Escape and a press on the scrim dismiss it, and the title bar stays free
// to move and close the window). It is rebuilt in place from `view.dialog` on every render; the focus starts on Cancel, for every
// confirmation (the permanent ones included: a second Enter never approves). A close the view asked for sends nothing; one the user
// asked for (Escape, the scrim) sends `closeDialog`. The 300 ms and held-key guard of commands.ts stays on the dialog's own keydown.

import { noteHtml, openContentDialog } from '@acaprino/imago/ui';
import { dialogKeySwallowed } from '../commands.ts';
import { fill, fromHtml, h } from '../dom.ts';
import { buttonIntent } from '../intents.ts';
import type { ButtonView, DialogView, SectionView } from '../types.ts';
import type { Handlers } from './shell.ts';

let dialog: HTMLDialogElement | null = null;
let layer: { close(): void } | null = null;
/** What is open: the dialog's id and its section, and the handlers of the last render (the close the user asks for sends an intent). */
let shown: { id: string; section: SectionView['section']; on: Handlers } | null = null;
/** Set while the view closes the dialog: that close sends nothing. */
let closingByView = false;
/** When the dialog on screen opened (`performance.now()`): its buttons ignore Enter and Space for a moment (commands.ts). */
let openedAt = 0;

/** The dialog element, made once: its keys are guarded for its whole life. */
function element(): HTMLDialogElement {
  if (dialog) return dialog;
  const made = document.createElement('dialog');
  made.setAttribute('role', 'dialog');
  made.classList.add('dlg');
  // Enter / Space on a control become a click in the browser's own handler: stopped here while they are held or too soon after
  // the dialog opened. Space clicks on key-up, so the key-up is held back too.
  const guard = (event: KeyboardEvent): void => {
    if (dialogKeySwallowed(event, performance.now() - openedAt)) { event.preventDefault(); event.stopPropagation(); }
  };
  made.addEventListener('keydown', guard);
  made.addEventListener('keyup', (event) => { if (event.key === ' ') guard(event); });
  dialog = made;
  return made;
}

/** The confirming button of a permanent delete is Imago's confirming danger button: still the one primary action of the dialog. */
function dialogButton(view: SectionView, b: ButtonView, danger: boolean, cancel: boolean, on: Handlers): HTMLButtonElement {
  return h('button', {
    type: 'button', class: b.primary ? (danger ? 'im-btn im-btn--confirm-danger' : 'im-btn im-btn--primary') : 'im-btn',
    'data-button': b.id, 'data-key': `dlg:${b.id}`, disabled: !b.enabled, autofocus: cancel,
    onClick: () => on.send(view.section, buttonIntent(view.section, b.id)),
  }, h('span', { 'data-fit': '' }, b.label));
}

function ackBox(view: SectionView, d: DialogView, on: Handlers): HTMLElement {
  const checked = d.ack === true;
  const box = h('input', { type: 'checkbox', class: 'im-check', checked, 'data-key': 'dlg:ack' });
  box.addEventListener('change', () => on.send(view.section, { kind: 'setAck', on: !checked }));
  return h('label', { class: 'im-option ack' }, box, 'I have read the warnings');
}

/** A note of the dialog with a title and plain lines under it: the warnings and the cautions are notes of their tone. */
function listNote(title: string | null, items: HTMLElement[], cls: string): HTMLElement {
  const note = fromHtml(document, noteHtml({ title: title ?? undefined, tone: 'warn', actionsHtml: '<span data-slot></span>' })).firstElementChild as HTMLElement;
  note.classList.add(cls);
  const slot = note.querySelector('[data-slot]');
  const row = slot?.parentElement;
  slot?.remove();
  row?.replaceWith(h('ul', { class: 'xi-lines' }, ...items));
  return note;
}

/** The buttons in the order of a confirmation: Cancel first, the others as Rust wrote them, the confirming one last. */
function ordered(buttons: ButtonView[]): ButtonView[] {
  return [...buttons.filter((b) => b.id === 'cancel'), ...buttons.filter((b) => b.id !== 'cancel' && !b.primary), ...buttons.filter((b) => b.id !== 'cancel' && b.primary)];
}

function content(view: SectionView, d: DialogView, on: Handlers): HTMLElement[] {
  const body: (HTMLElement | null)[] = [
    ...d.text.map((line) => h('p', { class: 'dlg-text' }, line)),
    d.error === null ? null : h('div', { class: 'dlg-error' }, fromHtml(document, noteHtml({ message: d.error, tone: 'err' }))),
    ...d.summary.map((line) => h('p', { class: 'dlg-sum' }, line)),
    d.rows.length === 0 ? null : h('div', { class: 'im-scroll dlg-rows', role: 'region', tabindex: 0, 'aria-label': 'Items', 'data-key': 'dlg:rows' },
      h('table', { class: 'im-table' }, h('tbody', {},
        ...d.rows.map(([label, size]) => h('tr', { class: 'dr' },
          h('td', { title: label }, h('span', { class: 'ellipsis' }, label)), h('td', { class: 'im-num dr-size' }, size)))))),
    d.warnings.length === 0 && d.warningsTitle === null ? null : listNote(d.warningsTitle, d.warnings.map((w) => h('li', {}, w)), 'dlg-warn'),
    d.cautions.length === 0 && d.cautionTitle === null ? null
      : listNote(d.cautionTitle, d.cautions.map(([label, note]) => h('li', {}, h('b', { class: 'cl' }, label), ': ', h('small', {}, note))), 'dlg-caut'),
    d.ack === null ? null : ackBox(view, d, on),
  ];
  return [
    h('h2', { id: 'dlg-title', class: 'im-dialog-title dlg-title' }, d.title),
    h('div', { class: 'im-dialog-body im-content-text im-scroll dlg-body' }, ...body),
    h('div', { class: 'im-dialog-actions dlg-buttons' }, ...ordered(d.buttons).map((b) => dialogButton(view, b, d.permanent, b.id === 'cancel', on))),
  ];
}

/** Where the focus goes when the dialog has closed and the package found nothing to give it back to: the list. */
function focusList(): void {
  const active = document.activeElement;
  if (active === null || active === document.body) (document.querySelector<HTMLElement>('#rows .xi-table') ?? document.getElementById('rows'))?.focus();
}

/** Draws the open dialog of `view`, or closes it. Call it last in a render. */
export function renderDialog(view: SectionView, on: Handlers): void {
  const d = view.dialog;
  if (d === null) {
    if (layer === null) return;
    closingByView = true;
    try { layer.close(); } finally { closingByView = false; }
    return;
  }
  const shell = document.getElementById('app');
  if (!shell) return;
  const root = element();
  const same = layer !== null && shown !== null && shown.id === d.id && shown.section === view.section;
  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const keptKey = same && active !== null && root.contains(active) ? active.dataset.key ?? null : null;
  const scroll = root.querySelector('.dlg-body')?.scrollTop ?? 0;
  root.setAttribute('aria-labelledby', 'dlg-title');
  fill(root, ...content(view, d, on));
  const body = root.querySelector('.dlg-body');
  if (body) body.scrollTop = scroll;
  shown = { id: d.id, section: view.section, on };
  if (layer === null) {
    openedAt = performance.now();
    layer = openContentDialog(shell, root, {
      onClose: () => {
        layer = null;
        const closed = shown;
        shown = null;
        // The user closed it (Escape, a press on the scrim): the view is told, and Rust answers with the view without the dialog.
        if (!closingByView && closed !== null) closed.on.send(closed.section, { kind: 'closeDialog' });
        focusList();
      },
    });
    return;
  }
  if (same) {
    // A redraw of the same dialog (the ack box was ticked): the focus stays where it was.
    (keptKey === null ? null : root.querySelector<HTMLElement>(`[data-key="${CSS.escape(keptKey)}"]`))?.focus();
  } else {
    openedAt = performance.now();
    root.querySelector<HTMLElement>('[autofocus]')?.focus();
  }
}
