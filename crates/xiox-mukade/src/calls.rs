//! The tool calls of one session, read from `mukade dump <harness> <id> --stream`.
//!
//! The stream is one JSON object per entry (`id`, `kind`, `raw`) and a last `{"meta": ...}` line.
//! A `kind.step.calls[]` entry is a tool call; a `kind.results.results[]` entry is the result of
//! one, joined by `call_id`. The rows keep the field names of `bench/path-extractor/harvest.py`,
//! so its `calls.jsonl` files and this reader's output are interchangeable.

use crate::MukadeSessions;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use xiox_model::Agent;

/// Longest string kept in a call's `input`; a longer one is cut and ends with [`CUT_MARK`].
pub const MAX_INPUT_CHARS: usize = 4000;
pub const CUT_MARK: char = '…';

/// Characters of a result text searched for an exit code from the start, then from the end.
const EXIT_HEAD_CHARS: usize = 2000;
const EXIT_TAIL_CHARS: usize = 500;
const RESULT_HEAD_CHARS: usize = 300;
const EXIT_PHRASES: [&str; 3] = ["exit code", "exited with code", "exit status"];
const VERBATIM_PREFIX: &str = r"\\?\";
/// Variables that must not carry the `\\?\` prefix when the native `opencode.exe` runs.
const OPENCODE_ENV: [&str; 6] = ["USERPROFILE", "HOME", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP"];

/// Cuts every string longer than [`MAX_INPUT_CHARS`] characters; `true` when one was cut.
pub fn cut_input(v: Value) -> (Value, bool) {
    match v {
        Value::String(s) if s.chars().nth(MAX_INPUT_CHARS).is_some() => {
            let mut cut: String = s.chars().take(MAX_INPUT_CHARS).collect();
            cut.push(CUT_MARK);
            (Value::String(cut), true)
        }
        Value::Array(items) => {
            let (items, cuts): (Vec<_>, Vec<_>) = items.into_iter().map(cut_input).unzip();
            (Value::Array(items), cuts.contains(&true))
        }
        Value::Object(map) => {
            let mut any = false;
            let map = map
                .into_iter()
                .map(|(k, v)| {
                    let (v, cut) = cut_input(v);
                    any |= cut;
                    (k, v)
                })
                .collect::<Map<_, _>>();
            (Value::Object(map), any)
        }
        other => (other, false),
    }
}

/// Whether `s` has the exact shape [`cut_input`] gives a cut string, also in the `calls.jsonl`
/// rows `harvest.py` wrote.
pub fn was_cut(s: &str) -> bool {
    s.chars().count() == MAX_INPUT_CHARS + 1 && s.ends_with(CUT_MARK)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Outcome {
    Succeeded,
    Failed,
    Unknown,
}

/// One tool call of a session, with its result when the stream had one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub harness: String,
    pub store: String,
    pub session: String,
    pub parent: Option<String>,
    pub entry: Option<String>,
    pub call: Option<String>,
    pub tool: Option<String>,
    /// Mukade's harness-independent tool name, when it is a string. A row an older `harvest.py`
    /// wrote may hold an object here: it reads as `None`.
    #[serde(default, deserialize_with = "string_or_none")]
    pub canonical: Option<String>,
    /// The call's input; a value that is not an object is kept as `{"_": value}`.
    pub input: Value,
    pub cwd: Option<String>,
    /// Read like `parse_stream` reads it: a string or a number; OpenCode's object time is absent.
    #[serde(default, deserialize_with = "scalar_or_none")]
    pub time: Option<String>,
    pub error: Option<bool>,
    pub exit: Option<i64>,
    /// The first characters of the result text; `None` when no result was paired.
    pub result_head: Option<String>,
}

fn string_or_none<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(de)? {
        Value::String(s) => Some(s),
        _ => None,
    })
}

fn scalar_or_none<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    Ok(scalar(Some(&Value::deserialize(de)?)))
}

impl ToolCall {
    pub fn outcome(&self) -> Outcome {
        if self.error == Some(true) || self.exit.is_some_and(|code| code != 0) {
            Outcome::Failed
        } else if self.result_head.is_none() {
            Outcome::Unknown
        } else {
            Outcome::Succeeded
        }
    }
}

