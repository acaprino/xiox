//! What makes every webview of the app a desktop window and not a browser tab, in one place: a plugin, so a
//! window created later gets the same policy as `main` without anyone remembering to call it.
//!
//! - Navigation only to the app's own page (`allowed`): a link, a redirect or `location.href = ...` elsewhere is refused.
//! - WebView2 (`webview2`): no browser accelerators (F5, Ctrl+R, Ctrl+P, Ctrl+F, Ctrl+U, Alt+Left...; the zoom keys
//!   are the page's own commands, `src/zoom.ts`), a context menu reduced to editing (`menu_keep`), no engine error
//!   page, and recovery when a WebView2 process fails (`Failure`, `Retries`): the page reloads by itself a few
//!   times, then a native message owned by the window offers what is left (reload, restart, quit).
//!
//! WebView2's defaults this changes, as of webview2-com 0.39.1 / wry 0.57.2: `AreBrowserAcceleratorKeysEnabled`,
//! `IsBuiltInErrorPageEnabled` and the default context menu are all on, and wry registers no `ProcessFailed` handler.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::plugin::TauriPlugin;
use tauri::{Manager, RunEvent, Runtime, Url, Webview, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

/// How many automatic reloads within `RETRY_WINDOW` before the person is asked instead.
const RETRY_LIMIT: usize = 3;
const RETRY_WINDOW: Duration = Duration::from_secs(60);

/// The argument a restart passes to the new process: the id of the old one, which still holds the single
/// instance until it has exited (`main.rs` waits for it).
pub const RESTARTED_FROM: &str = "--restarted-from";

pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("xiox-desktop")
        .on_navigation(|webview, url| {
            let dev = if tauri::is_dev() { webview.app_handle().config().build.dev_url.clone() } else { None };
            allowed(url, dev.as_ref())
        })
        .on_webview_ready(|webview| {
            #[cfg(windows)]
            webview2::harden(&webview);
        })
        .on_event(|_, event| {
            // Closing ends the WebView2 processes too: that is not a failure to recover from.
            let closing = matches!(
                event,
                RunEvent::ExitRequested { .. } | RunEvent::Exit | RunEvent::WindowEvent { event: WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed, .. }
            );
            if closing {
                EXITING.store(true, Ordering::SeqCst);
            }
        })
        .build()
}

/// How long the window waits for the page's first frame (`window_ready`) before it is shown anyway: a page
/// that never signals (a script that failed to load) must not leave an invisible process behind.
const SHOW_ANYWAY: Duration = Duration::from_secs(3);

/// The window opens hidden (`visible: false` in tauri.conf.json) and is shown when the page has drawn its first
/// frame, so the first thing on screen is the page and not the empty window (tao paints it dark grey, not the
/// stage colour). Shown anyway after `SHOW_ANYWAY`.
pub fn show_when_drawn<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    let window = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_ANYWAY);
        show(&window);
    });
}

fn show<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    if !window.is_visible().unwrap_or(true) {
        let _ = window.show();
    }
}

/// The page has drawn its first frame (`src/main.ts`, after the first render): the window can be seen. Called
/// again after every reload; only the first call does anything.
#[tauri::command]
pub fn window_ready(window: tauri::WebviewWindow) {
    show(&window);
}

/// Whether the webview may go to `url`: the embedded frontend (`http://tauri.localhost` or, with
/// `useHttpsScheme`, `https://tauri.localhost`) or, in a dev build, exactly the `devUrl` origin. Scheme, host and
/// port compared whole, never by prefix (`tauri.localhost.example.com` is someone else).
pub fn allowed(url: &Url, dev: Option<&Url>) -> bool {
    let own = matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost") && url.port().is_none();
    let dev = dev.is_some_and(|dev| {
        url.scheme() == dev.scheme() && url.host_str() == dev.host_str() && url.port_or_known_default() == dev.port_or_known_default()
    });
    own || dev
}

/// The context menu entries kept: editing, in a field or on selected text. Navigation, reload, print, save,
/// share, web capture and inspect go; a menu left with nothing is not shown at all (a right click on a button or a
/// row does nothing, as in a desktop app).
const EDITING: [&str; 8] = ["undo", "redo", "cut", "copy", "paste", "pasteAndMatchStyle", "selectAll", "emoji"];

