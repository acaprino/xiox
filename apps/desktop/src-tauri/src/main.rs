#![cfg_attr(windows, windows_subsystem = "windows")]
//! The Xiox window (Tauri). The page draws what Rust sends and sends intents back; this crate is the
//! thin layer between the two: `Backend` (the engine behind `xiox_core::app::Core`, or the fixtures in
//! debug builds), the commands and the `xiox:view` event the view pump emits. The window itself is `imago-shell`'s
//! (`plugin`, `open_main`, `on_run_event`, `single_instance`): its material and theme, its place, Snap Layouts, the
//! navigation guard, the title and the taskbar, and WebView2 recovery. What is left here is the folder the shell keeps its
//! files in, the drop of a folder on the window (`dropped`) and the keyboard after a native dialog (`focus`).

mod backend;
mod commands;
mod dropped;
mod error;
#[cfg(debug_assertions)]
mod fake;
mod files;
mod focus;
mod real;

use backend::{Backend, BackendState};
use imago_shell::single_instance::{launch, Launch};
use imago_shell::{on_run_event, open_main, plugin, show_main, Material, Options, DARK_GROUND, LIGHT_GROUND};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, WindowEvent};
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

/// The folder the shell keeps the window theme, the glass and the window's place in: next to the Developers `settings.json`. None for
/// the fixtures window, which reads and writes nothing of the user's, and under `--only-root`, which promises that no file outside
/// its folder is written: then nothing is read or kept.
fn data_dir(fake: bool, only_root: bool) -> Option<PathBuf> {
    if fake || only_root {
        return None;
    }
    xiox_app::settings_path().parent().map(PathBuf::from)
}

/// The folder that names the one running copy. The real window and a `--only-root` run share the real folder's name (one at a time,
/// as before); the fixtures window has a name of its own, so it can run beside a real one.
fn instance_dir(fake: bool) -> PathBuf {
    if fake {
        std::env::temp_dir().join("xiox-ui-fixtures")
    } else {
        xiox_app::settings_path().parent().map_or_else(|| std::env::temp_dir().join("xiox-ui"), PathBuf::from)
    }
}

/// What the window is made of without glass: the ground of the look, the same as the page's, until the person chooses glass.
const MATERIAL: Material = Material::Opaque { dark: DARK_GROUND, light: LIGHT_GROUND };

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
    // Where the shell remembers the window theme, the glass and the window's place: not on fixtures, and not under `--only-root` (the
    // flag promises that no file outside its folder is written).
    let data_dir = data_dir(fake, only_root.is_some());
    // One window at a time: a copy started later shows the window of the one that runs, and ends.
    let raised = match launch(&instance_dir(fake), false) {
        Ok(Launch::First(instance)) => Some(instance),
        Ok(Launch::Running | Launch::Closing) => return,
        Err(error) => {
            eprintln!("xiox: single instance: {error}");
            None
        }
    };
    tauri::Builder::default()
        .plugin(plugin())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            // A folder dropped on the window is offered to Developers (`dropped`).
            if let WindowEvent::DragDrop(drag) = event
                && let Some(window) = window.app_handle().get_webview_window(window.label())
            {
                dropped::on_event(&window, drag);
            }
        })
        .setup(move |app| {
            let sink: Arc<dyn ViewSink> = Arc::new(EventSink(app.handle().clone()));
            let backend = match fake_backend(sink.clone()) {
                Some(fake) => fake,
                None => real_backend(sink, only_root.clone()),
            };
            app.manage(BackendState(backend));
            if let Some(instance) = raised {
                let handle = app.handle().clone();
                instance.on_raise(move || show_main(&handle));
            }
            // The window is the shell's: hidden until the page has drawn, where it was last time, in the theme and the glass it had.
            // Glass is the person's choice (Settings, or the palette): off at the first start.
            let options = Options {
                material: MATERIAL,
                data_dir,
                reload_label: "Reload interface".into(),
                stopped_message: "The Xiox window stopped responding. Reload it?".into(),
                lost_message: "The Xiox window could not be restored. Xiox will close.".into(),
                ..Options::for_app(app)?
            };
            open_main(app.handle(), options)?;
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
        ])
        .build(tauri::generate_context!())
        .expect("the Xiox window failed to start")
        // Without it a window that is built again, after its browser process was lost, ends the app.
        .run(on_run_event);
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

    #[test]
    fn the_data_folder_is_the_settings_folder_unless_the_run_remembers_nothing() {
        assert!(data_dir(false, false).is_some());
        assert_eq!(data_dir(false, false).as_deref(), xiox_app::settings_path().parent(), "next to settings.json, where window.json always was");
        assert_eq!(data_dir(true, false), None, "the fixtures window reads and writes nothing of the user's");
        assert_eq!(data_dir(false, true), None, "--only-root writes no file outside its folder");
        assert_eq!(data_dir(true, true), None);
    }

    #[test]
    fn the_fixtures_window_has_a_copy_of_its_own() {
        assert_ne!(instance_dir(true), instance_dir(false), "it runs beside a real window");
        assert_eq!(instance_dir(false).as_path(), xiox_app::settings_path().parent().unwrap(), "a real window and a --only-root run are one copy");
    }
}
