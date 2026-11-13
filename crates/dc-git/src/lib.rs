//! Git state via the `git` CLI (`dc_model::GitProbe`). No libgit2.
//!
//! Every invocation is `git -C <root> --no-optional-locks -c core.fsmonitor=false ...`
//! with `GIT_TERMINAL_PROMPT=0`, no console window and a 15 s timeout (the process is
//! killed on timeout). A missing git binary, a timeout or a `dubious ownership` refusal
//! never panics: the affected `GitState` fields are simply `None` / `false`.

mod project;
pub use project::*;

use dc_model::{GitProbe, GitState, normalize_path, path_key};
use std::io::Read;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `GitProbe` backed by the git command line.
#[derive(Clone, Copy, Debug, Default)]
pub struct GitCli;

/// True if a `git` executable can be started (`git --version` succeeds).
pub fn git_available() -> bool {
    let mut c = base_command();
    c.arg("--version");
    run_cmd(c, TIMEOUT).is_some_and(|o| o.ok)
}

impl GitCli {
    /// Like [`GitProbe::is_tracked`] but `None` when git could not answer (missing git,
    /// timeout, dubious ownership, ...). `Some(false)` is returned for a definite
    /// "not a repository" or "nothing tracked there". Callers that block deletion on
    /// tracked files should treat `None` as "unknown, do not touch".
    pub fn is_tracked_checked(&self, root: &Path, rel: &Path) -> Option<bool> {
        let rel = rel.to_string_lossy().replace('\\', "/");
        let o = git(root, &["ls-files", "-z", "--", &rel])?;
        if o.ok {
            return Some(!o.stdout.is_empty());
        }
        o.stderr.contains("not a git repository").then_some(false)
    }
}

