//! Whether git may be asked about a candidate repository: every place its `.git` makes git read
//! must stay on local fixed drives and never cross a link, at any depth.

use std::fs;
use std::path::{Path, PathBuf};

use super::fixtures::{clone, env, git, link_dir, repo, tmp};
use crate::gitref::{git_may_read, git_may_read_tree};
use crate::*;

/// How a row lays out `<base>\cand` (and what it points at) before the check; `None` skips the
/// row on a machine that cannot make a junction.
type Layout = fn(&Path) -> Option<()>;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// `<base>\real\.git` with its `objects`, a git folder some rows point at.
fn real_git(base: &Path) -> PathBuf {
    let g = base.join("real").join(".git");
    fs::create_dir_all(g.join("objects")).unwrap();
    g
}

/// `<base>\cand\.git` as a folder with its `objects`.
fn cand_git(base: &Path) -> PathBuf {
    let g = base.join("cand").join(".git");
    fs::create_dir_all(g.join("objects")).unwrap();
    g
}

fn config(base: &Path, text: &str) -> Option<()> {
    Some(write(&cand_git(base).join("config"), text))
}

#[test]
fn git_may_read_table() {
    let table: [(&str, Layout, bool); 22] = [
        ("a .git folder", |b| Some(drop(cand_git(b))), true),
        (
            "a relative gitdir",
            |b| {
                real_git(b);
                Some(write(&b.join("cand").join(".git"), "gitdir: ../real/.git\n"))
            },
            true,
        ),
        ("a gitdir on a network share", |b| Some(write(&b.join("cand").join(".git"), "gitdir: \\\\127.0.0.1\\share\\x.git\n")), false),
        ("a gitdir on a share, with forward slashes", |b| Some(write(&b.join("cand").join(".git"), "gitdir: //srv/s/x.git\n")), false),
        ("a verbatim UNC gitdir", |b| Some(write(&b.join("cand").join(".git"), "gitdir: \\\\?\\UNC\\srv\\s\\x.git\n")), false),
        (
            "a gitdir through a junction",
            |b| {
                real_git(b);
                link_dir(&b.join("link"), &b.join("real")).then_some(())?;
                Some(write(&b.join("cand").join(".git"), &format!("gitdir: {}\n", b.join("link").join(".git").display())))
            },
            false,
        ),
        (
            "a .git that is a junction",
            |b| {
                real_git(b);
                fs::create_dir_all(b.join("cand")).unwrap();
                link_dir(&b.join("cand").join(".git"), &b.join("real").join(".git")).then_some(())
            },
            false,
        ),
        ("a .git file without gitdir", |b| Some(write(&b.join("cand").join(".git"), "hello\n")), false),
        ("a commondir on a share", |b| Some(write(&cand_git(b).join("commondir"), "\\\\srv\\s\\c\n")), false),
        (
            "objects through a junction",
            |b| {
                let objects = real_git(b).join("objects");
                fs::create_dir_all(b.join("cand").join(".git")).unwrap();
                link_dir(&b.join("cand").join(".git").join("objects"), &objects).then_some(())
            },
            false,
        ),
        (
            "local alternates",
            |b| {
                let line = format!("# shared objects\n{}\n", real_git(b).join("objects").display());
                Some(write(&cand_git(b).join("objects").join("info").join("alternates"), &line))
            },
            true,
        ),
        ("alternates on a share", |b| Some(write(&cand_git(b).join("objects").join("info").join("alternates"), "\\\\srv\\s\\objects\n")), false),
        (
            "local alternates that borrow from a share",
            |b| {
                let objects = real_git(b).join("objects");
                write(&objects.join("info").join("alternates"), "\\\\srv\\s\\objects\n");
                Some(write(&cand_git(b).join("objects").join("info").join("alternates"), &format!("{}\n", objects.display())))
            },
            false,
        ),
        ("an include on a share", |b| config(b, "[include]\n\tpath = \\\\srv\\s\\x.cfg\n"), false),
        ("an include on the section line", |b| config(b, "[include] path = \\\\srv\\s\\x.cfg\n"), false),
        ("a BOM before the first section", |b| config(b, "\u{feff}[include]\n\tpath = \\\\srv\\s\\x.cfg\n"), false),
        (
            "an include that includes a share",
            |b| {
                write(&b.join("a.cfg"), "[include]\n\tpath = \\\\srv\\s\\b.cfg\n");
                config(b, "[include]\n\tpath = ../../a.cfg\n")
            },
            false,
        ),
        (
            "an includeIf in the home",
            |b| {
                write(&b.join("home").join("x.cfg"), "[user]\n");
                config(b, "[includeIf \"gitdir:x\"]\n\tpath = ~/x.cfg\n")
            },
            true,
        ),
        (
            "a relative include",
            |b| {
                write(&b.join("x.cfg"), "[user]\n");
                config(b, "[core]\n\tbare = false\n[include]\n\tpath = ../../x.cfg\n")
            },
            true,
        ),
        ("a worktree on a share", |b| config(b, "[core]\n\tworktree = \\\\srv\\s\\w\n"), false),
        ("an excludes file on a share", |b| config(b, "[core]\n\texcludesFile = \\\\srv\\s\\ignore\n"), false),
        ("a value continued on the next line", |b| config(b, "[include]\n\tpath = a\\\n\\\\srv\\s\\x.cfg\n"), false),
    ];
    let mut wrong = Vec::new();
    for (name, layout, expected) in table {
        let (_t, base) = tmp();
        if layout(&base).is_none() {
            println!("skipped: {name}: this machine cannot make a junction");
            continue;
        }
        if git_may_read(&base.join("cand"), Some(&base.join("home"))) != expected {
            wrong.push(name);
        }
    }
    assert!(wrong.is_empty(), "wrong answers: {wrong:#?}");
}

