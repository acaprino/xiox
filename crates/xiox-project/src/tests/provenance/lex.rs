use crate::provenance::{Shell, Token, lex};

fn words(shell: Shell, cmd: &str) -> Vec<Token> {
    lex(cmd, shell.dialect())
}

fn w(text: &str) -> Token {
    Token::Word(text.into())
}

fn heredoc(body: &str) -> Token {
    Token::Heredoc(Some(body.into()))
}

#[test]
fn bash_backslashes_follow_bash_quoting() {
    let cases = [
        (r#"mkdir "D:\runs\x""#, vec![w("mkdir"), w(r"D:\runs\x")]),
        (r#"echo \"a\""#, vec![w("echo"), w("\"a\"")]),
        (r"a\ b", vec![w("a b")]),
    ];
    for (cmd, expected) in cases {
        assert_eq!(words(Shell::Bash, cmd), expected, "{cmd}");
    }
}

#[test]
fn powershell_quotes() {
    let cases = [("'it''s'", "it's"), ("\"a`\"b\"", "a\"b")];
    for (cmd, expected) in cases {
        assert_eq!(words(Shell::PowerShell, cmd), vec![w(expected)], "{cmd}");
    }
}

#[test]
fn cmd_has_no_single_quotes() {
    assert_eq!(words(Shell::Cmd, "echo 'a b'"), vec![w("echo"), w("'a"), w("b'")]);
}

#[test]
fn fd_redirects_lex_as_redirect() {
    let cases = [
        (Shell::Bash, "echo x 2> e", vec![w("echo"), w("x"), Token::Op(">"), w("e")]),
        (Shell::PowerShell, "run *> all", vec![w("run"), Token::Op(">"), w("all")]),
        (Shell::Bash, "run 2>&1", vec![w("run"), Token::Op(">&"), w("1")]),
        (Shell::Cmd, "run 2>&1", vec![w("run"), Token::Op(">&"), w("1")]),
    ];
    for (shell, cmd, expected) in cases {
        assert_eq!(words(shell, cmd), expected, "{cmd}");
    }
}

#[test]
fn dollar_paren_stays_in_one_word() {
    assert_eq!(words(Shell::Bash, r#"cp src "$(mktemp -d)""#), vec![w("cp"), w("src"), w("$(mktemp -d)")]);
    assert_eq!(words(Shell::Bash, "$(a (b))"), vec![w("$(a (b))")]);
}

#[test]
fn newline_is_a_separator() {
    assert_eq!(words(Shell::Bash, "a\nb"), vec![w("a"), Token::Op(";"), w("b")]);
}

#[test]
fn operators_after_heredoc_delimiter_are_kept() {
    let tokens = words(Shell::Bash, "python - <<'EOF' > /d/o.json\nprint(1)\nEOF");
    assert_eq!(tokens, vec![w("python"), w("-"), heredoc("print(1)\n"), Token::Op(">"), w("/d/o.json"), Token::Op(";")]);
}

#[test]
fn heredoc_delimiter_tolerates_crlf() {
    let tokens = words(Shell::Bash, "cat <<EOF\r\nx\r\nEOF\r\nmkdir /d/after");
    assert_eq!(tokens, vec![w("cat"), heredoc("x\r\n"), Token::Op(";"), Token::Op(";"), w("mkdir"), w("/d/after")]);
}

#[test]
fn indented_dash_heredoc_closes_on_tabbed_delimiter() {
    let tokens = words(Shell::Bash, "cat <<-END\n\ta\n\tEND\nb");
    assert_eq!(tokens, vec![w("cat"), heredoc("\ta\n"), Token::Op(";"), Token::Op(";"), w("b")]);
}

#[test]
fn unterminated_quote_and_heredoc_end_the_input() {
    assert_eq!(words(Shell::Bash, "echo \"abc"), vec![w("echo"), w("abc")]);
    assert_eq!(words(Shell::Bash, "echo 'abc"), vec![w("echo"), w("abc")]);
    assert_eq!(words(Shell::Bash, "cat <<EOF\na\nb"), vec![w("cat"), heredoc("a\nb"), Token::Op(";")]);
    assert_eq!(words(Shell::Bash, "cat <<EOF"), vec![w("cat"), Token::Heredoc(None)]);
    assert_eq!(words(Shell::Bash, "cat <<"), vec![w("cat"), Token::Heredoc(None)]);
    assert_eq!(words(Shell::Bash, "echo $(a (b"), vec![w("echo"), w("$(a (b")]);
    assert_eq!(words(Shell::Bash, "echo \\"), vec![w("echo"), w("\\")]);
}

#[test]
fn non_ascii_words() {
    assert_eq!(words(Shell::Bash, r#"mkdir "D:\Città\è""#), vec![w("mkdir"), w(r"D:\Città\è")]);
    assert_eq!(words(Shell::Bash, "cat <<è\nà\nè"), vec![w("cat"), heredoc("à\n"), Token::Op(";")]);
}

#[test]
fn bash_brace_expansion() {
    let ws = |texts: &[&str]| texts.iter().map(|t| w(t)).collect::<Vec<_>>();
    let cases = [
        ("mkdir -p src/{api,ui}", vec!["mkdir", "-p", "src/api", "src/ui"]),
        ("cp f{,.bak}", vec!["cp", "f", "f.bak"]),
        ("mkdir d{1..3}", vec!["mkdir", "d1", "d2", "d3"]),
        ("mkdir d{3..1}", vec!["mkdir", "d3", "d2", "d1"]),
        ("mkdir {a..c}", vec!["mkdir", "a", "b", "c"]),
        ("mkdir x{01..02}", vec!["mkdir", "x01", "x02"]),
        ("mkdir {1..10..4}", vec!["mkdir", "1", "5", "9"]),
        ("echo {a,b}{1,2}", vec!["echo", "a1", "a2", "b1", "b2"]),
        ("echo {a,{b,c}}", vec!["echo", "a", "b", "c"]),
        ("echo pre{a,b}post", vec!["echo", "preapost", "prebpost"]),
        // Quoted, escaped, parameter braces and braces without a list or a sequence stay as written.
        ("mkdir \"{a,b}\"", vec!["mkdir", "{a,b}"]),
        ("mkdir '{a,b}'", vec!["mkdir", "{a,b}"]),
        (r"echo \{a,b\}", vec!["echo", "{a,b}"]),
        ("echo ${x}{a,b}", vec!["echo", "${x}a", "${x}b"]),
        ("echo {a} {} {1..x}", vec!["echo", "{a}", "{}", "{1..x}"]),
        ("mkdir {a,b", vec!["mkdir", "{a,b"]),
        ("echo a{${x},b}", vec!["echo", "a${x}", "ab"]),
    ];
    for (cmd, expected) in cases {
        assert_eq!(words(Shell::Bash, cmd), ws(&expected), "{cmd}");
    }
    // Cmd does not expand.
    assert_eq!(words(Shell::Cmd, "mkdir a{b,c}"), vec![w("mkdir"), w("a{b,c}")]);
}

#[test]
fn bash_brace_expansion_has_a_limit_and_never_panics() {
    let sixty = (0..60).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let fifty = (0..50).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let over = [format!("echo {{{sixty}}}"), "echo {1..51}".into(), "echo {a,b}{1..30}".into()];
    for cmd in over {
        assert_eq!(words(Shell::Bash, &cmd).len(), 2, "{cmd}");
    }
    assert_eq!(words(Shell::Bash, &format!("echo {{{fifty}}}")).len(), 51);
    let odd = ["{", "}", "{,", ",}", "{a,", "{{{{,,,,}}}}", "{é,→}{🙂..a}", "{..}", "{1..}", "{-3..-1}", "x{a,b}\\",
        "{9223372036854775807..-9223372036854775808}"];
    for cmd in odd {
        words(Shell::Bash, cmd);
    }
    assert_eq!(words(Shell::Bash, "echo {é,→}"), vec![w("echo"), w("é"), w("→")]);
    assert_eq!(words(Shell::Bash, "echo {-1..1}"), vec![w("echo"), w("-1"), w("0"), w("1")]);
}

#[test]
fn deeply_braced_words_stay_literal_and_never_overflow_the_stack() {
    // Mukade cuts a command at 4,000 characters: the largest word the lexer is given.
    let cases = [
        format!("echo {}{}", "{,".repeat(1333), "}".repeat(1333)),
        format!("echo {}", "{a,b}".repeat(2000)),
        format!("echo {}{}{}", "{a,".repeat(700), "x", "}".repeat(700)),
        format!("echo {}", "{a,b}{1..3}".repeat(300)),
    ];
    for cmd in cases {
        let tokens = words(Shell::Bash, &cmd);
        assert_eq!(tokens.len(), 2, "{} characters", cmd.len());
        assert_eq!(tokens[1], w(cmd.strip_prefix("echo ").unwrap()));
    }
}

#[test]
fn powershell_here_strings_are_one_word() {
    // D13: the body is the text between the lines, whatever it holds.
    let cases = [
        ("Set-Content x.txt @'\nline one\nline two\n'@", vec![w("Set-Content"), w("x.txt"), w("line one\nline two")]),
        ("Set-Content x.txt @\"\na b\n\"@; mkdir d", vec![w("Set-Content"), w("x.txt"), w("a b"), Token::Op(";"), w("mkdir"), w("d")]),
        ("f @'\r\n'x'\r\n'@", vec![w("f"), w("'x'")]),
        ("f @'\n\n'@ g", vec![w("f"), w(""), w("g")]),
        ("f @'\nà é\n'@", vec![w("f"), w("à é")]),
    ];
    for (cmd, expected) in cases {
        assert_eq!(words(Shell::PowerShell, cmd), expected, "{cmd:?}");
    }
}

#[test]
fn an_unterminated_here_string_takes_the_rest_and_never_panics() {
    assert_eq!(words(Shell::PowerShell, "f @'\nmkdir x"), vec![w("f"), w("mkdir x")]);
    assert_eq!(words(Shell::PowerShell, "f @'\n"), vec![w("f"), w("")]);
    assert_eq!(words(Shell::PowerShell, "f @'"), vec![w("f"), w("@")]);
    assert_eq!(words(Shell::PowerShell, "f @'\n'"), vec![w("f"), w("'")]);
}

#[test]
fn an_at_sign_elsewhere_is_unchanged() {
    assert_eq!(words(Shell::PowerShell, "a@'b'"), vec![w("a@b")]);
    assert_eq!(words(Shell::PowerShell, "f @'x' g"), vec![w("f"), w("@x"), w("g")]);
    assert_eq!(words(Shell::PowerShell, "f @(1)"), vec![w("f"), w("@"), Token::Op("("), w("1"), Token::Op(")")]);
    assert_eq!(words(Shell::Bash, "f @'\nx\n'@"), vec![w("f"), w("@x"), Token::Op(";"), w("@")]);
}
