use std::collections::HashSet;
use std::sync::OnceLock;

use super::fixtures::{Fixture, load_fixtures};
use crate::provenance::{Budget, Completeness, Env, Extraction, OpKind, RawOp, Resolved, Shell, extract, resolve};

/// Fixtures the extractor cannot read, with the reason: the limits the specification names.
const KNOWN_LIMITS: [&str; 1] = ["u06"]; // ForEach-Object: its body runs once per piped item

fn shell_of(name: &str) -> Shell {
    match name {
        "bash" => Shell::Bash,
        "powershell" => Shell::PowerShell,
        "cmd" => Shell::Cmd,
        other => panic!("unknown shell {other}"),
    }
}

fn env() -> Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| {
        let (home, temp) = load_fixtures().env;
        Env { home, temp }
    })
    .clone()
}

type Effect = (OpKind, String);

fn effects(resolved: &Resolved) -> HashSet<Effect> {
    resolved.candidates.iter().map(|c| (c.kind, c.dst.to_lowercase())).collect()
}

fn gold_effects(fixture: &Fixture) -> HashSet<Effect> {
    fixture.gold.iter().map(|(kind, _, dst)| (*kind, dst.to_lowercase())).collect()
}

fn read(fixture: &Fixture, completeness: Completeness) -> (Extraction, Resolved) {
    let extraction = extract(&fixture.cmd, shell_of(&fixture.shell), completeness);
    let resolved = resolve(&extraction.ops, &fixture.cwd, &env());
    (extraction, resolved)
}

fn run(shell: Shell, cmd: &str) -> (Extraction, Resolved) {
    let extraction = extract(cmd, shell, Completeness::Whole);
    let resolved = resolve(&extraction.ops, r"D:\Projects\Phazly", &env());
    (extraction, resolved)
}

fn kinds_and_dsts(resolved: &Resolved) -> Vec<(OpKind, &str)> {
    resolved.candidates.iter().map(|c| (c.kind, c.dst.as_str())).collect()
}

fn dsts(resolved: &Resolved) -> Vec<&str> {
    resolved.candidates.iter().map(|c| c.dst.as_str()).collect()
}

#[test]
fn every_fixture_is_exact_except_known_limits() {
    for fixture in &load_fixtures().events {
        if KNOWN_LIMITS.contains(&fixture.id.as_str()) {
            continue;
        }
        let (_, resolved) = read(fixture, Completeness::Whole);
        assert_eq!(effects(&resolved), gold_effects(fixture), "{}: {}", fixture.id, fixture.cmd);
    }
}

#[test]
fn no_fixture_invents_a_path() {
    for fixture in &load_fixtures().events {
        let (_, resolved) = read(fixture, Completeness::Whole);
        let invented: Vec<_> = effects(&resolved).difference(&gold_effects(fixture)).cloned().collect();
        assert!(invented.is_empty(), "{}: invented {invented:?}", fixture.id);
    }
}

#[test]
fn no_fixture_exhausts_the_budget() {
    for fixture in &load_fixtures().events {
        let (extraction, _) = read(fixture, Completeness::Whole);
        assert!(!extraction.flags.exhausted, "{}", fixture.id);
    }
}

/// Every prefix of `cmd` that ends on a `char` boundary, the empty one included.
fn prefixes(cmd: &str) -> impl Iterator<Item = &str> {
    cmd.char_indices().map(|(at, _)| &cmd[..at]).chain(std::iter::once(cmd))
}

#[test]
fn every_prefix_of_every_fixture_extracts_without_panic() {
    for fixture in &load_fixtures().events {
        let shell = shell_of(&fixture.shell);
        for prefix in prefixes(&fixture.cmd) {
            for completeness in [Completeness::Whole, Completeness::Truncated] {
                extract(prefix, shell, completeness);
            }
        }
    }
}

#[test]
fn truncated_prefixes_never_invent() {
    // D3: the text may end inside a word, a quote or a heredoc, so its last command says nothing certain.
    for fixture in &load_fixtures().events {
        let gold = gold_effects(fixture);
        let shell = shell_of(&fixture.shell);
        for prefix in prefixes(&fixture.cmd).filter(|prefix| prefix.len() < fixture.cmd.len()) {
            let extraction = extract(prefix, shell, Completeness::Truncated);
            let resolved = resolve(&extraction.ops, &fixture.cwd, &env());
            let invented: Vec<_> = effects(&resolved).difference(&gold).cloned().collect();
            assert!(invented.is_empty(), "{}: {prefix:?} invents {invented:?}", fixture.id);
        }
    }
}

