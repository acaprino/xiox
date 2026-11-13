//! Human output of `analyze`. Every text is English and built from structured fields: the
//! engine's own strings (`Artifact::note`, `ArchivedSession::blocked`, warnings) are never used.

use dc_model::{Agent, Level};
use dc_project::{
    Block, Category, Item, Note, ProjectScan, SearchMode, Target, Why, key_within, normalize_remote,
};
use std::path::{Component, Path};

const LEVEL_W: usize = 11;
const SIZE_W: usize = 9;
const MIN_WIDTH: usize = 52;
const MAX_PATH_COL: usize = 70;
const MAX_TITLE_CHARS: usize = 60;

/// `6.0 GB`, `265 MB`, `12 KB`, `512 B`: binary units, English, one decimal only for GB.
pub fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kb = bytes as f64 / 1024.0;
    if kb.round() < 1024.0 {
        return format!("{kb:.0} KB");
    }
    let mb = kb / 1024.0;
    if mb.round() < 1024.0 {
        return format!("{mb:.0} MB");
    }
    format!("{:.1} GB", mb / 1024.0)
}

/// True when the guard of the cleaner (`dc_cleaner::guard`) refuses the folder for being
/// directly under a drive root or share root: fewer than two folders below it.
pub fn is_top_level(path: &Path) -> bool {
    path.components().filter(|c| matches!(c, Component::Normal(_))).count() < 2
}

/// English reason of a block.
pub fn block_reason(b: Block) -> &'static str {
    match b {
        Block::LiveSession => "an agent session is open",
        Block::DirtyGit => "uncommitted changes",
        Block::UnpushedCommits => "commits not kept anywhere else",
        Block::Unreachable => "could not be checked",
        Block::SessionLocked => "session locked by its harness",
        Block::UnsavedWork => "has commits or changes the project does not have",
        Block::Unknown => "unknown reason",
    }
}

fn level_name(l: Level) -> &'static str {
    match l {
        Level::Recommended => "recommended",
        Level::Optional => "optional",
        Level::Caution => "caution",
    }
}

fn agent_name(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Opencode => "opencode",
        Agent::Pi => "pi",
    }
}

fn title_of(c: Category) -> &'static str {
    match c {
        Category::Builds => "BUILDS",
        Category::Worktrees => "WORKTREES",
        Category::Copies => "COPIES",
        Category::Named => "NAMED",
        Category::Logs => "LOGS",
        Category::Sessions => "SESSIONS",
    }
}

const ORDER: [Category; 6] = [
    Category::Builds,
    Category::Worktrees,
    Category::Copies,
    Category::Named,
    Category::Logs,
    Category::Sessions,
];

/// One line of free text taken from the user's data (a session title): no control characters, bounded.
fn one_line(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > MAX_TITLE_CHARS {
        let cut: String = flat.chars().take(MAX_TITLE_CHARS).collect();
        format!("{cut}...")
    } else {
        flat
    }
}

fn short_head(head: &str) -> &str {
    head.get(..7).unwrap_or(head)
}

fn why_text(why: &Why) -> String {
    match why {
        Why::CatalogFolder { label_en } => label_en.clone(),
        Why::LinkedWorktree { branch: Some(b), .. } => format!("clean, on branch {b}"),
        Why::LinkedWorktree { branch: None, head: Some(h) } => format!("clean, detached at {}", short_head(h)),
        Why::LinkedWorktree { branch: None, head: None } => "clean".to_string(),
        Why::SameRepository { shared_root_commit, same_origin, has_own_work } => {
            let identity = match (same_origin, shared_root_commit) {
                (true, true) => "same origin and first commit",
                (true, false) => "same origin",
                (false, true) => "same first commit",
                (false, false) => "same repository",
            };
            let work = match has_own_work {
                Some(true) => "has own work",
                Some(false) => "no own work",
                None => "own work not checked",
            };
            format!("copy of the repository: {identity}, {work}")
        }
        Why::NameOnly => "matched by name only".to_string(),
        Why::LogFile { ignored: true } => "log file, ignored by git".to_string(),
        Why::LogFile { ignored: false } => "log file".to_string(),
        Why::ArchivedSession { title, .. } => match title.as_deref().map(one_line).filter(|t| !t.is_empty()) {
            Some(t) => t,
            None => "untitled session".to_string(),
        },
    }
}