impl GitProbe for GitCli {
    fn state(&self, root: &Path) -> GitState {
        let mut st = GitState::default();

        match git(root, &["rev-parse", "--is-inside-work-tree"]) {
            Some(o) if o.ok && o.text() == "true" => st.is_repo = true,
            _ => return st,
        }

        st.dirty = git(root, &["status", "--porcelain=v1", "--untracked-files=normal"])
            .filter(|o| o.ok)
            .map(|o| o.stdout.iter().any(|b| !b.is_ascii_whitespace()));

        st.branch = git(root, &["symbolic-ref", "--short", "-q", "HEAD"])
            .filter(|o| o.ok)
            .map(|o| o.text())
            .filter(|s| !s.is_empty());

        st.has_upstream = git(root, &["rev-parse", "--abbrev-ref", "@{u}"]).map(|o| o.ok);

        // Unborn branch (no commits yet): leave the commit-derived fields as None.
        let has_head = git(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_some_and(|o| o.ok);
        if has_head {
            st.unpushed_commits = git(root, &["rev-list", "--count", "HEAD", "--not", "--remotes"])
                .filter(|o| o.ok)
                .and_then(|o| o.text().parse::<u32>().ok());
            st.last_commit_ms = git(root, &["log", "-1", "--format=%at"])
                .filter(|o| o.ok)
                .and_then(|o| o.text().parse::<i64>().ok())
                .and_then(|s| s.checked_mul(1000));
        }

        let (linked, main) = worktree_info(root);
        st.is_linked_worktree = linked;
        st.main_worktree = main;
        st
    }

    fn is_tracked(&self, root: &Path, rel: &Path) -> bool {
        self.is_tracked_checked(root, rel).unwrap_or(false)
    }

    fn is_ignored(&self, root: &Path, rel: &Path) -> Option<bool> {
        // Trailing `/` so directory-only patterns such as `dist/` match.
        let rel = format!("{}/", rel.to_string_lossy().replace('\\', "/").trim_end_matches('/'));
        // `check-ignore` rejects the literal-pathspec mode that `base_command` enables.
        match git(root, &["--no-literal-pathspecs", "check-ignore", "-q", "--", &rel])?.code {
            Some(0) => Some(true),
            Some(1) => Some(false),
            _ => None,
        }
    }
}

// ───────────────────────────── worktrees ─────────────────────────────

/// `(is_linked_worktree, main_worktree)`; `main_worktree` is only set for linked worktrees.
fn worktree_info(root: &Path) -> (bool, Option<PathBuf>) {
    let Some(o) = git(root, &["rev-parse", "--git-dir", "--git-common-dir"]).filter(|o| o.ok) else {
        return (false, None);
    };
    let text = o.text();
    let mut lines = text.lines();
    let (Some(git_dir), Some(common_dir)) = (lines.next(), lines.next()) else {
        return (false, None);
    };
    if path_key(&resolve(root, git_dir.trim())) == path_key(&resolve(root, common_dir.trim())) {
        return (false, None);
    }
    (true, main_worktree(root))
}

/// Absolute, normalized form of a path printed by git (absolute, or relative to `root`).
pub(crate) fn resolve(root: &Path, p: &str) -> PathBuf {
    let joined = root.join(p);
    normalize_path(&std::fs::canonicalize(&joined).unwrap_or(joined))
}

/// First entry of `git worktree list --porcelain` (always the main working tree).
pub(crate) fn main_worktree(root: &Path) -> Option<PathBuf> {
    // -z (git >= 2.36) avoids path quoting; fall back to the quoted form on older git.
    let path = if let Some(o) = git(root, &["worktree", "list", "--porcelain", "-z"]).filter(|o| o.ok) {
        let first = o.stdout.split(|b| *b == 0).next()?;
        String::from_utf8_lossy(first)
            .strip_prefix("worktree ")?
            .to_string()
    } else {
        let o = git(root, &["worktree", "list", "--porcelain"]).filter(|o| o.ok)?;
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        let rest = text.lines().next()?.strip_prefix("worktree ")?.to_string();
        unquote_c(&rest)
    };
    (!path.is_empty()).then(|| normalize_path(Path::new(&path)))
}

/// Undo git's C-style path quoting (`"caff\303\250"`); unquoted input is returned as is.
pub(crate) fn unquote_c(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return s.to_string();
    };
    let b = inner.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        if (b'0'..=b'7').contains(&c) {
            let mut v = 0u32;
            let mut n = 0;
            while n < 3 && i + 1 + n < b.len() && (b'0'..=b'7').contains(&b[i + 1 + n]) {
                v = v * 8 + u32::from(b[i + 1 + n] - b'0');
                n += 1;
            }
            out.push((v & 0xff) as u8);
            i += 1 + n;
            continue;
        }
        out.push(match c {
            b'n' => b'\n',
            b't' => b'\t',
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'r' => b'\r',
            b'v' => 0x0b,
            other => other, // \\ and \"
        });
        i += 2;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ───────────────────────────── process plumbing ─────────────────────────────

pub(crate) struct Out {
    ok: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

impl Out {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_string()
    }
}

fn base_command() -> Command {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C");
    // Never let an inherited repo selection (e.g. when launched from a git hook) leak in.
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_PREFIX"] {
        c.env_remove(var);
    }
    // Tests only: the OS temp dir may itself live inside some repo, which would turn the
    // "plain directory" fixtures into repos. Do not climb out of the temp dir.
    #[cfg(test)]
    c.env("GIT_CEILING_DIRECTORIES", tests::temp_root());
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Option<Out> {
    let mut c = base_command();
    c.arg("-C")
        .arg(root)
        .arg("--no-optional-locks")
        .args(["-c", "core.fsmonitor=false"])
        .args(args);
    run_cmd(c, TIMEOUT)
}

/// Runs the command to completion. `None` = could not spawn, timed out (child killed) or
/// output could not be collected.
fn run_cmd(mut cmd: Command, timeout: Duration) -> Option<Out> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().ok()?;

