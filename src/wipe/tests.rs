use super::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use pc_rules::Env;

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;

// ----- finto -----

/// Volume finto: tiene il conto dello spazio libero e dei file di
/// riempimento (solo numeri, nessun byte vero scritto da nessuna parte).
struct FakeVol {
    root: PathBuf,
    fixed_ntfs: bool,
    kind: Result<DiskKind, String>,
    free: Cell<u64>,
    /// Cartelle nella radice: nome → byte di ogni file.
    dirs: RefCell<BTreeMap<String, Vec<u64>>>,
    /// Cartelle tolte con `remove_dir`, in ordine.
    removed: RefCell<Vec<String>>,
    created: RefCell<Vec<String>>,
    writes: Cell<u32>,
    biggest_write: Cell<u64>,
    lowest_free: Cell<u64>,
    /// La scrittura numero N (da 0) fallisce.
    fail_at: Option<u32>,
    /// La scrittura numero N (da 0) va in panico.
    panic_at: Option<u32>,
    /// Dopo N scritture riuscite si alza l'annullamento.
    cancel_after: Option<u32>,
    cancel: AtomicBool,
    /// Lo spazio libero non cala scrivendo (come un file compresso).
    free_never_shrinks: bool,
    create_dir_fails: bool,
}

impl FakeVol {
    fn new(kind: Result<DiskKind, String>, free: u64) -> FakeVol {
        FakeVol {
            root: PathBuf::from(r"C:\"),
            fixed_ntfs: true,
            kind,
            free: Cell::new(free),
            dirs: RefCell::new(BTreeMap::new()),
            removed: RefCell::new(Vec::new()),
            created: RefCell::new(Vec::new()),
            writes: Cell::new(0),
            biggest_write: Cell::new(0),
            lowest_free: Cell::new(free),
            fail_at: None,
            panic_at: None,
            cancel_after: None,
            cancel: AtomicBool::new(false),
            free_never_shrinks: false,
            create_dir_fails: false,
        }
    }

    fn hdd(free: u64) -> FakeVol {
        FakeVol::new(Ok(DiskKind::Hdd), free)
    }

    fn written(&self) -> u64 {
        self.dirs.borrow().values().flatten().sum()
    }
}

struct FakeFill<'a> {
    vol: &'a FakeVol,
    dir: String,
    idx: usize,
}

