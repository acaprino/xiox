// The sidebar of the shell (`sidebarHtml`, written once by render/frame.ts): the section's categories (nav) with their count and size,
// each row with its rail item (an avatar with the category's initials), then the section's options, in Imago's rows and fields.
// Where the list shows every category one after another (Cleanup) the categories are a menu that scrolls the list to one of
// them, and marks the one the list is at; where each category is a list of its own (Developers) a click asks Rust for it.
// A control only reports what the user did through `optionIntent`; Rust decides what it means.

import { noteHtml } from '@acaprino/imago/ui';
import { fill, fromHtml, h } from '../dom.ts';
import { formatBytes } from '../format.ts';
import { optionIntent, selectIntent } from '../intents.ts';
import type { UiState } from '../state.ts';
import type { ChoiceItem, NavItem, OptionView, SectionId, SectionView } from '../types.ts';
import type { Handlers } from './shell.ts';

type OptionOf<K extends OptionView['kind']> = Extract<OptionView, { kind: K }>;

const TICK_WORDS: Record<string, string> = { on: 'all ticked', mixed: 'some ticked', off: 'none ticked', none: 'nothing to tick' };

/** The initials of a name, for the avatar a row shows in the rail: the first letters of its first two words. */
export function initials(name: string): string {
  const letters = name.replace(/[^\p{L}\p{N} ]/gu, '').split(' ').filter(Boolean).slice(0, 2).map((word) => Array.from(word)[0] ?? '');
  return letters.join('').toUpperCase();
}

const avatar = (name: string): HTMLElement => h('span', { class: 'im-avatar im-rail-item', 'aria-hidden': 'true' }, initials(name));

/** A row of the sidebar: its rail item, its name, a line under it, and what it adds up to on the right. */
function row(attrs: Record<string, string | boolean | null>, title: string, sub: string | null, meta: string | null, rail: HTMLElement, click: () => void): HTMLElement {
  return h('button', { type: 'button', class: 'im-row xi-row', role: 'option', onClick: click, ...attrs },
    rail,
    h('span', { class: 'im-row-text' }, h('span', { class: 'im-row-title', title }, title), sub ? h('span', { class: 'im-row-sub', title: sub }, sub) : null),
    meta === null ? null : h('span', { class: 'im-row-meta im-num' }, meta));
}

/** The menu of a list of categories: one entry per `group` row, the entry of the category the list is at is marked. */
function menuRow(item: NavItem, view: SectionView, spy: string | null, on: Handlers): HTMLElement {
  const group = view.rows.find((r) => r.kind === 'group' && r.id === item.id);
  const here = spy === item.id;
  // How much of the category is ticked is said in words, not by a picture: in the row's tooltip and for a screen reader.
  const sub = item.note || null;
  const ticked = TICK_WORDS[group?.check ?? 'none'];
  const entry = row({
    'data-key': `nav:${item.id}`, 'data-group': item.id, 'aria-selected': String(here), 'aria-current': here ? 'true' : null,
    disabled: group === undefined, title: group === undefined ? 'Nothing in this category matches the search' : `Go to ${item.name}: ${ticked}`,
  }, item.name, sub, item.bytes === null ? null : formatBytes(item.bytes), avatar(item.name), () => on.goToGroup(item.id));
  entry.querySelector('.im-row-text')?.append(h('span', { class: 'sr-only' }, `, ${ticked}`));
  return entry;
}

function navRow(s: SectionId, item: NavItem, on: Handlers): HTMLElement {
  return row({
    'data-key': `nav:${item.id}`, 'aria-selected': String(item.active), 'aria-current': item.active ? 'true' : null,
  }, item.name, item.note || null, item.bytes === null ? null : formatBytes(item.bytes), avatar(item.name), () => on.send(s, selectIntent(s, item.id)));
}

/** A kind of issue Registry looks for: a row whose rail item is its check box, the whole row is the target. */
function kindRow(s: SectionId, o: OptionOf<'toggle'>, on: Handlers): HTMLElement {
  const check = h('span', { class: 'im-check im-rail-item', role: 'checkbox', 'aria-checked': String(o.on), 'aria-label': o.label });
  return row({
    'data-key': `opt:${o.id}`, 'aria-selected': 'false', disabled: !o.enabled, title: o.hint,
  }, o.label, null, null, check, () => on.send(s, optionIntent(s, o, !o.on)));
}

function toggleControl(s: SectionId, o: OptionOf<'toggle'>, on: Handlers): HTMLElement {
  const box = h('input', { type: 'checkbox', class: 'im-check', checked: o.on, disabled: !o.enabled, 'data-key': `opt:${o.id}` });
  box.addEventListener('change', () => { box.checked = o.on; on.send(s, optionIntent(s, o, !o.on)); });
  return h('label', { class: 'im-option xi-opt', title: o.hint }, box, o.label);
}

function numberControl(s: SectionId, o: OptionOf<'number'>, on: Handlers): HTMLElement {
  const input = h('input', {
    type: 'number', min: o.min, max: o.max, step: 1, value: o.value,
    disabled: !o.enabled, id: `field-${o.id}`, 'data-key': `opt:${o.id}`,
  });
  // A value outside the range says so, in a sentence that starts with the word Error, once the box is left (aria-invalid says it at once).
  const error = h('span', { class: 'im-field-error', id: `hint-${o.id}`, hidden: true }, `Error: use a whole number from ${o.min} to ${o.max}.`);
  const invalid = (): boolean => !input.checkValidity() || Number.isNaN(input.valueAsNumber);
  const mark = (show: boolean): void => {
    const bad = invalid();
    input.setAttribute('aria-invalid', String(bad));
    if (bad) input.setAttribute('aria-describedby', error.id); else input.removeAttribute('aria-describedby');
    error.hidden = !(bad && show);
  };
  input.addEventListener('input', () => mark(false));
  input.addEventListener('blur', () => mark(true));
  // Sent when the box is left or Enter is pressed; a value the box itself refuses is put back.
  input.addEventListener('change', () => {
    if (invalid()) { input.value = String(o.value); mark(false); return; }
    on.send(s, optionIntent(s, o, input.valueAsNumber));
  });
  return h('div', { class: 'im-field' }, h('label', { class: 'im-field-label', for: `field-${o.id}` }, o.label), h('span', { class: 'im-input' }, input), error);
}

