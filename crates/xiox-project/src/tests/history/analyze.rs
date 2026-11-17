//! The real history source end to end: a synthetic Claude Code store read through Mukade.

use std::fs;
use std::path::{Path, PathBuf};

use xiox_model::{Agent, RemoveMode};
use xiox_mukade::test_support::write_claude_session;
use xiox_mukade::{MukadeSessions, StoreRoots};

use super::fake::{descriptor, input, listing, FakeCalls};
use crate::history::{Budget, MukadeCalls, ReadFailure, UserDirs, BUDGET};
use crate::tests::clean::{run_with, FakeBin};
use crate::tests::fixtures::*;
use crate::*;

/// The one session of [`World`]'s store.
const SESSION: &str = "11111111-2222-3333-4444-555555555555";

/// A git project and a Claude store whose one session ran in it and made `<base>\runs\bench-1`.
struct World {
    _guard: tempfile::TempDir,
    base: PathBuf,
    project: PathBuf,
    sessions: MukadeSessions,
    user: UserDirs,
}

impl World {
    fn new() -> World {
        let (guard, base) = tmp();
        // The project's parent is within reach of the history even with no search roots.
        let project = repo(&base.join("proj"));
        let made = base.join("runs").join("bench-1");
        fs::create_dir_all(&made).unwrap();
        let store = base.join("claude");
        let command = format!("mkdir \"{}\"", made.display());
        write_claude_session(&store, SESSION, &project.to_string_lossy(), &command);
        let roots = StoreRoots { claude_config_dir: Some(store), ..Default::default() };
        let sessions = MukadeSessions::with_roots(Agent::Claude, roots).with_cache_dir(Some(base.join("data")));
        let user = UserDirs { home: base.join("home"), temp: base.join("home").join("Temp"), resolved: true };
        World { _guard: guard, base, project, sessions, user }
    }

    fn scan(&self, opts: &Options) -> ProjectScan {
        self.with_env(|env| analyze_with(&self.project, opts, env).unwrap())
    }

    /// `run` with an environment that lists and reads the store as it is now, for an analysis
    /// and a cleaning alike.
    fn with_env<R>(&self, run: impl FnOnce(&Env) -> R) -> R {
        let listings = [(Agent::Claude, self.sessions.descriptors())];
        let own = std::slice::from_ref(&self.sessions);
        let calls = MukadeCalls::new(own);
        run(&Env { search_roots: Some(Vec::new()), history: input(&listings, Some(&calls), &self.user), ..env() })
    }

    /// The transcript file of [`SESSION`].
    fn transcript(&self) -> PathBuf {
        self.base.join("claude").join("projects").join("D--proj").join(format!("{SESSION}.jsonl"))
    }
}

#[test]
fn analyze_reads_a_fake_claude_store_through_mukade() {
    let world = World::new();

    let scan = world.scan(&Options::default());

    let scratch: Vec<&Item> = scan.items.iter().filter(|i| i.category == Category::Scratch).collect();
    assert_eq!(scratch.len(), 1, "{:?}", scan.items);
    assert_eq!(scratch[0].path.as_deref(), Some(world.base.join("runs").join("bench-1").as_path()));
    let provenance = scratch[0].provenance.as_ref().unwrap();
    assert_eq!(provenance.strength, Some(Strength::Created));
    assert_eq!(provenance.sessions[0].harness, "claude_code");
    assert_eq!(scan.history.as_ref().unwrap().sessions_read, 1);
}

