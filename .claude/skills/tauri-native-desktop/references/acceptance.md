# Desktop acceptance checklist

Use only the rows relevant to the supported platforms. Record each outcome as **observed**, **automated test**, **static inspection**, **to test** or **not applicable**. Do not treat the presence of code as proof of OS behavior. This checklist proposes checks; the reasons are in the topic files listed in the skill's SKILL.md, the source URLs in [sources.md](sources.md).

## Test sheet

Project/commit: … · OS/build/desktop: … · Tauri/plugins/runtime (resolved in the lockfile, compared with the minimum-version matrix in [guide.md](guide.md)): … · WebView2 Runtime/WebKit version: … · Installed package: … · Monitor/scale/text size: … · Date: …

## Windows and dialogs

- [ ] The external form is a system window and can extend past the main window's edges.
- [ ] Owner/transient/child relationship appropriate to the target; modality demonstrated with mouse and keyboard on the main window.
- [ ] Cancel, X, Escape, confirm and error have defined outcomes; the result is applied exactly once.
- [ ] Successful save: the OS window disappears from the window list, the main window is usable again and focus returns to the expected place. The form's fields disappearing alone does not count as closing; no empty host left behind.
- [ ] Save rejected or network down: window still open with readable draft and error, retry possible, no false confirmation.
- [ ] X, Alt+F4, Escape/IME, Cancel, owner close and app exit go through the appropriate close contract; late/duplicate events do not duplicate results or cleanup. Also test reopening after each path.
- [ ] With a JS close listener registered, X, Alt+F4 and the app's close command all reach the same finalizer; a code search finds no `window.close()` used to close windows.
- [ ] Creation/load failure, network loss, dialog destruction and shutdown do not leave the main window disabled.
- [ ] Reopening, nested dialogs, tray and second instance reactivate the relevant window; no orphan windows.
- [ ] With a React frontend, StrictMode's repeated mount does not create two OS windows for one dialog: a cancellable deferred open or other verified coordination avoids duplicate resources and races with destruction.
- [ ] In-page dialogs and popovers: Escape closes them, focus returns to the invoker, the background is inert while a modal is open; submenus stay open during diagonal pointer movement.
- [ ] Each control type follows one rule across the product (for example all context menus native, or all custom).

## Frame and monitors

- [ ] Single title bar, drag, double click, resize, minimize/maximize and close work; double click on a drag region respects `maximizable: false`.
- [ ] Windows: system menu, Alt+Space, Win+Z and Snap Layouts on hovering maximize verified separately; the minimum window width fits the snap zones.
- [ ] Cold start and opening of every secondary window, in light and dark theme: no white frame or frame of a color other than the theme background (record at 60 fps if needed).
- [ ] With a system material enabled: turning off transparency effects, battery saver, inactive window and high contrast leave readable, opaque content.
- [ ] macOS: traffic lights, full screen, app menu with Window and Help menus, Cmd+W/Cmd+Q and reopen from the Dock verified according to the chosen policy; `hiddenTitle`, tabbing, first-mouse click and link preview behave as decided.
- [ ] Linux: shipped compositors/sessions tested; file picker and dependencies work in the package.
- [ ] Linux Wayland (GNOME and KDE where shipped): correct icon, app name and window grouping in the dock/taskbar.
- [ ] Dialog and footer reachable at 100/150/200% scale, enlarged text and a removed monitor; sizes adapted to the work area.
- [ ] Windows text size 150% with 150% scale: no clipped content, actions reachable. Zoom to 200% works: through app commands when engine zoom is off, which is Tauri's default (`zoom_hotkeys_enabled` is `false`).
- [ ] A dialog taller or wider than the main window does not clip: long content scrolls and all actions are reachable by mouse and keyboard.

## Keyboard and accessibility