#[test]
fn git_may_read_tree_table() {
    // (name, layout, the repository itself, it and every repository inside it)
    let table: [(&str, Layout, bool, bool); 3] = [
        (
            "a clean nested repository",
            |b| {
                cand_git(b);
                real_git(b);
                Some(write(&b.join("cand").join("sub").join(".git"), "gitdir: ../../real/.git\n"))
            },
            true,
            true,
        ),
        (
            "a nested repository on a share",
            |b| {
                cand_git(b);
                Some(write(&b.join("cand").join("sub").join(".git"), "gitdir: \\\\srv\\s\\x\n"))
            },
            true,
            false,
        ),
        (
            "a repository two levels down on a share",
            |b| {
                cand_git(b);
                real_git(b);
                write(&b.join("cand").join("a").join(".git"), "gitdir: ../../real/.git\n");
                Some(write(&b.join("cand").join("a").join("b").join(".git"), "gitdir: \\\\srv\\s\\x\n"))
            },
            true,
            false,
        ),
    ];
    for (name, layout, itself, tree) in table {
        let (_t, base) = tmp();
        layout(&base).unwrap();
        let (cand, home) = (base.join("cand"), base.join("home"));
        assert_eq!((git_may_read(&cand, Some(&home)), git_may_read_tree(&cand, Some(&home))), (itself, tree), "{name}");
    }
}

fn item_at<'a>(scan: &'a ProjectScan, dir: &Path) -> &'a Item {
    let key = xiox_model::path_key(dir);
    scan.items.iter().find(|i| i.key == key).unwrap_or_else(|| panic!("{key} not listed: {:#?}", scan.items))
}

fn scan_of(project: &Path) -> ProjectScan {
    analyze_with(project, &Options { history: false, ..Options::default() }, &env()).unwrap()
}

#[test]
fn a_copy_whose_submodule_points_through_a_link_is_not_vouched_for() {
    // `git status` enters a populated submodule and follows its `.git` file.
    for through_a_link in [false, true] {
        let (_t, base) = tmp();
        let upstream = repo(&base.join("elsewhere").join("up"));
        let project = repo(&base.join("work").join("proj"));
        let up = upstream.to_string_lossy().into_owned();
        git(&project, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", &up, "sub"]);
        git(&project, &["commit", "-q", "-m", "sub"]);
        let copy = clone(&project, &base.join("work").join("proj-copy"));
        git(&copy, &["-c", "protocol.file.allow=always", "submodule", "update", "--init", "-q"]);
        if through_a_link {
            let modules = copy.join(".git").join("modules").join("sub");
            if !link_dir(&base.join("link"), &modules) {
                println!("skipped: this machine cannot make a junction");
                continue;
            }
            fs::write(copy.join("sub").join(".git"), format!("gitdir: {}\n", base.join("link").display())).unwrap();
        }

        let scan = scan_of(&project);
        let copy_item = item_at(&scan, &copy);
        assert_eq!(copy_item.category, Category::Copies, "{through_a_link}");
        let Why::SameRepository { has_own_work, .. } = copy_item.why else { panic!("{:?}", copy_item.why) };
        let expected = if through_a_link { (None, vec![Block::UnsavedWorkUnknown]) } else { (Some(false), vec![]) };
        assert_eq!((has_own_work, copy_item.blocked.clone()), expected, "through a link: {through_a_link}");
    }
}

#[test]
fn a_candidate_whose_gitdir_crosses_a_link_is_no_copy() {
    let (_t, base) = tmp();
    let project = repo(&base.join("work").join("proj"));
    let real = clone(&project, &base.join("elsewhere").join("real"));
    if !link_dir(&base.join("link"), &real) {
        println!("skipped: this machine cannot make a junction");
        return;
    }
    let candidate = base.join("work").join("proj-old");
    write(&candidate.join(".git"), &format!("gitdir: {}\n", base.join("link").join(".git").display()));

    let scan = scan_of(&project);
    let item = item_at(&scan, &candidate);
    assert_eq!((item.category, item.blocked.clone()), (Category::Named, vec![Block::UnsavedWorkUnknown]));
}

#[test]
fn a_named_folder_holding_a_repository_behind_a_link_is_not_vouched_for() {
    let (_t, base) = tmp();
    let project = repo(&base.join("work").join("proj"));
    // Seen through the link, git would find it clean and let the folder go.
    let real = clone(&project, &base.join("elsewhere").join("real"));
    if !link_dir(&base.join("link"), &real) {
        println!("skipped: this machine cannot make a junction");
        return;
    }
    let named = base.join("work").join("proj-box");
    write(&named.join("inner").join(".git"), &format!("gitdir: {}\n", base.join("link").join(".git").display()));

    let scan = scan_of(&project);
    let item = item_at(&scan, &named);
    assert_eq!((item.category, item.blocked.clone()), (Category::Named, vec![Block::UnsavedWorkUnknown]));
}
