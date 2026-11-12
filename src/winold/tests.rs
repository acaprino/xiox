//! Test di N7 con file system, registro e `cleanmgr` finti: nessun test
//! scrive nel registro vero né lancia `cleanmgr.exe`. L'unico test sul PC
//! vero (`live_winold_analysis_read_only`, `#[ignore]`) legge soltanto la
//! dimensione di `C:\Windows.old`.

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

fn win_old() -> PathBuf {
    PathBuf::from(r"C:\Windows.old")
}

fn file(name: &str, size: u64) -> Entry {
    Entry { name: name.into(), dir: false, reparse: false, size }
}

fn dir(name: &str) -> Entry {
    Entry { name: name.into(), dir: true, reparse: false, size: 0 }
}

/// Stato condiviso fra il sistema finto e il `cleanmgr` finto avviato.
#[derive(Default)]
struct Shared {
    log: RefCell<Vec<String>>,
    exited: Cell<bool>,
}

impl Shared {
    fn push(&self, s: &str) {
        self.log.borrow_mut().push(s.to_string());
    }
}

type Tree = HashMap<PathBuf, Result<Vec<Entry>, String>>;

/// File system, registro e `cleanmgr` finti in un solo oggetto.
struct Fake {
    s: Rc<Shared>,
    drive: Option<PathBuf>,
    /// `Windows.old` prima e dopo la fine di `cleanmgr`.
    before: Kind,
    after: Kind,
    tree: Tree,
    tree_after: Tree,
    flagged: Result<Vec<String>, String>,
    set_err: Option<String>,
    remove_err: Option<String>,
    available_err: Option<String>,
    start_err: Option<String>,
    /// Risposte `None` di `poll` prima della fine.
    polls_before_exit: u32,
    exit_code: i32,
    poll_panics: bool,
    /// Il compagno `PulisciPC_StateFlags0099` sotto `Previous Installations`.
    marker: Cell<bool>,
}

