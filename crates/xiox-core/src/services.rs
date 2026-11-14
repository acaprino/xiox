//! Windows services: list, memory use, disable / manual / restore.

use crate::sys;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use winreg::enums::*;
use winreg::RegKey;

pub const START_AUTO: u32 = 2;
/// SERVICE_DEMAND_START
pub const START_MANUAL: u32 = 3;
/// SERVICE_DISABLED
pub const START_DISABLED: u32 = 4;

pub struct Suggestion {
    pub name: &'static str,
    pub why: &'static str,
}

/// Services that are usually safe to turn off on a home PC (our own list, estimated).
pub const SUGGESTED: &[Suggestion] = &[
    Suggestion { name: "SysMain", why: "Preloads programs into RAM: leaving it off frees memory" },
    Suggestion { name: "DiagTrack", why: "Telemetry: sends usage data to Microsoft" },
    Suggestion { name: "dmwappushservice", why: "Routing of telemetry push messages" },
    Suggestion { name: "WSearch", why: "Indexes files for search (uses RAM and disk)" },
    Suggestion { name: "Spooler", why: "Printing: turn it off only if you do not use printers" },
    Suggestion { name: "Fax", why: "Sending and receiving faxes" },
    Suggestion { name: "MapsBroker", why: "Updates offline maps" },
    Suggestion { name: "RetailDemo", why: "Demo mode for shops" },
    Suggestion { name: "RemoteRegistry", why: "Lets another PC edit the registry" },
    Suggestion { name: "lfsvc", why: "Geolocation" },
    Suggestion { name: "PhoneSvc", why: "Telephony management" },
    Suggestion { name: "WerSvc", why: "Windows Error Reporting" },
    Suggestion { name: "wisvc", why: "Windows Insider program" },
    Suggestion { name: "TrkWks", why: "Tracks links to files on the network" },
    Suggestion { name: "WMPNetworkSvc", why: "Windows Media Player network sharing" },
    Suggestion { name: "XblAuthManager", why: "Xbox Live sign-in (turn it off if you do not play with Xbox)" },
    Suggestion { name: "XblGameSave", why: "Xbox Live saved games" },
    Suggestion { name: "XboxNetApiSvc", why: "Xbox Live networking" },
    Suggestion { name: "XboxGipSvc", why: "Xbox accessories" },
    Suggestion { name: "WpcMonSvc", why: "Parental controls" },
    Suggestion { name: "SEMgrSvc", why: "Payments and NFC" },
    Suggestion { name: "PcaSvc", why: "Program Compatibility Assistant" },
    Suggestion { name: "stisvc", why: "Image acquisition: turn it off if you do not use scanners" },
    Suggestion { name: "bthserv", why: "Bluetooth: turn it off only if you do not use it" },
    Suggestion { name: "edgeupdate", why: "Automatic update of Microsoft Edge" },
    Suggestion { name: "edgeupdatem", why: "Automatic update of Microsoft Edge" },
    Suggestion { name: "gupdate", why: "Automatic update of Google" },
    Suggestion { name: "gupdatem", why: "Automatic update of Google" },
    Suggestion { name: "AdobeARMservice", why: "Automatic update of Adobe" },
];

/// Services Windows needs to work: never changed by this app.
pub const PROTECTED: &[&str] = &[
    "RpcSs", "RpcEptMapper", "DcomLaunch", "LSM", "SamSs", "Winmgmt", "EventLog", "PlugPlay", "Power", "ProfSvc",
    "BFE", "mpssvc", "WinDefend", "WdNisSvc", "SecurityHealthService", "Sense", "Dhcp", "Dnscache", "nsi",
    "AudioSrv", "AudioEndpointBuilder", "Schedule", "SENS", "UserManager", "CoreMessagingRegistrar",
    "BrokerInfrastructure", "SystemEventsBroker", "StateRepository", "gpsvc", "CryptSvc", "wuauserv",
    "TrustedInstaller", "msiserver", "Appinfo", "Wcmsvc", "WlanSvc", "netprofm", "NlaSvc", "EventSystem",
    "KeyIso", "VaultSvc", "camsvc", "TokenBroker", "TimeBrokerSvc", "sppsvc", "ClipSVC", "AppXSvc",
    "DeviceInstall", "DispBrokerDesktopSvc", "LanmanWorkstation", "Netlogon", "WinHttpAutoProxySvc",
    "iphlpsvc", "FontCache", "Themes", "ShellHWDetection", "WpnService", "DoSvc", "UsoSvc", "BITS",
    "EFS", "seclogon", "TabletInputService", "TextInputManagementService", "Wecsvc", "W32Time", "VSS",
];

