//! Git queries used by the project cleanup engine: worktree list, repository identity
//! (root commits, origin), "is this HEAD kept somewhere else", untracked files, stale branches.
//!
//! Same rules as the rest of the crate: every call goes through `git()` (timeout, no console
//! window, literal pathspecs) and a failure is an empty / `None` answer, never a panic.

use crate::{git, main_worktree, unquote_c};
use xiox_model::normalize_path;
use std::path::{Path, PathBuf};

/// One entry of `git worktree list --porcelain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// Normalized (`xiox_model::normalize_path`).
    pub path: PathBuf,
    /// Full sha; `None` when the worktree has no commit yet.
    pub head: Option<String>,
    /// Short branch name; `None` when detached.
    pub branch: Option<String>,
    /// First entry of the list.
    pub is_main: bool,
    /// Git marks it prunable, or the folder does not exist.
    pub prunable: bool,
}

/// A local branch that looks stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    /// Merged into the main worktree's current branch.
    pub merged: bool,
    /// Checked out in a worktree that is prunable (folder gone).
    pub worktree_missing: bool,
}

/// Main working tree of the repository `path` is in (`path` may be a subfolder or a linked worktree).
pub fn repo_main_root(path: &Path) -> Option<PathBuf> {
    main_worktree(path)
}

/// `git worktree list --porcelain -z` from `repo` (fallback without `-z` as `main_worktree` does).
pub fn list_worktrees(repo: &Path) -> Option<Vec<WorktreeEntry>> {
    let fields: Vec<String> = if let Some(o) = git(repo, &["worktree", "list", "--porcelain", "-z"]).filter(|o| o.ok) {
        o.stdout
            .split(|b| *b == 0)
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .collect()
    } else {
        let o = git(repo, &["worktree", "list", "--porcelain"]).filter(|o| o.ok)?;
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| match l.strip_prefix("worktree ") {
                Some(p) => format!("worktree {}", unquote_c(p)),
                None => l.to_string(),
            })
            .collect()
    };
    let entries = parse_worktrees(&fields);
    (!entries.is_empty()).then_some(entries)
}

