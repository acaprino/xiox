//! Human output of `analyze` and `clean`. Every text is English and built from structured fields: the
//! engine's own strings (`Artifact::note`, `ArchivedSession::blocked`, warnings) are never used.

use dc_model::{Agent, Level, RemoveMode};
use dc_project::{
    Block, Category, Chosen, CleanOutcome, Item, Note, Outcome, ProjectScan, SearchMode, Target, Why, key_within,
    normalize_remote,
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

/// English reason of a block: the one wording of `dc_project`, also used in its errors.
pub fn block_reason(b: Block) -> &'static str {
    b.reason()
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
        Note::SessionsNotRead => {
            "the sessions of this PC could not be read: a live agent session may be missed, and archived \
sessions may be missing; 'clean' leaves worktrees, copies and folders alone until they can be read"
                .to_string()
        }
        Note::LogsNotChecked => {
            "git could not list the files of the project or of a worktree: its log files are not in the listing"
                .to_string()
        }
        Note::ContainerRoot => "this folder is a drive root, your user profile, or holds several projects: \
'xiox clean' refuses --builds, --worktrees, --logs and --sessions here (run it on one project, or name a path with --include)"
            .to_string(),
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
    let container = scan.notes.contains(&Note::ContainerRoot);
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
    // The flags are refused on a container root: no hint that would only fail.
    if !flags.is_empty() && !container {
        o.push_str(&format!("Clean with: xiox clean \"{}\" {}\n", p.root.display(), flags.join(" ")));
    }
    o
}

// ───────────────────────────── clean ─────────────────────────────

/// The items `chosen` names, in the order of `chosen.keys`.
fn chosen_items<'a>(scan: &'a ProjectScan, chosen: &Chosen) -> Vec<&'a Item> {
    chosen.keys.iter().filter_map(|k| scan.items.iter().find(|i| i.key == *k)).collect()
}

/// `Recycle Bin on C: is too small (3 MB to move, 1 MB free). Use --permanent, or empty the bin.`
fn shortfall_text(drive: char, needed: u64, room: u64) -> String {
    format!(
        "Recycle Bin on {drive}: is too small ({} to move, {} free). Use --permanent, or empty the bin.",
        fmt_size(needed),
        fmt_size(room)
    )
}