#[derive(Clone)]
#[allow(dead_code)]
pub struct Service {
    pub name: String,
    pub display: String,
    pub start: u32,
    pub delayed: bool,
    pub running: bool,
    pub pid: u32,
    pub mem_kb: u64,
    /// How many services share its process (1 = it has its own).
    pub shared: usize,
    pub suggestion: Option<&'static str>,
    pub protected: bool,
    pub selected: bool,
}

impl Service {
    pub fn start_text(&self) -> &'static str {
        match self.start {
            0 | 1 => "System",
            START_AUTO if self.delayed => "Automatic (delayed)",
            START_AUTO => "Automatic",
            START_MANUAL => "Manual",
            START_DISABLED => "Disabled",
            _ => "?",
        }
    }
    /// RAM freed by stopping it: only counted when it has its own process.
    pub fn freeable_kb(&self) -> u64 {
        if self.running && self.shared == 1 {
            self.mem_kb
        } else {
            0
        }
    }
}

fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for ch in line.chars() {
        match ch {
            '"' => q = !q,
            ',' if !q => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out
}

fn run_text(args: &[&str]) -> String {
    sys::hidden_cmd("tasklist")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

pub fn load() -> Vec<Service> {
    // service name -> pid, and pid -> service count
    let mem_job = std::thread::spawn(|| run_text(&["/fo", "csv", "/nh"]));
    let mut pid_of: HashMap<String, u32> = HashMap::new();
    let mut count: HashMap<u32, usize> = HashMap::new();
    for line in run_text(&["/svc", "/fo", "csv", "/nh"]).lines() {
        let f = csv_fields(line.trim());
        if f.len() < 3 {
            continue;
        }
        let Ok(pid) = f[1].trim().parse::<u32>() else { continue };
        for s in f[2].split(',') {
            let s = s.trim();
            if !s.is_empty() && s != "N/A" && s != "N/D" {
                pid_of.insert(s.to_lowercase(), pid);
                *count.entry(pid).or_default() += 1;
            }
        }
    }
    let mut mem: HashMap<u32, u64> = HashMap::new();
    for line in mem_job.join().unwrap_or_default().lines() {
        let f = csv_fields(line.trim());
        if f.len() < 5 {
            continue;
        }
        let Ok(pid) = f[1].trim().parse::<u32>() else { continue };
        let kb: String = f[f.len() - 1].chars().filter(|c| c.is_ascii_digit()).collect();
        mem.insert(pid, kb.parse().unwrap_or(0));
    }

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(base) = hklm.open_subkey_with_flags("SYSTEM\\CurrentControlSet\\Services", KEY_READ) else { return vec![] };
    let mut out = Vec::new();
    for name in base.enum_keys().flatten() {
        let Ok(k) = base.open_subkey_with_flags(&name, KEY_READ) else { continue };
        let ty = k.get_value::<u32, _>("Type").unwrap_or(0);
        if ty & 0x30 == 0 || ty & 0x40 != 0 || ty & 0x80 != 0 {
            continue; // drivers, per-user service templates and instances
        }
        let display = k.get_value::<String, _>("DisplayName").map(|d| sys::indirect_string(&d)).unwrap_or_else(|_| name.clone());
        let pid = pid_of.get(&name.to_lowercase()).copied().unwrap_or(0);
        let suggestion = SUGGESTED.iter().find(|s| s.name.eq_ignore_ascii_case(&name)).map(|s| s.why);
        out.push(Service {
            display: if display.is_empty() { name.clone() } else { display },
            start: k.get_value::<u32, _>("Start").unwrap_or(3),
            delayed: k.get_value::<u32, _>("DelayedAutostart").unwrap_or(0) == 1,
            running: pid != 0,
            pid,
            mem_kb: mem.get(&pid).copied().unwrap_or(0),
            shared: count.get(&pid).copied().unwrap_or(0),
            protected: PROTECTED.iter().any(|p| p.eq_ignore_ascii_case(&name)) || ty & 0x30 == 0,
            suggestion,
            selected: false,
            name,
        });
    }
    out.sort_by_key(|s| s.display.to_lowercase());
    out
}

/// The file with the start type each changed service had. `servizi_originali.txt` is the name
/// older versions wrote: an existing file keeps being used (see [`sys::data_file_in`]).
const BACKUP_FILE: &str = "services-original.txt";
const OLD_BACKUP_FILE: &str = "servizi_originali.txt";

fn backup_path_in(dir: &Path) -> PathBuf {
    sys::data_file_in(dir, BACKUP_FILE, OLD_BACKUP_FILE)
}

/// The services Xiox changed, with the start type each had (`name`, start, delayed).
pub fn read_backup() -> Vec<(String, u32, bool)> {
    read_backup_in(&sys::data_dir())
}

fn read_backup_in(dir: &Path) -> Vec<(String, u32, bool)> {
    std::fs::read_to_string(backup_path_in(dir))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            let n = p.next()?.to_string();
            let s = p.next()?.parse().ok()?;
            let d = p.next()? == "1";
            Some((n, s, d))
        })
        .collect()
}

