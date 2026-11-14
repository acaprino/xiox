//! The window remembers where it was: size, position and whether it was maximized, in
//! `%APPDATA%\Xiox\window.json` next to the Developers `settings.json`. A geometry that no longer
//! fits the screens (a monitor was unplugged, a corrupt file) is ignored and the window centres.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The smallest window the app is designed for (the same as `minWidth` / `minHeight` in `tauri.conf.json`).
const MIN_WIDTH: u32 = 1000;
const MIN_HEIGHT: u32 = 640;
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
        assert!(fits(&Geometry { width: 1000, height: 640, ..BIG }, &[SCREEN]));
        assert!(!fits(&Geometry { width: 999, ..BIG }, &[SCREEN]));
        assert!(!fits(&Geometry { height: 639, ..BIG }, &[SCREEN]));
        assert!(!fits(&Geometry { width: 0, height: 0, ..BIG }, &[SCREEN]));
    }
}
