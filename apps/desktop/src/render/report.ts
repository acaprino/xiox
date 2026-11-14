// The report that replaces the list when a clean has finished (or was cancelled): what was freed,
// what was skipped or failed, and each item. It is drawn from the ReportView alone; the only things
// the page adds are the static headings, number formatting and which item's lines are open.

import { button, fill, h } from '../dom.ts';
import { formatBytes, formatDuration } from '../format.ts';
import type { ReportItem, ReportLine, ReportView, SectionId } from '../types.ts';
import { noticeNode, type Handlers } from './shell.ts';

const count = (n: number): string => n.toLocaleString('en-US');

/** One cell of a report table: one line, cut with an ellipsis, the full text in the tooltip. */
function ellipsis(text: string, cls = ''): HTMLElement {
  return h('span', { class: `ellipsis ${cls}`.trim(), title: text, role: 'cell' }, text);
}

/** `Skipped (n)` / `Failed (n)`: the item and why; the full path (when there is one) is a line of its own, not a tooltip. */
function lineTable(title: string, lines: ReportLine[]): HTMLElement | null {
  if (lines.length === 0) return null;
  return h('section', { class: 'rp-block' },
    h('h3', { class: 'rp-sub' }, `${title} (${lines.length})`),
    h('div', { class: 'rp-tab rp-lines', role: 'table', 'aria-label': title },
      ...lines.map((l) => h('div', { class: 'rp-row', role: 'row' },
        ellipsis(l.item, 'rp-name'), ellipsis(l.why, 'rp-why'),
        l.path === '' ? null : h('span', { class: 'rp-path', role: 'cell' }, l.path)))));
}

function itemRow(item: ReportItem, index: number, expanded: ReadonlySet<string>, on: Handlers): HTMLElement[] {
  const key = `report:${index}`;
  const open = expanded.has(key);
  const caret = item.lines.length === 0
    ? h('span', { class: 'caret-gap' })
    : h('button', {
      type: 'button', class: 'caret', 'aria-expanded': String(open), 'aria-label': `${item.name}: show or hide the files`,
      'data-key': `caret:${key}`, onClick: () => on.toggleExpanded(key),
    });
  const row = h('div', { class: 'rp-row rp-item', role: 'row' },
    h('div', { class: 'nm', role: 'cell' }, caret, h('span', { class: 'ellipsis rp-name', title: item.name }, item.name)),
    h('span', { class: 'num rp-n', role: 'cell' }, formatBytes(item.freedBytes)),
    h('span', { class: 'num rp-n', role: 'cell' }, count(item.removed)),
    h('span', { class: 'num rp-n', role: 'cell' }, count(item.left)),
    ellipsis(item.note ?? '', 'rp-why'));
  const lines = open && item.lines.length > 0 ? [h('ul', { class: 'lines rp-open' }, ...item.lines.map((l) => h('li', {}, l)))] : [];
  return [row, ...lines];
}

/** Draws the report into `root` (the list area), replacing whatever it held. */
export function renderReport(root: HTMLElement, section: SectionId, report: ReportView, expanded: ReadonlySet<string>, on: Handlers): void {
  const head = h('div', { class: 'rp-head' },
    h('h2', { class: 'rp-title', id: 'rp-title' }, report.title),
    h('span', { class: 'rp-buttons' },
      report.canSave ? button(h('span', { 'data-fit': '' }, 'Save as text…'), { class: 'btn sm', 'data-key': 'report:save', onClick: () => on.saveReport(section) }) : null,
      button(h('span', { 'data-fit': '' }, 'Close'), { class: 'btn sm primary', 'data-key': 'report:close', onClick: () => on.send(section, { kind: 'closeDialog' }) })));
  const sum = h('div', { class: 'rp-sum' },
    h('p', { class: 'rp-freed' }, 'Space freed: ', h('b', { class: 'num' }, formatBytes(report.freedBytes))),
    report.recycledBytes > 0 ? h('p', {}, 'Sent to the Recycle Bin: ', h('b', { class: 'num' }, formatBytes(report.recycledBytes))) : null,
    h('p', {}, `Files deleted: ${count(report.files)} · Folders deleted: ${count(report.dirs)} · Duration: ${formatDuration(report.elapsedMs)}`),
    report.cancelled ? h('p', { class: 'rp-cancelled' }, 'Cancelled: the clean stopped before the end.') : null);
  const categories = report.byCategory.length === 0 ? null : h('section', { class: 'rp-block' },
    h('h3', { class: 'rp-sub' }, 'By category'),
    h('div', { class: 'rp-tab rp-cats', role: 'table', 'aria-label': 'By category' },
      ...report.byCategory.map(([name, bytes]) => h('div', { class: 'rp-row', role: 'row' }, ellipsis(name, 'rp-name'), h('span', { class: 'num rp-n', role: 'cell' }, formatBytes(bytes))))));
  const items = report.items.length === 0 ? null : h('section', { class: 'rp-block' },
    h('h3', { class: 'rp-sub' }, `Items (${report.items.length})`),
    h('div', { class: 'rp-tab rp-items', role: 'table', 'aria-label': 'Items' },
      h('div', { class: 'rp-row rp-hrow', role: 'row' },
        h('span', { role: 'columnheader' }, 'item'), h('span', { class: 'rp-n', role: 'columnheader' }, 'freed'), h('span', { class: 'rp-n', role: 'columnheader' }, 'removed'), h('span', { class: 'rp-n', role: 'columnheader' }, 'left'), h('span', { role: 'columnheader' }, 'note')),
      ...report.items.flatMap((item, index) => itemRow(item, index, expanded, on))));
  const saved = report.saved === null ? null
    : h('p', { class: `rp-saved ${report.saved.ok ? 'tone-ok' : 'tone-warn'}`, role: report.saved.ok ? 'status' : 'alert' }, report.saved.text);
  const body = h('div', { class: 'rp-body' },
    sum, ...report.notes.map((note) => noticeNode(note, { expanded, on })), saved, categories, lineTable('Skipped', report.skipped), lineTable('Failed', report.failed), items);
  fill(root, h('section', { class: 'report', role: 'region', 'aria-labelledby': 'rp-title', tabindex: -1 }, head, body));
}
