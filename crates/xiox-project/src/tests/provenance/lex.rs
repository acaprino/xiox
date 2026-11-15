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
