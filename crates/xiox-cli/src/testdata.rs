//! Hand-built scans for the unit tests of `render` and `json`.

use xiox_model::{Agent, ArtifactKind};
use xiox_project::{Activity, Level, Risk, Strength};
use xiox_project::{
    Block, Category, Item, Note, Project, ProjectScan, SCHEMA, SearchMode, StaleBranch, Target, Why,
};
use std::path::PathBuf;

pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub fn project() -> Project {
    Project {
        root: PathBuf::from("D:\\Projects\\mukade"),
        name: "mukade".into(),
        is_repo: true,
        origin: Some("git@GitHub.com:Acme/Mukade.git".into()),
        worktrees: vec![PathBuf::from("D:\\Projects\\mukade-chunked")],
    }
}

pub fn item(category: Category, path: &str, bytes: Option<u64>, level: Level, blocked: Vec<Block>, why: Why) -> Item {
    let p = PathBuf::from(path);
    let target = match category {
        Category::Worktrees => Target::Worktree { repo: PathBuf::from("D:\\Projects\\mukade") },
        Category::Logs => Target::File,
        _ => Target::Dir { kind: ArtifactKind::Catalog },
    };
    Item {
        key: xiox_model::path_key(&p),
        category,
        path: Some(p),
        bytes,
        level,
        risks: Vec::new(),
        identity: None,
        blocked,
        why,
        target,
        provenance: None,
    }
}

pub fn scan(items: Vec<Item>) -> ProjectScan {
    ProjectScan {
        schema: SCHEMA,
        project: project(),
        items,
        branches: Vec::new(),
        notes: Vec::new(),
        search: SearchMode::BoundedWalk,
        scanned_at_ms: 1_700_000_000_000,
        elapsed_ms: 1234,
        history: None,
    }
}

pub fn build() -> Item {
    item(
        Category::Builds,
        "D:\\Projects\\mukade\\target",
        Some(6 * GB),
        Level::Recommended,
        vec![],
        Why::CatalogFolder { label_en: "Rust build".into() },
    )
}

pub fn clean_worktree() -> Item {
    item(
        Category::Worktrees,
        "D:\\Projects\\mukade-chunked",
        Some(265 * MB),
        Level::Recommended,
        vec![],
        Why::LinkedWorktree { branch: None, head: Some("b574bc7e1d0f3a9c2f6a8e4b5c7d9e0f1a2b3c4d".into()) },
    )
}

pub fn blocked_worktree() -> Item {
    item(
        Category::Worktrees,
        "D:\\Projects\\mukade-wip",
        Some(40 * MB),
        Level::Recommended,
        vec![Block::DirtyGit, Block::UnpushedCommits],
        Why::LinkedWorktree { branch: Some("wip".into()), head: None },
    )
}

/// A folder directly under the drive root, matched by name only.
pub fn named_top_level() -> Item {
    item(Category::Named, "D:\\mukade-test-scratch", Some(21 * MB), Level::Caution, vec![], Why::NameOnly)
}

pub fn copy_with_work() -> Item {
    item(
        Category::Copies,
        "D:\\Projects\\old\\mukade-copy",
        Some(900 * MB),
        Level::Caution,
        vec![Block::UnsavedWork],
        Why::SameRepository { shared_root_commit: true, same_origin: true, has_own_work: Some(true) },
    )
}

pub fn session() -> Item {
    Item {
        key: "session:claude:abc-123".into(),
        category: Category::Sessions,
        path: None,
        bytes: Some(3 * MB),
        level: Level::Optional,
        risks: Vec::new(),
        identity: None,
        blocked: vec![],
        why: Why::ArchivedSession { agent: Agent::Claude, title: Some("refactor the parser".into()) },
        target: Target::Session { agent: Agent::Claude, id: "abc-123".into() },
        provenance: None,
    }
}

/// A folder an agent session made, outside any repository: `danger`.
pub fn danger_scratch() -> Item {
    let why = Why::SessionFolder { strength: Strength::Created, from_script: false, activity: Activity::Unknown };
    let mut i = item(Category::Scratch, "D:\\agent-runs\\bench", Some(5 * MB), Level::Danger, vec![], why);
    i.target = Target::Dir { kind: ArtifactKind::ProjectFolder };
    i.risks = vec![Risk::NoGit];
    i
}

/// A clean linked worktree used in the last day: `danger`.
pub fn danger_worktree() -> Item {
    Item { level: Level::Danger, risks: vec![Risk::RecentActivity], ..clean_worktree() }
}

/// A build, a `danger` session folder and a `danger` worktree.
pub fn danger_scan() -> ProjectScan {
    scan(vec![build(), danger_scratch(), danger_worktree()])
}

/// The scan of the reference example: builds, worktrees (one blocked), named, copies, sessions.
pub fn full_scan() -> ProjectScan {
    let mut s = scan(vec![
        build(),
        clean_worktree(),
        blocked_worktree(),
        copy_with_work(),
        named_top_level(),
        session(),
    ]);
    s.branches =
        vec![StaleBranch { name: "old-feature".into(), merged: true, worktree_missing: true }];
    s.notes = vec![Note::PrunableWorktree { path: PathBuf::from("D:\\Projects\\gone") }];
    s
}

/// Every variant of `Block`, `Note` and `Why`, to look for stray text.
pub fn every_variant_scan() -> ProjectScan {
    let all_blocks = vec![
        Block::LiveSession,
        Block::DirtyGit,
        Block::UnpushedCommits,
        Block::Unreachable,
        Block::SessionLocked,
        Block::UnsavedWork,
        Block::UnsavedWorkUnknown,
        Block::Unknown,
    ];
    let mut s = scan(vec![
        build(),
        item(Category::Worktrees, "D:\\Projects\\w1", Some(1), Level::Caution, all_blocks, Why::LinkedWorktree { branch: Some("b".into()), head: None }),
        item(Category::Worktrees, "D:\\Projects\\w2", None, Level::Recommended, vec![], Why::LinkedWorktree { branch: None, head: None }),
        item(Category::Copies, "D:\\Projects\\c1", Some(1), Level::Caution, vec![], Why::SameRepository { shared_root_commit: true, same_origin: true, has_own_work: Some(false) }),
        item(Category::Copies, "D:\\Projects\\c2", Some(1), Level::Caution, vec![], Why::SameRepository { shared_root_commit: false, same_origin: true, has_own_work: None }),
        item(Category::Copies, "D:\\Projects\\c3", Some(1), Level::Caution, vec![], Why::SameRepository { shared_root_commit: true, same_origin: false, has_own_work: Some(true) }),
        named_top_level(),
        item(Category::Logs, "D:\\Projects\\mukade\\a.log", Some(2048), Level::Recommended, vec![], Why::LogFile { ignored: true }),
        item(Category::Logs, "D:\\Projects\\mukade\\b.log", Some(10), Level::Optional, vec![], Why::LogFile { ignored: false }),
        session(),
    ]);
    s.branches = vec![
        StaleBranch { name: "a".into(), merged: true, worktree_missing: false },
        StaleBranch { name: "b".into(), merged: false, worktree_missing: true },
    ];
    s.notes = vec![
        Note::NotARepository,
        Note::GitMissing,
        Note::NameTooShort,
        Note::WalkMayMissDeepFolders,
        Note::PrunableWorktree { path: PathBuf::from("D:\\Projects\\gone") },
        Note::EngineWarning,
        Note::SessionsNotRead,
        Note::LogsNotChecked,
        Note::ContainerRoot,
    ];
    s.search = SearchMode::MftIndex;
    s
}
