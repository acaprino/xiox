//! The queue of the project's sessions and the budgeted reading of their calls.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;
use xiox_model::{path_key, Agent, Loaded};
use xiox_mukade::Descriptor;

use super::fake::{bash, call, descriptor, input, listing, FakeCalls};
use crate::history::{Budget, History, UserDirs, BUDGET};
use crate::tests::fixtures::*;
use crate::*;

struct Run {
    report: HistoryReport,
    items: Vec<Item>,
}

/// Reads the history of `project` with `calls`, starting from the items the search would have
/// found. Everywhere is in scope, so the folders a session names are all reachable.
fn run(
    project: &Path,
    listings: &[(Agent, Loaded<Descriptor>)],
    calls: &FakeCalls,
    budget: Budget,
    mut items: Vec<Item>,
) -> Run {
    let project = resolve_project(project).unwrap();
    let opts = Options::default();
    let user = UserDirs::default();
    let env = Env { search_roots: None, history: input(listings, Some(calls), &user), ..env() };
    let mut history = History::start(&project, &opts, &env).unwrap().with_budget(budget);
    history.read(&project, &mut items, &mut |dirs| crate::search::verify_dirs(&project, &opts, &env, dirs));
    history.attach(&mut items);
    Run { report: history.into_report(), items }
}

fn cwd(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn claude(store: &str, id: &str, cwd: &Path, updated: &str, parent: Option<&str>) -> Descriptor {
    descriptor(Agent::Claude, store, id, &cwd.to_string_lossy(), updated, parent)
}

/// `n` sessions of the project, `s0` the newest.
fn project_sessions(project: &Path, n: usize) -> Vec<Descriptor> {
    (0..n).map(|i| claude("main", &format!("s{i}"), project, &format!("2026-11-1{}T10:00:00Z", 9 - i), None)).collect()
}

fn claude_row(run: &Run) -> &SourceReport {
    run.report.sources.iter().find(|s| s.agent == Agent::Claude).unwrap()
}

#[test]
fn only_sessions_of_the_project_are_read_newest_first() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    let elsewhere = base.join("elsewhere");
    let listings = [listing(
        Agent::Claude,
        vec![
            claude("main", "old", &project, "2026-11-10T10:00:00Z", None),
            claude("main", "other", &elsewhere, "2026-11-11T10:00:00Z", None),
            claude("main", "new", &project.join("src"), "2026-11-10T12:00:00Z", None),
        ],
    )];
    let calls = FakeCalls::default();

    let run = run(&project, &listings, &calls, BUDGET, Vec::new());

    assert_eq!(calls.asked(), ["new", "old"]);
    let row = claude_row(&run);
    assert_eq!((row.listed, row.related, row.read), (3, 2, 2));
    assert_eq!(run.report.sessions_read, 2);
}

#[test]
fn child_sessions_follow_their_parent() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    let elsewhere = base.join("elsewhere");
    let listings = [listing(
        Agent::Claude,
        vec![
            claude("main", "parent", &project, "2026-11-10T10:00:00Z", None),
            claude("main", "child", &elsewhere, "2026-11-10T09:00:00Z", Some("parent")),
            claude("main", "grandchild", &elsewhere, "2026-11-10T08:00:00Z", Some("child")),
            claude("main", "stranger", &elsewhere, "2026-11-10T11:00:00Z", None),
            // The parent id names a session of another store: not the same session.
            claude("other", "orphan", &elsewhere, "2026-11-10T11:30:00Z", Some("parent")),
        ],
    )];
    let calls = FakeCalls::default();

    run(&project, &listings, &calls, BUDGET, Vec::new());

    assert_eq!(calls.asked(), ["parent", "child", "grandchild"]);
}

#[test]
fn same_id_in_two_stores_is_two_sessions() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    let copy = clone(&project, &base.join("copy"));
    let read_in_copy = call("read", "Read", json!({ "file_path": copy.join("README.md") }), Some(&cwd(&project)));
    let listings = [listing(
        Agent::Claude,
        vec![
            claude("main", "s1", &project, "2026-11-10T10:00:00Z", None),
            claude("work", "s1", &project, "2026-11-10T11:00:00Z", None),
        ],
    )];
    let calls = FakeCalls::default()
        .answer("main", "s1", Ok(vec![read_in_copy.clone()]))
        .answer("work", "s1", Ok(vec![read_in_copy]));
    let found = crate::search::verify_dirs(
        &resolve_project(&project).unwrap(),
        &Options::default(),
        &Env { search_roots: None, ..env() },
        &[copy.clone()],
    );

    let run = run(&project, &listings, &calls, BUDGET, found);

    assert_eq!(calls.asked(), ["s1", "s1"]);
    let item = run.items.iter().find(|i| i.key == path_key(&copy)).unwrap();
    let stores: Vec<&str> = item.provenance.as_ref().unwrap().sessions.iter().map(|s| s.store.as_str()).collect();
    assert_eq!(stores, ["main", "work"]);
}

