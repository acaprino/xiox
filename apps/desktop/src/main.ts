import '@acaprino/imago/css/imago.css';
import './styles/xiox.css';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { guardDesktop, mountWindowControls, mountZoom } from '@acaprino/imago/shell';
import { tauriWindowApi } from '@acaprino/imago/tauri';
import { api, errorText, inTauri, onView } from './api.ts';
import { actionCommands, actionForKey, listKey, registerCommands, SELECTABLE, type CommandApp } from './commands.ts';
import { actionIntent, toggleIntent } from './intents.ts';
import { applyLook } from '@acaprino/imago/shell';
import { LOOK_KEY, readLook, readTheme, THEME_KEY, type Look, type ThemeChoice } from './look.ts';
import { claimKey, createRegistry, isTextTarget, mountApp, notify, openMenu, PALETTE_COMMAND } from '@acaprino/imago/ui';
import { buildFrame } from './render/frame.ts';
import { inspectorOpen } from './render/inspector.ts';
import { renderSettings } from './render/settings.ts';
import { renderFailure, renderShell, type Handlers } from './render/shell.ts';
import { cycleSort, nextSort, rowParents } from './rows.ts';
import { acceptView, createGate, createInflight, createUi, currentFocus, foldedGroups, moveFocus, SECTION_ORDER, shownRows } from './state.ts';
import type { AboutView, AnyIntent, AppInfo, SectionId } from './types.ts';

const ui = createUi();
let app: AppInfo | null = null;
/** The last failed command and the section it concerns: shown only while that section is open; any later success, a section switch or a push for another section clears it. */
let errorSection: SectionId | null = null;
/** The words of the last failed command: the error state of the panel says them when there is no view to draw. */
let lastFailure = '';
/** Closes the notice of the failed command on screen. */
let dismissError: (() => void) | null = null;

/** A command failed: an error notice that stays until it is dismissed, or until the next command of that section succeeds. */
function fail(section: SectionId, text: string): void {
  lastFailure = text;
  clearError();
  errorSection = section;
  dismissError = notify(shell, text, { tone: 'err' });
}

/** The next command succeeded, or the section changed: the notice of the failure goes. */
function clearError(): void {
  dismissError?.();
  dismissError = null;
  errorSection = null;
}
/** Rapid clicks on the section bar: only the answer to the last one is drawn. */
const opening = createGate();
/** The intents waiting for Rust's answer: a repeat of one of them (a double click) is not sent. */
const inflight = createInflight();

/** The look and the theme the user chose (look.ts); `drawLook` writes them on the root. Declared first: the commands of the look read them. */
let look: Look = 'smooth';
let theme: ThemeChoice = 'system';
/** What Rust says of the app, read when Settings opens. */
let about: AboutView | null = null;

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
  // The look and the theme from the palette (Smooth, ONI, follow Windows, light, dark): the same five commands in every app.
  appearance: { get: () => ({ look, theme }), set: (next) => applyAppearance(next) },
});

const SECTION_TITLES: Record<SectionId, string> = { cleanup: 'Cleanup', registry: 'Registry', developers: 'Developers' };
// The title bar's two buttons: the views (what the tabs are, when the window is too narrow for them) and the palette.
const viewsButton = shell.querySelector<HTMLElement>('.im-views-menu');
viewsButton?.addEventListener('click', () => openMenu(viewsButton, [
  ...SECTION_ORDER.map((id, index) => ({ label: SECTION_TITLES[id], keys: String(index + 1), checked: () => ui.section === id, run: () => { void open(id); } })),
  { label: 'Settings', keys: '4', checked: () => ui.settings, run: () => { void openSettings(); } },
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
    return;
  }
  viewActions.sync(view);
  const active = document.activeElement instanceof HTMLElement ? document.activeElement.dataset.key : undefined;
  const scrolls = SCROLLERS.map((selector) => document.querySelector(selector)?.scrollTop ?? 0);
  renderShell({ app, view, ui, registry }, handlers);
  // Settings is drawn over the section's sidebar, header, content and action bar; the status row, the footer and the window title stay the section's.
  if (ui.settings && about !== null) renderSettings(about, { look, theme }, applyAppearance);
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
    if (lost && view.dialog === null && !ui.settings) document.querySelector<HTMLElement>(reporting ? '#rows .report' : '#rows .xi-table')?.focus();
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
      ui.settings = false;
      clearError();
      render();
    },
    (failure) => {
      // The section stays as it was: the error belongs to the one on screen.
      fail(ui.section, errorText(failure));
      render();
    },
  );
}

async function send(section: SectionId, intent: AnyIntent | null): Promise<void> {
  if (intent === null || !inflight.begin(section, intent)) return;
  try {
    const view = await api.intent(section, intent);
    clearError();
    acceptView(ui, view);
  } catch (failure) {
    fail(section, errorText(failure));
  } finally {
    inflight.end(section, intent);
  }
  render();
}

/** Settings, the fourth view: the page's own, with no intent. A confirmation that is open keeps the keyboard, so Settings waits. */
async function openSettings(): Promise<void> {
  if (ui.settings || ui.views[ui.section]?.dialog != null) return;
  try {
    about = await api.about();
  } catch (failure) {
    fail(ui.section, errorText(failure));
    render();
    return;
  }
  ui.settings = true;
  ui.inspector = null;
  render();
  document.querySelector<HTMLElement>('#sidebar-list [data-key^="settings:"]')?.focus();
}

