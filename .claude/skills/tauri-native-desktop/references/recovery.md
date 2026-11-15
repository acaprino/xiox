# Recovering from WebView failures

Part of the tauri-native-desktop references: versions, scope and limits are in [guide.md](guide.md). Bracketed names are sources; their URLs are in [sources.md](sources.md).

On Windows the user can land on the WebView2 built-in error page ("can't reach this page") when a navigation fails (dev URL not running, a build without the embedded frontend, a link or redirect to an unreachable local address) or when a WebView2 process dies (crash, low memory, runtime update). WebView2 does not reload by itself: after a renderer exit "the content in impacted frames … is replaced with an error page" and the app "must handle recovery"; after a browser process exit all controls are closed and "need to be recreated". If the app disabled browser accelerators (F5/Ctrl+R) the user has no way out. [Process-related events].

Proposal; items 1 to 3 go through `with_webview` in the shared bootstrap ([web-layer.md](web-layer.md)), right after the window is built. The switch applies from the next navigation: a first load that fails before it can still show the built-in page once, until the retry navigates again.

1. Disable the built-in error page with `IsBuiltInErrorPageEnabled = FALSE`. When disabled "a blank page is displayed", so the app must own the error/recovery surface: a native message or a page bundled in the app that explains the problem and offers retry. [ICoreWebView2Settings].
2. Handle `ProcessFailed` by `COREWEBVIEW2_PROCESS_FAILED_KIND`: renderer exit (`RENDER_PROCESS_EXITED`) → `Reload`; browser process exit (`BROWSER_PROCESS_EXITED`) → every webview of the app is closed for good: restart the app, or destroy and rebuild its windows; unresponsive renderer → offer reload; other kinds → log. Microsoft's sample uses the same split. Restart when the app's state lives on disk or in the native side and nothing must keep running. Rebuild the windows in the same process only when native work must survive (a recording, a transfer): a `WebviewWindow` has no call that replaces its webview, so rebuilding means destroying the window and building it again, after the environment's `BrowserProcessExited` event when the new webviews use the same user data folder (the event is reachable through `with_webview` → `environment()` cast to `ICoreWebView2Environment5`; not compiled for this guide). Add a retry limit (for example N failures within a time window, then stop and show the app's error surface) to avoid reload loops. wry 0.57.2 registers no `ProcessFailed` handler, so on Windows this recovery is entirely the app's job; recheck the resolved version. [wry 0.57.2 source], [ICoreWebView2].
3. Handle `NavigationCompleted` with `IsSuccess` false and its `WebErrorStatus`: retry app URLs or show the app surface; keep external URLs out of the webview (Tauri's `on_navigation` builder hook) and open them in the system browser. [NavigationCompletedEventArgs], [WebviewWindowBuilder], [Opener plugin].
4. Offer a reload command handled by the app (menu item or shortcut) that runs from the native side, so it works when the page is blank or dead.
5. Verify that release builds embed the frontend: the page URL must be the app's custom-protocol origin (`useHttpsScheme` decides `http://tauri.localhost` or `https://tauri.localhost` on Windows), not the dev server `devUrl`. The `custom-protocol` Cargo feature is "managed by the Tauri CLI. When enabled, Tauri assumes a production environment": a manual `cargo build` that omits it yields a dev-mode binary that loads `devUrl`. Check what the project's build scripts pass and inspect the shipped binary. [Tauri configuration], [tauri crate features].

## Windows implementation notes

Compiled and run on Windows 11 against tauri 2.12.5, wry 0.57.1, webview2-com 0.39.1 and tauri-plugin-single-instance 2.5.5; recheck against the project's lockfile.

- **Nothing modal inside the callbacks.** `ProcessFailed` and `NavigationCompleted` handlers run inside WebView2's COM callbacks on the UI thread. Do not show a dialog, run a menu or re-enter the webview there: hand the dialog, the reload or the restart to another thread (Tauri's `reload()` and window methods dispatch back to the main thread). Keep one recovery dialog open at a time and treat a browser-process exit once: several windows can report the same failure. The same holds for an IPC command that answers a reload request: reload after the command returns.
- **Cancelled is not failed.** `NavigationCompleted` reports `IsSuccess` false with `COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED` for a navigation the app refused in `on_navigation` or that a newer one replaced. Exclude that status before raising the recovery surface. Whether the engine reports every refused navigation this way was not tested. [WebView2 error status].
- **Navigation allowlist.** In `on_navigation` allow only the app's origin: `http://tauri.localhost` or `https://tauri.localhost` on Windows and, in a dev build (`tauri::is_dev()`), the `devUrl` origin. Compare scheme and host exactly, never by prefix (`tauri.localhost.example.com`). Register it on every window builder.
- **Where the reload command lives.** With a custom title bar and browser accelerators off, a dead page cannot offer its own reload. A surface that does not depend on the page is an item appended to the window's system menu (`GetSystemMenu` + `AppendMenuW`, handled as `WM_SYSCOMMAND` in a window subclass; use an id below `0xF000`, the range of Windows' own commands, with the low four bits zero, which Windows uses internally in `wParam`). It is reachable from the system menu and from the taskbar button.
- **Save before reloading a live page.** The native side cannot tell a dead page from a live one with unsaved work. Emit a reload request to the page with a token; the page saves what it can without asking (a non-interactive save, as when the OS session ends) and answers with a command carrying the token; reload on the answer, or after a timeout of a few seconds when no answer comes. A live page reloads in well under the timeout; a dead one still reloads.
- **What the native side must know without the page.** Recovery dialogs and the menu item appear exactly when the page cannot draw: if the app is localized or themed, persist the language and the theme where the native side reads them.
- **Restart after a browser-process exit.** Use `AppHandle::request_restart()`: it runs `cleanup_before_exit`. If the app uses tauri-plugin-single-instance, version 2.5.5 releases its lock there, so the restarted process becomes the instance with no extra code; an exit through `std::process::exit` skips that cleanup and needs the plugin's `destroy` first. `request_restart()` ends in Tauri's `process::restart`, which spawns the new process before the old one exits, so a lock the app itself holds until process exit (a file lock taken without waiting, a listening port) races with the new process: pass the old process id to the new one (an environment variable set before the restart) and wait for it to exit, or retry the lock. [single-instance lib.rs], [process.rs].
- **A normal exit is not a failure.** Closing the app ends its WebView2 processes too: check that no recovery surface appears then.

