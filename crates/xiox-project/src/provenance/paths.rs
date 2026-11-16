//! Resolution of the paths a command wrote down into absolute Windows paths.
//!
//! Pure string work: components are normalised by hand and the disk is never touched.

use super::op::{Candidate, OpKind, RawOp};

/// The user directories a command may name through `~` and `/tmp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Env {
    pub home: String,
    pub temp: String,
}

/// Why a path did not become an absolute path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    /// The text depends on a variable, a substitution or a glob.
    Unknown,
    /// A relative path with no known working directory.
    NoCwd,
    /// A form with no single answer: another MSYS root, a drive-relative or root-relative path.
    Unsupported,
}

/// What a list of operations resolves to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub candidates: Vec<Candidate>,
    /// Every directory a `cd` or `pushd` entered, resolved where it ran; `cd .` and `popd` are not visits.
    pub visits: Vec<String>,
    /// Operations whose paths could not be resolved.
    pub unresolved: usize,
}

const NULL_DEVICES: [&str; 5] = ["/dev/null", "nul", "nul:", "$null", "-"];

pub fn is_null_device(p: &str) -> bool {
    let p = p.trim().to_lowercase();
    NULL_DEVICES.contains(&p.as_str())
}

/// Resolves `p` against `cwd` into an absolute, normalised path without a trailing separator.
pub fn resolve_path(p: &str, cwd: Option<&str>, env: &Env) -> Result<String, PathError> {
    let p = p.trim();
    if is_unknown(p) {
        return Err(PathError::Unknown);
    }
    let p = expand_roots(p, env)?.replace('/', r"\");
    if has_absolute_prefix(&p) {
        return Ok(normalize(&p));
    }
    if is_drive_or_root_relative(&p) {
        return Err(PathError::Unsupported);
    }
    let cwd = cwd.ok_or(PathError::NoCwd)?;
    Ok(normalize(&format!(r"{cwd}\{p}")))
}

/// Resolves a command's operations in order, tracking `cd`, `pushd` and `popd`.
pub fn resolve(ops: &[RawOp], cwd: &str, env: &Env) -> Resolved {
    let mut walk = Walk { cur: Some(cwd.to_owned()), stack: Vec::new(), env, out: Resolved::default() };
    for op in ops {
        match op {
            RawOp::Cd { dst, push } => walk.change_dir(dst.as_deref(), *push),
            RawOp::PopCd => walk.pop_dir(),
            RawOp::Path { kind, src, dst, base, script } => {
                walk.record(*kind, src.as_deref(), dst.as_deref(), base.as_deref(), *script)
            }
        }
    }
    walk.out
}

/// The state `resolve` carries from one operation to the next.
struct Walk<'a> {
    /// `None` after a `cd` to a place the resolver cannot name: relative paths then stay unresolved.
    cur: Option<String>,
    stack: Vec<Option<String>>,
    env: &'a Env,
    out: Resolved,
}

impl Walk<'_> {
    fn change_dir(&mut self, dst: Option<&str>, push: bool) {
        if push {
            self.stack.push(self.cur.clone());
        }
        let Some(dst) = dst else { return };
        match resolve_path(dst, self.cur.as_deref(), self.env) {
            Ok(dir) => {
                if dst != "." {
                    self.out.visits.push(dir.clone());
                }
                self.cur = Some(dir);
            }
            Err(_) => {
                self.cur = None;
                self.out.unresolved += 1;
            }
        }
    }

    fn pop_dir(&mut self) {
        if let Some(previous) = self.stack.pop() {
            self.cur = previous;
        }
    }

    fn record(&mut self, kind: OpKind, src: Option<&str>, dst: Option<&str>, base: Option<&str>, script: bool) {
        if dst.is_some_and(is_null_device) {
            return;
        }
        match self.candidate(kind, src, dst, base, script) {
            Ok(Some(candidate)) => self.out.candidates.push(candidate),
            Ok(None) => {}
            Err(_) => self.out.unresolved += 1,
        }
    }

    /// `Ok(None)`: an operation that names no destination and has none to infer.
    fn candidate(
        &self,
        kind: OpKind,
        src: Option<&str>,
        dst: Option<&str>,
        base: Option<&str>,
        script: bool,
    ) -> Result<Option<Candidate>, PathError> {
        let base = match base.filter(|b| !b.is_empty()) {
            Some(b) => Some(resolve_path(b, self.cur.as_deref(), self.env)?),
            None => self.cur.clone(),
        };
        let resolved_src = src.and_then(|s| {
            if is_url(s) { Some(s.to_owned()) } else { resolve_path(s, base.as_deref(), self.env).ok() }
        });
        let Some(dst) = dst.map(str::to_owned).or_else(|| inferred_destination(kind, src)) else {
            return Ok(None);
        };
        let dst = resolve_path(&dst, base.as_deref(), self.env)?;
        Ok(Some(Candidate { kind, src: resolved_src, dst, script }))
    }
}

