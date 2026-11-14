//! The backend over the engine: `Core` behind a mutex. Every intent goes through the section state's
//! own checks (`Core` maps the intent to the state's method; the state refuses what the old window
//! had disabled); a refusal is turned into `CmdError::refused` with the state's English reason.

use crate::backend::Backend;
use crate::error::{CmdError, CmdResult};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use xiox_core::app::Core;
use xiox_core::notify::SectionId;
use xiox_core::view::{AboutView, AppInfo, CleanupIntent, DevIntent, RegistryIntent, SectionView, ServicesIntent};

pub struct RealBackend {
    core: Arc<Mutex<Core>>,
}

impl RealBackend {
    pub fn new(core: Arc<Mutex<Core>>) -> Self {
        RealBackend { core }
    }

    /// A worker that panicked while it held the lock must not take the window down with it.
    fn core(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Backend for RealBackend {
    fn app_info(&self) -> AppInfo {
        self.core().app_info(false)
    }

    fn about(&self) -> AboutView {
        self.core().about()
    }

    fn show(&self, s: SectionId) -> SectionView {
        self.core().show(s)
    }

    fn view(&self, s: SectionId) -> SectionView {
        self.core().view(s)
    }

    fn cleanup(&self, i: CleanupIntent) -> CmdResult<SectionView> {
        self.core().cleanup(i).map_err(CmdError::refused)
    }

    fn registry(&self, i: RegistryIntent) -> CmdResult<SectionView> {
        self.core().registry(i).map_err(CmdError::refused)
    }

    fn services(&self, i: ServicesIntent) -> CmdResult<SectionView> {
        self.core().services(i).map_err(CmdError::refused)
    }

    fn developers(&self, i: DevIntent) -> CmdResult<SectionView> {
        self.core().developers(i).map_err(CmdError::refused)
    }

    fn add_root(&self, dir: PathBuf) -> CmdResult<SectionView> {
        self.core().add_root(dir).map_err(CmdError::refused)
    }

    fn report_text(&self, s: SectionId) -> Option<String> {
        self.core().report_text(s)
    }

    fn set_saved(&self, s: SectionId, text: String, ok: bool) -> SectionView {
        self.core().set_saved(s, text, ok)
    }

    fn is_fake(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiox_core::app::CoreOptions;
    use xiox_core::cleanup::{CleanJob, CleanMsg, CleanupState, CleanupWorkers};
    use xiox_core::dev::{DevState, Section};
    use xiox_core::notify::{NoNotifier, Outbox};
    use xiox_core::regstate::RegistryState;
    use xiox_core::svcstate::ServicesState;

    /// A cleanup worker that only says it is done: no test here reaches the engine.
    fn quiet(_: CleanJob, out: Outbox<CleanMsg>) {
        out.send(CleanMsg::Done);
    }

    fn backend() -> RealBackend {
        // `--only-root`: settings in memory, nothing written, and nothing searched outside this folder.
        let dev = DevState::only_root(PathBuf::from(r"C:\xiox-test-nowhere"), Section::Projects, false);
        let core = Core::with_states(
            CoreOptions { version: "t".into(), admin: false, only_root: None, load_rules: false, update_rules: false },
            Arc::new(NoNotifier),
            CleanupState::with(Vec::new(), Vec::new(), CleanupWorkers { job: quiet }),
            RegistryState::new(),
            ServicesState::new(),
            dev,
        );
        RealBackend::new(Arc::new(Mutex::new(core)))
    }

    /// The state's reason reaches the page as a refusal, and nothing started.
    #[test]
    fn a_refusal_of_the_state_is_a_refused_command_error() {
        let b = backend();
        let e = b.cleanup(CleanupIntent::Confirm).unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), ("refused", xiox_core::dev::NO_DIALOG));
        let e = b.developers(DevIntent::Permanent).unwrap_err();
        assert_eq!(e.kind, "refused");
        let e = b.registry(RegistryIntent::Fix { backup: false }).unwrap_err();
        assert_eq!(e.kind, "refused");
        let e = b.services(ServicesIntent::Confirm).unwrap_err();
        assert_eq!(e.kind, "refused");
        assert!(!b.is_fake());
        assert_eq!(b.report_text(SectionId::Cleanup), None);
        assert!(b.view(SectionId::Cleanup).job.is_none());
    }
}
