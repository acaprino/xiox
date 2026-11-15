use std::collections::HashSet;

use crate::provenance::{
    Context, Invocation, Node, OpKind, RawOp, Registry, Shell, lex, parse,
};

/// The operations the registry reads from the first simple command of `cmd`.
fn recognize(shell: Shell, cmd: &str) -> (Vec<RawOp>, Context) {
    let dialect = shell.dialect();
    let nodes = parse(lex(cmd, dialect), dialect);
    let Some(Node::Simple(simple)) = nodes.first() else { panic!("not a simple command: {cmd}") };
    let name = match &simple.words[0] {
        crate::provenance::Arg::Text(word) => word.to_lowercase(),
        crate::provenance::Arg::Comma => panic!("command starts with a comma"),
    };
    let invocation = Invocation { name, args: &simple.words[1..], shell, heredoc: None };
    let mut ctx = Context::default();
    let ops = Registry::standard().recognize(&invocation, &mut ctx);
    (ops, ctx)
}

fn path(kind: OpKind, src: Option<&str>, dst: Option<&str>) -> RawOp {
    RawOp::Path { kind, src: src.map(Into::into), dst: dst.map(Into::into), base: None, script: false }
}

fn create(dst: &str) -> RawOp {
    path(OpKind::Create, None, Some(dst))
}

fn write(dst: &str) -> RawOp {
    path(OpKind::Write, None, Some(dst))
}

fn copy(src: &str, dst: &str) -> RawOp {
    path(OpKind::Copy, Some(src), Some(dst))
}

fn cd(dst: &str) -> RawOp {
    RawOp::Cd { dst: Some(dst.into()), push: false }
}

#[test]
fn file_system_recognizers() {
    use Shell::{Bash, Cmd, PowerShell};
    let cases: Vec<(Shell, &str, Vec<RawOp>)> = vec![
        (Cmd, r"cd /d D:\runs", vec![cd(r"D:\runs")]),
        (Bash, "cd", vec![cd("~")]),
        (PowerShell, r"Set-Location -Path D:\w", vec![cd(r"D:\w")]),
        (Bash, "pushd /d/w", vec![RawOp::Cd { dst: Some("/d/w".into()), push: true }]),
        (Bash, "popd", vec![RawOp::PopCd]),
        (Bash, "mkdir -p a b", vec![create("a"), create("b")]),
        (Cmd, r"mkdir D:\a /p", vec![create(r"D:\a")]),
        (PowerShell, "mkdir out", vec![create("out")]),
        (PowerShell, r"New-Item -Path D:\runs -Name r -ItemType Directory", vec![create(r"D:\runs\r")]),
        (PowerShell, "New-Item -Name n.txt", vec![write(r".\n.txt")]),
        (PowerShell, "New-Item f.txt", vec![write("f.txt")]),
        (PowerShell, "New-Item -ItemType SymbolicLink l", vec![]),
        (Bash, "cp -t /d/x a b", vec![copy("a", "/d/x"), copy("b", "/d/x")]),
        (Bash, "cp a b c", vec![copy("a", "c"), copy("b", "c")]),
        (Bash, "mv a b", vec![path(OpKind::Move, Some("a"), Some("b"))]),
        (Bash, "cp a", vec![]),
        (PowerShell, r"Copy-Item .\data D:\s -Recurse", vec![copy(r".\data", r"D:\s")]),
        (PowerShell, r"Copy-Item -Path a, b -Destination D:\s", vec![copy("a", r"D:\s"), copy("b", r"D:\s")]),
        (PowerShell, "Copy-Item a", vec![]),
        (PowerShell, r"cp a D:\s", vec![copy("a", r"D:\s")]),
        (Bash, "Copy-Item a D:/s", vec![copy("a", "D:/s")]),
        (Bash, "robocopy src dst /E /XD node_modules", vec![copy("src", "dst")]),
        (Cmd, r"xcopy /E /I a D:\b", vec![copy("a", r"D:\b")]),
        (Bash, "tar -xzf a.tgz -C /d/x", vec![copy("a.tgz", "/d/x")]),
        (Bash, "tar xzf a.tgz --directory=/d/x", vec![copy("a.tgz", "/d/x")]),
        (Bash, "tar --extract --file a.tgz", vec![path(OpKind::Copy, Some("a.tgz"), None)]),
        (Bash, "tar -czf out.tgz dir", vec![write("out.tgz")]),
        (Bash, "tar -cf - dir", vec![]),
        (Bash, "tar -tf a.tgz", vec![]),
        (Bash, "unzip -q d.zip -d /d/u", vec![copy("d.zip", "/d/u")]),
        (Bash, "unzip d.zip", vec![path(OpKind::Copy, Some("d.zip"), None)]),
        (PowerShell, r"Expand-Archive -Path r.zip -DestinationPath D:\e", vec![copy("r.zip", r"D:\e")]),
        (PowerShell, r"Expand-Archive r.zip D:\e", vec![copy("r.zip", r"D:\e")]),
        (Bash, "tee a b", vec![write("a"), write("b")]),
        (Bash, "touch f", vec![write("f")]),
        (PowerShell, r"Out-File -FilePath D:\o.txt -Encoding utf8", vec![write(r"D:\o.txt")]),
        (PowerShell, "Set-Content f.txt -Value hi", vec![write("f.txt")]),
        (Bash, "sed -i 's/a/b/' x.rs y.md", vec![write("x.rs"), write("y.md")]),
        (Bash, "sed -ni -e p x", vec![write("x")]),
        (Bash, "sed --in-place=.bak s/a/b/ x", vec![write("x")]),
        (Bash, "sed -n 1p f", vec![]),
        (Bash, "pytest --basetemp=/d/p", vec![write("/d/p")]),
        (Bash, "cargo build --target-dir /d/t", vec![write("/d/t")]),
        (Bash, "tool --out", vec![]),
        (Bash, "tool --output= x", vec![]),
        (Bash, "ls -la", vec![]),
    ];
    for (shell, cmd, expected) in cases {
        assert_eq!(recognize(shell, cmd).0, expected, "{shell:?}: {cmd}");
    }
}

#[test]
fn loop_names_set_the_flag() {
    for name in ["for", "foreach", "foreach-object", "%"] {
        let (ops, ctx) = recognize(Shell::PowerShell, &format!("{name} --out x"));
        assert!(ops.is_empty(), "{name}");
        assert!(ctx.flags.looped, "{name}");
    }
    assert!(!recognize(Shell::Bash, "ls").1.flags.looped);
}

#[test]
fn registry_names_are_unique() {
    let mut seen = HashSet::new();
    for family in Registry::families() {
        for name in family.names() {
            assert!(seen.insert(*name), "`{name}` has two recognizers");
            assert_eq!(*name, name.to_lowercase());
        }
    }
    let _ = Registry::standard();
}