fn base() -> Fake {
    let mut tree = Tree::new();
    tree.insert(win_old(), Ok(vec![file("a.dat", 100), dir("Windows")]));
    tree.insert(win_old().join("Windows"), Ok(vec![file("b.dll", 50)]));
    Fake {
        s: Rc::new(Shared::default()),
        drive: Some(PathBuf::from(r"C:\")),
        before: Kind::Dir,
        after: Kind::Missing,
        tree,
        tree_after: Tree::new(),
        flagged: Ok(Vec::new()),
        set_err: None,
        remove_err: None,
        available_err: None,
        start_err: None,
        polls_before_exit: 2,
        exit_code: 0,
        poll_panics: false,
        marker: Cell::new(false),
    }
}

impl Fake {
    /// Le chiamate a registro e processo, in ordine (senza `kind`/`list`).
    fn calls(&self) -> Vec<String> {
        self.s.log.borrow().iter().filter(|c| !c.starts_with("kind") && !c.starts_with("list")).cloned().collect()
    }
    fn listed(&self) -> Vec<String> {
        self.s.log.borrow().iter().filter_map(|c| c.strip_prefix("list:").map(str::to_string)).collect()
    }
}

impl Files for Fake {
    fn system_drive(&self) -> Option<PathBuf> {
        self.drive.clone()
    }
    fn kind(&self, p: &Path) -> Kind {
        self.s.push("kind");
        if p.as_os_str().to_string_lossy().eq_ignore_ascii_case(&win_old().to_string_lossy()) {
            if self.s.exited.get() { self.after } else { self.before }
        } else {
            let tree = if self.s.exited.get() { &self.tree_after } else { &self.tree };
            if matches!(tree.get(p), Some(Ok(_))) { Kind::Dir } else { Kind::Missing }
        }
    }
    fn list(&self, d: &Path) -> Result<Vec<Entry>, String> {
        self.s.push(&format!("list:{}", d.display()));
        let tree = if self.s.exited.get() { &self.tree_after } else { &self.tree };
        tree.get(d).cloned().unwrap_or_else(|| Err("non trovata".into()))
    }
}

impl VolumeCaches for Fake {
    fn flagged(&self) -> Result<Vec<String>, String> {
        self.s.push("flagged");
        self.flagged.clone()
    }
    fn set_flag(&self) -> Result<(), String> {
        self.s.push("set");
        self.set_err.clone().map_or(Ok(()), Err)
    }
    fn remove_flag(&self) -> Result<(), String> {
        self.s.push("remove");
        self.remove_err.clone().map_or(Ok(()), Err)
    }
    fn has_marker(&self) -> Result<bool, String> {
        self.s.push("has_marker");
        Ok(self.marker.get())
    }
    fn set_marker(&self) -> Result<(), String> {
        self.s.push("set_marker");
        self.marker.set(true);
        Ok(())
    }
    fn remove_marker(&self) -> Result<(), String> {
        self.s.push("remove_marker");
        self.marker.set(false);
        Ok(())
    }
}

impl Cleanmgr for Fake {
    fn available(&self) -> Result<(), String> {
        self.s.push("available");
        self.available_err.clone().map_or(Ok(()), Err)
    }
    fn start(&self) -> Result<Box<dyn Child>, String> {
        self.s.push("start");
        if let Some(e) = &self.start_err {
            return Err(e.clone());
        }
        Ok(Box::new(FakeChild {
            s: self.s.clone(),
            left: self.polls_before_exit,
            code: self.exit_code,
            panics: self.poll_panics,
        }))
    }
}

struct FakeChild {
    s: Rc<Shared>,
    left: u32,
    code: i32,
    panics: bool,
}

impl Child for FakeChild {
    fn poll(&mut self) -> Result<Option<i32>, String> {
        self.s.push("poll");
        if self.panics {
            panic!("panico finto durante l'attesa");
        }
        if self.left == 0 {
            self.s.exited.set(true);
            return Ok(Some(self.code));
        }
        self.left -= 1;
        Ok(None)
    }
}

/// Orologio finto: ogni lettura avanza di `step`.
fn ticking(step: Duration) -> impl Fn() -> Duration {
    let t = Cell::new(Duration::ZERO);
    move || {
        t.set(t.get() + step);
        t.get()
    }
}

fn analyze_fake(f: &Fake) -> NativeFound {
    let cancel = AtomicBool::new(false);
    analyze(f, &ticking(Duration::from_millis(1)), &cancel)
}

fn clean_fake(f: &Fake, found: &NativeFound) -> NativeCleaned {
    let cancel = AtomicBool::new(false);
    clean_fake_with(f, found, &cancel, &|_| {})
}

fn clean_fake_with(f: &Fake, found: &NativeFound, cancel: &AtomicBool, sleep: &dyn Fn(Duration)) -> NativeCleaned {
    let clock = ticking(Duration::from_millis(1));
    let d = Deps { fs: f, reg: f, cleanmgr: f, clock: &clock, sleep };
    clean(&d, found, cancel)
}

/// `NativeFound` come lo darebbe l'analisi, senza chiamarla.
fn forced(items: &[&str]) -> NativeFound {
    NativeFound { count: items.len() as u64, items: items.iter().map(|s| s.to_string()).collect(), ..Default::default() }
}

// ---------------------------------------------------------------------------
// Voce.

#[test]
fn item_is_off_with_warning_needs_admin_and_waits_for_cleanmgr() {
    let v = items();
    assert_eq!(v.len(), 1);
    let i = &v[0];
    assert_eq!(i.id, "windows_old");
    assert_eq!(i.group, "Windows");
    assert_eq!(i.label, "Vecchia installazione di Windows");
    assert!(!i.default_on);
    assert!(i.needs_admin);
    // Si elimina solo dopo aver visto Windows.old e la sua dimensione nell'Analisi.
    assert!(i.needs_preview);
    assert_eq!(i.warning, Some(WARNING));
    assert!(WARNING.contains("tornare indietro"));
    assert!(WARNING.contains(r"Windows.old\Users") && WARNING.contains("file personali"), "{WARNING}");
    assert!(i.running.iter().any(|p| p.eq_ignore_ascii_case("cleanmgr.exe")), "{:?}", i.running);
    assert!(!i.description.contains("BleachBit") && !i.description.is_empty());
}

// ---------------------------------------------------------------------------
// Analisi.

#[test]
fn path_is_windows_old_in_the_system_drive_root_only() {
    let mut f = base();
    assert_eq!(windows_old_path(&f), Some(win_old()));
    f.drive = Some(PathBuf::from(r"d:\"));
    assert_eq!(windows_old_path(&f), Some(PathBuf::from(r"d:\Windows.old")));
    for bad in [r"\\server\share\", r"C:\Windows", r"C:", r"\\?\C:\", "relativo"] {
        f.drive = Some(PathBuf::from(bad));
        assert_eq!(windows_old_path(&f), None, "{bad}");
    }
    f.drive = None;
    assert_eq!(windows_old_path(&f), None);
}

#[test]
fn analysis_finds_nothing_without_windows_old() {
    let mut f = base();
    f.before = Kind::Missing;
    let found = analyze_fake(&f);
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty() && found.detail.is_empty());
    assert!(f.listed().is_empty());
    f.drive = None;
    assert_eq!(analyze_fake(&f).count, 0);
}

#[test]
fn analysis_never_proposes_a_link_a_file_or_an_unreadable_path() {
    for k in [Kind::Reparse, Kind::Other, Kind::Unreadable] {
        let mut f = base();
        f.before = k;
        let found = analyze_fake(&f);
        assert_eq!(found.count, 0, "{k:?}");
        assert!(found.items.is_empty(), "{k:?}");
        assert_eq!(found.skipped, 1, "{k:?}");
        assert!(found.detail[0].contains("non si tocca"), "{:?}", found.detail);
        assert!(f.listed().is_empty(), "{k:?}");
    }
}

#[test]
fn analysis_sums_files_and_never_enters_reparse_dirs() {
    let mut f = base();
    f.tree.insert(
        win_old(),
        Ok(vec![
            file("a.dat", 100),
            dir("Windows"),
            Entry { name: "Documents and Settings".into(), dir: true, reparse: true, size: 0 },
            Entry { name: "compresso.dll".into(), dir: false, reparse: true, size: 7 },
        ]),
    );
    f.tree.insert(win_old().join("Documents and Settings"), Ok(vec![file("enorme", 1_000_000)]));
    let found = analyze_fake(&f);
    assert_eq!(found.count, 1);
    assert_eq!(found.files, 0);
    assert_eq!(found.bytes, 157);
    assert_eq!(found.items, vec![r"C:\Windows.old".to_string()]);
    assert!(found.detail[0].starts_with(r"C:\Windows.old: circa"), "{:?}", found.detail);
    assert!(found.detail.iter().any(|l| l.contains("Pulizia disco")), "{:?}", found.detail);
    assert!(!f.listed().iter().any(|p| p.contains("Documents and Settings")), "{:?}", f.listed());
}

#[test]
fn analysis_stops_after_the_time_budget_and_says_oltre() {
    let mut f = base();
    f.tree.insert(win_old(), Ok(vec![file("a", 100), dir("d1")]));
    f.tree.insert(win_old().join("d1"), Ok(vec![file("b", 50), dir("d2")]));
    f.tree.insert(win_old().join("d1").join("d2"), Ok(vec![file("c", 25)]));
    let cancel = AtomicBool::new(false);
    // 6 s a lettura: la radice si legge (6 s), `d1` no (12 s ≥ 10 s).
    let found = analyze(&f, &ticking(Duration::from_secs(6)), &cancel);
    assert_eq!(found.bytes, 100);
    assert_eq!(found.count, 1, "la cartella c'è: si propone anche con la stima parziale");
    assert!(found.detail[0].contains("oltre"), "{:?}", found.detail);
    assert!(found.detail[0].contains("10 s"), "{:?}", found.detail);
    // Si legge solo la radice (più Users e la radice del disco, fuori dal conto della dimensione).
    let measured: Vec<_> = f.listed().into_iter().filter(|p| !p.ends_with("Users") && p != r"C:\").collect();
    assert_eq!(measured.len(), 1, "{:?}", f.listed());
}

#[test]
fn unreadable_folders_make_the_size_a_minimum() {
    let mut f = base();
    f.tree.insert(win_old().join("Windows"), Err("accesso negato".into()));
    let found = analyze_fake(&f);
    assert_eq!(found.bytes, 100);
    assert!(found.detail[0].contains("almeno"), "{:?}", found.detail);
    assert!(found.detail[0].contains("1 cartella non leggibile"), "{:?}", found.detail);
}

#[test]
fn cancelled_analysis_proposes_nothing() {
    let f = base();
    let cancel = AtomicBool::new(true);
    let found = analyze(&f, &ticking(Duration::from_millis(1)), &cancel);
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty());
}

// ---------------------------------------------------------------------------
// Pulizia.

#[test]
fn clean_sets_only_the_flag_runs_cleanmgr_waits_and_removes_the_flag() {
    let f = base();
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(
        f.calls(),
        ["available", "flagged", "set_marker", "set", "start", "poll", "poll", "poll", "remove", "remove_marker"]
    );
    assert!(!f.marker.get(), "compagno tolto");
    assert_eq!(out.removed, 1);
    assert_eq!(out.failed, 0, "{:?}", out.notes);
    assert_eq!(out.freed, 150);
    assert!(out.done.iter().any(|d| d.contains("Windows.old")), "{:?}", out.done);
}

#[test]
fn clean_does_nothing_if_another_program_set_stateflags0099() {
    let mut f = base();
    f.flagged = Ok(vec!["Temporary Files".into(), "Previous Installations".into()]);
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls(), ["available", "flagged"], "né scrittura, né cleanmgr, né rimozione del valore altrui");
    assert_eq!(out.failed, 1);
    assert_eq!(out.removed, 0);
    let n = out.notes.join("\n");
    assert!(n.contains("StateFlags0099") && n.contains("Temporary Files"), "{n}");
}

#[test]
fn clean_does_nothing_if_the_flags_cannot_be_read() {
    let mut f = base();
    f.flagged = Err("accesso negato a Temporary Files".into());
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls(), ["available", "flagged"]);
    assert_eq!(out.failed, 1);
    assert!(out.notes.join("\n").contains("accesso negato a Temporary Files"), "{:?}", out.notes);
}

#[test]
fn nothing_is_written_when_cleanmgr_is_missing() {
    let mut f = base();
    f.available_err = Some("cleanmgr.exe non trovato".into());
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls(), ["available"]);
    assert_eq!(out.failed, 1);
    assert!(out.notes.join("\n").contains("cleanmgr.exe non trovato"), "{:?}", out.notes);
}

