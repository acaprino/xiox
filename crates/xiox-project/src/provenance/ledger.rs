//! Folder evidence per session: what a session shows about each folder, from every tool call in it.
//!
//! A command the parsers cannot read may still be shown by the session elsewhere: a structured
//! Write or Read, a patch, another command, a `cd`. The ledger keeps, per folder, the strongest
//! evidence that did not fail and every reference behind it. A failed operation stays visible
//! under `failed` but carries no strength.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use xiox_mukade::{was_cut, Outcome, ToolCall};

use super::{
    extract, patch_ops, resolve, resolve_path, shell_command, strip_verbatim, Completeness, Env, OpKind,
    PatchAction, ShellCommand,
};

/// How much a piece of evidence says about a folder: `Used < Written < Created`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Strength {
    /// Read, searched, entered with `cd`, or the cwd of a call.
    Used = 1,
    /// A file in it was written.
    Written = 2,
    /// Made, copied, moved, cloned or added as a worktree.
    Created = 3,
}

/// A session is told apart by its store and harness as well as its id: two stores can reuse an id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct SessionKey {
    pub store: String,
    pub harness: String,
    pub session: String,
}

impl SessionKey {
    fn of(call: &ToolCall) -> Self {
        SessionKey { store: call.store.clone(), harness: call.harness.clone(), session: call.session.clone() }
    }
}

/// Which part of a tool call produced the evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A tool with a path field, by tool name.
    Structured(String),
    /// An operation read from a shell command; `script` when it came from an inline script.
    Shell { kind: OpKind, script: bool },
    Patch(PatchAction),
    /// A directory a `cd` entered, resolved where it ran.
    Cd,
    /// The working directory of the call.
    Cwd,
}

/// Where a piece of evidence is in the session, and how its call ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceRef {
    pub session: SessionKey,
    pub entry: Option<String>,
    pub call: Option<String>,
    pub time: Option<String>,
    /// The call's effective working directory, without a `\\?\` prefix.
    pub cwd: Option<String>,
    pub source: Source,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderEvidence {
    /// The folder as first seen; the ledger matches folders without regard to case.
    pub path: String,
    /// The strongest evidence that did not fail.
    pub strength: Option<Strength>,
    pub refs: Vec<EvidenceRef>,
    pub failed: Vec<EvidenceRef>,
}

/// What the shell parsers could not read, summed over the shell calls of a session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub shell_calls: usize,
    /// Extractor and resolver combined.
    pub unresolved: usize,
    pub looped: usize,
    pub script: usize,
    pub truncated: usize,
    pub exhausted: usize,
}

/// The folder an operation speaks about.
// shortcut: un ultimo segmento con estensione è trattato come file, così --out D:\runs\v1.2 conta come file;
// da aggiornare quando il risultato del tool indica il tipo.
pub fn folder_of(kind: OpKind, path: &str) -> String {
    if matches!(kind, OpKind::Create | OpKind::Clone | OpKind::Worktree) {
        return path.to_owned();
    }
    let name = path.rsplit('\\').next().unwrap_or(path);
    if name.trim_matches('.').contains('.') { parent_dir(path) } else { path.to_owned() }
}

/// The folder holding `path`; a drive or share root keeps its separator.
fn parent_dir(path: &str) -> String {
    match path.rfind('\\') {
        Some(i) if path[..i].ends_with(':') => path[..=i].to_owned(),
        Some(i) => path[..i].to_owned(),
        None => path.to_owned(),
    }
}

/// Evidence of one session, keyed by folder in lowercase so the order is stable.
#[derive(Debug, Default)]
pub struct Ledger {
    folders: BTreeMap<String, FolderEvidence>,
    coverage: Coverage,
}

impl Ledger {
    /// Adds what `call` shows: its cwd, then its structured paths, or its patch and shell command.
    /// A structured call gives neither of the others.
    pub fn record(&mut self, call: &ToolCall, env: &Env) {
        let at = Site::of(call);
        self.record_cwd(&at);
        if self.record_structured(call, &at, env) {
            return;
        }
        // A shell command may carry a patch (`apply_patch <<EOF`): it gives both evidences.
        self.record_patch(call, &at, env);
        self.record_shell(call, &at, env);
    }

    pub fn get(&self, folder: &str) -> Option<&FolderEvidence> {
        self.folders.get(&folder.to_lowercase())
    }

    pub fn folders(&self) -> impl Iterator<Item = &FolderEvidence> {
        self.folders.values()
    }

    pub fn coverage(&self) -> &Coverage {
        &self.coverage
    }

    /// Evidence of an operation: a failed call keeps it under `failed`, without strength.
    fn add(&mut self, at: &Site, folder: &str, strength: Strength, source: Source) {
        let reference = at.reference(source);
        let entry = self.entry(folder);
        if at.outcome == Outcome::Failed {
            entry.failed.push(reference);
        } else {
            entry.refs.push(reference);
            entry.strength = entry.strength.max(Some(strength));
        }
    }

    /// Evidence that the call ran in `folder` (its cwd, a `cd`): it holds even when the call failed.
    fn add_visit(&mut self, at: &Site, folder: &str, source: Source) {
        let reference = at.reference(source);
        let entry = self.entry(folder);
        entry.refs.push(reference);
        entry.strength = entry.strength.max(Some(Strength::Used));
    }

    fn entry(&mut self, folder: &str) -> &mut FolderEvidence {
        self.folders
            .entry(folder.to_lowercase())
            .or_insert_with(|| FolderEvidence { path: folder.to_owned(), strength: None, refs: vec![], failed: vec![] })
    }

