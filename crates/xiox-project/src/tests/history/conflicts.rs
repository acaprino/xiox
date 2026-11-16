//! Sessions of other projects that wrote in a candidate: the folder is not this project's alone.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use xiox_model::{path_key, Agent};
use xiox_mukade::ToolCall;

use super::fake::{bash, call, descriptor, input, listing, FakeCalls};
use crate::history::{Budget, UserDirs, BUDGET};
use crate::tests::fixtures::*;
use crate::*;

struct World {
    _guard: tempfile::TempDir,
    base: PathBuf,
    project: PathBuf,
    user: UserDirs,
}

impl World {
    fn new() -> World {
        let (guard, base) = tmp();
        let project = repo(&base.join("work").join("proj"));
        fs::create_dir_all(base.join("shared").join("run")).unwrap();
        let home = base.join("home");
        fs::create_dir_all(&home).unwrap();
        let user = UserDirs { home: home.clone(), temp: home.join("AppData").join("Local").join("Temp"), resolved: true };
        World { _guard: guard, base, project, user }
    }

    fn at(&self, rel: &str) -> PathBuf {
        self.base.join(rel)
    }

    fn slash(&self, rel: &str) -> String {
        self.at(rel).to_string_lossy().replace('\\', "/")
    }

    /// The analysis with session `A` of the project making `shared/run`, plus `others`.
    fn analyze(&self, others: &[(&str, PathBuf, Vec<ToolCall>)], budget: Budget) -> (ProjectScan, FakeCalls) {
        let cwd = self.project.to_string_lossy().into_owned();
        let mut sessions = vec![descriptor(Agent::Claude, "main", "A", &cwd, "2026-11-10T10:00:00Z", None)];
        let mut source = FakeCalls::default().answer("main", "A", Ok(vec![bash(&format!("mkdir -p {}", self.slash("shared/run")), &cwd)]));
        for (id, dir, calls) in others {
            sessions.push(descriptor(Agent::Claude, "main", id, &dir.to_string_lossy(), "2026-11-11T10:00:00Z", None));
            source = source.answer("main", id, Ok(calls.clone()));
        }
        let listings = [listing(Agent::Claude, sessions)];
        let env = Env { search_roots: Some(vec![self.base.clone()]), history: input(&listings, Some(&source), &self.user), ..env() };
        let scan = crate::owned::analyze_with(&self.project, &Options::default(), &env, budget, &[]).unwrap();
        (scan, source)
    }
}

fn run_item<'a>(scan: &'a ProjectScan, world: &World) -> &'a Item {
    let key = path_key(&world.at("shared/run"));
    scan.items.iter().find(|i| i.key == key).unwrap_or_else(|| panic!("no run item: {:?}", scan.items))
}

fn write_to(path: &Path, cwd: &Path) -> ToolCall {
    call("write", "Write", json!({ "file_path": path.to_string_lossy() }), Some(&cwd.to_string_lossy()))
}

#[test]
fn writes_of_another_project_block_the_shared_folder() {
    let w = World::new();
    let shared = w.at("shared");
    let (scan, _) = w.analyze(&[("B", shared.clone(), vec![write_to(&w.at("shared/run/b.txt"), &shared)])], BUDGET);
    let item = run_item(&scan, &w);
    assert!(item.blocked.contains(&Block::OtherProjectWrites), "{:?}", item.blocked);
    let others = &item.provenance.as_ref().unwrap().other_projects;
    assert_eq!(others.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["B"]);
    assert_eq!(scan.history.as_ref().unwrap().conflicts, ConflictReport { read: 1, unread: 0 });
}

#[test]
fn a_session_that_only_read_there_is_no_conflict() {
    let w = World::new();
    let shared = w.at("shared");
    let read = call("read", "Read", json!({ "file_path": w.at("shared/run/a.txt").to_string_lossy() }), Some(&shared.to_string_lossy()));
    let glob = call("glob", "Glob", json!({ "path": w.at("shared/run").to_string_lossy() }), Some(&shared.to_string_lossy()));
    let (scan, _) = w.analyze(&[("B", shared.clone(), vec![read, glob])], BUDGET);
    let item = run_item(&scan, &w);
    assert!(!item.blocked.contains(&Block::OtherProjectWrites), "{:?}", item.blocked);
    assert_eq!(scan.history.as_ref().unwrap().conflicts.read, 1);
}

#[test]
fn far_sessions_are_never_read() {
    let w = World::new();
    let far = w.at("elsewhere");
    fs::create_dir_all(&far).unwrap();
    let (scan, source) = w.analyze(&[("F", far.clone(), vec![write_to(&w.at("shared/run/f.txt"), &far)])], BUDGET);
    assert!(!source.asked().contains(&"F".to_string()), "{:?}", source.asked());
    assert_eq!(scan.history.as_ref().unwrap().conflicts, ConflictReport { read: 0, unread: 0 });
}

#[test]
fn a_home_or_drive_cwd_is_not_near() {
    let w = World::new();
    let drive = PathBuf::from(format!("{}\\", &w.base.to_string_lossy()[..2]));
    let others = [
        ("H", w.user.home.clone(), vec![write_to(&w.at("shared/run/h.txt"), &w.user.home)]),
        ("D", drive.clone(), vec![write_to(&w.at("shared/run/d.txt"), &drive)]),
    ];
    let (_, source) = w.analyze(&others, BUDGET);
    let asked = source.asked();
    assert!(!asked.contains(&"H".to_string()) && !asked.contains(&"D".to_string()), "{asked:?}");
}

#[test]
fn unread_writers_leave_the_folder_unchecked() {
    let w = World::new();
    let shared = w.at("shared");
    let budget = Budget { max_sessions: 1, ..BUDGET };
    let (scan, source) = w.analyze(&[("B", shared.clone(), vec![write_to(&w.at("shared/run/b.txt"), &shared)])], budget);
    assert!(!source.asked().contains(&"B".to_string()));
    let item = run_item(&scan, &w);
    assert!(!item.blocked.contains(&Block::OtherProjectWrites));
    assert_eq!(scan.history.as_ref().unwrap().conflicts, ConflictReport { read: 0, unread: 1 });
}

#[test]
fn unchecked_conflicts_are_partial_coverage() {
    let w = World::new();
    let shared = w.at("shared");
    let others = [("B", shared.clone(), vec![write_to(&w.at("shared/run/b.txt"), &shared)])];
    let partial = |budget| run_item(&w.analyze(&others, budget).0, &w).risks.contains(&Risk::PartialCoverage);
    assert!(partial(Budget { max_sessions: 1, ..BUDGET }), "a writer left unread");
    assert!(!partial(BUDGET), "every writer read");
}