#[test]
fn flag_is_removed_even_when_writing_it_fails() {
    let mut f = base();
    f.set_err = Some("gestore «Previous Installations» assente".into());
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls(), ["available", "flagged", "set_marker", "set", "remove", "remove_marker"]);
    assert_eq!(out.failed, 1);
    assert_eq!(out.removed, 0);
    assert!(out.notes.join("\n").contains("Previous Installations"), "{:?}", out.notes);
}

#[test]
fn flag_is_removed_even_when_cleanmgr_does_not_start() {
    let mut f = base();
    f.start_err = Some("avvio negato".into());
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls(), ["available", "flagged", "set_marker", "set", "start", "remove", "remove_marker"]);
    assert_eq!(out.failed, 1);
    assert!(out.notes.join("\n").contains("avvio negato"), "{:?}", out.notes);
}

#[test]
fn flag_is_removed_even_if_waiting_panics() {
    let mut f = base();
    f.poll_panics = true;
    let found = analyze_fake(&f);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| clean_fake(&f, &found)));
    assert!(r.is_err());
    assert_eq!(f.calls(), ["available", "flagged", "set_marker", "set", "start", "poll", "remove", "remove_marker"]);
}

#[test]
fn cancel_stops_waiting_without_killing_cleanmgr_and_removes_the_flag() {
    let mut f = base();
    f.polls_before_exit = 1_000;
    let found = analyze_fake(&f);
    let cancel = AtomicBool::new(false);
    let sleeps = Cell::new(0u32);
    let sleep = |_: Duration| {
        sleeps.set(sleeps.get() + 1);
        if sleeps.get() == 3 {
            cancel.store(true, Ordering::Relaxed);
        }
    };
    let out = clean_fake_with(&f, &found, &cancel, &sleep);
    let calls = f.calls();
    assert_eq!(calls.last().map(String::as_str), Some("remove_marker"));
    assert_eq!(calls.iter().filter(|c| *c == "poll").count(), 4);
    assert_eq!(out.removed, 0);
    assert_eq!(out.failed, 0);
    assert_eq!(out.freed, 0);
    assert!(out.notes.join("\n").contains("smesso di aspettare"), "{:?}", out.notes);
}

