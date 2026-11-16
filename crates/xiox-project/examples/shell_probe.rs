//! Parity probe: the Rust extractor and session ledger on a `calls.jsonl` written by
//! `bench/path-extractor/harvest.py`, written as JSONL for `python real.py parity` to compare.
//!
//!     cargo run -j 1 -p xiox-project --example shell_probe -- --calls <calls.jsonl> --out <dir> --home <dir>
//!
//! Writes `calls.jsonl` (one row per shell call with a cwd) and `ledger.jsonl` (one row per folder
//! of each session) in `<dir>`. Command text never reaches the output: identifiers and counts only.

use std::collections::BTreeSet;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{json, Value};
use xiox_mukade::ToolCall;
use xiox_project::{
    effective_cwd, extract, resolve, session_ledgers, shell_command, EvidenceRef, Ledger, PathEnv, SessionKey, ShellCommand,
    Source,
};

struct Args {
    calls: PathBuf,
    out: PathBuf,
    home: String,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut values = std::collections::HashMap::new();
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(format!("{flag} needs a value"))?;
            values.insert(flag, value);
        }
        let mut take = |flag: &str| values.remove(flag).ok_or(format!("missing {flag}"));
        let args = Args { calls: take("--calls")?.into(), out: take("--out")?.into(), home: take("--home")? };
        match values.keys().next() {
            Some(extra) => Err(format!("unknown argument {extra}")),
            None => Ok(args),
        }
    }

    /// The directories `~` and `/tmp` name, as the prototype's `real.ENV` builds them.
    fn env(&self) -> PathEnv {
        PathEnv { home: self.home.clone(), temp: format!(r"{}\AppData\Local\Temp", self.home) }
    }
}

fn read_calls(path: &Path) -> Result<Vec<ToolCall>, Box<dyn Error>> {
    let mut calls = Vec::new();
    for (n, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // Not `{e}`: a serde message can quote a fragment of the row, which may be command text.
        let row = serde_json::from_str(&line)
            .map_err(|e| format!("row {}: {:?} error at column {}", n + 1, e.classify(), e.column()))?;
        calls.push(row);
    }
    Ok(calls)
}

fn call_row(call: &ToolCall, shell: ShellCommand, cwd: &str, env: &PathEnv) -> Value {
    let extraction = extract(&shell.text, shell.shell, shell.completeness);
    let resolved = resolve(&extraction.ops, cwd, env);
    let f = &extraction.flags;
    let flags: Vec<&str> = [("looped", f.looped), ("script", f.script), ("truncated", f.truncated), ("exhausted", f.exhausted)]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect();
    let candidates: Vec<Value> =
        resolved.candidates.iter().map(|c| json!([c.kind.as_str(), c.src, c.dst])).collect();
    json!({
        "store": call.store, "session": call.session, "entry": call.entry, "call": call.call,
        "candidates": candidates, "flags": flags,
        "unresolved": resolved.unresolved, "extract_unresolved": extraction.unresolved,
    })
}

fn source_name(source: &Source) -> String {
    match source {
        Source::Structured { tool, .. } => format!("structured:{tool}"),
        Source::Shell { kind, .. } => format!("shell:{}", kind.as_str()),
        Source::Patch(action) => format!("patch:{}", json!(action).as_str().unwrap_or_default()),
        Source::Cd => "cd".into(),
        Source::Cwd => "cwd".into(),
    }
}

fn folder_rows(key: &SessionKey, ledger: &Ledger) -> impl Iterator<Item = Value> {
    ledger.folders().map(|folder| {
        let all = || folder.refs.iter().chain(&folder.failed);
        let sources: BTreeSet<String> = all().map(|r| source_name(&r.source)).collect();
        let refs: BTreeSet<(&Option<String>, &Option<String>)> = all().map(|r| (&r.entry, &r.call)).collect();
        // The prototype ignores patches: what the other sources show is compared apart.
        let plain = |r: &&EvidenceRef| !matches!(r.source, Source::Patch(_));
        json!({
            "store": key.store, "session": key.session, "folder": folder.path,
            "strength": folder.strength.map(|s| s as u8), "failed": folder.failed.len(),
            "non_patch_strength": folder.refs.iter().filter(plain).filter_map(|r| r.strength).max().map(|s| s as u8),
            "non_patch_failed": folder.failed.iter().filter(plain).count(),
            "sources": sources, "refs": refs,
        })
    })
}

fn write_rows(path: PathBuf, rows: impl IntoIterator<Item = Value>) -> Result<usize, Box<dyn Error>> {
    let mut out = BufWriter::new(File::create(path)?);
    let mut n = 0;
    for row in rows {
        writeln!(out, "{row}")?;
        n += 1;
    }
    out.flush()?;
    Ok(n)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    let started = Instant::now();
    let env = args.env();
    let calls = read_calls(&args.calls)?;
    fs::create_dir_all(&args.out)?;

    let rows = calls.iter().filter_map(|call| {
        let cwd = effective_cwd(call)?;
        Some(call_row(call, shell_command(call)?, &cwd, &env))
    });
    let shell_calls = write_rows(args.out.join("calls.jsonl"), rows)?;

    let ledgers = session_ledgers(&calls, &env);
    let folders = write_rows(args.out.join("ledger.jsonl"), ledgers.iter().flat_map(|(key, l)| folder_rows(key, l)))?;
    eprintln!(
        "{} calls, {shell_calls} shell calls, {} sessions, {folders} folders, {:.2?}",
        calls.len(),
        ledgers.len(),
        started.elapsed()
    );
    Ok(())
}