#[test]
fn a_truncated_destination_is_dropped_and_counted() {
    let cmd = r#"mkdir D:\keep; mkdir "D:\runs\destination"#;
    let cut = &cmd[..cmd.find("destin").unwrap() + "destin".len()];
    let truncated = extract(cut, Shell::PowerShell, Completeness::Truncated);
    let resolved = resolve(&truncated.ops, r"D:\w", &env());
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Create, r"D:\keep")]);
    assert!(truncated.flags.truncated);
    assert_eq!(truncated.unresolved, 1);
    // The prototype reads the same text as whole, and shortens the destination.
    let whole = extract(cut, Shell::PowerShell, Completeness::Whole);
    let resolved = resolve(&whole.ops, r"D:\w", &env());
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Create, r"D:\keep"), (OpKind::Create, r"D:\runs\destin")]);
    assert!(!whole.flags.truncated);
}

#[test]
fn command_after_crlf_heredoc_is_extracted() {
    // D2: `EOF\r` closes the body.
    let (_, resolved) = run(Shell::Bash, "cat <<EOF > /d/a.txt\r\nx\r\nEOF\r\nmkdir /d/after");
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Write, r"D:\a.txt"), (OpKind::Create, r"D:\after")]);
}

fn nested_shells(levels: usize) -> String {
    (0..levels).fold("mkdir /d/deep".to_owned(), |inner, _| format!("bash -c '{}'", inner.replace('\'', r"'\''")))
}

#[test]
fn nested_loops_and_shells_stop_at_the_budget() {
    // D4: a shared budget caps expansion, depth and operations.
    let items = (1..=50).map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
    let cmd = format!("for a in {items}; do for b in {items}; do for c in {items}; do mkdir /d/$a$b$c; done; done; done");
    let (extraction, _) = run(Shell::Bash, &cmd);
    assert!(extraction.ops.len() <= Budget::MAX_OPS, "{} operations", extraction.ops.len());
    assert!(extraction.flags.exhausted);
    assert!(extraction.unresolved > 0);

    let (extraction, _) = run(Shell::Bash, &nested_shells(10));
    assert!(extraction.flags.exhausted);

    let (extraction, resolved) = run(Shell::Bash, "for i in 1 2; do mkdir /d/$i; done");
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Create, r"D:\1"), (OpKind::Create, r"D:\2")]);
    assert!(!extraction.flags.exhausted);
    assert_eq!(extraction.unresolved, 0);
    // Nested shells within the depth limit stay exact.
    let (extraction, resolved) = run(Shell::Bash, &nested_shells(Budget::MAX_DEPTH));
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Create, r"D:\deep")]);
    assert!(!extraction.flags.exhausted);
}

fn script_flags_of(extraction: &Extraction) -> Vec<bool> {
    extraction
        .ops
        .iter()
        .filter_map(|op| match op {
            RawOp::Path { script, .. } => Some(*script),
            _ => None,
        })
        .collect()
}

#[test]
fn bun_has_one_owner_and_three_forms() {
    // PowerShell: in Bash a backslash would escape the next character.
    let (_, resolved) = run(Shell::PowerShell, r"bun create vite D:\a");
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Create, r"D:\a")]);
    let (_, resolved) = run(Shell::PowerShell, r"bun init D:\b");
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Create, r"D:\b")]);
    let (extraction, resolved) = run(Shell::Bash, r#"bun -e "require('fs').mkdirSync('D:/c')""#);
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Create, r"D:\c")]);
    assert!(extraction.flags.script);
    assert_eq!(script_flags_of(&extraction), vec![true]);
}

