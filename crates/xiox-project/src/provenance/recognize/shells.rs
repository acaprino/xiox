//! Shells that run a command line given to them: `bash -c`, `powershell -Command`, `cmd /c`. The
//! inner text is read as a command of its own, in its own dialect.

use super::{Context, Invocation, Recognizer, words};
use crate::provenance::extract::extract_nested;
use crate::provenance::{Completeness, RawOp, Shell};

pub(super) struct Shells;

/// PowerShell options that run a script file, which Xiox does not read.
const POWERSHELL_FILE_OPTIONS: [&str; 5] = ["-file", "-f", "-encodedcommand", "-e", "-ec"];
const POWERSHELL_COMMAND_OPTIONS: [&str; 4] = ["-c", "-command", "-com", "-comm"];

impl Recognizer for Shells {
    fn names(&self) -> &'static [&'static str] {
        &["bash", "sh", "zsh", "dash", "powershell", "pwsh", "cmd"]
    }

    fn recognize(&self, inv: &Invocation, ctx: &mut Context) -> Vec<RawOp> {
        let words = words(inv.args);
        let (start, inner) = script_start(&inv.name, &words);
        if inner == Shell::PowerShell
            && words.iter().any(|word| POWERSHELL_FILE_OPTIONS.contains(&word.to_lowercase().as_str()))
        {
            ctx.flags.script = true;
        }
        let script = start.and_then(|at| script_text(inner, &words[at.min(words.len())..]));
        let Some(script) = script.filter(|script| !script.is_empty()) else { return Vec::new() };
        let completeness = if inv.cut { Completeness::Truncated } else { Completeness::Whole };
        let Some(inner_ops) = extract_nested(&script, inner, completeness, ctx) else { return Vec::new() };
        // The inner shell starts where the outer one is and leaves it there.
        let mut ops = vec![RawOp::Cd { dst: Some(".".into()), push: true }];
        ops.extend(inner_ops);
        ops.push(RawOp::PopCd);
        ops
    }

    /// D3: `bash -c "…cut` reads what the script holds; a cut word that is not the script (a missing
    /// argument, a cut path after it) leaves the command unknown.
    fn reads_cut_script(&self, inv: &Invocation) -> bool {
        let words = words(inv.args);
        let (start, shell) = script_start(&inv.name, &words);
        start.is_some_and(|at| if shell == Shell::Bash { at + 1 == words.len() } else { at < words.len() })
    }
}

/// Where the script starts in `words`, and the shell that reads it.
fn script_start(name: &str, words: &[&str]) -> (Option<usize>, Shell) {
    let after = |at: usize| at + 1;
    match name {
        "bash" | "sh" | "zsh" | "dash" => (words.iter().position(|word| is_command_option(word)).map(after), Shell::Bash),
        "powershell" | "pwsh" => {
            let at = words.iter().position(|word| POWERSHELL_COMMAND_OPTIONS.contains(&word.to_lowercase().as_str()));
            (at.map(after), Shell::PowerShell)
        }
        _ => {
            let at = words.iter().position(|word| matches!(word.to_lowercase().as_str(), "/c" | "/k"));
            (at.map(after), Shell::Cmd)
        }
    }
}

/// The script the words from its start hold: one word in Bash, the rest of the line elsewhere.
fn script_text(shell: Shell, from_start: &[&str]) -> Option<String> {
    match shell {
        Shell::Bash => from_start.first().map(|script| (*script).to_owned()),
        _ => Some(from_start.join(" ")),
    }
}

/// `-c`, possibly bundled with other lowercase flags (`-lc`, `-ec`).
fn is_command_option(word: &str) -> bool {
    word.strip_prefix('-').is_some_and(|flags| flags.contains('c') && flags.bytes().all(|b| b.is_ascii_lowercase()))
}
