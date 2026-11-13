use crate::{Measured, measure_bytes};
use std::fs;
use std::path::Path;
use std::time::Duration;

const LONG: Duration = Duration::from_secs(30);

fn whole(bytes: u64) -> Option<Measured> {
    Some(Measured { bytes, complete: true })
}

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
    assert_eq!(measure_bytes(dir.path(), 1000, LONG), whole(600));
    // A plain file is its own size.
    assert_eq!(measure_bytes(&dir.path().join("a.bin"), 1000, LONG), whole(100));
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
    assert_eq!(measure_bytes(dir.path(), 1000, LONG), whole(10), "the target of the link must not count");
    // A link as the root is not followed either.
    assert_eq!(measure_bytes(&link, 1000, LONG), whole(0));
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
fn entry_cap_returns_the_partial_total_as_a_lower_bound() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..10 {
        write(&dir.path().join(format!("f{n}.bin")), 1);
    }
    // Over the cap: what was counted so far, flagged incomplete.
    let partial = measure_bytes(dir.path(), 5, LONG).expect("a lower bound, not nothing");
    assert!(!partial.complete);
    assert_eq!(partial.bytes, 5, "the first five entries were counted");
    assert_eq!(measure_bytes(dir.path(), 10, LONG), whole(10), "exactly the cap still fits");
    assert_eq!(measure_bytes(dir.path(), 100, LONG), whole(10));
}

#[test]
fn the_partial_total_never_exceeds_the_whole() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.bin"), 100);
    write(&dir.path().join("sub/b.bin"), 200);
    write(&dir.path().join("sub/deeper/c.bin"), 300);
    for cap in 0..6 {
        match measure_bytes(dir.path(), cap, LONG) {
            Some(m) if !m.complete => assert!(m.bytes <= 600, "cap {cap}: {m:?}"),
            Some(m) => assert_eq!(m.bytes, 600, "cap {cap}"),
            None => panic!("a readable root always gives a lower bound (cap {cap})"),
        }
    }
}

#[test]
fn time_cap_returns_a_partial_total_too() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.bin"), 1);
    write(&dir.path().join("sub/b.bin"), 1);
    let m = measure_bytes(dir.path(), 1000, Duration::from_millis(0)).expect("a lower bound");
    assert!(!m.complete);
    assert!(m.bytes <= 2);
}

#[test]
fn missing_root_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(measure_bytes(&dir.path().join("nope"), 1000, LONG), None);
}
