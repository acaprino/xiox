# Keyboard, zoom and accessibility

Part of the tauri-native-desktop references: versions, scope and limits are in [guide.md](guide.md). Bracketed names are sources; their URLs are in [sources.md](sources.md).

## Zoom, text size and accessibility

WebView2 follows the Windows "Text size" setting, but by enlarging the whole page: `RasterizationScale` is "the combination of the monitor DPI scale and text scaling set by the user". There is no flag to scale only text; the only override is handling `ShouldDetectMonitorScaleChanges` and `RasterizationScale` from native code. Engine zoom is off unless the project turns it on: `zoom_hotkeys_enabled` defaults to `false`, so Ctrl/Cmd+Plus/Minus and Ctrl+wheel do not zoom, and disabling browser accelerators removes Ctrl+Plus/Minus as well. In that state, which is the default one, the app takes on the job WCAG 1.4.4 assigns to the user agent: enable `zoom_hotkeys_enabled` (not enough on Windows when browser accelerators are off) or offer Ctrl/Cmd+=, Ctrl/Cmd+- and Ctrl/Cmd+0 as app commands up to at least 200%, for example with the webview's `setZoom`/`set_zoom`, and test layouts at effective factors up to 225% combined with DPI scale, plus mixed monitors and small windows. [ICoreWebView2Controller3], [WebView2Feedback #3699], [High DPI], [WCAG Resize Text].

Prefer semantic HTML, associated labels, announced errors, selected state not conveyed by color alone and predictable focus order. Enter and Escape respect textareas and IME composition. The native frame does not fix missing names or roles inside the WebView. Verify Narrator/VoiceOver/Orca on the target; on Windows inspect UI Automation with Accessibility Insights. [Microsoft accessibility testing].

Dark theme, forced palette and reduced motion are different preferences. In `forced-colors` `box-shadow` and `text-shadow` become `none` while borders and outlines are drawn in system colors: build focus rings and separators with `outline` and `border`. `prefers-reduced-motion` lets you remove, reduce or replace non-essential animations. Verify that the target's engine picks up OS changes. [Forced colors], [Reduced motion].

CSS does not reliably report the user's accent in Tauri webviews: in Safari/WKWebView "`AccentColor` is a fixed value (blue)", Chromium exposes `AccentColor` only in installed web app contexts, which a WebView2 host is not (not rechecked on WebView2 Runtime 150 and later), and WebKitGTK has no platform-independent way to read scheme, accent and contrast (libadwaita reads the dark preference from the Settings portal). If the product uses the system accent, read it natively (Windows `UISettings.GetColorValue(UIColorType::Accent)`, macOS AppKit, Linux the portal), pass it as a CSS variable and update it when settings change. On Linux, if `prefers-color-scheme` does not follow the desktop, read `org.freedesktop.appearance.color-scheme` from the portal. [BCD color], [MDN system-color], [GNOME dark style], [GTK #6821].

For exclusive color/icon selection consider named radios with arrows and Space, following the [WAI pattern] (inside a toolbar the arrows move focus without selecting); do not force the same behavior on every grid.

On WebKit (WKWebView on macOS, presumably WebKitGTK) the Enter that confirms an IME candidate arrives after `compositionend`, with `isComposing` already false (WebKit 165004). In Enter handlers check `isComposing` or `keyCode === 229`, nested inside the Enter check; a flag of your own reset in `compositionend` is not enough. Ctrl/Cmd+Enter shortcuts need no guard. Migrating from `keyCode === 13` to `event.key === 'Enter'` without this check introduces premature submits. [Chrome IME-safe Enter], [WebKit 165004].

```js
// Illustrative.
textarea.addEventListener('keydown', e => {
  if (e.key !== 'Enter' || e.shiftKey) return;
  if (e.isComposing || e.keyCode === 229) return;
  e.preventDefault();
  form.requestSubmit();
});
```

For appearance, on Windows consider Segoe UI Variable with a Segoe UI/system fallback and verify languages and enlarged text; in HTML the font must be requested in CSS. This does not turn the controls into WinUI. Materials are covered in [frame.md](frame.md). [Microsoft typography].

## Command registry

If the product has more than a few shortcuts, proposal: define one command registry from which bindings, menu items with their accelerator, tooltips and the shortcut list all derive; use a `mod` key meaning Cmd on macOS and Ctrl elsewhere; separate global shortcuts from focus-scoped ones and do not trigger single-character keys while the user types. Show shortcuts in menus so users learn them; a command palette is a visible entry point. [Blueprint hotkeys], [Linear: Invisible details], [VS Code 1.82].
