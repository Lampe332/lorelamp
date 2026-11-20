//! "Mit Windows starten": one value in the user's own `Run` key that starts this program at
//! sign-in, optionally minimized (`--minimized`). The registry is the only place this is kept, so
//! what the settings show is what Windows will do.
//!
//! Only the desktop app offers it ([`offer`]); a plain `rp-server` and phones do not. A test
//! server points it at a scratch key with `RP_AUTOSTART_KEY` and never touches the real one.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The user's autostart programs (below `HKEY_CURRENT_USER`).
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// The value's name: how the entry shows in the Task Manager's autostart list.
const VALUE: &str = "Lorelamp";
/// Marks a start by Windows; the desktop app shows its window minimized with it.
pub const MINIMIZED_ARG: &str = "--minimized";

/// Where the entry lives and which program it starts.
struct Target {
    key: String,
    exe: PathBuf,
}

static TARGET: OnceLock<Option<Target>> = OnceLock::new();

fn target() -> Option<&'static Target> {
    TARGET
        .get_or_init(|| {
            let key = std::env::var("RP_AUTOSTART_KEY").ok().filter(|k| !k.trim().is_empty())?;
            Some(Target { key, exe: std::env::current_exe().ok()? })
        })
        .as_ref()
}

/// Called once by the desktop app before the server starts: this program can start with Windows.
pub fn offer() {
    let _ = TARGET.set(std::env::current_exe().ok().map(|exe| Target { key: RUN_KEY.into(), exe }));
}

/// What the settings show.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct State {
    pub on: bool,
    pub minimized: bool,
}

/// The command line Windows runs at sign-in.
fn command(exe: &Path, minimized: bool) -> String {
    let mut cmd = format!("\"{}\"", exe.display());
    if minimized {
        cmd.push(' ');
        cmd.push_str(MINIMIZED_ARG);
    }
    cmd
}

/// Reads an entry back: on only when it starts exactly `exe` (an entry left by a copy of the app
/// in another folder counts as off, and switching on replaces it).
fn read(cmd: &str, exe: &Path) -> State {
    let Some(rest) = cmd.trim().strip_prefix('"') else { return State::default() };
    let Some((path, args)) = rest.split_once('"') else { return State::default() };
    if !path.eq_ignore_ascii_case(&exe.display().to_string()) {
        return State::default();
    }
    State { on: true, minimized: args.split_whitespace().any(|a| a == MINIMIZED_ARG) }
}

/// `None`: this program does not offer it (no desktop app, or not Windows).
pub fn state() -> Option<State> {
    let t = target()?;
    Some(sys::get(&t.key, VALUE).map(|cmd| read(&cmd, &t.exe)).unwrap_or_default())
}

/// Writes or removes the entry. Err `unsupported` when this program does not offer it.
pub fn set(want: State) -> std::io::Result<()> {
    let t = target().ok_or_else(|| std::io::Error::other("unsupported"))?;
    if want.on {
        sys::put(&t.key, VALUE, &command(&t.exe, want.minimized))
    } else {
        sys::remove(&t.key, VALUE)
    }
}

/// The desktop app was started with [`MINIMIZED_ARG`] (by Windows at sign-in).
pub fn started_minimized(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().skip(1).any(|a| a == MINIMIZED_ARG)
}

#[cfg(windows)]
mod sys {
    use std::io;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn get(key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut bytes: u32 = 0;
        // SAFETY: null-terminated strings; a null buffer asks for the size only.
        let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut bytes) };
        if rc != ERROR_SUCCESS || bytes == 0 {
            return None;
        }
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
        // SAFETY: `buf` holds at least `bytes` bytes, as the call above reported.
        let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut bytes) };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    pub fn put(key: &str, value: &str, data: &str) -> io::Result<()> {
        let (key, value, data) = (wide(key), wide(value), wide(data));
        // SAFETY: null-terminated strings; the size is that of `data` in bytes, terminator included.
        let rc = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
        if rc == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(rc as i32)) }
    }

    pub fn remove(key: &str, value: &str) -> io::Result<()> {
        let (key, value) = (wide(key), wide(value));
        // SAFETY: null-terminated strings.
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(io::Error::from_raw_os_error(rc as i32)) }
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn get(_: &str, _: &str) -> Option<String> {
        None
    }
    pub fn put(_: &str, _: &str, _: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("unsupported"))
    }
    pub fn remove(_: &str, _: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("unsupported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_round_trip() {
        let exe = Path::new(r"C:\Program Files\Lorelamp\app.exe");
        assert_eq!(command(exe, false), r#""C:\Program Files\Lorelamp\app.exe""#);
        assert_eq!(command(exe, true), r#""C:\Program Files\Lorelamp\app.exe" --minimized"#);
        assert_eq!(read(&command(exe, false), exe), State { on: true, minimized: false });
        assert_eq!(read(&command(exe, true), exe), State { on: true, minimized: true });
        // Case does not matter in Windows paths; another program's entry is not ours.
        assert_eq!(read(r#""c:\program files\lorelamp\APP.EXE""#, exe), State { on: true, minimized: false });
        assert_eq!(read(r#""D:\old\app.exe" --minimized"#, exe), State::default());
        assert_eq!(read("", exe), State::default());
        assert_eq!(read(r"C:\unquoted\app.exe", exe), State::default());
    }

    #[test]
    fn minimized_only_by_argument() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(started_minimized(args(&["app.exe", "--minimized"])));
        assert!(!started_minimized(args(&["app.exe"])));
        assert!(!started_minimized(args(&["--minimized"])), "the program's own name is not an argument");
    }

    /// The real registry calls, in a scratch key of this test only (never the `Run` key).
    #[cfg(windows)]
    #[test]
    fn registry_value_is_written_read_and_removed() {
        let key = format!(r"Software\Lorelamp-test\autostart-{}", std::process::id());
        assert_eq!(sys::get(&key, VALUE), None);
        sys::put(&key, VALUE, r#""C:\x y\app.exe" --minimized"#).unwrap();
        assert_eq!(sys::get(&key, VALUE).as_deref(), Some(r#""C:\x y\app.exe" --minimized"#));
        sys::remove(&key, VALUE).unwrap();
        assert_eq!(sys::get(&key, VALUE), None);
        sys::remove(&key, VALUE).unwrap(); // removing what is not there is fine
        // Tidy up: the scratch key, then its parent (which only goes when it is empty).
        use windows_sys::Win32::System::Registry::{RegDeleteKeyW, HKEY_CURRENT_USER};
        for k in [key.as_str(), r"Software\Lorelamp-test"] {
            let wide: Vec<u16> = k.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: a null-terminated name of a key this test made.
            unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide.as_ptr()) };
        }
    }
}