/// Which entries of a context menu stay: `items` are `(name, is_separator)` in menu order. Separators stay only
/// between two kept entries, never doubled, first or last.
pub fn menu_keep(items: &[(String, bool)]) -> Vec<bool> {
    let mut keep: Vec<bool> = items.iter().map(|(name, separator)| !separator && EDITING.contains(&name.as_str())).collect();
    let mut last_kept_entry = false;
    let mut pending: Option<usize> = None;
    for (i, (_, separator)) in items.iter().enumerate() {
        if *separator {
            if last_kept_entry && pending.is_none() {
                pending = Some(i);
            }
        } else if keep[i] {
            if let Some(at) = pending.take() {
                keep[at] = true;
            }
            last_kept_entry = true;
        }
    }
    keep
}

/// What went wrong with a webview, from WebView2's `ProcessFailed` and `NavigationCompleted`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The page's process exited: the page is blank until it is reloaded.
    PageGone,
    /// The page's process stopped answering input.
    PageHung,
    /// The engine itself (the browser process) exited: every webview of the process is closed for good.
    EngineGone,
    /// The page did not load (the WebView2 error status, operation-cancelled excluded).
    NotLoaded(i32),
}

/// A sliding window of attempts: at most `limit` within `window`.
pub struct Retries {
    times: Vec<Instant>,
    limit: usize,
    window: Duration,
}

impl Retries {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self { times: Vec::new(), limit, window }
    }

    /// Whether one more attempt is allowed at `now` (and counts it if so).
    pub fn allow(&mut self, now: Instant) -> bool {
        self.times.retain(|t| now.duration_since(*t) < self.window);
        if self.times.len() >= self.limit {
            return false;
        }
        self.times.push(now);
        true
    }
}

static EXITING: AtomicBool = AtomicBool::new(false);
/// One recovery message at a time; the engine's exit is handled once (every webview reports it).
static ASKING: AtomicBool = AtomicBool::new(false);
static ENGINE_GONE: AtomicBool = AtomicBool::new(false);

fn retries() -> &'static Mutex<Retries> {
    static RETRIES: OnceLock<Mutex<Retries>> = OnceLock::new();
    RETRIES.get_or_init(|| Mutex::new(Retries::new(RETRY_LIMIT, RETRY_WINDOW)))
}

/// Called from inside WebView2's callbacks, on the UI thread: nothing modal and no call into the webview here.
/// A reload is dispatched to the event loop; a message is shown from a thread of its own.
pub fn on_failure<R: Runtime>(webview: &Webview<R>, failure: Failure) {
    if EXITING.load(Ordering::SeqCst) {
        return;
    }
    eprintln!("xiox: webview {}: {failure:?}", webview.label());
    match failure {
        Failure::PageGone | Failure::NotLoaded(_) => {
            let again = retries().lock().unwrap_or_else(|e| e.into_inner()).allow(Instant::now());
            if again {
                let _ = webview.reload();
            } else {
                ask(webview, failure);
            }
        }
        Failure::PageHung => ask(webview, failure),
        Failure::EngineGone => {
            if !ENGINE_GONE.swap(true, Ordering::SeqCst) {
                ask(webview, failure);
            }
        }
    }
}

/// After a native dialog (folder, save, message) closes the window is active again, but WebView2 does not always hand the
/// keyboard back to the page: without this the arrows, Space and Enter do nothing until the next click. The page's own
/// `focus` handler (`src/main.ts`) then puts the focus on the list.
pub fn give_back_focus<R: Runtime>(webview: &Webview<R>) {
    let _ = webview.window().set_focus();
    let _ = webview.set_focus();
}

/// The recovery message, owned by the window (modal to it), with the two ways out the failure leaves.
fn ask<R: Runtime>(webview: &Webview<R>, failure: Failure) {
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (text, yes, no) = match failure {
        Failure::PageGone => ("The Xiox window stopped working several times in a row.", "Reload", "Quit"),
        Failure::NotLoaded(_) => ("The Xiox window could not be loaded.", "Try again", "Quit"),
        Failure::PageHung => ("The Xiox window is not responding.", "Reload", "Wait"),
        Failure::EngineGone => ("The component that draws the Xiox window has stopped. Xiox must restart to continue.", "Restart", "Quit"),
    };
    let webview = webview.clone();
    std::thread::spawn(move || {
        let window = webview.window();
        // A page that failed before its first frame left the window hidden: the message needs it on screen.
        if !window.is_visible().unwrap_or(true) {
            let _ = window.show();
        }
        let yes_pressed = webview
            .app_handle()
            .dialog()
            .message(text)
            .title("Xiox")
            .kind(MessageDialogKind::Error)
            .parent(&window)
            .buttons(MessageDialogButtons::OkCancelCustom(yes.to_string(), no.to_string()))
            .blocking_show();
        ASKING.store(false, Ordering::SeqCst);
        give_back_focus(&webview);
        let app = webview.app_handle();
        match (failure, yes_pressed) {
            (Failure::EngineGone, true) => restart(app),
            (Failure::EngineGone, false) => app.exit(0),
            (Failure::PageHung, false) => {}
            (_, true) => {
                let _ = webview.reload();
            }
            (_, false) => app.exit(0),
        }
    });
}

