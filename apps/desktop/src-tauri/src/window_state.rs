//! The window remembers where it was: size, position and whether it was maximized, in
//! `%APPDATA%\Xiox\window.json` next to the Developers `settings.json`. A geometry that no longer
//! fits the screens (a monitor was unplugged, a corrupt file) is ignored and the window centres.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The smallest window the app is designed for (the same as `minWidth` / `minHeight` in `tauri.conf.json`): it fits a
/// 1366x768 laptop at 125 % (1093 x 582 logical once the taskbar is taken out).
const MIN_WIDTH: u32 = 960;
const MIN_HEIGHT: u32 = 560;
/// How much of the window's top-left corner must lie on a monitor for the title bar to stay reachable.
const GRAB: i64 = 64;

/// Outer position and inner size in physical pixels.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// `window.json` next to the settings file.
pub fn path() -> PathBuf {
    path_in(&xiox_app::settings_path())
}

/// `window.json` next to the given settings file.
pub fn path_in(settings_file: &Path) -> PathBuf {
    settings_file.with_file_name("window.json")
}

/// Whether this run keeps the window's place at all: the fixtures window reads and writes nothing of
/// the user's, and `--only-root` writes no file outside its folder.
pub fn remembers(fake: bool, only_root: bool) -> bool {
    !fake && !only_root
}

/// What to remember when the window closes. A maximized or minimized window reports the screen
/// (or `(-32000, -32000)` and size 0), so the geometry is the last normal one, which is what
/// restoring brings back; only a normal window is measured (`live`).
pub fn to_save(maximized: bool, minimized: bool, normal: Option<Geometry>, live: impl FnOnce() -> Option<Geometry>) -> Option<Geometry> {
    if maximized || minimized {
        return normal.map(|g| Geometry { maximized, ..g });
    }
    live()
}

