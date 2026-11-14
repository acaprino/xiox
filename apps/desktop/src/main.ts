import './styles/tokens.css';
import './styles/frame.css';
import './styles/shell.css';
import './styles/list.css';
import './styles/dialog.css';
import { api, errorText, onView } from './api.ts';
import { actionIntent, toggleIntent } from './intents.ts';
import { actionForKey, keyAction } from './keys.ts';
import { aboutOpen, closeAbout, showAbout } from './render/about.ts';
import { renderFailure, renderShell, type Handlers } from './render/shell.ts';
import { rowParents } from './rows.ts';
import { acceptView, createGate, createInflight, createUi, currentFocus, foldedGroups, moveFocus, SERVICES_POLL_MS, shouldPollServices, shownRows } from './state.ts';
import type { AnyIntent, AppInfo, SectionId } from './types.ts';

const ui = createUi();
let app: AppInfo | null = null;
/** The last failed command and the section it concerns: shown only while that section is open; any later success, a section switch or a push for another section clears it. */
let error: { section: SectionId; text: string } | null = null;
/** Rapid clicks on the section bar: only the answer to the last one is drawn. */
const opening = createGate();
/** The intents waiting for Rust's answer: a repeat of one of them (a double click) is not sent. */
const inflight = createInflight();

const SCROLLERS = ['#bar2 .items', '#rows .grid', '#rows .rp-body'];

/**
 * Where the list of categories is scrolled to, for the menu beside it. Each category header is preceded by an empty `.anchor`
 * that does not stick, so its place is its real place in the list (the sticky header itself moves while it is stuck).
 */
const SPY_PIN_MS = 900;
/** After a click on the menu the marked entry is the one clicked, however far the scroll gets (the last categories cannot reach the top). */
let spyPinUntil = 0;
const gridNode = (): HTMLElement | null => document.querySelector<HTMLElement>('#rows .grid');
const headHeight = (grid: HTMLElement): number => grid.querySelector<HTMLElement>('.r.h')?.offsetHeight ?? 24;
const reducedMotion = (): boolean => window.matchMedia('(prefers-reduced-motion: reduce)').matches;

function anchorTop(grid: HTMLElement, anchor: HTMLElement): number {
  return anchor.getBoundingClientRect().top - grid.getBoundingClientRect().top + grid.scrollTop;
}

function scrollToGroup(id: string): void {
  const grid = gridNode();
  const anchor = grid?.querySelector<HTMLElement>(`.anchor[data-anchor="${CSS.escape(id)}"]`);
  if (!grid || !anchor) return;
  grid.scrollTo({ top: Math.max(0, anchorTop(grid, anchor) - headHeight(grid)), behavior: reducedMotion() ? 'auto' : 'smooth' });
}

/** Marks, in the menu, the category the list is at: the last one whose header has reached the top. */
function updateSpy(): void {
  const view = ui.views[ui.section];
  const grid = gridNode();
  if (!view || !grid) return;
  const anchors = [...grid.querySelectorAll<HTMLElement>('.anchor')];
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
  for (const entry of document.querySelectorAll<HTMLElement>('#bar2 .it.menu')) {
    const here = entry.dataset.group === id;
    entry.classList.toggle('on', here);
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
    if (lost && view.dialog === null && !aboutOpen()) document.querySelector<HTMLElement>(reporting ? '#rows .report' : '#rows .grid')?.focus();
  }
}

async function open(section: SectionId): Promise<void> {
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

/** The About window: the page's own, with no intent. A confirmation that is open keeps the keyboard, so About waits. */
async function openAbout(): Promise<void> {
  if (aboutOpen() || ui.views[ui.section]?.dialog != null) return;
  const from = document.activeElement instanceof HTMLElement ? document.activeElement.dataset.key ?? null : null;
  try {
    showAbout(await api.about(), from, closeAbout);
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
  document.querySelector<HTMLElement>('#rows .r.sel')?.scrollIntoView({ block: 'nearest' });
}

const FIELD = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]';
/** Enter and Space on these belong to the control (the browser presses it), never to the global keys. */
const PRESSABLE = 'button, a[href], summary, [role="checkbox"], [role="button"]';

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
      const next = moveFocus(rows, currentFocus(ui, view), act.delta);
      if (next !== null) { ui.focus[section] = next; render(); showSelected(); }
      break;
    }
    case 'toggle': {
      const row = rows.find((r) => r.id === currentFocus(ui, view));
      if (!listed || !row || row.kind === 'header' || row.check === 'none' || !row.enabled || view.job !== null) return;
      handlers.send(section, toggleIntent(section, row.id));
      break;
    }
    case 'expand': {
      const id = currentFocus(ui, view);
      if (!listed || id === null) return;
      if (rows.find((r) => r.id === id)?.kind === 'group') {
        ui.fold.set(id, !act.open);
        render();
        break;
      }
      if (!rowParents(view.rows).has(id)) return;
      if (act.open) ui.expanded.add(id); else ui.expanded.delete(id);
      render();
      break;
    }
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

/**
 * The RAM meter moves by itself: while Services & RAM is shown, the window visible and no dialog open, its view is
 * read again every 2 s. A reading that comes back after a newer view (an intent answered meanwhile) is dropped by
 * `acceptView`; a failed one is not worth a message (the next one will say, and an intent shows its own errors).
 */
function pollServices(): void {
  if (!shouldPollServices(ui, document.visibilityState === 'visible')) return;
  void api.view('services').then((view) => {
    if (acceptView(ui, view) && ui.section === 'services') render();
  }, () => undefined);
}

setInterval(pollServices, SERVICES_POLL_MS);

async function start(): Promise<void> {
  try {
    app = await api.appInfo();
    onView((view) => {
      // News about another section ends an error that was about this one.
      if (error !== null && error.section !== view.section) error = null;
      if (acceptView(ui, view) && view.section === ui.section) render();
    });
    await open('cleanup');
  } catch (failure) {
    // No window state to draw: say what happened instead of leaving a blank window.
    renderFailure(errorText(failure));
  }
}

void start();
