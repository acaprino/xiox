//! Measurement probe, throwaway code: how long the session-history analysis takes on a project
//! and what the budget cuts. It runs the same analysis as `xiox analyze`, in light mode.
//!
//!     cargo run -j 1 -p xiox-project --example session_probe -- <project> [--sample N] [--max-sessions N] [--max-secs N]
//!
//! Stdout: one JSON object with `history`, the number of items per category and `elapsedMs`.
//! `--sample N` writes to stderr, for the first N `scratch` items and copies with provenance, the
//! path, the strength, the session ids and the operations `{source, outcome}`. Command text and
//! session content never reach the output. The session cache is written under the temporary
//! folder, not into Xiox's own.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use serde_json::{json, Value};
use xiox_project::{history_budget, history_probe, Category, Item};

struct Args {
    project: PathBuf,
    sample: usize,
    max_sessions: Option<usize>,
    max_secs: Option<u64>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut project = None;
        let (mut sample, mut max_sessions, mut max_secs) = (0, None, None);
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut number = |flag: &str| {
                args.next().and_then(|v| v.parse::<u64>().ok()).ok_or(format!("{flag} needs a number"))
            };
            match arg.as_str() {
                "--sample" => sample = number("--sample")? as usize,
                "--max-sessions" => max_sessions = Some(number("--max-sessions")? as usize),
                "--max-secs" => max_secs = Some(number("--max-secs")?),
                flag if flag.starts_with("--") => return Err(format!("unknown argument {flag}")),
                _ if project.is_some() => return Err("one project only".into()),
                path => project = Some(PathBuf::from(path)),
            }
        }
        let project = project.ok_or("usage: session_probe <project> [--sample N] [--max-sessions N] [--max-secs N]")?;
        Ok(Args { project, sample, max_sessions, max_secs })
    }

    /// The analysis budget with the flags applied; `None` when none was given.
    fn budget(&self) -> Option<(usize, usize, u64)> {
        if self.max_sessions.is_none() && self.max_secs.is_none() {
            return None;
        }
        let (sessions, calls, secs) = history_budget();
        Some((self.max_sessions.unwrap_or(sessions), calls, self.max_secs.unwrap_or(secs)))
    }
}

fn category_name(category: Category) -> String {
    json!(category).as_str().unwrap_or_default().to_string()
}

/// What `--sample` shows of one item: identifiers and outcomes, never command text.
fn sample_row(item: &Item) -> Option<Value> {
    let provenance = item.provenance.as_ref()?;
    let sessions: Vec<Value> = provenance
        .sessions
        .iter()
        .map(|s| {
            let operations: Vec<Value> =
                s.operations.iter().map(|o| json!({ "source": o.source, "outcome": o.outcome })).collect();
            json!({ "harness": s.harness, "store": s.store, "id": s.id, "operations": operations, "omitted": s.omitted })
        })
        .collect();
    Some(json!({ "path": item.path, "strength": provenance.strength, "sessions": sessions }))
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    let started = Instant::now();
    let scan = history_probe(&args.project, args.budget()).map_err(|e| format!("{}", json!(e)))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    for item in &scan.items {
        *categories.entry(category_name(item.category)).or_default() += 1;
    }
    println!("{}", json!({ "history": scan.history, "categories": categories, "elapsedMs": elapsed_ms }));

    for row in scan.items.iter().filter_map(sample_row).take(args.sample) {
        eprintln!("{row}");
    }
    Ok(())
}
