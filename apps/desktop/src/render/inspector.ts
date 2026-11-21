// The inspector of the shell (`inspector(shell)`, not modal): what the content header's `Details` (`i`) and `Options` (`o`) open.
// Details are the facts of the row the selection is on (item, files, size, level, mode, why) and any free lines; Options are the
// section's options as a form that saves itself: each control sends today's intent when it changes. Registry's kinds of issue are
// its categories and stay in the sidebar. The content is built again at every render (the view is the page's, a new one may come
// at any time) and what the user was doing in it is kept: the focus, and the text typed in a number field.

import { iconButtonHtml, inspector } from '@acaprino/imago/ui';
import { fromHtml, h } from '../dom.ts';
import type { UiState } from '../state.ts';
import type { OptionView, RowView, SectionView } from '../types.ts';
import { optionControl } from './sidebar.ts';
import type { Handlers } from './shell.ts';

export type InspectorKind = 'details' | 'options';

const NUMBER_KEYS = new Set(['files', 'size']);

/** The options the inspector shows: all of the section's, but Registry's kinds, which are its categories in the sidebar. */
export function panelOptions(view: SectionView): OptionView[] {
  const kinds = view.nav.length === 0 && view.section === 'registry';
  return kinds ? view.options.filter((o) => o.kind !== 'toggle') : view.options;
}

function facts(list: [string, string][]): HTMLElement {
  return h('dl', { class: 'im-facts' }, ...list.flatMap(([key, value]) => [
    h('dt', {}, key),
    h('dd', { class: NUMBER_KEYS.has(key) ? 'im-num' : null }, value),
  ]));
}

/** What a row's detail says, in two blocks: its facts, then the reason (`why`) and any free lines. */
function detailBlocks(row: RowView): HTMLElement[] {
  const main = row.detail.filter(([key]) => key !== 'why');
  const reason: [string, string][] = row.detail.filter(([key]) => key === 'why');
  if (reason.length === 0 && row.why && !row.detail.some(([, value]) => value === row.why)) reason.push(['why', row.why]);
  // A blocked row says why it cannot be chosen, even when its detail already has a description under `why`.
  if (row.muted && !row.enabled && row.why && !reason.some(([, value]) => value === row.why)) reason.unshift(['blocked', row.why]);
  const blocks: HTMLElement[] = [];
  if (main.length > 0) blocks.push(h('div', { class: 'xi-detail-box' }, facts(main)));
  if (reason.length > 0 || row.lines.length > 0) {
    blocks.push(h('div', { class: 'xi-detail-box' },
      reason.length > 0 ? facts(reason) : null,
      row.lines.length > 0 ? h('ul', { class: 'xi-lines' }, ...row.lines.map((line) => h('li', {}, line))) : null));
  }
  return blocks;
}

/** The panel's header: its title, a line under it, and the button that closes it (the inspector writes none). */
function header(title: string, sub: string, close: () => void): HTMLElement {
  const button = fromHtml(document, iconButtonHtml('Close', 'close-inspector', 'close')).firstElementChild as HTMLElement;
  button.setAttribute('data-key', 'inspector:close');
  button.addEventListener('click', close);
  return h('header', { class: 'im-panel-header' }, h('h4', { class: 'im-panel-title' }, title), h('span', { class: 'im-panel-sub' }, h('span', { class: 'ellipsis', title: sub }, sub)), h('span', { class: 'im-spacer' }), button);
}

function content(kind: InspectorKind, view: SectionView, row: RowView | null, on: Handlers): DocumentFragment {
  const fragment = document.createDocumentFragment();
  if (kind === 'details') {
    const body = h('div', { class: 'im-panel-body im-content-text', tabindex: 0, role: 'region', 'aria-label': 'Details' },
      ...(row === null ? [h('p', { class: 'xi-hint' }, 'Select a row to see its details: paths, files in use, why.')] : detailBlocks(row)));
    fragment.append(header('Details', row?.name ?? view.title, on.closeInspector), body);
  } else {
    const options = panelOptions(view);
    const body = h('div', { class: 'im-panel-body xi-options' },
      ...(options.length === 0 ? [h('p', { class: 'xi-hint' }, 'This section has no options.')] : options.map((o) => optionControl(view.section, o, on))));
    fragment.append(header('Options', view.title, on.closeInspector), body);
  }
  return fragment;
}

/** The control in the inspector that has the focus, and what a number field in it holds: both are put back after a redraw. */
function kept(region: Element | null): { key: string; value: string | null } | null {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement) || region === null || !region.contains(active) || active.dataset.key === undefined) return null;
  return { key: active.dataset.key, value: active instanceof HTMLInputElement && active.type === 'number' ? active.value : null };
}

/** Whether the inspector is open now (the package closes it with Escape, behind the page's back). */
export function inspectorOpen(shell: HTMLElement): boolean {
  return inspector(shell).isOpen();
}

/** The inspector follows `ui.inspector`: closed when it is null, else the details or the options, drawn again for this view. */
export function renderInspector(shell: HTMLElement, view: SectionView, ui: UiState, row: RowView | null, on: Handlers): void {
  const panel = inspector(shell);
  const kind = ui.inspector;
  if (kind === null) {
    if (panel.isOpen()) panel.close();
    return;
  }
  const region = shell.querySelector('.im-inspector');
  const before = kept(region);
  const body = content(kind, view, row, on);
  // Opened without an opener: the focus stays where it is (on the list, after `i`, so the arrows keep moving the selection the details
  // follow; on the button that was pressed). The header is built again at every render, so an opener would be a detached button by
  // the time the inspector closes; the page gives the focus back itself (`focusAfterInspector`, main.ts). F6 moves into the inspector.
  panel.open(body);
  if (before === null) return;
  const again = region?.querySelector<HTMLElement>(`[data-key="${CSS.escape(before.key)}"]`);
  if (!again) return;
  if (before.value !== null && again instanceof HTMLInputElement) again.value = before.value;
  again.focus();
}
