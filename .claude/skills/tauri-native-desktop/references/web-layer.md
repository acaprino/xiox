# Web layer: bootstrap, browser behaviors, chrome, scrolling

Part of the tauri-native-desktop references: versions, scope and limits are in [guide.md](guide.md). Bracketed names are sources; their URLs are in [sources.md](sources.md).

**Bootstrap for every WebView:** apply to windows created after startup the same relevant policies as the main window: theme/initial background ([frame.md](frame.md)), browser switches, navigation guard, context menu, the accelerator policy for release builds, error and failure recovery ([recovery.md](recovery.md)) and listeners. Centralize this adapter to avoid secondary windows initialized differently; verify the related permissions instead of granting a global set. This is an implementation recommendation derived from the window creation lifecycle.

## Browser behaviors to switch off

Tauri 2.12.5 exposes only part of the switches: `zoom_hotkeys_enabled` (default `false`; on Windows it sets `IsZoomControlEnabled` and `IsPinchZoomEnabled`, on macOS/Linux it injects a zoom polyfill that needs `core:webview:allow-set-webview-zoom`), `general_autofill_enabled` (2.11.0; it "does not disable password or credit card autofill"), `disable_drag_drop_handler` or the `dragDropEnabled` key (needed for HTML5 drag and drop on Windows), `browser_extensions_enabled` and `additional_browser_args`, which replaces wry's default flags `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` (re-add them if still wanted). [WebviewWindowBuilder].