#[test]
fn a_failed_read_is_counted_and_the_rest_is_read() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    let listings = [listing(Agent::Claude, project_sessions(&project, 3))];
    let calls = FakeCalls::default().answer("main", "s1", Err(ReadFailure::TimedOut));
    let user = UserDirs::default();
    let env = Env { search_roots: Some(Vec::new()), history: input(&listings, Some(&calls), &user), ..env() };

    let scan = crate::analyze_with(&project, &Options::default(), &env).unwrap();

    let history = scan.history.as_ref().unwrap();
    let row = history.sources.iter().find(|s| s.agent == Agent::Claude).unwrap();
    assert_eq!(history.sessions_read, 2);
    assert_eq!((row.related, row.read), (3, 2));
    assert_eq!(row.failed, BTreeMap::from([(ReadFailure::TimedOut, 1)]));
    assert!(scan.notes.contains(&Note::HistoryPartial), "{:?}", scan.notes);
    assert!(!serde_json::to_string(&scan).unwrap().contains("mukade"));
}

#[test]
fn an_unreadable_listing_is_counted_without_asking_the_source() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    let mut sessions = project_sessions(&project, 2);
    sessions[0].info.unreadable = Some("corrupt".into());
    let listings = [listing(Agent::Claude, sessions)];
    let calls = FakeCalls::default();

    let run = run(&project, &listings, &calls, BUDGET, Vec::new());

    assert_eq!(calls.asked(), ["s1"]);
    assert_eq!(claude_row(&run).failed, BTreeMap::from([(ReadFailure::Failed, 1)]));
}

#[test]
fn budget_stops_and_says_how_many_were_left() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    let listings = [listing(Agent::Claude, project_sessions(&project, 5))];
    let calls = FakeCalls::default();

    let run = run(&project, &listings, &calls, Budget { max_sessions: 2, ..BUDGET }, Vec::new());

    assert_eq!(run.report.sessions_read, 2);
    assert_eq!(run.report.limits, [Limit::Sessions { max: 2, left: 3 }]);
}

#[test]
fn call_budget_stops_after_the_session_that_reaches_it() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    let listings = [listing(Agent::Claude, project_sessions(&project, 3))];
    let two_calls = || Ok(vec![bash("ls", &cwd(&project)), bash("ls", &cwd(&project))]);
    let calls = FakeCalls::default().answer("main", "s0", two_calls()).answer("main", "s1", two_calls());

    let run = run(&project, &listings, &calls, Budget { max_calls: 3, ..BUDGET }, Vec::new());

    assert_eq!((run.report.sessions_read, run.report.calls), (2, 4));
    assert_eq!(run.report.limits, [Limit::Calls { max: 3 }]);
}

#[test]
fn a_clone_made_by_a_session_extends_the_queue() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    // The source runs nothing: the clone the session made has to exist already.
    let c2 = clone(&project, &base.join("c2"));
    let slashed = |p: &Path| cwd(p).replace('\\', "/");
    let made = bash(&format!("git clone {} {}", slashed(&project), slashed(&c2)), &cwd(&project));
    let used = call("read", "Read", json!({ "file_path": c2.join("README.md") }), Some(&cwd(&c2)));
    let listings = [listing(
        Agent::Claude,
        vec![
            claude("main", "a", &project, "2026-11-10T10:00:00Z", None),
            claude("main", "b", &c2, "2026-11-10T11:00:00Z", None),
        ],
    )];
    let calls = FakeCalls::default().answer("main", "a", Ok(vec![made])).answer("main", "b", Ok(vec![used]));

    let run = run(&project, &listings, &calls, BUDGET, Vec::new());

    assert_eq!(calls.asked(), ["a", "b"]);
    let item = run.items.iter().find(|i| i.key == path_key(&c2)).expect("the clone is listed");
    assert_eq!(item.category, Category::Copies);
    let evidence: Vec<(&str, Option<Strength>)> =
        item.provenance.as_ref().unwrap().sessions.iter().map(|s| (s.id.as_str(), s.strength)).collect();
    assert_eq!(evidence, [("a", Some(Strength::Created)), ("b", Some(Strength::Used))]);
}

#[test]
fn listing_failure_is_reported_apart_from_an_absent_store() {
    let (_t, base) = tmp();
    let project = repo(&base.join("proj"));
    let failed = Loaded { items: Vec::<Descriptor>::new(), warnings: vec!["list failed".to_string()] };
    let listings = [(Agent::Claude, failed), listing(Agent::Codex, Vec::new())];
    let user = UserDirs::default();
    let calls = FakeCalls::default();
    let env = Env { search_roots: Some(Vec::new()), history: input(&listings, Some(&calls), &user), ..env() };

    let scan = crate::analyze_with(&project, &Options::default(), &env).unwrap();

    let rows: Vec<(Agent, bool, usize)> =
        scan.history.unwrap().sources.iter().map(|s| (s.agent, s.listing_failed, s.listed)).collect();
    assert_eq!(rows, [(Agent::Claude, true, 0), (Agent::Codex, false, 0)]);
}
