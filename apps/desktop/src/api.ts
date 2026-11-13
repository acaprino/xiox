// The commands of src-tauri (Task 11), typed. Outside Tauri (the browser at `npm run dev`, the
// Playwright smoke test) the same calls answer from `mock.ts`, so every view can be seen and
// tested without the engine.

import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AboutView, AnyIntent, AppInfo, CmdError, SectionId, SectionView } from './types.ts';

export const inTauri = typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== 'undefined';

type Mock = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
let mock: Promise<Mock> | null = null;

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return tauriInvoke<T>(command, args);
  // The mock is a development aid: a production build leaves it out.
  if (!import.meta.env.DEV) throw { kind: 'failed', message: 'Xiox runs inside its desktop window' } satisfies CmdError;
  mock ??= import('./mock.ts').then((module) => module.answer);
  return (await mock)(command, args) as Promise<T>;
}

/** The message of whatever a command threw. */
export function errorText(error: unknown): string {
  if (error && typeof error === 'object' && 'message' in error) return String((error as CmdError).message);
  return String(error);
}

const intentCommand: Record<SectionId, string> = {
  cleanup: 'cleanup', registry: 'registry', services: 'services', developers: 'developers',
};

export const api = {
  appInfo: () => invoke<AppInfo>('app_info'),
  about: () => invoke<AboutView>('about'),
  show: (s: SectionId) => invoke<SectionView>('show', { section: s }),
  view: (s: SectionId) => invoke<SectionView>('view', { section: s }),
  /** Sends one intent to the section's own command; the answer is the section's new view. */
  intent: (s: SectionId, i: AnyIntent) => invoke<SectionView>(intentCommand[s], { intent: i }),
  /** The folder picker opens in Rust; the page never sees a path. */
  addRoot: () => invoke<SectionView>('developers_add_root'),
  saveReport: (s: SectionId) => invoke<SectionView>('save_report', { section: s }),
};

/** Views pushed by the core (`xiox:view` in Tauri, the mock's emitter outside). */
export function onView(handler: (view: SectionView) => void): void {
  if (inTauri) {
    void listen<SectionView>('xiox:view', (event) => handler(event.payload));
  } else if (import.meta.env.DEV) {
    void import('./mock.ts').then((module) => module.onMockView(handler));
  }
}
