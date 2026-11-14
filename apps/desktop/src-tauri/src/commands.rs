//! The ten commands of the contract (`apps/desktop/src/api.ts`): what the page can ask, and nothing
//! else. Each one moves its work off the window's thread (`spawn_blocking`) and answers with a
//! `SectionView` (or a `CmdError`).
//!
//! No command takes a path. The two commands that need one open the native dialog HERE, in Rust,
//! and the page only learns what came of it. The page has no permission of the dialog plugin.

use crate::backend::{Backend, BackendState};
use crate::error::{CmdError, CmdResult};
use crate::files;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tauri_plugin_dialog::DialogExt;
use xiox_core::notify::SectionId;
use xiox_core::view::*;

/// Where the debug window on fixtures "adds" a folder: no dialog opens, no real folder is named.
const FAKE_ROOT: &str = r"D:\Work\extra";
/// What the debug window on fixtures says it saved: no file is written.
const FAKE_SAVED: &str = r"Report saved to C:\Users\demo\Documents\Xiox report.txt";
const NOTHING_TO_SAVE: &str = "Nothing to save.";

/// Run a blocking call off the UI thread.
pub async fn blocking<T, F>(work: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| CmdError::failed(format!("the worker stopped: {error}")))?
}

fn backend(state: &State<'_, BackendState>) -> Arc<dyn Backend> {
    state.0.clone()
}

#[tauri::command]
pub async fn app_info(state: State<'_, BackendState>) -> CmdResult<AppInfo> {
    let b = backend(&state);
    blocking(move || Ok(b.app_info())).await
}

#[tauri::command]
pub async fn about(state: State<'_, BackendState>) -> CmdResult<AboutView> {
    let b = backend(&state);
    blocking(move || Ok(b.about())).await
}

#[tauri::command]
pub async fn show(state: State<'_, BackendState>, section: SectionId) -> CmdResult<SectionView> {
    let b = backend(&state);
    blocking(move || Ok(b.show(section))).await
}

#[tauri::command]
pub async fn view(state: State<'_, BackendState>, section: SectionId) -> CmdResult<SectionView> {
    let b = backend(&state);
    blocking(move || Ok(b.view(section))).await
}

#[tauri::command]
pub async fn cleanup(state: State<'_, BackendState>, intent: CleanupIntent) -> CmdResult<SectionView> {
    let b = backend(&state);
    blocking(move || b.cleanup(intent)).await
}

#[tauri::command]
pub async fn registry(state: State<'_, BackendState>, intent: RegistryIntent) -> CmdResult<SectionView> {
    let b = backend(&state);
    blocking(move || b.registry(intent)).await
}

#[tauri::command]
pub async fn services(state: State<'_, BackendState>, intent: ServicesIntent) -> CmdResult<SectionView> {
    let b = backend(&state);
    blocking(move || b.services(intent)).await
}

#[tauri::command]
pub async fn developers(state: State<'_, BackendState>, intent: DevIntent) -> CmdResult<SectionView> {
    let b = backend(&state);
    blocking(move || b.developers(intent)).await
}

/// «Add folder» of Developers: the folder picker opens here and the page never names a path.
/// Cancelling the picker changes nothing.
#[tauri::command]
pub async fn developers_add_root(app: tauri::AppHandle, state: State<'_, BackendState>) -> CmdResult<SectionView> {
    let b = backend(&state);
    blocking(move || {
        if b.is_fake() {
            return b.add_root(PathBuf::from(FAKE_ROOT));
        }
        let picked = app.dialog().file().set_title("Folder to analyze").blocking_pick_folder();
        match picked.and_then(|path| path.into_path().ok()) {
            Some(dir) => b.add_root(dir),
            None => Ok(b.view(SectionId::Developers)),
        }
    })
    .await
}

/// «Save as text…»: the save dialog opens here, the report text comes from Rust and so does the
/// outcome the report shows. Cancelling the dialog changes nothing; a section with no report has
/// nothing to save.
#[tauri::command]
pub async fn save_report(app: tauri::AppHandle, state: State<'_, BackendState>, section: SectionId) -> CmdResult<SectionView> {
    let b = backend(&state);
    blocking(move || {
        let Some(text) = b.report_text(section) else {
            return Err(CmdError::refused(NOTHING_TO_SAVE));
        };
        if b.is_fake() {
            return Ok(b.set_saved(section, FAKE_SAVED.to_string(), true));
        }
        let picked = app
            .dialog()
            .file()
            .set_title("Save the report")
            .set_file_name("Xiox report.txt")
            .add_filter("Text", &["txt"])
            .blocking_save_file();
        match picked.and_then(|path| path.into_path().ok()) {
            Some(path) => {
                let (saved, ok) = files::write_report(&path, &text);
                Ok(b.set_saved(section, saved, ok))
            }
            None => Ok(b.view(section)),
        }
    })
    .await
}
