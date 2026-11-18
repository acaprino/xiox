//! The title bar of Smooth is the page's (the window has no frame of its own in that look): what Windows gives a framed
//! window and an HTML bar cannot, done here. Ported from Shinatama (`apps/desktop/src-tauri/src/native_frame.rs`).
//!
//! Snap Layouts: WebView2 covers the whole client area, so Windows never asks the window what lies under the cursor. A
//! child window that never paints lies over the HTML maximize button and answers `HTMAXBUTTON`: Windows 11 offers its
//! layouts on hover, and a click maximizes or restores. It tells the page about hover and press (`caption-maximize`), so
//! the HTML button draws them. The keyboard stays with the HTML button (Enter and Space press it).
#![cfg_attr(not(windows), allow(dead_code, unused_variables))]

use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

static APP: OnceLock<AppHandle> = OnceLock::new();

/// The page reports where its maximize button is, in physical pixels of the client area, on every layout change; an empty
/// rectangle (the button hidden: ONI MODE, which has the system frame) takes it out of Windows' hit test.
#[tauri::command]
pub fn window_caption_maximize(window: WebviewWindow, x: i32, y: i32, width: i32, height: i32) -> Result<(), String> {
    let _ = APP.set(window.app_handle().clone());
    #[cfg(windows)]
    {
        let hwnd = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
        window
            .run_on_main_thread(move || {
                // SAFETY: on the thread that owns the window, which is checked to be alive.
                unsafe { win::place_overlay(hwnd as _, x, y, width, height) };
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Hover and press of the overlay, for the HTML button to draw.
fn report_caption(hover: bool, pressed: bool) {
    if let Some(app) = APP.get()
        && let Some(main) = app.get_webview_window("main")
    {
        let _ = main.emit("caption-maximize", serde_json::json!({ "hover": hover, "pressed": pressed }));
    }
}

#[cfg(windows)]
mod win {
    use super::report_caption;
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::{
        Win32::{
            Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM},
            Graphics::Gdi::{GetStockObject, NULL_BRUSH, ValidateRect},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
                Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent},
                WindowsAndMessaging::{
                    CreateWindowExW, DefWindowProcW, GetParent, GetPropW, HTMAXBUTTON, HWND_TOP, IDC_ARROW, IsWindow, IsZoomed,
                    LoadCursorW, RegisterClassExW, RemovePropW, SC_MAXIMIZE, SC_RESTORE, SM_CYFRAME, SW_HIDE, SWP_NOACTIVATE,
                    SWP_SHOWWINDOW, SendMessageW, SetCursor, SetPropW, SetWindowPos, ShowWindow, WM_DESTROY, WM_ERASEBKGND,
                    WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_PAINT, WM_SETCURSOR,
                    WM_SYSCOMMAND, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
                },
            },
        },
        core::w,
    };

    const OVERLAY_CLASS: *const u16 = w!("XioxMaximizeOverlay");
    const OVERLAY_PROP: *const u16 = w!("XioxMaximizeOverlay");
    static OVERLAY_CLASS_REGISTERED: Once = Once::new();
    static HOVER: AtomicBool = AtomicBool::new(false);
    static PRESSED: AtomicBool = AtomicBool::new(false);

    fn report() {
        report_caption(HOVER.load(Ordering::Acquire), PRESSED.load(Ordering::Acquire));
    }

    pub unsafe fn place_overlay(parent: HWND, x: i32, y: i32, width: i32, height: i32) {
        unsafe {
            if IsWindow(parent) == 0 {
                return;
            }
            let mut overlay = GetPropW(parent, OVERLAY_PROP) as HWND;
            if width <= 0 || height <= 0 {
                if !overlay.is_null() {
                    ShowWindow(overlay, SW_HIDE);
                }
                return;
            }
            if overlay.is_null() {
                let instance = GetModuleHandleW(std::ptr::null());
                OVERLAY_CLASS_REGISTERED.call_once(|| {
                    let class = WNDCLASSEXW {
                        cbSize: size_of::<WNDCLASSEXW>() as u32,
                        lpfnWndProc: Some(overlay_proc),
                        hInstance: instance,
                        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                        // Never painted: the HTML button shows through.
                        hbrBackground: GetStockObject(NULL_BRUSH),
                        lpszClassName: OVERLAY_CLASS,
                        ..std::mem::zeroed()
                    };
                    RegisterClassExW(&class);
                });
                // The window text is the name UI Automation reads for the hit-test area.
                overlay = CreateWindowExW(
                    0,
                    OVERLAY_CLASS,
                    w!("Maximize"),
                    WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                    x,
                    y,
                    width,
                    height,
                    parent,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if overlay.is_null() {
                    return;
                }
                SetPropW(parent, OVERLAY_PROP, overlay as HANDLE);
            }
            // A restored window keeps its top border for resizing, as Windows' own buttons do.
            let top = if IsZoomed(parent) != 0 { y } else { y.max(GetSystemMetricsForDpi(SM_CYFRAME, GetDpiForWindow(parent))) };
            // Above the webview and Tauri's resize border, both created before it.
            SetWindowPos(overlay, HWND_TOP, x, top, width, (y + height - top).max(1), SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }

    unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: every call below acts on this live window or its parent.
        unsafe {
            match msg {
                WM_NCHITTEST => {
                    if !HOVER.swap(true, Ordering::AcqRel) {
                        let mut track = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE | TME_NONCLIENT,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        TrackMouseEvent(&mut track);
                        report();
                    }
                    HTMAXBUTTON as LRESULT
                }
                WM_NCMOUSELEAVE => {
                    HOVER.store(false, Ordering::Release);
                    PRESSED.store(false, Ordering::Release);
                    report();
                    0
                }
                WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam == HTMAXBUTTON as WPARAM => {
                    PRESSED.store(true, Ordering::Release);
                    report();
                    0
                }
                WM_NCLBUTTONUP if wparam == HTMAXBUTTON as WPARAM => {
                    // Released where it was pressed: leaving the button first cancels, as in Windows.
                    if PRESSED.swap(false, Ordering::AcqRel) {
                        report();
                        let parent = GetParent(hwnd);
                        let command = if IsZoomed(parent) != 0 { SC_RESTORE } else { SC_MAXIMIZE };
                        SendMessageW(parent, WM_SYSCOMMAND, command as WPARAM, 0);
                    }
                    0
                }
                WM_SETCURSOR => {
                    SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
                    1
                }
                WM_ERASEBKGND => 1,
                WM_PAINT => {
                    ValidateRect(hwnd, std::ptr::null());
                    0
                }
                WM_DESTROY => {
                    let parent = GetParent(hwnd);
                    if !parent.is_null() && GetPropW(parent, OVERLAY_PROP) as HWND == hwnd {
                        RemovePropW(parent, OVERLAY_PROP);
                    }
                    HOVER.store(false, Ordering::Release);
                    PRESSED.store(false, Ordering::Release);
                    0
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    }
}
