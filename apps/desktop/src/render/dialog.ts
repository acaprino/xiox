// The modal confirmation: Imago's dialog over its scrim, which leaves the title bar free so the window can be moved and closed
// while a confirmation is open. The page under it is `inert`, so nothing outside can be clicked or reached with Tab; Tab and
// Shift+Tab also cycle inside (the browser alone would leave the page). The dialog is rebuilt from the view on every render, so
// the focused control is found again by its `data-key`. Everything it says comes from the DialogView.

import { noteHtml } from '@acaprino/imago/ui';
import { button, fill, fromHtml, h } from '../dom.ts';
import { buttonIntent } from '../intents.ts';
import { cycleFocus, dialogKeySwallowed } from '../commands.ts';
import type { ButtonView, DialogView, SectionView } from '../types.ts';
import type { Handlers } from './shell.ts';

/** What a confirmation makes inert: the sidebar and the work area. The title bar stays free, so the window can be moved and closed. */
const BACKGROUND = ['body'];
const FOCUSABLE = 'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** What was open on the last render, and who had the focus before it opened. */
let shown: { id: string; section: string; opener: string | null } | null = null;
let wired = false;
/** When the dialog on screen opened (`performance.now()`): its buttons ignore Enter and Space for a moment (keys.ts). */
let openedAt = 0;

function host(): HTMLElement {
  const node = document.getElementById('modal');
  if (!node) throw new Error('#modal is missing from index.html');
  return node;
}

function controls(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)];
}

/** Tab is trapped here; the other keys are the browser's (Enter and Space press the focused button, Escape is main.ts's). */
function wire(): void {
  if (wired) return;
  wired = true;
  // Enter / Space on a control become a click in the browser's own handler: stopped here while they are held
  // or too soon after the dialog opened. Space clicks on key-up, so the key-up is held back too.
  const guard = (event: KeyboardEvent): void => {
    if (dialogKeySwallowed(event, performance.now() - openedAt)) { event.preventDefault(); event.stopPropagation(); }
  };
  host().addEventListener('keydown', guard);
  host().addEventListener('keyup', (event) => { if (event.key === ' ') guard(event); });
  host().addEventListener('keydown', (event) => {
    if (event.key !== 'Tab' || event.ctrlKey || event.altKey || event.metaKey) return;
    const list = controls(host());
    const to = cycleFocus(list.length, list.indexOf(document.activeElement as HTMLElement), event.shiftKey);
    event.preventDefault();
    if (to === null) host().querySelector<HTMLElement>('[role=dialog]')?.focus();
    else list[to]?.focus();
  });
}

/** The confirming button of a permanent delete is Imago's confirming danger button: still the one primary action of the dialog. */
function dialogButton(view: SectionView, b: ButtonView, danger: boolean, on: Handlers): HTMLButtonElement {
  return button(h('span', { 'data-fit': '' }, b.label), {
    class: b.primary ? (danger ? 'im-btn im-btn--confirm-danger' : 'im-btn im-btn--primary') : 'im-btn',
    'data-button': b.id, 'data-key': `dlg:${b.id}`, disabled: !b.enabled,
    onClick: () => on.send(view.section, buttonIntent(view.section, b.id)),
  });
}

function ackBox(view: SectionView, dialog: DialogView, on: Handlers): HTMLElement {
  const checked = dialog.ack === true;
  const box = h('input', { type: 'checkbox', class: 'im-check', checked, 'data-key': 'dlg:ack' });
  box.addEventListener('change', () => on.send(view.section, { kind: 'setAck', on: !checked }));
  return h('label', { class: 'im-option ack' }, box, 'I have read the warnings');
}

/** A note of the dialog with a title and plain lines under it: the warnings and the cautions are notes of their tone. */
function listNote(title: string | null, tone: 'warn', items: HTMLElement[], cls: string): HTMLElement {
  const note = fromHtml(document, noteHtml({ title: title ?? undefined, tone, actionsHtml: '<span data-slot></span>' })).firstElementChild as HTMLElement;
  note.classList.add(cls);
  const slot = note.querySelector('[data-slot]');
  const row = slot?.parentElement;
  slot?.remove();
  row?.replaceWith(h('ul', { class: 'xi-lines' }, ...items));
  return note;
}

