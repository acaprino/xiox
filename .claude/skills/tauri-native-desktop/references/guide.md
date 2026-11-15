# Guide: making a Tauri app as native as possible

Overview, minimum versions, proof and limits. The topic files are listed in the skill's SKILL.md. Bracketed names are sources; their URLs are in [sources.md](sources.md).

Research verified on 2026-11-14. Observed versions: tauri 2.12.5 (latest stable), tauri-runtime-wry 2.12.3, wry 0.57.2, tauri-plugin-notification 2.8.1, tauri-plugin-dialog 2.8.4 (depends on rfd 0.17). Tauri 3.0 is in alpha and not covered: every "Tauri does not have X" statement applies to the 2.x series only. This is a decision base, not code compiled on every platform. Check the project's versions and targets before implementing. Proposals are kept separate from documented contracts; snippets marked "illustrative" were not compiled and must be checked against the project's lockfile. [crates.io tauri], [crates.io wry], [tauri-runtime-wry 2.12.3 Cargo.toml].

## Minimum versions

Many useful APIs are recent. Compare the tauri version resolved in the lockfile with this matrix before proposing an API. Source: [tauri CHANGELOG].

| Feature | Minimum tauri | Note |
|---|---|---|
| `WindowConfig::default().focus` is `true` | 2.10.0 | Bug fix: earlier versions returned `false`, so windows built from the default config opened unfocused |
| `set_simple_fullscreen` on `WebviewWindow` | 2.10.0 | macOS |
| `general_autofill_enabled` | 2.11.0 | Does not disable password or credit card autofill |
| `data-tauri-drag-region="deep"` | 2.11.0 | Opt out with `="false"` |
| Bring All to Front menu item; `dbus` feature for the Linux theme | 2.11.0 | `dbus` enabled by default |
| `Builder::on_web_content_process_terminate` | 2.11.0 | macOS/iOS; with no hook the runtime reloads the webview by itself |
| Double click on a drag region respects `maximizable: false` | 2.11.3 | Earlier versions maximized anyway |
| Liquid Glass effects, `no_redirection_bitmap`, `set_fullscreen_on_monitor`, `activate_ignoring_other_apps`, icons from exe resources | 2.12.0 | Windows 7 no longer supported |
| `asset://` files loaded asynchronously | 2.12.0 | A slow path no longer blocks every window |
| macOS transparency and fullscreen without `macOSPrivateApi` | 2.12.1 | The flag "is no longer required" for them |

## Choose what native means

| Goal | Solution | Limit |
|---|---|---|
| Open/save files, pick folders, show standard messages | OS dialogs through the dialog adapter/plugin | They do not build arbitrary forms |
| Move a form outside the main window | A separate `WebviewWindow` with a frame and a relationship to the main window | Fields stay HTML/CSS |
| Have fields, pickers and layout from the native toolkit | AppKit/GTK/WinUI integration or evaluating another toolkit | Platform-specific work, not a CSS property |

The distinction follows the [dialog plugin] catalog and the [WebviewWindowBuilder] contract. Moving tooltips and dropdown menus into new windows is not needed to satisfy a request about dialogs.