impl Fill for FakeFill<'_> {
    fn write_zeros(&mut self, len: u64) -> Result<(), String> {
        let v = self.vol;
        let n = v.writes.get();
        if v.panic_at == Some(n) {
            panic!("panico finto durante la scrittura");
        }
        if v.fail_at == Some(n) {
            return Err("errore finto di scrittura".into());
        }
        assert!(len > 0 && len.is_multiple_of(ALIGN), "scrittura non allineata: {len}");
        assert!(len <= BLOCK, "blocco oltre 64 MB: {len}");
        if len > v.free.get() {
            return Err("disco pieno".into());
        }
        if !v.free_never_shrinks {
            v.free.set(v.free.get() - len);
        }
        v.lowest_free.set(v.lowest_free.get().min(v.free.get()));
        v.biggest_write.set(v.biggest_write.get().max(len));
        v.dirs.borrow_mut().get_mut(&self.dir).expect("cartella creata")[self.idx] += len;
        v.writes.set(n + 1);
        if v.cancel_after == Some(n + 1) {
            v.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

impl Volume for FakeVol {
    fn root(&self) -> &Path {
        &self.root
    }
    fn is_fixed_ntfs(&self) -> bool {
        self.fixed_ntfs
    }
    fn kind(&self) -> Result<DiskKind, String> {
        self.kind.clone()
    }
    fn free_bytes(&self) -> Result<u64, String> {
        Ok(self.free.get())
    }
    fn leftover_dirs(&self) -> Result<Vec<String>, String> {
        Ok(self.dirs.borrow().keys().cloned().collect())
    }
    fn create_dir(&self, name: &str) -> Result<(), String> {
        if self.create_dir_fails {
            return Err("accesso negato".into());
        }
        assert!(self.dirs.borrow_mut().insert(name.to_string(), Vec::new()).is_none(), "cartella già esistente");
        self.created.borrow_mut().push(name.to_string());
        Ok(())
    }
    fn create_fill(&self, dir: &str, file: &str) -> Result<Box<dyn Fill + '_>, String> {
        assert!(is_fill_name(file), "{file}");
        let mut dirs = self.dirs.borrow_mut();
        let files = dirs.get_mut(dir).ok_or("cartella assente")?;
        files.push(0);
        Ok(Box::new(FakeFill { vol: self, dir: dir.to_string(), idx: files.len() - 1 }))
    }
    fn remove_dir(&self, name: &str) -> Result<u64, String> {
        let files = self.dirs.borrow_mut().remove(name).ok_or("cartella assente")?;
        let bytes: u64 = files.iter().sum();
        if !self.free_never_shrinks {
            self.free.set(self.free.get() + bytes);
        }
        self.removed.borrow_mut().push(name.to_string());
        Ok(bytes)
    }
}

fn found_for(vol: &FakeVol) -> NativeFound {
    NativeFound { count: 1, items: vec![vol.root.to_string_lossy().into_owned()], ..Default::default() }
}

fn clean(vol: &FakeVol) -> NativeCleaned {
    clean_wipe(vol, &found_for(vol), &vol.cancel)
}

// ----- nomi -----

#[test]
fn wipe_names_are_strict() {
    assert_eq!(wipe_dir_name(42, 7), "PulisciPC_wipe_42_7");
    assert!(is_wipe_dir_name("PulisciPC_wipe_42_7"));
    for bad in [
        "PulisciPC_wipe_",
        "PulisciPC_wipe_42",
        "PulisciPC_wipe_42_",
        "PulisciPC_wipe__7",
        "PulisciPC_wipe_a_7",
        "PulisciPC_wipe_42_7_8",
        r"PulisciPC_wipe_42_7\x",
        "pulisciPC_wipe_42_7",
        "Documenti",
    ] {
        assert!(!is_wipe_dir_name(bad), "{bad}");
    }
    assert_eq!(fill_name(1), "fill_00001.bin");
    assert!(is_fill_name("fill_00001.bin"));
    assert!(is_fill_name("fill_123456.bin"));
    for bad in ["fill_1.bin", "fill_00001.txt", "fill_0000a.bin", "Fill_00001.bin", "fill_00001.bin:x", "nota.txt"] {
        assert!(!is_fill_name(bad), "{bad}");
    }
}

// ----- voce -----

#[test]
fn item_is_off_by_default_with_warning() {
    let v = items();
    assert_eq!(v.len(), 1);
    let it = &v[0];
    assert_eq!(it.id, "wipe_free_space");
    assert_eq!(it.group, "Windows");
    assert_eq!(it.label, "Pulisci spazio libero");
    assert!(!it.default_on);
    assert!(it.needs_admin);
    assert_eq!(
        it.warning,
        Some(
            "Sovrascrive lo spazio libero perché i file cancellati non si possano recuperare. Può richiedere molto \
             tempo; durante la scrittura il disco resta quasi pieno (si lascia 1 GB libero)."
        )
    );
}

// ----- analisi -----

#[test]
fn ssd_is_refused_and_says_trim() {
    let vol = FakeVol::new(Ok(DiskKind::Ssd), 100 * GIB);
    let found = analyze_wipe(&vol);
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty());
    assert!(found.detail[0].contains("Non serve su SSD: Windows usa TRIM"), "{:?}", found.detail);
    assert!(found.detail[0].contains("100,00 GB"), "{:?}", found.detail);

    // Anche un `found` forzato non fa scrivere nulla su un SSD.
    let out = clean(&vol);
    assert_eq!(vol.writes.get(), 0);
    assert!(vol.created.borrow().is_empty());
    assert_eq!(out.removed, 0);
    assert!(out.notes.iter().any(|n| n.contains("TRIM")), "{:?}", out.notes);
}

#[test]
fn unknown_disk_type_or_not_fixed_ntfs_is_refused() {
    let vol = FakeVol::new(Err("IOCTL non supportato".into()), 100 * GIB);
    let found = analyze_wipe(&vol);
    assert_eq!(found.count, 0);
    assert!(found.detail[0].contains("tipo di disco non determinato"), "{:?}", found.detail);
    clean(&vol);
    assert_eq!(vol.writes.get(), 0);

    let mut vol = FakeVol::hdd(100 * GIB);
    vol.fixed_ntfs = false;
    let found = analyze_wipe(&vol);
    assert_eq!(found.count, 0);
    assert!(found.detail[0].contains("NTFS"), "{:?}", found.detail);
    clean(&vol);
    assert_eq!(vol.writes.get(), 0);
}

