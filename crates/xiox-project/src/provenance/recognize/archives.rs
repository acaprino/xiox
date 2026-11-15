//! Archive tools: extraction copies an archive into a directory, creation writes the archive.

use super::{Context, Invocation, Recognizer, path_op, powershell_with_path, words, write_op};
use crate::provenance::{OpKind, RawOp, posix};

pub(super) struct Archives;

impl Recognizer for Archives {
    fn names(&self) -> &'static [&'static str] {
        &["tar", "bsdtar", "unzip", "expand-archive"]
    }

    fn recognize(&self, inv: &Invocation, _ctx: &mut Context) -> Vec<RawOp> {
        match inv.name.as_str() {
            "unzip" => unzip(inv),
            "expand-archive" => expand_archive(inv),
            _ => tar(inv),
        }
    }
}

fn unzip(inv: &Invocation) -> Vec<RawOp> {
    let parsed = posix(inv.args, &["-d", "-P"]);
    let dst = parsed.options.get("-d").cloned().flatten();
    let src = parsed.positional.into_iter().next();
    src.map(|src| path_op(OpKind::Copy, Some(src), dst)).into_iter().collect()
}

fn expand_archive(inv: &Invocation) -> Vec<RawOp> {
    let params = powershell_with_path(inv.args, &[("destinationpath", "dst")]);
    let mut positional = params.positional.into_iter();
    let src = match params.named.get("path") {
        Some(paths) => paths.first().cloned(),
        None => positional.next().and_then(|paths| paths.into_iter().next()),
    };
    let dst = match params.named.get("dst") {
        Some(dst) => dst.first().cloned(),
        None => positional.next().and_then(|paths| paths.into_iter().next()),
    };
    src.map(|src| path_op(OpKind::Copy, Some(src), dst)).into_iter().collect()
}

/// What `tar` was asked to do and with which paths.
#[derive(Default)]
struct TarCall {
    extract: bool,
    create: bool,
    archive: Option<String>,
    directory: Option<String>,
}

fn tar(inv: &Invocation) -> Vec<RawOp> {
    let call = tar_call(&words(inv.args));
    if call.extract {
        vec![path_op(OpKind::Copy, call.archive, call.directory)]
    } else if call.create {
        call.archive.filter(|archive| archive != "-").map(write_op).into_iter().collect()
    } else {
        Vec::new()
    }
}

/// Reads long options (`--file=x`, `--directory x`), bundled short options (`-xzf x`) and the
/// old style without a dash (`tar xzf x`, first word only).
fn tar_call(words: &[&str]) -> TarCall {
    let mut call = TarCall::default();
    let mut i = 0;
    while i < words.len() {
        let word = words[i];
        if let Some(long) = word.strip_prefix("--") {
            let (key, inline) = match long.split_once('=') {
                Some((key, value)) => (key, Some(value)),
                None => (long, None),
            };
            match key {
                "extract" | "get" => call.extract = true,
                "create" => call.create = true,
                "file" | "directory" => {
                    let value = inline.or_else(|| {
                        i += 1;
                        words.get(i).copied()
                    });
                    let slot = if key == "file" { &mut call.archive } else { &mut call.directory };
                    *slot = value.map(str::to_owned);
                }
                _ => {}
            }
        } else if word.starts_with('-') || (i == 0 && word.chars().all(char::is_alphabetic)) {
            let letters = word.trim_start_matches('-');
            for (at, letter) in letters.char_indices() {
                match letter {
                    'x' => call.extract = true,
                    'c' => call.create = true,
                    'f' | 'C' => {
                        let rest = &letters[at + 1..];
                        let value = if rest.is_empty() {
                            i += 1;
                            words.get(i).copied()
                        } else {
                            Some(rest)
                        };
                        let slot = if letter == 'f' { &mut call.archive } else { &mut call.directory };
                        *slot = value.map(str::to_owned);
                        break;
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }
    call
}
