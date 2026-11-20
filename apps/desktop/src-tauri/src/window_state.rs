//! The remembered window: size, position, maximized and monitor, kept in `<data>/window.json`.
//! Pure logic and file I/O only, so it can be tested without a window. All values are physical
//! pixels; the position is the outer top-left corner, the size the inner (client) size.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// File name inside the app's data folder.
pub const FILE_NAME: &str = "window.json";

/// Smaller saved sizes are treated as broken and ignored.
const MIN_W: u32 = 400;
const MIN_H: u32 = 300;
/// How much of the window must lie on a monitor to count as reachable.
const VISIBLE_W: i64 = 120;
const VISIBLE_H: i64 = 80;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    /// Name of the monitor the window was on (informational; placement checks the rectangle).
    #[serde(default)]
    pub monitor: Option<String>,
}

/// A monitor's rectangle in physical pixels (virtual desktop coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Where the window goes at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// What the window looks like right now, as read from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    pub minimized: bool,
    pub monitor: Option<String>,
}

/// Reads the saved state; a missing, unreadable or broken file gives `None` (= default window).
pub fn load(path: &Path) -> Option<WindowState> {
    let text = std::fs::read_to_string(path).ok()?;
    let state: WindowState = serde_json::from_str(&text).ok()?;
    (state.width >= MIN_W && state.height >= MIN_H).then_some(state)
}

