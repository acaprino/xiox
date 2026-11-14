#![cfg_attr(windows, windows_subsystem = "windows")]
//! The Xiox window (Tauri). The page draws what Rust sends and sends intents back; this crate is the
//! thin layer between the two: `Backend` (the engine behind `xiox_core::app::Core`, or the fixtures in
//! debug builds), the ten commands and the `xiox:view` event the view pump emits.

mod backend;
mod commands;
mod error;
#[cfg(debug_assertions)]
mod fake;
mod files;
mod real;

use backend::{Backend, BackendState};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use xiox_core::app::{Core, CoreOptions};
use xiox_core::pump::{pump_channel, spawn_pump, ViewSink};
use xiox_core::view::SectionView;

/// The event the page listens to (`apps/desktop/src/api.ts`).
const VIEW_EVENT: &str = "xiox:view";

/// Views reach the page as the `xiox:view` event. (A newtype: `ViewSink` and `AppHandle` are both
/// foreign to this crate.)
struct EventSink(tauri::AppHandle);

impl ViewSink for EventSink {
    fn send(&self, view: &SectionView) {
        let _ = self.0.emit(VIEW_EVENT, view);
    }
}

/// `--only-root <dir>`: Developers looks only in that folder and keeps its settings in memory.
fn only_root(args: &[String]) -> Option<PathBuf> {
    let at = args.iter().position(|a| a == "--only-root")?;
    args.get(at + 1).map(PathBuf::from)
}

/// The window on fixtures: debug builds only, and only when asked. A release build has no such
/// code at all.
#[cfg(debug_assertions)]
fn fake_backend(sink: Arc<dyn ViewSink>) -> Option<Arc<dyn Backend>> {
    if std::env::var("XIOX_UI_FAKE").as_deref() != Ok("1") {
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
    let core = Core::new(
        CoreOptions {
            version: env!("CARGO_PKG_VERSION").to_string(),
            admin: xiox_core::sys::is_admin(),
            only_root,
            load_rules: true,
            update_rules: xiox_core::rules::should_update(false),
        },
        Arc::new(notifier),
    );
    let core = Arc::new(Mutex::new(core));
    spawn_pump(core.clone(), rx, sink);
    Arc::new(real::RealBackend::new(core))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let sink: Arc<dyn ViewSink> = Arc::new(EventSink(app.handle().clone()));
            let backend = match fake_backend(sink.clone()) {
                Some(fake) => fake,
                None => real_backend(sink, only_root(&args)),
            };
            app.manage(BackendState(backend));
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::about,
            commands::show,
            commands::view,
            commands::cleanup,
            commands::registry,
            commands::services,
            commands::developers,
            commands::developers_add_root,
            commands::save_report,
        ])
        .run(tauri::generate_context!())
        .expect("the Xiox window failed to start");
}
