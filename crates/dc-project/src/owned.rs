//! Builds, worktrees and sessions of a project.

use crate::resolve::resolve_full;
use crate::{
    Block, Category, Env, Error, Item, Note, Options, Project, ProjectScan, SCHEMA, StaleBranch, Target,
    Why,
};
use dc_git::{GitCli, WorktreeEntry};
use dc_model::{
    Agent, Artifact, ArtifactKind, Class, EngineConfig, GitProbe, Level, ProjectReport, ProjectStatus, ScanOptions,
    SessionSource, normalize_path, path_key,
};
use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub(crate) fn analyze_with(path: &Path, opts: &Options, env: &Env) -> Result<ProjectScan, Error> {
    let started = Instant::now();
    let (project, entries) = resolve_full(path)?;
    let mut notes = Vec::new();
    if !project.is_repo {
        notes.push(if dc_git::git_available() { Note::NotARepository } else { Note::GitMissing });
    }

    let reports = engine_reports(&project, env, &mut notes);
    let mut items = builds(&project, &reports, opts);
    let build_dirs: Vec<PathBuf> = items.iter().filter_map(|i| i.path.clone()).collect();
    items.extend(crate::logs::log_items(&project, &build_dirs));
    items.extend(worktrees(&project, &entries, &reports, env, opts, &mut notes));
    items.extend(sessions(&project, &entries, env));
    let found = crate::search::search(&project, opts, env);
    items.extend(found.items);
    notes.extend(found.notes);
    sort_items(&mut items);

    let branches = if project.is_repo {
        dc_git::stale_branches(&project.root)
            .into_iter()
            .map(|b| StaleBranch { name: b.name, merged: b.merged, worktree_missing: b.worktree_missing })
            .collect()
    } else {
        Vec::new()
    };
    Ok(ProjectScan {
        schema: SCHEMA,
        project,
        items,
        branches,
        notes,
        search: found.mode,
        scanned_at_ms: env.now_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Category order, then bytes descending (unmeasured last), then key.
pub(crate) fn sort_items(items: &mut [Item]) {
    items.sort_by(|a, b| {
        (a.category, Reverse(a.bytes), &a.key).cmp(&(b.category, Reverse(b.bytes), &b.key))
    });
}

// ───────────────────────────────── helpers ─────────────────────────────────

/// Component-aware "`child` is `parent` or inside it", on `path_key` strings.
pub fn key_within(child: &str, parent: &str) -> bool {
    if parent.is_empty() {
        return false;
    }
    child == parent
        || child.strip_prefix(parent).is_some_and(|rest| {
            parent.ends_with(['\\', '/']) || rest.starts_with(['\\', '/'])
        })
}

fn within_any(p: &Path, roots: &[String]) -> bool {
    let k = path_key(p);
    roots.iter().any(|r| key_within(&k, r))
}

/// The project root and every existing linked worktree: where the engine looks for builds.
fn scan_roots(project: &Project) -> Vec<PathBuf> {
    std::iter::once(project.root.clone()).chain(project.worktrees.iter().cloned()).collect()
}

pub(crate) fn agent_slug(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Opencode => "opencode",
        Agent::Pi => "pi",
    }
}

/// English label of a catalog folder from its kind (or class and folder name).
fn label_en(a: &Artifact) -> String {
    let fixed = match a.kind {
        ArtifactKind::RustTarget => "Rust build",
        ArtifactKind::NodeModules => "Node modules",
        ArtifactKind::PythonVenv => "Python virtual environment",
        ArtifactKind::PythonCache => "Python cache",
        ArtifactKind::Gradle => "Gradle build",
        ArtifactKind::Maven => "Maven build",
        ArtifactKind::DotNet => ".NET build",
        ArtifactKind::CMake => "CMake build",
        ArtifactKind::WorktreeDir => "Worktree",
        ArtifactKind::Catalog | ArtifactKind::Other | ArtifactKind::ProjectFolder => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    let class = match a.class {
        Class::Cache => "Cache",
        Class::Build => "Build output",
        Class::Dependency => "Dependencies",
        Class::Log => "Logs",
        Class::Personal => "Personal data",
    };
    match a.path.file_name() {
        Some(n) => format!("{class} ({})", n.to_string_lossy()),
        None => class.to_string(),
    }
}

// ───────────────────────────────── builds ──────────────────────────────────

/// The engine over the project root and its existing worktrees. Its warnings are Italian: one
/// `Note::EngineWarning` stands for all of them.
fn engine_reports(project: &Project, env: &Env, notes: &mut Vec<Note>) -> Vec<ProjectReport> {
    let cfg = EngineConfig {
        roots: scan_roots(project),
        container_roots: Vec::new(),
        inactive_days: dc_app::default_settings().inactive_days,
        scan: ScanOptions::default(),
        now_ms: env.now_ms,
    };
    let sources: Vec<&dyn SessionSource> = env.sessions.iter().map(|s| s as &dyn SessionSource).collect();
    let res = dc_engine::analyze(&cfg, &dc_scanner::DefaultScanner, None, &sources, &GitCli);
    if !res.warnings.is_empty() {
        notes.push(Note::EngineWarning);
    }
    res.reports
}

/// Level of a build folder as if the project were inactive. The engine's `assess`
/// (`dc_engine`, private) sets `artifact.level` from `class.base_level()`, then `Caution` when
/// git tracks it, it is not verified, or its name is ambiguous and git does not ignore it; the
/// project's activity only decides `preselected`, never `level`. So the engine's level already
/// is that recomputation; the unverified rule is re-applied here so a scanner placeholder can
/// never come out stronger than the rules allow.
fn build_level(a: &Artifact) -> Level {
    if !a.verified || a.level == Level::Caution {
        Level::Caution
    } else {
        a.level
    }
}

pub(crate) fn builds(project: &Project, reports: &[ProjectReport], opts: &Options) -> Vec<Item> {
    let roots: Vec<String> = scan_roots(project).iter().map(|r| path_key(r)).collect();
    let mut out: Vec<Item> = Vec::new();
    for r in reports.iter().filter(|r| within_any(&r.root, &roots)) {
        let blocked: Vec<Block> = match r.status {
            ProjectStatus::InUse => vec![Block::LiveSession],
            ProjectStatus::Unreachable => vec![Block::Unreachable],
            _ => Vec::new(),
        };
        for a in r.artifacts.iter().filter(|a| a.kind != ArtifactKind::WorktreeDir) {
            let key = path_key(&a.path);
            if out.iter().any(|i| i.key == key) {
                continue;
            }
            out.push(Item {
                key,
                category: Category::Builds,
                path: Some(normalize_path(&a.path)),
                bytes: opts.measure.then(|| a.size.effective_bytes()),
                level: build_level(a),
                blocked: blocked.clone(),
                why: Why::CatalogFolder { label_en: label_en(a) },
                target: Target::Dir { kind: a.kind },
            });
        }
    }
    out
}

// ──────────────────────────────── worktrees ────────────────────────────────

/// A live session in `dir`: an engine report at or inside it is `InUse`, or a live session
/// record works there (a worktree without build folders has no engine report).
fn live_in(dir: &Path, reports: &[ProjectReport], env: &Env) -> bool {
    let k = [path_key(dir)];
    reports.iter().any(|r| r.status == ProjectStatus::InUse && within_any(&r.root, &k))
        || env
            .sessions
            .iter()
            .flat_map(|p| p.loaded.items.iter())
            .any(|s| s.live && within_any(&s.project_path, &k))
}

fn worktrees(
    project: &Project,
    entries: &[WorktreeEntry],
    reports: &[ProjectReport],
    env: &Env,
    opts: &Options,
    notes: &mut Vec<Note>,
) -> Vec<Item> {
    let mut out = Vec::new();
    for e in entries.iter().filter(|e| !e.is_main) {
        let path = normalize_path(&e.path);
        if e.prunable {
            notes.push(Note::PrunableWorktree { path });
            continue;
        }
        let mut blocked = Vec::new();
        if GitCli.state(&path).dirty != Some(false) {
            blocked.push(Block::DirtyGit);
        }
        if dc_git::head_kept_elsewhere(&path) != Some(true) {
            blocked.push(Block::UnpushedCommits);
        }
        if live_in(&path, reports, env) {
            blocked.push(Block::LiveSession);
        }
        out.push(Item {
            key: path_key(&path),
            category: Category::Worktrees,
            bytes: opts.measure.then(|| dc_cleaner::measure_dir(&path).effective_bytes()),
            level: if blocked.is_empty() { Level::Recommended } else { Level::Caution },
            blocked,
            why: Why::LinkedWorktree { branch: e.branch.clone(), head: e.head.clone() },
            target: Target::Worktree { repo: project.root.clone() },
            path: Some(path),
        });
    }
    out
}

// ──────────────────────────────── sessions ─────────────────────────────────

/// Archived sessions whose working folder is the project or one of its worktrees (also a
/// worktree whose folder is gone: the session still belongs to the project).
fn sessions(project: &Project, entries: &[WorktreeEntry], env: &Env) -> Vec<Item> {
    let mut roots = vec![path_key(&project.root)];
    roots.extend(entries.iter().filter(|e| !e.is_main).map(|e| path_key(&e.path)));
    env.archived
        .iter()
        .filter(|s| s.cwd.as_deref().is_some_and(|c| within_any(c, &roots)))
        .map(|s| Item {
            key: format!("session:{}:{}", agent_slug(s.agent), s.id),
            category: Category::Sessions,
            path: None,
            bytes: Some(s.bytes),
            level: Level::Optional,
            blocked: if s.blocked.is_some() { vec![Block::SessionLocked] } else { Vec::new() },
            why: Why::ArchivedSession { agent: s.agent, title: s.title.clone() },
            target: Target::Session { agent: s.agent, id: s.id.clone() },
        })
        .collect()
}
