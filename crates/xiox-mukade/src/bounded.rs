//! Running a child process under a deadline and an output cap.
//!
//! `Command::output()` waits forever and buffers whatever the child writes. [`run_bounded`] reads
//! both pipes on their own threads (so a full pipe never stalls the child), polls for the exit,
//! and kills the child when the deadline passes or stdout outgrows its cap.

use std::fmt;
use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(10);
const READ_CHUNK: usize = 64 * 1024;
/// Bytes of stderr kept; the rest is read and dropped.
const STDERR_KEPT: usize = 64 * 1024;

/// How long a child may run and how much stdout it may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub timeout: Duration,
    pub max_stdout: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(120), max_stdout: 256 * 1024 * 1024 }
    }
}

/// What a child that ended in time wrote.
#[derive(Debug)]
pub struct Captured {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub enum BoundedError {
    Io(std::io::Error),
    TimedOut(Duration),
    StdoutCap(usize),
}

impl fmt::Display for BoundedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::TimedOut(t) => write!(f, "timed out after {} s", t.as_secs_f64()),
            Self::StdoutCap(cap) => write!(f, "more than {cap} bytes on stdout"),
        }
    }
}

impl std::error::Error for BoundedError {}

impl From<std::io::Error> for BoundedError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// A pipe read to its end on its own thread; only the first `keep` bytes are retained and `over`
/// is raised as soon as more arrive.
struct Pipe {
    reader: JoinHandle<Vec<u8>>,
    over: Arc<AtomicBool>,
}

impl Pipe {
    fn spawn(mut source: impl Read + Send + 'static, keep: usize) -> Self {
        let over = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&over);
        let reader = thread::spawn(move || {
            let (mut kept, mut chunk) = (Vec::new(), vec![0u8; READ_CHUNK]);
            while let Ok(n) = source.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let room = keep.saturating_sub(kept.len()).min(n);
                kept.extend_from_slice(&chunk[..room]);
                if room < n {
                    flag.store(true, Ordering::Relaxed);
                }
            }
            kept
        });
        Self { reader, over }
    }

    fn is_over(&self) -> bool {
        self.over.load(Ordering::Relaxed)
    }

    fn finish(self) -> (Vec<u8>, bool) {
        let kept = self.reader.join().unwrap_or_default();
        (kept, self.over.load(Ordering::Relaxed))
    }
}

/// Runs `cmd` with piped output until it exits, `limits.timeout` passes or its stdout passes
/// `limits.max_stdout`; in the last two cases the child is killed. The reader threads of a killed
/// child are left to end on their own, since a grandchild may still hold the pipes open.
pub fn run_bounded(cmd: &mut Command, limits: Limits) -> Result<Captured, BoundedError> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = Pipe::spawn(child.stdout.take().expect("stdout is piped"), limits.max_stdout);
    let stderr = Pipe::spawn(child.stderr.take().expect("stderr is piped"), STDERR_KEPT);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let failure = if stdout.is_over() {
            BoundedError::StdoutCap(limits.max_stdout)
        } else if started.elapsed() >= limits.timeout {
            BoundedError::TimedOut(limits.timeout)
        } else {
            thread::sleep(POLL);
            continue;
        };
        let _ = child.kill();
        let _ = child.wait();
        return Err(failure);
    };
    let (stdout, over) = stdout.finish();
    if over {
        return Err(BoundedError::StdoutCap(limits.max_stdout));
    }
    Ok(Captured { status, stdout, stderr: stderr.finish().0 })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn cmd_c(script: &str) -> Command {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", script]);
        cmd
    }

    fn limits(secs: u64, cap: usize) -> Limits {
        Limits { timeout: Duration::from_secs(secs), max_stdout: cap }
    }

    #[test]
    fn a_quick_child_is_captured_with_stderr_and_status() {
        let out = run_bounded(&mut cmd_c("echo hello& echo oops 1>&2& exit 3"), limits(30, 1024)).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "oops");
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn a_slow_child_is_killed_at_the_deadline() {
        let started = Instant::now();
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "30", "127.0.0.1"]);
        let err = run_bounded(&mut cmd, Limits { timeout: Duration::from_millis(300), ..Limits::default() })
            .unwrap_err();
        assert!(matches!(err, BoundedError::TimedOut(_)), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    }

    #[test]
    fn runaway_stdout_is_cut_at_the_cap() {
        let script = "for /l %i in (1,1,200000) do @echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
        let err = run_bounded(&mut cmd_c(script), limits(60, 1024)).unwrap_err();
        assert!(matches!(err, BoundedError::StdoutCap(1024)), "{err}");
    }

    #[test]
    fn output_just_under_the_cap_is_kept() {
        let out = run_bounded(&mut cmd_c("echo abc"), limits(30, 5)).unwrap();
        assert_eq!(out.stdout.len(), 5);
    }

    #[test]
    fn a_missing_program_is_an_io_error() {
        let err = run_bounded(&mut Command::new("no-such-program-xiox"), Limits::default()).unwrap_err();
        assert!(matches!(err, BoundedError::Io(_)), "{err}");
    }
}
