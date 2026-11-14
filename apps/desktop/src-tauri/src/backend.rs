//! What the commands talk to. Two implementations: `RealBackend` (the engine, through `Core`
//! behind a mutex) and, in debug builds only, `FakeBackend` (scripted fixtures, no engine).
//!
//! The commands never see which one they have; every answer is a `SectionView` the page draws, and
//! a refusal is a `CmdError { kind: "refused" }` with an English reason.

use crate::error::CmdResult;
use std::path::PathBuf;
use std::sync::Arc;
use xiox_core::notify::SectionId;
use xiox_core::view::{AboutView, AppInfo, CleanupIntent, DevIntent, RegistryIntent, SectionView, ServicesIntent};

pub trait Backend: Send + Sync + 'static {
    fn app_info(&self) -> AppInfo;
    fn about(&self) -> AboutView;
    fn show(&self, s: SectionId) -> SectionView;
    fn view(&self, s: SectionId) -> SectionView;
    fn cleanup(&self, i: CleanupIntent) -> CmdResult<SectionView>;
    fn registry(&self, i: RegistryIntent) -> CmdResult<SectionView>;
    fn services(&self, i: ServicesIntent) -> CmdResult<SectionView>;
    fn developers(&self, i: DevIntent) -> CmdResult<SectionView>;
    /// A folder for Developers to analyze too. The path comes from the folder picker the command opens
    /// in Rust, never from the page.
    fn add_root(&self, dir: PathBuf) -> CmdResult<SectionView>;
    /// The text of the report on screen, when there is one to save.
    fn report_text(&self, s: SectionId) -> Option<String>;
    /// What the save dialog and the write came to, put on the report.
    fn set_saved(&self, s: SectionId, text: String, ok: bool) -> SectionView;
    /// Only the debug-build fake window asks (`XIOX_UI_FAKE`); a release build never reads it.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    fn is_fake(&self) -> bool;
}

/// The tauri managed state: the commands' only handle on the application.
pub struct BackendState(pub Arc<dyn Backend>);
