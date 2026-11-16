//! Calls with a literal path argument inside Python and JavaScript source passed to an interpreter.
//!
//! One pass over the text with a stack of open calls: a literal or a nested bracket advances the
//! frame on top, arguments end at the commas of depth zero, and a frame is judged when its `)`
//! arrives. Text is never read twice, so adversarial input costs time linear in its length.
//! Only a whole literal counts: an f-string, a `${}` template, a format string, a concatenation or
//! a variable gives nothing, and a call that wrote to a path we cannot read adds to `unresolved`.

use std::ops::Range;

use super::op::{OpKind, RawOp};

/// Source languages the scanner knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Python,
    JavaScript,
}

/// What a script does to disk, as far as literal arguments tell.
#[derive(Debug, Default)]
pub struct ScriptScan {
    /// Operations with `script: true`, in the order the calls appear.
    pub ops: Vec<RawOp>,
    /// Table calls whose path argument is not a whole literal.
    pub unresolved: usize,
}

/// A call that performs a disk operation when its path argument is a literal.
struct Call {
    /// Function names; matched at an identifier boundary and followed by `(`.
    callee: &'static [&'static str],
    /// Index of the path argument.
    arg: usize,
    kind: OpKind,
    /// Text that must follow the closing `)`, as in `Path("x").mkdir(`.
    then: Option<&'static [&'static str]>,
    /// Index of a mode literal that must write (`open(path, "w")`).
    mode: Option<usize>,
}

const PYTHON_CALLS: &[Call] = &[
    Call { callee: &["open"], arg: 0, kind: OpKind::Write, then: None, mode: Some(1) },
    Call { callee: &["os.makedirs", "os.mkdir"], arg: 0, kind: OpKind::Create, then: None, mode: None },
    Call { callee: &["Path"], arg: 0, kind: OpKind::Create, then: Some(&[".mkdir("]), mode: None },
    Call {
        callee: &["Path"],
        arg: 0,
        kind: OpKind::Write,
        then: Some(&[".write_text(", ".write_bytes(", ".touch("]),
        mode: None,
    },
    Call {
        callee: &["shutil.copytree", "shutil.copy2", "shutil.copy", "shutil.copyfile"],
        arg: 1,
        kind: OpKind::Copy,
        then: None,
        mode: None,
    },
    Call { callee: &["shutil.move"], arg: 1, kind: OpKind::Move, then: None, mode: None },
];

/// Any receiver: `fs.`, `fsp.`, `require('fs').` and a destructured name all end in the method name.
const JAVASCRIPT_CALLS: &[Call] = &[
    Call {
        callee: &["writeFileSync", "writeFile", "appendFileSync", "createWriteStream"],
        arg: 0,
        kind: OpKind::Write,
        then: None,
        mode: None,
    },
    Call { callee: &["mkdirSync", "mkdir"], arg: 0, kind: OpKind::Create, then: None, mode: None },
    Call { callee: &["copyFileSync", "copyFile", "cpSync"], arg: 1, kind: OpKind::Copy, then: None, mode: None },
    Call { callee: &["renameSync"], arg: 1, kind: OpKind::Move, then: None, mode: None },
];

/// Open calls tracked at once; deeper openings are read as plain brackets.
const MAX_FRAMES: usize = 64;
/// Arguments kept per call: the table never looks past the second.
const KEPT_ARGS: usize = 2;
/// Characters that make a mode literal write.
const WRITE_MODES: [char; 3] = ['w', 'a', 'x'];

/// Scans `code` for table calls with a literal path argument.
pub fn scan_script(code: &str, lang: Lang) -> ScriptScan {
    let table = match lang {
        Lang::Python => PYTHON_CALLS,
        Lang::JavaScript => JAVASCRIPT_CALLS,
    };
    Scanner { code, lang, table, stack: Vec::new(), found: Vec::new(), unresolved: 0 }.run()
}

/// The text of a string literal, between its quotes.
#[derive(Clone, Debug)]
struct Body {
    range: Range<usize>,
    raw: bool,
}

/// What one argument looks like so far.
#[derive(Clone, Debug)]
enum Shape {
    Blank,
    /// The word `mode`, which may turn out to be a keyword argument.
    Keyword,
    Literal(Body),
    Other,
}

#[derive(Clone, Debug)]
struct Arg {
    shape: Shape,
    /// Written as `mode=<literal>`.
    keyworded: bool,
}

impl Arg {
    const BLANK: Arg = Arg { shape: Shape::Blank, keyworded: false };

    /// Anything that is not a lone literal: after this the argument can never be one.
    fn touch(&mut self) {
        self.shape = Shape::Other;
    }

