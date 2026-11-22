//! Small helpers around Windows: admin rights, accounts, memory, recycle bin.

use std::path::Path;

pub fn is_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// Account of the process (e.g. the administrator that approved the UAC prompt).
fn process_user() -> Option<String> {
    use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;
    let mut buf = [0u16; 257];
    let mut len = buf.len() as u32;
    if unsafe { GetUserNameW(buf.as_mut_ptr(), &mut len) } == 0 || len == 0 {
        return None;
    }
    // `len` counts the terminating NUL.
    Some(String::from_utf16_lossy(&buf[..(len as usize - 1).min(buf.len())]))
}

/// Account that opened the interactive session (the person at the keyboard).
fn session_user() -> Option<String> {
    use windows_sys::Win32::System::RemoteDesktop::{
        WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName, WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION,
    };
    let mut p: *mut u16 = std::ptr::null_mut();
    let mut bytes = 0u32;
    let ok = unsafe { WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSUserName, &mut p, &mut bytes) };
    if ok == 0 || p.is_null() {
        return None;
    }
    let name = unsafe {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    };
    unsafe { WTSFreeMemory(p.cast()) };
    Some(name)
}

/// True when the two account names are different people: case-insensitive, the part before the last `\` (domain) ignored.
fn names_differ(process: &str, session: &str) -> bool {
    let base = |n: &str| n.rsplit('\\').next().unwrap_or(n).to_lowercase();
    base(process) != base(session)
}

/// `Some((process account, session account))` when the program runs under a different account
/// from the one that opened the session (the UAC prompt was approved with another administrator):
/// "the user's" folders would then be those of the wrong profile.
pub fn profile_mismatch() -> Option<(String, String)> {
    let (p, s) = (process_user()?, session_user()?);
    if s.is_empty() || !names_differ(&p, &s) {
        return None;
    }
    Some((p, s))
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

pub use xiox_model::fmt_size;

/// Folder where the app keeps its backups (Documents\Xiox).
pub fn data_dir() -> std::path::PathBuf {
    let base = std::env::var("USERPROFILE").map(|p| Path::new(&p).join("Documents")).unwrap_or_else(|_| ".".into());
    data_dir_in(&base)
}

/// [`data_dir`] under the given Documents folder.
pub fn data_dir_in(documents: &Path) -> std::path::PathBuf {
    let d = documents.join(xiox_model::APP_DIR);
    let _ = std::fs::create_dir_all(&d);
    d
}

/// A count for the user, grouped with commas (en-US): `18204` is `18,204`. Sizes keep their own
/// format ([`fmt_size`]). Every file / item / issue count that reaches a view goes through this.
pub fn fmt_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `1 item`, `0 items`, `18,204 items`: the count (grouped, see [`fmt_count`]) and the word that
/// agrees with it. Any unsigned count type works.
pub fn plural<N: TryInto<u64>>(n: N, one: &str, many: &str) -> String {
    let n = n.try_into().unwrap_or(u64::MAX);
    format!("{} {}", fmt_count(n), if n == 1 { one } else { many })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// I backup stanno in `Documents\Xiox`, che si crea al primo uso.
    #[test]
    fn backups_folder_is_xiox_under_documents_and_is_created() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(data_dir_in(t.path()), t.path().join("Xiox"));
        assert!(t.path().join("Xiox").is_dir());
    }

    #[test]
    fn counts_are_grouped_and_agree_with_their_word() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1_000), "1,000");
        assert_eq!(fmt_count(18_204), "18,204");
        assert_eq!(fmt_count(1_234_567), "1,234,567");
        assert_eq!(plural(0usize, "item", "items"), "0 items");
        assert_eq!(plural(1u64, "item", "items"), "1 item");
        assert_eq!(plural(2u32, "item", "items"), "2 items");
        assert_eq!(plural(18_204usize, "file", "files"), "18,204 files");
    }

    #[test]
    fn same_user_any_case() {
        assert!(!names_differ("Alfio", "alfio"));
    }

    #[test]
    fn domain_prefix_ignored() {
        assert!(!names_differ(r"PC\alfio", "alfio"));
    }

    #[test]
    fn other_account() {
        assert!(names_differ("Admin", "alfio"));
    }
}
