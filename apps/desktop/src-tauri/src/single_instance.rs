//! One Xiox window at a time: a named mutex held for the life of the process. A second launch finds
//! the name taken, brings the first window to the front and exits.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::ptr::null;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

/// Holds the named mutex; releasing it (drop) lets the next launch in.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle came from `CreateMutexW` in `acquire` and is closed exactly once, here.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A NUL-terminated UTF-16 copy of `text` for the wide Win32 calls.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Takes the mutex `name` (`Local\XioxUi`): `None` when another process already holds it.
pub fn acquire(name: &str) -> Option<InstanceGuard> {
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated and outlives the call; no security attributes; not owned at creation.
    let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    if handle.is_null() {
        // The mutex cannot be made at all (not a second instance): do not block the launch on it.
        return Some(InstanceGuard(handle));
    }
    // SAFETY: read straight after the call that sets it, on the same thread.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // The call still handed back a handle to the existing mutex: let go of it.
        // SAFETY: a valid handle we own, closed once.
        unsafe { CloseHandle(handle) };
        return None;
    }
    Some(InstanceGuard(handle))
}

/// What the window search walks with: the app name its titles end with, our own program's file
/// name, and the window found.
struct Search {
    app: String,
    exe: OsString,
    found: HWND,
}

/// Whether `title` is a title of this app's window: the name alone («Xiox», before the first section is shown) or a
/// section followed by it («Cleanup - Xiox», `chrome::title`). The title changes with the section, so it is matched by
/// what it ends with; the owner's program (`same_program`) tells our window from anyone else's.
pub fn is_title_of(app: &str, title: &str) -> bool {
    title == app || title.strip_suffix(app).is_some_and(|rest| rest.ends_with(" - "))
}

/// Whether `image` (the executable of the process that owns a window) is the same program as `exe`
/// (this one): the same file name, any case. A window titled “Xiox” that belongs to anything else
/// (an Explorer window on `Documents\Xiox`, say) is not ours.
fn same_program(image: &Path, exe: &OsStr) -> bool {
    image.file_name().is_some_and(|name| name.eq_ignore_ascii_case(exe))
}

/// The full path of the executable of the process that owns `window`, if it can be read.
///
/// # Safety
/// `window` is a window handle the system gave out; it is only passed back to the system.
unsafe fn owner_image(window: HWND) -> Option<PathBuf> {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out-pointer for the call.
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if pid == 0 {
        return None;
    }
    // SAFETY: a plain query-only open of a process id; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units and `len` is updated to the number written.
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) } != 0;
    // SAFETY: the handle we opened above, closed once.
    unsafe { CloseHandle(process) };
    ok.then(|| PathBuf::from(OsString::from_wide(&buf[..len as usize])))
}

/// `EnumWindows` callback: stops (returns 0) at the first window with one of our titles and our program.
unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: `lparam` is the `&mut Search` that `focus_existing` passed and keeps alive for the walk.
    let search = unsafe { &mut *(lparam as *mut Search) };
    let mut buf = [0u16; 256];
    // SAFETY: `buf` holds the length passed.
    let n = unsafe { GetWindowTextW(window, buf.as_mut_ptr(), buf.len() as i32) };
    if n > 0 && is_title_of(&search.app, &String::from_utf16_lossy(&buf[..n as usize])) {
        // SAFETY: a handle the system just gave us.
        if unsafe { owner_image(window) }.is_some_and(|image| same_program(&image, &search.exe)) {
            search.found = window;
            return 0;
        }
    }
    1
}

/// Raises the window titled with `app` (see `is_title_of`) that belongs to THIS program (restoring it first when
/// minimized), if there is one. A window with such a title of another program is left alone.
pub fn focus_existing(app: &str) {
    let Some(exe) = std::env::current_exe().ok().and_then(|p| p.file_name().map(OsStr::to_os_string)) else {
        return;
    };
    let mut search = Search { app: app.to_string(), exe, found: std::ptr::null_mut() };
    // SAFETY: `visit` only reads and writes `search`, which outlives the walk; the handle found is
    // only passed back to the system.
    unsafe {
        EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
        let window = search.found;
        if window.is_null() {
            return;
        }
        // Only a minimized window is restored: SW_RESTORE on a maximized one would shrink it.
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window);
    }
}

/// Waits, at most `timeout`, for the process `pid` to exit: a restart (`desktop::restart`) starts this
/// process while the old one still holds the instance. A process that is already gone (or never was)
/// returns at once.
pub fn wait_for_exit(pid: u32, timeout: std::time::Duration) {
    let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    unsafe {
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if process.is_null() {
            return;
        }
        WaitForSingleObject(process, ms);
        CloseHandle(process);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_for_a_process_that_is_gone_returns_at_once() {
        let started = std::time::Instant::now();
        // Id 0 is the System Idle Process, which cannot be opened: nothing to wait for.
        wait_for_exit(0, std::time::Duration::from_secs(5));
        let mut child = std::process::Command::new("cmd").args(["/c", "exit"]).spawn().expect("cmd starts");
        let pid = child.id();
        let _ = child.wait();
        wait_for_exit(pid, std::time::Duration::from_secs(5));
        assert!(started.elapsed() < std::time::Duration::from_secs(4), "{:?}", started.elapsed());
    }

    #[test]
    fn the_second_holder_of_a_name_is_refused() {
        // The process id in the name: a running Xiox, or a parallel test run, cannot collide with it.
        let name = format!(r"Local\XioxUiTest{}", std::process::id());
        let first = acquire(&name);
        assert!(first.is_some(), "the first holder gets the name");
        assert!(acquire(&name).is_none(), "the second is refused while the first lives");
        drop(first);
        assert!(acquire(&name).is_some(), "the name is free again after the first is dropped");
    }

    /// The title follows the section (`Cleanup - Xiox`): the running window is found by its end, and nothing else is.
    #[test]
    fn the_title_of_any_section_is_ours_and_a_lookalike_is_not() {
        assert!(is_title_of("Xiox", "Xiox"));
        assert!(is_title_of("Xiox", "Cleanup - Xiox"));
        assert!(is_title_of("Xiox", "Developers - Xiox"));
        assert!(!is_title_of("Xiox", "Xiox Setup"));
        assert!(!is_title_of("Xiox", "NotXiox"), "the suffix is a whole word after a separator");
        assert!(!is_title_of("Xiox", "Cleanup-Xiox"));
        assert!(!is_title_of("Xiox", ""));
    }

    /// The window titled “Xiox” is ours only if its owner is this program: an Explorer window on
    /// `Documents\Xiox` has the same title and must not be raised.
    #[test]
    fn a_window_of_another_program_is_not_ours() {
        let exe = OsStr::new("xiox-ui.exe");
        assert!(same_program(Path::new(r"C:\Program Files\Xiox\xiox-ui.exe"), exe));
        assert!(same_program(Path::new(r"D:\build\XIOX-UI.EXE"), exe), "any case");
        assert!(!same_program(Path::new(r"C:\Windows\explorer.exe"), exe));
        assert!(!same_program(Path::new(r"C:\Windows\xiox-ui.exe.bak"), exe));
        assert!(!same_program(Path::new(""), exe));
    }

    /// With no window of ours anywhere, nothing is raised and nothing breaks (no program has a
    /// window with this title).
    #[test]
    fn focusing_a_window_that_does_not_exist_does_nothing() {
        focus_existing("Xiox window that no program has 8f3a2c");
    }
}
