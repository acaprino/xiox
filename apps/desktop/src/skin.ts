// The look of the window: Smooth (the default) or ONI MODE (the look of the approved mock-up). The choice is an attribute
// on the root (`data-skin`): styles/smooth.css is scoped to it, the ONI stylesheets are what is left without it. Kept in the
// webview's own storage like the zoom. No DOM here, so node can test it.

export type Skin = 'smooth' | 'oni';

/** Where the choice is kept (localStorage). */
export const SKIN_KEY = 'xiox.skin';

/** The look a stored value stands for: ONI MODE only when it was chosen, anything else (nothing stored, a stale value) is Smooth. */
export const storedSkin = (raw: string | null): Skin => (raw === 'oni' ? 'oni' : 'smooth');

/** The other look: what the ONI MODE switch turns to. */
export const otherSkin = (skin: Skin): Skin => (skin === 'oni' ? 'smooth' : 'oni');

/** Smooth's theme: the one of Windows, or light or dark whatever Windows says. ONI MODE has none: it is always dark. */
export type Theme = 'system' | 'light' | 'dark';

/** Where the theme is kept (localStorage). */
export const THEME_KEY = 'xiox.theme';

/** The theme a stored value stands for: anything that is not light or dark follows Windows. */
export const storedTheme = (raw: string | null): Theme => (raw === 'light' || raw === 'dark' ? raw : 'system');

/** What the window is asked for (src-tauri chrome.rs `Look`): ONI MODE dark, Smooth its theme. */
export const windowTheme = (skin: Skin, theme: Theme): Theme => (skin === 'oni' ? 'dark' : theme);

/** The palette Smooth draws with (`data-theme` on the root, smooth.css): its theme, or for `system` what Windows says. */
export const effectiveTheme = (theme: Theme, systemLight: boolean): 'light' | 'dark' => (theme === 'system' ? (systemLight ? 'light' : 'dark') : theme);