/// Saves the start type `svc` has now, once (the very first state is the one kept). `true` when
/// it is on record afterwards (it already was, or it was written and read back); `false` when the
/// file cannot be read or written: the service must then NOT be changed, because «Restore
/// originals» could not put it back.
fn remember_in(dir: &Path, svc: &Service) -> bool {
    let path = backup_path_in(dir);
    let mut text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return false, // a list that cannot be read must not be overwritten
    };
    let recorded = |text: &str| text.lines().any(|l| l.split('\t').next().is_some_and(|n| n.eq_ignore_ascii_case(&svc.name)));
    if recorded(&text) {
        return true;
    }
    text.push_str(&format!("{}\t{}\t{}\n", svc.name, svc.start, if svc.delayed { 1 } else { 0 }));
    std::fs::write(&path, &text).is_ok() && std::fs::read_to_string(&path).is_ok_and(|back| recorded(&back))
}

fn sc(args: &[&str]) -> bool {
    sys::hidden_cmd("sc").args(args).status().map(|s| s.success()).unwrap_or(false)
}

fn start_word(start: u32, delayed: bool) -> &'static str {
    match start {
        0 => "boot",
        1 => "system",
        START_AUTO if delayed => "delayed-auto",
        START_AUTO => "auto",
        START_DISABLED => "disabled",
        _ => "demand",
    }
}

/// How a change of one service ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Done,
    /// `sc` refused (usually: not administrator), or the service is protected.
    Failed,
    /// Its original start type could not be saved, so `sc` was NOT run: the service is untouched.
    NotSaved,
}

/// Sets the start type (Disabled or Manual) and stops the service.
pub fn set_start(svc: &Service, start: u32) -> Change {
    set_start_in(&sys::data_dir(), svc, start, &mut sc)
}

/// [`set_start`] with the folder of the saved originals and the `sc` call given.
fn set_start_in(dir: &Path, svc: &Service, start: u32, run: &mut dyn FnMut(&[&str]) -> bool) -> Change {
    if svc.protected {
        return Change::Failed;
    }
    if !remember_in(dir, svc) {
        return Change::NotSaved;
    }
    if !run(&["config", &svc.name, "start=", start_word(start, false)]) {
        return Change::Failed;
    }
    if svc.running {
        let _ = run(&["stop", &svc.name]);
    }
    Change::Done
}

/// Puts every service changed by this app back the way it was. Returns (restored, failed).
pub fn restore_all() -> (usize, usize) {
    restore_all_in(&sys::data_dir(), &mut sc)
}