function content(view: SectionView, d: DialogView, on: Handlers): HTMLElement {
  const body: (HTMLElement | null)[] = [
    ...d.text.map((line) => h('p', { class: 'dlg-text' }, line)),
    d.error === null ? null : h('div', { class: 'dlg-error' }, fromHtml(document, noteHtml({ message: d.error, tone: 'err' }))),
    ...d.summary.map((line) => h('p', { class: 'dlg-sum' }, line)),
    d.rows.length === 0 ? null : h('div', { class: 'im-scroll dlg-rows', role: 'region', tabindex: 0, 'aria-label': 'Items', 'data-key': 'dlg:rows' },
      h('table', { class: 'im-table' }, h('tbody', {},
        ...d.rows.map(([label, size]) => h('tr', { class: 'dr' },
          h('td', { title: label }, h('span', { class: 'ellipsis' }, label)), h('td', { class: 'im-num dr-size' }, size)))))),
    d.warnings.length === 0 && d.warningsTitle === null ? null
      : listNote(d.warningsTitle, 'warn', d.warnings.map((w) => h('li', {}, w)), 'dlg-warn'),
    d.cautions.length === 0 && d.cautionTitle === null ? null
      : listNote(d.cautionTitle, 'warn', d.cautions.map(([label, note]) => h('li', {}, h('b', { class: 'cl' }, label), ': ', h('small', {}, note))), 'dlg-caut'),
    d.ack === null ? null : ackBox(view, d, on),
  ];
  return h('div', { class: 'im-dialog im-float dlg', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'dlg-title', tabindex: -1 },
    h('h2', { id: 'dlg-title', class: 'im-dialog-title dlg-title' }, d.title),
    h('div', { class: 'im-dialog-body im-content-text im-scroll dlg-body' }, ...body),
    h('div', { class: 'im-dialog-actions dlg-buttons' }, ...d.buttons.map((b) => dialogButton(view, b, d.permanent, on))));
}

/**
 * The control that gets the focus when a dialog opens: Cancel for a permanent delete (a second Enter then never approves it),
 * else the primary button if it is enabled, else the ack box, else Cancel.
 */
function firstFocus(root: HTMLElement, d: DialogView): HTMLElement | null {
  const primary = d.permanent ? undefined : d.buttons.find((b) => b.primary && b.enabled);
  const key = primary ? `dlg:${primary.id}` : d.permanent ? 'dlg:cancel' : d.ack !== null ? 'dlg:ack' : 'dlg:cancel';
  return root.querySelector<HTMLElement>(`[data-key="${key}"]`)
    ?? controls(root).find((n) => n.matches('.im-btn')) ?? root.querySelector<HTMLElement>('[role=dialog]');
}

function setInert(on: boolean): void {
  for (const id of BACKGROUND) document.getElementById(id)?.toggleAttribute('inert', on);
}

/** Where the focus goes when the dialog closes: where it was before, else the list. */
function restoreFocus(opener: string | null): void {
  const back = opener === null ? null : document.querySelector<HTMLElement>(`[data-key="${CSS.escape(opener)}"]`);
  const target = back && !(back as HTMLButtonElement).disabled ? back : document.querySelector<HTMLElement>('#rows .xi-table') ?? document.getElementById('rows');
  target?.focus();
}

/** Draws the open dialog of `view`, or removes it. Call it last in a render. */
export function renderDialog(view: SectionView, on: Handlers, focusedBefore: string | null): void {
  wire();
  const root = host();
  const d = view.dialog;
  if (d === null) {
    if (shown === null) return;
    const { opener } = shown;
    shown = null;
    setInert(false);
    root.hidden = true;
    fill(root);
    restoreFocus(opener);
    return;
  }
  const same = shown !== null && shown.id === d.id && shown.section === view.section;
  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const keptKey = same && active !== null && root.contains(active) ? active.dataset.key ?? null : null;
  const scroll = root.querySelector('.dlg-body')?.scrollTop ?? 0;
  const opener = same && shown !== null ? shown.opener : (focusedBefore ?? active?.dataset.key ?? null);
  fill(root, h('div', { class: 'im-scrim' }, content(view, d, on)));
  root.hidden = false;
  setInert(true);
  if (!same) openedAt = performance.now();
  shown = { id: d.id, section: view.section, opener };
  const body = root.querySelector('.dlg-body');
  if (body) body.scrollTop = scroll;
  if (same) {
    // A redraw of the same dialog (the ack box was ticked): the focus stays where it was.
    (keptKey === null ? null : root.querySelector<HTMLElement>(`[data-key="${CSS.escape(keptKey)}"]`))?.focus();
  } else {
    firstFocus(root, d)?.focus();
  }
}
