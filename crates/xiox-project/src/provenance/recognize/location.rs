//! `cd` and `pushd` change the directory later paths are relative to; `popd` undoes a `pushd`.

use super::{Context, Invocation, Recognizer, powershell_with_path, ps_paths, words};
use crate::provenance::{RawOp, Shell};

pub(super) struct Location;

impl Recognizer for Location {
    fn names(&self) -> &'static [&'static str] {
        &["cd", "chdir", "set-location", "sl", "pushd", "push-location", "popd", "pop-location"]
    }

    fn recognize(&self, inv: &Invocation, _ctx: &mut Context) -> Vec<RawOp> {
        if matches!(inv.name.as_str(), "popd" | "pop-location") {
            return vec![RawOp::PopCd];
        }
        let push = matches!(inv.name.as_str(), "pushd" | "push-location");
        target(inv).map(|dst| RawOp::Cd { dst: Some(dst), push }).into_iter().collect()
    }
}

/// What `cd -` stands for: the previous directory, which the extractor cannot name.
const PREVIOUS_DIRECTORY: &str = "$OLDPWD";

fn target(inv: &Invocation) -> Option<String> {
    let dst = match inv.shell {
        Shell::PowerShell => ps_paths(&powershell_with_path(inv.args, &[])).into_iter().next(),
        Shell::Cmd => {
            let mut words = words(inv.args).into_iter();
            words.find(|word| !word.eq_ignore_ascii_case("/d")).map(str::to_owned)
        }
        // A bare `cd` goes home.
        Shell::Bash => Some(bash_directory(&words(inv.args)).unwrap_or("~").to_owned()),
    };
    dst.map(|dst| if dst == "-" { PREVIOUS_DIRECTORY.to_owned() } else { dst })
}

/// The first word that is not an option (`-L`, `-P`, `-e`, `-@`); `--` ends the options.
fn bash_directory<'a>(words: &[&'a str]) -> Option<&'a str> {
    let mut words = words.iter().copied();
    while let Some(word) = words.next() {
        match word {
            "--" => return words.next(),
            _ if word.len() > 1 && word.starts_with('-') => {}
            _ => return Some(word),
        }
    }
    None
}
