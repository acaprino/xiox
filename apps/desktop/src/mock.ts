// The Tauri commands answered without Tauri (the browser at `npm run dev`, the Playwright smoke
// test): one MockModel over fixtures/views.json, an emitter standing in for the `xiox:view` event.

import fixture from '../fixtures/views.json' with { type: 'json' };
import { MockModel, type Fixture } from './mockmodel.ts';
import type { AnyIntent, SectionId, SectionView } from './types.ts';

const model = new MockModel(fixture as unknown as Fixture);
const listeners = new Set<(view: SectionView) => void>();

function emit(view: SectionView): void {
  for (const listener of listeners) listener(view);
}

export function onMockView(handler: (view: SectionView) => void): void {
  listeners.add(handler);
}

/**
 * The profile warning (`AppInfo.profileMismatch`). The page reads app info once at start, so the smoke script sets
 * it with `setMismatch` and reloads; sessionStorage carries it across the reload (and is only a convenience: without it the warning is simply absent).
 */
const MISMATCH_KEY = 'xioxMock.mismatch';
function storedMismatch(): string | null {
  try { return sessionStorage.getItem(MISMATCH_KEY); } catch { return null; }
}
let mismatch: string | null = storedMismatch();

type Args = Record<string, unknown> | undefined;
const section = (args: Args): SectionId => (args as { section: SectionId }).section;
const intent = (args: Args): AnyIntent => (args as { intent: AnyIntent }).intent;

/** Every command the page sent, in order (the smoke script reads it: what the page asked, not what it drew). */
const calls: { command: string; args: unknown }[] = [];

export async function answer(command: string, args?: Args): Promise<unknown> {
  calls.push({ command, args: structuredClone(args ?? null) });
  switch (command) {
    case 'app_info': return { ...structuredClone((fixture as unknown as Fixture).app), ...(mismatch === null ? {} : { profileMismatch: mismatch }) };
    case 'about': return structuredClone((fixture as unknown as Fixture).about);
    case 'show': return model.show(section(args));
    case 'cleanup': return model.intent('cleanup', intent(args));
    case 'registry': return model.intent('registry', intent(args));
    case 'developers': return model.intent('developers', intent(args));
    case 'developers_add_root': return model.view('developers');
    case 'save_report': return model.saveReport(section(args));
    case 'window_ready': return null;
    case 'window_chrome': return null;
    default: throw { kind: 'failed', message: `unknown command ${command}` };
  }
}

let paused = false;
setInterval(() => { if (!paused) for (const view of model.tick()) emit(view); }, 200);

(window as unknown as { __xioxMock: unknown }).__xioxMock = {
  /** The commands received so far; `clearCalls` starts the list again. */
  calls,
  clearCalls(): void { calls.length = 0; },
  /** The smoke script holds the timer while it inspects a busy state. */
  pause(on: boolean): void { paused = on; },
  /** The next app info (the next page load) carries this profile warning; `null` removes it. */
  setMismatch(text: string | null): void {
    mismatch = text;
    try {
      if (text === null) sessionStorage.removeItem(MISMATCH_KEY); else sessionStorage.setItem(MISMATCH_KEY, text);
    } catch { /* no storage: the warning only lasts until the reload */ }
  },
  /** Edits a state of the working copy (a view Rust would not send); `go` then shows it. */
  patch(target: SectionId, state: string, fields: Partial<SectionView>): void {
    model.patch(target, state, fields);
  },
  go(target: SectionId, state: string): SectionView {
    const view = model.go(target, state);
    emit(view);
    return view;
  },
};
