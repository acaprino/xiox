import './styles/tokens.css';
import './styles/frame.css';
import './styles/shell.css';
import { api, errorText, onView } from './api.ts';
import { renderShell } from './render/shell.ts';
import { accept, state } from './state.ts';
import type { SectionId, SectionView } from './types.ts';

function render(): void {
  const view = state.views[state.open];
  if (state.app && view) renderShell(state.app, view);
}

async function open(section: SectionId): Promise<void> {
  try {
    const view: SectionView = await api.show(section);
    accept(view);
    state.open = section;
    render();
  } catch (error) {
    document.getElementById('status-text')?.replaceChildren(errorText(error));
  }
}

async function start(): Promise<void> {
  state.app = await api.appInfo();
  onView((view) => {
    if (accept(view) && view.section === state.open) render();
  });
  document.getElementById('sections')?.addEventListener('click', (event) => {
    const target = (event.target as Element).closest<HTMLElement>('[data-section]');
    if (target) void open(target.dataset.section as SectionId);
  });
  await open('cleanup');
}

void start();