```rust
// Compiled against tauri 2.12.5 / webview2-com 0.39.1 / windows-core 0.62. Call it from
// `window.with_webview(move |webview| { let _ = unsafe { recover(&webview.controller(), owner) }; })`
// with `owner` a clone of the window. `on_process_failed` and `on_page_not_loaded` are the app's:
// they only record the event and spawn a thread (see "Nothing modal inside the callbacks").
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{NavigationCompletedEventHandler, ProcessFailedEventHandler};
use windows_core::BOOL;

unsafe fn recover(controller: &ICoreWebView2Controller, window: tauri::WebviewWindow) -> windows_core::Result<()> {
    unsafe {
        let core = controller.CoreWebView2()?;
        core.Settings()?.SetIsBuiltInErrorPageEnabled(false)?;
        let mut token = 0i64;
        let failed = window.clone();
        core.add_ProcessFailed(&ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            args.ProcessFailedKind(&mut kind)?;
            // BROWSER_PROCESS_EXITED, RENDER_PROCESS_EXITED, RENDER_PROCESS_UNRESPONSIVE, others
            on_process_failed(&failed, kind);
            Ok(())
        })), &mut token)?;
        core.add_NavigationCompleted(&NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut success = BOOL(1);
            args.IsSuccess(&mut success)?;
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
            args.WebErrorStatus(&mut status)?;
            if !success.as_bool() && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                on_page_not_loaded(&window, status);
            }
            Ok(())
        })), &mut token)?;
        Ok(())
    }
}
```

## Testing recovery on Windows without Task Manager

Start the built executable with a disposable user data folder (the `WEBVIEW2_USER_DATA_FOLDER` environment variable, or the app's own data-directory setting) and `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>` (test runs only), and attach to the page over the DevTools protocol (for example Playwright's `connectOverCDP`). The app's WebView2 processes are the `msedgewebview2.exe` whose command line contains that user data folder: the ones with `--type=renderer` are page processes, the one with no `--type=` is the browser process. Killing a renderer raises `RENDER_PROCESS_EXITED`; killing the browser process raises `BROWSER_PROCESS_EXITED`. Set a marker on `window` before the kill and wait for a drawn page without it to prove a reload. Native dialogs are outside the page: find them with `EnumWindows` (class `#32770`, owner the main window), assert `IsWindowEnabled` of the main window is false while one is open, and answer with `WM_COMMAND`. Kill the renderer one time more than the retry limit to see the limit, and close the app normally at the end to see that nothing appears.

Other engines. macOS/iOS: WKWebView calls [`webViewWebContentProcessDidTerminate(_:)`] on its navigation delegate, and Tauri covers it. From 2.11.0 `Builder::on_web_content_process_terminate` registers a hook that receives the webview; with no hook, tauri-runtime-wry (checked in 2.11.4 and 2.12.3) reloads the webview by itself; the source shows no retry limit, and how that behaves under repeated crashes was not tested. Register the hook when the product needs the retry limit or an app-owned surface. Linux: WebKitGTK emits [`web-process-terminated`] "when the web process terminates abnormally"; neither wry 0.57.2 nor tauri 2.12.5 connects to it, so reaching it through `with_webview` is the app's job and is not verified. [app.rs], [runtime lib.rs].
