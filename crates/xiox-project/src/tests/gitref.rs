//! Whether git may be asked about a candidate repository: every indirect reference of its `.git`
//! must stay on local fixed drives and never cross a link.

use std::fs;
use std::path::Path;

use super::fixtures::{link_dir, tmp};
use crate::gitref::git_may_read;

/// How a row lays out `<base>\cand` (and what it points at) before the check.
type Layout = fn(&Path) -> Option<()>;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// `<base>\real\.git`, a folder some rows point at.
fn real_git(base: &Path) -> std::path::PathBuf {
    let g = base.join("real").join(".git");
    fs::create_dir_all(g.join("objects")).unwrap();
    g
}

#[test]
fn git_may_read_table() {
    // (name, layout; `None` from the layout skips the row on this machine, expected)
    let table: [(&str, Layout, bool); 11] = [
        ("a .git folder", |b| Some(fs::create_dir_all(b.join("cand").join(".git")).unwrap()), true),
        (
            "a relative gitdir",
            |b| {
                real_git(b);
                Some(write(&b.join("cand").join(".git"), "gitdir: ../real/.git\n"))
            },
            true,
        ),
        ("a gitdir on a network share", |b| Some(write(&b.join("cand").join(".git"), "gitdir: \\\\127.0.0.1\\share\\x.git\n")), false),
        (
            "a gitdir through a junction",
            |b| {
                real_git(b);
                link_dir(&b.join("link"), &b.join("real")).then_some(())?;
                Some(write(&b.join("cand").join(".git"), &format!("gitdir: {}\n", b.join("link").join(".git").display())))
            },
            false,
        ),
        ("a .git file without gitdir", |b| Some(write(&b.join("cand").join(".git"), "hello\n")), false),
        ("a commondir on a share", |b| Some(write(&b.join("cand").join(".git").join("commondir"), "\\\\srv\\s\\c\n")), false),
        (
            "local alternates",
            |b| {
                let objects = real_git(b).join("objects");
                let line = format!("# shared objects\n{}\n", objects.display());
                Some(write(&b.join("cand").join(".git").join("objects").join("info").join("alternates"), &line))
            },
            true,
        ),
        (
            "alternates on a share",
            |b| Some(write(&b.join("cand").join(".git").join("objects").join("info").join("alternates"), "\\\\srv\\s\\objects\n")),
            false,
        ),
        ("an include on a share", |b| Some(write(&b.join("cand").join(".git").join("config"), "[include]\n\tpath = \\\\srv\\s\\x.cfg\n")), false),
        (
            "an includeIf in the home",
            |b| {
                write(&b.join("home").join("x.cfg"), "[user]\n");
                Some(write(&b.join("cand").join(".git").join("config"), "[includeIf \"gitdir:x\"]\n\tpath = ~/x.cfg\n"))
            },
            true,
        ),
        (
            "a relative include",
            |b| {
                write(&b.join("x.cfg"), "[user]\n");
                Some(write(&b.join("cand").join(".git").join("config"), "[core]\n\tbare = false\n[include]\n\tpath = ../../x.cfg\n"))
            },
            true,
        ),
    ];
    for (name, layout, expected) in table {
        let (_t, base) = tmp();
        if layout(&base).is_none() {
            println!("skipped: {name}: this machine cannot make a junction");
            continue;
        }
        let home = base.join("home");
        assert_eq!(git_may_read(&base.join("cand"), Some(&home)), expected, "{name}");
    }
}