/// What is about to be removed, shown before the question: the chosen items by category with
/// their sizes, the total, where they go (`to the Recycle Bin` or `permanently`), the worktree
/// rule, every folder that was named with `--include` (copies and name-matched folders can only
/// be chosen that way), the folders the guard will refuse, and the bins that are too small.
pub fn render_summary(scan: &ProjectScan, chosen: &Chosen, mode: RemoveMode, shortfall: &[(char, u64, u64)]) -> String {
    let items = chosen_items(scan, chosen);
    let mut o = String::new();
    let path_w = items.iter().map(|i| path_text(i).chars().count()).max().unwrap_or(0).min(MAX_PATH_COL);
    let width = MIN_WIDTH.max(2 + SIZE_W + 2 + path_w);
    o.push_str("To be removed:\n");
    for category in ORDER {
        let of: Vec<&Item> = items.iter().copied().filter(|i| i.category == category).collect();
        if of.is_empty() {
            continue;
        }
        let title = title_of(category);
        let total = fmt_size(sum_bytes(&of));
        let pad = width.saturating_sub(title.len()).max(total.len() + 1);
        o.push_str(&format!("\n{title}{total:>pad$}\n"));
        for item in of {
            let size = item.bytes.map_or_else(|| "-".to_string(), fmt_size);
            let line = format!("  {size:>SIZE_W$}  {:<path_w$}  {}", path_text(item), why_text(&item.why));
            o.push_str(line.trim_end());
            o.push('\n');
        }
    }

    // Copies and name-matched folders are never taken by a category: each one was named.
    let named: Vec<&Item> =
        items.iter().copied().filter(|i| matches!(i.category, Category::Copies | Category::Named)).collect();
    if !named.is_empty() {
        o.push_str("\nNamed with --include (check each one before you go on):\n");
        for i in named {
            let note = match &i.why {
                Why::SameRepository { has_own_work: Some(false), .. } => {
                    "copy of the repository, no unsaved work found".to_string()
                }
                Why::SameRepository { has_own_work: Some(true), .. } => {
                    "copy of the repository WITH its own work".to_string()
                }
                Why::SameRepository { has_own_work: None, .. } => {
                    "copy of the repository: could not check for unsaved work".to_string()
                }
                Why::NameOnly => "matched by name only: nothing else shows it belongs to the project".to_string(),
                other => why_text(other),
            };
            o.push_str(&format!("  {}  {note}\n", path_text(i)));
        }
    }

    let shallow: Vec<&Item> = items.iter().copied().filter(|i| too_shallow(i)).collect();
    if !shallow.is_empty() {
        o.push_str(
            "\nWarning: these are top-level folders (directly under a drive or share root). \
The safety guard refuses them, so they will be skipped:\n",
        );
        for i in shallow {
            o.push_str(&format!("  {}\n", path_text(i)));
        }
    }

    let count = items.len();
    o.push_str(&format!(
        "\nTotal {} in {count} item{}, {}.\n",
        fmt_size(chosen.bytes),
        if count == 1 { "" } else { "s" },
        match mode {
            RemoveMode::Recycle => "to the Recycle Bin",
            RemoveMode::Permanent => "permanently",
        }
    ));
    if mode == RemoveMode::Recycle && !chosen.permanent_anyway.is_empty() {
        o.push_str("Worktrees are removed by git and do not go to the Recycle Bin.\n");
    }
    for (drive, needed, room) in shortfall {
        o.push_str(&shortfall_text(*drive, *needed, *room));
        o.push('\n');
    }
    o
}

/// `claude session abc-123` for `session:claude:abc-123`; the path of any other item.
fn label_of(key: &str, path: &Option<std::path::PathBuf>) -> String {
    if let Some(p) = path {
        return p.display().to_string();
    }
    match key.strip_prefix("session:").and_then(|rest| rest.split_once(':')) {
        Some((agent, id)) => format!("{agent} session {id}"),
        None => key.to_string(),
    }
}

const STATUS_W: usize = 12;