    // Drain both pipes on helper threads so a chatty child can never block on a full pipe.
    let (tx, rx) = mpsc::channel::<(bool, Vec<u8>)>();
    if let Some(mut s) = child.stdout.take() {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut v = Vec::new();
            let _ = s.read_to_end(&mut v);
            let _ = tx.send((true, v));
        });
    }
    if let Some(mut s) = child.stderr.take() {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut v = Vec::new();
            let _ = s.read_to_end(&mut v);
            let _ = tx.send((false, v));
        });
    }
    drop(tx);

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(3)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };

    // The child has exited; a grandchild holding the pipe open must not hang us.
    let grace = Instant::now() + Duration::from_secs(5);
    let (mut stdout, mut stderr, mut got) = (Vec::new(), Vec::new(), 0);
    while got < 2 {
        match rx.recv_timeout(grace.saturating_duration_since(Instant::now())) {
            Ok((true, v)) => stdout = v,
            Ok((false, v)) => stderr = v,
            Err(_) => return None,
        }
        got += 1;
    }
    Some(Out {
        ok: status.success(),
        code: status.code(),
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

// ─────────────────────────────────── tests ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Canonical OS temp dir; used as GIT_CEILING_DIRECTORIES for every git call.
    pub(super) fn temp_root() -> PathBuf {
        let t = std::env::temp_dir();
        normalize_path(&fs::canonicalize(&t).unwrap_or(t))
    }

    /// Self-cleaning temp dir, canonicalized so paths compare equal to what git prints.
    pub(super) struct Tmp(pub(super) PathBuf);

    impl Tmp {
        pub(super) fn new() -> Tmp {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let p = temp_root().join(format!("dcgit-{}-{n}", std::process::id()));
            fs::create_dir_all(&p).unwrap();
            Tmp(normalize_path(&fs::canonicalize(&p).unwrap()))
        }
        pub(super) fn sub(&self, name: &str) -> PathBuf {
            let p = self.0.join(name);
            fs::create_dir_all(&p).unwrap();
            p
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The command the tests run git with: author identity fixed, and never an inherited
    /// repository selection (a test run from a git hook must not touch the hook's repository).
    fn git_cmd(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Command {
        let mut c = Command::new("git");
        c.arg("-C")
            .arg(dir)
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_TERMINAL_PROMPT", "0");
        for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_PREFIX"] {
            c.env_remove(var);
        }
        for (k, v) in envs {
            c.env(k, v);
        }
        #[cfg(windows)]
        c.creation_flags(CREATE_NO_WINDOW);
        c
    }

    #[test]
    fn the_test_git_never_inherits_a_repository_selection() {
        let c = git_cmd(Path::new("."), &["status"], &[]);
        for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_PREFIX"] {
            let set = c.get_envs().find(|(k, _)| *k == std::ffi::OsStr::new(var));
            assert!(matches!(set, Some((_, None))), "{var} must be removed from the child environment");
        }
    }

    fn git_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
        let out = git_cmd(dir, args, envs).output().expect("git must be installed to run these tests");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    pub(super) fn g(dir: &Path, args: &[&str]) -> String {
        git_env(dir, args, &[])
    }

    pub(super) fn init(dir: &Path) {
        g(dir, &["init", "-q", "-b", "main"]);
    }

    pub(super) fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    pub(super) fn commit_file(dir: &Path, rel: &str, content: &str) {
        write(dir, rel, content);
        g(dir, &["add", "-A"]);
        g(dir, &["commit", "-q", "-m", &format!("add {rel}")]);
    }

    /// repo with one commit, plus a bare remote it is pushed to (with `-u` if `track`).
    fn repo_with_remote(t: &Tmp, track: bool) -> (PathBuf, PathBuf) {
        let bare = t.sub("remote.git");
        g(&bare, &["init", "-q", "--bare", "-b", "main"]);
        let repo = t.sub("work");
        init(&repo);
        commit_file(&repo, "a.txt", "a");
        g(&repo, &["remote", "add", "origin", &bare.to_string_lossy()]);
        if track {
            g(&repo, &["push", "-q", "-u", "origin", "main"]);
        } else {
            g(&repo, &["push", "-q", "origin", "main"]);
        }
        (repo, bare)
    }

    #[test]
    fn git_is_available() {
        assert!(git_available());
    }

    #[test]
    fn clean_repo() {
        let t = Tmp::new();
        let repo = t.sub("clean");
        init(&repo);
        git_env(
            &repo,
            &["commit", "-q", "--allow-empty", "-m", "first"],
            &[("GIT_AUTHOR_DATE", "2020-01-02T03:04:05Z")],
        );
        write(&repo, "a.txt", "a");
        g(&repo, &["add", "-A"]);
        g(&repo, &["commit", "-q", "-m", "second"]);
        let st = GitCli.state(&repo);
        assert!(st.is_repo);
        assert_eq!(st.dirty, Some(false));
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.has_upstream, Some(false));
        assert_eq!(st.unpushed_commits, Some(2), "no remotes: every commit is unpushed");
        assert!(!st.is_linked_worktree);
        assert_eq!(st.main_worktree, None);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let last = st.last_commit_ms.expect("last commit");
        assert!((now - last).abs() < 120_000, "last={last} now={now}");
    }

    #[test]
    fn author_date_is_reported_in_ms() {
        let t = Tmp::new();
        let repo = t.sub("dated");
        init(&repo);
        git_env(
            &repo,
            &["commit", "-q", "--allow-empty", "-m", "old"],
            &[("GIT_AUTHOR_DATE", "2020-01-02T03:04:05Z")],
        );
        assert_eq!(GitCli.state(&repo).last_commit_ms, Some(1_577_934_245_000));
    }

    #[test]
    fn unborn_repo_has_no_commit_fields() {
        let t = Tmp::new();
        let repo = t.sub("unborn");
        init(&repo);
        let st = GitCli.state(&repo);
        assert!(st.is_repo);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.unpushed_commits, None);
        assert_eq!(st.last_commit_ms, None);
        assert_eq!(st.dirty, Some(false));
        write(&repo, "x.txt", "x");
        assert_eq!(GitCli.state(&repo).dirty, Some(true));
    }

    #[test]
    fn dirty_modified_and_untracked() {
        let t = Tmp::new();
        let repo = t.sub("dirty");
        init(&repo);
        commit_file(&repo, "a.txt", "a");

        write(&repo, "a.txt", "changed");
        assert_eq!(GitCli.state(&repo).dirty, Some(true), "modified tracked file");
        g(&repo, &["checkout", "-q", "--", "a.txt"]);
        assert_eq!(GitCli.state(&repo).dirty, Some(false));

        write(&repo, "new.txt", "n");
        assert_eq!(GitCli.state(&repo).dirty, Some(true), "untracked file");
    }

    #[test]
    fn staged_change_is_dirty() {
        let t = Tmp::new();
        let repo = t.sub("staged");
        init(&repo);
        commit_file(&repo, "a.txt", "a");
        write(&repo, "b.txt", "b");
        g(&repo, &["add", "b.txt"]);
        assert_eq!(GitCli.state(&repo).dirty, Some(true));
    }

    #[test]
    fn ignored_files_are_not_dirty() {
        let t = Tmp::new();
        let repo = t.sub("ign");
        init(&repo);
        commit_file(&repo, ".gitignore", "target/\n");
        write(&repo, "target/debug/x.o", "obj");
        assert_eq!(GitCli.state(&repo).dirty, Some(false));
    }

    #[test]
    fn unpushed_commits_with_bare_remote() {
        let t = Tmp::new();
        let (repo, _bare) = repo_with_remote(&t, true);
        let st = GitCli.state(&repo);
        assert_eq!(st.unpushed_commits, Some(0));
        assert_eq!(st.has_upstream, Some(true));
        assert_eq!(st.dirty, Some(false));

        commit_file(&repo, "b.txt", "b");
        commit_file(&repo, "c.txt", "c");
        let st = GitCli.state(&repo);
        assert_eq!(st.unpushed_commits, Some(2));
        assert_eq!(st.has_upstream, Some(true));

        g(&repo, &["push", "-q"]);
        assert_eq!(GitCli.state(&repo).unpushed_commits, Some(0));
    }

    #[test]
    fn pushed_but_no_upstream() {
        let t = Tmp::new();
        let (repo, _bare) = repo_with_remote(&t, false);
        let st = GitCli.state(&repo);
        assert_eq!(st.has_upstream, Some(false));
        assert_eq!(st.unpushed_commits, Some(0), "commit exists on origin/main");
        assert_eq!(st.branch.as_deref(), Some("main"));
    }

    #[test]
    fn local_only_branch_commit_counts_as_unpushed() {
        let t = Tmp::new();
        let (repo, _bare) = repo_with_remote(&t, true);
        g(&repo, &["checkout", "-q", "-b", "feature"]);
        commit_file(&repo, "f.txt", "f");
        let st = GitCli.state(&repo);
        assert_eq!(st.branch.as_deref(), Some("feature"));
        assert_eq!(st.has_upstream, Some(false));
        assert_eq!(st.unpushed_commits, Some(1));
    }

    #[test]
    fn detached_head() {
        let t = Tmp::new();
        let repo = t.sub("detached");
        init(&repo);
        commit_file(&repo, "a.txt", "a");
        g(&repo, &["checkout", "-q", "--detach"]);
        let st = GitCli.state(&repo);
        assert!(st.is_repo);
        assert_eq!(st.branch, None);
        assert_eq!(st.has_upstream, Some(false));
        assert_eq!(st.dirty, Some(false));
    }

    #[test]
    fn linked_worktree_reports_main_worktree() {
        let t = Tmp::new();
        // Space and non-ASCII on purpose: git quotes such paths in non-z output.
        let main = t.sub("m\u{e4}in repo");
        init(&main);
        commit_file(&main, "a.txt", "a");
        let wt = t.0.join("wt \u{e8} x");
        g(&main, &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "feat"]);

        let st = GitCli.state(&wt);
        assert!(st.is_repo);
        assert!(st.is_linked_worktree);
        assert_eq!(st.branch.as_deref(), Some("feat"));
        assert_eq!(
            path_key(&st.main_worktree.expect("main worktree")),
            path_key(&main)
        );

        let st = GitCli.state(&main);
        assert!(!st.is_linked_worktree);
        assert_eq!(st.main_worktree, None);
    }

    #[test]
    fn linked_worktree_from_subdirectory() {
        let t = Tmp::new();
        let main = t.sub("main");
        init(&main);
        commit_file(&main, "a.txt", "a");
        let wt = t.0.join("wt");
        g(&main, &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "feat"]);
        let sub = wt.join("deep").join("er");
        fs::create_dir_all(&sub).unwrap();
        let st = GitCli.state(&sub);
        assert!(st.is_linked_worktree);
        assert_eq!(path_key(&st.main_worktree.unwrap()), path_key(&main));
    }

    #[test]
    fn non_repo_dir_and_missing_dir() {
        let t = Tmp::new();
        let plain = t.sub("plain");
        let st = GitCli.state(&plain);
        assert!(!st.is_repo);
        assert_eq!(st.dirty, None);
        assert_eq!(st.unpushed_commits, None);
        assert_eq!(st.has_upstream, None);
        assert_eq!(st.branch, None);
        assert!(!st.is_linked_worktree);
        assert_eq!(st.main_worktree, None);
        assert_eq!(st.last_commit_ms, None);

        let st = GitCli.state(&t.0.join("does-not-exist"));
        assert!(!st.is_repo);

        assert!(!GitCli.is_tracked(&plain, Path::new("target")));
        assert_eq!(GitCli.is_tracked_checked(&plain, Path::new("target")), Some(false));
    }

    #[test]
    fn tracked_vs_ignored_target() {
        let t = Tmp::new();
        let repo = t.sub("proj");
        init(&repo);
        write(&repo, ".gitignore", "target/\n");
        write(&repo, "src/main.rs", "fn main() {}");
        write(&repo, "target/debug/app.exe", "bin");
        g(&repo, &["add", "-A"]);
        g(&repo, &["commit", "-q", "-m", "init"]);

        assert!(!GitCli.is_tracked(&repo, Path::new("target")), "ignored target/ is not tracked");
        assert!(GitCli.is_tracked(&repo, Path::new("src")));
        assert!(GitCli.is_tracked(&repo, Path::new("src/main.rs")));
        assert!(GitCli.is_tracked(&repo, Path::new(r"src\main.rs")), "backslash separators");
        assert!(!GitCli.is_tracked(&repo, Path::new("nope")));

        // Force-add something under target/: now it is tracked.
        g(&repo, &["add", "-f", "target/debug/app.exe"]);
        g(&repo, &["commit", "-q", "-m", "oops"]);
        assert!(GitCli.is_tracked(&repo, Path::new("target")));
        assert!(GitCli.is_tracked(&repo, Path::new("target/debug")));
    }

    #[test]
    fn is_ignored_three_outcomes() {
        let t = Tmp::new();
        let repo = t.sub("proj");
        init(&repo);
        write(&repo, ".gitignore", "dist/\n");
        std::fs::create_dir_all(repo.join("dist")).unwrap();
        std::fs::create_dir_all(repo.join("out")).unwrap();

        assert_eq!(GitCli.is_ignored(&repo, Path::new("dist")), Some(true));
        assert_eq!(GitCli.is_ignored(&repo, Path::new("out")), Some(false));

        let plain = t.sub("plain");
        assert_eq!(GitCli.is_ignored(&plain, Path::new("dist")), None);
    }

    #[test]
    fn path_with_spaces() {
        let t = Tmp::new();
        let repo = t.sub("my proj (copy)");
        init(&repo);
        commit_file(&repo, "sub dir/file name.txt", "x");
        write(&repo, "other dir/untracked.txt", "u");
        let st = GitCli.state(&repo);
        assert!(st.is_repo);
        assert_eq!(st.dirty, Some(true));
        assert!(GitCli.is_tracked(&repo, Path::new("sub dir")));
        assert!(GitCli.is_tracked(&repo, Path::new("sub dir/file name.txt")));
        assert!(!GitCli.is_tracked(&repo, Path::new("other dir")));
    }

    #[test]
    fn pathspec_is_literal() {
        let t = Tmp::new();
        let repo = t.sub("lit");
        init(&repo);
        commit_file(&repo, "a.txt", "a");
        assert!(!GitCli.is_tracked(&repo, Path::new("*.txt")));
        assert!(!GitCli.is_tracked(&repo, Path::new(":(top)a.txt")));
    }

    #[test]
    fn unquote_c_style() {
        assert_eq!(unquote_c("plain"), "plain");
        assert_eq!(unquote_c(r#""caff\303\250 x""#), "caff\u{e8} x");
        assert_eq!(unquote_c(r#""a\\b\"c\td""#), "a\\b\"c\td");
    }

    #[cfg(windows)]
    #[test]
    fn timeout_kills_child_and_returns_none() {
        let mut c = Command::new("ping");
        c.args(["-n", "30", "127.0.0.1"]).creation_flags(CREATE_NO_WINDOW);
        let t0 = Instant::now();
        assert!(run_cmd(c, Duration::from_millis(300)).is_none());
        assert!(t0.elapsed() < Duration::from_secs(5), "took {:?}", t0.elapsed());
    }

    #[test]
    fn missing_binary_is_none_not_panic() {
        let c = Command::new("definitely-not-a-real-binary-dc-git");
        assert!(run_cmd(c, Duration::from_secs(1)).is_none());
    }
}
