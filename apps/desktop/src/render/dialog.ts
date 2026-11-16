// The modal confirmation: a cut-corner frame over a dimmed page. The page behind it is `inert`, so
// nothing outside can be clicked or reached with Tab; Tab and Shift+Tab also cycle inside (the
// browser alone would leave the page). The dialog is rebuilt from the view on every render, so the
// focused control is found again by its `data-key`. Everything it says comes from the DialogView.

import { button, fill, h } from '../dom.ts';
import { buttonIntent } from '../intents.ts';
import { cycleFocus, dialogKeySwallowed } from '../keys.ts';
import type { ButtonView, DialogView, SectionView } from '../types.ts';
import type { Handlers } from './shell.ts';

const BACKGROUND = ['top', 'status', 'profile', 'job', 'body', 'foot'];
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

/** The primary button of a permanent delete is the danger variant of the primary look (frame.css): still the one primary action. */
function dialogButton(view: SectionView, b: ButtonView, danger: boolean, on: Handlers): HTMLButtonElement {
  return button(h('span', { 'data-fit': '' }, b.label), {
    class: b.primary ? (danger ? 'btn primary danger' : 'btn primary') : 'btn', 'data-button': b.id, 'data-key': `dlg:${b.id}`, disabled: !b.enabled,
    onClick: () => on.send(view.section, buttonIntent(view.section, b.id)),
  });
}

function ackBox(view: SectionView, dialog: DialogView, on: Handlers): HTMLElement {
  const checked = dialog.ack === true;
  return h('button', {
    type: 'button', class: 'ack', role: 'checkbox', 'aria-checked': String(checked), 'data-key': 'dlg:ack',
    onClick: () => on.send(view.section, { kind: 'setAck', on: !checked }),
  }, h('i', { class: 'ckbox', 'aria-hidden': 'true' }), h('span', {}, 'I have read the warnings'));
}

function content(view: SectionView, d: DialogView, on: Handlers): HTMLElement {
  const body: (HTMLElement | null)[] = [
    ...d.text.map((line) => h('p', { class: 'dlg-text' }, line)),
    d.error === null ? null : h('p', { class: 'dlg-error', role: 'alert' }, d.error),
    ...d.summary.map((line) => h('p', { class: 'dlg-sum' }, line)),
    d.rows.length === 0 ? null : h('div', { class: 'dlg-rows', role: 'list', tabindex: 0, 'aria-label': 'Items', 'data-key': 'dlg:rows' },
      ...d.rows.map(([label, size]) => h('div', { class: 'dr', role: 'listitem' },
        h('span', { class: 'ellipsis', title: label }, label), h('span', { class: 'num dr-size' }, size)))),
    d.warnings.length === 0 && d.warningsTitle === null ? null : h('div', { class: 'dlg-warn' },
      d.warningsTitle === null ? null : h('b', { class: 'dlg-sub' }, d.warningsTitle),
      h('ul', {}, ...d.warnings.map((w) => h('li', {}, w)))),
    d.cautions.length === 0 && d.cautionTitle === null ? null : h('div', { class: 'dlg-caut' },
      d.cautionTitle === null ? null : h('b', { class: 'dlg-sub' }, d.cautionTitle),
      h('ul', {}, ...d.cautions.map(([label, note]) => h('li', {}, h('span', { class: 'cl' }, label), h('small', {}, note))))),
    d.ack === null ? null : ackBox(view, d, on),
  ];
  return h('div', { class: 'dlg frame', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'dlg-title', tabindex: -1 },
    h('h2', { id: 'dlg-title', class: 'dlg-title' }, d.title),
    h('div', { class: 'dlg-body' }, ...body),
    h('div', { class: 'dlg-buttons' }, ...d.buttons.map((b) => dialogButton(view, b, d.permanent, on))));
}

/**
 * The control that gets the focus when a dialog opens: Cancel for a permanent delete (a second Enter then never approves it),
 * else the primary button if it is enabled, else the ack box, else Cancel.
 */
function firstFocus(root: HTMLElement, d: DialogView): HTMLElement | null {
  const primary = d.permanent ? undefined : d.buttons.find((b) => b.primary && b.enabled);
  const key = primary ? `dlg:${primary.id}` : d.permanent ? 'dlg:cancel' : d.ack !== null ? 'dlg:ack' : 'dlg:cancel';
  return root.querySelector<HTMLElement>(`[data-key="${key}"]`)
    ?? controls(root).find((n) => n.matches('.btn')) ?? root.querySelector<HTMLElement>('[role=dialog]');
}

function setInert(on: boolean): void {
  for (const id of BACKGROUND) document.getElementById(id)?.toggleAttribute('inert', on);
}

/** Where the focus goes when the dialog closes: where it was before, else the list. */
function restoreFocus(opener: string | null): void {
  const back = opener === null ? null : document.querySelector<HTMLElement>(`[data-key="${CSS.escape(opener)}"]`);
  const target = back && !(back as HTMLButtonElement).disabled ? back : document.querySelector<HTMLElement>('#rows .grid') ?? document.getElementById('rows');
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
  fill(root, h('div', { class: 'scrim' }, content(view, d, on)));
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