/// Reads the output of `mukade dump --stream`. Split on `\n` only: U+2028 and friends occur
/// inside JSON strings.
pub fn parse_stream(
    stream: &str,
    harness: &str,
    store: &str,
    session: &str,
    session_cwd: Option<&str>,
) -> Vec<ToolCall> {
    let mut calls: Vec<ToolCall> = Vec::new();
    let mut pending: HashMap<String, usize> = HashMap::new();
    for line in stream.split('\n').filter(|l| !l.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        let Some(kind) = entry.get("kind").and_then(Value::as_object) else { continue };
        let raw = entry.get("raw").and_then(|r| r.get(0)).filter(|r| r.is_object());
        let raw = raw.unwrap_or(&Value::Null);
        for call in array_at(kind.get("step"), "calls") {
            let input = match call.get("input") {
                Some(v @ Value::Object(_)) => v.clone(),
                other => serde_json::json!({ "_": other.cloned().unwrap_or(Value::Null) }),
            };
            let cwd = call_cwd(&input, raw, session_cwd);
            let (input, _) = cut_input(input);
            if let Some(id) = call.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) {
                pending.insert(id.to_string(), calls.len());
            }
            calls.push(ToolCall {
                harness: harness.into(),
                store: store.into(),
                session: session.into(),
                parent: None,
                entry: scalar(entry.get("id")),
                call: scalar(call.get("id")),
                tool: scalar(call.get("source_name")),
                canonical: call.get("canonical").and_then(Value::as_str).map(Into::into),
                input,
                cwd,
                time: scalar(raw.get("timestamp").filter(|t| truthy(t)).or_else(|| raw.get("time"))),
                error: None,
                exit: None,
                result_head: None,
            });
        }
        for result in array_at(kind.get("results"), "results") {
            let paired = result.get("call_id").and_then(Value::as_str).and_then(|id| pending.remove(id));
            let Some(call) = paired.map(|at| &mut calls[at]) else { continue };
            let text = result_text(result.get("content"));
            call.error = result.get("is_error").and_then(Value::as_bool);
            call.exit = exit_code(text);
            call.result_head = Some(text.chars().take(RESULT_HEAD_CHARS).collect());
        }
    }
    calls
}

/// The elements of `parent[key]` when it is an array, nothing otherwise.
fn array_at<'a>(parent: Option<&'a Value>, key: &str) -> impl Iterator<Item = &'a Value> {
    parent.and_then(|p| p.get(key)).and_then(Value::as_array).into_iter().flatten()
}

/// `input.workdir`, `cwd`, `working_directory` or `directory`, then the entry's own `cwd`, then
/// the session's; an empty value falls through.
fn call_cwd(input: &Value, raw: &Value, session_cwd: Option<&str>) -> Option<String> {
    let from_input = ["workdir", "cwd", "working_directory", "directory"]
        .iter()
        .find_map(|k| input.get(k).and_then(Value::as_str));
    [from_input, raw.get("cwd").and_then(Value::as_str), session_cwd]
        .into_iter()
        .flatten()
        .find(|c| !c.is_empty())
        .map(Into::into)
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false)) && v.as_str() != Some("")
}

