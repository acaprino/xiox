use crate::provenance::{Arg, Completeness, Node, Shell, Simple, lex, parse};

fn nodes(shell: Shell, cmd: &str) -> Vec<Node> {
    let dialect = shell.dialect();
    parse(lex(cmd, dialect), dialect, Completeness::Whole)
}

fn text(words: &[&str]) -> Vec<Arg> {
    words.iter().map(|word| Arg::Text((*word).into())).collect()
}

fn simple(words: &[&str], redirects: &[&str]) -> Node {
    Node::Simple(Simple { words: text(words), redirects: redirects.iter().map(|r| (*r).into()).collect(), ..Simple::default() })
}

fn assign(name: &str, value: &str) -> Node {
    Node::Assign { name: name.into(), value: value.into() }
}

#[test]
fn separators_split_simple_commands() {
    let cases = [
        (Shell::Bash, "a && b || c ; d | e & f", vec!["a", "b", "c", "d", "e", "f"]),
        (Shell::Bash, "(a; b)", vec!["a", "b"]),
        (Shell::Bash, "a |& b", vec!["a", "b"]),
        (Shell::PowerShell, "& { a; b }", vec!["a", "b"]),
        (Shell::PowerShell, "(a) ; b", vec!["a", "b"]),
    ];
    for (shell, cmd, heads) in cases {
        let expected: Vec<Node> = heads.iter().map(|head| simple(&[head], &[])).collect();
        assert_eq!(nodes(shell, cmd), expected, "{cmd}");
    }
}

#[test]
fn redirect_targets_leave_the_words() {
    let cases = [
        (Shell::Bash, "cargo bench > out.txt 2>&1", simple(&["cargo", "bench"], &["out.txt"])),
        (Shell::Bash, "cargo bench >> a &> b", simple(&["cargo", "bench"], &["a", "b"])),
        (Shell::Bash, "sort < in.txt", simple(&["sort"], &[])),
        (Shell::Bash, "run 2>&1 tail", simple(&["run", "tail"], &[])),
        (Shell::Bash, "> only.txt", simple(&[], &["only.txt"])),
    ];
    for (shell, cmd, expected) in cases {
        assert_eq!(nodes(shell, cmd), vec![expected], "{cmd}");
    }
}

#[test]
fn assignments_per_dialect() {
    let cases = [
        (Shell::Bash, "W=/d/r", assign("W", "/d/r")),
        (Shell::PowerShell, r"$o = 'D:\x'", assign("o", r"D:\x")),
        (Shell::PowerShell, r"$O='D:\x'", assign("o", r"D:\x")),
        (Shell::Cmd, "set X=v", assign("x", "v")),
    ];
    for (shell, cmd, expected) in cases {
        assert_eq!(nodes(shell, cmd), vec![expected], "{cmd}");
    }
}

#[test]
fn leading_assignments_are_dropped_from_a_command() {
    assert_eq!(nodes(Shell::Bash, "X=1 Y=2 mkdir /d/r"), vec![simple(&["mkdir", "/d/r"], &[])]);
}

#[test]
fn bash_for_loop_collects_body_until_matching_done() {
    let expected = Node::For {
        var: "i".into(),
        items: Some(vec!["1".into(), "2".into()]),
        body: vec![
            simple(&["mkdir", "$i"], &[]),
            Node::For {
                var: "j".into(),
                items: Some(vec!["a".into()]),
                body: vec![simple(&[":"], &[])],
                done_redirects: vec![],
            },
        ],
        done_redirects: vec!["log".into()],
    };
    let cmd = "for i in 1 2; do mkdir $i; for j in a; do :; done; done > log";
    assert_eq!(nodes(Shell::Bash, cmd), vec![expected]);
    let after = nodes(Shell::Bash, "for i in 1; do a; done; b");
    assert_eq!(after.len(), 2);
    assert_eq!(after[1], simple(&["b"], &[]));
}

#[test]
fn for_over_arithmetic_or_missing_in_has_no_items() {
    for cmd in ["for ((i=0;i<3;i++)); do a; done", "for x; do a; done"] {
        match &nodes(Shell::Bash, cmd)[0] {
            Node::For { items, .. } => assert_eq!(*items, None, "{cmd}"),
            other => panic!("{cmd}: {other:?}"),
        }
    }
}

#[test]
fn keywords_do_then_else_are_stripped() {
    let cases = [
        ("then mkdir /d/a", vec![simple(&["mkdir", "/d/a"], &[])]),
        (
            "if x; then a; else b; fi",
            vec![simple(&["x"], &[]), simple(&["a"], &[]), simple(&["b"], &[]), simple(&["fi"], &[])],
        ),
        ("sudo env nohup time mkdir /d/a", vec![simple(&["mkdir", "/d/a"], &[])]),
        ("do", vec![]),
    ];
    for (cmd, expected) in cases {
        assert_eq!(nodes(Shell::Bash, cmd), expected, "{cmd}");
    }
}

#[test]
fn powershell_commas_are_kept_as_args() {
    let expected = Node::Simple(Simple {
        words: vec![Arg::Text("mkdir".into()), Arg::Text("a".into()), Arg::Comma, Arg::Text("b".into())],
        ..Simple::default()
    });
    assert_eq!(nodes(Shell::PowerShell, "mkdir a,b"), vec![expected]);
}

#[test]
fn heredoc_bodies_ride_on_their_command() {
    let parsed = nodes(Shell::Bash, "cat > x.py <<END\nhi\nEND");
    let expected =
        Simple { words: text(&["cat"]), redirects: vec!["x.py".into()], heredoc: Some("hi\n".into()), ..Simple::default() };
    assert_eq!(parsed, vec![Node::Simple(expected)]);
}

#[test]
fn first_heredoc_with_a_body_wins() {
    // The second heredoc has no body (the text ends before it starts): it must not replace the first.
    let parsed = nodes(Shell::Bash, "cat <<A <<B\nfirst\nA");
    let Node::Simple(simple) = &parsed[0] else { panic!("not simple: {parsed:?}") };
    assert_eq!(simple.heredoc.as_deref(), Some("first\n"));
    let parsed = nodes(Shell::Bash, "cat <<A <<B\nfirst\nA\nsecond\nB");
    let Node::Simple(simple) = &parsed[0] else { panic!("not simple: {parsed:?}") };
    assert_eq!(simple.heredoc.as_deref(), Some("first\n"));
}

fn loop_depth(nodes: &[Node]) -> usize {
    match nodes {
        [Node::For { body, .. }] => 1 + loop_depth(body),
        _ => 0,
    }
}

#[test]
fn nested_loops_close_innermost_first_in_one_pass() {
    let deep = 300;
    let cmd = format!("{}x{}", "for i in 1; do ".repeat(deep), "; done".repeat(deep));
    let parsed = nodes(Shell::Bash, &cmd);
    assert_eq!(parsed.len(), 1);
    assert_eq!(loop_depth(&parsed), deep);
    // An outer loop that never closes takes the rest of the text; the inner one still closes.
    let parsed = nodes(Shell::Bash, "for a in 1; do for b in 2; do x; done; y");
    assert_eq!(parsed.len(), 1);
    let Node::For { body, .. } = &parsed[0] else { panic!("not a loop") };
    assert_eq!(body.len(), 2);
    assert!(matches!(body[0], Node::For { .. }));
    assert_eq!(body[1], simple(&["y"], &[]));
}
