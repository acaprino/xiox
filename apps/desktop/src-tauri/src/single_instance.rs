//! One Xiox window at a time: a named mutex held for the life of the process. A second launch finds
//! the name taken, brings the first window to the front and exits.

use std::ptr::null;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};

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

/// Raises the window titled `title` (restoring it first when minimized), if there is one.
pub fn focus_existing(title: &str) {
    let title = wide(title);
    // SAFETY: `title` is NUL-terminated and outlives the calls; the handle is only passed back to the system.
    unsafe {
        let window = FindWindowW(null(), title.as_ptr());
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
