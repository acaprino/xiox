//! What the window does that is Xiox's own, beside what `imago-shell` does for every app of the suite: it is shown when the page
//! has drawn its first frame, the keyboard goes back to the page after a native dialog, and a WebView2 failure ends in a native
//! message owned by the window that offers what is left (reload, restart, quit). The navigation guard, the browser keys, the
//! context menu, Snap Layouts, the reload item and what to do about a failure (`recovery_action`) are the crate's.

use imago_shell::{
    disable_browser_accelerators, install_reload_menu, install_snap_layouts, recovery_action, trim_context_menu, watch_process_failures, ProcessFailure, Recovery,
    RecoveryAction,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, RunEvent, Runtime, Webview, WebviewWindow, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::shell::Overlays;

/// The argument a restart passes to the new process: the id of the old one, which still holds the single
/// instance until it has exited (`main.rs` waits for it).
pub const RESTARTED_FROM: &str = "--restarted-from";

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

/// The reloads of each window in the last minute, by label, and whether it is closing: kept for the life of the app, so a
/// window that fails does not use up the reloads of another. A window that is built opens with a count of its own.
#[derive(Default)]
pub struct Failures {
    recoveries: Mutex<HashMap<String, Recovery>>,
}

/// The engine itself (the browser process) is gone: every webview reports it, the person is asked once.
static ENGINE_GONE: AtomicBool = AtomicBool::new(false);
/// One recovery message at a time.
static ASKING: AtomicBool = AtomicBool::new(false);

/// Everything the main window gets right after it is built: the browser's keys off and the engine's context menu trimmed to
/// editing (in a release build only: a development build keeps reload, DevTools and the whole menu), the WebView2 failure
/// watcher, the Snap Layouts overlay of the maximize button (kept by label, a rebuilt window's replacing the old one) and the
/// "Reload interface" item of the window menu.
pub fn wire(app: &AppHandle, window: &WebviewWindow) -> tauri::Result<()> {
    if let Ok(mut recoveries) = app.state::<Failures>().recoveries.lock() {
        // A window opened anew counts from nothing, also under a label used before.
        recoveries.insert(window.label().to_owned(), Recovery::new());
    }
    if !tauri::is_dev() {
        disable_browser_accelerators(window)?;
        trim_context_menu(window)?;
    }
    let (handle, watched) = (app.clone(), window.clone());
    watch_process_failures(window, move |failure| on_failure(&handle, &watched, failure))?;
    let overlay = install_snap_layouts(window)?;
    if let Ok(mut overlays) = app.state::<Overlays>().0.lock() {
        overlays.insert(window.label().to_owned(), overlay);
    }
    let reloaded = window.clone();
    install_reload_menu(window, "Reload interface", move || {
        let _ = reloaded.reload();
    })
}

/// A WebView2 failure, on a thread of its own: the crate decides (`recovery_action`) and the window acts. A page that stopped
/// is reloaded after the delay the crate says; what is left to do after three reloads in a minute, a page that does not
/// answer and a lost engine is asked of the person.
fn on_failure(app: &AppHandle, window: &WebviewWindow, failure: ProcessFailure) {
    let action = {
        let state = app.state::<Failures>();
        let Ok(mut recoveries) = state.recoveries.lock() else { return };
        let recovery = recoveries.entry(window.label().to_owned()).or_default();
        recovery_action(failure, recovery, Instant::now(), false)
    };
    eprintln!("xiox: webview {}: {failure:?} is {action:?}", window.label());
    match action {
        RecoveryAction::Reload => {
            std::thread::sleep(failure.reload_delay());
            let _ = window.reload();
        }
        RecoveryAction::GiveUp => ask(app, window.label(), failure),
        // Xiox does not rebuild its window: it asks to restart (once, every webview reports the loss).
        RecoveryAction::Rebuild => {
            if !ENGINE_GONE.swap(true, Ordering::SeqCst) {
                ask(app, window.label(), failure);
            }
        }
        RecoveryAction::Ignore => {}
    }
}

/// WebView2 reports the processes that end with the application, or with a window, as failed: that is not a failure to recover
/// from. An exit begins, or a window is closing: the `Recovery` of every window, or of that one, is closed, and the crate
/// answers every failure after that with `Ignore`. Xiox prevents no exit and no close, and never rebuilds its window.
pub fn on_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::ExitRequested { .. } | RunEvent::Exit => close_recoveries(app, None),
        RunEvent::WindowEvent { label, event: WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed, .. } => close_recoveries(app, Some(label)),
        _ => {}
    }
}

