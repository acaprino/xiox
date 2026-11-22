// The look of the window is Imago's: Smooth (the default) or ONI MODE, in a dark or a light theme. `applyLook` writes it on
// the root (main.ts); this file keeps what Xiox stores and says what the window is asked for. The choice is kept in the
// webview's own storage like the zoom, under the keys Xiox 0.2.2 already used: a look stored then reads back as the same look.
// No DOM here, so node can test it.

import { parseLook, parseTheme, resolveTheme, windowThemeFor } from '@acaprino/imago/shell';
import type { Look, ThemeChoice, WindowTheme } from '@acaprino/imago/shell';

export type { Look, ThemeChoice, WindowTheme };
export { parseLook, parseTheme, resolveTheme, windowThemeFor };

/** Where the look is kept (localStorage). The word is `skin` for the sake of the 0.2.2 settings that hold it. */
export const LOOK_KEY = 'xiox.skin';
/** Where the theme is kept (localStorage). */
export const THEME_KEY = 'xiox.theme';

/** The look kept in `storage`: Smooth unless ONI MODE was chosen, whatever is wrong with the stored value. */
export function readLook(storage: Pick<Storage, 'getItem'> | null): Look {
  try { return parseLook(storage?.getItem(LOOK_KEY) ?? null); } catch { return parseLook(null); }
}

/** The theme kept in `storage`: Windows' unless light or dark was chosen. */
export function readTheme(storage: Pick<Storage, 'getItem'> | null): ThemeChoice {
  try { return parseTheme(storage?.getItem(THEME_KEY) ?? null); } catch { return parseTheme(null); }
}
