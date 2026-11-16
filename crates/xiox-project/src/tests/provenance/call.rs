use serde_json::{json, Value};
use xiox_mukade::{ToolCall, CUT_MARK, MAX_INPUT_CHARS};

use crate::provenance::Completeness::{Truncated, Whole};
use crate::provenance::Shell::{Bash, Cmd, PowerShell};
use crate::provenance::{shell_command, Completeness, Shell};

fn call(canonical: Option<&str>, tool: Option<&str>, input: Value) -> ToolCall {
    ToolCall {
        harness: "test".into(),
        store: "store".into(),
        session: "s1".into(),
        parent: None,
        entry: None,
        call: None,
        tool: tool.map(Into::into),
        canonical: canonical.map(Into::into),
        input,
        cwd: None,
        time: None,
        error: None,
        exit: None,
        result_head: None,
    }
}

fn cut(prefix: &str) -> String {
    format!("{prefix}{}{CUT_MARK}", "x".repeat(MAX_INPUT_CHARS - prefix.len()))
}

/// D15: OpenCode's `shell` tool runs PowerShell on Windows; a hint in the call still wins.
#[test]
fn opencode_shell_is_powershell_unless_hinted() {
    let rows = [
        ("opencode shell", "opencode", "shell", json!({"command": "ls"}), Some(PowerShell)),
        ("opencode Shell, any case", "opencode", "Shell", json!({"command": "ls"}), Some(PowerShell)),
        ("shell hint wins", "opencode", "shell", json!({"command": "ls", "shell": "bash"}), Some(Bash)),
        ("shell_type hint wins", "opencode", "shell", json!({"command": "dir", "shell_type": "cmd"}), Some(Cmd)),
        ("opencode bash tool stays Bash", "opencode", "bash", json!({"command": "ls"}), Some(Bash)),
        ("another harness's shell tool stays Bash", "codex", "shell", json!({"command": "ls"}), Some(Bash)),
    ];
    for (name, harness, tool, input, want) in rows {
        let mut c = call(Some("bash"), Some(tool), input);
        c.harness = harness.into();
        assert_eq!(shell_command(&c).map(|c| c.shell), want, "{name}");
    }
}

#[test]
fn shell_selection_table() {
    type Expected<'a> = Option<(Shell, &'a str, Completeness)>;
    let cut_text = cut("rm ");
    let argv_text = format!("bash -lc \"{cut_text}\"");
    let cases: Vec<(&str, Option<&str>, Option<&str>, Value, Expected)> = vec![
        ("claude bash", Some("bash"), Some("Bash"), json!({"command": "ls -la"}), Some((Bash, "ls -la", Whole))),
        (
            "codex shell argv, no hint (R2)",
            None,
            Some("shell"),
            json!({"command": ["powershell.exe", "-Command", "mkdir D:\\x"]}),
            Some((Bash, "powershell.exe -Command \"mkdir D:\\x\"", Whole)),
        ),
        (
            "codex exec_command with pwsh hint",
            None,
            Some("exec_command"),
            json!({"cmd": "mkdir x", "shell": "pwsh"}),
            Some((PowerShell, "mkdir x", Whole)),
        ),
        ("shell_type hint", None, Some("shell"), json!({"cmd": "dir", "shell_type": "cmd"}), Some((Cmd, "dir", Whole))),
        ("empty shell falls to shell_type", None, Some("shell"), json!({"cmd": "a", "shell": "", "shell_type": "PowerShell"}), Some((PowerShell, "a", Whole))),
        ("opencode bash", Some("bash"), Some("bash"), json!({"command": "git status"}), Some((Bash, "git status", Whole))),
        ("terminal tool", None, Some("terminal"), json!({"command": "ls"}), Some((Bash, "ls", Whole))),
        ("cmd tool", Some("cmd"), Some("Cmd"), json!({"command": "dir /b"}), Some((Cmd, "dir /b", Whole))),
        ("tool name when canonical is not a string", None, Some("PowerShell"), json!({"command": "ls"}), Some((PowerShell, "ls", Whole))),
        ("canonical wins over tool name", Some("bash"), Some("PowerShell"), json!({"command": "ls"}), Some((Bash, "ls", Whole))),
        ("read is no shell", Some("read"), Some("Read"), json!({"command": "x"}), None),
        ("no tool at all", None, None, json!({"command": "x"}), None),
        ("empty command", Some("bash"), Some("Bash"), json!({"command": "  \n"}), None),
        ("command key present but empty hides cmd", Some("bash"), None, json!({"command": "", "cmd": "ls"}), None),
        ("non-string command", Some("bash"), None, json!({"command": 7}), None),
        ("no command", Some("bash"), None, json!({"other": "x"}), None),
        ("padding trimmed", Some("bash"), None, json!({"command": "\n  ls \n"}), Some((Bash, "ls", Whole))),
        ("cut text", Some("bash"), None, json!({"command": cut_text.clone()}), Some((Bash, cut_text.trim(), Truncated))),
        (
            "cut argv element",
            None,
            Some("shell"),
            json!({"command": ["bash", "-lc", cut_text.clone()]}),
            Some((Bash, argv_text.as_str(), Truncated)),
        ),
    ];
    for (name, canonical, tool, input, expected) in cases {
        let got = shell_command(&call(canonical, tool, input)).map(|c| (c.shell, c.text, c.completeness));
        let want = expected.map(|(s, t, c)| (s, t.to_string(), c));
        assert_eq!(got, want, "{name}");
    }
}
