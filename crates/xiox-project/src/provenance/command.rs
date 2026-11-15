//! The structure of a command: simple commands split at separators, with their redirect targets
//! taken out of the words, literal assignments, and Bash `for` loops with their bodies.

use super::dialect::{Dialect, Shell};
use super::lex::Token;

/// One argument of a simple command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    Text(String),
    /// The PowerShell `,` operator, kept as the separator of array parameters.
    Comma,
}

impl Arg {
    fn text(&self) -> Option<&str> {
        match self {
            Arg::Text(text) => Some(text),
            Arg::Comma => None,
        }
    }
}

/// A command with its arguments, the targets it redirects output to and its heredoc body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Simple {
    pub words: Vec<Arg>,
    pub redirects: Vec<String>,
    pub heredoc: Option<String>,
}

/// A statement of the command, in the order it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Simple(Simple),
    /// A segment that only assigns a literal; `name` is lowercase outside Bash.
    Assign { name: String, value: String },
    /// A Bash `for`; `items` is `None` without an `in` list (including the arithmetic form).
    For { var: String, items: Option<Vec<String>>, body: Vec<Node>, done_redirects: Vec<String> },
}

/// The top-level nodes in order; the last one is the one that reaches the end of the text.
pub fn parse(tokens: Vec<Token>, dialect: &dyn Dialect) -> Vec<Node> {
    build(&segments(tokens), dialect.shell())
}

const SEPARATORS: [&str; 10] = ["&&", "||", "|&", ";", "|", "&", "(", ")", "{", "}"];
const REDIRECT_OUT: [&str; 3] = [">", ">>", "&>"];
/// Words that start a command without being it: wrappers and the keywords of compound commands.
const PREFIXES: [&str; 18] = [
    "sudo", "env", "time", "command", "exec", "nohup", "do", "then", "else", "elif", "if", "while", "until", "&", "!",
    "{", "call", "start",
];
/// Keywords that open a loop closed by `done`.
const LOOP_HEADS: [&str; 4] = ["for", "while", "until", "select"];

/// The raw simple commands between separators; empty ones are dropped.
fn segments(tokens: Vec<Token>) -> Vec<Simple> {
    let mut out = Vec::new();
    let mut current = Simple::default();
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            Token::Op(op) if SEPARATORS.contains(&op) => out.push(std::mem::take(&mut current)),
            Token::Op(op) if REDIRECT_OUT.contains(&op) => {
                if let Some(Token::Word(target)) = tokens.next_if(|next| matches!(next, Token::Word(_))) {
                    current.redirects.push(target);
                }
            }
            // Input and descriptor redirects write nothing: they only consume their word.
            Token::Op("<" | ">&") => {
                tokens.next_if(|next| matches!(next, Token::Word(_)));
            }
            Token::Op(_) => current.words.push(Arg::Comma),
            Token::Heredoc(body) => current.heredoc = body,
            Token::Word(text) => current.words.push(Arg::Text(text)),
        }
    }
    out.push(current);
    out.retain(|segment| !segment.words.is_empty() || !segment.redirects.is_empty());
    out
}

fn build(segments: &[Simple], shell: Shell) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut at = 0;
    while at < segments.len() {
        let segment = &segments[at];
        if shell == Shell::Bash {
            if let Some((var, items)) = for_header(segment) {
                let done = matching_done(segments, at);
                let end = done.unwrap_or(segments.len());
                let done_redirects = done.map(|done| segments[done].redirects.clone()).unwrap_or_default();
                let body = build(&segments[at + 1..end], shell);
                nodes.push(Node::For { var, items, body, done_redirects });
                at = end + 1;
                continue;
            }
        }
        nodes.extend(node(segment, shell));
        at += 1;
    }
    nodes
}

fn node(segment: &Simple, shell: Shell) -> Option<Node> {
    if let Some((name, value)) = assignment(&segment.words, shell) {
        return Some(Node::Assign { name, value });
    }
    let skipped = segment.words.iter().take_while(|word| is_prefix(word)).count();
    let words = segment.words[skipped..].to_vec();
    if words.is_empty() && segment.redirects.is_empty() {
        return None;
    }
    Some(Node::Simple(Simple { words, redirects: segment.redirects.clone(), heredoc: segment.heredoc.clone() }))
}

fn is_prefix(word: &Arg) -> bool {
    word.text().is_some_and(|text| PREFIXES.contains(&text.to_lowercase().as_str()) || is_bash_assignment(text))
}

/// `NAME=` at the start of a word.
fn is_bash_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| is_name(name))
}

fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// `(name, value)` of a segment that only assigns a literal.
fn assignment(words: &[Arg], shell: Shell) -> Option<(String, String)> {
    let texts: Vec<&str> = words.iter().filter_map(Arg::text).collect();
    let powershell_var = |word: &str| word.strip_prefix('$').filter(|name| is_word(name)).map(str::to_lowercase);
    match (shell, texts.as_slice()) {
        (Shell::Bash, [word]) if is_bash_assignment(word) => split_assignment(word, str::to_owned),
        (Shell::PowerShell, [var, "=", value]) => Some((powershell_var(var)?, (*value).to_owned())),
        (Shell::PowerShell, [word]) => {
            let (var, value) = word.split_once('=').filter(|(_, value)| !value.is_empty())?;
            Some((powershell_var(var)?, value.to_owned()))
        }
        (Shell::Cmd, [set, word]) if set.eq_ignore_ascii_case("set") && word.contains('=') => {
            split_assignment(word, str::to_lowercase)
        }
        _ => None,
    }
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn split_assignment(word: &str, name: fn(&str) -> String) -> Option<(String, String)> {
    let (left, value) = word.split_once('=')?;
    Some((name(left), value.to_owned()))
}

/// The words of a statement after the keywords that may precede its head.
fn heads(segment: &Simple) -> Vec<&str> {
    let texts = segment.words.iter().filter_map(Arg::text);
    texts.skip_while(|text| matches!(*text, "do" | "then" | "else" | "{")).collect()
}

/// The variable and the `in` list of a `for` header segment.
fn for_header(segment: &Simple) -> Option<(String, Option<Vec<String>>)> {
    let head = heads(segment);
    if head.first() != Some(&"for") {
        return None;
    }
    let items = (head.get(2) == Some(&"in")).then(|| head[3..].iter().map(|item| (*item).to_owned()).collect());
    let var = head.get(1).map(|var| (*var).to_owned()).unwrap_or_default();
    Some((var, items))
}

/// The index of the `done` that closes the loop opened at `open`, counting nested loops.
fn matching_done(segments: &[Simple], open: usize) -> Option<usize> {
    let mut depth = 1;
    for (at, segment) in segments.iter().enumerate().skip(open + 1) {
        match heads(segment).first() {
            Some(head) if LOOP_HEADS.contains(head) => depth += 1,
            Some(&"done") => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return Some(at);
        }
    }
    None
}