/// What happened, item by item and in total. Every text is English (the reasons come from
/// `dc_project` already in English).
pub fn render_outcome(o: &CleanOutcome) -> String {
    let mut out = String::new();
    for (key, path, _category, outcome) in &o.items {
        let label = label_of(key, path);
        let line = match outcome {
            Outcome::Removed { bytes_freed } => {
                let status = if o.dry_run { "would remove" } else { "removed" };
                format!("  {status:<STATUS_W$}  {:>SIZE_W$}  {label}", fmt_size(*bytes_freed))
            }
            Outcome::Skipped { reason } => format!("  {:<STATUS_W$}  {:>SIZE_W$}  {label}  ({reason})", "skipped", "-"),
            Outcome::Failed { error } => format!("  {:<STATUS_W$}  {:>SIZE_W$}  {label}  ({error})", "failed", "-"),
        };
        out.push_str(&line);
        out.push('\n');
    }
    if o.dry_run {
        out.push_str(&format!("Would free {}\n", fmt_size(o.bytes_freed)));
    } else if o.recycled_bytes > 0 {
        out.push_str(&format!(
            "Freed {} ({} in the Recycle Bin: freed when it is emptied)\n",
            fmt_size(o.bytes_freed),
            fmt_size(o.recycled_bytes)
        ));
    } else {
        out.push_str(&format!("Freed {}\n", fmt_size(o.bytes_freed)));
    }
    let n = o.not_removed();
    if n > 0 {
        out.push_str(&format!(
            "Not everything was removed: {n} item{} skipped or failed.\n",
            if n == 1 { "" } else { "s" }
        ));
    }
    out
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
        assert!(
            out.lines()
                .next()
                .unwrap()
                .starts_with("Project  mukade   D:\\Projects\\mukade   (git, origin github.com/acme/mukade)"),
            "{out}"
        );
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
            Block::UnsavedWorkUnknown,
            Block::Unknown,
        ];
        let reasons: std::collections::HashSet<_> = all.iter().map(|b| block_reason(*b)).collect();
        assert_eq!(reasons.len(), all.len());
        assert_eq!(block_reason(Block::LiveSession), "an agent session is open");
        assert_eq!(block_reason(Block::SessionLocked), "session locked by its harness");
        assert_eq!(block_reason(Block::UnsavedWork), "has commits or changes the project does not have");
        assert_eq!(block_reason(Block::UnsavedWorkUnknown), "could not be checked for unsaved work");
        // One wording for the listing and for the errors of `--include`.
        for b in all {
            assert_eq!(block_reason(b), b.to_string());
        }
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
    fn a_container_root_says_so_and_offers_no_clean_hint() {
        let mut s = full_scan();
        s.notes = vec![Note::ContainerRoot];
        let out = render_scan(&s);
        assert!(
            out.contains("note: this folder is a drive root, your user profile, or holds several projects"),
            "{out}"
        );
        assert!(!out.contains("Clean with:"), "{out}");
        // Without the note the hint is there.
        assert!(render_scan(&full_scan()).contains("Clean with:"));
    }

    #[test]
    fn failed_session_and_log_reads_are_notes() {
        let mut s = full_scan();
        s.notes = vec![Note::SessionsNotRead, Note::LogsNotChecked];
        let out = render_scan(&s);
        assert!(out.contains("note: the sessions of this PC could not be read"), "{out}");
        assert!(out.contains("note: git could not list the files of the project or of a worktree"), "{out}");
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

    // ── clean: summary and outcome ──

    fn chosen_of(scan: &ProjectScan, keys: &[&str]) -> Chosen {
        let items: Vec<&Item> = scan.items.iter().filter(|i| keys.iter().any(|k| *k == i.key)).collect();
        Chosen {
            keys: items.iter().map(|i| i.key.clone()).collect(),
            bytes: items.iter().map(|i| i.bytes.unwrap_or(0)).sum(),
            permanent_anyway: items
                .iter()
                .filter(|i| matches!(i.target, Target::Worktree { .. }))
                .map(|i| i.key.clone())
                .collect(),
        }
    }

    fn all_keys(scan: &ProjectScan) -> Vec<String> {
        scan.items.iter().map(|i| i.key.clone()).collect()
    }

    #[test]
    fn summary_mentions_the_recycle_bin_or_permanently() {
        let s = scan(vec![build(), session()]);
        let c = chosen_of(&s, &all_keys(&s).iter().map(String::as_str).collect::<Vec<_>>());
        let recycle = render_summary(&s, &c, RemoveMode::Recycle, &[]);
        assert!(recycle.contains("to the Recycle Bin"), "{recycle}");
        assert!(!recycle.contains("permanently"), "{recycle}");
        assert!(recycle.contains("Total 6.0 GB in 2 items, to the Recycle Bin."), "{recycle}");
        assert!(line_with(&recycle, "D:\\Projects\\mukade\\target").contains("Rust build"), "{recycle}");
        assert!(line_with(&recycle, "claude session abc-123").contains("refactor the parser"), "{recycle}");
        assert!(recycle.lines().any(|l| l.starts_with("BUILDS") && l.ends_with("6.0 GB")), "{recycle}");
        // No worktree: no worktree rule.
        assert!(!recycle.contains("removed by git"), "{recycle}");

        let permanent = render_summary(&s, &c, RemoveMode::Permanent, &[]);
        assert!(permanent.contains("Total 6.0 GB in 2 items, permanently."), "{permanent}");
        assert!(!permanent.contains("Recycle Bin"), "{permanent}");
    }

    #[test]
    fn summary_says_worktrees_never_go_to_the_bin_only_when_there_is_one_and_the_bin_is_used() {
        let s = scan(vec![build(), clean_worktree()]);
        let both = chosen_of(&s, &all_keys(&s).iter().map(String::as_str).collect::<Vec<_>>());
        let rule = "Worktrees are removed by git and do not go to the Recycle Bin.";
        assert!(render_summary(&s, &both, RemoveMode::Recycle, &[]).contains(rule));
        assert!(!render_summary(&s, &both, RemoveMode::Permanent, &[]).contains(rule));
        let only_build = chosen_of(&s, &[s.items[0].key.as_str()]);
        assert!(!render_summary(&s, &only_build, RemoveMode::Recycle, &[]).contains(rule));
    }

    #[test]
    fn summary_flags_included_folders_unchecked_copies_and_top_level_folders() {
        let mut unchecked = copy_with_work();
        unchecked.blocked.clear();
        unchecked.why = Why::SameRepository { shared_root_commit: true, same_origin: true, has_own_work: None };
        let mut checked = copy_with_work();
        checked.blocked.clear();
        checked.key = dc_model::path_key(std::path::Path::new("D:\\Projects\\old\\other-copy"));
        checked.path = Some(std::path::PathBuf::from("D:\\Projects\\old\\other-copy"));
        checked.why = Why::SameRepository { shared_root_commit: true, same_origin: true, has_own_work: Some(false) };
        let s = scan(vec![build(), unchecked, checked, named_top_level()]);
        let c = chosen_of(&s, &all_keys(&s).iter().map(String::as_str).collect::<Vec<_>>());
        let out = render_summary(&s, &c, RemoveMode::Permanent, &[]);
        assert!(out.contains("Named with --include"), "{out}");
        let block = out.split("Named with --include").nth(1).unwrap();
        assert!(line_with(block, "mukade-copy").contains("could not check for unsaved work"), "{out}");
        assert!(line_with(block, "other-copy").contains("no unsaved work found"), "{out}");
        assert!(line_with(block, "mukade-test-scratch").contains("matched by name only"), "{out}");
        // The build is not an --include-only item.
        assert!(!block.split("Warning").next().unwrap().contains("Projects\\mukade\\target"), "{out}");
        // D:\mukade-test-scratch is directly under the drive root: the guard refuses it.
        let warning = out.split("Warning:").nth(1).expect("a warning");
        assert!(warning.contains("will be skipped"), "{out}");
        assert!(warning.contains("D:\\mukade-test-scratch"), "{out}");
        assert!(!warning.contains("mukade-copy"), "{out}");
    }

    #[test]
    fn summary_without_a_top_level_folder_or_an_include_has_neither_warning() {
        let s = scan(vec![build()]);
        let c = chosen_of(&s, &all_keys(&s).iter().map(String::as_str).collect::<Vec<_>>());
        let out = render_summary(&s, &c, RemoveMode::Recycle, &[]);
        assert!(!out.contains("Warning") && !out.contains("--include"), "{out}");
        assert!(out.contains("Total 6.0 GB in 1 item, to the Recycle Bin."), "{out}");
    }

    #[test]
    fn summary_carries_the_recycle_bin_shortfall() {
        let s = scan(vec![build()]);
        let c = chosen_of(&s, &[s.items[0].key.as_str()]);
        let out = render_summary(&s, &c, RemoveMode::Recycle, &[('D', 6 * GB, 2 * MB)]);
        assert!(
            out.contains("Recycle Bin on D: is too small (6.0 GB to move, 2 MB free). Use --permanent, or empty the bin."),
            "{out}"
        );
    }

    fn outcome(dry_run: bool, items: Vec<(String, Option<std::path::PathBuf>, Category, Outcome)>) -> CleanOutcome {
        let freed = items
            .iter()
            .map(|(.., o)| if let Outcome::Removed { bytes_freed } = o { *bytes_freed } else { 0 })
            .sum();
        CleanOutcome {
            schema: 1,
            mode: RemoveMode::Permanent,
            dry_run,
            items,
            bytes_freed: freed,
            recycled_bytes: 0,
            elapsed_ms: 5,
        }
    }

    fn entry(path: &str, category: Category, o: Outcome) -> (String, Option<std::path::PathBuf>, Category, Outcome) {
        (dc_model::path_key(std::path::Path::new(path)), Some(std::path::PathBuf::from(path)), category, o)
    }

    #[test]
    fn outcome_lists_skipped_items_with_their_reason() {
        let o = outcome(
            false,
            vec![
                entry("D:\\p\\target", Category::Builds, Outcome::Removed { bytes_freed: 3 * MB }),
                entry("D:\\p\\wt", Category::Worktrees, Outcome::Skipped { reason: "now blocked: uncommitted changes".into() }),
                entry("D:\\p\\old", Category::Named, Outcome::Failed { error: "access denied (os error 5)".into() }),
                (
                    "session:claude:abc-123".into(),
                    None,
                    Category::Sessions,
                    Outcome::Skipped { reason: "no longer archived, or in use now".into() },
                ),
            ],
        );
        let out = render_outcome(&o);
        assert!(line_with(&out, "D:\\p\\target").contains("removed") && line_with(&out, "D:\\p\\target").contains("3 MB"), "{out}");
        let wt = line_with(&out, "D:\\p\\wt");
        assert!(wt.contains("skipped") && wt.contains("(now blocked: uncommitted changes)"), "{out}");
        let old = line_with(&out, "D:\\p\\old");
        assert!(old.contains("failed") && old.contains("(access denied (os error 5))"), "{out}");
        assert!(line_with(&out, "claude session abc-123").contains("no longer archived"), "{out}");
        assert!(out.contains("Freed 3 MB\n"), "{out}");
        assert!(out.contains("Not everything was removed: 3 items skipped or failed."), "{out}");
    }

    #[test]
    fn outcome_says_what_goes_to_the_bin_what_a_dry_run_would_free_and_ignores_overlap() {
        let mut o = outcome(false, vec![entry("D:\\p\\target", Category::Builds, Outcome::Removed { bytes_freed: 3 * MB })]);
        o.mode = RemoveMode::Recycle;
        o.recycled_bytes = 3 * MB;
        assert!(render_outcome(&o).contains("Freed 3 MB (3 MB in the Recycle Bin: freed when it is emptied)"));

        let dry = outcome(true, vec![entry("D:\\p\\target", Category::Builds, Outcome::Removed { bytes_freed: 3 * MB })]);
        let out = render_outcome(&dry);
        assert!(out.contains("would remove") && out.contains("Would free 3 MB"), "{out}");
        assert!(!out.contains("Freed"), "{out}");

        // An item that went with the folder holding it is the ordinary overlap, not a failure.
        let overlap = outcome(
            false,
            vec![
                entry("D:\\p\\target", Category::Builds, Outcome::Removed { bytes_freed: MB }),
                entry(
                    "D:\\p\\target\\debug",
                    Category::Builds,
                    Outcome::Skipped { reason: "already removed with D:\\p\\target".into() },
                ),
            ],
        );
        let out = render_outcome(&overlap);
        assert!(out.contains("already removed with") && !out.contains("Not everything"), "{out}");

        // The count of "skipped or failed" leaves the overlap out and keeps the real ones.
        let mut mixed = overlap;
        mixed.items.push(entry("D:\\p\\x",Category::Builds, Outcome::Skipped { reason: "no longer there".into() }));
        let out = render_outcome(&mixed);
        assert!(out.contains("Not everything was removed: 1 item skipped or failed."), "{out}");
    }
}
