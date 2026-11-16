//! The JSON shape of the history parts of the result: names, casing and the old fields that stay.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};
use xiox_model::{Agent, ArtifactKind};

use crate::*;

fn project() -> Project {
    Project { root: PathBuf::from(r"D:\p"), name: "p".into(), is_repo: false, origin: None, worktrees: Vec::new() }
}

fn item(category: Category, why: Why, provenance: Option<Provenance>) -> Item {
    Item {
        key: "d:\\p\\x".into(),
        category,
        path: Some(PathBuf::from(r"D:\p\x")),
        bytes: None,
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why,
        target: Target::Dir { kind: ArtifactKind::ProjectFolder },
        provenance,
    }
}

fn scan(items: Vec<Item>, history: Option<HistoryReport>) -> ProjectScan {
    ProjectScan {
        schema: SCHEMA,
        project: project(),
        items,
        branches: Vec::new(),
        notes: Vec::new(),
        history,
        search: SearchMode::ParentOnly,
        scanned_at_ms: 0,
        elapsed_ms: 0,
    }
}

fn json_of<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap()
}

#[test]
fn scratch_item_serializes_with_its_provenance() {
    let operation = Operation {
        source: OperationSource::Shell { op: OpKind::Create, script: false },
        outcome: CallOutcome::Succeeded,
        strength: Some(Strength::Created),
        entry: Some("e1".into()),
        call: Some("c1".into()),
        time: None,
        cwd: None,
    };
    let session = SessionEvidence {
        agent: Agent::Claude,
        harness: "claude".into(),
        store: "main".into(),
        id: "s1".into(),
        parent: None,
        last_activity_ms: Some(1_700_000_000_000),
        strength: Some(Strength::Created),
        listed_cwd: false,
        operations: vec![operation],
        omitted: 0,
        failed: 0,
        gaps: false,
    };
    let provenance =
        Provenance { strength: Some(Strength::Created), last_activity_ms: Some(1_700_000_000_000), sessions: vec![session], other_projects: Vec::new() };
    let why = Why::SessionFolder {
        strength: Strength::Created,
        from_script: false,
        activity: Activity::Unknown,
    };
    let risks = vec![Risk::NoGit, Risk::RecentActivity];
    let v = json_of(&Item { risks, ..item(Category::Scratch, why, Some(provenance)) });

    assert_eq!(v["category"], "scratch");
    assert_eq!(v["why"]["type"], "session-folder");
    assert_eq!(v["risks"], json!(["no-git", "recent-activity"]));
    assert!(v["why"].get("risks").is_none());
    assert_eq!(v["why"]["activity"], "unknown");
    let session = &v["provenance"]["sessions"][0];
    assert_eq!(session["operations"][0]["source"], json!({"type": "shell", "op": "create", "script": false}));
    assert_eq!(session["operations"][0]["outcome"], "succeeded");
    assert_eq!(session["strength"], "created");
    assert!(session.get("lastActivityMs").is_some());
}

#[test]
fn old_items_carry_null_provenance_and_scan_is_schema_2() {
    let builds = item(Category::Builds, Why::CatalogFolder { label_en: "Rust build".into() }, None);
    assert_eq!(json_of(&builds)["provenance"], Value::Null);

    let v = json_of(&scan(vec![builds], None));
    assert_eq!(v["history"], Value::Null);
    assert_eq!(v["schema"], 2);
}

#[test]
fn history_report_serializes_counts_by_kind() {
    let report = HistoryReport {
        sources: vec![SourceReport {
            agent: Agent::Codex,
            listing_failed: false,
            listed: 4,
            related: 2,
            read: 1,
            failed: BTreeMap::from([(ReadFailure::TimedOut, 1)]),
        }],
        sessions_read: 1,
        calls: 10,
        shell: Coverage { shell_calls: 7, ..Coverage::default() },
        excluded: BTreeMap::from([(Exclusion::OutsideScope, 2)]),
        limits: vec![Limit::Sessions { max: 250, left: 3 }],
        conflicts: ConflictReport::default(),
    };
    let v = json_of(&report);

    assert_eq!(v["sources"][0]["failed"], json!({"timed-out": 1}));
    assert_eq!(v["sources"][0]["listingFailed"], false);
    assert_eq!(v["excluded"], json!({"outside-scope": 2}));
    assert_eq!(v["limits"][0], json!({"type": "sessions", "max": 250, "left": 3}));
    assert_eq!(v["shell"]["shellCalls"], 7);
    assert!(report.partial());
}

#[test]
fn a_report_is_partial_only_for_a_cap_a_failed_listing_or_a_failed_read() {
    let source = |listing_failed, failed: BTreeMap<ReadFailure, usize>| SourceReport {
        agent: Agent::Pi,
        listing_failed,
        listed: 0,
        related: 0,
        read: 0,
        failed,
    };
    let report = |sources, limits| HistoryReport {
        sources,
        sessions_read: 0,
        calls: 0,
        shell: Coverage::default(),
        excluded: BTreeMap::new(),
        limits,
        conflicts: ConflictReport::default(),
    };
    let cases = [
        (report(vec![source(false, BTreeMap::new())], vec![]), false),
        (report(vec![source(true, BTreeMap::new())], vec![]), true),
        (report(vec![source(false, BTreeMap::from([(ReadFailure::Failed, 1)]))], vec![]), true),
        (report(vec![], vec![Limit::Calls { max: 5 }]), true),
    ];
    for (r, partial) in cases {
        assert_eq!(r.partial(), partial, "{r:?}");
    }
}

fn scratch_item(risks: Vec<Risk>) -> Item {
    let why = Why::SessionFolder { strength: Strength::Created, from_script: false, activity: Activity::Unknown };
    Item { risks, ..item(Category::Scratch, why, None) }
}

#[test]
fn schema_2_moves_risks_to_the_item() {
    let v = json_of(&scan(vec![scratch_item(vec![Risk::NoGit])], None));
    assert_eq!(v["schema"], json!(2));
    assert_eq!(v["items"][0]["risks"], json!(["no-git"]));
    assert!(v["items"][0]["why"].get("risks").is_none(), "{}", v["items"][0]["why"]);
}

#[test]
fn danger_level_and_new_blocks_are_kebab_case() {
    assert_eq!(json_of(&crate::Level::Danger), json!("danger"));
    let blocks = [
        (Block::OtherProjectWrites, "other-project-writes"),
        (Block::HoldsRepositories, "holds-repositories"),
        (Block::TrackedFiles, "tracked-files"),
    ];
    for (block, name) in blocks {
        assert_eq!(json_of(&block), json!(name));
    }
    let target = json_of(&Target::Subtree { repo: PathBuf::from(r"D:\p") });
    assert_eq!(target, json!({"type": "subtree", "repo": r"D:\p"}));
}

#[test]
fn identity_never_reaches_the_json() {
    let dir = tempfile::tempdir().unwrap();
    let id = crate::identity::folder_id(dir.path());
    let v = json_of(&Item { identity: id, ..scratch_item(Vec::new()) });
    assert!(v.get("identity").is_none(), "{v}");
}

#[test]
fn risk_reasons_are_the_guide_texts() {
    let cases = [
        (Risk::NoGit, "not a git repository"),
        (Risk::WeakEvidence, "the sessions wrote or worked here; no creation is recorded"),
        (Risk::PartialCoverage, "the session history was read only in part"),
        (Risk::RecentActivity, "used or changed in the last 24 hours"),
    ];
    for (risk, text) in cases {
        assert_eq!(risk.reason(), text, "{risk:?}");
    }
}