/// Starts this program again with the same arguments, telling the new process which one to wait for: the single
/// instance (`single_instance`) is held until this process has exited.
fn restart<R: Runtime>(app: &tauri::AppHandle<R>) {
    let Ok(exe) = std::env::current_exe() else {
        app.exit(1);
        return;
    };
    let args = restart_args(std::env::args().skip(1).collect(), std::process::id());
    if let Err(e) = std::process::Command::new(exe).args(args).spawn() {
        eprintln!("xiox: restart failed: {e}");
    }
    app.exit(0);
}

/// The arguments of the restarted process: this one's, without an earlier `--restarted-from <pid>`, plus the
/// current id.
pub fn restart_args(mut args: Vec<String>, pid: u32) -> Vec<String> {
    if let Some(at) = args.iter().position(|a| a == RESTARTED_FROM) {
        let end = (at + 2).min(args.len());
        args.drain(at..end);
    }
    args.push(RESTARTED_FROM.to_string());
    args.push(pid.to_string());
    args
}

/// The id a restart passed (`--restarted-from <pid>`), if any.
pub fn restarted_from(args: &[String]) -> Option<u32> {
    let at = args.iter().position(|a| a == RESTARTED_FROM)?;
    args.get(at + 1)?.parse().ok()
}

#[cfg(windows)]
mod webview2 {
    use super::{menu_keep, on_failure, Failure};
    use tauri::{Runtime, Webview};
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::{take_pwstr, ContextMenuRequestedEventHandler, NavigationCompletedEventHandler, ProcessFailedEventHandler};
    use windows_core::{Interface, BOOL, PWSTR};

    /// Applies the WebView2 part of the policy as soon as the webview exists. The switches take effect from the
    /// next navigation, so the first load still has them as WebView2 defaults them; the page loads next.
    pub fn harden<R: Runtime>(webview: &Webview<R>) {
        let owner = webview.clone();
        let applied = webview.with_webview(move |platform| {
            // SAFETY: COM calls on the controller WebView2 hands to this callback, on the thread it calls it on.
            if let Err(e) = unsafe { install(&platform.controller(), owner) } {
                eprintln!("xiox: webview policy not applied: {e}");
            }
        });
        if let Err(e) = applied {
            eprintln!("xiox: webview policy not applied: {e}");
        }
    }