/// Writes the state through a temporary file, so a crash never leaves a half-written file.
pub fn save(path: &Path, state: &WindowState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

/// The next state to keep after the window changed. `None` = keep what is saved (minimized
/// is never stored). While maximized the last normal rectangle is kept, so un-maximizing after
/// a restart returns to it.
pub fn update(prev: Option<&WindowState>, now: &Observed) -> Option<WindowState> {
    if now.minimized || now.width == 0 || now.height == 0 {
        return None;
    }
    if now.maximized {
        if let Some(prev) = prev {
            return Some(WindowState { maximized: true, monitor: now.monitor.clone(), ..prev.clone() });
        }
    }
    Some(WindowState {
        x: now.x,
        y: now.y,
        width: now.width,
        height: now.height,
        maximized: now.maximized,
        monitor: now.monitor.clone(),
    })
}

/// Where to put the window at start. `None` = no saved state or no monitor known: the default
/// size, centered. A window that would not be reachable on any current monitor (monitor
/// unplugged, resolution changed) is centered on the primary monitor, shrunk to fit if needed.
pub fn place(saved: Option<&WindowState>, monitors: &[Rect], primary: Option<Rect>) -> Option<Placement> {
    let saved = saved?;
    if monitors.iter().any(|m| reachable(saved, m)) {
        return Some(Placement {
            x: saved.x,
            y: saved.y,
            width: saved.width,
            height: saved.height,
            maximized: saved.maximized,
        });
    }
    let m = primary.or_else(|| monitors.first().copied())?;
    let width = saved.width.min(m.width);
    let height = saved.height.min(m.height);
    Some(Placement {
        x: m.x + ((m.width - width) / 2) as i32,
        y: m.y + ((m.height - height) / 2) as i32,
        width,
        height,
        maximized: saved.maximized,
    })
}

/// A big enough part of the window lies on the monitor and its top edge (the title bar) is on it.
fn reachable(s: &WindowState, m: &Rect) -> bool {
    let (wx0, wy0) = (i64::from(s.x), i64::from(s.y));
    let (wx1, wy1) = (wx0 + i64::from(s.width), wy0 + i64::from(s.height));
    let (mx0, my0) = (i64::from(m.x), i64::from(m.y));
    let (mx1, my1) = (mx0 + i64::from(m.width), my0 + i64::from(m.height));
    let w = wx1.min(mx1) - wx0.max(mx0);
    let h = wy1.min(my1) - wy0.max(my0);
    let title_on_screen = wy0 >= my0 && wy0 < my1 - 20;
    w >= VISIBLE_W.min(i64::from(s.width)) && h >= VISIBLE_H.min(i64::from(s.height)) && title_on_screen
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect { x: 0, y: 0, width: 2560, height: 1440 };
    const RIGHT: Rect = Rect { x: 2560, y: 0, width: 1920, height: 1080 };

    fn state(x: i32, y: i32, maximized: bool) -> WindowState {
        WindowState { x, y, width: 1200, height: 820, maximized, monitor: Some("DISPLAY1".into()) }
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rp-desktop-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join(FILE_NAME)
    }

    #[test]
    fn keeps_position_on_a_present_monitor() {
        let s = state(2800, 100, false);
        let p = place(Some(&s), &[MAIN, RIGHT], Some(MAIN)).unwrap();
        assert_eq!((p.x, p.y, p.width, p.height, p.maximized), (2800, 100, 1200, 820, false));
    }

    #[test]
    fn unplugged_monitor_falls_back_to_primary_center() {
        let s = state(2800, 100, true);
        let p = place(Some(&s), &[MAIN], Some(MAIN)).unwrap();
        assert_eq!((p.x, p.y), ((2560 - 1200) / 2, (1440 - 820) / 2));
        assert!(p.maximized);
    }

    #[test]
    fn too_big_window_is_shrunk_to_the_primary_monitor() {
        let s = WindowState { x: -5000, y: 0, width: 3000, height: 2000, maximized: false, monitor: None };
        let small = Rect { x: 0, y: 0, width: 1920, height: 1080 };
        let p = place(Some(&s), &[small], Some(small)).unwrap();
        assert_eq!((p.x, p.y, p.width, p.height), (0, 0, 1920, 1080));
    }

    #[test]
    fn title_bar_above_the_screen_is_not_reachable() {
        let s = state(100, -600, false);
        let p = place(Some(&s), &[MAIN], Some(MAIN)).unwrap();
        assert_eq!((p.x, p.y), (680, 310));
    }

    #[test]
    fn barely_visible_corner_is_not_reachable() {
        let s = state(2560 - 50, 100, false);
        let p = place(Some(&s), &[MAIN], Some(MAIN)).unwrap();
        assert_eq!(p.x, 680);
    }

    #[test]
    fn nothing_saved_or_no_monitor_gives_default() {
        assert_eq!(place(None, &[MAIN], Some(MAIN)), None);
        assert_eq!(place(Some(&state(9999, 9999, false)), &[], None), None);
    }

    #[test]
    fn save_and_load_round_trip() {
        let path = temp_file("roundtrip");
        let s = state(10, 20, true);
        save(&path, &s).unwrap();
        assert_eq!(load(&path), Some(s));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn broken_missing_or_tiny_file_gives_default() {
        let path = temp_file("broken");
        assert_eq!(load(&path), None);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, br#"{"x":0,"y":0,"width":10,"height":10,"maximized":false}"#).unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, br#"{"x":0,"y":0,"width":800,"height":600,"maximized":false}"#).unwrap();
        assert_eq!(load(&path).map(|s| s.monitor), Some(None));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn observed(maximized: bool, minimized: bool) -> Observed {
        Observed { x: 0, y: 0, width: 2560, height: 1400, maximized, minimized, monitor: Some("DISPLAY1".into()) }
    }

    #[test]
    fn minimized_is_never_stored() {
        assert_eq!(update(Some(&state(10, 20, false)), &observed(false, true)), None);
    }

    #[test]
    fn maximized_keeps_the_normal_rectangle() {
        let next = update(Some(&state(10, 20, false)), &observed(true, false)).unwrap();
        assert_eq!(next, state(10, 20, true));
    }

    #[test]
    fn normal_window_takes_the_observed_rectangle() {
        let next = update(Some(&state(10, 20, true)), &observed(false, false)).unwrap();
        assert_eq!((next.width, next.height, next.maximized), (2560, 1400, false));
    }
}
