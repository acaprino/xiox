use crate::measure_bytes;
use std::fs;
use std::path::Path;
use std::time::Duration;

const LONG: Duration = Duration::from_secs(30);

fn write(path: &Path, len: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; len]).unwrap();
}

#[test]
fn sums_nested_files() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.bin"), 100);
    write(&dir.path().join("sub/b.bin"), 200);
    write(&dir.path().join("sub/deeper/c.bin"), 300);
    fs::create_dir_all(dir.path().join("empty")).unwrap();
    assert_eq!(measure_bytes(dir.path(), 1000, LONG), Some(600));
    // A plain file is its own size.
    assert_eq!(measure_bytes(&dir.path().join("a.bin"), 1000, LONG), Some(100));
}

#[test]
fn does_not_follow_a_junction_or_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("big.bin"), 5000);
    write(&dir.path().join("own.bin"), 10);
    let link = dir.path().join("link");
    if !make_dir_link(outside.path(), &link) {
        eprintln!("skipped: this account may not create a junction or a directory symlink here");
        return;
    }
    assert_eq!(measure_bytes(dir.path(), 1000, LONG), Some(10), "the target of the link must not count");
    // A link as the root is not followed either.
    assert_eq!(measure_bytes(&link, 1000, LONG), Some(0));
}

#[cfg(windows)]
fn make_dir_link(target: &Path, link: &Path) -> bool {
    let junction = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(link)
        .arg(target)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    junction || std::os::windows::fs::symlink_dir(target, link).is_ok()
}

#[cfg(not(windows))]
fn make_dir_link(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[test]
fn entry_cap_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..10 {
        write(&dir.path().join(format!("f{n}.bin")), 1);
    }
    assert_eq!(measure_bytes(dir.path(), 5, LONG), None);
    assert_eq!(measure_bytes(dir.path(), 10, LONG), Some(10), "exactly the cap still fits");
    assert_eq!(measure_bytes(dir.path(), 100, LONG), Some(10));
}

#[test]
fn time_cap_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.bin"), 1);
    write(&dir.path().join("sub/b.bin"), 1);
    assert_eq!(measure_bytes(dir.path(), 1000, Duration::from_millis(0)), None);
}

#[test]
fn missing_root_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(measure_bytes(&dir.path().join("nope"), 1000, LONG), None);
}
