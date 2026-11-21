import '@acaprino/imago/css/imago.css';
import './styles/xiox.css';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { guardDesktop, mountWindowControls, mountZoom } from '@acaprino/imago/shell';
import { tauriWindowApi } from '@acaprino/imago/tauri';
import { api, errorText, inTauri, onView } from './api.ts';
import { actionIntent, toggleIntent } from './intents.ts';
import { actionForKey, chordAction, keyAction, SELECTABLE } from './keys.ts';
import { applyLook } from '@acaprino/imago/shell';
import { LOOK_KEY, parseTheme, readLook, readTheme, THEME_KEY, type Look, type ThemeChoice } from './look.ts';
import { createRegistry, mountApp, openMenu, PALETTE_COMMAND } from '@acaprino/imago/ui';
import { aboutOpen, closeAbout, showAbout } from './render/about.ts';
import { buildFrame } from './render/frame.ts';
import { inspectorOpen } from './render/inspector.ts';
import { renderFailure, renderShell, type Handlers } from './render/shell.ts';
import { cycleSort, nextSort, rowParents } from './rows.ts';
import { acceptView, createGate, createInflight, createUi, currentFocus, foldedGroups, moveFocus, SECTION_ORDER, shownRows } from './state.ts';
import type { AnyIntent, AppInfo, SectionId } from './types.ts';

const ui = createUi();
let app: AppInfo | null = null;
/** The last failed command and the section it concerns: shown only while that section is open; any later success, a section switch or a push for another section clears it. */
let error: { section: SectionId; text: string } | null = null;
/** Rapid clicks on the section bar: only the answer to the last one is drawn. */
const opening = createGate();
/** The intents waiting for Rust's answer: a repeat of one of them (a double click) is not sent. */
const inflight = createInflight();

/** The frame of the window (render/frame.ts), written once; the views fill its regions. */
const shell = buildFrame(document);

/** Whether the sidebar is folded into its rail: kept for the next start, the package stores nothing. */
const FOLD_KEY = 'xiox.sidebar';
function readFold(): boolean {
  try { return localStorage.getItem(FOLD_KEY) === '1'; } catch { return false; }
}

/** Escape and the press outside, the sidebar's drawer and fold, F6, Ctrl+K and Ctrl+F: Imago's, mounted before the page's own keys (below) so they answer first. */
const registry = createRegistry();
mountApp(shell, registry, {
  search: () => document.querySelector<HTMLInputElement>('#rows .xi-search input')?.focus(),
  sidebar: {
    collapsed: readFold(),
    onChange: (collapsed) => { try { localStorage.setItem(FOLD_KEY, collapsed ? '1' : '0'); } catch { /* no storage: the fold lasts until the window closes */ } },
  },
});

const SECTION_TITLES: Record<SectionId, string> = { cleanup: 'Cleanup', registry: 'Registry', developers: 'Developers' };
// The title bar's two buttons: the views (what the tabs are, when the window is too narrow for them) and the palette.
const viewsButton = shell.querySelector<HTMLElement>('.im-views-menu');
viewsButton?.addEventListener('click', () => openMenu(viewsButton, [
  ...SECTION_ORDER.map((id, index) => ({ label: SECTION_TITLES[id], keys: String(index + 1), checked: () => ui.section === id, run: () => { void open(id); } })),
  { label: 'Settings', keys: 'Shift+?', run: () => { void openAbout(); } },
]));
shell.querySelector<HTMLElement>('.im-titlebar-command')?.addEventListener('click', () => { void registry.get(PALETTE_COMMAND)?.run(); });

// Escape closes the inspector in the package's own handler, behind the page's back: the page follows it.
const inspectorRegion = shell.querySelector<HTMLElement>('.im-inspector');
if (inspectorRegion) {
  new MutationObserver(() => {
    if (inspectorRegion.hidden && ui.inspector !== null) { ui.inspector = null; render(); }
  }).observe(inspectorRegion, { attributes: true, attributeFilter: ['hidden'] });
}

const SCROLLERS = ['#sidebar-list', '#rows .xi-list', '#rows .rp-body'];

/**
 * Where the list of categories is scrolled to, for the menu beside it. Each category header is preceded by an empty `.anchor`
 * that does not stick, so its place is its real place in the list (the sticky header itself moves while it is stuck).
 */
