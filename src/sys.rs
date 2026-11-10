//! Small helpers around Windows: admin rights, memory, recycle bin, commands.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn hidden_cmd(program: &str) -> Command {
    let mut c = Command::new(program);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn is_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// Starts this program again with administrator rights (Windows asks the person).
pub fn relaunch_as_admin() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let r = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    (r as isize) > 32
}

pub struct Memory {
    pub total: u64,
    pub available: u64,
}

pub fn memory() -> Memory {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    unsafe { GlobalMemoryStatusEx(&mut m) };
    Memory { total: m.ullTotalPhys, available: m.ullAvailPhys }
}

/// (bytes, items) in the recycle bin of every drive.
pub fn recycle_bin_info() -> (u64, u64) {
    use windows_sys::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
    let mut info: SHQUERYRBINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHQUERYRBINFO>() as u32;
    let hr = unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) };
    if hr == 0 {
        (info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64)
    } else {
        (0, 0)
    }
}

pub fn empty_recycle_bin() -> bool {
    // SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND
    let hr = unsafe {
        windows_sys::Win32::UI::Shell::SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), 0x1 | 0x2 | 0x4)
    };
    hr == 0
}

/// Turns "@%SystemRoot%\system32\x.dll,-100" into the text it points to.
pub fn indirect_string(s: &str) -> String {
    if !s.starts_with('@') {
        return s.to_string();
    }
    let src = wide(s);
    let mut buf = vec![0u16; 512];
    let hr = unsafe {
        windows_sys::Win32::UI::Shell::SHLoadIndirectString(src.as_ptr(), buf.as_mut_ptr(), buf.len() as u32, std::ptr::null_mut())
    };
    if hr == 0 {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        String::from_utf16_lossy(&buf[..n])
    } else {
        s.to_string()
    }
}

/// Replaces %VAR% with the environment variable's value.
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        if let Some(b) = after.find('%') {
            let name = &after[..b];
            match std::env::var(name) {
                Ok(v) if !name.is_empty() => out.push_str(&v),
                _ => {
                    out.push('%');
                    out.push_str(name);
                    out.push('%');
                }
            }
            rest = &after[b + 1..];
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

pub fn windir() -> String {
    std::env::var("WINDIR").or_else(|_| std::env::var("SystemRoot")).unwrap_or_else(|_| "C:\\Windows".into())
}

pub fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

pub fn fmt_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", b / (1024.0 * 1024.0 * 1024.0)).replace('.', ",")
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0)).replace('.', ",")
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{} byte", bytes)
    }
}

/// Folder where the app keeps its backups (Documents\PulisciPC).
pub fn data_dir() -> std::path::PathBuf {
    let base = std::env::var("USERPROFILE").map(|p| Path::new(&p).join("Documents")).unwrap_or_else(|_| ".".into());
    let d = base.join("PulisciPC");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Local date and time as "AAAAMMGG_HHMMSS" (from the system clock, UTC offset ignored).
pub fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (y, m, d) = civil(days as i64);
    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", y, m, d, rem / 3600, (rem / 60) % 60, rem % 60)
}

fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