/// Fields are the porcelain lines; an empty field closes the current entry.
fn parse_worktrees(fields: &[String]) -> Vec<WorktreeEntry> {
    let mut out: Vec<WorktreeEntry> = Vec::new();
    let mut cur: Option<WorktreeEntry> = None;
    let flush = |cur: &mut Option<WorktreeEntry>, out: &mut Vec<WorktreeEntry>| {
        if let Some(mut e) = cur.take() {
            e.is_main = out.is_empty();
            if !e.path.exists() {
                e.prunable = true;
            }
            out.push(e);
        }
    };
    for f in fields {
        let f = f.trim_end_matches('\r');
        if f.is_empty() {
            flush(&mut cur, &mut out);
            continue;
        }
        let (key, value) = f.split_once(' ').unwrap_or((f, ""));
        match key {
            "worktree" => {
                flush(&mut cur, &mut out);
                cur = Some(WorktreeEntry {
                    path: normalize_path(Path::new(value)),
                    head: None,
                    branch: None,
                    is_main: false,
                    prunable: false,
                });
            }
            "HEAD" => {
                if let Some(e) = cur.as_mut() {
                    // An unborn branch reports an all-zero id.
                    e.head = (!value.is_empty() && value.bytes().any(|b| b != b'0')).then(|| value.to_string());
                }
            }
            "branch" => {
                if let Some(e) = cur.as_mut() {
                    e.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_string());
                }
            }
            "prunable" => {
                if let Some(e) = cur.as_mut() {
                    e.prunable = true;
                }
            }
            _ => {}
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// `git rev-list --max-parents=0 HEAD`, sorted. Empty when there is no commit.
pub fn root_commits(repo: &Path) -> Vec<String> {
    let Some(o) = git(repo, &["rev-list", "--max-parents=0", "HEAD"]).filter(|o| o.ok) else {
        return Vec::new();
    };
    let mut v: Vec<String> = o
        .text()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    v.sort();
    v
}

pub fn origin_url(repo: &Path) -> Option<String> {
    let o = git(repo, &["config", "--get", "remote.origin.url"]).filter(|o| o.ok)?;
    let t = o.text();
    (!t.is_empty()).then_some(t)
}

/// Lowercase, no trailing `.git` or `/`, `git@host:a/b` and `ssh://git@host/a/b` and `https://host/a/b` -> `host/a/b`.
pub fn normalize_remote(url: &str) -> String {
    let mut s = url.trim().to_lowercase().replace('\\', "/");
    while s.ends_with('/') {
        s.pop();
    }
    if let Some(r) = s.strip_suffix(".git") {
        s = r.to_string();
    }
    while s.ends_with('/') {
        s.pop();
    }
    if let Some((_, rest)) = s.split_once("://") {
        // scheme://[user@]host[:port]/path
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host.split(':').next().unwrap_or(host);
        return if path.is_empty() { host.to_string() } else { format!("{host}/{path}") };
    }
    // scp-like `[user@]host:path`; a one-letter "host" is a Windows drive, left alone.
    if let Some((left, path)) = s.split_once(':')
        && !left.contains('/')
        && left.len() > 1
    {
        let host = left.rsplit_once('@').map_or(left, |(_, h)| h);
        return format!("{host}/{}", path.trim_start_matches('/'));
    }
    s
}

/// Some(true) when HEAD of `worktree` is contained in a remote branch or in a local branch other than
/// the one checked out there (`git branch -a --contains HEAD`); None when git cannot answer or no HEAD.
pub fn head_kept_elsewhere(worktree: &Path) -> Option<bool> {
    git(worktree, &["rev-parse", "--verify", "-q", "HEAD"]).filter(|o| o.ok)?;
    let current = git(worktree, &["branch", "--show-current"])
        .filter(|o| o.ok)
        .map(|o| o.text())
        .filter(|b| !b.is_empty());
    let o = git(worktree, &["branch", "-a", "--contains", "HEAD", "--format=%(refname)"]).filter(|o| o.ok)?;
    let text = o.text();
    Some(text.lines().map(str::trim).any(|r| {
        r.starts_with("refs/remotes/")
            || r.strip_prefix("refs/heads/").is_some_and(|b| Some(b) != current.as_deref())
    }))
}

/// Untracked files relative to `root`: `ignored = true` -> `git ls-files -z --others --ignored --exclude-standard`,
/// `false` -> `git ls-files -z --others --exclude-standard`. None when not a repository.
pub fn untracked_files(root: &Path, ignored: bool) -> Option<Vec<PathBuf>> {
    let mut args = vec!["ls-files", "-z", "--others"];
    if ignored {
        args.push("--ignored");
    }
    args.push("--exclude-standard");
    let o = git(root, &args).filter(|o| o.ok)?;
    // A nested repository (or submodule folder) is listed as a directory entry (`inner/`):
    // files only, never a folder that could be taken for a file and removed as a whole.
    Some(
        o.stdout
            .split(|b| *b == 0)
            .filter(|f| !f.is_empty() && !matches!(f.last(), Some(b'/' | b'\\')))
            .map(|f| PathBuf::from(String::from_utf8_lossy(f).into_owned()))
            .collect(),
    )
}

/// `url` without the credentials of its authority (`https://user:token@host/a/b` becomes
/// `https://host/a/b`). Anything that is not a `scheme://authority/...` URL (an scp-like
/// `git@host:a/b`, a local path) is returned as it is: no secret travels in those.
pub fn strip_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.to_string() };
    let end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    match authority.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://{host}{tail}"),
        None => url.to_string(),
    }
}

/// `Some(true)` when `dir` has uncommitted changes or untracked files (what `git status` shows),
/// `None` when git cannot read the folder (not a repository, `dubious ownership`, no git).
pub fn working_tree_dirty(dir: &Path) -> Option<bool> {
    let o = git(dir, &["status", "--porcelain=v1", "--untracked-files=normal"]).filter(|o| o.ok)?;
    Some(o.stdout.iter().any(|b| !b.is_ascii_whitespace()))
}