const SPY_PIN_MS = 900;
/** After a click on the menu the marked entry is the one clicked, however far the scroll gets (the last categories cannot reach the top). */
let spyPinUntil = 0;
const gridNode = (): HTMLElement | null => document.querySelector<HTMLElement>('#rows .xi-list');
const headHeight = (grid: HTMLElement): number => grid.querySelector<HTMLElement>('thead')?.offsetHeight ?? 24;
const reducedMotion = (): boolean => window.matchMedia('(prefers-reduced-motion: reduce)').matches;

function anchorTop(grid: HTMLElement, anchor: HTMLElement): number {
  return anchor.getBoundingClientRect().top - grid.getBoundingClientRect().top + grid.scrollTop;
}

function scrollToGroup(id: string): void {
  const grid = gridNode();
  const anchor = grid?.querySelector<HTMLElement>(`.xi-anchor[data-anchor="${CSS.escape(id)}"]`);
  if (!grid || !anchor) return;
  grid.scrollTo({ top: Math.max(0, anchorTop(grid, anchor) - headHeight(grid)), behavior: reducedMotion() ? 'auto' : 'smooth' });
}

/** Marks, in the menu, the category the list is at: the last one whose header has reached the top. */
function updateSpy(): void {
  const view = ui.views[ui.section];
  const grid = gridNode();
  if (!view || !grid) return;
  const anchors = [...grid.querySelectorAll<HTMLElement>('.xi-anchor')];
  if (anchors.length === 0) return;
  let id = ui.spy[ui.section];
  if (performance.now() >= spyPinUntil || id === undefined) {
    const line = grid.getBoundingClientRect().top + headHeight(grid) + 8;
    id = anchors[0]?.dataset.anchor;
    for (const anchor of anchors) {
      if (anchor.getBoundingClientRect().top <= line) id = anchor.dataset.anchor; else break;
    }
  }
  if (id === undefined) return;
  // The menu follows the list into view only when the category changed: a render must not pull the bar back from the options the user scrolled to.
  const moved = ui.spy[ui.section] !== id;
  ui.spy[ui.section] = id;
  for (const entry of document.querySelectorAll<HTMLElement>('#sidebar .im-row[data-group]')) {
    const here = entry.dataset.group === id;
    entry.setAttribute('aria-selected', String(here));
    if (here) { entry.setAttribute('aria-current', 'true'); if (moved) entry.scrollIntoView({ block: 'nearest' }); } else entry.removeAttribute('aria-current');
  }
}

let spyFrame = 0;
const rowsArea = document.getElementById('rows');
rowsArea?.addEventListener('scroll', () => {
  cancelAnimationFrame(spyFrame);
  spyFrame = requestAnimationFrame(updateSpy);
}, { capture: true, passive: true });
// The user takes the scroll back from a click on the menu.
rowsArea?.addEventListener('wheel', () => { spyPinUntil = 0; }, { passive: true });
rowsArea?.addEventListener('touchmove', () => { spyPinUntil = 0; }, { passive: true });
/** Whether the last render drew a report: when that changes the list (or the report) is rebuilt and the focused control is gone. */
let reportShown = false;

/** Draws the open section's view and keeps what the user was doing: the focused control and the scroll positions. */
function render(): void {
  const view = ui.views[ui.section];
  if (!app || !view) {
    // Nothing to draw yet and a command failed: say so rather than leave a blank window.
    if (error !== null) renderFailure(error.text);
    return;
  }
  const active = document.activeElement instanceof HTMLElement ? document.activeElement.dataset.key : undefined;
  const scrolls = SCROLLERS.map((selector) => document.querySelector(selector)?.scrollTop ?? 0);
  renderShell({ app, view, ui, focused: active ?? null, error: error !== null && error.section === ui.section ? error.text : null }, handlers);
  SCROLLERS.forEach((selector, i) => {
    const node = document.querySelector(selector);
    if (node) node.scrollTop = scrolls[i] ?? 0;
  });
  // An open dialog manages its own focus (render/dialog.ts); the rest of the page is inert behind it.
  if (view.dialog === null && active !== undefined && !(document.activeElement instanceof HTMLElement && document.activeElement.dataset.key === active)) {
    document.querySelector<HTMLElement>(`[data-key="${CSS.escape(active)}"]`)?.focus();
  }
  updateSpy();
  // A report opening or closing replaces the list: if the focus fell to the page, put it where the user is working
  // (the report, then the list again) instead of leaving a screen reader on nothing.
  const reporting = view.report !== null;
  if (reporting !== reportShown) {
    reportShown = reporting;
    const lost = document.activeElement === null || document.activeElement === document.body;
    if (lost && view.dialog === null && !aboutOpen()) document.querySelector<HTMLElement>(reporting ? '#rows .report' : '#rows .xi-table')?.focus();
  }
  syncChrome();
}

