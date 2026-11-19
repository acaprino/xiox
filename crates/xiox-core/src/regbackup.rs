//! Backup `.reg` sicuro, comune alla scheda Registro (`registry.rs`) e alle
//! regole di Pulizia file (`rules.rs`).
//!
//! - Esportazione con `%SystemRoot%\System32\reg.exe` (percorso assoluto da
//!   `GetSystemDirectoryW`, mai cercato nel `PATH` o nella cartella dell'exe),
//!   vista a 64 bit.
//! - I file temporanei vanno in una cartella di lavoro nuova, solo per
//!   amministratori e con nome casuale (`%ProgramData%\Xiox\tmp\<casuale>`,
//!   protetta con `updater::secure_dir`); se non si ottiene, il backup fallisce.
//!   Un processo non elevato usa invece una cartella nuova dell'utente
//!   ([`work_dir`], [`user_work_dir`]).
//! - I backup con segreti (password FTP cifrate) vanno in
//!   `%LOCALAPPDATA%\Xiox\backup` con l'ACL solo per l'utente, SYSTEM e
//!   gli amministratori ([`private_backup_dir`]).
//! - Il file finale va nella cartella di destinazione (`Documenti\Xiox`)
//!   solo se è una cartella vera (né collegamento né giunzione), con
//!   `create_new`: `<stem>.reg`, poi `<stem>_2.reg`, `<stem>_3.reg`… mai
//!   sovrascritto.
//! - Una chiave che non si esporta fa fallire tutto (`Err`).

use std::path::{Path, PathBuf};

use xiox_rules::RegRoot;

/// Esportazione di una chiave `root\path` nel file `out` (formato di
/// `reg export`, UTF-16), sostituibile nei test.
pub type Export<'a> = dyn Fn(RegRoot, &str, &Path) -> Result<(), String> + 'a;

/// Cartella di lavoro nuova per le esportazioni, sostituibile nei test.
pub type WorkDir<'a> = dyn Fn() -> Result<PathBuf, String> + 'a;

/// Quanti nomi `<stem>_N.reg` si provano prima di arrendersi.
const MAX_BACKUP_NAMES: u32 = 1000;

pub fn root_name(root: RegRoot) -> &'static str {
    match root {
        RegRoot::Hkcu => "HKCU",
        RegRoot::Hklm => "HKLM",
    }
}

/// Scrive in `dest` un `.reg` con le chiavi `keys` (già filtrate da chi
/// chiama: solo quelle che esistono). Nessuna chiave: nessun file (`None`).
/// Vedi la documentazione del modulo per le garanzie.
pub fn write_backup(
    dest: &Path,
    stem: &str,
    work: &WorkDir,
    export: &Export,
    keys: &[(RegRoot, String)],
) -> Result<Option<PathBuf>, String> {
    let mut keys = keys.to_vec();
    keys.sort_by_key(|(r, p)| (root_name(*r), p.to_lowercase()));
    keys.dedup_by(|a, b| a.0 == b.0 && a.1.eq_ignore_ascii_case(&b.1));
    if keys.is_empty() {
        return Ok(None);
    }
    check_real_dir(dest)?;
    let dir = work()?;
    let mut tmp_files = Vec::new();
    let text = export_all(&keys, &dir, export, &mut tmp_files);
    for f in &tmp_files {
        // `remove_file` toglie un collegamento come collegamento.
        let _ = std::fs::remove_file(f);
    }
    let _ = std::fs::remove_dir(&dir);
    let text = text?;
    let mut data = vec![0xFF, 0xFE];
    for w in text.encode_utf16() {
        data.extend_from_slice(&w.to_le_bytes());
    }
    check_real_dir(dest)?;
    write_new(dest, stem, &data).map(Some)
}

fn export_all(
    keys: &[(RegRoot, String)],
    dir: &Path,
    export: &Export,
    tmp_files: &mut Vec<PathBuf>,
) -> Result<String, String> {
    let mut text = String::from("Windows Registry Editor Version 5.00\r\n");
    for (n, (root, path)) in keys.iter().enumerate() {
        let tmp = dir.join(format!("{n}.reg"));
        tmp_files.push(tmp.clone());
        let full = format!("{}\\{path}", root_name(*root));
        export(*root, path, &tmp).map_err(|e| format!("{full} not exported: {e}"))?;
        let bytes = std::fs::read(&tmp).map_err(|e| format!("{full}: {e}"))?;
        let words: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let s = String::from_utf16_lossy(&words);
        let s = s.trim_start_matches('\u{feff}');
        let body = s.split_once('\n').map(|x| x.1).unwrap_or("");
        text.push_str(body);
    }
    Ok(text)
}

