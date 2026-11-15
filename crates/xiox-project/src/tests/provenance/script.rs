use std::time::{Duration, Instant};

use crate::provenance::{Lang, OpKind, RawOp, scan_script};

fn op(kind: OpKind, dst: &str) -> RawOp {
    RawOp::Path { kind, src: None, dst: Some(dst.into()), base: None, script: true }
}

/// Each case is `(code, expected operations)`; the scan must also leave nothing unresolved.
fn assert_ops(lang: Lang, cases: &[(&str, &[(OpKind, &str)])]) {
    for (code, expected) in cases {
        let scan = scan_script(code, lang);
        let expected: Vec<RawOp> = expected.iter().map(|(kind, dst)| op(*kind, dst)).collect();
        assert_eq!(scan.ops, expected, "ops of {code:?}");
        assert_eq!(scan.unresolved, 0, "unresolved of {code:?}");
    }
}

#[test]
fn python_literal_calls() {
    assert_ops(
        Lang::Python,
        &[
            ("os.makedirs('D:/r', exist_ok=True)", &[(OpKind::Create, "D:/r")]),
            ("os.mkdir('D:/m')", &[(OpKind::Create, "D:/m")]),
            ("open(\"a.txt\", \"w\")", &[(OpKind::Write, "a.txt")]),
            ("open(\"a.txt\")", &[]),
            ("open('a.txt', 'r')", &[]),
            ("open('x', mode='wb')", &[(OpKind::Write, "x")]),
            ("open('x', mode = 'a')", &[(OpKind::Write, "x")]),
            ("Path(\"D:/p\").mkdir(parents=True)", &[(OpKind::Create, "D:/p")]),
            ("Path('D:/f').write_text('hi')", &[(OpKind::Write, "D:/f")]),
            ("Path('D:/f').touch()", &[(OpKind::Write, "D:/f")]),
            ("Path('D:/f').exists()", &[]),
            ("shutil.copytree(\"src\", \"D:/t\")", &[(OpKind::Copy, "D:/t")]),
            ("shutil.copy2(a, 'D:/c')", &[(OpKind::Copy, "D:/c")]),
            ("shutil.move('a', 'D:/mv')", &[(OpKind::Move, "D:/mv")]),
            ("os.makedirs(r\"D:\\x\")", &[(OpKind::Create, "D:\\x")]),
            ("os.makedirs(\"D:\\\\x\")", &[(OpKind::Create, "D:\\x")]),
            (
                "os.makedirs('a'); open('b', 'w'); Path('c').mkdir()",
                &[(OpKind::Create, "a"), (OpKind::Write, "b"), (OpKind::Create, "c")],
            ),
        ],
    );
}

#[test]
fn js_literal_calls() {
    assert_ops(
        Lang::JavaScript,
        &[
            ("require('fs').mkdirSync('D:/n', {recursive: true})", &[(OpKind::Create, "D:/n")]),
            ("fs.writeFileSync(\"D:/p.json\", JSON.stringify({ ok: true }))", &[(OpKind::Write, "D:/p.json")]),
            ("fsp.appendFileSync('D:/log', 'x')", &[(OpKind::Write, "D:/log")]),
            ("fs.copyFileSync(a, 'D:/dst')", &[(OpKind::Copy, "D:/dst")]),
            ("fs.renameSync('a', 'D:/b')", &[(OpKind::Move, "D:/b")]),
            ("fs.readFileSync('D:/read')", &[]),
        ],
    );
}

#[test]
fn unicode_identifier_boundary() {
    assert_ops(
        Lang::Python,
        &[
            ("\u{e9}open('D:/ghost','w')", &[]),
            ("_open('D:/g','w')", &[]),
            ("(open('D:/k','w'))", &[(OpKind::Write, "D:/k")]),
        ],
    );
}

#[test]
fn calls_inside_string_literals_are_text() {
    assert_ops(
        Lang::Python,
        &[("print(\"open('D:/x','w')\")", &[]), ("x = '''os.makedirs('D:/y')'''", &[])],
    );
}

#[test]
fn non_literals_are_counted() {
    let cases: &[(&str, Lang)] = &[
        ("os.makedirs(f\"{d}/x\")", Lang::Python),
        ("open(\"a\" + b, \"w\")", Lang::Python),
        ("fs.mkdirSync(`${r}/x`)", Lang::JavaScript),
        ("open(\"{}/x\".format(d), \"w\")", Lang::Python),
        ("os.makedirs(os.path.join(a, \"b\"))", Lang::Python),
        ("os.makedirs(root)", Lang::Python),
    ];
    for (code, lang) in cases {
        let scan = scan_script(code, *lang);
        assert!(scan.ops.is_empty(), "ops of {code:?}");
        assert_eq!(scan.unresolved, 1, "unresolved of {code:?}");
    }
}

#[test]
fn uncertain_calls_are_not_counted() {
    let cases = ["open(name)", "open(name, 'r')", "open(name, m)", "Path(root).exists()", "os.makedirs()"];
    for code in cases {
        let scan = scan_script(code, Lang::Python);
        assert!(scan.ops.is_empty(), "ops of {code:?}");
        assert_eq!(scan.unresolved, 0, "unresolved of {code:?}");
    }
}

#[test]
fn nested_calls_are_read_once() {
    let scan = scan_script("shutil.copy(src, os.path.join(\"D:/a\", n))", Lang::Python);
    assert!(scan.ops.is_empty());
    assert_eq!(scan.unresolved, 1);

    let scan = scan_script("open(os.makedirs('D:/in'), 'w')", Lang::Python);
    assert_eq!(scan.ops, vec![op(OpKind::Create, "D:/in")]);
    assert_eq!(scan.unresolved, 1);
}

#[test]
fn truncated_call_gives_nothing() {
    let scan = scan_script("open(\"a.txt\", \"w", Lang::Python);
    assert!(scan.ops.is_empty());
    assert_eq!(scan.unresolved, 0);
}

#[test]
fn adversarial_scripts_scan_once() {
    let started = Instant::now();
    let unclosed = "open(".repeat(50_000);
    let escapes = format!("open(\"{}", "\\\\".repeat(20_000));
    for code in [unclosed, escapes] {
        let scan = scan_script(&code, Lang::Python);
        assert!(scan.ops.is_empty());
    }
    assert!(started.elapsed() < Duration::from_secs(2));
}