async function open(section: SectionId): Promise<void> {
  // The tab answers the click at once; the next render (shell.ts) clears the mark, whatever came back.
  document.querySelector(`#sections [data-section="${section}"]`)?.setAttribute('aria-busy', 'true');
  await opening.run(
    () => api.show(section),
    (view) => {
      acceptView(ui, view);
      ui.section = section;
      error = null;
      render();
    },
    (failure) => {
      // The section stays as it was: the error belongs to the one on screen.
      error = { section: ui.section, text: errorText(failure) };
      render();
    },
  );
}

async function send(section: SectionId, intent: AnyIntent | null): Promise<void> {
  if (intent === null || !inflight.begin(section, intent)) return;
  try {
    const view = await api.intent(section, intent);
    error = null;
    acceptView(ui, view);
  } catch (failure) {
    error = { section, text: errorText(failure) };
  } finally {
    inflight.end(section, intent);
  }
  render();
}

/** The Settings window: the page's own, with no intent. A confirmation that is open keeps the keyboard, so Settings waits. */
async function openAbout(): Promise<void> {
  if (aboutOpen() || ui.views[ui.section]?.dialog != null) return;
  const from = document.activeElement instanceof HTMLElement ? document.activeElement.dataset.key ?? null : null;
  try {
    showAbout(await api.about(), from, { skin: look, choose: chooseLook }, closeAbout);
  } catch (failure) {
    error = { section: ui.section, text: errorText(failure) };
    render();
  }
}

const handlers: Handlers = {
  openSection: (id) => { void open(id); },
  openAbout: () => { void openAbout(); },
  send: (section, intent) => { void send(section, intent); },
  focusRow: (id) => { ui.focus[ui.section] = id; render(); },
  toggleRow: (id, section) => {
    ui.focus[section] = id;
    void send(section, toggleIntent(section, id));
  },
  toggleExpanded: (id) => {
    if (!ui.expanded.delete(id)) ui.expanded.add(id);
    render();
  },
  toggleFold: (id) => {
    const view = ui.views[ui.section];
    if (!view) return;
    ui.fold.set(id, !foldedGroups(ui, view).has(id));
    ui.focus[ui.section] = id;
    render();
  },
  foldAll: (fold) => {
    const view = ui.views[ui.section];
    if (!view) return;
    for (const row of view.rows) if (row.kind === 'group') ui.fold.set(row.id, fold);
    render();
  },
  toggleInspector: (kind) => {
    // The inspector may have been closed by Escape since the last render: what is open now decides.
    ui.inspector = ui.inspector === kind && inspectorOpen(shell) ? null : kind;
    render();
  },
  closeInspector: () => {
    ui.inspector = null;
    render();
  },
  sortBy: (column) => {
    const view = ui.views[ui.section];
    if (!view) return;
    const key = nextSort(view.columns, ui.sort[ui.section] ?? null, column);
    if (key === null) delete ui.sort[ui.section]; else ui.sort[ui.section] = key;
    render();
  },
  goToGroup: (id) => {
    const view = ui.views[ui.section];
    if (!view) return;
    if (foldedGroups(ui, view).has(id)) ui.fold.set(id, false);
    ui.focus[ui.section] = id;
    ui.spy[ui.section] = id;
    spyPinUntil = performance.now() + SPY_PIN_MS;
    render();
    scrollToGroup(id);
  },
  saveReport: (section) => {
    void api.saveReport(section).then((view) => { error = null; acceptView(ui, view); render(); }, (failure) => { error = { section, text: errorText(failure) }; render(); });
  },
  addRoot: () => {
    void api.addRoot().then((view) => { error = null; acceptView(ui, view); render(); }, (failure) => { error = { section: ui.section, text: errorText(failure) }; render(); });
  },
};

/** The row the selection is on, after a move: the page scrolls to keep it in view. */
function showSelected(): void {
  document.querySelector<HTMLElement>('#rows tr[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
}

const FIELD = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]';
/** Enter and Space on these belong to the control (the browser presses it), never to the global keys. */
const PRESSABLE = 'button, a[href], summary, input[type="checkbox"], input[type="radio"], [role="button"]';

/**
 * The global keys (keys.ts decides what a key means). Arrows move the selection, Space ticks the focused
 * row, Enter / a press the view's action that carries that key, Esc closes the dialog or the
 * report, else cancels the job. The page presses only what the view offers enabled; Rust still decides.
 */
