//! Backup `.reg` sicuro, comune alla scheda Registro (`registry.rs`) e alle
//! regole di Pulizia file (`rules.rs`).
//!
//! - Esportazione con `%SystemRoot%\System32\reg.exe` (percorso assoluto da
//!   `GetSystemDirectoryW`, mai cercato nel `PATH` o nella cartella dell'exe),
//!   vista a 64 bit.
//! - I file temporanei vanno in una cartella di lavoro nuova, solo per
//!   amministratori e con nome casuale (`%ProgramData%\PulisciPC\tmp\<casuale>`,
//!   protetta con `updater::secure_dir`); se non si ottiene, il backup fallisce.
//! - Il file finale va nella cartella di destinazione (`Documenti\PulisciPC`)
//!   solo se è una cartella vera (né collegamento né giunzione), con
//!   `create_new`: `<stem>.reg`, poi `<stem>_2.reg`, `<stem>_3.reg`… mai
//!   sovrascritto.
//! - Una chiave che non si esporta fa fallire tutto (`Err`).

use std::path::{Path, PathBuf};

use pc_rules::RegRoot;

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
        export(*root, path, &tmp).map_err(|e| format!("{full} non esportata: {e}"))?;
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
    let md = std::fs::symlink_metadata(dir).map_err(|e| format!("cartella dei backup {}: {e}", dir.display()))?;
    if md.file_type().is_symlink() || md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !md.is_dir() {
        return Err(format!("cartella dei backup non valida (collegamento?): {}", dir.display()));
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
    Err("troppi backup con lo stesso nome".into())
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
    let reg = system_dir().map(|d| d.join("reg.exe")).filter(|p| p.is_file()).ok_or("reg.exe non trovato")?;
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
        Err(format!("reg.exe terminato con codice {}", st.code().unwrap_or(-1)))
    }
}

/// Cartella di lavoro nuova per l'esportazione: `%ProgramData%\PulisciPC\tmp\<casuale>`,
/// modificabile solo dagli amministratori (`updater::secure_dir`, che
/// rifiuta collegamenti nel percorso e verifica l'ACL). Se non si riesce
/// il backup fallisce: niente esportazioni in una cartella dell'utente.
pub fn admin_work_dir() -> Result<PathBuf, String> {
    let app = pc_rules::updater::data_dir();
    let base = app.parent().ok_or("cartella dati senza padre")?.join("tmp");
    pc_rules::updater::secure_dir(&base).map_err(|e| format!("cartella di lavoro non protetta: {e}"))?;
    let dir = base.join(random_name());
    if dir.exists() {
        return Err("cartella di lavoro già esistente".into());
    }
    pc_rules::updater::secure_dir(&dir).map_err(|e| format!("cartella di lavoro non protetta: {e}"))?;
    Ok(dir)
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
