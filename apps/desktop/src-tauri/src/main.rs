#![cfg_attr(windows, windows_subsystem = "windows")]
//! The Xiox window (Tauri). The page draws what Rust sends and sends intents back; this crate is the
//! thin layer between the two: `Backend` (the engine behind `xiox_core::app::Core`, or the fixtures in
//! debug builds), the ten commands and the `xiox:view` event the view pump emits. Around it: one
//! instance at a time (`single_instance`), and the window itself, which is `imago-shell`'s: built from the glass config with
//! the navigation guard (`build_main`), placed where it was (`shell::place`), its theme, Snap Layouts, context menu and
//! WebView2 recovery from the crate (`shell`, `desktop`).

mod backend;
mod chrome;
mod commands;
mod desktop;
mod dropped;
mod error;
#[cfg(debug_assertions)]
mod fake;
mod files;
mod real;
mod shell;
mod single_instance;

use backend::{Backend, BackendState};
use imago_shell::{glass_window_config, guard_navigation_with_scheme, load_state, save_state, saved_theme, state_to_save, WindowState};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use xiox_core::app::{Core, CoreOptions};
use xiox_core::pump::{pump_channel, spawn_pump, ViewSink};
use xiox_core::view::SectionView;

/// The event the page listens to (`apps/desktop/src/api.ts`).
pub(crate) const VIEW_EVENT: &str = "xiox:view";

/// Views reach the page as the `xiox:view` event. (A newtype: `ViewSink` and `AppHandle` are both
/// foreign to this crate.)
struct EventSink(tauri::AppHandle);

impl ViewSink for EventSink {
    fn send(&self, view: &SectionView) {
        let _ = self.0.emit(VIEW_EVENT, view);
    }
}

/// `--only-root <dir>`: Developers looks only in that folder and keeps its settings in memory.
/// Fails closed: a flag without a usable value (last argument, empty, or another flag) is an error, never
/// a silent fall back to the real profile.
fn only_root(args: &[String]) -> Result<Option<PathBuf>, String> {
    let Some(at) = args.iter().position(|a| a == "--only-root") else {
        return Ok(None);
    };
    match args.get(at + 1) {
        Some(dir) if !dir.is_empty() && !dir.starts_with("--") => Ok(Some(PathBuf::from(dir))),
        Some(other) => Err(format!("--only-root needs a folder, but is followed by {other:?}")),
        None => Err("--only-root needs a folder, but it is the last argument".to_string()),
    }
}

/// Whether the engine loads the Winapp2 rules and whether it may download them. A Developers-only run
/// (`--only-root`) does neither: no rules cache is read, no download starts, no file outside the folder is written.
fn rules_policy(only_root: Option<&PathBuf>, update_due: impl FnOnce() -> bool) -> (bool, bool) {
    match only_root {
        Some(_) => (false, false),
        None => (true, update_due()),
    }
}

/// Whether this run is the window on fixtures: debug builds only, and only when asked.
fn fake_requested() -> bool {
    cfg!(debug_assertions) && std::env::var("XIOX_UI_FAKE").as_deref() == Ok("1")
}

/// The window on fixtures. A release build has no such code at all.
#[cfg(debug_assertions)]
fn fake_backend(sink: Arc<dyn ViewSink>) -> Option<Arc<dyn Backend>> {
    if !fake_requested() {
        return None;
    }
    match fake::FakeBackend::start(sink) {
        Ok(fake) => Some(fake),
        Err(e) => panic!("the fixtures do not load: {e}"),
    }
}

#[cfg(not(debug_assertions))]
fn fake_backend(_sink: Arc<dyn ViewSink>) -> Option<Arc<dyn Backend>> {
    None
}

