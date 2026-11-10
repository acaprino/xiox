//! Best-effort "who holds this file open" through the Windows Restart Manager.
//! Read-only: never kills or restarts anything. Any failure yields an empty list.

use std::path::Path;

#[cfg(windows)]
pub fn locking_processes(path: &Path) -> Vec<String> {
    use dc_model::normalize_path;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
    use windows_sys::Win32::System::RestartManager::{
        CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources, RmStartSession,
    };

    // Restart Manager wants a plain `C:\...` path.
    let wide: Vec<u16> = normalize_path(path).as_os_str().encode_wide().chain(Some(0)).collect();
    let mut names: Vec<String> = Vec::new();
    // SAFETY: all pointers passed to the RM API point to live, correctly sized buffers
    // for the duration of each call; the session is always ended before returning.
    unsafe {
        let mut session = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        if RmStartSession(&mut session, 0, key.as_mut_ptr()) != 0 {
            return names;
        }
        let files = [wide.as_ptr()];
        if RmRegisterResources(session, 1, files.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null()) == 0 {
            let (mut needed, mut have, mut reasons) = (0u32, 0u32, 0u32);
            let rc = RmGetList(session, &mut needed, &mut have, std::ptr::null_mut(), &mut reasons);
            if rc == ERROR_MORE_DATA && needed > 0 {
                let mut infos: Vec<RM_PROCESS_INFO> = vec![std::mem::zeroed(); needed as usize];
                have = needed;
                if RmGetList(session, &mut needed, &mut have, infos.as_mut_ptr(), &mut reasons) == 0 {
                    for info in infos.iter().take(have as usize) {
                        let n = &info.strAppName;
                        let len = n.iter().position(|&c| c == 0).unwrap_or(n.len());
                        let name = String::from_utf16_lossy(&n[..len]);
                        let label = format!("{} (pid {})", name, info.Process.dwProcessId);
                        if !names.contains(&label) {
                            names.push(label);
                        }
                    }
                }
            }
        }
        RmEndSession(session);
    }
    names
}

#[cfg(not(windows))]
pub fn locking_processes(_path: &Path) -> Vec<String> {
    Vec::new()
}
