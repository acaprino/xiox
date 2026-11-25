//! The keyboard after a native dialog. A folder picker, a save dialog or a message closes and the window is active again, but
//! WebView2 does not always hand the keyboard back to the page: without this the arrows, Space and Enter do nothing until the
//! next click. The page's own `focus` handler (`src/xiox.tsx`) then puts the focus on the list.

use tauri::{Runtime, Webview};

/// Gives the window and then its page the focus again. Called by every command that opened a native dialog, once it closed.
pub fn give_back<R: Runtime>(webview: &Webview<R>) {
    let _ = webview.window().set_focus();
    let _ = webview.set_focus();
}
