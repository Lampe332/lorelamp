//! Starting with Windows (settings tab "Allgemein"): the desktop app is the program that can be
//! put into the user's autostart, and a start by Windows may come up minimized.

use tauri::WebviewWindow;

/// Before the server starts: this program offers "Mit Windows starten" (`rp_server::autostart`).
pub fn offer() {
    rp_server::autostart::offer();
}

/// After the window is placed and shown: a start with `--minimized` (the autostart entry with
/// "minimiert starten") puts it into the taskbar instead of in front.
pub fn apply(win: &WebviewWindow) {
    if rp_server::autostart::started_minimized(std::env::args()) {
        let _ = win.minimize();
    }
}