/// A string or a number as text; anything else is absent.
fn scalar(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn result_text(content: Option<&Value>) -> &str {
    match content {
        Some(Value::String(s)) => s,
        Some(Value::Object(o)) => o.get("text").and_then(Value::as_str).unwrap_or(""),
        _ => "",
    }
}

/// An exit code announced in the first [`EXIT_HEAD_CHARS`] characters, else in the last
/// [`EXIT_TAIL_CHARS`]. Windows are cut on character boundaries, never by byte.
fn exit_code(text: &str) -> Option<i64> {
    let head_end = text.char_indices().nth(EXIT_HEAD_CHARS).map_or(text.len(), |(i, _)| i);
    let tail_start = text.char_indices().rev().nth(EXIT_TAIL_CHARS - 1).map_or(0, |(i, _)| i);
    exit_in(&text[..head_end]).or_else(|| exit_in(&text[tail_start..]))
}

/// The first `exit code: N`, `exited with code N` or `exit status N`, case-insensitive. The
/// phrases are ASCII, so every match starts on a character boundary.
fn exit_in(window: &str) -> Option<i64> {
    let bytes = window.as_bytes();
    (0..bytes.len()).find_map(|at| {
        let phrase = EXIT_PHRASES.iter().find(|p| {
            bytes.get(at..at + p.len()).is_some_and(|b| b.eq_ignore_ascii_case(p.as_bytes()))
        })?;
        let rest = &window[at + phrase.len()..];
        let number = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
        if number.len() == rest.len() {
            return None;
        }
        let sign = usize::from(number.starts_with('-'));
        let digits = number[sign..].chars().take_while(char::is_ascii_digit).count();
        number[..sign + digits].parse().ok()
    })
}

impl MukadeSessions {
    /// Every tool call of session `id`, from `mukade dump <harness> <id> --stream`. The store is
    /// only read; an OpenCode database is read from a copy that lives until this returns.
    pub fn dump_calls(&self, id: &str, session_cwd: Option<&str>) -> anyhow::Result<Vec<ToolCall>> {
        let store = self.store_root().ok_or_else(|| anyhow::anyhow!("no store configured"))?;
        self.opencode_exe()?;
        let (_copy_dir, read_from) = match self.agent {
            Agent::Opencode => {
                let dir = tempfile::tempdir_in(snapshot_base(&std::env::temp_dir()))?;
                let copy = opencode_snapshot(store, dir.path())?;
                (Some(dir), copy)
            }
            _ => (None, store.to_path_buf()),
        };
        let mut cmd = self.dump_command(id, &read_from)?;
        let out = cmd
            .output()
            .map_err(|e| anyhow::anyhow!("{}: {e}", cmd.get_program().to_string_lossy()))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("mukade dump {} {id}: {}", self.harness(), err.trim());
        }
        let stream = String::from_utf8_lossy(&out.stdout);
        Ok(parse_stream(&stream, self.harness(), &store.to_string_lossy(), id, session_cwd))
    }

    /// `mukade dump <harness> <id> --stream` reading `store`, which `dump` takes from the same
    /// environment variable as `list`.
    fn dump_command(&self, id: &str, store: &Path) -> anyhow::Result<Command> {
        let opencode_exe = self.opencode_exe()?;
        let store = if opencode_exe.is_some() { snapshot_base(store) } else { store.to_path_buf() };
        let mut cmd = self.mukade_command(&store)?;
        cmd.args(["dump", self.harness(), id, "--stream"]);
        if let Some(exe) = opencode_exe {
            cmd.arg("--opencode-exe").arg(exe);
            for var in OPENCODE_ENV {
                if let Some(value) = std::env::var_os(var) {
                    cmd.env(var, strip_verbatim(&value.to_string_lossy()).as_ref());
                }
            }
        }
        Ok(cmd)
    }

    /// The native `opencode.exe` an OpenCode dump needs; `None` for every other harness.
    fn opencode_exe(&self) -> anyhow::Result<Option<&Path>> {
        if self.agent != Agent::Opencode {
            return Ok(None);
        }
        let exe = self.opencode_exe.as_deref();
        exe.map(Some).ok_or_else(|| {
            anyhow::anyhow!(
                "opencode.exe is not configured (set XIOX_OPENCODE_EXE): mukade needs the native binary to dump OpenCode"
            )
        })
    }
}

/// Copies `opencode.db` and, when present, its `-wal` and `-shm` files into `into`; the copy is
/// what Mukade reads, so the live database is never opened by it.
fn opencode_snapshot(db: &Path, into: &Path) -> std::io::Result<PathBuf> {
    let name = db.file_name().ok_or_else(|| std::io::Error::other("database path has no file name"))?;
    let copy = into.join(name);
    std::fs::copy(db, &copy)?;
    for suffix in ["-wal", "-shm"] {
        let mut side = name.to_os_string();
        side.push(suffix);
        let from = db.with_file_name(&side);
        if from.is_file() {
            std::fs::copy(from, into.join(side))?;
        }
    }
    Ok(copy)
}

/// `\\?\C:\x` becomes `C:\x` and `\\?\UNC\server\share\x` becomes `\\server\share\x`; any other
/// path is kept.
pub fn strip_verbatim(p: &str) -> Cow<'_, str> {
    match p.strip_prefix(VERBATIM_PREFIX) {
        Some(unc) if unc.starts_with(r"UNC\") => Cow::Owned(format!(r"\\{}", &unc[4..])),
        Some(drive) if drive.as_bytes().get(1) == Some(&b':') => Cow::Borrowed(drive),
        _ => Cow::Borrowed(p),
    }
}