/// The saved geometry, or `None` when there is no file or it cannot be read.
pub fn load(path: &Path) -> Option<Geometry> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Writes the geometry to `<file>.tmp` and renames it over `path`, so a crash never leaves half a file.
pub fn save(path: &Path, g: &Geometry) -> Result<(), String> {
    let text = serde_json::to_string_pretty(g).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// Whether a saved geometry can be restored: at least the minimum size and its top-left
/// `GRAB` x `GRAB` corner inside one of the monitors `(x, y, width, height)`.
pub fn fits(g: &Geometry, monitors: &[(i32, i32, u32, u32)]) -> bool {
    if g.width < MIN_WIDTH || g.height < MIN_HEIGHT {
        return false;
    }
    let (x, y) = (i64::from(g.x), i64::from(g.y));
    monitors.iter().any(|&(mx, my, mw, mh)| {
        let (mx, my) = (i64::from(mx), i64::from(my));
        x >= mx && y >= my && x + GRAB <= mx + i64::from(mw) && y + GRAB <= my + i64::from(mh)
    })
}

/// A rectangle `(x, y, width, height)` in physical pixels: a monitor's work area (the screen without the taskbar).
pub type Rect = (i32, i32, u32, u32);

/// The geometry brought inside the work area that holds its top-left corner (the first one when none does):
/// never larger than the area, and moved so that the whole window, its frame included, is in it. `frame` is the
/// window's outer size minus its inner size (title bar and borders), because the geometry holds the inner size.
/// Without any work area the geometry is returned as it is.
pub fn clamp(g: Geometry, frame: (u32, u32), areas: &[Rect]) -> Geometry {
    let holds = |&(ax, ay, aw, ah): &Rect| g.x >= ax && g.y >= ay && i64::from(g.x) < i64::from(ax) + i64::from(aw) && i64::from(g.y) < i64::from(ay) + i64::from(ah);
    let Some(&(ax, ay, aw, ah)) = areas.iter().find(|a| holds(a)).or(areas.first()) else {
        return g;
    };
    let width = g.width.min(aw.saturating_sub(frame.0));
    let height = g.height.min(ah.saturating_sub(frame.1));
    let (outer_w, outer_h) = (i64::from(width) + i64::from(frame.0), i64::from(height) + i64::from(frame.1));
    let place = |at: i32, start: i32, room: u32, outer: i64| {
        let far = i64::from(start) + i64::from(room) - outer;
        i32::try_from(i64::from(at).clamp(i64::from(start), far.max(i64::from(start)))).unwrap_or(start)
    };
    Geometry { x: place(g.x, ax, aw, outer_w), y: place(g.y, ay, ah, outer_h), width, height, ..g }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (i32, i32, u32, u32) = (0, 0, 1920, 1080);
    const SECOND: (i32, i32, u32, u32) = (1920, 0, 1280, 1024);
    const BIG: Geometry = Geometry { x: 100, y: 80, width: 1280, height: 800, maximized: false };

    #[test]
    fn geometry_round_trips_through_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("window.json");
        assert_eq!(load(&file), None, "no file yet");
        let g = Geometry { x: -8, y: 12, width: 1400, height: 900, maximized: true };
        save(&file, &g).unwrap();
        assert_eq!(load(&file), Some(g));
        assert!(!file.with_extension("json.tmp").exists(), "the temporary file is renamed away");
        // A later save replaces the earlier one.
        save(&file, &BIG).unwrap();
        assert_eq!(load(&file), Some(BIG));
        // A file that is not a geometry is no geometry.
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(load(&file), None);
        // The file sits next to the settings.
        let settings = dir.path().join("Xiox").join("settings.json");
        assert_eq!(path_in(&settings), dir.path().join("Xiox").join("window.json"));
    }

    #[test]
    fn the_fixtures_window_and_an_only_root_run_remember_nothing() {
        assert!(remembers(false, false));
        assert!(!remembers(true, false));
        assert!(!remembers(false, true), "--only-root writes no file outside its folder");
    }

    #[test]
    fn a_minimized_or_maximized_window_saves_the_last_normal_geometry() {
        let live = || Some(Geometry { x: -32000, y: -32000, width: 0, height: 0, maximized: false });
        // Normal: measured now.
        assert_eq!(to_save(false, false, Some(BIG), live), live());
        // Minimized: the window reports (-32000, -32000) and size 0; the last normal one is kept.
        assert_eq!(to_save(false, true, Some(BIG), || panic!("a minimized window is not measured")), Some(BIG));
        // Maximized (also when minimized from maximized): the normal geometry, flagged maximized.
        assert_eq!(to_save(true, false, Some(BIG), || panic!("not measured")), Some(Geometry { maximized: true, ..BIG }));
        assert_eq!(to_save(true, true, Some(BIG), || panic!("not measured")), Some(Geometry { maximized: true, ..BIG }));
        // Nothing normal was ever seen: nothing is saved rather than a wrong place.
        assert_eq!(to_save(false, true, None, || panic!("not measured")), None);
    }

    #[test]
    fn a_window_off_every_monitor_is_not_restored() {
        assert!(fits(&BIG, &[SCREEN]));
        // Left on a monitor that is gone.
        assert!(!fits(&Geometry { x: 2500, ..BIG }, &[SCREEN]));
        assert!(fits(&Geometry { x: 2500, ..BIG }, &[SCREEN, SECOND]));
        // Only a sliver of the corner on the screen: the title bar cannot be grabbed.
        assert!(!fits(&Geometry { x: 1900, ..BIG }, &[SCREEN]));
        assert!(!fits(&Geometry { y: 1070, ..BIG }, &[SCREEN]));
        assert!(!fits(&Geometry { x: -1000, ..BIG }, &[SCREEN]));
        // No monitor at all: nothing to restore onto.
        assert!(!fits(&BIG, &[]));
    }

    #[test]
    fn a_window_below_the_minimum_is_not_restored() {
        assert!(fits(&Geometry { width: 960, height: 560, ..BIG }, &[SCREEN]));
        assert!(!fits(&Geometry { width: 959, ..BIG }, &[SCREEN]));
        assert!(!fits(&Geometry { height: 559, ..BIG }, &[SCREEN]));
        assert!(!fits(&Geometry { width: 0, height: 0, ..BIG }, &[SCREEN]));
    }

    /// The title bar and borders of a window at 100 %: 16 x 39 physical pixels around the page.
    const FRAME: (u32, u32) = (16, 39);
    /// A 1366x768 laptop at 100 %: the taskbar takes 40 pixels.
    const LAPTOP: Rect = (0, 0, 1366, 728);

    #[test]
    fn a_window_taller_than_the_work_area_is_shrunk_to_fit_with_its_frame() {
        let g = clamp(BIG, FRAME, &[LAPTOP]);
        assert_eq!((g.width, g.height), (1280, 728 - 39), "the page plus the title bar is the work area");
        assert_eq!((g.x, g.y), (70, 0), "moved up and left until the whole window is in");
        assert_eq!(g.maximized, BIG.maximized);
    }

    #[test]
    fn a_window_that_fits_is_left_alone() {
        let g = Geometry { x: 10, y: 10, width: 1000, height: 600, maximized: true };
        assert_eq!(clamp(g, FRAME, &[SCREEN]), g);
        // Without a work area there is nothing to clamp to.
        assert_eq!(clamp(BIG, FRAME, &[]), BIG);
    }

    #[test]
    fn a_window_wider_than_the_area_is_cut_to_it_and_clamped_on_the_monitor_it_is_on() {
        // On the second monitor (1280x1024 at x = 1920) the 1400-wide window is cut to the area and sits at its left edge.
        let wide = Geometry { x: 2000, y: 100, width: 1400, height: 900, maximized: false };
        let g = clamp(wide, FRAME, &[SCREEN, SECOND]);
        assert_eq!((g.x, g.width), (1920, 1280 - 16));
        assert_eq!((g.y, g.height), (85, 900), "the height fits; the window moves up just enough to have its frame in");
        // A corner on no monitor: the first work area decides.
        assert_eq!(clamp(Geometry { x: 9000, ..wide }, FRAME, &[SCREEN]).x, 1920 - 1400 - 16);
    }

    #[test]
    fn the_minimum_fits_a_1366x768_laptop_at_125_percent() {
        // 1366x768 at 125 %: 1093 x 614 logical, 1093 x 582 below the 40 px taskbar.
        assert!(MIN_WIDTH <= 1093 && MIN_HEIGHT <= 582, "{MIN_WIDTH}x{MIN_HEIGHT}");
    }
}
