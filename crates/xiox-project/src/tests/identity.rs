//! The identity of a folder: stable while it exists, new when it is recreated, never followed.

#[cfg(windows)]
#[test]
fn folder_id_is_stable_and_changes_when_recreated() {
    use crate::identity::folder_id;

    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("d");
    std::fs::create_dir(&dir).unwrap();
    let first = folder_id(&dir).expect("an id for a folder");
    assert_eq!(folder_id(&dir), Some(first));

    let link = base.path().join("link");
    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if made {
        let through = folder_id(&link).expect("an id for the junction");
        assert_ne!(through, first, "the junction itself, not its target");
    } else {
        println!("skipped: no junction");
    }

    std::fs::remove_dir(&dir).unwrap();
    std::fs::create_dir(&dir).unwrap();
    assert_ne!(folder_id(&dir), Some(first), "a recreated folder is another folder");
}