/// Closes the count of the window `label`, or with `None` every count: of each window there is, also one that has not failed
/// yet and has no count.
fn close_recoveries(app: &AppHandle, label: Option<String>) {
    let state = app.state::<Failures>();
    let Ok(mut recoveries) = state.recoveries.lock() else { return };
    match label {
        Some(label) => recoveries.entry(label).or_default().close(),
        None => {
            for label in app.webview_windows().into_keys() {
                recoveries.entry(label).or_default();
            }
            for recovery in recoveries.values_mut() {
                recovery.close();
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
fn ask(app: &AppHandle, label: &str, failure: ProcessFailure) {
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (text, yes, no) = match failure {
        ProcessFailure::Renderer => ("The Xiox window stopped working several times in a row.", "Reload", "Quit"),
        ProcessFailure::NotLoaded(_) => ("The Xiox window could not be loaded.", "Try again", "Quit"),
        ProcessFailure::Unresponsive => ("The Xiox window is not responding.", "Reload", "Wait"),
        ProcessFailure::Browser => ("The component that draws the Xiox window has stopped. Xiox must restart to continue.", "Restart", "Quit"),
        // `recovery_action` never asks about these.
        ProcessFailure::Other => {
            ASKING.store(false, Ordering::SeqCst);
            return;
        }
    };
    let (app, label) = (app.clone(), label.to_owned());
    std::thread::spawn(move || {
        let Some(webview) = app.get_webview(&label) else {
            ASKING.store(false, Ordering::SeqCst);
            return;
        };
        let window = webview.window();
        // A page that failed before its first frame left the window hidden: the message needs it on screen.
        if !window.is_visible().unwrap_or(true) {
            let _ = window.show();
        }
        let yes_pressed = app
            .dialog()
            .message(text)
            .title("Xiox")
            .kind(MessageDialogKind::Error)
            .parent(&window)
            .buttons(MessageDialogButtons::OkCancelCustom(yes.to_string(), no.to_string()))
            .blocking_show();
        ASKING.store(false, Ordering::SeqCst);
        give_back_focus(&webview);
        match (failure, yes_pressed) {
            (ProcessFailure::Browser, true) => restart(&app),
            (ProcessFailure::Browser, false) => app.exit(0),
            (ProcessFailure::Unresponsive, false) => {}
            (_, true) => {
                // The person chose to reload: the page has a fresh minute.
                if let Ok(mut recoveries) = app.state::<Failures>().recoveries.lock()
                    && let Some(recovery) = recoveries.get_mut(&label)
                {
                    recovery.reset();
                }
                let _ = webview.reload();
            }
            (_, false) => app.exit(0),
        }
    });
}

/// Starts this program again with the same arguments, telling the new process which one to wait for: the single
/// instance (`single_instance`) is held until this process has exited.
fn restart(app: &AppHandle) {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// What `on_failure` asks of the crate for each failure, window by window: the same `Recovery` per label.
    fn action(recovery: &mut Recovery, failure: ProcessFailure, seconds: u64, t0: Instant) -> RecoveryAction {
        recovery_action(failure, recovery, t0 + Duration::from_secs(seconds), false)
    }

    #[test]
    fn a_page_is_reloaded_three_times_a_minute_and_then_the_person_is_asked() {
        let (mut recovery, t0) = (Recovery::new(), Instant::now());
        for second in 0..3 {
            assert_eq!(action(&mut recovery, ProcessFailure::Renderer, second, t0), RecoveryAction::Reload);
        }
        assert_eq!(action(&mut recovery, ProcessFailure::Renderer, 3, t0), RecoveryAction::GiveUp, "the fourth within a minute asks");
        assert_eq!(action(&mut recovery, ProcessFailure::Renderer, 61, t0), RecoveryAction::Reload, "a minute later it may reload again");
    }

    #[test]
    fn a_hung_page_and_a_lost_engine_are_asked_about_and_a_helper_process_is_ignored() {
        let (mut recovery, t0) = (Recovery::new(), Instant::now());
        assert_eq!(action(&mut recovery, ProcessFailure::Unresponsive, 0, t0), RecoveryAction::GiveUp);
        assert_eq!(action(&mut recovery, ProcessFailure::Browser, 0, t0), RecoveryAction::Rebuild, "Xiox asks to restart");
        assert_eq!(action(&mut recovery, ProcessFailure::Other, 0, t0), RecoveryAction::Ignore);
        assert_eq!(ProcessFailure::NotLoaded(-1).reload_delay(), Duration::from_secs(1), "a page that did not load waits a moment");
    }

    #[test]
    fn a_failure_while_the_window_or_the_application_closes_asks_nothing() {
        let (mut recovery, t0) = (Recovery::new(), Instant::now());
        recovery.close();
        for failure in [ProcessFailure::Renderer, ProcessFailure::Unresponsive, ProcessFailure::Browser, ProcessFailure::NotLoaded(-1)] {
            assert_eq!(action(&mut recovery, failure, 0, t0), RecoveryAction::Ignore, "{failure:?}");
        }
    }
}