The builder counterpart of the `dragDropEnabled` key is `disable_drag_drop_handler()`: the config key calls that method (checked in tauri-runtime 2.12.2). The Windows-only builder method `drag_and_drop(false)` is a different, window-level setting and does not free HTML5 drag and drop. A report that the flag "is ignored from the Rust builder" came from calling it instead. [tauri#13761], [tauri-runtime webview.rs].

wry 0.57.2 already turns off two more switches: `IsStatusBarEnabled` is always `false`, and `IsSwipeNavigationEnabled` follows a wry attribute that Tauri 2.12.5 does not expose, so it stays `false`. The remaining WebView2 switches (`AreBrowserAcceleratorKeysEnabled`, `AreDefaultContextMenusEnabled`, `IsBuiltInErrorPageEnabled`) are reachable only through `with_webview` → `controller()`, default to `TRUE` and take effect from the next navigation. `IsPasswordAutosaveEnabled` needs no code: its WebView2 default is `FALSE` and wry (checked in 0.57.1 and 0.57.2) never sets it, so there is no "save password?" prompt to switch off; touch it only if the product wants password saving. State each switch's default before writing code for it: a line that sets a default is not a fix. [ICoreWebView2Settings4]. `AreBrowserAcceleratorKeysEnabled` separates browser accelerators from copy/cut/paste and other editing commands, does not affect the `AcceleratorKeyPressed` event and also disables Ctrl+Plus/Minus, so zoom becomes the app's job. WKWebView and WebKitGTK lack these switches: use JS guards. Decide each switch per product, apply it in the shared bootstrap, keep editing and app shortcuts, and reimplement as app commands what you switch off and the product needs (find, print, zoom, reload). Disabling all context menus indiscriminately also removes useful functions: keep a native editing menu or an accessible equivalent. [CoreWebView2Settings], [WebView2 Settings3], [Settings], [wry 0.57.2 source].

```rust
// Illustrative, compile against your lockfile: check names and signatures against the
// webview2-com and windows versions it resolves (tauri 2.12 uses webview2-com 0.39, windows 0.62).
// The status bar and swipe navigation are already off in wry: no code needed for them.
// `windows::core::Interface` is the `windows_core::Interface` trait: import it from the crate you depend on.
#[cfg(windows)]
fn harden_webview2(win: &tauri::WebviewWindow) -> tauri::Result<()> {
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use windows::core::Interface;
    win.with_webview(|wv| unsafe {
        let Ok(core) = wv.controller().CoreWebView2() else { return };
        let Ok(s) = core.Settings() else { return };
        // Only if the app provides its own error/recovery surface (recovery.md):
        let _ = s.SetIsBuiltInErrorPageEnabled(false);
        if let Ok(s3) = s.cast::<ICoreWebView2Settings3>() { let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false); }
        // Password autosave is off by default: no line for it.
    })
}
```

```js
// Illustrative cross-engine guards (needed on WKWebView/WebKitGTK, useful everywhere).
for (const t of ['dragover', 'drop']) window.addEventListener(t, e => {
  if (!e.target.closest('[data-dropzone]')) { e.preventDefault(); if (t === 'dragover') e.dataTransfer.dropEffect = 'none'; }
});
window.addEventListener('mouseup', e => { if (e.button === 3 || e.button === 4) e.preventDefault(); });
document.addEventListener('auxclick', e => { if (e.target.closest('a')) e.preventDefault(); });
```

## Chrome and content

Separate the chrome from the content: the chrome is not selectable, uses the arrow cursor and is not draggable; the content (document bodies, fields, code, data to copy) stays selectable. A global `user-select: none` without an opt-in for content is a regression, not a native choice. Microsoft: "Always use the arrow cursor for clickable elements. don't use the pointing hand cursor for links or other interactive elements. Instead, use hover effects", and "Don't use focus rectangles with mouse interactions". On WKWebView unprefixed `user-select` arrives only with Safari 27.3 (in beta), so keep the `-webkit-` prefix. [Mouse interactions], [BCD user-select].

The hand cursor hides in three places: `cursor: pointer` in the app's CSS, the same rule in the stylesheets of component libraries, and utility classes in the markup such as Tailwind's `cursor-pointer`. Search all three and guard them with a static check. Links inside rendered content (a message or a document body) are content, not chrome: decide them separately; keeping the engine's hand there is a legitimate choice.

```css
/* Illustrative: adapt the opt-in selectors to the project's content. */
html { cursor: default; -webkit-user-select: none; user-select: none; }
input, textarea, [contenteditable], .selectable, pre, code {
  -webkit-user-select: text; user-select: text; cursor: auto;
}
img, a { -webkit-user-drag: none; }      /* or draggable="false" in the markup */
:focus-visible { outline: 2px solid Highlight; outline-offset: 2px; }
:focus:not(:focus-visible) { outline: none; }
```

## Bounce, rubber-banding and scroll chaining

**Scroll bounce** is the elastic feedback at the edge; **scroll chaining** hands the scroll over to an ancestor container. Both are distinct from spring animations on buttons/panels. The specification, still a 2019 First Public Working Draft, does not require an OS to have bounce: CSS can limit it where implemented, not add it automatically. [W3C css-overscroll-1], [Editor's draft].

| Value | Handoff to ancestors | Elastic feedback |
|---|---|---|
| `auto` | Default behavior | Depends on system and engine |
| `contain` | Prevented | May remain |
| `none` | Prevented | Suppressed where supported |

Apply the property to the actual scroll container: it is not inherited. Verify the shipped runtime; syntax support does not prove the result with a trackpad. WebKit support is partial: no effect on containers without scrollable overflow (WebKit 243452) and residual bounce on short pages even with `none` (WebKit 244232); MDN's trick of `contain` on an `overflow: hidden` container to block chaining does not work in WKWebView. [MDN overscroll-behavior], [BCD overscroll-behavior], [WebKit 244232].

**Proposed policy:** on macOS keep the system feedback in lists when appropriate; prevent the app's fixed structure from being dragged or revealing empty backgrounds. On Windows/Linux follow the runtime's behavior without globally simulating macOS rubber-banding. Use `contain` to isolate nested lists and `none` on the surfaces where you explicitly want to remove the bounce. These are per-surface choices, not a universal reset.

```css
/* Illustrative classes: assign them to the project's actual scroll containers. */
.isolated-list {
  overflow: auto;
  overscroll-behavior: contain;
}
.no-bounce-scroll {
  overflow: auto;
  overscroll-behavior: none;
}
```

If the problem is the viewport, consider its scrolling element instead of applying `none` only to a child. The change can affect navigation/pull-to-refresh gestures in the web frontend: verify them separately. Prefer CSS over global `wheel`/`touchmove` blocks that interfere with scrolling and latency.

For **elastic animations created by the app**, decide where they help feedback; do not make them mandatory to look native. Reduce/remove non-essential ones with `prefers-reduced-motion`. This does not guarantee control over the engine's native bounce. Do not add animation dependencies if scroll handling is enough.

**Scrollbars:** Windows 11 style overlay scrollbars are a stable WebView2 API since Runtime 125, exposed by Tauri as `scroll_bar_style(ScrollBarStyle::FluentOverlay)`, which "must be given the same value for all webviews that target the same data directory". On Windows consider it for all webviews sharing the data folder. Prefer native scrollbars themed by `color-scheme`, `scrollbar-width` and `scrollbar-color` over `::-webkit-scrollbar`; use `scrollbar-gutter: stable` where the scrollbar's appearance shifts the layout. In `forced-colors` `scrollbar-color` reverts to `auto`. [SDK 1.0.2535.41], [WebviewWindowBuilder].