fn path_text(item: &Item) -> String {
    match (&item.path, &item.target) {
        (Some(p), _) => p.display().to_string(),
        (None, Target::Session { agent, id }) => format!("{} session {id}", agent_name(*agent)),
        (None, _) => item.key.clone(),
    }
}

/// The guard of `clean` (`dc_cleaner::guard`) refuses any path directly under a drive or share
/// root, whatever the kind of item: it is shown, never offered.
fn too_shallow(item: &Item) -> bool {
    item.path.as_deref().is_some_and(is_top_level)
}

fn reason_text(item: &Item) -> String {
    let mut reason = if item.blocked.is_empty() {
        why_text(&item.why)
    } else {
        item.blocked.iter().map(|b| block_reason(*b)).collect::<Vec<_>>().join(", ")
    };
    if too_shallow(item) {
        reason.push_str("; cannot be removed: top-level folder");
    }
    reason
}

/// Sum of the sizes of `items`, an item inside another one counted once (with the outer one);
/// an unmeasured item counts 0.
fn sum_bytes(items: &[&Item]) -> u64 {
    items
        .iter()
        .filter(|i| {
            !(i.path.is_some()
                && items.iter().any(|o| o.path.is_some() && o.key != i.key && key_within(&i.key, &o.key)))
        })
        .map(|i| i.bytes.unwrap_or(0))
        .sum()
}

fn is_recommended(i: &Item) -> bool {
    i.level == Level::Recommended && i.blocked.is_empty() && !too_shallow(i)
}

fn note_text(n: &Note, scan: &ProjectScan) -> Option<String> {
    Some(match n {
        Note::NotARepository => {
            "not a git repository, so worktrees and copies of the repository cannot be identified".to_string()
        }
        Note::GitMissing => "git was not found on this PC: worktrees and copies were not checked".to_string(),
        Note::NameTooShort => {
            "the project name is too short to match folders by name; only copies of the repository are looked for"
                .to_string()
        }
        // The search line already says so when the walk is the only search.
        Note::WalkMayMissDeepFolders if scan.search == SearchMode::BoundedWalk => return None,
        Note::WalkMayMissDeepFolders => {
            "some drives were searched only to 3 levels (no MFT index): deeper folders may be missed".to_string()
        }
        Note::PrunableWorktree { path } => format!(
            "git still lists a worktree whose folder is gone: {} (git worktree prune clears it)",
            path.display()
        ),
        Note::EngineWarning => "the analysis reported warnings; the listing may be incomplete".to_string(),
    })
}

