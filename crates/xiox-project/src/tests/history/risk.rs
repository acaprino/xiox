//! Risks, activity and the order of the scratch candidates.

use std::fs::{self, File, FileTimes, OpenOptions};
use std::path::Path;
use std::time::{Duration, SystemTime};

use xiox_model::{Agent, Loaded, SessionRecord};

use super::scratch::{Do::*, Site};
use crate::history::risk::{activity, risks, RECENT_MS};
use crate::history::{Provenance, SessionEvidence};
use crate::owned::sort_items;
use crate::tests::fixtures::{env, tmp};
use crate::*;

const NOW: i64 = 1_793_527_200_000;
const HOUR: i64 = 60 * 60 * 1000;

fn evidence(gaps: bool) -> SessionEvidence {
    SessionEvidence {
        agent: Agent::Claude,
        harness: "claude".into(),
        store: "main".into(),
        id: "s1".into(),
        parent: None,
        last_activity_ms: None,
        strength: None,
        listed_cwd: false,
        operations: Vec::new(),
        omitted: 0,
        failed: 0,
        gaps,
    }
}

fn provenance(strength: Strength, last: Option<i64>, gaps: bool) -> Provenance {
    Provenance { strength: Some(strength), last_activity_ms: last, sessions: vec![evidence(gaps)], other_projects: Vec::new() }
}

fn set_mtime(dir: &Path, ms: i64) {
    let mut open = OpenOptions::new();
    open.write(true);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(&mut open, 0x0200_0000); // FILE_FLAG_BACKUP_SEMANTICS
    let file: File = open.open(dir).unwrap();
    let at = SystemTime::UNIX_EPOCH + Duration::from_millis(ms as u64);
    file.set_times(FileTimes::new().set_modified(at)).unwrap();
}

#[test]
fn risks_table() {
    use Risk::*;
    // (name, strength, has .git, last activity, mtime, report partial, session gaps, expected)
    let table: [(&str, Strength, bool, i64, i64, bool, bool, Vec<Risk>); 5] = [
        ("no git and an hour ago", Strength::Created, false, NOW - HOUR, NOW - 48 * HOUR, false, false, vec![NoGit, RecentActivity]),
        ("only written", Strength::Written, true, NOW - 48 * HOUR, NOW - 48 * HOUR, false, false, vec![WeakEvidence]),
        ("the folder changed an hour ago", Strength::Created, true, NOW - 48 * HOUR, NOW - HOUR, false, false, vec![RecentActivity]),
        ("the report is partial", Strength::Created, true, NOW - 48 * HOUR, NOW - 48 * HOUR, true, false, vec![PartialCoverage]),
        ("a session has gaps", Strength::Created, true, NOW - 48 * HOUR, NOW - 48 * HOUR, false, true, vec![PartialCoverage]),
    ];
    for (name, strength, git, last, mtime, partial, gaps, expected) in table {
        let (_t, base) = tmp();
        let dir = base.join("run");
        fs::create_dir_all(&dir).unwrap();
        if git {
            fs::create_dir(dir.join(".git")).unwrap();
        }
        set_mtime(&dir, mtime);

        let found = risks(&dir, &provenance(strength, Some(last), gaps), partial, NOW);

        assert_eq!(found, expected, "{name}");
    }
}

#[test]
fn the_recent_window_is_a_day() {
    assert_eq!(RECENT_MS, 24 * HOUR);
}

#[test]
fn old_mtime_does_not_drop_a_candidate() {
    let site = Site::new();
    fs::create_dir_all(site.at("agent-runs/run1")).unwrap();
    set_mtime(&site.at("agent-runs/run1"), NOW - 365 * 24 * HOUR);

    let (items, excluded) = site.within_base(&[Mkdir("agent-runs/run1"), Write("agent-runs/run1/a.txt")]);

    assert_eq!(items.len(), 1, "{items:?} {excluded:?}");
    assert!(matches!(items[0].why, Why::SessionFolder { strength: Strength::Created, .. }), "{:?}", items[0].why);
    let provenance = items[0].provenance.as_ref().expect("a scratch item has its provenance");
    assert_eq!(provenance.strength, Some(Strength::Created));
    assert_eq!(provenance.sessions.len(), 1);
}

fn scratch_item(key: &str, last: Option<i64>) -> Item {
    Item {
        key: key.into(),
        category: Category::Scratch,
        path: None,
        bytes: Some(1),
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why: Why::SessionFolder { strength: Strength::Created, from_script: false, activity: Activity::Unknown },
        target: Target::File,
        provenance: Some(Provenance { strength: Some(Strength::Created), last_activity_ms: last, sessions: Vec::new(), other_projects: Vec::new() }),
    }
}

#[test]
fn scratch_is_sorted_by_observed_activity() {
    let at = |h: i64| Some(NOW - 24 * HOUR + h * HOUR);
    // (name, items as (key, last activity), expected key order)
    let table: [(&str, Vec<(&str, Option<i64>)>, Vec<&str>); 2] = [
        ("dated first, newest first", vec![("a", at(10)), ("b", None), ("c", at(12))], vec!["c", "a", "b"]),
        ("the undated stay in key order", vec![("z", None), ("a", None), ("m", at(1))], vec!["m", "a", "z"]),
    ];
    for (name, items, expected) in table {
        let mut items: Vec<Item> = items.into_iter().map(|(k, t)| scratch_item(k, t)).collect();

        sort_items(&mut items);

        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, expected, "{name}");
    }
}

fn live_sessions(dir: &Path, live: bool) -> Vec<xiox_app::Preloaded> {
    vec![xiox_app::Preloaded {
        agent: Agent::Claude,
        loaded: Loaded {
            items: vec![SessionRecord {
                agent: Agent::Claude,
                session_id: Some("live".into()),
                project_path: dir.join("src"),
                last_activity_ms: NOW,
                archived: false,
                live,
            }],
            warnings: Vec::new(),
        },
        worktrees: Vec::new(),
    }]
}

#[test]
fn live_session_inside_is_active_and_blocked() {
    let site = Site::new();
    let dir = site.at("agent-runs/run1");
    fs::create_dir_all(&dir).unwrap();
    let session = [Mkdir("agent-runs/run1"), Write("agent-runs/run1/a.txt")];

    let (items, _) = site.scratch_with(&session, Some(vec![site.base.clone()]), &live_sessions(&dir, true));
    assert!(matches!(items[0].why, Why::SessionFolder { activity: Activity::Active, .. }), "{:?}", items[0].why);
    assert!(items[0].blocked.contains(&Block::LiveSession));

    let (items, _) = site.scratch_with(&session, Some(vec![site.base.clone()]), &live_sessions(&dir, false));
    assert!(matches!(items[0].why, Why::SessionFolder { activity: Activity::Unknown, .. }), "{:?}", items[0].why);
    assert!(!items[0].blocked.contains(&Block::LiveSession));
    assert_eq!(activity(&dir, &Env { sessions: &live_sessions(&dir, false), ..env() }), Activity::Unknown);
}

#[test]
fn scratch_items_carry_their_risks() {
    let site = Site::new();
    fs::create_dir_all(site.at("agent-runs/run1")).unwrap();

    let (items, _) = site.within_base(&[Mkdir("agent-runs/run1"), Write("agent-runs/run1/a.txt")]);

    // A folder made now, outside any repository: the folder date is only recency.
    assert_eq!(items[0].risks, [Risk::NoGit, Risk::RecentActivity]);
    match &items[0].why {
        Why::SessionFolder { activity, .. } => {
            assert_eq!(*activity, Activity::Unknown);
        }
        other => panic!("{other:?}"),
    }
}