/// The commits of `repo` that would be lost with it: HEAD, every local branch, every tag and
/// every stash entry, sorted, without duplicates. `Some(empty)` for a repository without a commit,
/// `None` when git cannot list them.
pub fn work_tips(repo: &Path) -> Option<Vec<String>> {
    let refs = git(repo, &["for-each-ref", "--format=%(objectname)", "refs/heads", "refs/tags"]).filter(|o| o.ok)?;
    let mut tips: Vec<String> = refs.text().lines().map(|l| l.trim().to_string()).collect();
    if let Some(head) = git(repo, &["rev-parse", "--verify", "-q", "HEAD"]).filter(|o| o.ok) {
        tips.push(head.text());
    }
    // `refs/stash` keeps only the newest entry; the older ones live in its reflog.
    let stash = git(repo, &["stash", "list", "--format=%H"]).filter(|o| o.ok)?;
    tips.extend(stash.text().lines().map(|l| l.trim().to_string()));
    tips.retain(|t| !t.is_empty());
    tips.sort();
    tips.dedup();
    Some(tips)
}

/// How many tips go on one command line (a Windows command line holds about 32 000 characters).
const TIPS_PER_CALL: usize = 200;

/// `Some(true)` when at least one of `tips` is not reachable from any ref (or HEAD) of `repo`:
/// the commit is not in `repo` at all, or only as a dangling object (a deleted branch, a rebase).
/// `Some(false)` when every tip is reachable (also when there is no tip). `None` when git cannot
/// answer. `tips` come from another repository: that is the point.
pub fn tips_missing_from_refs(repo: &Path, tips: &[String]) -> Option<bool> {
    unreached(repo, tips, "--all")
}

/// `Some(true)` when at least one of `tips` (of `repo` itself) is not reachable from a
/// remote-tracking branch: the commit exists nowhere but here, as far as `repo` knows.
pub fn tips_not_pushed(repo: &Path, tips: &[String]) -> Option<bool> {
    unreached(repo, tips, "--remotes")
}

/// `git rev-list -n1 --missing=print <tips> --not <scope>`: any output (a commit, or `?<sha>` for
/// an object `repo` does not have) is a tip that `scope` does not reach.
fn unreached(repo: &Path, tips: &[String], scope: &str) -> Option<bool> {
    for chunk in tips.chunks(TIPS_PER_CALL) {
        let mut args: Vec<&str> = vec!["rev-list", "-n1", "--missing=print"];
        args.extend(chunk.iter().map(String::as_str));
        args.extend(["--not", scope]);
        let o = git(repo, &args).filter(|o| o.ok)?;
        if !o.text().is_empty() {
            return Some(true);
        }
    }
    Some(false)
}

