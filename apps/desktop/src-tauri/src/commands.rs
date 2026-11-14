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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::State;
use tauri_plugin_dialog::DialogExt;
use xiox_core::notify::SectionId;
use xiox_core::view::*;

/// Where the debug window on fixtures "adds" a folder: no dialog opens, no real folder is named.
#[cfg(debug_assertions)]
const FAKE_ROOT: &str = r"D:\Work\extra";
/// What the debug window on fixtures says it saved: no file is written.
#[cfg(debug_assertions)]
const FAKE_SAVED: &str = r"Report saved to C:\Users\demo\Documents\Xiox report.txt";
const NOTHING_TO_SAVE: &str = "Nothing to save.";
const ALREADY_OPEN: &str = "Already open.";

/// A native dialog is open (the folder picker or the save dialog): a second command that would open
/// one is refused until it closes, so the page cannot stack dialogs.
static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

/// Held while a native dialog is open; dropping it (also on a panic) frees the slot.
struct DialogSlot;

impl DialogSlot {
    fn take() -> CmdResult<DialogSlot> {
        match DIALOG_OPEN.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => Ok(DialogSlot),
            Err(_) => Err(CmdError::refused(ALREADY_OPEN)),
        }
    }
}

impl Drop for DialogSlot {
    fn drop(&mut self) {
        DIALOG_OPEN.store(false, Ordering::SeqCst);
    }
}

/// The fake window answers the picker without opening one (debug builds only; the release twin is
/// `None`, so a release build has neither the fake paths nor their strings).
#[cfg(debug_assertions)]
fn fake_add_root(b: &Arc<dyn Backend>) -> Option<CmdResult<SectionView>> {
    b.is_fake().then(|| b.add_root(PathBuf::from(FAKE_ROOT)))
}

#[cfg(not(debug_assertions))]
fn fake_add_root(_: &Arc<dyn Backend>) -> Option<CmdResult<SectionView>> {
    None
}

#[cfg(debug_assertions)]
fn fake_save(b: &Arc<dyn Backend>, section: SectionId) -> Option<CmdResult<SectionView>> {
    b.is_fake().then(|| Ok(b.set_saved(section, FAKE_SAVED.to_string(), true)))
}

#[cfg(not(debug_assertions))]
fn fake_save(_: &Arc<dyn Backend>, _: SectionId) -> Option<CmdResult<SectionView>> {
    None
}

/// What the save dialog and the write came to goes on the report ONLY if the report on screen is
/// still the one that was serialized (the page may have closed it or started another job while the
/// dialog was open); otherwise the person gets the current view, without the notice.
fn apply_saved(b: &Arc<dyn Backend>, section: SectionId, serialized: &str, saved: String, ok: bool) -> SectionView {
    if b.report_text(section).as_deref() == Some(serialized) {
        b.set_saved(section, saved, ok)
    } else {
        b.view(section)
    }
}

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
        if let Some(answer) = fake_add_root(&b) {
            return answer;
        }
        let _slot = DialogSlot::take()?;
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
        if let Some(answer) = fake_save(&b, section) {
            return answer;
        }
        let _slot = DialogSlot::take()?;
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
                Ok(apply_saved(&b, section, &text, saved, ok))
            }
            None => Ok(b.view(section)),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One native dialog at a time: the second is refused until the first is closed (or dropped).
    #[test]
    fn a_second_dialog_is_refused_while_one_is_open() {
        let first = DialogSlot::take().expect("the first dialog opens");
        let e = DialogSlot::take().err().expect("the second is refused");
        assert_eq!((e.kind, e.message.as_str()), ("refused", "Already open."));
        drop(first);
        let again = DialogSlot::take().expect("it can open again once the first is gone");
        drop(again);
    }

    /// A backend that only knows its report: the text on screen and what was saved on it.
    struct Screen {
        report: std::sync::Mutex<Option<String>>,
        saved: std::sync::Mutex<Vec<(String, bool)>>,
    }

    fn blank() -> SectionView {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/views.json")).unwrap();
        serde_json::from_value(fixture["views"]["cleanup"]["idle"].clone()).unwrap()
    }

    impl Backend for Screen {
        fn app_info(&self) -> AppInfo {
            unimplemented!()
        }
        fn about(&self) -> AboutView {
            unimplemented!()
        }
        fn show(&self, _: SectionId) -> SectionView {
            unimplemented!()
        }
        fn view(&self, _: SectionId) -> SectionView {
            blank()
        }
        fn cleanup(&self, _: CleanupIntent) -> CmdResult<SectionView> {
            unimplemented!()
        }
        fn registry(&self, _: RegistryIntent) -> CmdResult<SectionView> {
            unimplemented!()
        }
        fn services(&self, _: ServicesIntent) -> CmdResult<SectionView> {
            unimplemented!()
        }
        fn developers(&self, _: DevIntent) -> CmdResult<SectionView> {
            unimplemented!()
        }
        fn add_root(&self, _: PathBuf) -> CmdResult<SectionView> {
            unimplemented!()
        }
        fn report_text(&self, _: SectionId) -> Option<String> {
            self.report.lock().unwrap().clone()
        }
        fn set_saved(&self, _: SectionId, text: String, ok: bool) -> SectionView {
            self.saved.lock().unwrap().push((text, ok));
            blank()
        }
        fn is_fake(&self) -> bool {
            false
        }
    }

    /// The outcome of a save lands on the report that was saved and on no other: if the report on
    /// screen changed while the dialog was open, the person gets the current view and no notice.
    #[test]
    fn the_saved_notice_goes_only_on_the_report_that_was_saved() {
        let screen = Arc::new(Screen { report: std::sync::Mutex::new(Some("report A".into())), saved: Default::default() });
        let b: Arc<dyn Backend> = screen.clone();
        apply_saved(&b, SectionId::Cleanup, "report A", "Report saved to x".into(), true);
        assert_eq!(*screen.saved.lock().unwrap(), vec![("Report saved to x".to_string(), true)]);
        // The report was closed and another one finished meanwhile.
        *screen.report.lock().unwrap() = Some("report B".into());
        apply_saved(&b, SectionId::Cleanup, "report A", "Report saved to y".into(), true);
        // Or closed with nothing in its place.
        *screen.report.lock().unwrap() = None;
        apply_saved(&b, SectionId::Cleanup, "report A", "Report saved to z".into(), true);
        assert_eq!(screen.saved.lock().unwrap().len(), 1, "the notice was applied once, to the report that was saved");
    }
}
