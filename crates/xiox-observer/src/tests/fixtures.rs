//! Hand-built scans: no disk, no git.

use std::path::PathBuf;
use xiox_model::{ArtifactKind, Level};
use xiox_project::{Block, Category, Item, Project, ProjectScan, SCHEMA, SearchMode, Target, Why};

pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub fn project() -> Project {
    Project {
        root: PathBuf::from(r"D:\Projects\mukade"),
        name: "mukade".into(),
        is_repo: true,
        origin: None,
        worktrees: vec![],
    }
}

pub fn scan(items: Vec<Item>) -> ProjectScan {
    ProjectScan {
        schema: SCHEMA,
        project: project(),
        items,
        branches: vec![],
        notes: vec![],
        search: SearchMode::BoundedWalk,
        scanned_at_ms: 0,
        elapsed_ms: 0,
    }
}

pub fn item(category: Category, path: &str, bytes: Option<u64>, level: Level, blocked: Vec<Block>) -> Item {
    let p = PathBuf::from(path);
    let (target, why) = match category {
        Category::Worktrees => (
            Target::Worktree { repo: PathBuf::from(r"D:\Projects\mukade") },
            Why::LinkedWorktree { branch: None, head: None },
        ),
        Category::Logs => (Target::File, Why::LogFile { ignored: false }),
        _ => (Target::Dir { kind: ArtifactKind::Catalog }, Why::CatalogFolder { label_en: "Rust build".into() }),
    };
    Item { key: xiox_model::path_key(&p), category, path: Some(p), bytes, level, blocked, why, target }
}

/// A recommended, unblocked build folder two levels deep.
pub fn build(name: &str, bytes: Option<u64>) -> Item {
    item(Category::Builds, &format!(r"D:\Projects\mukade\{name}"), bytes, Level::Recommended, vec![])
}

pub fn worktree(name: &str) -> Item {
    item(Category::Worktrees, &format!(r"D:\Projects\{name}"), None, Level::Optional, vec![])
}