fn restore_all_in(dir: &Path, run: &mut dyn FnMut(&[&str]) -> bool) -> (usize, usize) {
    let (mut ok, mut bad) = (0, 0);
    let saved = read_backup_in(dir);
    let mut kept = String::new();
    for (name, start, delayed) in &saved {
        if run(&["config", name, "start=", start_word(*start, *delayed)]) {
            ok += 1;
            if *start == START_AUTO {
                let _ = run(&["start", name]);
            }
        } else {
            bad += 1;
            kept.push_str(&format!("{}\t{}\t{}\n", name, start, if *delayed { 1 } else { 0 }));
        }
    }
    if !saved.is_empty() {
        let _ = std::fs::write(backup_path_in(dir), kept);
    }
    (ok, bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, start: u32, running: bool, protected: bool) -> Service {
        Service {
            name: name.into(),
            display: name.into(),
            start,
            delayed: false,
            running,
            pid: 0,
            mem_kb: 0,
            shared: 1,
            suggestion: None,
            protected,
            selected: true,
        }
    }

    type Calls = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

    /// Records every `sc` call and answers `ok` to all of them.
    fn recorder(ok: bool) -> (Calls, impl FnMut(&[&str]) -> bool) {
        let calls: Calls = Default::default();
        let seen = calls.clone();
        (calls, move |args: &[&str]| {
            seen.borrow_mut().push(args.join(" "));
            ok
        })
    }

    #[test]
    fn the_original_is_saved_first_and_in_the_english_file() {
        let t = tempfile::tempdir().unwrap();
        let (calls, mut run) = recorder(true);
        assert_eq!(set_start_in(t.path(), &svc("Fax", START_AUTO, true, false), START_DISABLED, &mut run), Change::Done);
        assert_eq!(*calls.borrow(), vec!["config Fax start= disabled".to_string(), "stop Fax".to_string()]);
        assert_eq!(std::fs::read_to_string(t.path().join("services-original.txt")).unwrap(), "Fax\t2\t0\n");
        assert!(!t.path().join("servizi_originali.txt").exists());
        // The first state is the one kept: a second change records nothing new.
        let (_, mut run) = recorder(true);
        set_start_in(t.path(), &svc("fax", START_DISABLED, false, false), START_MANUAL, &mut run);
        assert_eq!(read_backup_in(t.path()), vec![("Fax".to_string(), START_AUTO, false)]);
    }

    /// The bug: the original could not be saved and the service was disabled anyway, so
    /// «Restore originals» could never undo it. Here the backup path is a directory, so no file can
    /// be written there.
    #[test]
    fn a_service_whose_original_cannot_be_saved_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("services-original.txt")).unwrap();
        let (calls, mut run) = recorder(true);
        assert_eq!(set_start_in(t.path(), &svc("Fax", START_AUTO, true, false), START_DISABLED, &mut run), Change::NotSaved);
        assert!(calls.borrow().is_empty(), "sc must not run: {:?}", calls.borrow());
    }

    #[test]
    fn protected_and_refused_services_report_a_failure() {
        let t = tempfile::tempdir().unwrap();
        let (calls, mut run) = recorder(true);
        assert_eq!(set_start_in(t.path(), &svc("RpcSs", START_AUTO, true, true), START_DISABLED, &mut run), Change::Failed);
        assert!(calls.borrow().is_empty());
        let (_, mut run) = recorder(false);
        assert_eq!(set_start_in(t.path(), &svc("Fax", START_AUTO, false, false), START_DISABLED, &mut run), Change::Failed);
        // `sc` refused after the original was saved: the record stays (restoring it is harmless).
        assert_eq!(read_backup_in(t.path()).len(), 1);
    }

    #[test]
    fn restore_finds_the_services_file_of_an_older_version_and_keeps_using_it() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("servizi_originali.txt"), "Fax\t2\t0\nSpooler\t3\t0\n").unwrap();
        assert_eq!(read_backup_in(t.path()).len(), 2);
        let (calls, mut run) = recorder(true);
        assert_eq!(restore_all_in(t.path(), &mut run), (2, 0));
        assert_eq!(*calls.borrow(), vec!["config Fax start= auto", "start Fax", "config Spooler start= demand"]);
        assert_eq!(std::fs::read_to_string(t.path().join("servizi_originali.txt")).unwrap(), "");
        assert!(!t.path().join("services-original.txt").exists(), "nothing is copied or created");
    }

    #[test]
    fn a_service_that_does_not_go_back_stays_on_the_list_and_both_files_prefer_the_new_one() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("servizi_originali.txt"), "Old\t2\t0\n").unwrap();
        std::fs::write(t.path().join("services-original.txt"), "Fax\t2\t0\n").unwrap();
        assert_eq!(read_backup_in(t.path()), vec![("Fax".to_string(), START_AUTO, false)]);
        let (_, mut run) = recorder(false);
        assert_eq!(restore_all_in(t.path(), &mut run), (0, 1));
        assert_eq!(std::fs::read_to_string(t.path().join("services-original.txt")).unwrap(), "Fax\t2\t0\n");
        assert_eq!(std::fs::read_to_string(t.path().join("servizi_originali.txt")).unwrap(), "Old\t2\t0\n", "the old file is not touched");
    }

    #[test]
    fn restoring_with_nothing_saved_creates_no_file() {
        let t = tempfile::tempdir().unwrap();
        let (calls, mut run) = recorder(true);
        assert_eq!(restore_all_in(t.path(), &mut run), (0, 0));
        assert!(calls.borrow().is_empty());
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 0);
    }
}
