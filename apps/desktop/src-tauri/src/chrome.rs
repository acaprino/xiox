//! What the window says outside the page: its title (Alt+Tab, the taskbar and screen readers tell Cleanup from
//! Registry) and the taskbar button (progress while a job runs, attention when one ends behind other windows). The
//! page reports the open section and the running job's percent whenever either changes (`window_chrome`); what each
//! change means is decided here, in pure functions.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::UserAttentionType;
use xiox_core::notify::SectionId;

/// The name every window title ends with: `single_instance::is_title_of` finds the window by it.
pub const APP_NAME: &str = "Xiox";

/// The title of the window while `section` is open: «Cleanup - Xiox».
pub fn title(section: SectionId) -> String {
    let name = match section {
        SectionId::Cleanup => "Cleanup",
        SectionId::Registry => "Registry",
        SectionId::Services => "Services",
        SectionId::Developers => "Developers",
    };
    format!("{name} - {APP_NAME}")
}

/// What the taskbar button shows.
#[derive(Debug, PartialEq, Eq)]
pub enum Bar {
    /// No job: no progress.
    None,
    /// A job at this percent (0 to 100).
    Percent(u8),
}

/// The taskbar's answer to a report of the job: the progress to show, and whether to ask for attention.
#[derive(Debug, PartialEq, Eq)]
pub struct Taskbar {
    pub bar: Bar,
    pub attention: bool,
}

/// `was_running`: a job was running at the last report; `job`: its percent now (None when there is none);
/// `focused`: the window has the focus. A job that ends while the window is not in front asks for attention;
/// the person looking at the window needs no flash.
pub fn taskbar(was_running: bool, job: Option<u8>, focused: bool) -> Taskbar {
    Taskbar {
        bar: job.map_or(Bar::None, |percent| Bar::Percent(percent.min(100))),
        attention: was_running && job.is_none() && !focused,
    }
}

/// A job was running at the last report. Process-wide on purpose: the app has one window.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// The page's report: the open section and the percent of the running job, if any.
#[tauri::command]
pub fn window_chrome(window: tauri::WebviewWindow, section: SectionId, job: Option<u8>) {
    let _ = window.set_title(&title(section));
    let was_running = RUNNING.swap(job.is_some(), Ordering::SeqCst);
    let focused = window.is_focused().unwrap_or(true);
    let Taskbar { bar, attention } = taskbar(was_running, job, focused);
    let state = match bar {
        Bar::None => ProgressBarState { status: Some(ProgressBarStatus::None), progress: None },
        Bar::Percent(percent) => ProgressBarState { status: Some(ProgressBarStatus::Normal), progress: Some(u64::from(percent)) },
    };
    let _ = window.set_progress_bar(state);
    if attention {
        // Cleared by Windows when the window is activated.
        let _ = window.request_user_attention(Some(UserAttentionType::Informational));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_names_the_section_and_ends_with_the_app() {
        for (section, want) in [
            (SectionId::Cleanup, "Cleanup - Xiox"),
            (SectionId::Registry, "Registry - Xiox"),
            (SectionId::Services, "Services - Xiox"),
            (SectionId::Developers, "Developers - Xiox"),
        ] {
            assert_eq!(title(section), want);
            assert!(crate::single_instance::is_title_of(APP_NAME, &title(section)), "the running window is still found by `{want}`");
        }
    }

    #[test]
    fn the_taskbar_follows_the_job() {
        assert_eq!(taskbar(false, None, true), Taskbar { bar: Bar::None, attention: false });
        assert_eq!(taskbar(false, Some(0), true), Taskbar { bar: Bar::Percent(0), attention: false });
        assert_eq!(taskbar(true, Some(40), false), Taskbar { bar: Bar::Percent(40), attention: false });
        assert_eq!(taskbar(true, Some(250), false).bar, Bar::Percent(100), "never past the end");
    }

    #[test]
    fn attention_only_when_a_job_ends_while_the_window_is_not_in_front() {
        assert!(taskbar(true, None, false).attention, "ended behind other windows");
        assert!(!taskbar(true, None, true).attention, "the person is looking at it");
        assert!(!taskbar(false, None, false).attention, "no job ended: another report of the same state");
        assert!(!taskbar(true, Some(99), false).attention, "still running");
    }
}
