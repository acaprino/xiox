//! A folder dragged onto the window. Tauri's drag and drop stays on (`dragDropEnabled` is not turned off in
//! `tauri.conf.json`): the drop reaches Rust as a window event with the paths, and the page is never navigated to a
//! dropped file. A dropped FOLDER is offered to the Developers section's «Add folder» flow (`Backend::add_root`), after the
//! person has said yes in a message owned by the window: the path comes from the system's drop event, never from the
//! page, and nothing is added without the answer. Anything else dropped gets a short message saying what is taken.

use crate::backend::BackendState;
use crate::commands::DialogSlot;
use crate::desktop;
use std::path::{Path, PathBuf};
use tauri::{DragDropEvent, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

/// The folder to offer: the first dropped path that is a folder.
pub fn folder_of(paths: &[PathBuf]) -> Option<&Path> {
    paths.iter().map(PathBuf::as_path).find(|path| path.is_dir())
}

/// The question the person answers before a dropped folder is added.
pub fn question(dir: &Path) -> String {
    format!("Add “{}” to the folders Developers analyzes?", dir.display())
}

/// What is said when nothing dropped is a folder.
const ONLY_FOLDERS: &str = "Only a folder can be dropped here: it is added to the folders Developers analyzes.";

/// The window event of a drop (the other drag events, hover and leave, need nothing): offers the folder on its own
/// thread, because the message blocks until it is answered.
pub fn on_event(window: &WebviewWindow, event: &DragDropEvent) {
    if let DragDropEvent::Drop { paths, .. } = event {
        let (window, paths) = (window.clone(), paths.clone());
        std::thread::spawn(move || offer(&window, &paths));
    }
}

fn offer(window: &WebviewWindow, paths: &[PathBuf]) {
    // One native dialog at a time: a drop while one is open is ignored, as a second click would be.
    let Ok(slot) = DialogSlot::take() else { return };
    let dir = folder_of(paths);
    let answer = match dir {
        Some(dir) => ask(window, &question(dir), MessageDialogKind::Info, MessageDialogButtons::OkCancelCustom("Add".into(), "Cancel".into())),
        None => ask(window, ONLY_FOLDERS, MessageDialogKind::Info, MessageDialogButtons::Ok),
    };
    drop(slot);
    desktop::give_back_focus(window.as_ref());
    let Some(dir) = dir.filter(|_| answer) else { return };
    let backend = window.state::<BackendState>();
    match backend.0.add_root(dir.to_path_buf()) {
        Ok(view) => {
            let _ = window.app_handle().emit(crate::VIEW_EVENT, &view);
        }
        Err(refused) => {
            ask(window, &refused.message, MessageDialogKind::Warning, MessageDialogButtons::Ok);
            desktop::give_back_focus(window.as_ref());
        }
    }
}

fn ask(window: &WebviewWindow, text: &str, kind: MessageDialogKind, buttons: MessageDialogButtons) -> bool {
    window.app_handle().dialog().message(text).title("Xiox").kind(kind).parent(window).buttons(buttons).blocking_show()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_dropped_folder_is_offered_and_files_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        let gone = dir.path().join("gone");
        assert_eq!(folder_of(&[file.clone(), folder.clone()]), Some(folder.as_path()), "a file before it is skipped");
        assert_eq!(folder_of(&[gone, file]), None, "no folder: nothing to offer");
        assert_eq!(folder_of(&[]), None);
    }

    #[test]
    fn the_question_names_the_folder() {
        assert_eq!(question(Path::new(r"D:\Work\acme")), r"Add “D:\Work\acme” to the folders Developers analyzes?");
    }
}
