//! Words and operators of one command, with the shell's own quoting. One pass over the characters.

use std::str::CharIndices;

use super::dialect::{Dialect, Shell};

/// A lexical unit of a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Word(String),
    Op(&'static str),
    /// A `<<` operator with its body; `None` when the input ended before the body started.
    Heredoc(Option<String>),
}

/// Splits `cmd` into tokens. A newline is a `;` operator; heredoc bodies ride on their token.
pub fn lex(cmd: &str, dialect: &dyn Dialect) -> Vec<Token> {
    // D2: unlike the Python prototype, a `\r` after a heredoc delimiter still closes the body.
    Lexer::new(cmd, dialect).run()
}

struct Lexer<'a> {
    src: &'a str,
    chars: CharIndices<'a>,
    dialect: &'a dyn Dialect,
    tokens: Vec<Token>,
    word: String,
    /// True once the current word has started, so `""` is an empty word and not nothing.
    in_word: bool,
    /// Heredocs whose body starts at the next newline: delimiter and index of their token.
    pending: Vec<(String, usize)>,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str, dialect: &'a dyn Dialect) -> Self {
        Lexer { src, chars: src.char_indices(), dialect, tokens: Vec::new(), word: String::new(), in_word: false, pending: Vec::new() }
    }

    fn run(mut self) -> Vec<Token> {
        while let Some((at, c)) = self.chars.clone().next() {
            self.step(at, c);
        }
        self.flush();
        self.tokens
    }

    fn peek_second(&self) -> Option<char> {
        let mut ahead = self.chars.clone();
        ahead.next();
        ahead.next().map(|(_, c)| c)
    }

    fn skip(&mut self, count: usize) {
        self.chars.by_ref().take(count).for_each(drop);
    }

    fn push(&mut self, c: char) {
        self.word.push(c);
        self.in_word = true;
    }

    fn flush(&mut self) {
        if self.in_word {
            self.tokens.push(Token::Word(std::mem::take(&mut self.word)));
            self.in_word = false;
        }
    }

    fn step(&mut self, at: usize, c: char) {
        let src = self.src;
        let rest = &src[at..];
        match c {
            ' ' | '\t' | '\r' => {
                self.flush();
                self.chars.next();
            }
            '\n' => self.newline(),
            _ if Some(c) == self.dialect.escape() && self.peek_second().is_some() => self.escaped(),
            '\'' if self.dialect.single_quotes() => self.single_quoted(),
            '"' => self.double_quoted(),
            '$' if rest.starts_with("$(") => self.substitution(),
            // `2>` and `*>` redirect like `>`: the descriptor prefix is dropped.
            _ if !self.in_word && (c.is_ascii_digit() || c == '*') && self.peek_second() == Some('>') => {
                self.chars.next();
            }
            _ => match self.dialect.operators().iter().find(|op| rest.starts_with(**op)) {
                Some(op) => self.operator(op, rest),
                None => {
                    self.push(c);
                    self.chars.next();
                }
            },
        }
    }

    /// A newline separates commands and starts the bodies of the heredocs opened on its line.
    fn newline(&mut self) {
        self.flush();
        self.tokens.push(Token::Op(";"));
        self.chars.next();
        for (delim, token) in std::mem::take(&mut self.pending) {
            let rest = self.chars.as_str();
            let (body, consumed) = heredoc_body(rest, &delim);
            let (body, skipped) = (body.to_owned(), rest[..consumed].chars().count());
            self.tokens[token] = Token::Heredoc(Some(body));
            self.skip(skipped);
        }
    }

    fn escaped(&mut self) {
        self.chars.next();
        if let Some((_, next)) = self.chars.next() {
            // An escaped newline continues the line.
            if next != '\n' {
                self.push(next);
            }
        }
    }

    fn single_quoted(&mut self) {
        self.chars.next();
        self.in_word = true;
        let doubled = self.dialect.shell() == Shell::PowerShell;
        while let Some((_, c)) = self.chars.next() {
            if c != '\'' {
                self.word.push(c);
            } else if doubled && self.chars.clone().next().is_some_and(|(_, next)| next == '\'') {
                self.chars.next();
                self.word.push('\'');
            } else {
                break;
            }
        }
    }

    fn double_quoted(&mut self) {
        self.chars.next();
        self.in_word = true;
        while let Some((_, c)) = self.chars.next() {
            if c == '"' {
                break;
            }
            let escapes = Some(c) == self.dialect.escape()
                && self.chars.clone().next().is_some_and(|(_, next)| self.dialect.escapes_in_double(next));
            if escapes {
                self.word.extend(self.chars.next().map(|(_, next)| next));
            } else {
                self.word.push(c);
            }
        }
    }

    /// `$(...)` stays inside its word, nested parentheses included.
    fn substitution(&mut self) {
        let mut depth = 0i32;
        while let Some((_, c)) = self.chars.next() {
            self.push(c);
            depth += i32::from(c == '(') - i32::from(c == ')');
            if depth == 0 && c == ')' {
                break;
            }
        }
    }

    fn operator(&mut self, op: &'static str, rest: &str) {
        self.flush();
        self.skip(op.chars().count());
        if matches!(op, "<<" | "<<-") {
            self.heredoc_start(&rest[op.len()..]);
        } else {
            self.tokens.push(Token::Op(op));
        }
    }

    /// `after` follows the `<<`: the delimiter is read now, the body at the next newline.
    fn heredoc_start(&mut self, after: &str) {
        let token = self.tokens.len();
        self.tokens.push(Token::Heredoc(None));
        if let Some((delim, consumed)) = heredoc_delimiter(after) {
            self.pending.push((delim.to_owned(), token));
            self.skip(after[..consumed].chars().count());
        }
    }
}

/// `\s*(['"]?)([\w.-]+)\1` at the start of `text`: the delimiter and the bytes the match spans.
fn heredoc_delimiter(text: &str) -> Option<(&str, usize)> {
    let trimmed = text.trim_start();
    let lead = text.len() - trimmed.len();
    let quote = trimmed.chars().next().filter(|c| matches!(c, '\'' | '"'));
    let start = quote.map_or(0, char::len_utf8);
    let is_name = |c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '-');
    let len = trimmed[start..].find(|c: char| !is_name(c)).unwrap_or(trimmed.len() - start);
    if len == 0 {
        return None;
    }
    let name_end = start + len;
    let end = match quote {
        Some(q) if trimmed[name_end..].starts_with(q) => name_end + q.len_utf8(),
        Some(_) => return None,
        None => name_end,
    };
    Some((&trimmed[start..name_end], lead + end))
}

/// The body up to the line `^\t*<delim>[ \t\r]*$`, and the bytes consumed through that line
/// without its newline; with no closing line the body is the rest of the text.
fn heredoc_body<'t>(text: &'t str, delim: &str) -> (&'t str, usize) {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let closes = content
            .trim_start_matches('\t')
            .strip_prefix(delim)
            .is_some_and(|tail| tail.chars().all(|c| matches!(c, ' ' | '\t' | '\r')));
        if closes {
            return (&text[..offset], offset + content.len());
        }
        offset += line.len();
    }
    (text, text.len())
}