    fn record_cwd(&mut self, at: &Site) {
        if let Some(cwd) = &at.cwd {
            self.add_visit(at, cwd, Source::Cwd);
        }
    }

    /// Returns whether the call is a structured one, which then gives no other evidence.
    fn record_structured(&mut self, call: &ToolCall, at: &Site, env: &Env) -> bool {
        let Some(&(_, strength, searches_folder)) =
            STRUCTURED.iter().find(|(name, ..)| call.canonical.as_deref() == Some(name))
        else {
            return false;
        };
        if patch_text(&call.input).is_some() {
            return false;
        }
        let tool = call.tool.clone().or_else(|| call.canonical.clone()).unwrap_or_default();
        for field in PATH_FIELDS {
            let Some(value) = call.input.get(field).and_then(Value::as_str).filter(|v| !v.is_empty()) else {
                continue;
            };
            let Ok(path) = resolve_path(value, at.cwd.as_deref(), env) else { continue };
            let folder = if searches_folder { path } else { parent_dir(&path) };
            self.add(at, &folder, strength, Source::Structured(tool.clone()));
        }
        true
    }

    /// R1: a call is a patch when a top-level string of its input holds one.
    fn record_patch(&mut self, call: &ToolCall, at: &Site, env: &Env) {
        let Some(text) = patch_text(&call.input) else { return };
        let completeness = if was_cut(text) { Completeness::Truncated } else { Completeness::Whole };
        for (action, path) in patch_ops(text, completeness) {
            let Ok(path) = resolve_path(&path, at.cwd.as_deref(), env) else {
                self.coverage.unresolved += 1;
                continue;
            };
            // A deletion or the source of a move names a folder that already existed.
            let strength = match action {
                PatchAction::Add | PatchAction::Update | PatchAction::MoveTo => Strength::Written,
                PatchAction::Delete | PatchAction::MoveFrom => Strength::Used,
            };
            // Patch paths name files, whatever their last segment looks like.
            self.add(at, &parent_dir(&path), strength, Source::Patch(action));
        }
    }

    fn record_shell(&mut self, call: &ToolCall, at: &Site, env: &Env) {
        let Some(ShellCommand { shell, text, completeness }) = shell_command(call) else { return };
        // Without a cwd no relative path can be placed, as in the prototype.
        let Some(cwd) = &at.cwd else { return };
        let extraction = extract(&text, shell, completeness);
        let resolved = resolve(&extraction.ops, cwd, env);

        let coverage = &mut self.coverage;
        coverage.shell_calls += 1;
        coverage.unresolved += extraction.unresolved + resolved.unresolved;
        let flags = &extraction.flags;
        for (flag, count) in [
            (flags.looped, &mut coverage.looped),
            (flags.script, &mut coverage.script),
            (flags.truncated, &mut coverage.truncated),
            (flags.exhausted, &mut coverage.exhausted),
        ] {
            *count += usize::from(flag);
        }

        for candidate in &resolved.candidates {
            let source = Source::Shell { kind: candidate.kind, script: candidate.script };
            self.add(at, &folder_of(candidate.kind, &candidate.dst), strength_of(candidate.kind), source);
        }
        for visit in &resolved.visits {
            self.add_visit(at, visit, Source::Cd);
        }
    }
}

/// Structured tools by canonical name: the strength they give and whether the path is a folder.
const STRUCTURED: [(&str, Strength, bool); 6] = [
    ("write", Strength::Written, false),
    ("edit", Strength::Written, false),
    ("notebook_edit", Strength::Written, false),
    ("read", Strength::Used, false),
    ("glob", Strength::Used, true),
    ("grep", Strength::Used, true),
];
const PATH_FIELDS: [&str; 3] = ["file_path", "notebook_path", "path"];
const PATCH_MARKER: &str = "*** Begin Patch";

fn strength_of(kind: OpKind) -> Strength {
    match kind {
        OpKind::Write => Strength::Written,
        OpKind::Create | OpKind::Copy | OpKind::Move | OpKind::Clone | OpKind::Worktree => Strength::Created,
    }
}

/// The patch a call carries: the first top-level string of its input that holds a patch header.
fn patch_text(input: &Value) -> Option<&str> {
    input.as_object()?.values().filter_map(Value::as_str).find(|text| text.contains(PATCH_MARKER))
}

/// What every piece of evidence of one call shares.
struct Site {
    session: SessionKey,
    entry: Option<String>,
    call: Option<String>,
    time: Option<String>,
    cwd: Option<String>,
    outcome: Outcome,
}

impl Site {
    fn of(call: &ToolCall) -> Self {
        Site {
            session: SessionKey::of(call),
            entry: call.entry.clone(),
            call: call.call.clone(),
            time: call.time.clone(),
            cwd: call
                .cwd
                .as_deref()
                .map(|cwd| strip_verbatim(cwd).replace('/', r"\"))
                .filter(|cwd| !cwd.is_empty()),
            outcome: call.outcome(),
        }
    }

    fn reference(&self, source: Source) -> EvidenceRef {
        EvidenceRef {
            session: self.session.clone(),
            entry: self.entry.clone(),
            call: self.call.clone(),
            time: self.time.clone(),
            cwd: self.cwd.clone(),
            source,
            outcome: self.outcome,
        }
    }
}

/// One ledger per session, grouped by the full key.
pub fn session_ledgers<'a>(calls: impl IntoIterator<Item = &'a ToolCall>, env: &Env) -> BTreeMap<SessionKey, Ledger> {
    let mut ledgers: BTreeMap<SessionKey, Ledger> = BTreeMap::new();
    for call in calls {
        ledgers.entry(SessionKey::of(call)).or_default().record(call, env);
    }
    ledgers
}
