#![cfg_attr(windows, windows_subsystem = "windows")]

#[allow(dead_code)]
mod error;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the Xiox window failed to start");
}