#[test]
fn cancel_before_starting_writes_nothing() {
    let f = base();
    let found = analyze_fake(&f);
    let cancel = AtomicBool::new(true);
    let out = clean_fake_with(&f, &found, &cancel, &|_| {});
    assert!(f.calls().is_empty(), "{:?}", f.calls());
    assert_eq!(out.removed + out.failed, 0);
}

#[test]
fn clean_acts_only_on_what_the_same_job_found() {
    for items in [&[][..], &[r"D:\Windows.old"][..], &[r"C:\Windows"][..], &[r"C:\Windows.old\Windows"][..]] {
        let f = base();
        let out = clean_fake(&f, &forced(items));
        assert!(f.calls().is_empty(), "{items:?}: {:?}", f.calls());
        assert_eq!(out.removed + out.failed, 0, "{items:?}");
    }
    // Stesso percorso con maiuscole diverse: è lo stesso elemento.
    let f = base();
    let out = clean_fake(&f, &forced(&[r"c:\WINDOWS.OLD"]));
    assert_eq!(out.removed, 1, "{:?}", out.notes);
}

#[test]
fn folder_already_gone_is_reported_and_nothing_runs() {
    let mut f = base();
    f.before = Kind::Missing;
    let out = clean_fake(&f, &forced(&[r"C:\Windows.old"]));
    assert!(f.calls().is_empty(), "{:?}", f.calls());
    assert_eq!(out.removed + out.failed, 0);
    assert!(out.notes.join("\n").contains("già assente"), "{:?}", out.notes);
}

