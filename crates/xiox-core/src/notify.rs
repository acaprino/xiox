//! How a worker thread wakes whoever shows its section: a [`Notifier`] instead of a graphics context.
//!
//! A worker talks to its section's state over an `mpsc` channel and then calls
//! [`Notifier::notify`]; the state's `poll` drains the channel. [`Outbox`] bundles the two so a
//! worker cannot forget the second half, and so a worker that dies still wakes the UI.

use std::sync::mpsc::Sender;
use std::sync::Arc;

/// The four sections of the window. Serialised in lower case (`"cleanup"`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SectionId {
    Cleanup,
    Registry,
    Services,
    Developers,
}

/// "Something changed in this section, poll it". May be called from any thread, at any time.
pub trait Notifier: Send + Sync {
    fn notify(&self, section: SectionId);
}

pub type SharedNotifier = Arc<dyn Notifier>;

/// A notifier that does nothing (tests, headless use).
pub struct NoNotifier;

impl Notifier for NoNotifier {
    fn notify(&self, _: SectionId) {}
}

/// The sending end of a worker: `send` = `tx.send(m)` (a closed receiver is ignored) then `notify`.
///
/// `Drop` drops the sender FIRST and notifies afterwards, so the poller that wakes up sees
/// `Disconnected`: a worker that returns or panics always ends with a wake-up and a closed channel.
pub struct Outbox<M: Send> {
    tx: Option<Sender<M>>,
    notifier: SharedNotifier,
    section: SectionId,
}

impl<M: Send> Outbox<M> {
    pub fn new(tx: Sender<M>, notifier: SharedNotifier, section: SectionId) -> Self {
        Outbox { tx: Some(tx), notifier, section }
    }

    pub fn send(&self, m: M) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(m);
        }
        self.notifier.notify(self.section);
    }
}

impl<M: Send> Drop for Outbox<M> {
    fn drop(&mut self) {
        // Close the channel before waking the poller, or it could still see `Empty`.
        self.tx = None;
        self.notifier.notify(self.section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver, TryRecvError};
    use std::sync::{Arc, Mutex};

    /// A notifier that looks into the channel each time it is woken: what the poller would see.
    struct Probe {
        rx: Mutex<Receiver<u8>>,
        seen: Mutex<Vec<Result<u8, TryRecvError>>>,
    }

    impl Probe {
        fn new(rx: Receiver<u8>) -> Arc<Probe> {
            Arc::new(Probe { rx: Mutex::new(rx), seen: Mutex::new(Vec::new()) })
        }

        fn seen(&self) -> Vec<Result<u8, TryRecvError>> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Notifier for Probe {
        fn notify(&self, _: SectionId) {
            let r = self.rx.lock().unwrap().try_recv();
            self.seen.lock().unwrap().push(r);
        }
    }

    #[test]
    fn outbox_drop_disconnects_before_notifying() {
        let (tx, rx) = channel();
        let probe = Probe::new(rx);
        let out = Outbox::new(tx, probe.clone(), SectionId::Developers);
        out.send(1u8);
        drop(out);
        assert_eq!(probe.seen(), vec![Ok(1), Err(TryRecvError::Disconnected)]);
    }

    #[test]
    fn a_panicking_worker_still_notifies() {
        let (tx, rx) = channel();
        let probe = Probe::new(rx);
        let out = Outbox::new(tx, probe.clone(), SectionId::Developers);
        let worker = std::thread::spawn(move || {
            let _out = out;
            panic!("worker died");
        });
        assert!(worker.join().is_err());
        assert_eq!(probe.seen(), vec![Err(TryRecvError::Disconnected)]);
    }
}
