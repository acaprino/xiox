//! Prova da amministratore dell'aggiornamento di Winapp2 contro la rete e la cartella vere
//! (`%ProgramData%\PulisciPC\winapp2`): è il primo avvio del prodotto, senza interfaccia.
//! Serve un terminale elevato. Scrive solo nella cartella delle regole dell'app.
//!
//!     cargo run -p pc-rules --release --example update_real -- [file di resoconto]
//!
//! Controlla, in ordine: 1) scarico (o già aggiornato), 2) lettura della copia valida,
//! 3) secondo giro = «già aggiornato» (ETag), 4) rete irraggiungibile = si tiene la copia
//! vecchia, 5) la cartella è scrivibile solo da amministratori. Esce con 1 se qualcosa manca.

use pc_rules::updater::{
    AdminOnly, CurlFetch, UpdateOutcome, WINAPP2_URL, WORK_NAME, check_admin_only, data_dir, read_valid, update,
};
use std::fmt::Write as _;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::Instant;

fn is_admin() -> bool {
    // SAFETY: nessun argomento, nessuna precondizione.
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0
    }
    #[cfg(not(windows))]
    false
}

fn main() -> ExitCode {
    let mut out = String::new();
    let mut ok = true;
    macro_rules! say {
        ($($t:tt)*) => {{ let _ = writeln!(out, $($t)*); println!($($t)*); }};
    }
    macro_rules! check {
        ($cond:expr, $what:expr) => {{
            let c = $cond;
            say!("[{}] {}", if c { "ok" } else { "ERRORE" }, $what);
            ok &= c;
        }};
    }

    if !is_admin() {
        say!("Serve un terminale da amministratore.");
        finish(&out);
        return ExitCode::from(2);
    }
    let dir = data_dir();
    say!("cartella delle regole: {}", dir.display());
    let fetch = CurlFetch::new(dir.join(WORK_NAME), Arc::new(AdminOnly));

    let t = Instant::now();
    let first = update(&dir, &fetch, WINAPP2_URL);
    say!("1) primo giro ({} ms): {first:?}", t.elapsed().as_millis());
    check!(matches!(first, UpdateOutcome::Updated { .. } | UpdateOutcome::UpToDate), "scarico o già aggiornato");

    match read_valid(&dir) {
        Ok((text, version)) => say!("2) copia valida letta: versione {version}, {} byte", text.len()),
        Err(e) => {
            say!("2) copia NON valida: {e}");
            ok = false;
        }
    }
    check!(read_valid(&dir).is_ok(), "read_valid accetta la copia (cartella solo per amministratori)");

    let second = update(&dir, &fetch, WINAPP2_URL);
    say!("3) secondo giro: {second:?}");
    check!(matches!(second, UpdateOutcome::UpToDate), "secondo giro = già aggiornato (ETag)");

    let offline = update(&dir, &fetch, "https://invalid.invalid/Winapp2.ini");
    say!("4) rete irraggiungibile: {offline:?}");
    check!(matches!(offline, UpdateOutcome::KeptOld(_)), "con la rete assente si tiene la copia vecchia");
    check!(read_valid(&dir).is_ok(), "la copia vecchia è ancora valida");

    let acl = check_admin_only(&dir);
    say!("5) controllo dei permessi: {acl:?}");
    check!(acl.is_ok(), "la cartella non è scrivibile da utenti senza privilegi");
    if let Ok(o) = Command::new(r"C:\Windows\System32\icacls.exe").arg(&dir).output() {
        say!("icacls:\n{}", String::from_utf8_lossy(&o.stdout));
    }

    say!("{}", if ok { "TUTTO OK" } else { "ALMENO UNA PROVA È FALLITA" });
    finish(&out);
    if ok { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

/// Se è stato dato un file di resoconto lo scrive (utile quando la finestra elevata si chiude).
fn finish(out: &str) {
    if let Some(p) = std::env::args().nth(1) {
        let _ = std::fs::write(p, out);
    }
}