#[test]
fn below_two_gb_free_is_refused() {
    let vol = FakeVol::hdd(2 * GIB - 1);
    let found = analyze_wipe(&vol);
    assert_eq!(found.count, 0);
    assert!(found.detail[0].contains("sotto 2 GB"), "{:?}", found.detail);
    clean(&vol);
    assert_eq!(vol.writes.get(), 0);
    assert!(vol.created.borrow().is_empty());

    let vol = FakeVol::hdd(2 * GIB);
    assert_eq!(analyze_wipe(&vol).count, 1, "2 GB esatti bastano");
}

#[test]
fn hdd_analysis_shows_kind_and_free_space() {
    let vol = FakeVol::hdd(50 * GIB);
    let found = analyze_wipe(&vol);
    assert_eq!(found.count, 1);
    assert_eq!(found.items, vec![r"C:\".to_string()]);
    assert_eq!(found.bytes, 0, "non libera spazio");
    assert_eq!(found.files, 0);
    assert!(found.detail[0].contains("disco rigido"), "{:?}", found.detail);
    assert!(found.detail[0].contains("50,00 GB"), "{:?}", found.detail);
    assert_eq!(vol.writes.get(), 0, "l'analisi non scrive");
}

#[test]
fn leftover_from_a_previous_run_is_removed_by_analysis() {
    let vol = FakeVol::hdd(10 * GIB);
    vol.dirs.borrow_mut().insert("PulisciPC_wipe_123_456".into(), vec![3 * GIB, GIB]);
    vol.dirs.borrow_mut().insert("Documenti".into(), vec![5]);
    vol.dirs.borrow_mut().insert("PulisciPC_wipe_altro".into(), vec![5]);
    let found = analyze_wipe(&vol);
    assert_eq!(*vol.removed.borrow(), vec!["PulisciPC_wipe_123_456".to_string()]);
    assert_eq!(vol.free.get(), 14 * GIB);
    assert!(vol.dirs.borrow().contains_key("Documenti"));
    assert!(vol.dirs.borrow().contains_key("PulisciPC_wipe_altro"));
    assert!(
        found.detail.iter().any(|d| d.contains("PulisciPC_wipe_123_456") && d.contains("4,00 GB")),
        "{:?}",
        found.detail
    );
    // Lo spazio ritrovato conta già nell'analisi.
    assert!(found.detail[0].contains("14,00 GB"), "{:?}", found.detail);
}

// ----- pulizia -----

#[test]
fn fill_stops_leaving_one_gb_free_and_the_folder_goes_away() {
    let start = 5 * GIB + 37 * MIB + 123;
    let vol = FakeVol::hdd(start);
    let out = clean(&vol);
    assert!(vol.lowest_free.get() >= RESERVE, "sotto la riserva: {}", vol.lowest_free.get());
    assert!(vol.lowest_free.get() < RESERVE + ALIGN, "fermo troppo presto: {}", vol.lowest_free.get());
    assert!(vol.biggest_write.get() <= BLOCK);
    let created = vol.created.borrow().clone();
    assert_eq!(created.len(), 1);
    assert!(is_wipe_dir_name(&created[0]), "{}", created[0]);
    assert_eq!(*vol.removed.borrow(), created, "la cartella si cancella alla fine");
    assert!(vol.dirs.borrow().is_empty());
    assert_eq!(vol.free.get(), start, "spazio restituito");
    assert_eq!(out.removed, 1);
    assert_eq!(out.failed, 0);
    assert_eq!(out.freed, 0);
    assert!(out.notes.iter().any(|n| n.contains("4,04 GB")), "{:?}", out.notes);
    assert_eq!(out.done.len(), 1);
}

#[test]
fn fill_files_are_at_most_one_gb_each() {
    // Un volume finto che ricorda le dimensioni dei file subito prima di cancellarli.
    let vol = FakeVol::hdd(4 * GIB + 512 * MIB);
    let sizes: RefCell<Vec<u64>> = RefCell::new(Vec::new());
    struct Spy<'a>(&'a FakeVol, &'a RefCell<Vec<u64>>);
    impl Volume for Spy<'_> {
        fn root(&self) -> &Path {
            self.0.root()
        }
        fn is_fixed_ntfs(&self) -> bool {
            true
        }
        fn kind(&self) -> Result<DiskKind, String> {
            self.0.kind()
        }
        fn free_bytes(&self) -> Result<u64, String> {
            self.0.free_bytes()
        }
        fn leftover_dirs(&self) -> Result<Vec<String>, String> {
            self.0.leftover_dirs()
        }
        fn create_dir(&self, name: &str) -> Result<(), String> {
            self.0.create_dir(name)
        }
        fn create_fill(&self, dir: &str, file: &str) -> Result<Box<dyn Fill + '_>, String> {
            self.0.create_fill(dir, file)
        }
        fn remove_dir(&self, name: &str) -> Result<u64, String> {
            *self.1.borrow_mut() = self.0.dirs.borrow()[name].clone();
            self.0.remove_dir(name)
        }
    }
    let spy = Spy(&vol, &sizes);
    clean_wipe(&spy, &found_for(&vol), &AtomicBool::new(false));
    assert_eq!(*sizes.borrow(), vec![GIB, GIB, GIB, 512 * MIB]);
}

