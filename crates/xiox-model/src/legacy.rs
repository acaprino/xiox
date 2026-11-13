//! Names the product had before it was called Xiox, and the one rule for moving an existing
//! folder to its new name without ever putting the user's data at risk.
//!
//! Every folder the app owns (settings, caches, registry and service backups, the Winapp2
//! rules) used to live under `PulisciPC`. Installs that already have data there keep working:
//! the folder is renamed once to `Xiox` (same parent, so the same volume: one atomic call) and
//! when that is not possible the old folder simply stays in use. Nothing is ever copied and
//! then deleted, and a folder that exists under the new name is never touched.

use std::io;
use std::path::{Path, PathBuf};

/// Name of every folder the app owns.
pub const APP_DIR: &str = "Xiox";
/// The same folder before the rename.
pub const LEGACY_APP_DIR: &str = "PulisciPC";

/// The folder to use for `new`, an app folder that used to be called `old` (a sibling).
///
/// * `new` exists as a folder: it is used (the old one, if any, is left alone).
/// * only `old` exists (and `new` is free): it is renamed to `new` and `new` is used; **if the rename fails `old`
///   stays in use**, so the user never loses sight of their backups.
/// * neither exists: `new` (the caller creates it when it first writes).
pub fn migrate_legacy_dir(old: &Path, new: &Path) -> PathBuf {
    migrate_legacy_dir_with(old, new, |a, b| std::fs::rename(a, b))
}

/// [`migrate_legacy_dir`] with the rename passed in (tests make it fail on purpose).
pub fn migrate_legacy_dir_with(old: &Path, new: &Path, rename: impl FnOnce(&Path, &Path) -> io::Result<()>) -> PathBuf {
    if new.is_dir() {
        return new.to_path_buf();
    }
    if present(old) {
        // Something that is not a folder (a file, a broken link) already has the new name: the
        // operating system would replace it, so the rename is not even tried.
        if present(new) {
            return old.to_path_buf();
        }
        return match rename(old, new) {
            Ok(()) => new.to_path_buf(),
            Err(_) => old.to_path_buf(),
        };
    }
    new.to_path_buf()
}

/// The app folder `<base>\Xiox`, migrated from `<base>\PulisciPC` when needed
/// ([`migrate_legacy_dir`]). Call it before reading or writing anything in there.
pub fn app_dir_in(base: &Path) -> PathBuf {
    migrate_legacy_dir(&base.join(LEGACY_APP_DIR), &base.join(APP_DIR))
}

/// `true` if something (even a broken link) is at `p`.
fn present(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn put(dir: &Path, file: &str, text: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(file), text).unwrap();
    }

    #[test]
    fn new_folder_exists_it_is_used_and_the_old_one_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("PulisciPC"), t.path().join("Xiox"));
        put(&old, "backup_1.reg", "old");
        put(&new, "backup_2.reg", "new");
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert_eq!(fs::read_to_string(old.join("backup_1.reg")).unwrap(), "old");
        assert_eq!(fs::read_to_string(new.join("backup_2.reg")).unwrap(), "new");
        assert!(!new.join("backup_1.reg").exists(), "no merge, no copy");
    }

    #[test]
    fn only_the_old_folder_exists_it_is_renamed_with_all_its_content() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("PulisciPC"), t.path().join("Xiox"));
        put(&old, "backup_1.reg", "old");
        put(&old.join("sub"), "x.txt", "deep");
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert!(!old.exists());
        assert_eq!(fs::read_to_string(new.join("backup_1.reg")).unwrap(), "old");
        assert_eq!(fs::read_to_string(new.join("sub").join("x.txt")).unwrap(), "deep");
        // Second call: already migrated, nothing changes.
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert_eq!(fs::read_to_string(new.join("backup_1.reg")).unwrap(), "old");
    }

    #[test]
    fn rename_fails_the_old_folder_keeps_being_used_and_nothing_is_lost() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("PulisciPC"), t.path().join("Xiox"));
        put(&old, "backup_1.reg", "old");
        let got = migrate_legacy_dir_with(&old, &new, |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)));
        assert_eq!(got, old);
        assert!(!new.exists(), "no half-made new folder");
        assert_eq!(fs::read_to_string(old.join("backup_1.reg")).unwrap(), "old");
    }

    #[test]
    fn a_real_failing_rename_falls_back_to_the_old_folder() {
        let t = tempfile::tempdir().unwrap();
        let old = t.path().join("PulisciPC");
        put(&old, "backup_1.reg", "old");
        // The parent of the target does not exist: the operating system refuses the rename.
        let new = t.path().join("missing").join("Xiox");
        assert_eq!(migrate_legacy_dir(&old, &new), old);
        assert_eq!(fs::read_to_string(old.join("backup_1.reg")).unwrap(), "old");
    }

    #[test]
    fn neither_exists_the_new_name_is_used_and_nothing_is_created() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("PulisciPC"), t.path().join("Xiox"));
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert!(!old.exists() && !new.exists());
    }

    #[test]
    fn app_dir_in_migrates_under_the_base_folder() {
        let t = tempfile::tempdir().unwrap();
        put(&t.path().join("PulisciPC"), "settings.json", "{}");
        let d = app_dir_in(t.path());
        assert_eq!(d, t.path().join("Xiox"));
        assert_eq!(fs::read_to_string(d.join("settings.json")).unwrap(), "{}");
        assert!(!t.path().join("PulisciPC").exists());
    }

    #[test]
    fn a_file_with_the_new_name_is_not_replaced_and_the_old_folder_stays() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("PulisciPC"), t.path().join("Xiox"));
        put(&old, "backup_1.reg", "old");
        fs::write(&new, "a file").unwrap();
        // The name is taken by a file: it is never replaced, the old folder stays in use.
        assert_eq!(migrate_legacy_dir(&old, &new), old);
        assert_eq!(fs::read_to_string(&new).unwrap(), "a file");
        assert_eq!(fs::read_to_string(old.join("backup_1.reg")).unwrap(), "old");
    }
}
