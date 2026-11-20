//! Tauri glue for the remembered window (`window_state`) and the one-time bring-to-front at start.
//! The window is built hidden, placed from `<data>/window.json`, then shown. Changes are saved
//! 600 ms after the last move/resize and once more when the window is closed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow, Window, WindowEvent};

use crate::window_state::{self, Observed, Rect, WindowState};

const DEBOUNCE: Duration = Duration::from_millis(600);

/// Keeps the window state in memory and on disk.
pub struct Keeper {
    path: PathBuf,
    state: Mutex<Option<WindowState>>,
    /// Off until the saved state has been applied, so the hidden start-up window is not stored.
    ready: AtomicBool,
    changed: Mutex<Option<Sender<()>>>,
}

impl Keeper {
    pub fn new(data_dir: &std::path::Path) -> Arc<Self> {
        let path = data_dir.join(window_state::FILE_NAME);
        let state = window_state::load(&path);
        Arc::new(Self { path, state: Mutex::new(state), ready: AtomicBool::new(false), changed: Mutex::new(None) })
    }

    /// Places the hidden window as remembered, shows it and brings it to the front once.
    pub fn restore<R: Runtime>(self: &Arc<Self>, win: &WebviewWindow<R>) {
        let monitors: Vec<Rect> = win.available_monitors().unwrap_or_default().iter().map(rect).collect();
        let primary = win.primary_monitor().ok().flatten().map(|m| rect(&m));
        let saved = self.state.lock().unwrap().clone();
        let maximized = match window_state::place(saved.as_ref(), &monitors, primary) {
            Some(p) => {
                let _ = win.set_size(PhysicalSize::new(p.width, p.height));
                let _ = win.set_position(PhysicalPosition::new(p.x, p.y));
                p.maximized
            }
            None => false,
        };
        if maximized {
            let _ = win.maximize();
        }
        let _ = win.show();
        let _ = win.unminimize();
        bring_to_front(win, maximized);
        let _ = win.set_focus();
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Starts the thread that saves after changes have settled.
    pub fn start_saver<R: Runtime>(self: &Arc<Self>, app: AppHandle<R>) {
        let (tx, rx) = mpsc::channel::<()>();
        *self.changed.lock().unwrap() = Some(tx);
        let keeper = self.clone();
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                // Wait until no change came for DEBOUNCE.
                loop {
                    match rx.recv_timeout(DEBOUNCE) {
                        Ok(()) => continue,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                if let Some(win) = app.get_webview_window("main") {
                    keeper.capture_and_save(&win.as_ref().window());
                }
            }
        });
    }

    /// Handles the window's events: moves/resizes are saved later, closing saves now.
    pub fn on_event<R: Runtime>(&self, win: &Window<R>, event: &WindowEvent) {
        if !self.ready.load(Ordering::SeqCst) {
            return;
        }
        match event {
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                if let Some(tx) = self.changed.lock().unwrap().as_ref() {
                    let _ = tx.send(());
                }
            }
            WindowEvent::CloseRequested { .. } => self.capture_and_save(win),
            _ => {}
        }
    }

    fn capture_and_save<R: Runtime>(&self, win: &Window<R>) {
        let (Ok(pos), Ok(size)) = (win.outer_position(), win.inner_size()) else { return };
        let now = Observed {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized: win.is_maximized().unwrap_or(false),
            minimized: win.is_minimized().unwrap_or(false),
            monitor: win.current_monitor().ok().flatten().and_then(|m| m.name().cloned()),
        };
        let mut state = self.state.lock().unwrap();
        let Some(next) = window_state::update(state.as_ref(), &now) else { return };
        if state.as_ref() == Some(&next) {
            return;
        }
        if let Err(e) = window_state::save(&self.path, &next) {
            eprintln!("saving the window state failed: {e}");
        }
        *state = Some(next);
    }
}

fn rect(m: &tauri::Monitor) -> Rect {
    let (p, s) = (m.position(), m.size());
    Rect { x: p.x, y: p.y, width: s.width, height: s.height }
}

/// Windows refuses `SetForegroundWindow` to a process that is not in the foreground (the launch
/// came from a script or a shortcut while another window was active), and the first `ShowWindow`
/// of a process may take the start mode from the launcher (minimized). So: show it explicitly,
/// attach to the foreground thread's input for a moment, lift it via topmost and back, and take
/// the foreground. Called only once at start.
#[cfg(windows)]
fn bring_to_front<R: Runtime>(win: &WebviewWindow<R>, maximized: bool) {
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SetForegroundWindow, SetWindowPos,
        ShowWindow, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE,
        SW_SHOWMAXIMIZED,
    };

    let Ok(hwnd) = win.hwnd() else { return };
    let hwnd = hwnd.0 as windows_sys::Win32::Foundation::HWND;
    let cmd = if maximized { SW_SHOWMAXIMIZED } else { SW_RESTORE };
    // SAFETY: `hwnd` is the live top-level window of this process, and this runs on its own UI
    // thread (Tauri's setup). The calls only change z-order, show state and input attachment; the
    // attachment is undone before returning.
    unsafe {
        let me = GetCurrentThreadId();
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
        ShowWindow(hwnd, cmd);
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, cmd);
        }
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
        SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        if attached {
            AttachThreadInput(me, fg_thread, 0);
        }
    }
}

#[cfg(not(windows))]
fn bring_to_front<R: Runtime>(_win: &WebviewWindow<R>, _maximized: bool) {}