pub fn render_scan(scan: &ProjectScan) -> String {
    let mut o = String::new();
    let p = &scan.project;
    let kind = if !p.is_repo {
        "not a git repository".to_string()
    } else if let Some(origin) = &p.origin {
        format!("git, origin {}", normalize_remote(origin))
    } else {
        "git, no origin".to_string()
    };
    o.push_str(&format!("Project  {}   {}   ({kind})\n", p.name, p.root.display()));
    let search = match scan.search {
        SearchMode::MftIndex => "all fixed drives, MFT index",
        SearchMode::BoundedWalk => {
            "fixed drives to 3 levels (deeper folders may be missed; an elevated terminal uses the MFT index)"
        }
        SearchMode::ParentOnly => "project's parent only",
    };
    o.push_str(&format!("Search   {search}\n"));

    // Column widths shared by every block.
    let path_w = scan.items.iter().map(|i| path_text(i).chars().count()).max().unwrap_or(0).min(MAX_PATH_COL);
    let width = MIN_WIDTH.max(2 + LEVEL_W + 2 + SIZE_W + 2 + path_w);

    for category in ORDER {
        let items: Vec<&Item> = scan.items.iter().filter(|i| i.category == category).collect();
        if items.is_empty() {
            continue;
        }
        let title = title_of(category);
        let total = fmt_size(sum_bytes(&items));
        let pad = width.saturating_sub(title.len()).max(total.len() + 1);
        o.push_str(&format!("\n{title}{total:>pad$}\n"));
        for item in items {
            let level = if item.blocked.is_empty() { level_name(item.level) } else { "blocked" };
            let size = item.bytes.map_or_else(|| "-".to_string(), fmt_size);
            let line =
                format!("  {level:<LEVEL_W$}  {size:>SIZE_W$}  {:<path_w$}  {}", path_text(item), reason_text(item));
            o.push_str(line.trim_end());
            o.push('\n');
        }
    }

    if !scan.branches.is_empty() {
        let name_w = scan.branches.iter().map(|b| b.name.chars().count()).max().unwrap_or(0);
        o.push_str("\nBRANCHES (not removed)\n");
        for b in &scan.branches {
            let mut facts = vec![if b.merged { "merged" } else { "not merged" }];
            if b.worktree_missing {
                facts.push("its worktree folder is gone");
            }
            o.push_str(&format!("  {:<name_w$}  {}\n", b.name, facts.join(", ")));
        }
    }

    let notes: Vec<String> = scan.notes.iter().filter_map(|n| note_text(n, scan)).collect();
    if !notes.is_empty() {
        o.push('\n');
        for n in notes {
            o.push_str(&format!("note: {n}\n"));
        }
    }

    o.push('\n');
    if scan.items.is_empty() {
        o.push_str("Nothing found for this project.\n");
        return o;
    }
    let all: Vec<&Item> = scan.items.iter().collect();
    let recommended: Vec<&Item> = scan.items.iter().filter(|i| is_recommended(i)).collect();
    o.push_str(&format!(
        "Total {} · recommended {}\n",
        fmt_size(sum_bytes(&all)),
        fmt_size(sum_bytes(&recommended))
    ));
    let flags: Vec<&str> = [
        (Category::Builds, "--builds"),
        (Category::Worktrees, "--worktrees"),
        (Category::Logs, "--logs"),
        (Category::Sessions, "--sessions"),
    ]
    .iter()
    .filter(|(c, _)| recommended.iter().any(|i| i.category == *c))
    .map(|(_, f)| *f)
    .collect();
    if !flags.is_empty() {
        o.push_str(&format!("Clean with: xiox clean \"{}\" {}\n", p.root.display(), flags.join(" ")));
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::*;

    fn line_with<'a>(text: &'a str, needle: &str) -> &'a str {
        text.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("no line with {needle:?} in:\n{text}"))
    }

    #[test]
    fn text_has_one_block_per_non_empty_category_and_the_clean_hint() {
        let out = render_scan(&full_scan());
        for title in ["BUILDS", "WORKTREES", "COPIES", "NAMED", "SESSIONS"] {
            assert_eq!(out.lines().filter(|l| l.starts_with(title)).count(), 1, "{title}:\n{out}");
        }
        assert!(!out.lines().any(|l| l.starts_with("LOGS")), "{out}");
        assert!(out.contains("BRANCHES (not removed)"), "{out}");
        assert!(out.lines().next().unwrap().starts_with("Project  mukade   D:\\Projects\\mukade   (git, origin github.com/acme/mukade)"), "{out}");
        assert!(line_with(&out, "Search   ").contains("fixed drives to 3 levels"), "{out}");
        assert!(line_with(&out, "target").trim_start().starts_with("recommended"), "{out}");
        assert!(line_with(&out, "D:\\Projects\\mukade\\target").contains("6.0 GB"), "{out}");
        assert!(line_with(&out, "mukade-chunked").contains("265 MB"), "{out}");
        assert!(line_with(&out, "mukade-chunked").contains("clean, detached at b574bc7"), "{out}");
        assert!(line_with(&out, "mukade-chunked").trim_start().starts_with("recommended"), "{out}");
        assert!(line_with(&out, "abc-123").contains("refactor the parser"), "{out}");
        // Total (every item) and recommended (the build and the clean worktree), right-aligned titles.
        assert!(out.contains("Total 7.2 GB · recommended 6.3 GB"), "{out}");
        let last = out.trim_end().lines().last().unwrap();
        assert_eq!(last, "Clean with: xiox clean \"D:\\Projects\\mukade\" --builds --worktrees", "{out}");
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn blocked_worktree_shows_blocked_and_its_reason_and_is_not_in_the_hint() {
        // Recommended by level but blocked: still blocked, never in the hint.
        let out = render_scan(&scan(vec![blocked_worktree()]));
        let l = line_with(&out, "mukade-wip");
        assert!(l.trim_start().starts_with("blocked"), "{l}");
        assert!(!l.contains("recommended"), "{l}");
        assert!(l.contains("uncommitted changes"), "{l}");
        assert!(l.contains("commits not kept anywhere else"), "{l}");
        assert!(!out.contains("Clean with:"), "{out}");
        // Next to a clean one the hint exists and names the category once.
        let out = render_scan(&scan(vec![blocked_worktree(), clean_worktree()]));
        assert!(out.trim_end().ends_with("--worktrees"), "{out}");
        assert_eq!(out.matches("--worktrees").count(), 1, "{out}");
    }

    #[test]
    fn every_block_has_its_own_english_reason() {
        let all = [
            Block::LiveSession,
            Block::DirtyGit,
            Block::UnpushedCommits,
            Block::Unreachable,
            Block::SessionLocked,
            Block::UnsavedWork,
            Block::Unknown,
        ];
        let reasons: std::collections::HashSet<_> = all.iter().map(|b| block_reason(*b)).collect();
        assert_eq!(reasons.len(), all.len());
        assert_eq!(block_reason(Block::LiveSession), "an agent session is open");
        assert_eq!(block_reason(Block::SessionLocked), "session locked by its harness");
        assert_eq!(block_reason(Block::UnsavedWork), "has commits or changes the project does not have");
        let out = render_scan(&scan(vec![copy_with_work()]));
        let l = line_with(&out, "mukade-copy");
        assert!(l.trim_start().starts_with("blocked"), "{l}");
        assert!(l.contains("has commits or changes the project does not have"), "{l}");
    }

    #[test]
    fn no_recommended_item_means_no_clean_hint() {
        let out = render_scan(&scan(vec![named_top_level(), session()]));
        assert!(!out.contains("Clean with:"), "{out}");
        assert!(out.contains("Total 24 MB · recommended 0 B"), "{out}");
        let empty = render_scan(&scan(vec![]));
        assert!(!empty.contains("Clean with:"), "{empty}");
        assert!(empty.contains("Nothing found"), "{empty}");
    }

    #[test]
    fn top_level_candidates_are_marked() {
        let out = render_scan(&full_scan());
        assert!(line_with(&out, "mukade-test-scratch").contains("matched by name only"), "{out}");
        assert!(line_with(&out, "mukade-test-scratch").contains("cannot be removed: top-level folder"), "{out}");
        assert!(!line_with(&out, "mukade-copy").contains("top-level"), "{out}");
        assert!(!line_with(&out, "mukade-chunked").contains("top-level"), "{out}");
        assert!(is_top_level(Path::new("D:\\foo")));
        assert!(is_top_level(Path::new("D:\\")));
        assert!(is_top_level(Path::new("\\\\server\\share\\foo")));
        assert!(!is_top_level(Path::new("D:\\a\\b")));
        assert!(!is_top_level(Path::new("\\\\server\\share\\a\\b")));
    }

    #[test]
    fn top_level_worktree_and_log_are_marked_and_never_in_the_hint() {
        let wt = item(
            Category::Worktrees,
            r"D:\mukade-wt",
            Some(10 * MB),
            Level::Recommended,
            vec![],
            Why::LinkedWorktree { branch: Some("x".into()), head: None },
        );
        let log = item(Category::Logs, r"D:\app.log", Some(MB), Level::Recommended, vec![], Why::LogFile { ignored: false });
        let out = render_scan(&scan(vec![wt.clone(), log]));
        for needle in ["mukade-wt", "app.log"] {
            let l = line_with(&out, needle);
            assert!(l.contains("cannot be removed: top-level folder"), "{l}");
        }
        assert!(!out.contains("Clean with:"), "{out}");
        assert!(out.contains("recommended 0 B"), "{out}");
        // With a deep recommended item next to them, only that item's category is in the hint.
        let out = render_scan(&scan(vec![wt, build()]));
        assert!(out.trim_end().ends_with("--builds"), "{out}");
        assert!(!out.contains("--worktrees"), "{out}");
    }

    #[test]
    fn sizes_are_english_and_binary() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1023), "1023 B");
        assert_eq!(fmt_size(1024), "1 KB");
        assert_eq!(fmt_size(265 * MB), "265 MB");
        assert_eq!(fmt_size(1024 * MB - 1), "1.0 GB");
        assert_eq!(fmt_size(6 * GB), "6.0 GB");
        assert_eq!(fmt_size(6 * GB + 300 * MB), "6.3 GB");
        assert!(!fmt_size(GB + 512 * MB).contains(','));
    }

    #[test]
    fn unmeasured_items_show_a_dash_and_nested_items_count_once() {
        let mut inner = build();
        inner.path = Some("D:\\Projects\\mukade-chunked\\target".into());
        inner.key = dc_model::path_key(inner.path.as_ref().unwrap());
        inner.bytes = Some(100 * MB);
        let mut unmeasured = named_top_level();
        unmeasured.bytes = None;
        let out = render_scan(&scan(vec![clean_worktree(), inner, unmeasured]));
        let l = line_with(&out, "mukade-test-scratch");
        assert!(l.contains(" - "), "{l}");
        // The folder inside the worktree is in the listing but not added twice.
        assert!(out.contains("Total 265 MB · recommended 265 MB"), "{out}");
    }

    #[test]
    fn notes_and_search_line_follow_the_structured_fields() {
        let mut s = full_scan();
        s.search = dc_project::SearchMode::ParentOnly;
        s.project.is_repo = false;
        s.project.origin = None;
        s.notes = vec![Note::NameTooShort, Note::EngineWarning];
        let out = render_scan(&s);
        assert!(out.contains("(not a git repository)"), "{out}");
        assert!(line_with(&out, "Search   ").contains("project's parent only"), "{out}");
        assert!(out.contains("note: "), "{out}");
        assert!(out.contains("too short"), "{out}");
        let mut s = full_scan();
        s.search = dc_project::SearchMode::MftIndex;
        assert!(line_with(&render_scan(&s), "Search   ").contains("MFT index"));
    }

    #[test]
    fn no_italian_in_the_output() {
        let out = render_scan(&every_variant_scan());
        let mut all = out.to_lowercase();
        all.push_str(&crate::args::help().to_lowercase());
        for w in ["consigliato", "attenzione", "cartella", "sessione", "pulisci", "progetto", "errore", "cestino", "percorso"] {
            assert!(!all.contains(w), "{w:?} in:\n{out}");
        }
        // The meaningful part: every variant was rendered (blocked, notes, branches, each why).
        for needle in ["blocked", "note: ", "BRANCHES", "log file", "first commit", "own work", "session"] {
            assert!(out.contains(needle), "{needle:?} missing in:\n{out}");
        }
    }
}
