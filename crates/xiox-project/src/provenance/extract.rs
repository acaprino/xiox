//! The extraction pipeline: lex, parse, then walk the statements with the variables known so far,
//! handing each simple command to the recognizers.
//!
//! Variables are substituted only when the script assigned them a literal; anything else stays
//! unresolved and is counted. Loops over literal lists are expanded, within the shared [`Budget`].

use std::collections::HashMap;

use super::command::{Arg, Node, Simple, parse};
use super::paths::is_unknown;
use super::op::{Completeness, Extraction, OpKind, RawOp};
use super::recognize::{Context, Invocation, Registry};
use super::{Budget, Shell, lex};

/// Variable values assigned literally so far, by name.
type Known = HashMap<String, String>;

/// Reads the operations of `cmd`, written for `shell`.
pub fn extract(cmd: &str, shell: Shell, completeness: Completeness) -> Extraction {
    let mut ctx = Context::default();
    let mut nodes = read_nodes(cmd, shell);
    // D3: a truncated text may end inside a word, a quote or a heredoc, so what remains of its last
    // command says nothing certain. The prototype read it as if whole and could shorten a destination.
    if completeness == Completeness::Truncated && nodes.pop().is_some() {
        ctx.flags.truncated = true;
        ctx.unresolved += 1;
    }
    let mut ops = Vec::new();
    walk(&nodes, shell, &mut Known::new(), &mut ctx, &mut ops);
    Extraction { ops, flags: ctx.flags, unresolved: ctx.unresolved }
}

/// The operations of the command line a nested shell runs, as a scope of its own: variables do not
/// cross the boundary. `None` when the nesting is too deep to read.
pub(super) fn extract_nested(script: &str, shell: Shell, ctx: &mut Context) -> Option<Vec<RawOp>> {
    if !ctx.budget.enter() {
        skip(ctx);
        return None;
    }
    let mut ops = Vec::new();
    walk(&read_nodes(script, shell), shell, &mut Known::new(), ctx, &mut ops);
    ctx.budget.leave();
    Some(ops)
}

fn read_nodes(cmd: &str, shell: Shell) -> Vec<Node> {
    let dialect = shell.dialect();
    parse(lex(cmd, dialect), dialect)
}

/// D4: a statement the budget could not pay for is not read; it is counted, one for each.
fn skip(ctx: &mut Context) {
    ctx.flags.exhausted = true;
    ctx.unresolved += 1;
}

fn walk(nodes: &[Node], shell: Shell, known: &mut Known, ctx: &mut Context, ops: &mut Vec<RawOp>) {
    for node in nodes {
        if !ctx.budget.take() {
            skip(ctx);
            continue;
        }
        match node {
            Node::Assign { name, value } => assign(name, value, shell, known),
            Node::For { var, items, body, done_redirects } => {
                if for_loop(var, items.as_deref(), body, shell, known, ctx, ops) {
                    ops.extend(done_redirects.iter().map(|target| write(substitute(target, shell, known))));
                }
            }
            Node::Simple(simple) => simple_command(simple, shell, known, ctx, ops),
        }
    }
}

/// A literal value is remembered; any other value makes the name unknown again, so a later use
/// stays unresolved instead of reading the stale literal.
fn assign(name: &str, value: &str, shell: Shell, known: &mut Known) {
    let value = substitute(value, shell, known);
    if is_unknown(&value) {
        known.remove(name);
    } else {
        known.insert(name.to_owned(), value);
    }
}

/// Walks a loop body once per literal item, or once if the items are not literal. False when the
/// budget refused the loop.
fn for_loop(
    var: &str,
    items: Option<&[String]>,
    body: &[Node],
    shell: Shell,
    known: &Known,
    ctx: &mut Context,
    ops: &mut Vec<RawOp>,
) -> bool {
    let items: Vec<String> = items.unwrap_or_default().iter().map(|item| substitute(item, shell, known)).collect();
    if !is_literal_list(&items) {
        ctx.flags.looped = true;
        walk(body, shell, &mut known.clone(), ctx, ops);
        return true;
    }
    if !ctx.budget.enter() {
        skip(ctx);
        return false;
    }
    for item in items {
        let mut scope = known.clone();
        scope.insert(var.to_owned(), item);
        walk(body, shell, &mut scope, ctx, ops);
    }
    ctx.budget.leave();
    true
}