function onKey(event: KeyboardEvent): void {
  const view = ui.views[ui.section];
  if (!view || event.defaultPrevented) return;
  const target = event.target instanceof Element ? event.target : null;
  const inField = target?.closest(FIELD) != null;
  const act = keyAction(event, { inField, dialog: view.dialog !== null || aboutOpen() });
  if (act === null) return;
  if ((event.key === 'Enter' || event.key === ' ') && target?.closest(PRESSABLE) != null) return;
  const section = view.section;
  const rows = shownRows(ui, view);
  const listed = view.report === null;
  switch (act.kind) {
    case 'section':
      if (act.section !== section) void open(act.section);
      break;
    case 'closeDialog':
      if (aboutOpen()) closeAbout(); else handlers.send(section, { kind: 'closeDialog' });
      break;
    case 'about':
      void openAbout();
      break;
    case 'move': {
      if (!listed) return;
      const next = moveFocus(rows, currentFocus(ui, view), act.delta, rowParents(view.rows));
      if (next !== null) { ui.focus[section] = next; render(); showSelected(); }
      break;
    }
    case 'toggle': {
      const row = rows.find((r) => r.id === currentFocus(ui, view));
      if (!listed || !row || row.check === 'none' || !row.enabled || view.job !== null) return;
      handlers.send(section, toggleIntent(section, row.id));
      break;
    }
    case 'expand': {
      const id = currentFocus(ui, view);
      if (!listed || id === null) return;
      const parents = rowParents(view.rows);
      // A category, and a folder of the projects' tree (a sub-heading with rows under it), fold; a parent row expands.
      const kind = rows.find((r) => r.id === id)?.kind;
      if (kind === 'group' || (kind === 'header' && parents.has(id))) {
        ui.fold.set(id, !act.open);
        render();
        break;
      }
      if (!parents.has(id)) return;
      if (act.open) ui.expanded.add(id); else ui.expanded.delete(id);
      render();
      break;
    }
    case 'sort': {
      if (!listed) return;
      const key = cycleSort(view.columns, ui.sort[section] ?? null, act.reverse);
      if (key === null) delete ui.sort[section]; else ui.sort[section] = key;
      render();
      showSelected();
      break;
    }
    case 'details':
    case 'options':
      if (!listed) return;
      handlers.toggleInspector(act.kind);
      break;
    case 'action': {
      if (act.key === 'Esc' && inField) { (target as HTMLElement).blur(); break; }
      if (act.key === 'Esc' && view.report !== null) { handlers.send(section, { kind: 'closeDialog' }); break; }
      const action = actionForKey(view, act.key);
      if (action === null) return;
      handlers.send(section, actionIntent(section, action.id));
      break;
    }
  }
  event.preventDefault();
}

document.addEventListener('keydown', onKey);

/** Ctrl+A selects the copyable text of the region the focus is in (keys.ts `SELECTABLE`), else nothing. */
function selectAll(target: Element | null): void {
  for (const [within, select] of SELECTABLE) {
    const region = target?.closest(within);
    const text = region === null || region === undefined ? null : region.matches(select) ? region : region.querySelector(select);
    if (text) { window.getSelection()?.selectAllChildren(text); return; }
  }
  window.getSelection()?.removeAllRanges();
}

/**
 * The look and the theme (look.ts), Imago's: written on the root by `applyLook` before the first view is drawn, and again at
 * every change, also when Windows changes its colours while the theme follows them. The window is asked for the same theme
 * (`syncChrome`), so its title bar and the page agree. Both looks are glass in the desktop window.
 */
let look: Look = 'smooth';
let theme: ThemeChoice = 'system';
const systemDark = window.matchMedia('(prefers-color-scheme: dark)');
function drawLook(): void {
  applyLook(document.documentElement, { app: 'xiox', look, theme, glass: inTauri }, {
    // The window is told the theme of the look, with the look and the theme drawn, and keeps the theme for the first frame of the next
    // start; outside the window (Vite, the smoke test) there is nobody to tell.
    setWindowTheme: inTauri ? (word, detail) => api.setWindowTheme(word, detail.look, detail.resolved).catch(() => undefined) : undefined,
  });
  for (const choice of document.querySelectorAll<HTMLInputElement>('input[data-theme-choice]')) choice.checked = choice.dataset.themeChoice === theme;
  syncChrome();
}
systemDark.addEventListener('change', () => { if (theme === 'system') drawLook(); });

