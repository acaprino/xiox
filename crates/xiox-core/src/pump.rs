//! The view pump: what turns a worker's wake-up into a view for the page.
//!
//! No frame loop polls the states any more. A worker (through its `Outbox`) calls
//! [`Notifier::notify`]; [`PumpNotifier`] puts the section on a channel; the pump thread waits a
//! short while so a burst of wake-ups becomes one view per section, then locks the core, builds
//! the section's view (`Core::view` polls first, so a worker that died is seen as dead) and hands
//! it to the sink. The sink is called with the core UNLOCKED.

use crate::app::Core;
use crate::notify::{Notifier, SectionId};
use crate::view::SectionView;
use std::collections::BTreeSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Where the views go (the Tauri crate: the `xiox:view` event of the window).
pub trait ViewSink: Send + Sync + 'static {
    fn send(&self, view: &SectionView);
}

/// How long the pump waits after the first wake-up before it reads the rest.
pub const PUMP_COALESCE_MS: u64 = 100;

/// The `Notifier` the states' workers get: it only names the section that changed.
pub struct PumpNotifier {
    tx: Sender<SectionId>,
}

impl Notifier for PumpNotifier {
    fn notify(&self, section: SectionId) {
        let _ = self.tx.send(section);
    }
}

pub fn pump_channel() -> (PumpNotifier, Receiver<SectionId>) {
    let (tx, rx) = channel();
    (PumpNotifier { tx }, rx)
}

/// The pump thread: receive a section, wait `PUMP_COALESCE_MS`, drain the rest, then for each
/// distinct section lock the core (poison-tolerant), `view(s)` and `sink.send(&v)`. It ends when
/// every sender is gone (the `Core` itself holds one through its notifier, so in the application
/// it runs as long as the process does).
pub fn spawn_pump(core: Arc<Mutex<Core>>, rx: Receiver<SectionId>, sink: Arc<dyn ViewSink>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            std::thread::sleep(Duration::from_millis(PUMP_COALESCE_MS));
            let mut sections = BTreeSet::from([first]);
            while let Ok(s) = rx.try_recv() {
                sections.insert(s);
            }
            for s in sections {
                let view = core.lock().unwrap_or_else(|e| e.into_inner()).view(s);
                sink.send(&view);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::CoreOptions;
    use crate::cleanup::{CleanupState, CleanupWorkers};
    use crate::dev::{DevCleanJob, DevMsg, DevState, DevWorkers, Section};
    use crate::notify::Outbox;
    use crate::regstate::RegistryState;
    use crate::svcstate::ServicesState;
    use crate::view::{DevIntent, Tone};
    use std::time::Instant;

    /// Records every view it is given.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<SectionView>>);

    impl ViewSink for Recorder {
        fn send(&self, view: &SectionView) {
            self.0.lock().unwrap().push(view.clone());
        }
    }

    impl Recorder {
        fn views(&self) -> Vec<SectionView> {
            self.0.lock().unwrap().clone()
        }

        fn wait_for(&self, what: &str, found: impl Fn(&[SectionView]) -> bool) {
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                if found(&self.views()) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("within 2 s: {what}; got {:?}", self.views().iter().map(|v| (v.section, v.rev)).collect::<Vec<_>>());
        }
    }

    fn scan_never(_: xiox_model::Settings, _: bool, _: Outbox<DevMsg>) {}
    fn scan_dies(_: xiox_model::Settings, _: bool, _out: Outbox<DevMsg>) {
        panic!("the scan worker died");
    }
    fn clean_never(_: DevCleanJob, _: Outbox<DevMsg>) {}

    /// A core whose workers never reach the engine, whose notifier is the pump's, and the pump over it.
    fn running(scan: fn(xiox_model::Settings, bool, Outbox<DevMsg>)) -> (Arc<Mutex<Core>>, Arc<Recorder>, PumpNotifier) {
        let (notifier, rx) = pump_channel();
        let mut dev = DevState::with_settings(xiox_app::default_settings(), Section::Projects, false)
            .with_workers(DevWorkers { scan, clean: clean_never });
        dev.persist = false;
        let core = Core::with_states(
            CoreOptions { version: "t".into(), admin: false, only_root: None, load_rules: false, update_rules: false },
            Arc::new(PumpNotifier { tx: notifier.tx.clone() }),
            CleanupState::with(Vec::new(), Vec::new(), CleanupWorkers::REAL),
            RegistryState::new(),
            ServicesState::new(),
            dev,
        );
        let core = Arc::new(Mutex::new(core));
        let sink = Arc::new(Recorder::default());
        spawn_pump(core.clone(), rx, sink.clone());
        (core, sink, notifier)
    }

    #[test]
    fn pump_coalesces_and_publishes_views() {
        let (_core, sink, notifier) = running(scan_never);
        for s in [SectionId::Cleanup, SectionId::Registry, SectionId::Cleanup, SectionId::Cleanup, SectionId::Registry] {
            notifier.notify(s);
            std::thread::sleep(Duration::from_millis(2));
        }
        sink.wait_for("a view of each notified section", |v| v.len() >= 2);
        // A burst is one view per section, not one per wake-up: nothing else comes after the window.
        std::thread::sleep(Duration::from_millis(3 * PUMP_COALESCE_MS));
        let views = sink.views();
        assert_eq!(views.iter().map(|v| v.section).collect::<Vec<_>>(), vec![SectionId::Cleanup, SectionId::Registry]);
        assert!(views[1].rev > views[0].rev, "each view is a new revision");
        // A later wake-up is a later view, after its own window.
        notifier.notify(SectionId::Cleanup);
        sink.wait_for("the second burst", |v| v.len() == 3);
        assert_eq!(sink.views()[2].section, SectionId::Cleanup);
        assert!(sink.views()[2].rev > views[1].rev);
    }

    #[test]
    fn pump_publishes_the_view_after_a_worker_dies() {
        let (core, sink, _notifier) = running(scan_dies);
        core.lock().unwrap().developers(DevIntent::Analyze).expect("the scan starts");
        // The worker panics, its outbox wakes the pump, and the view that follows has no job left
        // and says what happened: no spinner that never stops.
        sink.wait_for("a Developers view without a job and with an error", |v| {
            v.iter().any(|v| v.section == SectionId::Developers && v.job.is_none() && v.notices.iter().any(|n| n.tone == Tone::Error))
        });
    }
}