const handlers: Handlers = {
  openSection: (id) => { void open(id); },
  openSettings: () => { void openSettings(); },
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
  listKey: (event) => {
    const view = ui.views[ui.section];
    const act = listKey(event);
    if (!view || act === null || !commandApp.listed()) return;
    const target = event.target instanceof Element ? event.target : null;
    // Enter and Space on a button, a box or a field of the list belong to it (the browser presses it).
    if ((act.kind === 'primary' || act.kind === 'toggle') && target?.closest('button, a[href], input, select, textarea') != null) return;
    const section = view.section;
    const rows = shownRows(ui, view);
    switch (act.kind) {
      case 'move': {
        const next = moveFocus(rows, currentFocus(ui, view), act.delta, rowParents(view.rows));
        if (next !== null) { ui.focus[section] = next; render(); showSelected(); }
        break;
      }
      case 'toggle': {
        const row = rows.find((r) => r.id === currentFocus(ui, view));
        if (!row || row.check === 'none' || !row.enabled || view.job !== null) return;
        handlers.send(section, toggleIntent(section, row.id));
        break;
      }
      case 'expand': {
        const id = currentFocus(ui, view);
        if (id === null) return;
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
      case 'primary': {
        const action = actionForKey(view, 'Enter');
        if (action === null) return;
        handlers.send(section, actionIntent(section, action.id));
        break;
      }
    }
    claimKey(event);
    event.preventDefault();
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
    void api.saveReport(section).then((view) => { clearError(); acceptView(ui, view); render(); }, (failure) => { fail(section, errorText(failure)); render(); });
  },
  addRoot: () => {
    void api.addRoot().then((view) => { clearError(); acceptView(ui, view); render(); }, (failure) => { fail(ui.section, errorText(failure)); render(); });
  },
};

/** The row the selection is on, after a move: the page scrolls to keep it in view. */
function showSelected(): void {
  document.querySelector<HTMLElement>('#rows tr[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
}

/**
 * What the commands act on (commands.ts): the page's own state and the handlers below. Imago's registry answers the keys of the window
 * (Escape included) and the list answers its own (`handlers.listKey`); there is no `keydown` of the page's own on the document.
 */
const commandApp: CommandApp = {
  view: () => ui.views[ui.section],
  idle: () => app !== null && ui.views[ui.section] !== undefined && ui.views[ui.section]?.dialog === null,
  listed: () => commandApp.idle() && ui.views[ui.section]?.report === null && !ui.settings,
  inTextField: () => isTextTarget(document.activeElement),
  openSection: (id) => { if (id !== ui.section || ui.settings) void open(id); },
  openSettings: () => { void openSettings(); },
  send: (section, intent) => handlers.send(section, intent),
  sort: (reverse) => {
    const view = ui.views[ui.section];
    if (!view) return;
    const key = cycleSort(view.columns, ui.sort[view.section] ?? null, reverse);
    if (key === null) delete ui.sort[view.section]; else ui.sort[view.section] = key;
    render();
    showSelected();
  },
  toggleInspector: (kind) => handlers.toggleInspector(kind),
  selectAll: () => selectAll(document.activeElement),
  escape: () => {
    const view = ui.views[ui.section];
    if (!view) return;
    const active = document.activeElement;
    // A confirmation's Escape is its Cancel; in a field it leaves the field; a report closes; else the view's Esc action (it cancels a job).
    if (view.dialog !== null) { handlers.send(view.section, { kind: 'closeDialog' }); return; }
    if (isTextTarget(active) && active instanceof HTMLElement) { active.blur(); return; }
    if (view.report !== null) { handlers.send(view.section, { kind: 'closeDialog' }); return; }
    const action = actionForKey(view, 'Esc');
    if (action !== null) handlers.send(view.section, actionIntent(view.section, action.id));
  },
};
registerCommands(registry, commandApp);
/** The commands of the open view's actions: registered again when the view's actions change (render). */
const viewActions = actionCommands(registry, commandApp);

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

/** A new look or theme, from Settings or from the palette: drawn at once, kept for the next start, and Settings drawn again to say it. */
function applyAppearance(next: { look: Look; theme: ThemeChoice }): void {
  look = next.look;
  theme = next.theme;
  drawLook();
  try {
    localStorage.setItem(LOOK_KEY, look);
    localStorage.setItem(THEME_KEY, theme);
  } catch { /* no storage: the look and the theme last until the window closes */ }
  if (ui.settings) render();
}

/** The webview's storage, or null when it refuses it (a blocked profile). */
function safeStorage(): Storage | null {
  try { return localStorage; } catch { return null; }
}

/** Whether the page already listens to the views Rust pushes (a Retry must not listen twice). */
let listening = false;

/** The first views: the app's facts, then Cleanup. A failure leaves the content panel in its error state, with a Retry that asks again. */
async function load(): Promise<void> {
  try {
    app = await api.appInfo();
    if (!listening) {
      listening = true;
      onView((view) => {
        // News about another section ends an error that was about this one.
        if (errorSection !== null && errorSection !== view.section) clearError();
        if (acceptView(ui, view) && view.section === ui.section) render();
        syncChrome();
      });
    }
    await open('cleanup');
  } catch (failure) {
    renderFailure(errorText(failure), () => { void load(); });
    return;
  }
  // `open` reports its own failure as a notice; with no view there is nothing to draw but the error state.
  if (ui.views[ui.section] === undefined) {
    clearError();
    renderFailure(lastFailure || 'The first command did not answer.', () => { void load(); });
  }
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
  await load();
  focusList();
  signalReady();
  const titlebar = document.getElementById('top');
  if (inTauri && titlebar !== null) {
    const onError = (action: string, failure: unknown): void => { fail(ui.section, `${action} failed: ${errorText(failure)}`); render(); };
    mountWindowControls(titlebar, tauriWindowApi(onError), { onError });
  }
}

/** The list is where the keys (arrows, Space, Enter) work: it takes the focus at the start and when the window gets it back. */
function focusList(): void {
  const dialog = document.querySelector<HTMLElement>('dialog[open]');
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