/// `Ok` se `dir` è una cartella vera (non un collegamento né una giunzione).
fn check_real_dir(dir: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let md = std::fs::symlink_metadata(dir).map_err(|e| format!("backup folder {}: {e}", dir.display()))?;
    if md.file_type().is_symlink() || md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !md.is_dir() {
        return Err(format!("backup folder not valid (link?): {}", dir.display()));
    }
    Ok(())
}

/// Crea `<stem>.reg` (o `<stem>_N.reg`) in `dir` senza mai sovrascrivere.
fn write_new(dir: &Path, stem: &str, data: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    for n in 1..=MAX_BACKUP_NAMES {
        let name = if n == 1 { format!("{stem}.reg") } else { format!("{stem}_{n}.reg") };
        let path = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(data).map_err(|e| format!("{}: {e}", path.display()))?;
                // Su disco prima che chi chiama scriva nel registro.
                f.sync_all().map_err(|e| format!("{}: {e}", path.display()))?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Err("too many backups with the same name".into())
}

/// `%SystemRoot%\System32` da `GetSystemDirectoryW`.
fn system_dir() -> Option<PathBuf> {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buf = [0u16; 520];
    // SAFETY: buffer e dimensione validi.
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// L'esportazione vera: `reg.exe` di sistema (percorso assoluto), senza
/// finestra né shell, vista a 64 bit.
pub fn reg_exe_export(root: RegRoot, path: &str, out: &Path) -> Result<(), String> {
    let reg = system_dir().map(|d| d.join("reg.exe")).filter(|p| p.is_file()).ok_or("reg.exe not found")?;
    let full = format!("{}\\{path}", root_name(root));
    let mut cmd = std::process::Command::new(&reg);
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let st = cmd
        .args(["export", &full])
        .arg(out)
        .args(["/y", "/reg:64"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("reg.exe ended with code {}", st.code().unwrap_or(-1)))
    }
}

/// Cartella di lavoro nuova per l'esportazione: `%ProgramData%\Xiox\tmp\<casuale>`,
/// modificabile solo dagli amministratori (`updater::secure_dir`, che
/// rifiuta collegamenti nel percorso e verifica l'ACL). Se non si riesce
/// il backup fallisce: niente esportazioni in una cartella dell'utente.
pub fn admin_work_dir() -> Result<PathBuf, String> {
    let app = xiox_rules::updater::data_dir();
    let base = app.parent().ok_or("data folder without a parent")?.join("tmp");
    xiox_rules::updater::secure_dir(&base).map_err(|e| format!("work folder not protected: {e}"))?;
    let dir = base.join(random_name());
    if dir.exists() {
        return Err("work folder already exists".into());
    }
    xiox_rules::updater::secure_dir(&dir).map_err(|e| format!("work folder not protected: {e}"))?;
    Ok(dir)
}

/// Cartella di lavoro per l'esportazione adatta al processo: con i diritti
/// di amministratore [`admin_work_dir`] (un utente non elevato non deve poter
/// preparare collegamenti dove scrive un processo elevato); senza, una
/// cartella nuova dell'utente stesso ([`user_work_dir`]): lì nessun confine di
/// privilegi da difendere, e il backup del PATH utente si fa anche senza
/// elevazione.
pub fn work_dir() -> Result<PathBuf, String> {
    if crate::sys::is_admin() {
        admin_work_dir()
    } else {
        user_work_dir()
    }
}

/// `%LOCALAPPDATA%\Xiox\tmp\<casuale>` (cartella nota, non la variabile),
/// nuova, senza collegamenti nel percorso. Solo per un processo **non**
/// elevato (vedi [`work_dir`]).
pub fn user_work_dir() -> Result<PathBuf, String> {
    let local = crate::sysclean::local_app_data_dir().ok_or("AppData\\Local folder not found")?;
    user_work_dir_in(&local)
}

/// [`user_work_dir`] sotto la `AppData\Local` data.
pub fn user_work_dir_in(local: &Path) -> Result<PathBuf, String> {
    let base = local.join(xiox_model::APP_DIR).join("tmp");
    if has_link_in_chain(&base) {
        return Err("work folder behind a link".into());
    }
    std::fs::create_dir_all(&base).map_err(|e| format!("work folder {}: {e}", base.display()))?;
    check_real_dir(&base)?;
    let dir = base.join(random_name());
    // `create_dir` (non `_all`): fallisce se esiste già.
    std::fs::create_dir(&dir).map_err(|e| format!("work folder {}: {e}", dir.display()))?;
    check_real_dir(&dir)?;
    Ok(dir)
}

/// Cartella dei backup che contengono segreti (le password FTP cifrate):
/// `%LOCALAPPDATA%\Xiox\backup`, fuori da `Documenti` (spesso
/// sincronizzata con OneDrive) e non sincronizzata, con l'ACL ristretta
/// all'utente `user_sid`, a SYSTEM e agli amministratori ([`private_dir`]).
pub fn private_backup_dir(user_sid: &str) -> Result<PathBuf, String> {
    let local = crate::sysclean::local_app_data_dir().ok_or("AppData\\Local folder not found")?;
    let dir = local.join(xiox_model::APP_DIR).join("backup");
    private_dir(&dir, user_sid)?;
    Ok(dir)
}

/// Argomenti di `icacls` che lasciano `dir` solo all'utente `user_sid`, a
/// SYSTEM e agli amministratori: niente ereditarietà, controllo completo
/// (anche per file e sottocartelle) solo a loro tre, SID e mai nomi.
pub fn private_icacls_args(dir: &Path, user_sid: &str) -> Vec<std::ffi::OsString> {
    let mut a = vec![dir.as_os_str().to_os_string()];
    let user = format!("*{user_sid}:(OI)(CI)F");
    for s in ["/inheritance:r", "/grant:r", &user, "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F", "/Q"] {
        a.push(s.into());
    }
    a
}

/// Crea `dir` (se manca) e ne restringe l'ACL con l'`icacls.exe` di sistema
/// ([`private_icacls_args`]). Rifiuta un collegamento nel percorso; se
/// `icacls` non riesce: `Err` (il backup non si fa).
pub fn private_dir(dir: &Path, user_sid: &str) -> Result<(), String> {
    if user_sid.is_empty() || !user_sid.starts_with("S-1-") || !user_sid.bytes().all(|b| b.is_ascii_digit() || b == b'-' || b == b'S') {
        return Err(format!("invalid user SID: {user_sid}"));
    }
    if has_link_in_chain(dir) {
        return Err(format!("backup folder behind a link: {}", dir.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("backup folder {}: {e}", dir.display()))?;
    check_real_dir(dir)?;
    let icacls = system_dir().map(|d| d.join("icacls.exe")).filter(|p| p.is_file()).ok_or("icacls.exe not found")?;
    let mut cmd = std::process::Command::new(&icacls);
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let st = cmd
        .args(private_icacls_args(dir, user_sid))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("icacls.exe: {e}"))?;
    if !st.success() {
        return Err(format!("backup folder permissions not set (icacls, code {})", st.code().unwrap_or(-1)));
    }
    check_real_dir(dir)
}

/// `true` se `path` o una cartella che lo contiene (fra quelle che esistono)
/// è un collegamento o una giunzione.
fn has_link_in_chain(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    path.ancestors().any(|p| {
        std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0)
    })
}

/// Nome difficile da indovinare (hash casuale della libreria standard,
/// processo e ora).
fn random_name() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let a = h.finish();
    let mut h2 = std::collections::hash_map::RandomState::new().build_hasher();
    h2.write_u64(a);
    format!("reg-{a:016x}{:016x}", h2.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_work_dir_lives_under_xiox() {
        let t = tempfile::tempdir().unwrap();
        let dir = user_work_dir_in(t.path()).unwrap();
        assert!(dir.starts_with(t.path().join(r"Xiox\tmp")), "{}", dir.display());
        assert!(dir.is_dir());
    }

    #[test]
    fn private_dir_acl_is_only_user_system_and_administrators() {
        let args: Vec<String> = private_icacls_args(Path::new(r"C:\Users\me\AppData\Local\Xiox\backup"), "S-1-5-21-1-2-3-1001")
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                r"C:\Users\me\AppData\Local\Xiox\backup",
                "/inheritance:r",
                "/grant:r",
                "*S-1-5-21-1-2-3-1001:(OI)(CI)F",
                "*S-1-5-18:(OI)(CI)F",
                "*S-1-5-32-544:(OI)(CI)F",
                "/Q",
            ]
        );
        // Mai Users (S-1-5-32-545) né Everyone (S-1-1-0).
        assert!(!args.iter().any(|a| a.contains("S-1-5-32-545") || a.contains("S-1-1-0")));
    }

    #[test]
    fn private_dir_refuses_a_bad_sid_before_touching_the_disk() {
        let dir = Path::new(r"Z:\non\esiste\mai");
        for sid in ["", "Everyone", "S-1-5-21-1:(OI)(CI)F", "*S-1-1-0", "S-1-5 /grant"] {
            let e = private_dir(dir, sid).unwrap_err();
            assert!(e.contains("SID"), "{sid}: {e}");
        }
        assert!(!dir.exists());
    }
}