#[test]
fn a_rewritten_transcript_is_read_again_not_reused() {
    // (name, the command the session's one call now holds, its old date put back, outcome)
    type Rewrite = fn(&Path) -> String;
    let table: [(&str, Rewrite, bool, &str); 3] = [
        ("another folder", |runs| format!("mkdir \"{}\"", runs.join("bench-2").display()), false, "no longer there"),
        // `bench-X` keeps the length of `bench-1`: same prefix, same size, same date.
        ("same size and date", |runs| format!("mkdir \"{}\"", runs.join("bench-X").display()), true, "no longer there"),
        (
            "only a write is left",
            |runs| format!("echo x > \"{}\"", runs.join("bench-1").join("a.txt").display()),
            false,
            "its risks changed since it was confirmed: choose it again",
        ),
    ];
    let choose = |world: &World| {
        let bench = world.base.join("runs").join("bench-1");
        let scan = world.scan(&Options::default());
        let request = CleanRequest {
            categories: Vec::new(),
            include: vec![bench.clone()],
            confirm_danger: vec![bench],
            mode: RemoveMode::Recycle,
        };
        let chosen = select(&scan, &request).unwrap();
        (scan, chosen)
    };
    // The control: the transcript left as it was, the same dry run removes the folder.
    let world = World::new();
    let (scan, chosen) = choose(&world);
    let bin = FakeBin::new(world.base.join("bin"));
    let out = world.with_env(|env| run_with(&scan, &chosen, RemoveMode::Recycle, true, env, &bin, &|_| Vec::new()));
    assert!(matches!(out.items[0].outcome, Outcome::Removed { .. }), "{:?}", out.items);

    for (name, rewrite, same_date, expected) in table {
        let world = World::new();
        let (scan, chosen) = choose(&world);

        let transcript = world.transcript();
        let before = fs::metadata(&transcript).unwrap();
        write_claude_session(&world.base.join("claude"), SESSION, &world.project.to_string_lossy(), &rewrite(&world.base.join("runs")));
        if same_date {
            assert_eq!(fs::metadata(&transcript).unwrap().len(), before.len(), "{name}");
            fs::File::options().write(true).open(&transcript).unwrap().set_modified(before.modified().unwrap()).unwrap();
        }
        let bin = FakeBin::new(world.base.join("bin"));
        let out = world.with_env(|env| run_with(&scan, &chosen, RemoveMode::Recycle, true, env, &bin, &|_| Vec::new()));
        assert_eq!(out.items[0].outcome, Outcome::Skipped { reason: expected.into() }, "{name}");
    }
}

#[test]
fn without_history_the_scan_is_the_old_one() {
    let world = World::new();
    let opts = Options { history: false, ..Options::default() };

    let scan = world.scan(&opts);

    let old = analyze_project_without_sessions(&world.project, &opts, &[]).unwrap();
    assert_eq!(scan.history, None);
    assert!(scan.items.iter().all(|i| i.category != Category::Scratch && i.provenance.is_none()));
    assert_eq!(scan.items, old.items);
}

#[test]
fn opencode_without_exe_counts_no_reader() {
    let (_guard, base) = tmp();
    let project = repo(&base.join("work").join("proj"));
    let db = base.join("opencode.db");
    fs::write(&db, b"main").unwrap();
    let roots = StoreRoots { opencode_db: Some(db), ..Default::default() };
    let sessions = [MukadeSessions::with_roots(Agent::Opencode, roots)];
    let cwd = project.to_string_lossy().into_owned();
    let listings = [listing(Agent::Opencode, vec![descriptor(Agent::Opencode, "db", "ses_1", &cwd, "2026-11-10T10:00:00Z", None)])];
    let calls = MukadeCalls::new(&sessions);
    let user = UserDirs::default();
    let env = Env { search_roots: Some(Vec::new()), history: input(&listings, Some(&calls), &user), ..env() };

    let scan = analyze_with(&project, &Options::default(), &env).unwrap();

    let history = scan.history.unwrap();
    assert_eq!(history.sources[0].failed.iter().collect::<Vec<_>>(), [(&ReadFailure::NoReader, &1)]);
    assert!(scan.notes.contains(&Note::HistoryPartial), "{:?}", scan.notes);
}

#[test]
fn probe_runs_on_a_fake_project() {
    let (_guard, base) = tmp();
    let project = repo(&base.join("proj"));
    let cwd = project.to_string_lossy().into_owned();
    let listings = [listing(
        Agent::Claude,
        vec![
            descriptor(Agent::Claude, "main", "new", &cwd, "2026-11-10T12:00:00Z", None),
            descriptor(Agent::Claude, "main", "old", &cwd, "2026-11-10T10:00:00Z", None),
        ],
    )];
    let calls = FakeCalls::default();
    let user = UserDirs::default();
    let env = Env { search_roots: Some(Vec::new()), history: input(&listings, Some(&calls), &user), ..env() };

    let scan = probe_with(&project, Budget { max_sessions: 1, ..BUDGET }, &env).unwrap();

    let history = scan.history.expect("the history was read");
    assert_eq!(history.limits, [Limit::Sessions { max: 1, left: 1 }]);
    assert_eq!(calls.asked(), ["new"]);
    assert!(scan.items.iter().all(|i| i.bytes.is_none()), "{:?}", scan.items);
}

#[test]
fn without_history_no_session_is_listed() {
    let (_guard, base) = tmp();
    let cache = base.join("data");

    let listings = real_listings(false, Some(cache.clone()));

    assert!(listings.is_empty());
    assert!(!cache.exists(), "the listing cache was written");
}
