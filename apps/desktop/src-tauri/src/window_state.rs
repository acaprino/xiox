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
    xiox_app::settings_path().with_file_name("window.json")
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
        assert_eq!(path().file_name().unwrap(), "window.json");
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
