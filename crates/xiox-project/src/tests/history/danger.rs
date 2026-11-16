//! The risks of each item by category, and the `danger` level they give.

use std::fs;
use std::path::{Path, PathBuf};

use xiox_model::{path_key, Agent};
use xiox_mukade::ToolCall;

use super::fake::{bash, descriptor, input, listing, FakeCalls};
use crate::history::{UserDirs, BUDGET};
use crate::tests::fixtures::*;
use crate::*;

const DAY: i64 = 24 * 60 * 60 * 1000;

/// The clock of the "old" cases: everything the test made is three days older.
fn later() -> i64 {
    xiox_app::now_ms() + 3 * DAY
}

struct Bench {
    _guard: tempfile::TempDir,
    base: PathBuf,
    project: PathBuf,
    user: UserDirs,
}

impl Bench {
    fn new() -> Bench {
        let (guard, base) = tmp();
        let project = repo(&base.join("work").join("phazly"));
        let home = base.join("home");
        fs::create_dir_all(&home).unwrap();
        let user = UserDirs { home: home.clone(), temp: home.join("AppData").join("Local").join("Temp"), resolved: true };
        Bench { _guard: guard, base, project, user }
    }

    fn at(&self, rel: &str) -> PathBuf {
        self.base.join(rel)
    }

    fn mkdir(&self, rel: &str) -> ToolCall {
        let dir = self.at(rel);
        fs::create_dir_all(&dir).unwrap();
        bash(&format!("mkdir -p {}", dir.to_string_lossy().replace('\\', "/")), &self.project.to_string_lossy())
    }

    fn write(&self, rel: &str) -> ToolCall {
        let file = self.at(rel);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        bash(&format!("echo x > {}", file.to_string_lossy().replace('\\', "/")), &self.project.to_string_lossy())
    }

    /// The analysis at `now`, with one session of the project that made `calls`.
    fn analyze(&self, calls: Vec<ToolCall>, now_ms: i64, history: bool) -> ProjectScan {
        let cwd = self.project.to_string_lossy().into_owned();
        let listings = [listing(Agent::Claude, vec![descriptor(Agent::Claude, "main", "s1", &cwd, "2026-11-10T10:00:00Z", None)])];
        let source = FakeCalls::default().answer("main", "s1", Ok(calls));
        let env = Env {
            search_roots: Some(vec![self.base.clone()]),
            now_ms,
            history: input(&listings, Some(&source), &self.user),
            ..env()
        };
        let opts = Options { history, ..Options::default() };
        crate::owned::analyze_with(&self.project, &opts, &env, BUDGET, &[]).unwrap()
    }
}

fn find<'a>(scan: &'a ProjectScan, dir: &Path) -> &'a Item {
    let key = path_key(dir);
    scan.items.iter().find(|i| i.key == key).unwrap_or_else(|| panic!("{key} not listed: {:?}", scan.items))
}

#[test]
fn danger_follows_the_risks_of_each_category() {
    use Risk::*;
    // (name, prepare: returns the calls and the folder to look at, clock, risks, level)
    type Prepare = fn(&Bench) -> (Vec<ToolCall>, PathBuf);
    let table: Vec<(&str, Prepare, bool, Vec<Risk>, Level)> = vec![
        ("scratch made, no git", |b| (vec![b.mkdir("agent-runs/r1")], b.at("agent-runs/r1")), false, vec![NoGit], Level::Danger),
        (
            "scratch made, with git",
            |b| {
                let calls = vec![b.mkdir("agent-runs/r2")];
                repo(&b.at("agent-runs/r2"));
                (calls, b.at("agent-runs/r2"))
            },
            false,
            vec![],
            Level::Caution,
        ),
        ("scratch only written", |b| (vec![b.write("agent-runs/r3/a.txt")], b.at("agent-runs/r3")), false, vec![NoGit, WeakEvidence], Level::Danger),
        (
            "named without git or provenance",
            |b| {
                fs::create_dir_all(b.at("work/phazly-old")).unwrap();
                (vec![], b.at("work/phazly-old"))
            },
            false,
            vec![NoGit],
            Level::Danger,
        ),
        (
            "named with git",
            |b| {
                repo(&b.at("work/phazly-other"));
                (vec![], b.at("work/phazly-other"))
            },
            false,
            vec![],
            Level::Caution,
        ),
        (
            "a copy changed now",
            |b| {
                clone(&b.project, &b.at("work/c1"));
                (vec![], b.at("work/c1"))
            },
            true,
            vec![RecentActivity],
            Level::Danger,
        ),
        (
            "an old copy",
            |b| {
                clone(&b.project, &b.at("work/c2"));
                (vec![], b.at("work/c2"))
            },
            false,
            vec![],
            Level::Caution,
        ),
        (
            "a worktree changed now",
            |b| (vec![], worktree(&b.project, &b.at("wt/one"), "one")),
            true,
            vec![RecentActivity],
            Level::Danger,
        ),
        ("an old worktree", |b| (vec![], worktree(&b.project, &b.at("wt/two"), "two")), false, vec![], Level::Recommended),
        ("a build folder", |b| (vec![], rust_build(&b.project, 2 * MB)), true, vec![], Level::Recommended),
    ];
    for (name, prepare, at_now, risks, level) in table {
        let bench = Bench::new();
        let (calls, dir) = prepare(&bench);
        let scan = bench.analyze(calls, if at_now { xiox_app::now_ms() } else { later() }, true);
        let item = find(&scan, &dir);
        assert_eq!((&item.risks, item.level), (&risks, level), "{name}");
    }
}

#[test]
fn phazly_example_is_danger() {
    let bench = Bench::new();
    let calls = vec![bench.mkdir("agent-runs/planner-benchmark-42"), bench.write("agent-runs/planner-benchmark-42/benchmark.py")];
    let scan = bench.analyze(calls, later(), true);
    let item = find(&scan, &bench.at("agent-runs/planner-benchmark-42"));
    assert_eq!((item.category, item.level), (Category::Scratch, Level::Danger));
    assert!(item.risks.contains(&Risk::NoGit), "{:?}", item.risks);
    assert_eq!(item.provenance.as_ref().unwrap().sessions.len(), 1);
}

#[test]
fn recent_activity_inside_reaches_the_container() {
    let bench = Bench::new();
    let named = bench.at("work/phazly-box");
    fs::create_dir_all(&named).unwrap();
    let copy = clone(&bench.project, &named.join("c"));
    let scan = bench.analyze(vec![], xiox_app::now_ms(), true);
    assert!(find(&scan, &copy).risks.contains(&Risk::RecentActivity));
    assert!(find(&scan, &named).risks.contains(&Risk::RecentActivity), "{:?}", find(&scan, &named).risks);
}

#[test]
fn without_history_named_without_git_is_still_danger() {
    let bench = Bench::new();
    let named = bench.at("work/phazly-old");
    fs::create_dir_all(&named).unwrap();
    let scan = bench.analyze(vec![], later(), false);
    let item = find(&scan, &named);
    assert_eq!((&item.risks, item.level), (&vec![Risk::NoGit], Level::Danger));
}
