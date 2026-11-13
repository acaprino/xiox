//! Cancellazione di un solo file, collegamento o cartella vuota, con i
//! controlli rifatti sullo stesso handle con cui si cancella.
//!
//! Fra la raccolta e la pulizia il disco può cambiare: una cartella del
//! percorso può diventare una giunzione verso un'altra parte del disco.
//! `delete_checked` apre la voce senza seguirne il collegamento finale,
//! chiede a Windows il percorso vero dell'handle e cancella solo se coincide
//! con quello atteso; la cancellazione avviene sullo stesso handle, quindi non
//! c'è un intervallo in cui il percorso possa cambiare di nuovo.

use std::path::Path;

use crate::guard::Guard;
use crate::model::Env;

/// Cancella `path` (un file, un collegamento o una cartella **vuota**) e
/// restituisce i byte liberati (0 per cartelle e collegamenti).
///
/// Passi: rifiuta i percorsi protetti; apre `path` con
/// `FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS` e accesso
/// `DELETE` (condivisione completa); verifica con `GetFinalPathNameByHandleW`
/// che il percorso finale sia uguale a `path` (nessun collegamento o
/// giunzione in un antenato) e non sia protetto; poi cancella attraverso lo
/// stesso handle (`FileDispositionInfoEx` con `DELETE | POSIX_SEMANTICS |
/// IGNORE_READONLY_ATTRIBUTE`, o il vecchio `FileDispositionInfo` dopo aver
/// tolto la sola lettura, sui file system che non lo conoscono). Un
/// collegamento o una giunzione si toglie come collegamento: la
/// destinazione non si tocca. Una cartella con qualcosa dentro non si
/// cancella (contratto di `Collected::dirs_to_remove`).
///
/// I file e i collegamenti passano da [`Guard::protected`], le cartelle (anche
/// le giunzioni) da [`Guard::protected_dir_removal`]. Costruisce un `Guard`
/// a ogni chiamata: per molte cancellazioni usare [`delete_checked_with`].
#[cfg(windows)]
pub fn delete_checked(path: &Path, env: &Env) -> Result<u64, String> {
    win::delete_checked(path, &Guard::new(env))
}

/// Come [`delete_checked`], con un `Guard` costruito una volta per pulizia.
#[cfg(windows)]
pub fn delete_checked_with(path: &Path, guard: &Guard) -> Result<u64, String> {
    win::delete_checked(path, guard)
}

/// Componenti confrontabili di un percorso assoluto con lettera di unità:
/// `/` come `\`, prefisso `\\?\` tolto, minuscolo, senza vuoti e `.`.
/// `None` se non è di quella forma o contiene `..` o un flusso (`:`).
fn comparable(path: &str) -> Option<Vec<String>> {
    let s = path.replace('/', "\\");
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let mut out: Vec<String> = Vec::new();
    for c in s.split('\\').filter(|c| !c.is_empty() && *c != ".") {
        if out.is_empty() {
            let b = c.as_bytes();
            if !(b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
                return None;
            }
        } else if c == ".." || c.contains(':') {
            return None;
        }
        out.push(c.to_lowercase());
    }
    (out.len() >= 2).then_some(out)
}

