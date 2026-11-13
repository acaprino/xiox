import './styles/tokens.css';
import './styles/frame.css';
import './styles/shell.css';
import './styles/list.css';
import { api, errorText, onView } from './api.ts';
import { renderFailure, renderShell, type Handlers } from './render/shell.ts';
import { acceptView, createGate, createUi } from './state.ts';
import type { AnyIntent, AppInfo, SectionId } from './types.ts';

const ui = createUi();
let app: AppInfo | null = null;
/** The last failed command and the section it concerns: shown only while that section is open; any later success, a section switch or a push for another section clears it. */
let error: { section: SectionId; text: string } | null = null;
/** Rapid clicks on the section bar: only the answer to the last one is drawn. */
const opening = createGate();

const SCROLLERS = ['#bar2 .items', '#rows .grid'];

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
  renderShell({ app, view, ui, error: error !== null && error.section === ui.section ? error.text : null }, handlers);
  SCROLLERS.forEach((selector, i) => {
    const node = document.querySelector(selector);
    if (node) node.scrollTop = scrolls[i] ?? 0;
  });
  if (active !== undefined && !(document.activeElement instanceof HTMLElement && document.activeElement.dataset.key === active)) {
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
  addRoot: () => {
    void api.addRoot().then((view) => { error = null; acceptView(ui, view); render(); }, (failure) => { error = { section: ui.section, text: errorText(failure) }; render(); });
  },
};

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
