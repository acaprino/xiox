// The frame of the window, written once at start with Imago's `shellFrameHtml`: the seven regions of the canon (title bar,
// sidebar, status row, content, inspector, action bar, footer). It returns the shell; the regions are named by ids so that the
// views (render/shell.ts, sidebar.ts) fill them on each render without touching the frame. The content's header and tools sit
// before its panel body, so that a panel state (render/list.ts) keeps them.

import { iconHtml, shellFrameHtml, sidebarHtml, titlebarHtml } from '@acaprino/imago/ui';
import { fromHtml } from '../dom.ts';

/** Names a region of the frame: the views find it by its id. */
function name(shell: HTMLElement, selector: string, id: string, ...classes: string[]): HTMLElement {
  const node = shell.querySelector<HTMLElement>(selector);
  if (!node) throw new Error(`the frame has no ${selector}`);
  node.id ||= id;
  node.classList.add(...classes);
  return node;
}

/** Builds the shell in the body of `doc` and returns it. */
export function buildFrame(doc: Document): HTMLElement {
  const shell = doc.createElement('div');
  shell.id = 'app';
  shell.className = 'im-shell';
  shell.setAttribute('data-im-sidebar', 'on');
  shell.replaceChildren(fromHtml(doc, shellFrameHtml({
    titlebarHtml: titlebarHtml({
      brand: { name: 'Xiox' },
      sidebarToggle: { label: 'Categories', controls: 'sidebar' },
      navigationHtml: '<nav id="sections" class="im-tabs" role="tablist" aria-label="Sections"></nav>',
      views: { label: 'Views', action: 'views' },
      commands: { label: 'Commands', action: 'commands', shortcut: 'Ctrl K' },
      windowControls: true,
    }),
    sidebarHtml: sidebarHtml({ id: 'sidebar', label: 'Categories', rowsHtml: '', collapse: { collapse: 'Collapse sidebar', expand: 'Expand sidebar' }, count: 0 }),
    statusHtml: '<span id="badge" class="im-chip"></span><span id="status-text" class="im-status-text" role="status" aria-atomic="true"></span>'
      + '<span id="job" class="im-progress-row xi-job" hidden></span><span class="im-spacer"></span><span id="status-right" class="im-status-end"></span>',
    contentLabel: 'Content',
    contentHeaderHtml: '<h4 class="im-panel-title"></h4><span class="im-panel-sub"></span><span class="im-spacer"></span>',
    contentToolsHtml: '<div id="notices" class="im-content-text"></div>',
    contentHtml: '<div id="rows"></div>',
    actionsHtml: '',
    footerHtml: '',
    skipLabel: 'Skip to content',
    inspectorLabel: 'Details and options',
  })));
  doc.body.append(shell);
  name(shell, '.im-titlebar', 'top');
  name(shell, '.im-body', 'body');
  name(shell, '.im-status', 'status');
  name(shell, '.im-content > .im-panel-header', 'list-head');
  name(shell, '.im-content > .im-panel-tools', 'notices-row');
  name(shell, '.im-content > .im-panel-body', 'content-body', 'xi-body');
  name(shell, '.im-actions', 'actions');
  name(shell, '.im-footer', 'foot');
  name(shell, '.im-sidebar > .im-panel-body', 'sidebar-list', 'xi-side');
  // titlebarHtml writes the Views button and the command button as their words only; the package's steps at 620 (the command button is
  // its icon) and 340 (the Views button is its chevron) are drawn for the markup of docs/using-imago.md section 4, so they are written so.
  const views = shell.querySelector<HTMLElement>('.im-views-menu');
  if (views) {
    views.classList.add('im-btn--sm');
    views.setAttribute('aria-haspopup', 'menu');
    views.setAttribute('aria-expanded', 'false');
    views.replaceChildren(doc.createTextNode('Views'), fromHtml(doc, iconHtml('chevron')));
  }
  const command = shell.querySelector<HTMLElement>('.im-titlebar-command');
  if (command) command.replaceChildren(fromHtml(doc, iconHtml('search')), fromHtml(doc, '<b>Commands</b>'), fromHtml(doc, '<kbd class="im-key">Ctrl K</kbd>'));
  return shell;
}
