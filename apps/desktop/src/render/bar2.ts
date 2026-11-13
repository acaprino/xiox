// Bar 2: the section's categories (nav) with their count and size, then the section's options.
// A control only reports what the user did through `optionIntent`; Rust decides what it means.

import { button, fill, h } from '../dom.ts';
import { formatBytes } from '../format.ts';
import { optionIntent, selectIntent } from '../intents.ts';
import type { ChoiceItem, NavItem, OptionView, SectionId, SectionView } from '../types.ts';
import type { Handlers } from './shell.ts';

type OptionOf<K extends OptionView['kind']> = Extract<OptionView, { kind: K }>;

function navRow(s: SectionId, item: NavItem, on: Handlers): HTMLElement {
  return h('button', {
    type: 'button', class: item.active ? 'it on' : 'it', 'data-key': `nav:${item.id}`, 'aria-current': item.active ? 'true' : null,
    onClick: () => on.send(s, selectIntent(s, item.id)),
  },
  h('span', { class: 'it-name', 'data-fit': '' }, item.name),
  item.bytes === null ? null : h('b', { class: 'num', 'data-fit': '' }, formatBytes(item.bytes)),
  item.note ? h('small', { 'data-fit': '' }, item.note) : null);
}

function toggleControl(s: SectionId, o: OptionOf<'toggle'>, on: Handlers): HTMLElement {
  const box = h('button', {
    type: 'button', class: 'ck', role: 'checkbox', 'aria-checked': String(o.on), 'aria-label': o.label,
    disabled: !o.enabled, 'data-key': `opt:${o.id}`,
    onClick: () => on.send(s, optionIntent(s, o, !o.on)),
  });
  return h('label', { class: 'ctl ctl-toggle', title: o.hint }, box, h('span', { class: 'ctl-label' }, o.label));
}

function numberControl(s: SectionId, o: OptionOf<'number'>, on: Handlers): HTMLElement {
  const input = h('input', {
    type: 'number', class: 'field num-in', min: o.min, max: o.max, step: 1, value: o.value,
    disabled: !o.enabled, 'aria-label': o.label, 'data-key': `opt:${o.id}`,
  });
  // Sent when the box is left or Enter is pressed; a value the box itself refuses is put back.
  input.addEventListener('change', () => {
    if (!input.checkValidity() || Number.isNaN(input.valueAsNumber)) { input.value = String(o.value); return; }
    on.send(s, optionIntent(s, o, input.valueAsNumber));
  });
  return h('label', { class: 'ctl ctl-number' }, h('span', { class: 'ctl-label' }, o.label), input);
}

function choiceControl(s: SectionId, o: OptionOf<'choice'>, on: Handlers): HTMLElement {
  const radio = (c: ChoiceItem): HTMLElement => h('label', { class: 'radio' },
    h('input', {
      type: 'radio', name: `${s}-${o.id}`, value: c.id, checked: c.id === o.value, disabled: !o.enabled,
      'data-key': `opt:${o.id}:${c.id}`, onChange: () => on.send(s, optionIntent(s, o, c.id)),
    }),
    h('span', {}, c.label));
  return h('div', { class: 'ctl ctl-choice', role: 'radiogroup', 'aria-label': o.label },
    h('span', { class: 'ctl-title' }, o.label), h('div', { class: 'radios' }, ...o.choices.map(radio)));
}

function listControl(s: SectionId, o: OptionOf<'list'>, on: Handlers): HTMLElement {
  const items = o.items.length === 0
    ? [h('span', { class: 'pick-empty' }, o.empty)]
    : o.items.map((text, index) => h('button', {
      type: 'button', class: 'pick-item', role: 'option', 'aria-selected': String(index === o.selected), disabled: !o.enabled,
      title: text, 'data-key': `opt:${o.id}:${index}`, onClick: () => on.send(s, optionIntent(s, o, index)),
    }, h('span', { class: 'ellipsis' }, text)));
  return h('div', { class: 'ctl ctl-list' },
    h('span', { class: 'ctl-title' }, o.label),
    h('div', { class: 'pick', role: 'listbox', 'aria-label': o.label }, ...items),
    h('div', { class: 'pick-buttons' },
      button(h('span', { 'data-fit': '' }, o.addLabel), { class: 'btn sm', disabled: !o.enabled, 'data-key': `opt:${o.id}:add`, onClick: () => on.addRoot() }),
      button(h('span', { 'data-fit': '' }, o.removeLabel), {
        class: 'btn sm', disabled: !o.enabled || o.selected === null, 'data-key': `opt:${o.id}:remove`,
        onClick: () => on.send(s, optionIntent(s, o, 'remove')),
      })));
}

function buttonControl(s: SectionId, o: OptionOf<'button'>, on: Handlers): HTMLElement {
  return h('div', { class: 'ctl' },
    button(h('span', { 'data-fit': '' }, o.label), {
      class: 'btn sm', disabled: !o.enabled, title: o.hint, 'data-key': `opt:${o.id}`, onClick: () => on.send(s, optionIntent(s, o, null)),
    }));
}

function meterControl(o: OptionOf<'meter'>): HTMLElement {
  const share = o.totalBytes > 0 ? Math.min(100, Math.max(0, (o.usedBytes / o.totalBytes) * 100)) : 0;
  const fill = h('i');
  fill.style.width = `${share.toFixed(1)}%`;
  return h('div', { class: `ctl ctl-meter tone-${o.tone}` },
    h('span', { class: 'ctl-title' }, o.label),
    h('div', { class: 'meter', role: 'meter', 'aria-label': o.label, 'aria-valuemin': 0, 'aria-valuemax': o.totalBytes, 'aria-valuenow': o.usedBytes }, fill),
    h('span', { class: 'meter-text num', 'data-fit': '' }, `${formatBytes(o.usedBytes)} / ${formatBytes(o.totalBytes)}`));
}

function textControl(o: OptionOf<'text'>): HTMLElement {
  return h('p', { class: `ctl ctl-text tone-${o.tone}`, title: o.hint }, o.text);
}

function optionControl(s: SectionId, o: OptionView, on: Handlers): HTMLElement {
  switch (o.kind) {
    case 'toggle': return toggleControl(s, o, on);
    case 'number': return numberControl(s, o, on);
    case 'choice': return choiceControl(s, o, on);
    case 'list': return listControl(s, o, on);
    case 'button': return buttonControl(s, o, on);
    case 'meter': return meterControl(o);
    case 'text': return textControl(o);
  }
}

function heading(title: string, count: number): HTMLElement {
  return h('div', { class: 'hd' }, h('span', { class: 'hd-title', 'data-fit': '' }, title), h('span', { class: 'count' }, count));
}

export function renderBar2(root: HTMLElement, view: SectionView, on: Handlers): void {
  const s = view.section;
  const parts: HTMLElement[] = [];
  if (view.nav.length > 0) {
    parts.push(h('div', { class: 'cats' }, ...view.nav.map((item) => navRow(s, item, on))));
    if (view.options.length > 0) parts.push(h('div', { class: 'hd hd-sub' }, h('span', { class: 'hd-title', 'data-fit': '' }, 'Options')));
  }
  parts.push(h('div', { class: 'opts' }, ...view.options.map((o) => optionControl(s, o, on))));
  const scroll = h('div', { class: 'items' }, ...parts);
  fill(root, heading(view.nav.length > 0 ? 'Categories' : 'Options', view.nav.length > 0 ? view.nav.length : view.options.length), scroll);
}