function choiceControl(s: SectionId, o: OptionOf<'choice'>, on: Handlers): HTMLElement {
  const radio = (c: ChoiceItem): HTMLElement => h('label', {},
    h('input', {
      type: 'radio', name: `${s}-${o.id}`, value: c.id, checked: c.id === o.value, disabled: !o.enabled,
      'data-key': `opt:${o.id}:${c.id}`, onChange: () => on.send(s, optionIntent(s, o, c.id)),
    }),
    h('span', {}, c.label));
  return h('div', { class: 'im-field' }, h('span', { class: 'im-field-label' }, o.label),
    h('div', { class: 'im-seg', role: 'radiogroup', 'aria-label': o.label }, ...o.choices.map(radio)));
}

function listControl(s: SectionId, o: OptionOf<'list'>, on: Handlers): HTMLElement {
  const items = o.items.length === 0
    ? [h('span', { class: 'xi-empty-note' }, o.empty)]
    : o.items.map((text, index) => h('button', {
      type: 'button', class: 'im-row im-row--1 xi-row', role: 'option', 'aria-selected': String(index === o.selected), disabled: !o.enabled,
      title: text, 'data-key': `opt:${o.id}:${index}`, onClick: () => on.send(s, optionIntent(s, o, index)),
    }, h('span', { class: 'im-row-text' }, h('span', { class: 'im-row-title' }, text))));
  return h('div', { class: 'im-field xi-folders' },
    h('span', { class: 'im-field-label' }, o.label),
    h('div', { class: 'im-list xi-pick', role: 'listbox', 'aria-label': o.label }, ...items),
    h('div', { class: 'xi-pick-buttons' },
      h('button', { type: 'button', class: 'im-btn im-btn--sm', disabled: !o.enabled, 'data-key': `opt:${o.id}:add`, onClick: () => on.addRoot() }, o.addLabel),
      h('button', {
        type: 'button', class: 'im-btn im-btn--sm', disabled: !o.enabled || o.selected === null, 'data-key': `opt:${o.id}:remove`,
        title: o.enabled && o.selected === null ? 'Pick a folder in the list first.' : null,
        onClick: () => on.send(s, optionIntent(s, o, 'remove')),
      }, o.removeLabel)));
}

function buttonControl(s: SectionId, o: OptionOf<'button'>, on: Handlers): HTMLElement {
  return h('div', { class: 'xi-option' },
    h('button', {
      type: 'button', class: 'im-btn im-btn--sm', disabled: !o.enabled, title: o.hint, 'data-key': `opt:${o.id}`, onClick: () => on.send(s, optionIntent(s, o, null)),
    }, o.label));
}

function textControl(o: OptionOf<'text'>): HTMLElement {
  const note = h('div', { class: 'xi-option', title: o.hint }, fromHtml(document, noteHtml({ message: o.text, tone: o.tone === 'error' ? 'err' : o.tone })));
  return note;
}

export function optionControl(s: SectionId, o: OptionView, on: Handlers): HTMLElement {
  switch (o.kind) {
    case 'toggle': return toggleControl(s, o, on);
    case 'number': return numberControl(s, o, on);
    case 'choice': return choiceControl(s, o, on);
    case 'list': return listControl(s, o, on);
    case 'button': return buttonControl(s, o, on);
    case 'text': return textControl(o);
  }
}

export function renderSidebar(root: HTMLElement, view: SectionView, ui: UiState, on: Handlers): void {
  const s = view.section;
  // Registry's options are the kinds of issue the scan looks for: they are its categories, rows with their check boxes.
  const kinds = view.nav.length === 0 && s === 'registry';
  const title = view.nav.length > 0 ? 'Categories' : kinds ? 'Look for' : 'Options';
  const rows: HTMLElement[] = [];
  let options = view.options;
  if (view.nav.length > 0) {
    const menu = view.rows.some((r) => r.kind === 'group');
    const spy = ui.spy[s] ?? view.rows.find((r) => r.kind === 'group')?.id ?? null;
    rows.push(...view.nav.map((item) => (menu ? menuRow(item, view, spy, on) : navRow(s, item, on))));
  } else if (kinds) {
    rows.push(...view.options.filter((o): o is OptionOf<'toggle'> => o.kind === 'toggle').map((o) => kindRow(s, o, on)));
    options = view.options.filter((o) => o.kind !== 'toggle');
  }
  const heading = root.querySelector('.im-panel-title');
  if (heading) heading.textContent = title;
  const count = root.querySelector('.im-count');
  if (count) count.textContent = String(view.nav.length > 0 || kinds ? rows.length : options.length);
  const list = root.querySelector<HTMLElement>('#sidebar-list');
  if (!list) throw new Error('#sidebar-list is missing from the frame');
  list.setAttribute('aria-label', title);
  root.setAttribute('aria-label', title);
  // The section's options are in the inspector (render/inspector.ts), under the content header's `Options`.
  fill(list, ...rows);
}
