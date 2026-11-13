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

type Args = Record<string, unknown> | undefined;
const section = (args: Args): SectionId => (args as { section: SectionId }).section;
const intent = (args: Args): AnyIntent => (args as { intent: AnyIntent }).intent;

export async function answer(command: string, args?: Args): Promise<unknown> {
  switch (command) {
    case 'app_info': return structuredClone((fixture as unknown as Fixture).app);
    case 'about': return structuredClone((fixture as unknown as Fixture).about);
    case 'show': return model.show(section(args));
    case 'view': return model.view(section(args));
    case 'cleanup': return model.intent('cleanup', intent(args));
    case 'registry': return model.intent('registry', intent(args));
    case 'services': return model.intent('services', intent(args));
    case 'developers': return model.intent('developers', intent(args));
    case 'developers_add_root': return model.view('developers');
    case 'save_report': return model.view(section(args));
    default: throw { kind: 'failed', message: `unknown command ${command}` };
  }
}

setInterval(() => { for (const view of model.tick()) emit(view); }, 200);

(window as unknown as { __xioxMock: unknown }).__xioxMock = {
  go(target: SectionId, state: string): SectionView {
    const view = model.go(target, state);
    emit(view);
    return view;
  },
};