#[test]
fn free_space_that_does_not_shrink_never_writes_more_than_measured() {
    let mut vol = FakeVol::hdd(3 * GIB);
    vol.free_never_shrinks = true;
    let out = clean(&vol);
    assert_eq!(vol.written(), 0, "cartella già cancellata");
    assert!(u64::from(vol.writes.get()) * BLOCK <= 2 * GIB, "scritture: {}", vol.writes.get());
    assert_eq!(vol.writes.get(), 32);
    assert_eq!(out.removed, 1);
}

#[test]
fn cancel_midway_removes_the_folder() {
    let mut vol = FakeVol::hdd(20 * GIB);
    vol.cancel_after = Some(3);
    let out = clean(&vol);
    assert_eq!(vol.writes.get(), 3, "annullamento controllato a ogni blocco");
    assert_eq!(vol.removed.borrow().len(), 1);
    assert!(vol.dirs.borrow().is_empty());
    assert_eq!(vol.free.get(), 20 * GIB);
    assert_eq!(out.removed, 0);
    assert!(out.notes.iter().any(|n| n.contains("Annullata")), "{:?}", out.notes);
}

#[test]
fn cancel_before_start_creates_nothing() {
    let vol = FakeVol::hdd(20 * GIB);
    vol.cancel.store(true, Ordering::Relaxed);
    let out = clean(&vol);
    assert!(vol.created.borrow().is_empty());
    assert_eq!(out.removed + out.failed, 0);
}

#[test]
fn write_error_removes_the_folder() {
    let mut vol = FakeVol::hdd(20 * GIB);
    vol.fail_at = Some(2);
    let out = clean(&vol);
    assert_eq!(vol.removed.borrow().len(), 1);
    assert!(vol.dirs.borrow().is_empty());
    assert_eq!(vol.free.get(), 20 * GIB);
    assert_eq!(out.failed, 1);
    assert_eq!(out.removed, 0);
    assert!(out.notes.iter().any(|n| n.contains("errore finto di scrittura")), "{:?}", out.notes);
}

#[test]
fn panic_while_writing_still_removes_the_folder() {
    let mut vol = FakeVol::hdd(20 * GIB);
    vol.panic_at = Some(1);
    let r = catch_unwind(AssertUnwindSafe(|| clean(&vol)));
    assert!(r.is_err());
    assert_eq!(vol.removed.borrow().len(), 1, "il guard con Drop cancella la cartella");
    assert!(vol.dirs.borrow().is_empty());
    assert_eq!(vol.free.get(), 20 * GIB);
}

#[test]
fn create_dir_failure_touches_nothing() {
    let mut vol = FakeVol::hdd(20 * GIB);
    vol.create_dir_fails = true;
    let out = clean(&vol);
    assert_eq!(out.failed, 1);
    assert!(vol.removed.borrow().is_empty(), "una cartella non creata da noi non si cancella");
    assert!(out.notes.iter().any(|n| n.contains("accesso negato")), "{:?}", out.notes);
}

