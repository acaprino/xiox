import './styles/tokens.css';
import './styles/frame.css';
import './styles/shell.css';
import './styles/list.css';
import './styles/dialog.css';
import { api, errorText, onView } from './api.ts';
import { actionIntent, toggleIntent } from './intents.ts';
import { actionForKey, keyAction } from './keys.ts';
import { renderFailure, renderShell, type Handlers } from './render/shell.ts';
import { rowParents, sortRows, visibleRows } from './rows.ts';
import { acceptView, createGate, createUi, currentFocus, moveFocus, SERVICES_POLL_MS, shouldPollServices } from './state.ts';
import type { AnyIntent, AppInfo, SectionId } from './types.ts';

const ui = createUi();
let app: AppInfo | null = null;
/** The last failed command and the section it concerns: shown only while that section is open; any later success, a section switch or a push for another section clears it. */
let error: { section: SectionId; text: string } | null = null;
/** Rapid clicks on the section bar: only the answer to the last one is drawn. */
const opening = createGate();

const SCROLLERS = ['#bar2 .items', '#rows .grid', '#rows .rp-body'];

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
  if (intent === null) return;
  try {
    const view = await api.intent(section, intent);
    error = null;
    acceptView(ui, view);
  } catch (failure) {
    error = { section, text: errorText(failure) };
  }
  render();
}

const handlers: Handlers = {
  openSection: (id) => { void open(id); },
  send: (section, intent) => { void send(section, intent); },
  focusRow: (id) => { ui.focus[ui.section] = id; render(); },
  toggleExpanded: (id) => {
    if (!ui.expanded.delete(id)) ui.expanded.add(id);
    render();
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
 * row, Enter / a / s / r press the view's action that carries that key, Esc closes the dialog or the
 * report, else cancels the job. The page presses only what the view offers enabled; Rust still decides.
 */
function onKey(event: KeyboardEvent): void {
  const view = ui.views[ui.section];
  if (!view || event.defaultPrevented) return;
  const target = event.target instanceof Element ? event.target : null;
  const inField = target?.closest(FIELD) != null;
  const act = keyAction(event, { inField, dialog: view.dialog !== null });
  if (act === null) return;
  if ((event.key === 'Enter' || event.key === ' ') && target?.closest(PRESSABLE) != null) return;
  const section = view.section;
  const rows = visibleRows(sortRows(view.rows), ui.expanded);
  const listed = view.report === null;
  switch (act.kind) {
    case 'section':
      if (act.section !== section) void open(act.section);
      break;
    case 'closeDialog':
      handlers.send(section, { kind: 'closeDialog' });
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
      if (!listed || id === null || !rowParents(view.rows).has(id)) return;
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
