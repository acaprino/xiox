//! The app's side of `imago-shell`: where it keeps what the shell remembers, the two commands the page calls (the window
//! theme and the maximize button's rectangle for Snap Layouts), and where the window opens. The crate does the rest: the glass
//! window, the theme and its acrylic, the navigation guard, Snap Layouts, the reload item, the WebView2 failures and the
//! clamping of a saved place to the monitors there are now (docs/desktop-shell.md of Imago, sections 2 to 4).

use imago_shell::{apply_theme, clamp_to_screens, drawn_by_page, parse_theme, save_theme, Rect, Screen, SnapLayouts, WindowState};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// The smallest window the app is designed for, in logical pixels (the same as `minWidth` and `minHeight` in
/// `tauri.conf.json`): 500 wide, the canon's minimum (the snap zones need it, and the three breakpoints of the page, 1000, 850 and
/// 620, are for the widths between), and it fits a 1366x768 laptop at 125 %: 1093 x 574 logical once the 48 px Windows 11
/// taskbar is out, and the page plus the title bar and borders (about 31 logical pixels high) must fit in that.
pub const MIN_WIDTH: f64 = 500.0;
pub const MIN_HEIGHT: f64 = 540.0;

/// The folder the shell keeps the window theme (`window-theme`) and the window's place (`window.json`) in: next to the
/// Developers `settings.json`. None for the fixtures window, which reads and writes nothing of the user's, and under
/// `--only-root`, which promises that no file outside its folder is written: then no theme and no place are read or kept.
pub fn data_dir(fake: bool, only_root: bool) -> Option<PathBuf> {
    if fake || only_root {
        return None;
    }
    xiox_app::settings_path().parent().map(PathBuf::from)
}

/// The folder of [`data_dir`], as the commands see it.
pub struct DataDir(pub Option<PathBuf>);

/// The Snap Layouts overlay of each window, by label. A window built again gets its entry replaced.
#[derive(Default)]
pub struct Overlays(pub Mutex<HashMap<String, SnapLayouts>>);