- [ ] Initial focus, Tab/Shift+Tab and return on close verified; no unintended trap; focus rings appear for keyboard use, not on mouse click.
- [ ] Useful editing and context menu present; selection and copy of the necessary data available.
- [ ] Ctrl/Cmd+A outside fields does not select the interface; content text can be selected and copied; no hand cursor on buttons and menu items (searched in the app's CSS, in component-library styles and in utility classes).
- [ ] F5, Ctrl+R, Ctrl+P, Ctrl+F, F12, Alt+Arrow, mouse back/forward buttons, middle click and Ctrl+click on links, and a file dropped outside a drop zone: none reloads, navigates or opens browser features; every disabled shortcut the product needs has an app command.
- [ ] Enter/Escape do not break IME or multiline input; with a Japanese or Chinese IME on macOS and Windows, confirming a candidate with Enter does not submit the form or message; shortcuts follow the target's conventions.
- [ ] Shortcuts work with the non-US keyboard layouts the product supports; the shortcut list shown to users matches the real bindings.
- [ ] Labels, roles, values, errors and color/icon pickers readable with the system screen reader; state not conveyed by color alone.
- [ ] Light/dark, contrast themes/forced colors, reduced motion and zoom tested separately; focus rings and separators stay visible in forced colors.
- [ ] Changing system accent and theme with the app open updates the interface without restart, on every shipped platform (if the product follows the system accent).

## Bounce and scrolling

- [ ] Explicit decision for list, panel and viewport: OS feedback kept or bounce suppressed; spring animations evaluated separately.
- [ ] Trackpad and wheel tested at the top/bottom edges and on the relevant axes: no unintended movement of the app structure or empty background.
- [ ] A nested list at its edge does not drag the underlying surface when it must be isolated; ordinary scrolling keeps working.
- [ ] `contain` and `none` tested on the shipped WebView, without inferring the outcome from the CSS alone (WKWebView short pages included).
- [ ] Web frontend: intended gestures preserved; reduced motion removes non-essential elastic animations without removing understandable feedback.
- [ ] Scrollbars follow theme and high contrast; their appearance does not shift the layout where that matters.

## Boundaries and state

- [ ] New window identified by allowed type/label/route; credentials absent from URLs.
- [ ] Negative test of privileged commands from the secondary window, checking the app command manifest and combined capabilities.
- [ ] Result correlated to the session/caller, applied through the authoritative data owner.
- [ ] Minimal/targeted events, handshake and re-read cover late listeners, reopening and errors.
- [ ] Listeners, sessions and menu resources released after repeated cycles; no duplicated notification streams.

## WebView failures

- [ ] Ending the WebView processes (on Windows `msedgewebview2.exe` in Task Manager, on macOS the app's web content process) while the app is open: a page-process exit recovers by itself (reload) without user action, up to the retry limit; a browser-process exit ends in rebuilt windows or an offered restart. On Windows this needs app code; on macOS Tauri reloads by default, or runs the app's hook. On Windows the check can be automated: see "Testing recovery on Windows without Task Manager" in [recovery.md](recovery.md).
- [ ] Dev server stopped, network offline and a link to an unreachable local address: no engine error page; the app's own recovery surface or the system browser appears instead.
- [ ] Repeated forced crashes: recovery stops after the retry limit and shows the app's error surface (on macOS only with the app's own hook: the default reload has no limit); no reload loop.
- [ ] The app-handled reload command works while the page is blank or dead; if the page can hold unsaved work, on a live page it reloads only after the page saved, and faster than the dead-page timeout.
- [ ] A recovery surface shown as a native dialog is owned by the main window and blocks it while open; answering it gives the main window back.
- [ ] After a restart triggered by recovery the new process opens its window: single-instance and every lock the app owns are free.
- [ ] A navigation the app refuses (for example `location.href = 'about:blank'` in the page) leaves the page in place and raises no recovery surface.
- [ ] A normal close and exit raise no recovery surface.
- [ ] The release package loads the embedded frontend: the page URL is the app's custom-protocol origin, not `devUrl`, with the dev server not running.

## Installed app

- [ ] Package name/icon and associations correct; second launch and deep link verified where planned.
- [ ] Linux with `enableGTKAppId` enabled: a second launch forwards its arguments to the running instance instead of failing silently.
- [ ] Window close, hide and exit are distinguishable and consistent with the product; on macOS an explicit Quit exits even when closing the last window keeps the app alive.
- [ ] Notification: click/buttons with the process running, app hidden, notification center and process terminated have distinct recorded outcomes; the non-installed development build is a separate case with its expected outcome declared.
- [ ] Taskbar/dock: badge or overlay icon, attention request and progress bar appear and clear as designed; jump list tasks reach the running instance.
- [ ] Signing/update/restart verified if the updater is in scope.
- [ ] Startup, scrolling and interaction latency and memory measured if relevant; child processes launched at startup start reliably on the installed package; DOM tests and desktop tests reported separately.