    fn literal(&mut self, body: Option<Body>) {
        self.shape = match (&self.shape, body) {
            (Shape::Blank, Some(body)) => Shape::Literal(body),
            _ => Shape::Other,
        };
    }

    fn word(&mut self, word: &str) {
        self.shape = match (&self.shape, word) {
            (Shape::Blank, "mode") if !self.keyworded => Shape::Keyword,
            _ => Shape::Other,
        };
    }

    fn equals(&mut self) {
        match self.shape {
            Shape::Keyword => {
                self.shape = Shape::Blank;
                self.keyworded = true;
            }
            _ => self.touch(),
        }
    }
}

/// A call whose `)` has not arrived yet.
struct Frame {
    /// Where the callee starts, to order operations by appearance.
    start: usize,
    /// The callee as the table spells it.
    name: &'static str,
    /// Open brackets inside the argument being read.
    depth: usize,
    current: Arg,
    /// The first `KEPT_ARGS` arguments.
    args: Vec<Arg>,
    /// Arguments seen, kept or not.
    count: usize,
}

impl Frame {
    fn new(start: usize, name: &'static str) -> Frame {
        Frame { start, name, depth: 0, current: Arg::BLANK, args: Vec::new(), count: 0 }
    }

    fn end_arg(&mut self) {
        let arg = std::mem::replace(&mut self.current, Arg::BLANK);
        if self.args.len() < KEPT_ARGS {
            self.args.push(arg);
        }
        self.count += 1;
    }

    /// The argument at `index`, unless it is missing or empty.
    fn arg(&self, index: usize) -> Option<&Arg> {
        self.args.get(index).filter(|arg| !matches!(arg.shape, Shape::Blank))
    }
}

struct Scanner<'a> {
    code: &'a str,
    lang: Lang,
    table: &'static [Call],
    stack: Vec<Frame>,
    found: Vec<(usize, RawOp)>,
    unresolved: usize,
}