for (const choice of document.querySelectorAll<HTMLElement>('input[data-theme-choice]')) {
  choice.addEventListener('change', () => {
    theme = parseTheme(choice.dataset.themeChoice ?? null);
    drawLook();
    try { localStorage.setItem(THEME_KEY, theme); } catch { /* no storage: the theme lasts until the window closes */ }
  });
}

/** The look chosen in Settings (render/about.ts), kept for the next start. */
function chooseLook(next: Look): void {
  look = next;
  drawLook();
  try { localStorage.setItem(LOOK_KEY, next); } catch { /* no storage: the look lasts until the window closes */ }
}

/** The app's own chord, in a field or not (keys.ts `chordAction`): Ctrl+A selects the copyable text. Zoom is Imago's (`mountZoom`, in `start`). */
document.addEventListener('keydown', (event) => {
  const target = event.target instanceof Element ? event.target : null;
  const chord = chordAction(event, { inField: target?.closest(FIELD) != null });
  if (chord === null) return;
  event.preventDefault();
  selectAll(target);
});

/** The webview's storage, or null when it refuses it (a blocked profile). */
function safeStorage(): Storage | null {
  try { return localStorage; } catch { return null; }
}

async function start(): Promise<void> {
  // Imago's window code. The guard takes away the browser's keys, its menu and the mouse's back and forward buttons (it also takes
  // reload and DevTools, so not under `tauri dev`); the zoom is 50 to 200 % in steps of 10, saved, applied at every start (a reload
  // after a crash keeps the webview's last factor); the window buttons report their failures as a notice.
  if (!import.meta.env.DEV) guardDesktop(document);
  if (inTauri) {
    mountZoom(document, { apply: (level) => getCurrentWebview().setZoom(level), storageKey: 'xiox.zoom' });
  }
  look = readLook(safeStorage());
  theme = readTheme(safeStorage());
  drawLook();
  try {
    app = await api.appInfo();
    onView((view) => {
      // News about another section ends an error that was about this one.
      if (error !== null && error.section !== view.section) error = null;
      if (acceptView(ui, view) && view.section === ui.section) render();
      syncChrome();
    });
    await open('cleanup');
  } catch (failure) {
    // No window state to draw: say what happened instead of leaving a blank window.
    renderFailure(errorText(failure));
  }
  focusList();
  signalReady();
  const titlebar = document.getElementById('top');
  if (inTauri && titlebar !== null) {
    const onError = (action: string, failure: unknown): void => { error = { section: ui.section, text: `${action} failed: ${errorText(failure)}` }; render(); };
    mountWindowControls(titlebar, tauriWindowApi(onError), { onError });
  }
}

/** The list is where the keys (arrows, Space, Enter) work: it takes the focus at the start and when the window gets it back. */
function focusList(): void {
  const dialog = document.querySelector<HTMLElement>('#modal:not([hidden]) [role=dialog]');
  (dialog ?? document.querySelector<HTMLElement>('#rows .xi-table') ?? document.getElementById('rows'))?.focus();
}

// A native dialog (folder, save, the recovery message) closing gives the window back, but WebView2 does not always put the
// focus back in the page: with nothing focused the list takes it, so the keys work without a click.
window.addEventListener('focus', () => {
  if (document.activeElement === null || document.activeElement === document.body) focusList();
});

/**
 * The window opens hidden and Rust shows it on this signal. The page is already in the DOM: once its first frame is on
 * screen (two animation frames: the second starts after the first was painted) the window can be seen. A hidden window may
 * run no animation frame at all, so a timer of 250 ms races them and the first to fire signals, once. A failure to ask is not shown:
 * Rust shows the window anyway after a few seconds.
 */
const READY_FALLBACK_MS = 250;
function signalReady(): void {
  let sent = false;
  const send = (): void => {
    if (sent) return;
    sent = true;
    void api.ready().catch(() => undefined);
  };
  requestAnimationFrame(() => requestAnimationFrame(send));
  setTimeout(send, READY_FALLBACK_MS);
}

/** What the window says outside the page (title, taskbar): the last thing sent, so that only a change is sent. */
let chromeSent = '';
function syncChrome(): void {
  const job = Object.values(ui.views).find((view) => view?.job)?.job ?? null;
  const next = { section: ui.section, job: job === null ? null : Math.min(100, Math.max(0, Math.round(job.fraction * 100))) };
  const key = JSON.stringify(next);
  if (key === chromeSent) return;
  chromeSent = key;
  void api.chrome(next.section, next.job).catch(() => undefined);
}

void start();
