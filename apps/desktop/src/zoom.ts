// The page's zoom is the app's job: Tauri leaves the engine's zoom off (`zoomHotkeysEnabled` is false) and the
// browser accelerators are switched off in Rust (`desktop.rs`), so Ctrl+= / Ctrl+- / Ctrl+0 and Ctrl+wheel are
// app commands, up to 200 % (WCAG 1.4.4). No DOM here, so node can test it; main.ts applies the factor to the webview.

/** The factors the commands step through: a browser's, from 67 % to 200 %. */
export const ZOOM_STEPS: readonly number[] = [0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];

export type ZoomStep = 'in' | 'out' | 'reset';

const EPSILON = 1e-6;

/** The factor after one step from `current`, which need not be one of the steps (a stored value): in goes to the next larger step, out to the next smaller one, both stopping at the ends. */
export function nextZoom(current: number, step: ZoomStep): number {
  const first = ZOOM_STEPS[0] as number;
  const last = ZOOM_STEPS[ZOOM_STEPS.length - 1] as number;
  if (step === 'reset') return 1;
  if (step === 'in') return ZOOM_STEPS.find((f) => f > current + EPSILON) ?? last;
  return [...ZOOM_STEPS].reverse().find((f) => f < current - EPSILON) ?? first;
}

/** A stored factor, if it is a usable one (anything else is 100 %). */
export function storedZoom(text: string | null): number {
  const value = text === null ? NaN : Number(text);
  const first = ZOOM_STEPS[0] as number;
  const last = ZOOM_STEPS[ZOOM_STEPS.length - 1] as number;
  return Number.isFinite(value) && value >= first - EPSILON && value <= last + EPSILON ? value : 1;
}

/**
 * How far the wheel must turn for one step, in unzoomed pixels (`wheelPixels`): one notch of a mouse wheel is about
 * 100, so every notch is a step; a touchpad pinch sends many small deltas that add up instead of jumping a step each.
 */
export const WHEEL_NOTCH = 50;

/**
 * A wheel event's `deltaY` in unzoomed pixels. WebView2 reports pixel deltas in CSS pixels, which shrink as the page
 * zooms in (one notch is 100 at 100 %, 50 at 200 %): without this the steps would grow farther apart the larger
 * the page. Line and page deltas (`deltaMode` 1 and 2) are not zoomed.
 */
export function wheelPixels(deltaY: number, deltaMode: number, zoom: number): number {
  if (deltaMode === 1) return deltaY * 40;
  if (deltaMode === 2) return deltaY * 800;
  return deltaY * zoom;
}