/// The page tells the window the theme of its look (`light`, `dark` or `system`), and the window keeps it for the first frame
/// of the next start. With it come the look (`smooth` or `oni`) and the theme the page draws (`light` or `dark`), from which
/// the window tints its acrylic on Windows 10; what an older page leaves out, `drawn_by_page` fills in. Nothing is kept when
/// there is no data folder (the fixtures window, `--only-root`).
#[tauri::command]
pub fn set_window_theme(window: tauri::WebviewWindow, dir: State<'_, DataDir>, theme: String, look: Option<String>, resolved: Option<String>) -> Result<(), String> {
    let theme = parse_theme(&theme).ok_or_else(|| format!("unknown window theme \"{theme}\""))?;
    let (look, resolved) = drawn_by_page(theme, look.as_deref(), resolved.as_deref());
    apply_theme(&window, theme, look, resolved).map_err(|error| error.to_string())?;
    match &dir.0 {
        Some(dir) => save_theme(dir, theme).map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// The page reports where its maximize button is, in physical pixels of the client area: the numbers go to the Snap Layouts
/// overlay of the window as they come. An error is sent back so the page sends the rectangle again at its next change (the
/// overlay does not exist yet while the window is still being built).
#[tauri::command]
pub fn window_caption_maximize(window: tauri::WebviewWindow, overlays: State<'_, Overlays>, x: i32, y: i32, width: i32, height: i32) -> Result<(), String> {
    let overlays = overlays.0.lock().map_err(|_| "overlays are unavailable".to_string())?;
    let overlay = overlays.get(window.label()).ok_or_else(|| format!("window \"{}\" has no Snap Layouts overlay yet", window.label()))?;
    overlay.set_rect(x, y, width, height);
    Ok(())
}

/// The monitors with their work areas (the screen without the taskbar) in physical pixels, the primary one first (a place on
/// no monitor goes to the first).
pub fn screens(window: &tauri::WebviewWindow) -> Vec<Screen> {
    let mut monitors = window.available_monitors().unwrap_or_default();
    if let Some(primary) = window.primary_monitor().ok().flatten()
        && let Some(at) = monitors.iter().position(|m| m.position() == primary.position() && m.size() == primary.size())
    {
        monitors.swap(0, at);
    }
    monitors
        .iter()
        .map(|monitor| {
            let work = monitor.work_area();
            Screen {
                work: Rect { x: work.position.x, y: work.position.y, width: work.size.width, height: work.size.height },
                full: Rect { x: monitor.position().x, y: monitor.position().y, width: monitor.size().width, height: monitor.size().height },
            }
        })
        .collect()
}

/// Where the window opens: its state, and whether the window is to be centred instead of placed at the state's corner.
#[derive(Debug, PartialEq, Eq)]
pub struct Placed {
    pub state: WindowState,
    pub centred: bool,
}

/// The window's place at the start, in physical pixels. A place saved (`window.json`) comes back inside the work area of its
/// monitor, at least the minimum (a window saved on a monitor that is gone returns on the primary one at its size, not
/// centred). With no place saved (a first launch, or a run that remembers nothing) the size the window was configured with
/// is brought inside the primary monitor's work area and the window is centred. `size` is that size, `scale` the monitor's
/// scale factor, `frame` the window's outer size minus its inner size.
pub fn place(saved: Option<WindowState>, size: (u32, u32), screens: &[Screen], scale: f64, frame: (u32, u32)) -> Placed {
    let minimum = |logical: f64| (logical * scale).round() as u32;
    let (min_width, min_height) = (minimum(MIN_WIDTH), minimum(MIN_HEIGHT));
    match saved {
        Some(state) => Placed { state: clamp_to_screens(state, screens, frame, min_width, min_height), centred: false },
        None => {
            let corner = screens.first().map_or((0, 0), |screen| (screen.work.x, screen.work.y));
            let first = WindowState { x: corner.0, y: corner.1, width: size.0, height: size.1, maximized: false };
            Placed { state: clamp_to_screens(first, screens, frame, min_width, min_height), centred: true }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Screen = Screen {
        work: Rect { x: 0, y: 0, width: 1920, height: 1032 },
        full: Rect { x: 0, y: 0, width: 1920, height: 1080 },
    };
    const SECOND: Screen = Screen {
        work: Rect { x: 1920, y: 0, width: 1280, height: 1024 },
        full: Rect { x: 1920, y: 0, width: 1280, height: 1024 },
    };
    const FRAME: (u32, u32) = (0, 31);

    fn state(x: i32, y: i32, width: u32, height: u32) -> WindowState {
        WindowState { x, y, width, height, maximized: false }
    }

    #[test]
    fn a_first_launch_is_centred_and_inside_the_work_area() {
        let placed = place(None, (1280, 800), &[SCREEN], 1.0, FRAME);
        assert!(placed.centred);
        assert_eq!((placed.state.width, placed.state.height), (1280, 800), "the configured size fits");
        // A configured size bigger than a small screen is brought down to it, frame included.
        let small = Screen::whole(Rect { x: 0, y: 0, width: 1366, height: 768 });
        let placed = place(None, (1920, 1200), &[small], 1.25, FRAME);
        assert!(placed.centred);
        assert_eq!((placed.state.width, placed.state.height), (1366, 768 - 31));
    }

    #[test]
    fn a_saved_place_comes_back_where_it_was() {
        let saved = state(200, 100, 1280, 800);
        assert_eq!(place(Some(saved), (1280, 800), &[SCREEN], 1.0, FRAME), Placed { state: saved, centred: false });
        let maximized = WindowState { maximized: true, ..saved };
        assert!(place(Some(maximized), (1280, 800), &[SCREEN], 1.0, FRAME).state.maximized, "the maximized state is kept");
    }

    #[test]
    fn a_saved_place_is_never_under_the_minimum_times_the_scale() {
        let placed = place(Some(state(10, 10, 400, 300)), (1280, 800), &[SCREEN], 1.5, FRAME);
        assert_eq!((placed.state.width, placed.state.height), (750, 810), "500 x 540 at 150 %");
    }

    #[test]
    fn a_place_on_a_monitor_that_is_gone_returns_on_the_primary_one() {
        // Saved on a third monitor that is unplugged: its corner is on none of the two there are.
        let placed = place(Some(state(4000, 200, 1280, 800)), (1280, 800), &[SCREEN, SECOND], 1.0, FRAME);
        assert!(!placed.centred);
        assert_eq!((placed.state.width, placed.state.height), (1280, 800), "at its size");
        assert!(placed.state.x >= 0 && placed.state.x + 1280 <= 1920, "inside the primary monitor, not centred: {placed:?}");
        // Saved on the second monitor, which is still there: it stays on it.
        let placed = place(Some(state(2000, 100, 1000, 700)), (1280, 800), &[SCREEN, SECOND], 1.0, FRAME);
        assert_eq!(placed.state.x, 2000);
    }

    #[test]
    fn the_data_folder_is_the_settings_folder_unless_the_run_remembers_nothing() {
        assert!(data_dir(false, false).is_some());
        assert_eq!(data_dir(false, false).as_deref(), xiox_app::settings_path().parent(), "next to settings.json, where window.json always was");
        assert_eq!(data_dir(true, false), None, "the fixtures window reads and writes nothing of the user's");
        assert_eq!(data_dir(false, true), None, "--only-root writes no file outside its folder");
        assert_eq!(data_dir(true, true), None);
    }
}