/// I due percorsi indicano la stessa voce (stessi componenti).
fn same_path(expected: &str, actual: &str) -> bool {
    match (comparable(expected), comparable(actual)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// File e collegamenti: `protected`; cartelle tolte esse stesse:
/// `protected_dir_removal`.
fn prot(path: &Path, guard: &Guard, is_dir: bool) -> Option<&'static str> {
    if is_dir {
        guard.protected_dir_removal(path)
    } else {
        guard.protected(path)
    }
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_DIR_NOT_EMPTY, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
        FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_WRITE_ATTRIBUTES, FileAttributeTagInfo, FileBasicInfo,
        FileDispositionInfo, FileDispositionInfoEx, FileStandardInfo,
        GetFileInformationByHandleEx, GetFinalPathNameByHandleW, OPEN_EXISTING,
        SetFileInformationByHandle, VOLUME_NAME_DOS,
    };

    use super::{prot, same_path};
    use crate::guard::Guard;

    /// Handle chiuso all'uscita.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: handle valido aperto da `CreateFileW`, chiuso una volta sola.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn delete_checked(path: &Path, guard: &Guard) -> Result<u64, String> {
        let shown = path.display();
        if let Some(why) = prot(path, guard, false) {
            return Err(format!("percorso protetto: {shown} ({why})"));
        }
        let text = path.to_string_lossy();
        if super::comparable(&text).is_none() {
            return Err(format!("percorso non assoluto o non valido: {shown}"));
        }
        let handle = open(path).map_err(|e| format!("non si apre {shown}: {e}"))?;
        let tag = attribute_tag(&handle).map_err(|e| format!("attributi di {shown}: {e}"))?;
        let is_link = tag.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        let is_dir = tag.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        let actual = final_path(&handle).map_err(|e| format!("percorso vero di {shown}: {e}"))?;
        if !same_path(&text, &actual.to_string_lossy()) {
            return Err(format!(
                "collegamento non seguito: {shown} porta a {}",
                actual.display()
            ));
        }
        if let Some(why) = prot(&actual, guard, is_dir) {
            return Err(format!("percorso protetto: {shown} ({why})"));
        }
        let size = if is_dir || is_link {
            0
        } else {
            standard_info(&handle)
                .map(|i| u64::try_from(i.end_of_file).unwrap_or(0))
                .unwrap_or(0)
        };
        dispose(&handle, tag.attributes).map_err(|code| {
            if code == ERROR_DIR_NOT_EMPTY {
                format!("cartella non vuota: {shown}")
            } else {
                format!("non si cancella {shown}: {}", std::io::Error::from_raw_os_error(code as i32))
            }
        })?;
        Ok(size)
    }

    /// `\\?\` davanti per i percorsi lunghi (il percorso è già assoluto con unità).
    fn wide(path: &Path) -> Vec<u16> {
        let text = path.to_string_lossy().replace('/', "\\");
        let full = if text.starts_with(r"\\?\") {
            text
        } else {
            format!(r"\\?\{text}")
        };
        std::ffi::OsStr::new(&full).encode_wide().chain(Some(0)).collect()
    }

    fn open(path: &Path) -> std::io::Result<Handle> {
        let name = wide(path);
        // SAFETY: `name` è una stringa UTF-16 terminata da zero che vive per
        // tutta la chiamata; gli altri argomenti sono costanti o nulli.
        let h = unsafe {
            CreateFileW(
                name.as_ptr(),
                DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Handle(h))
    }

    fn info<T: Default>(h: &Handle, class: i32) -> std::io::Result<T> {
        let mut out = T::default();
        // SAFETY: `out` è un `T` valido della dimensione passata; la classe
        // corrisponde al tipo (vedi chi chiama).
        let ok = unsafe {
            GetFileInformationByHandleEx(
                h.0,
                class,
                (&mut out as *mut T).cast(),
                std::mem::size_of::<T>() as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(out)
    }

    fn attribute_tag(h: &Handle) -> std::io::Result<TagInfo> {
        info::<TagInfo>(h, FileAttributeTagInfo)
    }

    fn standard_info(h: &Handle) -> std::io::Result<StandardInfo> {
        info::<StandardInfo>(h, FileStandardInfo)
    }

    /// Stessa disposizione di `FILE_ATTRIBUTE_TAG_INFO` e `FILE_STANDARD_INFO`,
    /// con `Default` per `info`.
    #[repr(C)]
    #[derive(Default)]
    struct TagInfo {
        attributes: u32,
        tag: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct StandardInfo {
        allocation: i64,
        end_of_file: i64,
        links: u32,
        delete_pending: u8,
        directory: u8,
    }

    fn final_path(h: &Handle) -> std::io::Result<PathBuf> {
        let mut buf = vec![0u16; 1024];
        loop {
            // SAFETY: `buf` ha `buf.len()` elementi scrivibili.
            let n = unsafe {
                GetFinalPathNameByHandleW(
                    h.0,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            } as usize;
            if n == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if n < buf.len() {
                return Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])));
            }
            buf.resize(n + 1, 0);
        }
    }

    /// Cancella attraverso l'handle. `Err(codice Win32)`.
    fn dispose(h: &Handle, attributes: u32) -> Result<(), u32> {
        for flags in [
            FILE_DISPOSITION_FLAG_DELETE
                | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
                | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
            // File system senza la semantica POSIX (FAT, exFAT…).
            FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        ] {
            let info = FILE_DISPOSITION_INFO_EX { Flags: flags };
            // SAFETY: `info` è una `FILE_DISPOSITION_INFO_EX` valida per la classe.
            let ok = unsafe {
                SetFileInformationByHandle(
                    h.0,
                    FileDispositionInfoEx,
                    (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
                )
            };
            if ok != 0 {
                return Ok(());
            }
            // SAFETY: nessun requisito.
            let code = unsafe { GetLastError() };
            if code == ERROR_DIR_NOT_EMPTY {
                return Err(code);
            }
        }
        // Vecchia via: si toglie la sola lettura, poi `DeleteFile = TRUE`.
        if attributes & FILE_ATTRIBUTE_READONLY != 0 {
            // 0 vorrebbe dire «non cambiare»: senza altri attributi, NORMAL.
            let rest = attributes & !FILE_ATTRIBUTE_READONLY;
            let basic = FILE_BASIC_INFO {
                FileAttributes: if rest == 0 { FILE_ATTRIBUTE_NORMAL } else { rest },
                ..Default::default()
            };
            // SAFETY: `basic` è una `FILE_BASIC_INFO` valida; i tempi a 0 non
            // si cambiano.
            unsafe {
                SetFileInformationByHandle(
                    h.0,
                    FileBasicInfo,
                    (&basic as *const FILE_BASIC_INFO).cast(),
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
            };
        }
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: `info` è una `FILE_DISPOSITION_INFO` valida per la classe.
        let ok = unsafe {
            SetFileInformationByHandle(
                h.0,
                FileDispositionInfo,
                (&info as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        if ok != 0 {
            Ok(())
        } else {
            // SAFETY: nessun requisito.
            Err(unsafe { GetLastError() })
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn env() -> Env {
        Env::default()
    }

    fn junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn normal_file_is_deleted_and_its_size_returned() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a.tmp");
        fs::write(&f, "12345").unwrap();
        assert_eq!(delete_checked(&f, &env()), Ok(5));
        assert!(!f.exists());
        // Ciò che non c'è più è un errore, non un panico.
        assert!(delete_checked(&f, &env()).is_err());
    }

    #[test]
    fn read_only_file_is_deleted() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("ro.tmp");
        fs::write(&f, "123").unwrap();
        let mut perm = fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&f, perm).unwrap();
        assert_eq!(delete_checked(&f, &env()), Ok(3));
        assert!(!f.exists());
    }

    #[test]
    fn parent_swapped_for_a_junction_is_refused_and_the_target_kept() {
        let t = tempfile::tempdir().unwrap();
        let other = t.path().join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("x.txt"), "precious").unwrap();
        // Raccolto come `cache\x.txt`; poi `cache` diventa una giunzione.
        let parent = t.path().join("cache");
        if !junction(&parent, &other) {
            println!("SKIP: mklink /J non riuscito");
            return;
        }
        let expected: PathBuf = parent.join("x.txt");
        let err = delete_checked(&expected, &env()).unwrap_err();
        assert!(err.starts_with("collegamento non seguito: "), "{err}");
        assert_eq!(fs::read_to_string(other.join("x.txt")).unwrap(), "precious");
    }

    #[test]
    fn protected_path_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("auth.json");
        fs::write(&f, "secret").unwrap();
        let err = delete_checked(&f, &env()).unwrap_err();
        assert!(err.starts_with("percorso protetto: "), "{err}");
        assert!(f.exists());
        let err = delete_checked(Path::new(r"\\host\share\x.tmp"), &env()).unwrap_err();
        assert!(err.starts_with("percorso protetto: "), "{err}");
    }

    #[test]
    fn only_empty_directories_are_deleted() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("d");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub").join("f.txt"), "1").unwrap();
        let err = delete_checked(&d.join("sub"), &env()).unwrap_err();
        assert!(err.starts_with("cartella non vuota: "), "{err}");
        assert!(d.join("sub").join("f.txt").exists());
        fs::remove_file(d.join("sub").join("f.txt")).unwrap();
        assert_eq!(delete_checked(&d.join("sub"), &env()), Ok(0));
        assert!(!d.join("sub").exists());
    }

    #[test]
    fn junction_itself_is_removed_and_its_target_kept() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep.txt"), "keep").unwrap();
        let link = t.path().join("link");
        if !junction(&link, &target) {
            println!("SKIP: mklink /J non riuscito");
            return;
        }
        assert_eq!(delete_checked(&link, &env()), Ok(0));
        assert!(std::fs::symlink_metadata(&link).is_err(), "la giunzione è rimasta");
        assert_eq!(fs::read_to_string(target.join("keep.txt")).unwrap(), "keep");
    }

    #[test]
    fn path_comparison_ignores_case_prefix_and_separators() {
        assert!(same_path(r"C:\Users\A\x.tmp", r"\\?\c:\users\a\X.TMP"));
        assert!(same_path("C:/Users/a/x", r"C:\Users\a\x\"));
        assert!(!same_path(r"C:\Users\a\cache\x", r"\\?\C:\Users\a\other\x"));
        assert!(!same_path(r"C:\a\x", r"\\?\UNC\host\share\x"));
        assert!(!same_path(r"C:\a\..\b", r"C:\b"));
        assert!(!same_path(r"C:\a\x:stream", r"C:\a\x"));
    }
}