#[test]
fn folder_turned_into_a_link_is_never_handed_to_cleanmgr() {
    for k in [Kind::Reparse, Kind::Other, Kind::Unreadable] {
        let mut f = base();
        f.before = k;
        let out = clean_fake(&f, &forced(&[r"C:\Windows.old"]));
        assert!(f.calls().is_empty(), "{k:?}: {:?}", f.calls());
        assert_eq!(out.failed, 1, "{k:?}");
    }
}

#[test]
fn folder_shrunk_after_cleanmgr_says_check_the_space_not_a_failure() {
    // cleanmgr esce presto e passa il lavoro a un altro processo: la cartella è più piccola.
    let mut f = base();
    f.after = Kind::Dir;
    f.tree_after.insert(win_old(), Ok(vec![file("resto", 30)]));
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls().last().map(String::as_str), Some("remove_marker"));
    assert_eq!(out.removed, 0);
    assert_eq!(out.failed, 0, "{:?}", out.notes);
    assert_eq!(out.freed, 120);
    let n = out.notes.join("\n");
    assert!(n.contains("Pulizia disco ha terminato") && n.contains("controlla lo spazio"), "{n}");
}

#[test]
fn folder_unchanged_after_cleanmgr_is_a_failure() {
    let mut f = base();
    f.after = Kind::Dir;
    f.tree_after = f.tree.clone();
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(out.failed, 1);
    assert_eq!(out.freed, 0);
    assert!(out.notes.join("\n").contains("ancora presente"), "{:?}", out.notes);
}

#[test]
fn leftover_flag_of_pulisci_pc_is_recognised_removed_and_the_clean_proceeds() {
    let mut f = base();
    f.flagged = Ok(vec!["Previous Installations".into()]);
    f.marker.set(true);
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(
        f.calls(),
        [
            "available", "flagged", "has_marker", "remove", "remove_marker", "set_marker", "set", "start", "poll", "poll",
            "poll", "remove", "remove_marker"
        ]
    );
    assert_eq!(out.removed, 1);
    assert_eq!(out.failed, 0, "{:?}", out.notes);
    assert!(out.notes.join("\n").contains("rimasto da una pulizia interrotta di PulisciPC"), "{:?}", out.notes);
}