/// The engine: leftovers of an interrupted wipe go first (never on a fake tree), then the core, its
/// rules thread and the view pump.
fn real_backend(sink: Arc<dyn ViewSink>, only_root: Option<PathBuf>) -> Arc<dyn Backend> {
    if only_root.is_none() {
        xiox_core::wipe::remove_leftovers_in_background();
    }
    let (notifier, rx) = pump_channel();
    let (load_rules, update_rules) = rules_policy(only_root.as_ref(), || xiox_core::rules::should_update(false));
    let core = Core::new(
        CoreOptions {
            version: env!("CARGO_PKG_VERSION").to_string(),
            admin: xiox_core::sys::is_admin(),
            only_root,
            load_rules,
            update_rules,
        },
        Arc::new(notifier),
    );
    let core = Arc::new(Mutex::new(core));
    spawn_pump(core.clone(), rx, sink);
    Arc::new(real::RealBackend::new(core))
}

/// The title bar and borders around the page: the window's outer size minus its inner size.
fn frame_of(window: &WebviewWindow) -> (u32, u32) {
    match (window.outer_size(), window.inner_size()) {
        (Ok(outer), Ok(inner)) => (outer.width.saturating_sub(inner.width), outer.height.saturating_sub(inner.height)),
        _ => (0, 0),
    }
}

/// The window as it is now, measured (only meaningful when it is neither maximized nor minimized).
fn measure(window: &WebviewWindow) -> Option<WindowState> {
    let (position, size) = (window.outer_position().ok()?, window.inner_size().ok()?);
    Some(WindowState { x: position.x, y: position.y, width: size.width, height: size.height, maximized: false })
}

/// What to remember of the window: see `imago_shell::state_to_save`.
fn current_state(window: &WebviewWindow, normal: Option<WindowState>) -> Option<WindowState> {
    let maximized = window.is_maximized().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    state_to_save(maximized, minimized, normal, || measure(window))
}

/// Puts the window where it opens, before it is shown: the place saved when there is one, else a first-launch size inside the
/// work area, centred (`shell::place`). Either way the window is inside the work area it opens on.
fn place_window(window: &WebviewWindow, saved: Option<WindowState>) {
    let size = window.inner_size().map_or((0, 0), |size| (size.width, size.height));
    let scale = window.scale_factor().unwrap_or(1.0);
    let placed = shell::place(saved, size, &shell::screens(window), scale, frame_of(window));
    let state = placed.state;
    let _ = window.set_size(PhysicalSize::new(state.width, state.height));
    if placed.centred {
        let _ = window.center();
    } else {
        let _ = window.set_position(PhysicalPosition::new(state.x, state.y));
    }
    if state.maximized {
        let _ = window.maximize();
    }
}