/// Local branches other than the main worktree's current one that are merged into it
/// (`git branch --merged`) or checked out in a prunable worktree.
pub fn stale_branches(repo: &Path) -> Vec<BranchInfo> {
    let Some(main) = repo_main_root(repo) else {
        return Vec::new();
    };
    let heads = |args: &[&str]| -> Option<Vec<String>> {
        let o = git(&main, args).filter(|o| o.ok)?;
        Some(
            o.text()
                .lines()
                .filter_map(|l| l.trim().strip_prefix("refs/heads/").map(str::to_string))
                .collect(),
        )
    };
    let Some(all) = heads(&["for-each-ref", "--format=%(refname)", "refs/heads"]) else {
        return Vec::new();
    };
    let merged = heads(&["branch", "--format=%(refname)", "--merged", "HEAD"]).unwrap_or_default();
    let current = git(&main, &["branch", "--show-current"])
        .filter(|o| o.ok)
        .map(|o| o.text())
        .filter(|b| !b.is_empty());
    let missing: Vec<String> = list_worktrees(&main)
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w.prunable)
        .filter_map(|w| w.branch)
        .collect();

    let mut out: Vec<BranchInfo> = all
        .into_iter()
        .filter(|b| Some(b.as_str()) != current.as_deref())
        .map(|name| BranchInfo {
            merged: merged.contains(&name),
            worktree_missing: missing.contains(&name),
            name,
        })
        .filter(|b| b.merged || b.worktree_missing)
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{Tmp, commit_file, g, init, write};
    use xiox_model::path_key;
    use std::fs;

    #[test]
    fn main_root_from_subfolder_and_from_linked_worktree() {
        let t = Tmp::new();
        let main = t.sub("main");
        init(&main);
        commit_file(&main, "a.txt", "a");
        let wt = t.0.join("wt");
        g(&main, &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "feat"]);
        let sub = main.join("deep").join("er");
        fs::create_dir_all(&sub).unwrap();
        let wsub = wt.join("x");
        fs::create_dir_all(&wsub).unwrap();

        for p in [&main, &sub, &wt, &wsub] {
            let r = repo_main_root(p).unwrap_or_else(|| panic!("no root for {p:?}"));
            assert_eq!(path_key(&r), path_key(&main), "from {p:?}");
        }
    }

    #[test]
    fn worktrees_listed_with_main_first_branch_and_detached() {
        let t = Tmp::new();
        let main = t.sub("main");
        init(&main);
        commit_file(&main, "a.txt", "a");
        let feat = t.0.join("feat-wt");
        let det = t.0.join("det-wt");
        g(&main, &["worktree", "add", "-q", &feat.to_string_lossy(), "-b", "feat"]);
        g(&main, &["worktree", "add", "-q", "--detach", &det.to_string_lossy()]);
        let sha = g(&main, &["rev-parse", "HEAD"]);

        let list = list_worktrees(&feat).expect("list");
        assert_eq!(list.len(), 3, "{list:?}");
        assert!(list[0].is_main);
        assert!(list[1..].iter().all(|e| !e.is_main));
        assert_eq!(path_key(&list[0].path), path_key(&main));
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(list[0].head.as_deref(), Some(sha.as_str()));

        let f = list.iter().find(|e| path_key(&e.path) == path_key(&feat)).expect("feat entry");
        assert_eq!(f.branch.as_deref(), Some("feat"));
        assert!(!f.prunable);
        let d = list.iter().find(|e| path_key(&e.path) == path_key(&det)).expect("detached entry");
        assert_eq!(d.branch, None);
        assert_eq!(d.head.as_deref(), Some(sha.as_str()));
        assert!(!d.prunable);
    }

    #[test]
    fn worktree_whose_folder_was_deleted_is_prunable() {
        let t = Tmp::new();
        let main = t.sub("main");
        init(&main);
        commit_file(&main, "a.txt", "a");
        let wt = t.0.join("gone");
        g(&main, &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "feat"]);
        fs::remove_dir_all(&wt).unwrap();

        let list = list_worktrees(&main).expect("list");
        assert_eq!(list.len(), 2);
        assert!(!list[0].prunable);
        let e = list.iter().find(|e| path_key(&e.path) == path_key(&wt)).expect("entry kept");
        assert!(e.prunable);
        assert_eq!(e.branch.as_deref(), Some("feat"));
    }

    #[test]
    fn root_commits_equal_for_a_clone_and_differ_for_an_unrelated_repo() {
        let t = Tmp::new();
        let a = t.sub("a");
        init(&a);
        commit_file(&a, "a.txt", "a");
        commit_file(&a, "b.txt", "b");
        let clone = t.0.join("clone");
        g(&t.0, &["clone", "-q", &a.to_string_lossy(), &clone.to_string_lossy()]);
        let other = t.sub("other");
        init(&other);
        commit_file(&other, "z.txt", "z");

        let ra = root_commits(&a);
        assert_eq!(ra.len(), 1);
        assert_eq!(ra, root_commits(&clone));
        assert_ne!(ra, root_commits(&other));
        // Unborn repository: no commit, empty answer.
        let empty = t.sub("empty");
        init(&empty);
        assert!(root_commits(&empty).is_empty());
    }

    #[test]
    fn origin_url_is_read_when_set() {
        let t = Tmp::new();
        let a = t.sub("a");
        init(&a);
        assert_eq!(origin_url(&a), None);
        g(&a, &["remote", "add", "origin", "git@GitHub.com:Acme/Mukade.git"]);
        assert_eq!(origin_url(&a).as_deref(), Some("git@GitHub.com:Acme/Mukade.git"));
    }

    #[test]
    fn normalize_remote_forms() {
        for u in [
            "git@GitHub.com:Acme/Mukade.git",
            "https://github.com/acme/mukade/",
            "ssh://git@github.com/acme/mukade.git",
        ] {
            assert_eq!(normalize_remote(u), "github.com/acme/mukade");
        }
        assert_eq!(
            normalize_remote("https://user:pw@GitHub.com:8443/Acme/Mukade.git"),
            "github.com/acme/mukade"
        );
    }

    #[test]
    fn head_kept_elsewhere_true_for_detached_at_main_false_for_a_new_commit_on_detached() {
        let t = Tmp::new();
        let main = t.sub("main");
        init(&main);
        commit_file(&main, "a.txt", "a");
        let det = t.0.join("det");
        g(&main, &["worktree", "add", "-q", "--detach", &det.to_string_lossy()]);
        assert_eq!(head_kept_elsewhere(&det), Some(true));
        // The main worktree itself: `main` is the checked out branch, nothing else holds HEAD.
        assert_eq!(head_kept_elsewhere(&main), Some(false));

        commit_file(&det, "b.txt", "b");
        assert_eq!(head_kept_elsewhere(&det), Some(false));
    }

    #[test]
    fn head_kept_elsewhere_false_for_a_branch_with_its_own_unmerged_commit() {
        let t = Tmp::new();
        let main = t.sub("main");
        init(&main);
        commit_file(&main, "a.txt", "a");
        let wt = t.0.join("wt");
        g(&main, &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "feat"]);
        // Fresh branch at the same commit as `main`: kept by `main`.
        assert_eq!(head_kept_elsewhere(&wt), Some(true));
        commit_file(&wt, "b.txt", "b");
        assert_eq!(head_kept_elsewhere(&wt), Some(false));
        // A remote branch holding the commit makes it kept.
        let bare = t.0.join("bare.git");
        g(&t.0, &["init", "-q", "--bare", &bare.to_string_lossy()]);
        g(&wt, &["remote", "add", "origin", &bare.to_string_lossy()]);
        g(&wt, &["push", "-q", "origin", "feat"]);
        assert_eq!(head_kept_elsewhere(&wt), Some(true));
    }

    #[test]
    fn untracked_files_split_ignored_from_not_ignored() {
        let t = Tmp::new();
        let r = t.sub("r");
        init(&r);
        write(&r, ".gitignore", "*.log\n");
        commit_file(&r, "tracked.txt", "t"); // `add -A` commits .gitignore too
        write(&r, "a.log", "x");
        write(&r, "sub/b.tmp", "y");

        let ignored = untracked_files(&r, true).expect("repo");
        assert_eq!(ignored, vec![PathBuf::from("a.log")]);
        let mut plain = untracked_files(&r, false).expect("repo");
        plain.sort();
        assert_eq!(plain, vec![PathBuf::from("sub/b.tmp")]);
    }

    #[test]
    fn untracked_files_never_returns_a_nested_repository() {
        let t = Tmp::new();
        let r = t.sub("r");
        init(&r);
        write(&r, ".gitignore", "*.log\nign/\n");
        commit_file(&r, "tracked.txt", "t");
        write(&r, "a.log", "x");
        write(&r, "plain.tmp", "y");
        for name in ["inner", "ign"] {
            let nested = r.join(name);
            fs::create_dir_all(&nested).unwrap();
            init(&nested);
            commit_file(&nested, "n.txt", "n");
        }

        for ignored in [false, true] {
            let v = untracked_files(&r, ignored).expect("repo");
            let shown: Vec<String> = v.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            assert!(
                shown.iter().all(|s| !s.ends_with('/') && !s.ends_with('\\')),
                "ignored={ignored}: directory entry returned: {shown:?}"
            );
            assert!(
                shown.iter().all(|s| !s.starts_with("inner") && !s.starts_with("ign")),
                "ignored={ignored}: nested repository content returned: {shown:?}"
            );
        }
        let plain = untracked_files(&r, false).expect("repo");
        assert!(plain.contains(&PathBuf::from("plain.tmp")));
        let ign = untracked_files(&r, true).expect("repo");
        assert!(ign.contains(&PathBuf::from("a.log")));
    }

    #[test]
    fn stale_branches_lists_merged_and_skips_unmerged() {
        let t = Tmp::new();
        let main = t.sub("main");
        init(&main);
        commit_file(&main, "a.txt", "a");
        g(&main, &["branch", "done"]);
        g(&main, &["checkout", "-q", "-b", "wip"]);
        commit_file(&main, "w.txt", "w");
        g(&main, &["checkout", "-q", "main"]);

        let v = stale_branches(&main);
        assert_eq!(
            v,
            vec![BranchInfo {
                name: "done".into(),
                merged: true,
                worktree_missing: false
            }]
        );
    }

    #[test]
    fn stale_branches_includes_the_branch_of_a_vanished_worktree() {
        let t = Tmp::new();
        let main = t.sub("main");
        init(&main);
        commit_file(&main, "a.txt", "a");
        let wt = t.0.join("wt");
        g(&main, &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "feat"]);
        commit_file(&wt, "f.txt", "f");
        fs::remove_dir_all(&wt).unwrap();

        // Asked from a subfolder of main: the answer is about the whole repository.
        let sub = main.join("s");
        fs::create_dir_all(&sub).unwrap();
        let v = stale_branches(&sub);
        assert_eq!(
            v,
            vec![BranchInfo {
                name: "feat".into(),
                merged: false,
                worktree_missing: true
            }]
        );
    }

    #[test]
    fn not_a_repository_gives_none_everywhere() {
        let t = Tmp::new();
        let d = t.sub("plain");
        assert_eq!(repo_main_root(&d), None);
        assert_eq!(list_worktrees(&d), None);
        assert!(root_commits(&d).is_empty());
        assert_eq!(origin_url(&d), None);
        assert_eq!(head_kept_elsewhere(&d), None);
        assert_eq!(untracked_files(&d, true), None);
        assert_eq!(untracked_files(&d, false), None);
        assert!(stale_branches(&d).is_empty());
        // Unborn repository: no HEAD, git cannot answer.
        let e = t.sub("unborn");
        init(&e);
        assert_eq!(head_kept_elsewhere(&e), None);
    }

    // ── the copy checks of `xiox` ──

    #[test]
    fn credentials_are_cut_from_a_remote_url_and_nothing_else_is() {
        assert_eq!(strip_credentials("https://user:tok3n@github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(strip_credentials("https://tok3n@github.com/a/b"), "https://github.com/a/b");
        assert_eq!(strip_credentials("ssh://git@host:22/a/b"), "ssh://host:22/a/b");
        assert_eq!(strip_credentials("git@github.com:a/b.git"), "git@github.com:a/b.git");
        assert_eq!(strip_credentials("https://github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(strip_credentials(r"D:\repos\b"), r"D:\repos\b");
        // An '@' after the host is part of the path, not a user.
        assert_eq!(strip_credentials("https://host/a/b@c"), "https://host/a/b@c");
        // The identity of a remote does not change.
        let url = "https://user:tok3n@github.com/a/b.git";
        assert_eq!(normalize_remote(url), normalize_remote(&strip_credentials(url)));
    }

    /// A project with one commit and a clone of it.
    fn project_and_clone(t: &Tmp) -> (PathBuf, PathBuf) {
        let p = t.sub("proj");
        init(&p);
        commit_file(&p, "a.txt", "a");
        let c = t.0.join("copy");
        g(&t.0, &["clone", "-q", &p.to_string_lossy(), &c.to_string_lossy()]);
        (p, c)
    }

    #[test]
    fn work_tips_are_head_branches_tags_and_every_stash_entry() {
        let t = Tmp::new();
        let (_p, c) = project_and_clone(&t);
        let base = work_tips(&c).expect("tips");
        assert_eq!(base, vec![g(&c, &["rev-parse", "HEAD"])], "a clean clone has one tip: HEAD (= main)");

        g(&c, &["branch", "side"]);
        g(&c, &["tag", "v1"]);
        write(&c, "a.txt", "changed");
        g(&c, &["stash", "push", "-q"]);
        write(&c, "a.txt", "changed again");
        g(&c, &["stash", "push", "-q"]);
        let tips = work_tips(&c).expect("tips");
        let stashes: Vec<String> = g(&c, &["stash", "list", "--format=%H"]).lines().map(str::to_string).collect();
        assert_eq!(stashes.len(), 2);
        for s in &stashes {
            assert!(tips.contains(s), "stash {s} missing from {tips:?}");
        }
        assert!(tips.contains(&g(&c, &["rev-parse", "HEAD"])));
        assert!(tips.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates: {tips:?}");
    }

    #[test]
    fn work_tips_of_an_unborn_repository_is_empty_and_of_a_plain_folder_is_none() {
        let t = Tmp::new();
        let e = t.sub("unborn");
        init(&e);
        assert_eq!(work_tips(&e), Some(Vec::new()));
        assert_eq!(work_tips(&t.sub("plain")), None);
    }

    #[test]
    fn tips_missing_from_refs_sees_private_commits_stashes_tags_and_dangling_objects() {
        let t = Tmp::new();
        let (p, c) = project_and_clone(&t);
        // A clean clone: every tip is in the project.
        let tips = work_tips(&c).unwrap();
        assert_eq!(tips_missing_from_refs(&p, &tips), Some(false));
        // Nothing to check: nothing is missing.
        assert_eq!(tips_missing_from_refs(&p, &[]), Some(false));

        // A stash whose commits the project has never seen.
        write(&c, "wip.txt", "wip");
        g(&c, &["add", "wip.txt"]);
        g(&c, &["stash", "push", "-q"]);
        assert_eq!(tips_missing_from_refs(&p, &work_tips(&c).unwrap()), Some(true), "stash");
        g(&c, &["stash", "drop", "-q"]);
        assert_eq!(tips_missing_from_refs(&p, &work_tips(&c).unwrap()), Some(false));

        // A tag on a commit no branch of the copy has any more.
        commit_file(&c, "private.txt", "x");
        g(&c, &["tag", "keep"]);
        g(&c, &["reset", "-q", "--hard", "HEAD~1"]);
        let tips = work_tips(&c).unwrap();
        assert_eq!(tips_missing_from_refs(&p, &tips), Some(true), "tag");
        g(&c, &["tag", "-d", "keep"]);

        // A branch that is only in the copy.
        g(&c, &["checkout", "-q", "-b", "mine"]);
        commit_file(&c, "mine.txt", "x");
        assert_eq!(tips_missing_from_refs(&p, &work_tips(&c).unwrap()), Some(true), "branch");

        // The project has the object but no ref reaches it (a deleted branch): still unsaved work.
        g(&p, &["checkout", "-q", "-b", "temp"]);
        commit_file(&p, "temp.txt", "x");
        let temp_tip = g(&p, &["rev-parse", "HEAD"]);
        g(&p, &["checkout", "-q", "main"]);
        g(&p, &["branch", "-D", "temp"]);
        assert_eq!(tips_missing_from_refs(&p, &[temp_tip]), Some(true), "dangling object");

        // git cannot answer in a folder that is not a repository.
        assert_eq!(tips_missing_from_refs(&t.sub("plain"), &[g(&c, &["rev-parse", "HEAD"])]), None);
    }

    #[test]
    fn tips_missing_from_refs_copes_with_thousands_of_tips() {
        let t = Tmp::new();
        let (p, c) = project_and_clone(&t);
        let sha = g(&c, &["rev-parse", "HEAD"]);
        let many = vec![sha; 3000];
        assert_eq!(tips_missing_from_refs(&p, &many), Some(false));
    }

    #[test]
    fn tips_not_pushed_looks_at_the_remotes_of_the_repository_itself() {
        let t = Tmp::new();
        let (repo, _bare) = repo_with_remote_for_tests(&t);
        assert_eq!(tips_not_pushed(&repo, &work_tips(&repo).unwrap()), Some(false));
        commit_file(&repo, "local.txt", "x");
        assert_eq!(tips_not_pushed(&repo, &work_tips(&repo).unwrap()), Some(true));
        // No remote at all: everything is unpushed.
        let lone = t.sub("lone");
        init(&lone);
        commit_file(&lone, "a.txt", "a");
        assert_eq!(tips_not_pushed(&lone, &work_tips(&lone).unwrap()), Some(true));
    }

    fn repo_with_remote_for_tests(t: &Tmp) -> (PathBuf, PathBuf) {
        let bare = t.sub("remote.git");
        g(&bare, &["init", "-q", "--bare", "-b", "main"]);
        let repo = t.sub("work");
        init(&repo);
        commit_file(&repo, "a.txt", "a");
        g(&repo, &["remote", "add", "origin", &bare.to_string_lossy()]);
        g(&repo, &["push", "-q", "-u", "origin", "main"]);
        (repo, bare)
    }

    #[test]
    fn working_tree_dirty_tells_clean_dirty_and_unreadable() {
        let t = Tmp::new();
        let (_p, c) = project_and_clone(&t);
        assert_eq!(working_tree_dirty(&c), Some(false));
        write(&c, "new.txt", "x");
        assert_eq!(working_tree_dirty(&c), Some(true));
        assert_eq!(working_tree_dirty(&t.sub("plain")), None);
    }
}
