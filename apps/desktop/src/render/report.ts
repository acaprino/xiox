// The report that replaces the list when a clean has finished (or was cancelled): what was freed,
// what was skipped or failed, and each item. It is drawn from the ReportView alone; the only things
// the page adds are the static headings, number formatting and which item's lines are open. Its tables are Imago's.

import { noteHtml } from '@acaprino/imago/ui';
import { fill, fromHtml, h } from '../dom.ts';
import { formatBytes, formatDuration } from '../format.ts';
import type { ReportItem, ReportLine, ReportView, SectionId } from '../types.ts';
import { noticeNode, type Handlers } from './shell.ts';

const count = (n: number): string => n.toLocaleString('en-US');

/** One cell of a report table: one line, cut with an ellipsis, the full text in the tooltip. */
function textCell(text: string, cls = ''): HTMLElement {
  return h('td', { class: cls || null, title: text }, h('span', { class: 'ellipsis' }, text));
}

const numberCell = (text: string): HTMLElement => h('td', { class: 'im-num rp-n' }, text);

/** `Skipped (n)` / `Failed (n)`: the item and why; the full path (when there is one) is a cell of its own, not a tooltip. */
function lineTable(title: string, lines: ReportLine[]): HTMLElement | null {
  if (lines.length === 0) return null;
  return h('section', { class: 'rp-block' },
    h('h3', { class: 'rp-sub' }, `${title} (${lines.length})`),
    h('table', { class: 'im-table rp-lines', 'aria-label': title }, h('tbody', {},
      ...lines.map((l) => h('tr', { class: 'rp-row' },
        textCell(l.item, 'rp-name'), textCell(l.why, 'rp-why'),
        h('td', { class: 'rp-path im-cell-sub' }, l.path === '' ? null : l.path))))));
}

function itemRow(item: ReportItem, index: number, expanded: ReadonlySet<string>, on: Handlers): HTMLElement[] {
  const key = `report:${index}`;
  const open = expanded.has(key);
  const caret = item.lines.length === 0
    ? h('span', { class: 'xi-caret-gap', 'aria-hidden': 'true' })
    : h('button', {
      type: 'button', class: 'xi-caret', 'aria-expanded': String(open), title: `${item.name}: show or hide the files`, 'aria-label': `${item.name}: show or hide the files`,
      'data-key': `caret:${key}`, onClick: () => on.toggleExpanded(key),
    });
  const row = h('tr', { class: 'rp-row rp-item' },
    h('td', { class: 'rp-nm' }, h('div', { class: 'xi-nm' }, caret, h('span', { class: 'ellipsis rp-name', title: item.name }, item.name))),
    numberCell(formatBytes(item.freedBytes)), numberCell(count(item.removed)), numberCell(count(item.left)),
    textCell(item.note ?? '', 'rp-why'));
  const lines = open && item.lines.length > 0
    ? [h('tr', { class: 'rp-open-row' }, h('td', { colspan: 5 }, h('ul', { class: 'xi-lines rp-open' }, ...item.lines.map((l) => h('li', {}, l)))))]
    : [];
  return [row, ...lines];
}

/** Draws the report into `root` (the list area), replacing whatever it held. */
export function renderReport(root: HTMLElement, section: SectionId, report: ReportView, expanded: ReadonlySet<string>, on: Handlers): void {
  const head = h('header', { class: 'im-panel-header rp-head' },
    h('h4', { class: 'im-panel-title', id: 'rp-title' }, report.title),
    h('span', { class: 'im-spacer' }),
    h('span', { class: 'rp-buttons' },
      report.canSave ? h('button', { type: 'button', class: 'im-btn im-btn--sm', 'data-key': 'report:save', onClick: () => on.saveReport(section) }, h('span', { 'data-fit': '' }, 'Save as text…')) : null,
      h('button', { type: 'button', class: 'im-btn im-btn--sm im-btn--primary', 'data-key': 'report:close', onClick: () => on.send(section, { kind: 'closeDialog' }) }, h('span', { 'data-fit': '' }, 'Close'))));
  const sum = h('div', { class: 'rp-sum' },
    h('p', { class: 'rp-freed' }, 'Space freed: ', h('b', { class: 'im-num' }, formatBytes(report.freedBytes))),
    report.recycledBytes > 0 ? h('p', {}, 'Sent to the Recycle Bin: ', h('b', { class: 'im-num' }, formatBytes(report.recycledBytes))) : null,
    h('p', {}, `Files deleted: ${count(report.files)} · Folders deleted: ${count(report.dirs)} · Duration: ${formatDuration(report.elapsedMs)}`),
    report.cancelled ? h('p', { class: 'rp-cancelled' }, 'Cancelled: the clean stopped before the end.') : null);
  const categories = report.byCategory.length === 0 ? null : h('section', { class: 'rp-block' },
    h('h3', { class: 'rp-sub' }, 'By category'),
    h('table', { class: 'im-table rp-cats', 'aria-label': 'By category' }, h('tbody', {},
      ...report.byCategory.map(([name, bytes]) => h('tr', { class: 'rp-row' }, textCell(name, 'rp-name'), numberCell(formatBytes(bytes)))))));
  const items = report.items.length === 0 ? null : h('section', { class: 'rp-block' },
    h('h3', { class: 'rp-sub' }, `Items (${report.items.length})`),
    h('table', { class: 'im-table rp-items', 'aria-label': 'Items' },
      h('thead', {}, h('tr', { class: 'rp-row rp-hrow' },
        h('th', {}, 'item'), h('th', { class: 'im-num' }, 'freed'), h('th', { class: 'im-num' }, 'removed'), h('th', { class: 'im-num' }, 'left'), h('th', {}, 'note'))),
      h('tbody', {}, ...report.items.flatMap((item, index) => itemRow(item, index, expanded, on)))));
  const saved = report.saved === null ? null
    : h('div', { class: 'rp-saved' }, fromHtml(document, noteHtml({ message: report.saved.text, tone: report.saved.ok ? 'ok' : 'warn' })));
  const body = h('div', { class: 'im-panel-body rp-body im-content-text' },
    sum, ...report.notes.map((note) => noticeNode(note, { expanded, on })), saved, categories, lineTable('Skipped', report.skipped), lineTable('Failed', report.failed), items);
  fill(root, h('section', { class: 'report im-panel', role: 'region', 'aria-labelledby': 'rp-title', tabindex: -1 }, head, body));
}