For in-page surfaces prefer the browser's top-layer primitives to hand-made overlays: `<dialog>.showModal()` makes the rest of the page inert, closes on Escape and handles initial focus; `popover` covers non-modal layers. `closedby` (light dismiss) exists in Chrome/Edge 134 and Firefox 141; BCD lists it for Safari only from 27.3 (WebKit 284592), the same release as unprefixed `user-select`, so on a WKWebView below that version handle it in JS. Save and restore focus to the invoker explicitly. [MDN dialog], [dialog-closedby], [BCD dialog]. Three newer primitives replace JS that desktop-style surfaces otherwise need, with uneven engine support. `popover="hint"` is a tooltip-class popover that does not close open menus: Chrome/Edge 133, not in Safari up to 26.5. Interest invokers (`interestfor`) open a popover on hover or keyboard focus without JS: Chrome/Edge 142 only. CSS anchor positioning places a popover against its invoker without a positioning library: Chrome/Edge 125 and Safari 26; Firefox 147 enabled it in January 2026, which secondary sources count as Baseline, while a compatibility page from April 2026 still rated it limited over failures across shadow DOM. On WebView2 with a current Runtime all three are usable. On WKWebView use anchor positioning from Safari 26 and keep a JS path for tooltips and hover triggers. WebKitGTK was not checked. [popover hint], [Chrome popover hint], [interest invokers], [WebKit 297786], [Chrome anchor positioning], [anchor positioning Baseline], [Adobe anchor positioning]. Proposal: give submenus a "safe triangle" between pointer and submenu so diagonal movement does not close them, as [Linear] describes, and choose one rule per control type (native context menus, or all custom) instead of mixing both, a mix users noticed in [1Password 8 for Mac].

**Proposed order:** window behavior → keyboard/focus → OS services → accessibility → appearance → effects. The priority comes from the risk of an app that looks desktop but is hard to use; it is not an order prescribed by the framework. What the user notices in the first second and on every click (startup flash, cursor, selection, browser behaviors: [frame.md](frame.md) and [web-layer.md](web-layer.md)) usually comes before dialog refinements.

Mature hybrid products do not get the native feel by replacing HTML with OS controls: they avoid web-page tells, keep the OS where it is irreplaceable and invest in keyboard and speed. 1Password 8 put "every feasible piece" of logic in a Rust core, stopping short of the UI, yet still needed per-platform implementations of some components. A review of a Tauri v2 Mac app credits speed, "no needless animations" and a clear line between content and chrome. Speed and restraint weigh as much as widget imitation; measure them (Proving the result, below). [1Password 8, the story so far], [Tolaria review].

## Proving the result

Use [the acceptance checklist](acceptance.md), recording OS, runtime version, package type and tests. Renderer tests do not prove the behavior of the frame, OS windows or task switcher. The current WebDriver guide separates direct `tauri-driver` on Windows/Linux from `@wdio/tauri-service` with the `tauri-plugin-wdio-webdriver` plugin, which also covers macOS; the CrabNebula fork's macOS support requires a paid key; with msedgedriver the version must match the WebView2 Runtime. Recheck support in the chosen toolchain. [Tauri WebDriver], [WebView2 WebDriver].

Measure cold start, scrolling, interaction latency, cold open and reopen of the dialog, memory with multiple WebViews and release after repeated closes if they are project risks. This research does not prove any universal number of cheap windows. Native and hybrid apps can both be slow: the result needs measurements of the product. Measure WebKit rendering on the macOS/Linux targets you ship (one project moved from Tauri to Electron over WebKit rendering and sidecar startup on Windows), and treat any child process launched at startup as a reliability risk to test on the installed package. [Moving OpenCode Desktop to Electron].

## Limits of this guide

Primary-source research and architectural synthesis; no cross-platform adapter compiled or tested. Orchestration examples are proposals and snippets are illustrative, except the ones marked as compiled, which name the versions they were built against. `latest` pages and GitHub branches move; recheck them before changing a new project. No guarantee about compositors, screen readers, IME or notification activation after process exit without tests on the actual package.

Not verified: Tauri 3.0 alpha changelogs; how the default macOS reload behaves under repeated crashes; the WebKitGTK `web-process-terminated` signal through `with_webview`; whether WebView2's Window Controls Overlay became stable after 1.0.4314.35; `overscroll-behavior` and `prefers-color-scheme` in WebKitGTK 2.50+; whether `AccentColor` gives the real accent in WebView2 Runtime 150+; `popover="hint"`, interest invokers, `closedby` and anchor positioning in WebKitGTK; the portal `modal` option on real backends. Apple HIG values such as default font sizes or tooltip delays were not researched: do not state them as facts. Client- vs server-side decorations on GNOME/KDE, Flatpak and AppImage packaging and the Dock menu were not studied.
