//! Shells that run a command line given to them: `bash -c`, `powershell -Command`, `cmd /c`. The
//! inner text is read as a command of its own, in its own dialect.

use super::{Context, Invocation, Recognizer, words};
use crate::provenance::extract::extract_nested;
use crate::provenance::{RawOp, Shell};

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
        let (script, inner) = match inv.name.as_str() {
            "bash" | "sh" | "zsh" | "dash" => {
                let at = words.iter().position(|word| is_command_option(word));
                (at.and_then(|at| words.get(at + 1)).map(|script| script.to_string()), Shell::Bash)
            }
            "powershell" | "pwsh" => {
                if words.iter().any(|word| POWERSHELL_FILE_OPTIONS.contains(&word.to_lowercase().as_str())) {
                    ctx.flags.script = true;
                }
                let at =
                    words.iter().position(|word| POWERSHELL_COMMAND_OPTIONS.contains(&word.to_lowercase().as_str()));
                (at.map(|at| words[at + 1..].join(" ")), Shell::PowerShell)
            }
            _ => {
                let at = words.iter().position(|word| matches!(word.to_lowercase().as_str(), "/c" | "/k"));
                (at.map(|at| words[at + 1..].join(" ")), Shell::Cmd)
            }
        };
        let Some(script) = script.filter(|script| !script.is_empty()) else { return Vec::new() };
        let Some(inner_ops) = extract_nested(&script, inner, ctx) else { return Vec::new() };
        // The inner shell starts where the outer one is and leaves it there.
        let mut ops = vec![RawOp::Cd { dst: Some(".".into()), push: true }];
        ops.extend(inner_ops);
        ops.push(RawOp::PopCd);
        ops
    }
}

/// `-c`, possibly bundled with other lowercase flags (`-lc`, `-ec`).
fn is_command_option(word: &str) -> bool {
    word.strip_prefix('-').is_some_and(|flags| flags.contains('c') && flags.bytes().all(|b| b.is_ascii_lowercase()))
}