#[test]
fn clean_acts_only_on_the_volume_found_by_analysis() {
    let vol = FakeVol::hdd(20 * GIB);
    let other = NativeFound { count: 1, items: vec![r"D:\".to_string()], ..Default::default() };
    let out = clean_wipe(&vol, &other, &vol.cancel);
    assert!(vol.created.borrow().is_empty());
    assert_eq!(out.removed + out.failed, 0);
    let out = clean_wipe(&vol, &NativeFound::default(), &vol.cancel);
    assert!(vol.created.borrow().is_empty());
    assert_eq!(out.removed + out.failed, 0);

    // Fra analisi e pulizia lo spazio è sceso sotto 2 GB: ricontrollato, niente.
    let vol = FakeVol::hdd(20 * GIB);
    let found = analyze_wipe(&vol);
    vol.free.set(GIB);
    let out = clean_wipe(&vol, &found, &vol.cancel);
    assert!(vol.created.borrow().is_empty());
    assert!(out.notes.iter().any(|n| n.contains("sotto 2 GB")), "{:?}", out.notes);
}

// ----- file system vero, solo in una cartella temporanea con file minuscoli -----

fn tiny(p: &Path) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, b"zero").unwrap();
}

fn junction(link: &Path, target: &Path) {
    let st = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(st.success(), "mklink /J");
}

#[test]
fn real_listing_takes_only_true_wipe_folders() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("PulisciPC_wipe_1_2")).unwrap();
    fs::create_dir(t.path().join("PulisciPC_wipe_x")).unwrap();
    fs::create_dir(t.path().join("Altro")).unwrap();
    tiny(&t.path().join("PulisciPC_wipe_5_6"));
    junction(&t.path().join("PulisciPC_wipe_3_4"), &t.path().join("Altro"));
    assert_eq!(list_wipe_dirs_at(t.path()).unwrap(), vec!["PulisciPC_wipe_1_2".to_string()]);
}

#[test]
fn real_removal_deletes_only_fill_files_and_never_follows_links() {
    let t = tempfile::tempdir().unwrap();
    let guard = Guard::new(&Env::default());

    // Cartella con soli file di riempimento: via tutto.
    let d = t.path().join("PulisciPC_wipe_1_2");
    tiny(&d.join("fill_00001.bin"));
    tiny(&d.join("fill_00002.bin"));
    assert_eq!(remove_wipe_dir_at(t.path(), "PulisciPC_wipe_1_2", &guard), Ok(8));
    assert!(!d.exists());

    // Un file estraneo resta, e con lui la cartella.
    let d = t.path().join("PulisciPC_wipe_3_4");
    tiny(&d.join("fill_00001.bin"));
    tiny(&d.join("nota.txt"));
    fs::create_dir(d.join("sotto")).unwrap();
    let r = remove_wipe_dir_at(t.path(), "PulisciPC_wipe_3_4", &guard);
    let e = r.unwrap_err();
    assert!(e.contains("nota.txt") && e.contains("sotto"), "{e}");
    assert!(!d.join("fill_00001.bin").exists());
    assert!(d.join("nota.txt").exists());
    assert!(d.join("sotto").exists());

    // Una giunzione con il nome giusto non si segue.
    let target = t.path().join("Bersaglio");
    tiny(&target.join("fill_00001.bin"));
    junction(&t.path().join("PulisciPC_wipe_5_6"), &target);
    assert!(remove_wipe_dir_at(t.path(), "PulisciPC_wipe_5_6", &guard).is_err());
    assert!(target.join("fill_00001.bin").exists());

    // Nomi che non sono dei nostri: rifiutati prima di guardare il disco.
    tiny(&t.path().join("Documenti").join("fill_00001.bin"));
    assert!(remove_wipe_dir_at(t.path(), "Documenti", &guard).is_err());
    assert!(remove_wipe_dir_at(t.path(), r"PulisciPC_wipe_1_2\..\Documenti", &guard).is_err());
    assert!(t.path().join("Documenti").join("fill_00001.bin").exists());
}

// ----- dal vivo, sola lettura -----

/// Stampa tipo di disco, file system e spazio libero del volume di sistema.
/// Non crea, non scrive e non cancella nulla (neppure gli avanzi: solo
/// l'elenco).
#[test]
#[ignore]
fn live_wipe_analysis_read_only() {
    let guard = Guard::new(&Env::default());
    let Some(vol) = win::RealVolume::system(&guard) else {
        println!("volume di sistema non determinato");
        return;
    };
    println!("radice: {}", vol.root().display());
    println!("fisso NTFS: {}", vol.is_fixed_ntfs());
    println!("tipo: {:?}", vol.kind());
    println!("libero: {:?}", vol.free_bytes().map(crate::sys::fmt_size));
    println!("avanzi: {:?}", vol.leftover_dirs());
}