/// Builds the main window, hidden: the glass config of the app's own (`tauri.conf.json` has `"create": false`) with the theme
/// the window had last time, so the first frame has the right acrylic, and the navigation guard on the builder (in a
/// development build also the dev server's origin). Then the native wiring (`desktop::wire`).
fn build_main(app: &AppHandle, data_dir: Option<&Path>) -> tauri::Result<WebviewWindow> {
    let base = app.config().app.windows.iter().find(|window| window.label == "main").cloned().ok_or(tauri::Error::WindowNotFound)?;
    let config = glass_window_config(base, data_dir.and_then(saved_theme));
    let dev_origin = if tauri::is_dev() { app.config().build.dev_url.as_ref().map(|url| url.origin().ascii_serialization()) } else { None };
    let origins: Vec<&str> = dev_origin.iter().map(String::as_str).collect();
    let window = guard_navigation_with_scheme(WebviewWindowBuilder::from_config(app, &config)?, &origins, config.use_https_scheme).build()?;
    desktop::wire(app, &window)?;
    Ok(window)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Before anything is taken or started: a bad flag must not end up on the real profile.
    let only_root = match only_root(&args) {
        Ok(dir) => dir,
        Err(message) => {
            eprintln!("xiox: {message}");
            std::process::exit(2);
        }
    };
    let fake = fake_requested();
    // Where the shell remembers the window theme and the window's place: not on fixtures, and not under `--only-root` (the flag
    // promises that no file outside its folder is written).
    let data_dir = shell::data_dir(fake, only_root.is_some());
    // A restart after the engine failed (`desktop`): the old process still holds the instance until it exits.
    if let Some(pid) = desktop::restarted_from(&args) {
        single_instance::wait_for_exit(pid, std::time::Duration::from_secs(10));
    }
    // One window at a time (the fixtures window has its own name, so it can run beside a real one).
    let Some(_instance) = single_instance::acquire(if fake { r"Local\XioxUiFake" } else { r"Local\XioxUi" }) else {
        single_instance::focus_existing(chrome::APP_NAME);
        return;
    };
    // The last normal (neither maximized nor minimized) state, kept as the window moves.
    let normal: Arc<Mutex<Option<WindowState>>> = Arc::new(Mutex::new(None));
    let tracked = normal.clone();
    let remembered = data_dir.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(move |window, event| {
            if let WindowEvent::DragDrop(drag) = event {
                if let Some(window) = window.app_handle().get_webview_window(window.label()) {
                    dropped::on_event(&window, drag);
                }
                return;
            }
            let Some(dir) = remembered.as_deref() else { return };
            let Some(window) = window.app_handle().get_webview_window(window.label()) else { return };
            match event {
                WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                    if !window.is_maximized().unwrap_or(true)
                        && !window.is_minimized().unwrap_or(true)
                        && let Some(state) = measure(&window)
                    {
                        *tracked.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
                    }
                }
                WindowEvent::CloseRequested { .. } => {
                    let last = *tracked.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(state) = current_state(&window, last) {
                        // Failing to remember the window must never keep it from closing.
                        let _ = save_state(dir, &state);
                    }
                }
                _ => {}
            }
        })
        .setup(move |app| {
            let sink: Arc<dyn ViewSink> = Arc::new(EventSink(app.handle().clone()));
            let backend = match fake_backend(sink.clone()) {
                Some(fake) => fake,
                None => real_backend(sink, only_root.clone()),
            };
            app.manage(BackendState(backend));
            // The state the wiring and the two commands of the shell read, before the window exists.
            app.manage(shell::DataDir(data_dir.clone()));
            app.manage(shell::Overlays::default());
            app.manage(desktop::Failures::default());
            let window = build_main(app.handle(), data_dir.as_deref())?;
            // The fixtures window opens where its config says; every other run is placed: where it was last closed, or by the
            // first-launch size (`--only-root` reads no saved place: that would reach the real profile).
            if !fake {
                place_window(&window, data_dir.as_deref().and_then(load_state));
            }
            // Hidden until the page has drawn (`desktop::window_ready`), so the first frame is the page.
            desktop::show_when_drawn(&window);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::about,
            commands::show,
            commands::cleanup,
            commands::registry,
            commands::developers,
            commands::developers_add_root,
            commands::save_report,
            desktop::window_ready,
            chrome::window_chrome,
            shell::set_window_theme,
            shell::window_caption_maximize,
        ])
        .build(tauri::generate_context!())
        .expect("the Xiox window failed to start")
        .run(desktop::on_run_event);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn only_root_reads_its_folder() {
        assert_eq!(only_root(&args(&["xiox"])), Ok(None));
        assert_eq!(only_root(&args(&["xiox", "--only-root", r"D:\Work"])), Ok(Some(PathBuf::from(r"D:\Work"))));
        assert_eq!(only_root(&args(&["xiox", "--only-root", "dir", "--other"])), Ok(Some(PathBuf::from("dir"))));
    }

    #[test]
    fn only_root_without_a_folder_is_an_error_not_the_real_profile() {
        let last = only_root(&args(&["xiox", "--only-root"])).unwrap_err();
        assert!(last.contains("--only-root") && last.contains("last argument"), "{last}");
        let flag = only_root(&args(&["xiox", "--only-root", "--screenshot"])).unwrap_err();
        assert!(flag.contains("--only-root"), "{flag}");
        assert!(only_root(&args(&["xiox", "--only-root", ""])).is_err());
    }

    #[test]
    fn a_developers_only_run_loads_and_downloads_no_rules() {
        let dir = PathBuf::from("dir");
        assert_eq!(rules_policy(Some(&dir), || panic!("the update check must not even run")), (false, false));
        // Without the flag the rules load as before; whether they update is the rules' own decision.
        assert_eq!(rules_policy(None, || true), (true, true));
        assert_eq!(rules_policy(None, || false), (true, false));
    }
}