    unsafe fn install<R: Runtime>(controller: &ICoreWebView2Controller, webview: Webview<R>) -> windows_core::Result<()> {
        unsafe {
            let core = controller.CoreWebView2()?;
            let settings = core.Settings()?;
            // Both default to TRUE. The page owns the error surface (`ask`) and the shortcuts it needs (zoom).
            settings.SetIsBuiltInErrorPageEnabled(false)?;
            settings.cast::<ICoreWebView2Settings3>()?.SetAreBrowserAcceleratorKeysEnabled(false)?;

            let mut token = 0i64;
            core.cast::<ICoreWebView2_11>()?.add_ContextMenuRequested(
                &ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
                    let Some(args) = args else { return Ok(()) };
                    trim_menu(&args)
                })),
                &mut token,
            )?;

            let failed = webview.clone();
            core.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    args.ProcessFailedKind(&mut kind)?;
                    let failure = match kind {
                        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => Some(Failure::PageGone),
                        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => Some(Failure::PageHung),
                        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => Some(Failure::EngineGone),
                        // The GPU and utility processes come back by themselves; the page has no frames.
                        _ => None,
                    };
                    if let Some(failure) = failure {
                        on_failure(&failed, failure);
                    }
                    Ok(())
                })),
                &mut token,
            )?;

            core.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut success = BOOL(1);
                    args.IsSuccess(&mut success)?;
                    if success.as_bool() {
                        return Ok(());
                    }
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    args.WebErrorStatus(&mut status)?;
                    // A navigation `allowed` refused, or one a newer navigation replaced: not a failure.
                    if status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                        on_failure(&webview, Failure::NotLoaded(status.0));
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            Ok(())
        }
    }

    /// Removes from the menu what `menu_keep` does not keep; with nothing left, no menu is shown.
    unsafe fn trim_menu(args: &ICoreWebView2ContextMenuRequestedEventArgs) -> windows_core::Result<()> {
        unsafe {
            let items = args.MenuItems()?;
            let mut count = 0u32;
            items.Count(&mut count)?;
            let mut entries = Vec::with_capacity(count as usize);
            for i in 0..count {
                let item = items.GetValueAtIndex(i)?;
                let mut name = PWSTR::null();
                item.Name(&mut name)?;
                let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
                item.Kind(&mut kind)?;
                entries.push((take_pwstr(name), kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR));
            }
            let keep = menu_keep(&entries);
            for i in (0..count).rev() {
                if !keep[i as usize] {
                    items.RemoveValueAtIndex(i)?;
                }
            }
            if !keep.contains(&true) {
                args.SetHandled(true)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn only_the_apps_own_page_is_a_destination() {
        assert!(allowed(&url("http://tauri.localhost/"), None));
        assert!(allowed(&url("https://tauri.localhost/index.html"), None));
        assert!(!allowed(&url("http://tauri.localhost.example.com/"), None), "not by prefix");
        assert!(!allowed(&url("http://tauri.localhost:8080/"), None), "not another port");
        assert!(!allowed(&url("about:blank"), None));
        assert!(!allowed(&url("https://example.com/"), None));
        assert!(!allowed(&url("file:///C:/Windows/"), None));
        // A dev build also loads its devUrl, and only that origin.
        let dev = url("http://localhost:1422");
        assert!(allowed(&url("http://localhost:1422/src/main.ts"), Some(&dev)));
        assert!(!allowed(&url("http://localhost:1423/"), Some(&dev)));
        assert!(!allowed(&url("http://localhost:1422/"), None), "a release build has no devUrl");
    }

    fn menu(items: &[&str]) -> Vec<(String, bool)> {
        items.iter().map(|name| (name.to_string(), *name == "-")).collect()
    }

    fn kept(items: &[&str]) -> Vec<String> {
        let list = menu(items);
        let keep = menu_keep(&list);
        list.into_iter().zip(keep).filter(|(_, k)| *k).map(|((name, _), _)| name).collect()
    }

    #[test]
    fn the_context_menu_keeps_editing_only() {
        // A right click on the page itself: nothing is left, so no menu.
        assert!(kept(&["back", "forward", "reload", "-", "saveAs", "print", "share", "-", "inspectElement"]).is_empty());
        // In a field: the editing commands, with the separators between them.
        assert_eq!(
            kept(&["emoji", "-", "undo", "redo", "-", "cut", "copy", "paste", "-", "selectAll", "-", "inspectElement"]),
            ["emoji", "-", "undo", "redo", "-", "cut", "copy", "paste", "-", "selectAll"]
        );
        // On selected text: copy, without a leading, doubled or trailing separator.
        assert_eq!(kept(&["-", "copy", "copyLinkToHighlight", "-", "-", "webCapture", "print", "-"]), ["copy"]);
    }

    #[test]
    fn reloads_stop_after_the_limit_and_come_back_with_time() {
        let mut retries = Retries::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(retries.allow(t0));
        assert!(retries.allow(t0 + Duration::from_secs(1)));
        assert!(retries.allow(t0 + Duration::from_secs(2)));
        assert!(!retries.allow(t0 + Duration::from_secs(3)), "the fourth within a minute is refused");
        assert!(retries.allow(t0 + Duration::from_secs(61)), "the first one has left the window");
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn a_restart_names_the_old_process_once() {
        assert_eq!(restart_args(args(&["--only-root", "D:\\w"]), 42), args(&["--only-root", "D:\\w", "--restarted-from", "42"]));
        assert_eq!(restart_args(args(&["--restarted-from", "7", "--only-root", "x"]), 42), args(&["--only-root", "x", "--restarted-from", "42"]));
        assert_eq!(restarted_from(&args(&["xiox-ui", "--restarted-from", "42"])), Some(42));
        assert_eq!(restarted_from(&args(&["xiox-ui", "--restarted-from"])), None);
        assert_eq!(restarted_from(&args(&["xiox-ui"])), None);
    }
}