#[test]
fn unknown_values_are_counted_not_guessed() {
    let cases = [
        (Shell::PowerShell, r#"New-Item "$env:TEMP\x""#),
        (Shell::Cmd, r"mkdir %TEMP%\y"),
        (Shell::Bash, r#"cp a "$(mktemp -d)""#),
        (Shell::Bash, "mkdir `pwd`/z"),
        (Shell::Bash, "T=$(mktemp -d); mkdir $T/x"),
        // Reassigning to a value that is not literal forgets the earlier literal.
        (Shell::Bash, "T=/d/old; T=$(mktemp -d); mkdir $T/x"),
        (Shell::Bash, r#"python -c "import os; os.makedirs(f'{r}/x')""#),
    ];
    for (shell, cmd) in cases {
        let (extraction, resolved) = run(shell, cmd);
        assert!(resolved.candidates.is_empty(), "{cmd}: {:?}", resolved.candidates);
        assert!(extraction.unresolved + resolved.unresolved > 0, "{cmd}: not counted");
    }
}

#[test]
fn known_variables_are_substituted_per_dialect() {
    let cases = [
        (Shell::Bash, "D=/d/runs; mkdir $D/a ${D}/b", vec![r"D:\runs\a", r"D:\runs\b"]),
        (Shell::PowerShell, r"$d = 'D:\runs'; mkdir $D\a", vec![r"D:\runs\a"]),
        (Shell::Cmd, r"set d=D:\runs && mkdir %d%\a", vec![r"D:\runs\a"]),
        (Shell::Bash, "N=/d/r; for i in 1 2; do mkdir $N$i; done", vec![r"D:\r1", r"D:\r2"]),
    ];
    for (shell, cmd, expected) in cases {
        let (_, resolved) = run(shell, cmd);
        assert_eq!(dsts(&resolved), expected, "{cmd}");
    }
}

#[test]
fn prefix_assignment_reaches_the_command() {
    let (_, resolved) = run(Shell::Bash, "X=1 mkdir /d/r");
    assert_eq!(kinds_and_dsts(&resolved), vec![(OpKind::Create, r"D:\r")]);
}

#[test]
fn flags_mark_scripts_and_loops() {
    let fixtures = load_fixtures();
    let flags_of = |id: &str| {
        let fixture = fixtures.events.iter().find(|f| f.id == id).unwrap();
        read(fixture, Completeness::Whole).0.flags
    };
    assert!(flags_of("x01").script);
    assert!(flags_of("x13").looped);
    let quiet = flags_of("c01");
    assert!(!quiet.script && !quiet.looped && !quiet.truncated && !quiet.exhausted);
}

#[test]
fn redirects_and_heredocs_feed_later_commands() {
    let cases = [
        // `done > file` writes the file once the loop ends.
        (Shell::Bash, "for i in 1 2; do echo $i; done > /d/log.txt", vec![(OpKind::Write, r"D:\log.txt")]),
        // A script written by a heredoc and run later in the same command line is read.
        (
            Shell::Bash,
            "cat > /d/m.py <<EOF\nimport os\nos.makedirs('D:/made')\nEOF\npython /d/m.py",
            vec![(OpKind::Write, r"D:\m.py"), (OpKind::Create, r"D:\made")],
        ),
        (Shell::Bash, "ls > /dev/null", vec![]),
    ];
    for (shell, cmd, expected) in cases {
        let (_, resolved) = run(shell, cmd);
        assert_eq!(kinds_and_dsts(&resolved), expected, "{cmd}");
    }
}

#[test]
fn interpreters_with_a_heredoc_always_mark_the_script() {
    for name in ["python", "node", "ruby", "bash", "pwsh"] {
        let (extraction, _) = run(Shell::Bash, &format!("{name} <<EOF\nnothing\nEOF"));
        assert!(extraction.flags.script, "{name}");
    }
    let (extraction, _) = run(Shell::Bash, "cat <<EOF\nnothing\nEOF");
    assert!(!extraction.flags.script);
}

#[test]
fn nested_shells_run_in_a_scope_of_their_own() {
    let cases = [
        (Shell::Bash, r"bash -c 'cd /d/w && mkdir sub'; mkdir after", vec![r"D:\w\sub", r"D:\Projects\Phazly\after"]),
        (Shell::Bash, r#"pwsh -Command "mkdir D:\x""#, vec![r"D:\x"]),
        (Shell::Cmd, r"cmd /c mkdir D:\y", vec![r"D:\y"]),
    ];
    for (shell, cmd, expected) in cases {
        let (_, resolved) = run(shell, cmd);
        assert_eq!(dsts(&resolved), expected, "{cmd}");
    }
    let (extraction, _) = run(Shell::Bash, "pwsh -File run.ps1");
    assert!(extraction.flags.script);
}
