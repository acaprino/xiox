//! The `scratch` category: which folders the sessions made become review candidates.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use xiox_model::{path_key, Agent, ArtifactKind, Level};
use xiox_mukade::ToolCall;

use super::fake::{bash, call, descriptor, input, listing, FakeCalls};
use crate::history::scratch_items;
use crate::history::{Folders, UserDirs};
use crate::tests::fixtures::*;
use crate::*;

/// What a session did. Folders are relative to the test base; `~` is the home, `%TEMP%` the
/// temp folder and `/` the root of the base's drive.
pub(super) enum Do {
    Mkdir(&'static str),
    FailedMkdir(&'static str),
    Write(&'static str),
    Read(&'static str),
    Glob(&'static str),
    Cd(&'static str),
}
pub(super) use Do::*;

/// A project with a worktree in `wt-box/wt`, and a home with a temp folder, under a temp base.
pub(super) struct Site {
    _guard: tempfile::TempDir,
    pub(super) base: PathBuf,
    pub(super) project: Project,
    pub(super) user: UserDirs,
}

impl Site {
    pub(super) fn new() -> Site {
        let (guard, base) = tmp();
        let root = repo(&base.join("work").join("proj"));
        worktree(&root, &base.join("wt-box").join("wt"), "wt");
        let user = UserDirs { home: base.join("home"), temp: base.join("home").join("AppData").join("Local").join("Temp") };
        Site { _guard: guard, base, project: resolve_project(&root).unwrap(), user }
    }

    pub(super) fn at(&self, rel: &str) -> PathBuf {
        match rel {
            "~" => self.user.home.clone(),
            "%TEMP%" => self.user.temp.clone(),
            "/" => PathBuf::from(format!("{}\\", &self.base.to_string_lossy()[..2])),
            _ => match (rel.strip_prefix("~/"), rel.strip_prefix("%TEMP%/")) {
                (Some(rest), _) => self.user.home.join(rest),
                (_, Some(rest)) => self.user.temp.join(rest),
                _ => self.base.join(rel),
            },
        }
    }

    fn calls(&self, session: &[Do]) -> Vec<ToolCall> {
        let cwd = self.project.root.to_string_lossy().into_owned();
        let text = |rel: &str| self.at(rel).to_string_lossy().replace('\\', "/");
        let file = |rel: &str| json!({ "file_path": self.at(rel).to_string_lossy() });
        session
            .iter()
            .map(|d| match d {
                Mkdir(rel) => bash(&format!("mkdir {}", text(rel)), &cwd),
                FailedMkdir(rel) => {
                    let mut c = bash(&format!("mkdir {}", text(rel)), &cwd);
                    c.error = Some(true);
                    c
                }
                Write(rel) => call("write", "Write", file(rel), Some(&cwd)),
                Read(rel) => call("read", "Read", file(rel), Some(&cwd)),
                Glob(rel) => call("glob", "Glob", json!({ "path": self.at(rel).to_string_lossy() }), Some(&cwd)),
                Cd(rel) => bash(&format!("cd {}", text(rel)), &cwd),
            })
            .collect()
    }

    fn folders(&self, session: &[Do]) -> Folders {
        let d = descriptor(Agent::Claude, "main", "s1", &self.project.root.to_string_lossy(), "2026-11-10T10:00:00Z", None);
        let mut folders = Folders::default();
        for ledger in session_ledgers(&self.calls(session), &self.user.path_env()).values() {
            folders.add_ledger(&d, ledger);
        }
        folders
    }

    /// The policy over what `session` did. `reach` are the search roots; `None` is every drive.
    pub(super) fn scratch(&self, session: &[Do], reach: Option<Vec<PathBuf>>) -> (Vec<Item>, BTreeMap<Exclusion, usize>) {
        self.scratch_with(session, reach, &[])
    }

    /// The same, with `sessions` the live listing the engine knows.
    pub(super) fn scratch_with(
        &self,
        session: &[Do],
        reach: Option<Vec<PathBuf>>,
        sessions: &[xiox_app::Preloaded],
    ) -> (Vec<Item>, BTreeMap<Exclusion, usize>) {
        let env = Env { sessions, search_roots: reach, history: input(&[], None, &self.user), ..env() };
        let mut excluded = BTreeMap::new();
        let items =
            scratch_items(&self.project, &self.folders(session), &[], false, &env, &Options::default(), &mut excluded);
        (items, excluded)
    }

    pub(super) fn within_base(&self, session: &[Do]) -> (Vec<Item>, BTreeMap<Exclusion, usize>) {
        self.scratch(session, Some(vec![self.base.clone()]))
    }
}

fn why(item: &Item) -> (Strength, bool) {
    match &item.why {
        Why::SessionFolder { strength, from_script, .. } => (*strength, *from_script),
        other => panic!("not a session folder: {other:?}"),
    }
}

#[test]
fn what_the_sessions_made_becomes_scratch() {
    // (name, what the session did, folders on the disk, expected (folder, strength))
    let table: Vec<(&str, Vec<Do>, Vec<&str>, Vec<(&str, Strength)>)> = vec![
        (
            "created folder with a script written in it",
            vec![Mkdir("agent-runs/planner-benchmark-42"), Write("agent-runs/planner-benchmark-42/benchmark.py")],
            vec!["agent-runs/planner-benchmark-42"],
            vec![("agent-runs/planner-benchmark-42", Strength::Created)],
        ),
        (
            "a file written in an existing folder is only written",
            vec![Write("existing/out.txt")],
            vec!["existing"],
            vec![("existing", Strength::Written)],
        ),
        (
            "a failed mkdir is no creation, even when the folder exists",
            vec![FailedMkdir("agent-runs/failed")],
            vec!["agent-runs/failed"],
            vec![],
        ),
        (
            "reading, searching and entering are only use",
            vec![Read("used/a.txt"), Glob("globbed"), Cd("entered")],
            vec!["used", "globbed", "entered"],
            vec![],
        ),
        (
            "the folder created is proposed, never the one around it",
            vec![Mkdir("agent-runs/batch/phazly")],
            vec!["agent-runs/batch/phazly"],
            vec![("agent-runs/batch/phazly", Strength::Created)],
        ),
        (
            "a folder next to it with no evidence is not proposed",
            vec![Mkdir("agent-runs/mine")],
            vec!["agent-runs/mine", "agent-runs/neighbour"],
            vec![("agent-runs/mine", Strength::Created)],
        ),
        (
            "what is written inside a created folder joins it",
            vec![Mkdir("runs/a"), Write("runs/a/src/x.rs")],
            vec!["runs/a/src"],
            vec![("runs/a", Strength::Created)],
        ),
        (
            "a folder written inside a used one stays on its own",
            vec![Read("runs/b/readme.md"), Write("runs/b/out/x.txt")],
            vec!["runs/b/out"],
            vec![("runs/b/out", Strength::Written)],
        ),
    ];
    for (name, session, on_disk, expected) in table {
        let site = Site::new();
        for rel in on_disk {
            fs::create_dir_all(site.at(rel)).unwrap();
        }
        let (items, excluded) = site.within_base(&session);
        let got: Vec<(String, Strength)> = items.iter().map(|i| (i.key.clone(), why(i).0)).collect();
        let want: Vec<(String, Strength)> = expected.iter().map(|(rel, s)| (path_key(&site.at(rel)), *s)).collect();
        assert_eq!(got, want, "{name}: {excluded:?}");
        for item in &items {
            assert_eq!((item.category, item.level), (Category::Scratch, Level::Caution), "{name}");
            assert_eq!(item.target, Target::Dir { kind: ArtifactKind::ProjectFolder }, "{name}");
        }
    }
}

#[test]
fn a_folder_made_by_a_shell_command_is_not_script_only() {
    let site = Site::new();
    fs::create_dir_all(site.at("runs/made")).unwrap();
    let (items, _) = site.within_base(&[Mkdir("runs/made")]);
    assert_eq!(why(&items[0]), (Strength::Created, false));
}

#[test]
fn a_git_repository_with_work_the_session_left_is_blocked() {
    let site = Site::new();
    // A repository with a commit no remote has: removing it would lose work.
    repo(&site.at("agent-runs/kept"));
    let (items, _) = site.within_base(&[Mkdir("agent-runs/kept"), Write("agent-runs/kept/a.txt")]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].blocked, vec![Block::UnsavedWork]);
}

#[test]
fn the_folders_inside_a_candidate_give_it_all_their_operations() {
    let site = Site::new();
    fs::create_dir_all(site.at("runs/a/src")).unwrap();
    let folders = site.folders(&[Mkdir("runs/a"), Write("runs/a/src/x.rs")]);

    let provenance = folders.provenance_within(&path_key(&site.at("runs/a"))).unwrap();

    assert_eq!(provenance.sessions.len(), 1);
    assert_eq!(provenance.sessions[0].operations.len(), 2, "{:?}", provenance.sessions[0].operations);
}

#[test]
fn broad_and_protected_folders_are_never_scratch() {
    // (name, what the session did, the folder it points at, why it is left out; `None`: allowed)
    let table: [(&str, Vec<Do>, &str, Option<Exclusion>); 9] = [
        ("the home", vec![Write("~/notes.txt")], "~", Some(Exclusion::Protected)),
        ("a folder of the home", vec![Write("~/Desktop/a.txt")], "~/Desktop", Some(Exclusion::Protected)),
        ("a drive root", vec![Write("/out.txt")], "/", Some(Exclusion::Protected)),
        ("%TEMP% itself", vec![Write("%TEMP%/a.txt")], "%TEMP%", Some(Exclusion::Protected)),
        ("the parent of the project", vec![Write("work/a.txt")], "work", Some(Exclusion::Protected)),
        ("the folder around a worktree", vec![Write("wt-box/a.txt")], "wt-box", Some(Exclusion::Protected)),
        ("inside the project", vec![Write("work/proj/tmp/a.txt")], "work/proj/tmp", Some(Exclusion::InsideProject)),
        ("a new folder of the home", vec![Mkdir("~/x")], "~/x", None),
        ("a folder inside %TEMP%", vec![Write("%TEMP%/run1/a.txt")], "%TEMP%/run1", None),
    ];
    for (name, session, folder, expected) in table {
        let site = Site::new();
        if folder != "/" {
            fs::create_dir_all(site.at(folder)).unwrap();
        }
        // Every drive, so that the scope is not what keeps the wide folders out.
        let (items, excluded) = site.scratch(&session, None);
        match expected {
            Some(why) => {
                assert!(items.is_empty(), "{name}: {items:?}");
                assert_eq!(excluded, BTreeMap::from([(why, 1)]), "{name}");
            }
            None => {
                let keys: Vec<&String> = items.iter().map(|i| &i.key).collect();
                assert_eq!(keys, vec![&path_key(&site.at(folder))], "{name}: {excluded:?}");
            }
        }
    }
}

/// A junction (or symlink) at `link` leading to `target`; `false` when this machine cannot make one.
fn link_dir(link: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .is_ok_and(|o| o.status.success());
    #[cfg(not(windows))]
    let made = std::os::unix::fs::symlink(target, link).is_ok();
    made
}

#[test]
fn junction_or_missing_folder_is_never_entered() {
    let site = Site::new();
    let target = site.at("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("big.bin"), vec![0u8; 1024]).unwrap();
    fs::create_dir_all(site.at("agent-runs")).unwrap();
    let link = site.at("agent-runs/was-a-folder");
    if !link_dir(&link, &target) {
        eprintln!("skipped: cannot create a junction or symlink here");
        return;
    }

    let (items, excluded) = site.within_base(&[Mkdir("agent-runs/was-a-folder"), Write("agent-runs/was-a-folder/a.txt")]);
    assert!(items.is_empty(), "{items:?}");
    assert_eq!(excluded, BTreeMap::from([(Exclusion::Link, 1)]));
    assert!(target.join("big.bin").exists());
    let _ = fs::remove_dir(&link);

    let (items, excluded) = site.within_base(&[Mkdir("agent-runs/gone")]);
    assert!(items.is_empty(), "{items:?}");
    assert_eq!(excluded, BTreeMap::from([(Exclusion::Missing, 1)]));
}

#[test]
fn an_ancestor_that_is_a_link_stops_the_descent() {
    let site = Site::new();
    let target = site.at("target");
    fs::create_dir_all(target.join("inner")).unwrap();
    let link = site.at("linked");
    if !link_dir(&link, &target) {
        eprintln!("skipped: cannot create a junction or symlink here");
        return;
    }
    let (items, excluded) = site.within_base(&[Mkdir("linked/inner")]);
    assert!(items.is_empty(), "{items:?}");
    assert_eq!(excluded, BTreeMap::from([(Exclusion::Link, 1)]));
    let _ = fs::remove_dir(&link);
}

#[test]
fn a_folder_in_another_repository_or_outside_the_scope_is_counted() {
    let site = Site::new();
    let other = repo(&site.at("other"));
    fs::create_dir_all(other.join("out")).unwrap();
    fs::create_dir_all(site.at("far/x")).unwrap();
    let session = [Mkdir("other/out"), Mkdir("far/x")];

    let (items, excluded) = site.scratch(&session, Some(vec![site.at("other")]));

    assert!(items.is_empty(), "{items:?}");
    assert_eq!(excluded, BTreeMap::from([(Exclusion::OtherRepository, 1), (Exclusion::OutsideScope, 1)]));
}

#[test]
fn analyze_lists_scratch_once_and_leaves_listed_folders_to_their_category() {
    let site = Site::new();
    let project = repo(&site.base.join("work").join("mukade"));
    let named = site.at("work/mukade-test");
    fs::create_dir_all(&named).unwrap();
    fs::create_dir_all(site.at("agent-runs/run1")).unwrap();
    let cwd = project.to_string_lossy().into_owned();
    let make = |rel: &str| bash(&format!("mkdir {}", site.at(rel).to_string_lossy().replace('\\', "/")), &cwd);
    let calls = vec![make("work/mukade-test"), make("agent-runs/run1")];
    let listings = [listing(Agent::Claude, vec![descriptor(Agent::Claude, "main", "s1", &cwd, "2026-11-10T10:00:00Z", None)])];
    let source = FakeCalls::default().answer("main", "s1", Ok(calls));
    let env = Env { search_roots: Some(vec![site.base.clone()]), history: input(&listings, Some(&source), &site.user), ..env() };

    let scan = analyze_with(&project, &Options::default(), &env).unwrap();

    let scratch: Vec<&Item> = scan.items.iter().filter(|i| i.category == Category::Scratch).collect();
    assert_eq!(scratch.len(), 1, "{:?}", scan.items);
    assert_eq!(scratch[0].key, path_key(&site.at("agent-runs/run1")));
    assert_eq!(scratch[0].provenance.as_ref().map(|p| p.sessions.len()), Some(1));
    assert!(scan.items.iter().any(|i| i.category == Category::Named && i.key == path_key(&named)));
}

#[test]
fn folders_around_the_home_or_a_plain_temp_are_protected() {
    // (name, home, temp, what the session did, the folder it points at, why it is left out)
    let table: [(&str, &str, &str, Vec<Do>, &str, Option<Exclusion>); 6] = [
        ("a folder that holds the home", "Profiles/alfio", "Profiles/alfio/Temp", vec![Write("Profiles/x.txt")], "Profiles", Some(Exclusion::Protected)),
        ("a folder that holds a plain temp", "h/alfio", "Temp/inner", vec![Write("Temp/x.txt")], "Temp", Some(Exclusion::Protected)),
        ("a plain temp itself", "h/alfio", "Temp", vec![Write("Temp/x.txt")], "Temp", Some(Exclusion::Protected)),
        ("a folder inside a plain temp", "h/alfio", "Temp", vec![Write("Temp/run1/x.txt")], "Temp/run1", None),
        ("a broad folder of a temp that is the home", "h/alfio", "h/alfio", vec![Write("h/alfio/Desktop/x.txt")], "h/alfio/Desktop", Some(Exclusion::Protected)),
        ("a new folder of a temp that is the home", "h/alfio", "h/alfio", vec![Mkdir("h/alfio/x")], "h/alfio/x", None),
    ];
    for (name, home, temp, session, folder, expected) in table {
        let mut site = Site::new();
        site.user = UserDirs { home: site.base.join(home), temp: site.base.join(temp) };
        fs::create_dir_all(&site.user.home).unwrap();
        fs::create_dir_all(&site.user.temp).unwrap();
        fs::create_dir_all(site.at(folder)).unwrap();

        let (items, excluded) = site.scratch(&session, None);

        match expected {
            Some(why) => {
                assert!(items.is_empty(), "{name}: {items:?}");
                assert_eq!(excluded, BTreeMap::from([(why, 1)]), "{name}");
            }
            None => {
                let keys: Vec<&String> = items.iter().map(|i| &i.key).collect();
                assert_eq!(keys, vec![&path_key(&site.at(folder))], "{name}: {excluded:?}");
            }
        }
    }
}
