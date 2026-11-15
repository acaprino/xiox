//! The shell-event fixtures: the data every extractor test runs on.

use crate::OpKind;
use serde::Deserialize;

const FIXTURE_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/shell-events.toml");
const KINDS: [OpKind; 6] =
    [OpKind::Create, OpKind::Copy, OpKind::Move, OpKind::Clone, OpKind::Worktree, OpKind::Write];

/// One labelled shell event: the command a tool ran and the paths it leaves on disk.
pub struct Fixture {
    pub id: String,
    pub cat: String,
    pub shell: String,
    pub cwd: String,
    pub cmd: String,
    pub exit: i32,
    pub gold: Vec<(OpKind, Option<String>, String)>,
}

pub struct Fixtures {
    /// `(home, temp)` of the machine the events were labelled on.
    pub env: (String, String),
    pub events: Vec<Fixture>,
}

#[derive(Deserialize)]
struct FileData {
    defaults: Defaults,
    event: Vec<EventData>,
}

#[derive(Deserialize)]
struct Defaults {
    cwd: String,
    home: String,
    temp: String,
}

#[derive(Deserialize)]
struct EventData {
    id: String,
    cat: String,
    shell: String,
    cwd: Option<String>,
    cmd: String,
    exit: Option<i32>,
    gold: Vec<GoldData>,
}

#[derive(Deserialize)]
struct GoldData {
    op: String,
    src: Option<String>,
    dst: String,
}

pub fn load_fixtures() -> Fixtures {
    let text = std::fs::read_to_string(FIXTURE_FILE).expect("fixture file is readable");
    let data: FileData = toml::from_str(&text).expect("fixture file is valid");
    let Defaults { cwd, home, temp } = data.defaults;
    let events = data
        .event
        .into_iter()
        .map(|e| Fixture {
            gold: e.gold.into_iter().map(|g| (kind_of(&g.op), g.src, g.dst)).collect(),
            cwd: e.cwd.unwrap_or_else(|| cwd.clone()),
            cmd: e.cmd.trim().to_string(),
            exit: e.exit.unwrap_or(0),
            id: e.id,
            cat: e.cat,
            shell: e.shell,
        })
        .collect();
    Fixtures { env: (home, temp), events }
}

fn kind_of(op: &str) -> OpKind {
    *KINDS.iter().find(|k| k.as_str() == op).unwrap_or_else(|| panic!("unknown op {op} in fixtures"))
}

const CATEGORIES: [&str; 16] = [
    "create", "copy", "move", "clone", "worktree", "write", "readonly", "delete", "chain", "spaces", "failed",
    "variable", "loop", "script", "nested", "tool-flag",
];

#[test]
fn fixtures_load_and_cover_every_category() {
    let fixtures = load_fixtures();
    assert_eq!(fixtures.events.len(), 106);
    for cat in CATEGORIES {
        assert!(fixtures.events.iter().any(|e| e.cat == cat), "no fixture in category {cat}");
    }
    assert_eq!(fixtures.env, (r"C:\Users\dev".to_string(), r"C:\Users\dev\AppData\Local\Temp".to_string()));
    for event in &fixtures.events {
        assert!(!event.cmd.is_empty() && !event.shell.is_empty() && event.exit >= 0, "{}: incomplete", event.id);
        for (_, _, dst) in &event.gold {
            let drive = dst.as_bytes();
            assert!(
                drive.len() > 3 && drive[0].is_ascii_alphabetic() && &dst[1..3] == ":\\",
                "{}: gold dst {dst} is not absolute",
                event.id
            );
        }
    }
    assert_eq!(fixtures.events[0].cwd, r"D:\Projects\Phazly");
}
