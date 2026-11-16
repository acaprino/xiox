//! Which tool calls are shell commands, and in which dialect. One classifier for the session
//! ledger and the parity probe.

use serde_json::Value;
use xiox_mukade::{was_cut, ToolCall};

use super::{Completeness, Shell};

/// How a shell tool name picks its dialect.
#[derive(Clone, Copy)]
enum Dialect {
    Fixed(Shell),
    /// The tool runs whatever shell the call asks for.
    Hinted,
}

/// Shell tools by canonical name or lowercased tool name.
const SHELLS: &[(&str, Dialect)] = &[
    ("bash", Dialect::Fixed(Shell::Bash)),
    ("terminal", Dialect::Fixed(Shell::Bash)),
    ("power_shell", Dialect::Fixed(Shell::PowerShell)),
    ("powershell", Dialect::Fixed(Shell::PowerShell)),
    ("cmd", Dialect::Fixed(Shell::Cmd)),
    ("shell", Dialect::Hinted),
    ("exec_command", Dialect::Hinted),
];

/// A tool call known to run a shell command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellCommand {
    pub shell: Shell,
    pub text: String,
    pub completeness: Completeness,
}

pub fn shell_command(call: &ToolCall) -> Option<ShellCommand> {
    let dialect = dialect_of(call)?;
    let command = call.input.get("command").or_else(|| call.input.get("cmd"))?;
    let (text, cut) = command_text(command)?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(ShellCommand {
        shell: match dialect {
            Dialect::Fixed(_) if is_opencode_shell(call) => hinted_shell(&call.input, Shell::PowerShell),
            Dialect::Fixed(shell) => shell,
            Dialect::Hinted => hinted_shell(&call.input, Shell::Bash),
        },
        text: text.to_string(),
        completeness: if cut { Completeness::Truncated } else { Completeness::Whole },
    })
}

/// D15: OpenCode's `shell` tool (Mukade canonical `bash`) runs PowerShell on Windows. Measured on the
/// real history: of 4,724 such calls 3,520 are written in PowerShell syntax and only 50 in Bash-only syntax.
fn is_opencode_shell(call: &ToolCall) -> bool {
    call.harness == "opencode" && call.tool.as_deref().is_some_and(|tool| tool.eq_ignore_ascii_case("shell"))
}

/// The canonical name decides when it is a shell tool, the tool name otherwise.
fn dialect_of(call: &ToolCall) -> Option<Dialect> {
    let lookup = |name: &str| SHELLS.iter().find(|(n, _)| *n == name).map(|(_, d)| *d);
    call.canonical
        .as_deref()
        .and_then(lookup)
        .or_else(|| lookup(&call.tool.as_deref()?.to_lowercase()))
}

/// The command as one string and whether any part of it was cut; an argv list is joined with
/// spaces, quoting the elements that contain one.
fn command_text(command: &Value) -> Option<(String, bool)> {
    match command {
        Value::String(s) => Some((s.clone(), was_cut(s))),
        Value::Array(argv) => {
            let parts: Vec<&str> = argv.iter().map(Value::as_str).collect::<Option<_>>()?;
            let text = parts
                .iter()
                .map(|p| if p.contains(' ') { format!("\"{p}\"") } else { p.to_string() })
                .collect::<Vec<_>>()
                .join(" ");
            Some((text, parts.iter().any(|p| was_cut(p))))
        }
        _ => None,
    }
}

/// The shell a call names in `shell` or `shell_type`; `default` when it names none.
fn hinted_shell(input: &Value, default: Shell) -> Shell {
    let hint = ["shell", "shell_type"]
        .iter()
        .filter_map(|key| input.get(key)?.as_str())
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_lowercase();
    if hint.is_empty() {
        default
    } else if hint.contains("power") || hint.contains("pwsh") {
        Shell::PowerShell
    } else if hint == "cmd" {
        Shell::Cmd
    } else {
        Shell::Bash
    }
}
