//! Reads the MFT of C: and D:, prints records, folders, time and the process peak memory,
//! then compares the MFT size of three real `target` folders under D:\Projects with
//! `xiox_cleaner::measure_dir`. Needs an administrator terminal:
//!
//!     cargo run -p xiox-disktree --release --example peak [-- <target dir>...]
//!
//! Exit code 1 when a volume cannot be read, the peak is 300 MB or more, or a folder differs
//! by 1% or more.

use xiox_disktree::{DiskIndex, Progress, volume_index};
use xiox_model::DirIndex;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::time::Instant;

const PEAK_LIMIT_MB: f64 = 300.0;
const MAX_DIFF_PCT: f64 = 1.0;
const PROJECTS: &str = r"D:\Projects";

#[cfg(windows)]
fn is_admin() -> bool {
    // SAFETY: no arguments, no preconditions.
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// Peak working set of this process, in MB.
#[cfg(windows)]
fn peak_mb() -> f64 {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: plain-old-data out-buffer of the size passed; pseudo-handle of this process.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) == 0 {
            return f64::NAN;
        }
        c.PeakWorkingSetSize as f64 / (1024.0 * 1024.0)
    }
}

#[cfg(not(windows))]
fn is_admin() -> bool {
    false
}
#[cfg(not(windows))]
fn peak_mb() -> f64 {
    f64::NAN
}

/// First three `target` folders with a CACHEDIR.TAG under D:\Projects (depth <= 4), not the
/// one this example runs from.
fn find_targets() -> Vec<PathBuf> {
    let own = std::env::current_exe().ok().map(|p| xiox_model::path_key(&p)).unwrap_or_default();
    let mut found = Vec::new();
    let mut level = vec![PathBuf::from(PROJECTS)];
    for _ in 0..4 {
        let mut next = Vec::new();
        level.sort();
        for dir in level {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut subs: Vec<PathBuf> = rd
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect();
            subs.sort();
            for p in subs {
                let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                if name == "target" {
                    let key = xiox_model::path_key(&p);
                    if p.join("CACHEDIR.TAG").is_file() && !own.starts_with(&format!("{key}\\")) {
                        found.push(p);
                    }
                } else if name != "node_modules" && name != ".git" {
                    next.push(p);
                }
            }
        }
        level = next;
    }
    found.truncate(3);
    found
}

fn read(letter: char, progress: &Progress) -> Option<DiskIndex> {
    let before = progress.records.load(Ordering::Relaxed);
    let t = Instant::now();
    match volume_index(letter, progress) {
        Ok(ix) => {
            println!(
                "{letter}: record {}  cartelle {}  tempo {:.2} s  picco finora {:.1} MB",
                progress.records.load(Ordering::Relaxed) - before,
                ix.dir_count(),
                t.elapsed().as_secs_f64(),
                peak_mb()
            );
            Some(ix)
        }
        Err(e) => {
            println!("{letter}: lettura MFT non riuscita: {e}");
            None
        }
    }
}

fn mb(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn main() -> ExitCode {
    let mut ok = true;
    let admin = is_admin();
    if !admin {
        println!(
            "Questo terminale non è da amministratore: la lettura della MFT (\\\\.\\C:) verrà rifiutata.\n\
             Per la misura vera: terminale da amministratore, poi\n    \
             cargo run -p xiox-disktree --release --example peak"
        );
    }
    let progress = Progress::default();
    println!("picco all'avvio {:.1} MB", peak_mb());

    let mut index: Option<DiskIndex> = None;
    for letter in ['C', 'D'] {
        match (read(letter, &progress), index.as_mut()) {
            (Some(ix), Some(all)) => all.merge(ix),
            (Some(ix), None) => index = Some(ix),
            (None, _) => ok = false,
        }
    }
    let peak = peak_mb();
    println!("picco dopo C: e D: {peak:.1} MB (limite {PEAK_LIMIT_MB} MB)");
    if peak >= PEAK_LIMIT_MB {
        println!("PICCO SOPRA IL LIMITE");
        ok = false;
    }

    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let targets = if args.is_empty() { find_targets() } else { args };
    if targets.is_empty() {
        println!("nessuna cartella target trovata sotto {PROJECTS}");
        ok = false;
    }
    for t in &targets {
        let t0 = Instant::now();
        let walk = xiox_cleaner::measure_dir(t);
        let walk_alloc = walk.allocated_bytes.unwrap_or(0);
        let walk_s = t0.elapsed().as_secs_f64();
        let mft = index.as_ref().and_then(|ix| ix.size_of(Path::new(t)));
        match mft {
            Some(m) => {
                let mft_alloc = m.allocated_bytes.unwrap_or(0);
                let diff = if walk_alloc == 0 {
                    if mft_alloc == 0 { 0.0 } else { 100.0 }
                } else {
                    (mft_alloc as f64 - walk_alloc as f64).abs() * 100.0 / walk_alloc as f64
                };
                let verdict = if diff < MAX_DIFF_PCT { "ok" } else { "DIFFERENZA TROPPO ALTA" };
                if diff >= MAX_DIFF_PCT {
                    ok = false;
                }
                println!(
                    "{}\n    allocati MFT {:.1} MB, misura {:.1} MB ({walk_s:.1} s): diff {diff:.3}% {verdict}\n    \
                     logici MFT {:.1} MB / misura {:.1} MB, file MFT {} / misura {}",
                    t.display(),
                    mb(mft_alloc),
                    mb(walk_alloc),
                    mb(m.logical_bytes),
                    mb(walk.logical_bytes),
                    m.file_count,
                    walk.file_count
                );
            }
            None => {
                println!(
                    "{}\n    allocati MFT n/d, misura {:.1} MB ({walk_s:.1} s), {} file",
                    t.display(),
                    mb(walk_alloc),
                    walk.file_count
                );
                ok = false;
            }
        }
    }
    println!("picco finale {:.1} MB", peak_mb());
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
