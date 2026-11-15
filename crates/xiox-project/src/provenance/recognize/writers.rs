//! Commands that write the files they are given: `tee`, `Out-File`, `Set-Content`, `touch`, and
//! `sed -i`.

use super::{Context, Invocation, Recognizer, powershell_with_path, ps_paths, words, write_op};
use crate::provenance::{Arg, RawOp, Shell, posix};

pub(super) struct Writers;

impl Recognizer for Writers {
    fn names(&self) -> &'static [&'static str] {
        &["tee", "tee-object", "out-file", "set-content", "add-content", "sc", "ac", "touch", "sed"]
    }

    fn recognize(&self, inv: &Invocation, _ctx: &mut Context) -> Vec<RawOp> {
        let paths = match (inv.name.as_str(), inv.shell) {
            ("sed", _) => sed_files(inv.args),
            (_, Shell::PowerShell) => ps_paths(&powershell_with_path(inv.args, &CMDLET_PARAMS)),
            _ => posix(inv.args, &["-d", "-r", "-t"]).positional,
        };
        paths.into_iter().map(write_op).collect()
    }
}

/// Parameters that take a value and are not paths; without them the value would read as a path.
const CMDLET_PARAMS: [(&str, &str); 8] = [
    ("filepath", "path"),
    ("value", "x"),
    ("encoding", "x"),
    ("width", "x"),
    ("inputobject", "x"),
    ("stream", "x"),
    ("filter", "x"),
    ("include", "x"),
];

const SCRIPT_OPTIONS: [&str; 4] = ["-e", "--expression", "-f", "--file"];

/// `-i`, possibly bundled with other short flags (`-ni`, `-i.bak`).
fn is_short_in_place(word: &str) -> bool {
    word.strip_prefix('-')
        .is_some_and(|rest| rest.trim_start_matches(['n', 'E', 'r', 's', 'z']).starts_with('i'))
}

fn is_in_place(word: &str) -> bool {
    word == "--in-place" || word.starts_with("--in-place=") || is_short_in_place(word)
}

/// The files `sed -i` edits; nothing without in-place editing. The first positional argument is
/// the script, unless `-e` or `-f` gave it.
fn sed_files(args: &[Arg]) -> Vec<String> {
    let words = words(args);
    if !words.iter().any(|word| is_in_place(word)) {
        return Vec::new();
    }
    let rest: Vec<Arg> = words
        .iter()
        .filter(|word| !is_short_in_place(word))
        .map(|word| Arg::Text((*word).into()))
        .collect();
    let parsed = posix(&rest, &SCRIPT_OPTIONS);
    let scripted = SCRIPT_OPTIONS.iter().any(|key| parsed.options.contains_key(*key));
    parsed.positional.into_iter().skip(usize::from(!scripted)).collect()
}