/// `p` without a `\\?\` prefix: the exe OpenCode runs rejects it in a store or temp path.
fn snapshot_base(p: &Path) -> PathBuf {
    PathBuf::from(strip_verbatim(&p.to_string_lossy()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreRoots;
    use serde_json::json;
    use xiox_model::Agent;

    fn line(v: Value) -> String {
        format!("{v}\n")
    }

    fn step(id: &str, raw: Value, calls: Value) -> String {
        line(json!({"id": id, "kind": {"step": {"calls": calls}}, "raw": [raw]}))
    }

    fn results(id: &str, rs: Value) -> String {
        line(json!({"id": id, "kind": {"results": {"results": rs}}, "raw": [{}]}))
    }

    fn parse(stream: &str) -> Vec<ToolCall> {
        parse_stream(stream, "claude_code", "/store", "s1", Some("/session/cwd"))
    }

    fn call_with(error: Option<bool>, exit: Option<i64>, head: Option<&str>) -> ToolCall {
        ToolCall {
            harness: "h".into(),
            store: "s".into(),
            session: "x".into(),
            parent: None,
            entry: None,
            call: None,
            tool: None,
            canonical: None,
            input: Value::Null,
            cwd: None,
            time: None,
            error,
            exit,
            result_head: head.map(Into::into),
        }
    }

    /// One call `c1` with an empty input, then one result carrying `content`.
    fn paired(content: Value) -> Vec<ToolCall> {
        let stream = [
            step("e1", json!({}), json!([{"id": "c1", "input": {}}])),
            results("e2", json!([{"call_id": "c1", "content": content}])),
        ]
        .concat();
        parse(&stream)
    }

    #[test]
    fn stream_pairs_calls_with_results() {
        let stream = [
            step(
                "e1",
                json!({"cwd": "/raw/cwd", "timestamp": "2026-11-01T10:00:00Z"}),
                json!([
                    {"id": "c1", "source_name": "Bash", "canonical": "bash",
                     "input": {"command": "git clone x"}},
                    {"id": "c2", "source_name": "Write", "canonical": "write",
                     "input": {"file_path": "a\u{2028}b.txt", "content": "line\u{2028}two"}},
                ]),
            ),
            line(json!({"meta": {"harness": "claude_code"}})),
            "not json at all\n".to_string(),
            results(
                "e2",
                json!([{"call_id": "c1", "is_error": true, "content": {"text": "Exit code: 128\nfatal"}}]),
            ),
        ]
        .concat();
        let calls = parse(&stream);
        assert_eq!(calls.len(), 2);
        let bash = &calls[0];
        assert_eq!(
            (bash.harness.as_str(), bash.store.as_str(), bash.session.as_str()),
            ("claude_code", "/store", "s1")
        );
        assert_eq!(bash.entry.as_deref(), Some("e1"));
        assert_eq!(bash.call.as_deref(), Some("c1"));
        assert_eq!(bash.tool.as_deref(), Some("Bash"));
        assert_eq!(bash.canonical.as_deref(), Some("bash"));
        assert_eq!(bash.time.as_deref(), Some("2026-11-01T10:00:00Z"));
        assert_eq!(bash.error, Some(true));
        assert_eq!(bash.exit, Some(128));
        assert_eq!(bash.result_head.as_deref(), Some("Exit code: 128\nfatal"));
        let write = &calls[1];
        assert_eq!(write.input["file_path"], "a\u{2028}b.txt");
        assert_eq!((write.error, write.exit, write.result_head.as_deref()), (None, None, None));
    }

    #[test]
    fn non_object_input_is_wrapped_and_text_content_is_read() {
        let stream = [
            step("e1", json!({}), json!([{"id": "c1", "source_name": "apply_patch", "input": "*** Begin Patch"}])),
            results("e2", json!([{"call_id": "c1", "content": "plain"}, {"call_id": "zz", "content": "x"}])),
            results("e3", json!([{"call_id": "c1", "content": {"text": "second result is ignored"}}])),
        ]
        .concat();
        let calls = parse(&stream);
        assert_eq!(calls[0].input, json!({"_": "*** Begin Patch"}));
        assert_eq!(calls[0].result_head.as_deref(), Some("plain"));
        assert_eq!(calls[0].canonical, None);

        let empty = [
            step("e1", json!({}), json!([{"id": "c1", "input": {}}])),
            results("e2", json!([{"call_id": "c1"}])),
        ]
        .concat();
        assert_eq!(parse(&empty)[0].result_head.as_deref(), Some(""));
    }

    #[test]
    fn outcomes_are_three() {
        let cases = [
            (call_with(None, None, None), Outcome::Unknown),
            (call_with(None, None, Some("ok")), Outcome::Succeeded),
            (call_with(Some(false), Some(0), Some("ok")), Outcome::Succeeded),
            (call_with(None, Some(2), Some("Exit code: 2")), Outcome::Failed),
            (call_with(Some(true), None, Some("boom")), Outcome::Failed),
            (call_with(Some(true), Some(0), None), Outcome::Failed),
            (call_with(None, Some(1), None), Outcome::Failed),
        ];
        for (call, want) in cases {
            assert_eq!(call.outcome(), want, "{:?}", (call.error, call.exit, &call.result_head));
        }
    }

    #[test]
    fn exit_windows_count_chars() {
        // 5,000 `è` (2 bytes each) with the phrase placed at a character offset.
        let pad = |before: usize, code: &str| {
            let after = 5000 - before - code.chars().count();
            format!("{}{code}{}", "è".repeat(before), "è".repeat(after))
        };
        let cases = [
            ("head window, past 2000 bytes", pad(1980, "exit code: 3"), Some(3)),
            ("tail window", pad(5000 - 12, "exit code: 4"), Some(4)),
            ("between the windows", pad(3000, "exit code: 5"), None),
        ];
        for (name, text, want) in cases {
            let calls = paired(json!({ "text": text }));
            assert_eq!(calls[0].exit, want, "{name}");
            assert_eq!(calls[0].result_head.as_deref().map(|h| h.chars().count()), Some(300), "{name}");
        }
    }

    #[test]
    fn exit_phrases() {
        let cases = [
            ("Exit code: 7", Some(7)),
            ("process exited with code -1073741510", Some(-1_073_741_510)),
            ("EXIT STATUS 2", Some(2)),
            ("exit code: none, then exit status: 9", Some(9)),
            ("exit code", None),
            ("exited", None),
        ];
        for (text, want) in cases {
            assert_eq!(paired(json!(text))[0].exit, want, "{text}");
        }
    }

    #[test]
    fn cut_input_marks_and_was_cut_recognizes() {
        let long = "é".repeat(MAX_INPUT_CHARS + 1);
        let (cut, did) = cut_input(json!({"a": [long.clone(), "short"], "n": 1}));
        assert!(did);
        let s = cut["a"][0].as_str().unwrap();
        assert_eq!(s.chars().count(), MAX_INPUT_CHARS + 1);
        assert!(s.ends_with(CUT_MARK));
        assert!(was_cut(s));
        assert!(!was_cut("short"));
        assert!(!was_cut(&long));
        assert_eq!(cut["a"][1], "short");

        let exact = "é".repeat(MAX_INPUT_CHARS);
        let (same, did) = cut_input(json!(exact.clone()));
        assert!(!did);
        assert_eq!(same, json!(exact));
    }

    #[test]
    fn cwd_falls_back_in_order() {
        let cases = [
            (json!({"workdir": "/w", "cwd": "/c"}), json!({"cwd": "/raw"}), Some("/w")),
            (json!({"cwd": "/c"}), json!({"cwd": "/raw"}), Some("/c")),
            (json!({"working_directory": "/wd"}), json!({}), Some("/wd")),
            (json!({"directory": "/d"}), json!({}), Some("/d")),
            (json!({"workdir": 3}), json!({"cwd": "/raw"}), Some("/raw")),
            (json!({"workdir": ""}), json!({"cwd": "/raw"}), Some("/raw")),
            (json!({"command": "ls"}), json!({"cwd": "/raw"}), Some("/raw")),
            (json!({"command": "ls"}), json!({}), Some("/session/cwd")),
        ];
        for (input, raw, want) in cases {
            let stream = step("e1", raw.clone(), json!([{"id": "c1", "input": input.clone()}]));
            assert_eq!(parse(&stream)[0].cwd.as_deref(), want, "{input} {raw}");
        }
        let bare = step("e", json!({}), json!([{"id": "c", "input": {}}]));
        assert_eq!(parse_stream(&bare, "h", "s", "i", None)[0].cwd, None);
    }

    fn write_claude_store(root: &Path, id: &str) {
        let dir = root.join("projects").join("D--proj");
        std::fs::create_dir_all(&dir).unwrap();
        let rec = |v: Value| format!("{v}\n");
        let text = [
            rec(json!({"type": "user", "uuid": "u1", "parentUuid": null, "sessionId": id,
                "timestamp": "2026-11-01T10:00:00.000Z", "cwd": "D:\\proj",
                "message": {"role": "user", "content": "make a folder"}})),
            rec(json!({"type": "assistant", "uuid": "a1", "parentUuid": "u1", "sessionId": id,
                "timestamp": "2026-11-01T10:00:01.000Z", "cwd": "D:\\proj",
                "message": {"role": "assistant", "model": "claude-x", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Bash",
                     "input": {"command": "mkdir /d/x"}}]}})),
            rec(json!({"type": "user", "uuid": "u2", "parentUuid": "a1", "sessionId": id,
                "timestamp": "2026-11-01T10:00:02.000Z", "cwd": "D:\\proj",
                "message": {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "done"}]}})),
        ]
        .concat();
        std::fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
    }

    #[test]
    fn dump_calls_reads_a_fake_claude_store() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("claude");
        let id = "11111111-2222-3333-4444-555555555555";
        write_claude_store(&store, id);
        let sessions = MukadeSessions::with_roots(
            Agent::Claude,
            StoreRoots { claude_config_dir: Some(store.clone()), ..Default::default() },
        )
        .with_cache_dir(Some(tmp.path().join("data")));
        let calls = sessions.dump_calls(id, Some("D:\\proj")).unwrap();
        assert_eq!(calls.len(), 1, "{calls:?}");
        let call = &calls[0];
        assert_eq!(call.canonical.as_deref(), Some("bash"));
        assert_eq!(call.store, store.to_string_lossy());
        assert_eq!(call.session, id);
        assert_eq!(call.harness, "claude_code");
        assert_eq!(call.input["command"], "mkdir /d/x");
        assert_eq!(call.result_head.as_deref(), Some("done"));
        assert_eq!(call.outcome(), Outcome::Succeeded);
    }

    fn args_of(cmd: &Command) -> Vec<String> {
        cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn arg_after(cmd: &Command, flag: &str) -> Option<String> {
        let args = args_of(cmd);
        let at = args.iter().position(|a| a == flag)?;
        args.get(at + 1).cloned()
    }

    fn env_of(cmd: &Command, key: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    fn opencode_sessions(db: &Path, exe: Option<&Path>) -> MukadeSessions {
        MukadeSessions::with_roots(
            Agent::Opencode,
            StoreRoots { opencode_db: Some(db.to_path_buf()), ..Default::default() },
        )
        .with_opencode_exe(exe.map(Path::to_path_buf))
    }

    #[test]
    fn opencode_dump_uses_a_copy_and_the_native_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        std::fs::write(&db, b"main").unwrap();
        std::fs::write(tmp.path().join("opencode.db-wal"), b"wal").unwrap();
        let exe = tmp.path().join("native").join("opencode.exe");
        let sessions = opencode_sessions(&db, Some(&exe));

        let into = tempfile::tempdir().unwrap();
        let copy = opencode_snapshot(&db, into.path()).unwrap();
        assert_ne!(copy, db);
        assert_eq!(std::fs::read(&copy).unwrap(), b"main");
        assert_eq!(std::fs::read(into.path().join("opencode.db-wal")).unwrap(), b"wal");
        assert!(!into.path().join("opencode.db-shm").exists());

        let cmd = sessions.dump_command("ses_1", &copy).unwrap();
        assert_eq!(args_of(&cmd)[..4], ["dump", "opencode", "ses_1", "--stream"]);
        assert_eq!(arg_after(&cmd, "--opencode-exe").as_deref(), Some(exe.to_str().unwrap()));
        let pointed = env_of(&cmd, "OPENCODE_DB").unwrap();
        assert_ne!(Path::new(&pointed), db);
        assert_eq!(std::fs::read(&pointed).unwrap(), std::fs::read(&db).unwrap());
        for var in ["USERPROFILE", "HOME", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP"] {
            if let Some(v) = env_of(&cmd, var) {
                assert!(!v.starts_with(r"\\?\"), "{var}={v}");
            }
        }
    }

    #[test]
    fn verbatim_prefix_is_stripped_only_when_present() {
        let cases = [
            (r"\\?\C:\Users\x", r"C:\Users\x"),
            (r"\\?\UNC\server\share\x", r"\\server\share\x"),
            (r"C:\Users\x", r"C:\Users\x"),
            (r"\\server\share\x", r"\\server\share\x"),
            (r"\\?\Volume{1}\x", r"\\?\Volume{1}\x"),
            ("", ""),
        ];
        for (value, want) in cases {
            assert_eq!(strip_verbatim(value), want, "{value}");
        }
    }

    #[test]
    fn opencode_store_and_snapshot_base_are_never_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        std::fs::write(&db, b"main").unwrap();
        let sessions = opencode_sessions(&db, Some(&tmp.path().join("opencode.exe")));

        let verbatim_db = PathBuf::from(format!(r"\\?\{}", db.display()));
        let cmd = sessions.dump_command("ses_1", &verbatim_db).unwrap();
        let pointed = env_of(&cmd, "OPENCODE_DB").unwrap();
        assert!(!pointed.starts_with(r"\\?\"), "{pointed}");
        assert_eq!(std::fs::read(&pointed).unwrap(), b"main");

        let base = snapshot_base(Path::new(&format!(r"\\?\{}", tmp.path().display())));
        assert!(!base.to_string_lossy().starts_with(r"\\?\"), "{base:?}");
        assert!(base.is_dir());
    }

    #[test]
    fn opencode_without_exe_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        std::fs::write(&db, b"main").unwrap();
        let err = opencode_sessions(&db, None).dump_calls("ses_1", None).unwrap_err();
        assert!(err.to_string().contains("opencode.exe"), "{err}");
    }

    #[test]
    fn dump_command_leaves_other_harnesses_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions = MukadeSessions::with_roots(
            Agent::Codex,
            StoreRoots { codex_home: Some(tmp.path().to_path_buf()), ..Default::default() },
        );
        let cmd = sessions.dump_command("id", tmp.path()).unwrap();
        assert_eq!(arg_after(&cmd, "--opencode-exe"), None);
        assert_eq!(env_of(&cmd, "CODEX_HOME").as_deref(), tmp.path().to_str());
    }

    #[test]
    fn harvest_rows_read_back_with_object_canonical_and_missing_fields() {
        let cases = [
            (json!("bash"), Some("bash")),
            (json!({"kind": "other"}), None),
            (json!(null), None),
        ];
        for (canonical, want) in cases {
            let row = json!({
                "harness": "h", "store": "s", "session": "x", "canonical": canonical, "input": {},
            });
            let call: ToolCall = serde_json::from_value(row).unwrap();
            assert_eq!(call.canonical.as_deref(), want, "{canonical}");
            assert_eq!((call.parent, call.entry, call.result_head), (None, None, None));
        }
        let call: ToolCall =
            serde_json::from_value(json!({"harness": "h", "store": "s", "session": "x", "input": {}})).unwrap();
        assert_eq!(call.canonical, None);
    }

    #[test]
    fn harvest_rows_read_back_whatever_shape_the_time_has() {
        // OpenCode's raw `time` is an object; harvest.py copies it as it is.
        let cases = [
            (json!("2026-11-15T10:00:00Z"), Some("2026-11-15T10:00:00Z")),
            (json!(1794474109950_i64), Some("1794474109950")),
            (json!({"created": 1794474109950_i64, "completed": 1794474113169_i64}), None),
            (json!(null), None),
        ];
        for (time, want) in cases {
            let row = json!({"harness": "h", "store": "s", "session": "x", "input": {}, "time": time});
            let call: ToolCall = serde_json::from_value(row).unwrap();
            assert_eq!(call.time.as_deref(), want, "{time}");
        }
    }
}