fn is_literal_list(items: &[String]) -> bool {
    const NOT_LITERAL: [char; 10] = ['$', '`', '*', '?', '[', ']', '{', '}', '(', ')'];
    !items.is_empty()
        && items.len() <= Budget::MAX_LOOP_ITEMS
        && items.iter().all(|item| !item.is_empty() && !item.contains(NOT_LITERAL))
}

fn simple_command(simple: &Simple, shell: Shell, known: &Known, ctx: &mut Context, ops: &mut Vec<RawOp>) {
    let words: Vec<Arg> = simple.words.iter().map(|word| substitute_arg(word, shell, known)).collect();
    let redirects: Vec<String> = simple.redirects.iter().map(|target| substitute(target, shell, known)).collect();
    if let Some(Arg::Text(head)) = words.first() {
        remember_script(head, &words[1..], &redirects, simple.heredoc.as_deref(), ctx);
        let invocation = Invocation { name: command_name(head), args: &words[1..], shell, heredoc: simple.heredoc.as_deref() };
        ops.extend(Registry::standard().recognize(&invocation, ctx));
    }
    ops.extend(redirects.into_iter().map(write));
}

/// `cat > x.py <<EOF` and `tee x.py <<EOF` write a script that a later command may run.
fn remember_script(head: &str, args: &[Arg], redirects: &[String], heredoc: Option<&str>, ctx: &mut Context) {
    let Some(body) = heredoc.filter(|_| matches!(head, "cat" | "tee")) else { return };
    let from_args = || args.iter().filter_map(Arg::text).map(str::to_owned);
    let targets: Vec<String> = if redirects.is_empty() { from_args().collect() } else { redirects.to_vec() };
    for target in targets {
        ctx.scripts.insert(target, body.to_owned());
    }
}

/// The lowercase file name of the first word, without `.exe`.
fn command_name(word: &str) -> String {
    let name = word.rsplit(['\\', '/']).next().unwrap_or(word).to_lowercase();
    name.strip_suffix(".exe").map(str::to_owned).unwrap_or(name)
}

fn write(dst: String) -> RawOp {
    RawOp::Path { kind: OpKind::Write, src: None, dst: Some(dst), base: None, script: false }
}

fn substitute_arg(arg: &Arg, shell: Shell, known: &Known) -> Arg {
    match arg {
        Arg::Text(text) => Arg::Text(substitute(text, shell, known)),
        Arg::Comma => Arg::Comma,
    }
}

/// Replaces the references to known variables: `$x` and `${x}` in Bash, `$x` without regard to case in
/// PowerShell, `%x%` in Cmd. Unknown references stay as written and resolve to nothing later.
fn substitute(text: &str, shell: Shell, known: &Known) -> String {
    if known.is_empty() {
        return text.to_owned();
    }
    let marker = if shell == Shell::Cmd { '%' } else { '$' };
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(marker) {
        out.push_str(&rest[..at]);
        let candidate = &rest[at..];
        match reference(candidate, shell) {
            Some((name, len)) => {
                let value = match shell {
                    Shell::Bash => known.get(name),
                    _ => known.get(&name.to_lowercase()),
                };
                out.push_str(value.map_or(&candidate[..len], String::as_str));
                rest = &candidate[len..];
            }
            None => {
                out.push(marker);
                rest = &candidate[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The variable name at the start of `text` (which starts with the marker) and the length of the
/// whole reference.
fn reference(text: &str, shell: Shell) -> Option<(&str, usize)> {
    let body = &text[1..];
    match shell {
        Shell::Bash => braced(body).or_else(|| {
            let len = word_len(body);
            (len > 0).then(|| (&body[..len], 1 + len))
        }),
        Shell::PowerShell => {
            let open = body.starts_with('{');
            let name = if open { &body[1..] } else { body };
            if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                return None;
            }
            let len = word_len(name);
            let close = name[len..].starts_with('}');
            Some((&name[..len], 1 + usize::from(open) + len + usize::from(close)))
        }
        Shell::Cmd => {
            let len = word_len(body);
            (len > 0 && body[len..].starts_with('%')).then(|| (&body[..len], 2 + len))
        }
    }
}

/// `{name}` at the start of `body`, as `${name}`.
fn braced(body: &str) -> Option<(&str, usize)> {
    let inner = body.strip_prefix('{')?;
    let len = word_len(inner);
    (len > 0 && inner[len..].starts_with('}')).then(|| (&inner[..len], 3 + len))
}

/// The byte length of the run of word characters at the start of `text`.
fn word_len(text: &str) -> usize {
    text.char_indices().find(|(_, c)| !(c.is_alphanumeric() || *c == '_')).map_or(text.len(), |(at, _)| at)
}