impl Scanner<'_> {
    fn run(mut self) -> ScriptScan {
        let mut i = 0;
        while let Some(c) = self.code[i..].chars().next() {
            i = match c {
                '\'' | '"' => self.string(i, ""),
                '`' if self.lang == Lang::JavaScript => self.string(i, ""),
                '(' | '[' | '{' => self.open_bracket(i + 1),
                ')' | ']' | '}' => self.close_bracket(i + 1),
                ',' => self.comma(i + 1),
                '#' if self.lang == Lang::Python => self.line_comment(i),
                '/' if self.lang == Lang::JavaScript && self.code[i..].starts_with("//") => self.line_comment(i),
                '/' if self.lang == Lang::JavaScript && self.code[i..].starts_with("/*") => self.block_comment(i),
                '=' => self.equals(i + 1),
                c if c.is_whitespace() => i + c.len_utf8(),
                c if is_word(c) => self.word(i),
                c => self.other(i + c.len_utf8()),
            };
        }
        self.found.sort_by_key(|(start, _)| *start);
        ScriptScan { ops: self.found.into_iter().map(|(_, op)| op).collect(), unresolved: self.unresolved }
    }

    fn current(&mut self) -> Option<&mut Arg> {
        self.stack.last_mut().map(|frame| &mut frame.current)
    }

    /// A comment is blank space: it neither changes an argument nor hides the code after it.
    fn line_comment(&self, start: usize) -> usize {
        self.code[start..].find('\n').map_or(self.code.len(), |offset| start + offset)
    }

    fn block_comment(&self, start: usize) -> usize {
        self.code[start + 2..].find("*/").map_or(self.code.len(), |offset| start + 2 + offset + 2)
    }

    fn other(&mut self, next: usize) -> usize {
        if let Some(arg) = self.current() {
            arg.touch();
        }
        next
    }

    fn equals(&mut self, next: usize) -> usize {
        if let Some(arg) = self.current() {
            arg.equals();
        }
        next
    }

    fn comma(&mut self, next: usize) -> usize {
        match self.stack.last_mut() {
            Some(frame) if frame.depth == 0 => frame.end_arg(),
            _ => {}
        }
        next
    }

    fn open_bracket(&mut self, next: usize) -> usize {
        if let Some(frame) = self.stack.last_mut() {
            frame.depth += 1;
            frame.current.touch();
        }
        next
    }

    fn close_bracket(&mut self, next: usize) -> usize {
        match self.stack.last_mut() {
            None => {}
            Some(frame) if frame.depth > 0 => frame.depth -= 1,
            Some(_) => {
                if let Some(mut frame) = self.stack.pop() {
                    frame.end_arg();
                    self.evaluate(&frame, next);
                }
                // The finished call is part of the argument it sits in.
                self.other(next);
            }
        }
        next
    }

    /// A word starting at `start`: a string prefix, a call, or plain text.
    fn word(&mut self, start: usize) -> usize {
        let rest = &self.code[start..];
        let len = rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
        let end = start + len;
        let word = &rest[..len];
        if self.lang == Lang::Python && is_string_prefix(word) && self.code[end..].starts_with(['\'', '"']) {
            return self.string(end, word);
        }
        if let Some(after_paren) = self.open_call(start) {
            return after_paren;
        }
        if let Some(arg) = self.current() {
            arg.word(word);
        }
        end
    }

    /// Opens a frame when a table callee starts at `start` and `(` follows; returns the index after it.
    fn open_call(&mut self, start: usize) -> Option<usize> {
        if self.stack.len() >= MAX_FRAMES {
            return None;
        }
        let rest = &self.code[start..];
        let name = self.table.iter().flat_map(|call| call.callee).copied().find(|name| {
            rest.strip_prefix(name).is_some_and(|after| after.trim_start().starts_with('('))
        })?;
        if let Some(arg) = self.current() {
            arg.touch();
        }
        self.stack.push(Frame::new(start, name));
        let after_name = &rest[name.len()..];
        let paren = name.len() + after_name.len() - after_name.trim_start().len();
        Some(start + paren + 1)
    }

    /// A string literal whose quote is at `quote`, after an optional `prefix`;
    /// returns the index after its closing quote.
    fn string(&mut self, quote: usize, prefix: &str) -> usize {
        let text = &self.code[quote..];
        let delimiter = text.chars().next().unwrap_or('"');
        let triple = self.lang == Lang::Python && text.starts_with(&delimiter.to_string().repeat(3));
        let open = if triple { 3 } else { 1 };
        let mut escaped = false;
        let mut run = 0;
        let mut body_end = None;
        for (offset, c) in text.char_indices().skip(open) {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == delimiter {
                run += 1;
                if run == open {
                    body_end = Some(offset + c.len_utf8() - open);
                    break;
                }
                continue;
            }
            run = 0;
        }
        let Some(body_end) = body_end else {
            // Unterminated: the rest of the text is inside the literal.
            self.other(self.code.len());
            return self.code.len();
        };
        let body = quote + open..quote + body_end;
        let end = quote + body_end + open;
        let whole = !triple
            && matches!(prefix, "" | "r" | "R" | "u" | "U")
            && !self.code[body.clone()].contains(['{', '}', '\n']);
        let raw = matches!(prefix, "r" | "R");
        if let Some(arg) = self.current() {
            arg.literal(whole.then_some(Body { range: body, raw }));
        }
        end
    }

    /// Judges a finished call: its operation, or one more unresolved path.
    fn evaluate(&mut self, frame: &Frame, after: usize) {
        for call in self.table.iter().filter(|call| call.callee.contains(&frame.name)) {
            let follows = |then: &&str| self.code[after..].starts_with(*then);
            if !call.then.is_none_or(|then| then.iter().any(follows)) {
                continue;
            }
            if call.mode.is_some_and(|index| !self.writes(frame, index)) {
                continue;
            }
            let Some(path) = frame.arg(call.arg) else { continue };
            match self.value(path, false).filter(|path| !path.is_empty()) {
                Some(dst) => {
                    let op = RawOp::Path { kind: call.kind, src: None, dst: Some(dst), base: None, script: true };
                    self.found.push((frame.start, op));
                }
                None if matches!(&path.shape, Shape::Literal(body) if body.range.is_empty()) => {}
                None => self.unresolved += 1,
            }
        }
    }

    /// Whether the mode argument is a literal that writes.
    fn writes(&self, frame: &Frame, index: usize) -> bool {
        frame
            .arg(index)
            .and_then(|mode| self.value(mode, true))
            .is_some_and(|mode| mode.contains(WRITE_MODES))
    }

    /// The text of an argument that is a lone literal; `keyword` accepts the `mode=` form.
    fn value(&self, arg: &Arg, keyword: bool) -> Option<String> {
        let Shape::Literal(body) = &arg.shape else { return None };
        if arg.keyworded && !keyword {
            return None;
        }
        let text = &self.code[body.range.clone()];
        Some(if body.raw { text.to_string() } else { text.replace("\\\\", "\\") })
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Prefixes Python allows before a quote; only `r` and `u` still leave a plain literal.
fn is_string_prefix(word: &str) -> bool {
    word.len() <= 2 && word.chars().all(|c| "rRuUbBfF".contains(c))
}
