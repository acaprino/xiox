// The panel under the list: what the focused row's `detail` says (item, files, size, level, mode), as Imago's facts,
// and in a second block the reason (`why`) and any free lines. Nothing focused: nothing drawn.

import { fill, h } from '../dom.ts';
import type { RowView } from '../types.ts';

const NUMBER_KEYS = new Set(['files', 'size']);

function facts(list: [string, string][]): HTMLElement {
  return h('dl', { class: 'im-facts' }, ...list.flatMap(([key, value]) => [
    h('dt', {}, key),
    h('dd', { class: NUMBER_KEYS.has(key) ? 'im-num' : null }, value),
  ]));
}

export function renderDetail(root: HTMLElement, row: RowView | null): void {
  root.setAttribute('role', 'region');
  root.setAttribute('aria-label', 'Details');
  // A scrollable region the keyboard can reach (arrows scroll it), only while there is something in it.
  if (!row) { root.removeAttribute('tabindex'); fill(root); return; }
  root.tabIndex = 0;
  const main = row.detail.filter(([key]) => key !== 'why');
  const reason: [string, string][] = row.detail.filter(([key]) => key === 'why');
  if (reason.length === 0 && row.why && !row.detail.some(([, value]) => value === row.why)) reason.push(['why', row.why]);
  // A blocked row says why it cannot be chosen, even when its detail already has a description under `why`.
  if (row.muted && !row.enabled && row.why && !reason.some(([, value]) => value === row.why)) reason.unshift(['blocked', row.why]);
  const second = reason.length > 0 || row.lines.length > 0;
  const blocks: HTMLElement[] = [];
  if (main.length > 0) blocks.push(h('div', { class: second ? 'xi-detail-box' : 'xi-detail-box xi-wide' }, facts(main)));
  if (second) {
    blocks.push(h('div', { class: main.length > 0 ? 'xi-detail-box' : 'xi-detail-box xi-wide' },
      reason.length > 0 ? facts(reason) : null,
      row.lines.length > 0 ? h('ul', { class: 'xi-lines' }, ...row.lines.map((line) => h('li', {}, line))) : null));
  }
  fill(root, ...blocks);
}