/// A clone without a destination lands in a folder named after the repository; a copy, in the cwd.
fn inferred_destination(kind: OpKind, src: Option<&str>) -> Option<String> {
    match (kind, src) {
        (OpKind::Clone, Some(src)) if !src.is_empty() => repo_name(src),
        (OpKind::Copy, _) => Some(".".to_owned()),
        _ => None,
    }
}

fn repo_name(src: &str) -> Option<String> {
    let trimmed = src.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\', ':']).next().unwrap_or(trimmed);
    let name = name.strip_suffix(".git").unwrap_or(name);
    (!name.is_empty()).then(|| name.to_owned())
}

/// Whether `p` still depends on something that is not literal: `$`, a backtick, `*`, `?`, `%NAME%`, or a
/// brace expression (`{a,b}`, `{1..3}`) the lexer left unexpanded.
pub(super) fn is_unknown(p: &str) -> bool {
    p.contains(['$', '`', '*', '?']) || has_percent_variable(p) || has_brace_expression(p)
}

fn has_percent_variable(p: &str) -> bool {
    p.match_indices('%').any(|(i, _)| {
        let rest = &p[i + 1..];
        let name = rest.split(|c: char| c == '%' || c.is_whitespace()).next().unwrap_or("");
        !name.is_empty() && rest[name.len()..].starts_with('%')
    })
}

/// A `{...}` holding a comma or `..`: the pairs the lexer expands, so one left over was too long to expand.
fn has_brace_expression(p: &str) -> bool {
    let (mut open, mut separated, mut previous) = (false, false, 'x');
    for c in p.chars() {
        match c {
            '{' => open = true,
            ',' => separated |= open,
            '.' => separated |= open && previous == '.',
            '}' if open => {
                if separated {
                    return true;
                }
                open = false;
            }
            _ => {}
        }
        previous = c;
    }
    false
}

/// `~`, `/c/...` and `/tmp/...` into Windows-rooted text; any other MSYS root is unsupported.
fn expand_roots(p: &str, env: &Env) -> Result<String, PathError> {
    let p = match p {
        "~" => env.home.clone(),
        _ if p.starts_with("~/") || p.starts_with("~\\") => format!("{}{}", env.home, &p[1..]),
        _ => p.to_owned(),
    };
    let mut chars = p.chars();
    if chars.next() == Some('/') {
        if let Some(letter) = chars.next().filter(char::is_ascii_alphabetic) {
            let rest = chars.as_str();
            if rest.is_empty() || rest.starts_with('/') {
                return Ok(format!(r"{}:\{}", letter.to_ascii_uppercase(), rest.trim_start_matches('/')));
            }
        }
        if p == "/tmp" || p.starts_with("/tmp/") {
            return Ok(format!("{}{}", env.temp, &p[4..]));
        }
        if !p.starts_with("//") {
            return Err(PathError::Unsupported);
        }
    }
    Ok(p)
}

fn has_absolute_prefix(p: &str) -> bool {
    drive_root_len(p).is_some() || p.starts_with(r"\\")
}

fn is_drive_or_root_relative(p: &str) -> bool {
    let bytes = p.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':') || p.starts_with('\\')
}

/// Length of a leading `X:\`, if any.
fn drive_root_len(p: &str) -> Option<usize> {
    let bytes = p.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\').then_some(3)
}

/// Collapses `.`, `..` and repeated separators; `..` stops at the root, like `ntpath.normpath`.
fn normalize(p: &str) -> String {
    let (root, rest) = split_root(p);
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let tail = parts.join(r"\");
    match (root.ends_with('\\'), tail.is_empty()) {
        (_, true) => root,
        (true, false) => format!("{root}{tail}"),
        (false, false) => format!(r"{root}\{tail}"),
    }
}

/// The drive root (`D:\`) or the UNC share (`\\server\share`) and what follows it.
fn split_root(p: &str) -> (String, &str) {
    if let Some(len) = drive_root_len(p) {
        return (p[..len].to_owned(), &p[len..]);
    }
    let body = p.trim_start_matches('\\');
    let mut names = body.splitn(3, '\\');
    let server = names.next().unwrap_or("");
    let share = names.next().unwrap_or("");
    let rest = names.next().unwrap_or("");
    let root = if share.is_empty() { format!(r"\\{server}") } else { format!(r"\\{server}\{share}") };
    (root, rest)
}

/// `scheme://...` or `user@host:...`.
fn is_url(s: &str) -> bool {
    has_scheme(s) || has_scp_form(s)
}

fn has_scheme(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once("://") else { return false };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
}

fn has_scp_form(s: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || "_.-".contains(c);
    let Some((user, rest)) = s.split_once('@') else { return false };
    let host_len = rest.find(|c| !word(c)).unwrap_or(rest.len());
    !user.is_empty() && user.chars().all(word) && host_len > 0 && rest[host_len..].starts_with(':')
}
