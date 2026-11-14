//! Windows services: list, memory use, disable / manual / restore.

use crate::sys;
use std::collections::HashMap;
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

fn backup_path() -> std::path::PathBuf {
    sys::data_dir().join("servizi_originali.txt")
}

pub fn read_backup() -> Vec<(String, u32, bool)> {
    std::fs::read_to_string(backup_path())
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

fn remember(svc: &Service) {
    let saved = read_backup();
    if saved.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(&svc.name)) {
        return; // keep the very first state
    }
    let mut text = std::fs::read_to_string(backup_path()).unwrap_or_default();
    text.push_str(&format!("{}\t{}\t{}\n", svc.name, svc.start, if svc.delayed { 1 } else { 0 }));
    let _ = std::fs::write(backup_path(), text);
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

/// Sets the start type (Disattivato or Manuale) and stops the service. Returns true on success.
pub fn set_start(svc: &Service, start: u32) -> bool {
    if svc.protected {
        return false;
    }
    remember(svc);
    let ok = sc(&["config", &svc.name, "start=", start_word(start, false)]);
    if ok && svc.running {
        let _ = sc(&["stop", &svc.name]);
    }
    ok
}

/// Puts every service changed by this app back the way it was. Returns (restored, failed).
pub fn restore_all() -> (usize, usize) {
    let (mut ok, mut bad) = (0, 0);
    let saved = read_backup();
    let mut kept = String::new();
    for (name, start, delayed) in &saved {
        if sc(&["config", name, "start=", start_word(*start, *delayed)]) {
            ok += 1;
            if *start == START_AUTO {
                let _ = sc(&["start", name]);
            }
        } else {
            bad += 1;
            kept.push_str(&format!("{}\t{}\t{}\n", name, start, if *delayed { 1 } else { 0 }));
        }
    }
    let _ = std::fs::write(backup_path(), kept);
    (ok, bad)
}
