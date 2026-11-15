---
name: tauri-native-desktop
description: "Use when working on the desktop side of a Tauri app: adding or converting windows and dialogs, title bar, menus, tray, notifications or shortcuts, or when the app behaves like a web page: white flash at startup, selectable interface text, hand cursors, browser shortcuts or context menu, the WebView error page, missing Snap Layouts, focus, modality, zoom or accessibility defects on Windows, macOS or Linux. Not for web-only restyling or mobile apps."
---

# Tauri Native Desktop

Act as a desktop integration engineer. Optimize the observable behavior on the chosen operating system while preserving the project's architecture and permissions. Visual resemblance does not replace correct focus, keyboard, accessibility and lifecycle.

## Identify the required level

Always distinguish:

- **Native OS control**: file picker, message box, menu, tray; use system APIs, directly or through a verified plugin.
- **Native window with web content**: a separate Tauri window managed by the window manager; the form inside stays HTML/CSS.
- **Fully native interface**: AppKit, WinUI/Win32 or GTK widgets; requires a platform-specific integration or a different toolkit choice. Tauri does not turn HTML components into these controls.

For forms outside the main window the second level is often appropriate. If native widgets are also needed, state the cost and verify the integration before promising it.

## Procedure

1. **Gather the context.** Read local instructions, manifests and lockfiles. Identify shipped platforms, Tauri/plugin/WebView versions, windows, any shared web frontend, authentication, application state and ACLs. Read [guide.md](references/guide.md) and compare the resolved tauri version with its minimum-version matrix before proposing an API; Tauri 3.0 alpha is not covered. If the project already holds an audit or a native-rules document from an earlier pass, take it as the baseline: check each reference in scope against it and report the differences. Ask only what changes the solution and cannot be derived from the project.
2. **Classify the surfaces.** Inventory dialogs, panels, menus and notices. For each one choose OS control, Tauri window or in-page surface. Record the rationale and the fallback. A request for external dialogs does not automatically turn tooltips and dropdown menus into windows.
3. **Define the desktop contract.** Read the reference files for the surfaces in scope (table below) and the matching sections of [the checklist](references/acceptance.md). Define owner, modality, sizing, focus, closing, result and authoritative state. Verify against the actual versions: `parent`, `owner`, `transient_for`, sheets and modality are not synonyms.
4. **Deliver the authorized scope.** In an audit, deliver evidence and priorities. When changes are requested, implement verifiable increments that reuse the existing domain and transport. Define the least privileges for new windows and test the caller. Keep the browser path when the product requires it. Before writing code that sets an engine switch, state its default in the resolved versions: do not "fix" a default. Where a rule can be checked without running the app (every window builder paired with the shared bootstrap, no hand cursor, every Enter handler guarded), leave that check in the project's own test suite so the rule survives later edits.
5. **Verify in the real app.** Run the relevant automated checks and tests in the installed app on the declared platforms. Distinguish inspected code, build, automated tests and observed behavior. Report the tests that still need a different environment.

## References

Read the files the work in scope needs, not all of them. Each holds the reasons, the version facts and the illustrative code for its topic.

| Work in scope | File |
|---|---|
| Versions covered, minimum-version matrix, choosing the level, proving the result, limits | [guide.md](references/guide.md) |
| Title bar, decorations, Snap Layouts, startup flash, drag regions, Mica/vibrancy/Liquid Glass | [frame.md](references/frame.md) |
| A form or dialog in its own OS window: owner/parent, modality, close lifecycle, React StrictMode | [dialogs.md](references/dialogs.md) |
| What a new window may call and read: capabilities, credentials, events, relay or broker | [boundaries.md](references/boundaries.md) |
| Bootstrap of every WebView, WebView2 switches, selection and cursors, bounce, scrollbars | [web-layer.md](references/web-layer.md) |
| Zoom, text size, screen readers, forced colors, system accent, IME, command registry | [keyboard-accessibility.md](references/keyboard-accessibility.md) |
| Menu, tray, single instance, deep link, badge, progress, exit and reopen, notifications, Linux file dialogs | [os-integration.md](references/os-integration.md) |
| Engine error page, WebView process crash, release build that loads the dev URL | [recovery.md](references/recovery.md) |
| Acceptance checks to run and record | [acceptance.md](references/acceptance.md) |
| URL of a source named in brackets in the files above | [sources.md](references/sources.md) |

## Key decisions

- **Frame.** Prefer the OS frame when native behavior is the priority. On Windows 11, `decorations: false` with HTML buttons loses Snap Layouts on hovering maximize unless dedicated Win32 code is added: keep the system decorations unless a custom bar is a requirement; if it is, budget that work and verify Win+Z and the other system features it replaces.
- **First frame.** Every window, main and secondary, opens on the theme background: no white flash.
- **Chrome and content.** Chrome is not selectable, not draggable and uses the arrow cursor; content (documents, fields, code, data to copy) stays selectable.
- **Modality.** A modal dialog gives the main window back on confirm, cancel, X, error and destruction. `always_on_top` is not modality.
- **Lifecycle.** An OS window brings its whole lifecycle: opening, readiness, confirm/save, cancel, Escape, X/Alt+F4, owner close, error and shutdown. A successful save closes the OS window, not just the form inside it: an empty window is a defect. A failed save keeps the draft, the message and the retry. Verify release of modality, focus return and single cleanup. One close decider per window (Rust or JS); `window.close()`, `close()`, `destroy()` and unmounting the form's component are different paths.
- **Sizing.** An in-page form cannot extend past the main window's frame: one that must needs an external surface. With small windows, enlarged text and long content the body scrolls and the actions stay reachable.
- **State and credentials.** Another WebView is a separate JavaScript context. Use the service/host that is already authoritative, a reliable initial read and targeted invalidation events; no second backend, no secrets in URLs. If only main has access to credentials, define the secondary window's authenticated path before implementing it.
- **Bootstrap.** Every new WebView gets the relevant desktop bootstrap, not only main. An external window need not block the main one: a Settings window can be non-modal and reusable.
- **Browser commands and zoom.** Remove irrelevant browser commands; keep editing, useful selection and IME; replace every intercepted shortcut the product needs with an app command. Engine zoom is off by default in Tauri, so accessible zoom is the app's job. With more than a few shortcuts, derive bindings, menus and the shortcut list from one command registry.
- **Failures.** Never leave the user on the engine's error page: an app-owned recovery surface, recovery from WebView process failures with a retry limit, and an app-handled reload.
- **Bounce.** Decide per surface and platform, keeping scrolling and motion preferences; overscroll/rubber-banding and elastic animations are separate choices.
- **Optional features.** Mica/vibrancy, tray, autostart, global shortcuts and updater are product choices, not a mandatory package for nativeness. A material needs a transparent window: the HTML paints opaque colors when the system turns the material off.
- **Speed and restraint** weigh as much as widget imitation: no decorative animations, quiet chrome, local interactions without spinners; measure startup, scrolling and latency on the shipped platforms.

## Delivery

Show the achieved behavior or the top-priority defect first. Include files and lines, platform/version, checks and limits. For an audit use `surface → current state → solution → acceptance test`. One concrete final action if work remains for the user.

The references reflect research done on 2026-11-14 (tauri 2.12.5). Recheck the needed APIs for every new project, especially modality, notifications and permissions. The skill provides expertise: on its own it does not authorize changes, installs or releases.
