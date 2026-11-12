//! End-to-end smoke on real data, read-only: nothing is cleaned, no session is removed.
//!
//! `cargo run -p dc-app --release --example smoke`                    projects from agent sessions
//! `cargo run -p dc-app --release --example smoke -- D:\Projects`     also walk that folder in full
//! `... -- admin` builds the MFT index (only works when the shell is elevated).
//! `SMOKE_ROOTS=1` lists the roots found from sessions and the folders skipped, with the reason.
//!
//! The only write is mukade's listing cache in `dc_app::cache_dir()`.
use dc_model::{Agent, DEFAULT_INACTIVE_DAYS, ProjectReport, RemoveMode, Settings};
use std::path::PathBuf;

fn gb(b: u64) -> f64 {
    b as f64 / 1e9
}

fn summary(label: &str, v: &[ProjectReport]) {
    let total: u64 = v.iter().map(|p| p.reclaimable_bytes).sum();
    let pre: u64 = v.iter().filter(|p| p.preselected).map(|p| p.reclaimable_bytes).sum();
    let blocked = v.iter().filter(|p| !p.blocked_by.is_empty()).count();
    println!(
        "{label:<16} {:>4}  reclaimable {:>7.2} GB  preselected {:>7.2} GB  blocked {blocked}",
        v.len(),
        gb(total),
        gb(pre)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let admin = args.iter().any(|a| a == "admin");
    let extra: Vec<PathBuf> = args.iter().filter(|a| *a != "admin").map(PathBuf::from).collect();
    let settings = Settings {
        session_projects: true,
        scan_roots: !extra.is_empty(),
        roots: extra,
        inactive_days: DEFAULT_INACTIVE_DAYS,
        remove_mode: RemoveMode::Recycle,
        all_drives: false,
    };

    let (sessions, _) = dc_app::load_sessions(Some(dc_app::cache_dir()));
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let plan = dc_app::plan_roots(&settings, &sessions, home.as_deref(), &[]);
    let mut by_reason: std::collections::BTreeMap<String, usize> = Default::default();
    for (_, why) in &plan.from_sessions.skipped {
        *by_reason.entry(format!("{why:?}")).or_default() += 1;
    }
    println!(
        "session roots={} skipped={:?} -> scan roots={}",
        plan.from_sessions.roots.len(),
        by_reason,
        plan.roots.len()
    );
    if std::env::var_os("SMOKE_ROOTS").is_some() {
        for r in &plan.roots {
            println!("  root: {}", r.display());
        }
        for (p, why) in &plan.from_sessions.skipped {
            println!("  skip {why:?}: {}", p.display());
        }
    }

    let started = std::time::Instant::now();
    let step = |text: &str, f: f32| println!("  [{:>5.1}s] {:>3.0}% {text}", started.elapsed().as_secs_f32(), f * 100.0);
    let r = dc_app::run_scan(&settings, admin, &step);

    println!("backend={} admin={admin} elapsed={}ms warnings={}", r.backend, r.elapsed_ms, r.warnings.len());
    for w in r.warnings.iter().take(8) {
        println!("  warn: {w}");
    }
    summary("projects", &r.projects);
    summary("worktrees", &r.worktrees);
    summary("other worktrees", &r.other_worktrees);
    for p in r.projects.iter().chain(&r.worktrees).take(10) {
        println!(
            "  {:>7.2} GB  {:<12} pre={:<5} {}  | {}",
            gb(p.reclaimable_bytes),
            format!("{:?}", p.status),
            p.preselected,
            p.root.display(),
            p.reason
        );
    }
    println!("archived sessions:");
    for agent in [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi] {
        let of: Vec<_> = r.archived.iter().filter(|s| s.agent == agent).collect();
        let blocked = of.iter().filter(|s| s.blocked.is_some()).count();
        let bytes: u64 = of.iter().map(|s| s.bytes).sum();
        println!("  {:<9} total {:>4}  blocked {blocked:>4}  {:>7.1} MB", format!("{agent:?}"), of.len(), bytes as f64 / 1e6);
    }
}