#[test]
fn flag_under_previous_installations_without_our_marker_is_not_ours() {
    let mut f = base();
    f.flagged = Ok(vec!["Previous Installations".into()]);
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls(), ["available", "flagged", "has_marker"]);
    assert_eq!(out.failed, 1);
    assert!(out.notes.join("\n").contains("altro programma"), "{:?}", out.notes);
    // Con il compagno ma anche un'altra chiave con il flag: non è solo nostro.
    let mut f = base();
    f.flagged = Ok(vec!["Previous Installations".into(), "Temporary Files".into()]);
    f.marker.set(true);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls(), ["available", "flagged"]);
    assert_eq!(out.failed, 1);
}

#[test]
fn leftover_flag_that_cannot_be_removed_stops_everything() {
    let mut f = base();
    f.flagged = Ok(vec!["Previous Installations".into()]);
    f.marker.set(true);
    f.remove_err = Some("accesso negato".into());
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls(), ["available", "flagged", "has_marker", "remove"]);
    assert!(f.marker.get(), "il compagno resta finché resta il flag");
    assert_eq!(out.failed, 1);
    assert!(out.notes.join("\n").contains("accesso negato"), "{:?}", out.notes);
}

#[test]
fn analysis_shows_personal_files_left_in_users_and_numbered_copies() {
    let mut f = base();
    let users = win_old().join("Users");
    f.tree.insert(win_old(), Ok(vec![file("a.dat", 100), dir("Users")]));
    f.tree.insert(
        users.clone(),
        Ok(vec![
            dir("Mario"),
            dir("Default"),
            dir("Public"),
            Entry { name: "All Users".into(), dir: true, reparse: true, size: 0 },
        ]),
    );
    f.tree.insert(users.join("Mario"), Ok(vec![dir("Documents"), dir("AppData")]));
    f.tree.insert(users.join("Mario").join("Documents"), Ok(vec![file("tesi.docx", 2048), file("desktop.ini", 1)]));
    f.tree.insert(users.join("Mario").join("Documents").join("x"), Ok(vec![]));
    f.tree.insert(users.join("Mario").join("AppData"), Ok(vec![file("cache", 999)]));
    f.tree.insert(users.join("Default").join("Documents"), Ok(vec![file("modello", 5)]));
    f.tree.insert(users.join("Public").join("Desktop"), Ok(vec![file("desktop.ini", 1)]));
    f.tree.insert(
        PathBuf::from(r"C:\"),
        Ok(vec![
            dir("Windows.old"),
            dir("Windows.old.000"),
            dir("windows.old.12"),
            dir("Windows.old.bak"),
            Entry { name: "Windows.old.001".into(), dir: true, reparse: true, size: 0 },
        ]),
    );
    let found = analyze_fake(&f);
    let d = found.detail.join("\n");
    assert!(d.contains("file personali") && d.contains("Mario (1 file, "), "{d}");
    assert!(!d.contains("Default (") && !d.contains("Public ("), "{d}");
    assert!(d.contains(r"C:\Windows.old.000") && d.contains(r"C:\windows.old.12"), "{d}");
    assert!(!d.contains("Windows.old.bak") && !d.contains("Windows.old.001"), "{d}");
    assert_eq!(found.items, vec![r"C:\Windows.old".to_string()]);
}

#[test]
fn no_personal_files_no_warning_line() {
    let f = base();
    let found = analyze_fake(&f);
    assert!(!found.detail.join("\n").contains("file personali"), "{:?}", found.detail);
}

#[test]
fn exit_code_is_reported_when_the_folder_stays() {
    let mut f = base();
    f.after = Kind::Dir;
    f.exit_code = 5;
    f.tree_after.insert(win_old(), Ok(vec![file("a.dat", 100), dir("Windows")]));
    f.tree_after.insert(win_old().join("Windows"), Ok(vec![file("b.dll", 50)]));
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(out.freed, 0);
    assert_eq!(out.failed, 1);
    assert!(out.notes.join("\n").contains("codice 5"), "{:?}", out.notes);
}

#[test]
fn failing_to_remove_the_flag_is_an_error_with_instructions() {
    let mut f = base();
    f.remove_err = Some("accesso negato".into());
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(out.removed, 1, "Windows.old è stata eliminata comunque");
    assert_eq!(out.failed, 1);
    let n = out.notes.join("\n");
    assert!(n.contains("StateFlags0099") && n.contains("non tolto") && n.contains("accesso negato"), "{n}");
}

#[test]
fn waiting_gives_up_after_the_maximum_time() {
    let mut f = base();
    f.polls_before_exit = u32::MAX;
    let mut child = f.start().unwrap();
    let r = wait_cleanmgr(child.as_mut(), &AtomicBool::new(false), Duration::from_secs(2), &|_| {});
    assert_eq!(r, Ok(Waited::TimedOut));
    // 0; 0,5; 1; 1,5; 2 s.
    assert_eq!(f.calls().iter().filter(|c| *c == "poll").count(), 5);
    f.polls_before_exit = 0;
    f.exit_code = 3;
    let mut child = f.start().unwrap();
    assert_eq!(wait_cleanmgr(child.as_mut(), &AtomicBool::new(true), Duration::from_secs(2), &|_| {}), Ok(Waited::Exited(3)));
}

#[test]
fn timeout_in_clean_is_a_note_and_the_flag_is_removed() {
    let mut f = base();
    f.polls_before_exit = u32::MAX;
    let found = analyze_fake(&f);
    let out = clean_fake(&f, &found);
    assert_eq!(f.calls().last().map(String::as_str), Some("remove_marker"));
    assert_eq!(out.removed, 0);
    assert!(out.notes.join("\n").contains("non ha finito"), "{:?}", out.notes);
}

// ---------------------------------------------------------------------------
// Implementazione vera, solo su una cartella temporanea.

#[test]
fn real_listing_measures_a_temp_tree() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Windows.old");
    std::fs::create_dir_all(root.join("sub").join("deep")).unwrap();
    std::fs::write(root.join("a.bin"), vec![0u8; 1000]).unwrap();
    std::fs::write(root.join("sub").join("b.bin"), vec![0u8; 234]).unwrap();
    std::fs::write(root.join("sub").join("deep").join("c.bin"), vec![0u8; 6]).unwrap();
    let fs = win::RealFiles;
    let mut names: Vec<_> = fs.list(&root).unwrap().into_iter().map(|e| (e.name.to_string_lossy().into_owned(), e.dir, e.size)).collect();
    names.sort();
    assert_eq!(names, vec![("a.bin".to_string(), false, 1000), ("sub".to_string(), true, 0)]);
    assert_eq!(fs.kind(&root), Kind::Dir);
    assert_eq!(fs.kind(&root.join("a.bin")), Kind::Other);
    assert_eq!(fs.kind(&root.join("manca")), Kind::Missing);
    assert!(fs.list(&root.join("manca")).is_err());
    let start = std::time::Instant::now();
    let size = measure(&fs, &root, SIZE_BUDGET, &|| start.elapsed(), &AtomicBool::new(false));
    assert_eq!(size, Size { bytes: 1240, files: 3, ..Default::default() });
    let drive = fs.system_drive().expect("disco di sistema");
    assert!(is_drive_root(&drive), "{}", drive.display());
}

/// Sul PC vero, **solo lettura**: esistenza e dimensione di
/// `%SystemDrive%\Windows.old` con l'analisi della voce. Mai `clean`, mai il
/// registro, mai `cleanmgr`.
#[test]
#[ignore]
fn live_winold_analysis_read_only() {
    let env = pc_rules::Env::for_rules();
    let guard = pc_rules::Guard::new(&env);
    let cancel = AtomicBool::new(false);
    let ctx = NativeCtx { env: &env, cancel: &cancel, guard: &guard };
    let item = items().into_iter().next().unwrap();
    let start = std::time::Instant::now();
    let found = (item.analyze)(&ctx);
    println!("percorso: {:?}", windows_old_path(&win::RealFiles));
    println!("count={} bytes={} skipped={} items={:?} ({:?})", found.count, found.bytes, found.skipped, found.items, start.elapsed());
    for l in &found.detail {
        println!("  {l}");
    }
}
