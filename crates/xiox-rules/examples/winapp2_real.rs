//! Legge un `Winapp2.ini` vero (sola lettura) e stampa tempo, voci e picco di memoria del
//! processo, prima e dopo aver lasciato i dati. Non cancella e non scrive nulla.
//!
//!     cargo run -p xiox-rules --release --example winapp2_real -- <percorso di Winapp2.ini>
//!
//! Esce con 1 se la lettura supera 500 ms o il picco supera 60 MB (Fase 2, Task 10).

use xiox_rules::model::Env;
use xiox_rules::winapp2::{parse, to_cleaners};
use std::process::ExitCode;
use std::time::Instant;

const TIME_LIMIT_MS: u128 = 500;
const PEAK_LIMIT_MB: f64 = 60.0;

#[cfg(windows)]
fn mem_mb() -> (f64, f64) {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: buffer di sola scrittura della dimensione indicata; pseudo-handle del processo.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) == 0 {
            return (f64::NAN, f64::NAN);
        }
        let mb = 1024.0 * 1024.0;
        (c.WorkingSetSize as f64 / mb, c.PeakWorkingSetSize as f64 / mb)
    }
}

#[cfg(not(windows))]
fn mem_mb() -> (f64, f64) {
    (f64::NAN, f64::NAN)
}

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("uso: winapp2_real <Winapp2.ini>");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("non leggo {path}: {e}");
            return ExitCode::from(2);
        }
    };
    let (base_ws, _) = mem_mb();
    let env = Env::for_rules();

    let t = Instant::now();
    let (w, parse_warnings) = parse(&text);
    let parse_ms = t.elapsed().as_millis();
    let entries = w.entries.len();

    let t = Instant::now();
    let (cleaners, report) = to_cleaners(&w, &env);
    let convert_ms = t.elapsed().as_millis();
    let (ws_loaded, peak) = mem_mb();
    println!("file: {} byte, {} voci, {} avvisi di lettura", text.len(), entries, parse_warnings.len());
    println!("lettura {parse_ms} ms, conversione + rilevamento {convert_ms} ms, cleaner rilevati {}", cleaners.len());
    println!("report: {report:?}");
    println!("memoria: base {base_ws:.1} MB, dopo il caricamento {ws_loaded:.1} MB, picco {peak:.1} MB");

    drop(text);
    drop(w);
    let kept = cleaners.len();
    drop(cleaners);
    let (ws_after, peak_after) = mem_mb();
    println!("dopo il rilascio: {ws_after:.1} MB (picco {peak_after:.1} MB), tenuti {kept} cleaner");

    let total = parse_ms + convert_ms;
    let ok = total < TIME_LIMIT_MS && (peak.is_nan() || peak < PEAK_LIMIT_MB);
    println!("{}: totale {total} ms (limite {TIME_LIMIT_MS}), picco {peak:.1} MB (limite {PEAK_LIMIT_MB})", if ok { "OK" } else { "FUORI LIMITE" });
    if ok { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
