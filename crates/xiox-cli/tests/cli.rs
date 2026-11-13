//! Runs the built `xiox` on fake trees. Never touches a real project or a real session:
//! `XIOX_SEARCH_ROOTS` replaces the drives and the home, `XIOX_NO_SESSIONS` skips the session
//! listing (both are read by debug builds only).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn xiox() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_xiox"));
    c.env("XIOX_NO_SESSIONS", "1");
    c.env_remove("XIOX_SEARCH_ROOTS");
    c.env("GIT_TERMINAL_PROMPT", "0");
    c
}

/// A fake project (not a git repository) with a Rust `target` folder; returns (temp dir, project).
fn fake_project() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let canon = std::fs::canonicalize(t.path()).unwrap();
    // `canonicalize` gives the verbatim form (backslash, backslash, question mark, backslash, then the
    // drive); xiox prints the plain one.
    let verbatim = String::from("\\\\?\\");
    let base = PathBuf::from(canon.to_string_lossy().trim_start_matches(verbatim.as_str()));
    let project = base.join("work").join("fakeproj");
    std::fs::create_dir_all(project.join("target").join("debug")).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(project.join("target").join("CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55\n").unwrap();
    std::fs::write(project.join("target").join("debug").join("big.bin"), vec![b'x'; 3 * 1024 * 1024]).unwrap();
    (t, project)
}

fn run_analyze(project: &Path, extra: &[&str]) -> Output {
    xiox()
        .arg("analyze")
        .arg(project)
        .args(extra)
        .env("XIOX_SEARCH_ROOTS", project.parent().unwrap())
        .env("GIT_CEILING_DIRECTORIES", project.parent().unwrap().parent().unwrap())
        .output()
        .unwrap()
}

#[test]
fn analyze_on_a_fake_project_exits_0_and_lists_its_target_folder() {
    let (_t, project) = fake_project();
    let out = run_analyze(&project, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}\nstdout: {stdout}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("Project  fakeproj"), "{stdout}");
    assert!(stdout.contains("BUILDS"), "{stdout}");
    assert!(stdout.contains(&project.join("target").to_string_lossy().into_owned()), "{stdout}");
    assert!(stdout.contains("Rust build"), "{stdout}");
    assert!(stdout.contains("(not a git repository)"), "{stdout}");
    assert!(stdout.contains("Clean with: xiox clean"), "{stdout}");
    assert!(stdout.contains("--builds"), "{stdout}");
}

#[test]
fn analyze_json_stdout_is_one_json_object() {
    let (_t, project) = fake_project();
    let out = run_analyze(&project, &["--json"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is exactly one JSON value");
    assert!(v.is_object());
    assert_eq!(v["schema"], 1);
    assert_eq!(v["project"]["name"], "fakeproj");
    assert_eq!(v["items"][0]["category"], "builds");
    assert!(out.stdout.ends_with(b"}\n"), "one object and a newline");
    assert_eq!(out.stdout.iter().filter(|&&b| b == b'\n').count(), 1);
    // Progress goes to stderr, not into the JSON.
    assert!(!out.stderr.is_empty());
}

#[test]
fn analyze_missing_path_exits_1_and_bad_flag_exits_2() {
    let (_t, project) = fake_project();
    let missing = project.join("nope");
    let out = xiox().arg("analyze").arg(&missing).env("XIOX_SEARCH_ROOTS", project.parent().unwrap()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error: path not found: "), "{err}");
    assert!(err.contains("nope"), "{err}");

    let out = xiox().args(["analyze", "--bogus"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown flag: --bogus"));

    let out = xiox().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = xiox().args(["clean", "--copies"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--include <path>"));
}

#[test]
fn help_and_version_exit_0() {
    let out = xiox().arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("xiox analyze") && s.contains("xiox clean"), "{s}");
    let out = xiox().arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("xiox {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn clean_is_not_implemented_yet_and_removes_nothing() {
    let (_t, project) = fake_project();
    let out = xiox().arg("clean").arg(&project).arg("--builds").arg("--yes").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(project.join("target").exists());
}
