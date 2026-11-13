// The panel under the list: what the focused row's `detail` says (item, files, size, level, mode),
// and in a second box the reason (`why`) and any free lines. Nothing focused: nothing drawn.

import { fill, h } from '../dom.ts';
import type { RowView } from '../types.ts';

const MONO_KEYS = new Set(['files', 'size']);

function pairs(list: [string, string][]): HTMLElement {
  return h('div', { class: 'kv' }, ...list.flatMap(([key, value]) => [
    h('span', {}, key),
    h('b', { class: MONO_KEYS.has(key) ? 'num' : null }, value),
  ]));
}

export function renderDetail(root: HTMLElement, row: RowView | null): void {
  root.setAttribute('role', 'region');
  root.setAttribute('aria-label', 'Details');
  if (!row) { fill(root); return; }
  const main = row.detail.filter(([key]) => key !== 'why');
  const reason: [string, string][] = row.detail.filter(([key]) => key === 'why');
  if (reason.length === 0 && row.why) reason.push(['why', row.why]);
  const second = reason.length > 0 || row.lines.length > 0;
  const boxes: HTMLElement[] = [];
  if (main.length > 0) boxes.push(h('div', { class: second ? 'box' : 'box wide' }, pairs(main)));
  if (second) {
    boxes.push(h('div', { class: main.length > 0 ? 'box' : 'box wide' },
      reason.length > 0 ? pairs(reason) : null,
      row.lines.length > 0 ? h('ul', { class: 'lines' }, ...row.lines.map((line) => h('li', {}, line))) : null));
  }
  fill(root, ...boxes);
}
