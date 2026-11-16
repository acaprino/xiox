//! Bash brace expansion of one word: `a{b,c}d`, `{a,{b,c}}`, `{1..3}`, `{a..e..2}`, `{01..10}`.
//!
//! The lexer knows which braces and commas are live (not quoted, not escaped, not part of `${...}`);
//! this module only builds the words. The work is bounded: a word that would give more than
//! [`MAX_WORDS`] words is not expanded.

use super::budget::Budget;

/// Words one word may expand to, the same bound as a loop's items.
pub const MAX_WORDS: usize = Budget::MAX_LOOP_ITEMS;

/// A live `{`, `}` or `,` at a byte offset of the word.
pub type Mark = (usize, char);

/// A matched `{...}`: byte offsets of its braces and of the commas at its own level.
struct Group {
    open: usize,
    close: usize,
    commas: Vec<usize>,
}

/// The words `text` expands to, or `None` when it would give more than [`MAX_WORDS`].
/// A word with no expandable brace expression expands to itself.
pub fn expand(text: &str, marks: &[Mark]) -> Option<Vec<String>> {
    let mut groups = groups(marks);
    groups.sort_by_key(|group| group.open);
    expand_range(text, &groups, 0, text.len())
}

/// Pairs the live braces in one pass; an unmatched brace is a literal character.
fn groups(marks: &[Mark]) -> Vec<Group> {
    let mut open: Vec<Group> = Vec::new();
    let mut done = Vec::new();
    for &(at, c) in marks {
        match c {
            '{' => open.push(Group { open: at, close: at, commas: Vec::new() }),
            ',' => {
                if let Some(top) = open.last_mut() {
                    top.commas.push(at);
                }
            }
            _ => {
                if let Some(mut group) = open.pop() {
                    group.close = at;
                    done.push(group);
                }
            }
        }
    }
    done
}

/// What a group stands for.
enum Items {
    /// The text between the commas, expanded in turn.
    List(Vec<(usize, usize)>),
    Sequence(Vec<String>),
}

/// `None` for a group that is neither a list nor a sequence, which stays literal; the outer `Option`
/// of the result is the word limit.
fn items(text: &str, group: &Group) -> Option<Option<Items>> {
    if !group.commas.is_empty() {
        let mut edges = vec![group.open];
        edges.extend(&group.commas);
        edges.push(group.close);
        return Some(Some(Items::List(edges.windows(2).map(|pair| (pair[0] + 1, pair[1])).collect())));
    }
    match sequence(&text[group.open + 1..group.close]) {
        Sequence::Words(words) => Some(Some(Items::Sequence(words))),
        Sequence::TooLong => None,
        Sequence::NotOne => Some(None),
    }
}

fn expand_range(text: &str, groups: &[Group], lo: usize, hi: usize) -> Option<Vec<String>> {
    let mut from = 0;
    while let Some(index) = groups[from..].iter().position(|g| g.open >= lo && g.close < hi).map(|i| i + from) {
        let group = &groups[index];
        let Some(items) = items(text, group)? else {
            // A literal pair of braces: what is inside may still expand.
            from = index + 1;
            continue;
        };
        let words = match items {
            Items::Sequence(words) => words,
            Items::List(ranges) => {
                let nested = &groups[index + 1..];
                let mut words = Vec::new();
                for (start, end) in ranges {
                    words.extend(expand_range(text, nested, start, end)?);
                    if words.len() > MAX_WORDS {
                        return None;
                    }
                }
                words
            }
        };
        let rest = expand_range(text, groups, group.close + 1, hi)?;
        if words.len() * rest.len() > MAX_WORDS {
            return None;
        }
        let prefix = &text[lo..group.open];
        return Some(words.iter().flat_map(|word| rest.iter().map(move |tail| format!("{prefix}{word}{tail}"))).collect());
    }
    Some(vec![text[lo..hi].to_owned()])
}

enum Sequence {
    Words(Vec<String>),
    /// A sequence with more than [`MAX_WORDS`] words.
    TooLong,
    /// Not a sequence: `{a}`, `{}`, `{1..x}`.
    NotOne,
}

/// `a..b` or `a..b..step` over integers (zero padding kept) or over single letters.
fn sequence(body: &str) -> Sequence {
    let parts: Vec<&str> = body.split("..").collect();
    let (from, to, step) = match parts[..] {
        [from, to] => (from, to, None),
        [from, to, step] => (from, to, Some(step)),
        _ => return Sequence::NotOne,
    };
    let step = match step.map(str::parse::<i64>) {
        None => 1u64,
        Some(Ok(step)) if step != 0 => step.unsigned_abs(),
        Some(_) => return Sequence::NotOne,
    };
    if let (Ok(a), Ok(b)) = (from.parse::<i64>(), to.parse::<i64>()) {
        let width = [from, to].iter().filter(|s| padded(s)).map(|s| s.trim_start_matches('-').len()).max().unwrap_or(0);
        return run(i128::from(a), i128::from(b), step, |n| {
            let digits = format!("{:0>width$}", n.unsigned_abs());
            if n < 0 { format!("-{digits}") } else { digits }
        });
    }
    match (single_letter(from), single_letter(to)) {
        (Some(a), Some(b)) => run(a as i128, b as i128, step, |n| char::from_u32(n as u32).unwrap_or('?').to_string()),
        _ => Sequence::NotOne,
    }
}

/// A number written with a leading zero, as `01` or `-007`.
fn padded(number: &str) -> bool {
    let digits = number.trim_start_matches('-');
    digits.len() > 1 && digits.starts_with('0')
}

fn single_letter(text: &str) -> Option<char> {
    let mut chars = text.chars();
    chars.next().filter(char::is_ascii_alphabetic).filter(|_| chars.next().is_none())
}

/// The values from `a` to `b` (either way round) every `step`, rendered by `show`.
fn run(a: i128, b: i128, step: u64, show: impl Fn(i128) -> String) -> Sequence {
    let count = a.abs_diff(b) / u128::from(step) + 1;
    if count > MAX_WORDS as u128 {
        return Sequence::TooLong;
    }
    let direction = if b >= a { 1 } else { -1 };
    Sequence::Words((0..count as i128).map(|i| show(a + direction * i128::from(step) * i)).collect())
}
