use crate::provenance::{Env, OpKind, PathError, RawOp, is_null_device, resolve, resolve_path};
use xiox_mukade::strip_verbatim;
use crate::Candidate;

fn env() -> Env {
    Env { home: r"C:\Users\me".into(), temp: r"C:\Temp".into() }
}

fn create(dst: &str) -> RawOp {
    RawOp::Path { kind: OpKind::Create, src: None, dst: Some(dst.into()), base: None, script: false }
}

fn cd(dst: &str, push: bool) -> RawOp {
    RawOp::Cd { dst: Some(dst.into()), push }
}

fn path_op(kind: OpKind, src: Option<&str>, dst: Option<&str>) -> RawOp {
    RawOp::Path { kind, src: src.map(Into::into), dst: dst.map(Into::into), base: None, script: false }
}

fn candidate(kind: OpKind, src: Option<&str>, dst: &str) -> Candidate {
    Candidate { kind, src: src.map(Into::into), dst: dst.into(), script: false }
}

#[test]
fn resolve_path_table() {
    use PathError::*;
    let cases: [(&str, Option<&str>, Result<&str, PathError>); 22] = [
        ("/d/runs/x", None, Ok(r"D:\runs\x")),
        ("/d", None, Ok(r"D:\")),
        ("/tmp/a", None, Ok(r"C:\Temp\a")),
        ("~/s", None, Ok(r"C:\Users\me\s")),
        ("~", None, Ok(r"C:\Users\me")),
        ("../b", Some(r"D:\P\q"), Ok(r"D:\P\b")),
        ("D:/x/", None, Ok(r"D:\x")),
        (r"D:\..\x", None, Ok(r"D:\x")),
        ("a/./b//c", Some(r"D:\P"), Ok(r"D:\P\a\b\c")),
        (r"\\server\s", None, Ok(r"\\server\s")),
        ("$OUT/a", Some(r"D:\P"), Err(Unknown)),
        (r"%TEMP%\a", Some(r"D:\P"), Err(Unknown)),
        ("`pwd`", Some(r"D:\P"), Err(Unknown)),
        ("*.json", Some(r"D:\P"), Err(Unknown)),
        ("50%", Some(r"D:\P"), Ok(r"D:\P\50%")),
        ("/usr/bin", Some(r"D:\P"), Err(Unsupported)),
        ("D:foo", Some(r"D:\P"), Err(Unsupported)),
        (r"\x", Some(r"D:\P"), Err(Unsupported)),
        ("a", None, Err(NoCwd)),
        (".", Some(r"D:\P"), Ok(r"D:\P")),
        ("  a  ", Some(r"D:\P"), Ok(r"D:\P\a")),
        ("é/ü", Some(r"D:\P"), Ok(r"D:\P\é\ü")),
    ];
    for (input, cwd, expected) in cases {
        assert_eq!(resolve_path(input, cwd, &env()), expected.map(String::from), "{input:?} from {cwd:?}");
    }
}

#[test]
fn impossible_characters_are_unsupported() {
    // D12: a Windows path cannot hold these, so the text was not a path.
    let cwd = Some(r"D:\P");
    for input in ["a\nb", "a\tb", "a\u{7f}b", "a\"b", "a<b", "a>b", "a|b", r"D:\x:y", "x:y", r"\\srv\s\a:b", "/d/a:b"] {
        assert_eq!(resolve_path(input, cwd, &env()), Err(PathError::Unsupported), "{input:?}");
    }
    // A glob stays unknown; the drive colon and ordinary punctuation stay valid.
    assert_eq!(resolve_path("a\n*", cwd, &env()), Err(PathError::Unknown));
    assert_eq!(resolve_path(r"D:\a b(1)'x'!.txt", cwd, &env()), Ok(r"D:\a b(1)'x'!.txt".to_owned()));
}

#[test]
fn strip_verbatim_and_null_devices() {
    let verbatim = [
        (r"\\?\D:\x", r"D:\x"),
        (r"\\?\UNC\server\share\x", r"\\server\share\x"),
        (r"D:\x", r"D:\x"),
        (r"\\server\share", r"\\server\share"),
    ];
    for (input, expected) in verbatim {
        assert_eq!(strip_verbatim(input), expected, "{input}");
    }
    for device in ["/dev/null", "NUL", "nul:", "$null", "-", " Nul "] {
        assert!(is_null_device(device), "{device}");
    }
    assert!(!is_null_device("null.txt"));
}

#[test]
fn resolve_tracks_cd_push_pop_and_base() {
    let based = RawOp::Path {
        kind: OpKind::Create,
        src: None,
        dst: Some("d".into()),
        base: Some(r"D:\P".into()),
        script: true,
    };
    let ops = [cd("/d/runs", false), create("a"), cd("x", true), create("b"), RawOp::PopCd, create("c"), based];
    let got = resolve(&ops, r"D:\start", &env());
    let mut scripted = candidate(OpKind::Create, None, r"D:\P\d");
    scripted.script = true;
    assert_eq!(
        got.candidates,
        vec![
            candidate(OpKind::Create, None, r"D:\runs\a"),
            candidate(OpKind::Create, None, r"D:\runs\x\b"),
            candidate(OpKind::Create, None, r"D:\runs\c"),
            scripted,
        ]
    );
    assert_eq!(got.unresolved, 0);
}

#[test]
fn visits_follow_each_cd_where_it_ran() {
    let ops = [cd("/d/a", false), cd("b", false), cd("c", true), RawOp::PopCd, cd(".", false)];
    let got = resolve(&ops, r"D:\P", &env());
    assert_eq!(got.visits, [r"D:\a", r"D:\a\b", r"D:\a\b\c"]);
}

#[test]
fn clone_without_destination_takes_repo_name() {
    let cases = [
        ("https://h/acme/planner.git", r"D:\w\planner"),
        ("git@h:acme/docs.git", r"D:\w\docs"),
        ("https://h/acme/tools/", r"D:\w\tools"),
    ];
    for (url, expected) in cases {
        let got = resolve(&[path_op(OpKind::Clone, Some(url), None)], r"D:\w", &env());
        assert_eq!(got.candidates, vec![candidate(OpKind::Clone, Some(url), expected)], "{url}");
    }
}

#[test]
fn copy_without_destination_is_cwd() {
    let got = resolve(&[path_op(OpKind::Copy, Some("f.txt"), None)], r"D:\w", &env());
    assert_eq!(got.candidates, vec![candidate(OpKind::Copy, Some(r"D:\w\f.txt"), r"D:\w")]);
}

#[test]
fn null_devices_are_dropped_not_counted() {
    let ops: Vec<RawOp> = ["/dev/null", "NUL", "$null", "-"].into_iter().map(create).collect();
    let got = resolve(&ops, r"D:\w", &env());
    assert!(got.candidates.is_empty());
    assert_eq!(got.unresolved, 0);
}

#[test]
fn unknown_cd_poisons_relative_paths_after_it() {
    let got = resolve(&[cd("$X", false), create("a"), create("/d/ok")], r"D:\w", &env());
    assert_eq!(got.candidates, vec![candidate(OpKind::Create, None, r"D:\ok")]);
    assert!(got.visits.is_empty());
    assert_eq!(got.unresolved, 2);
}

#[test]
fn unresolved_src_keeps_the_op() {
    let got = resolve(&[path_op(OpKind::Copy, Some("$SRC/f"), Some("out"))], r"D:\w", &env());
    assert_eq!(got.candidates, vec![candidate(OpKind::Copy, None, r"D:\w\out")]);
    assert_eq!(got.unresolved, 0);
}

#[test]
fn unresolved_destination_is_counted() {
    let got = resolve(&[create("$OUT/a"), path_op(OpKind::Move, Some("a"), None)], r"D:\w", &env());
    assert!(got.candidates.is_empty());
    assert_eq!(got.unresolved, 1);
}
