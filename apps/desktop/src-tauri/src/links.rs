//! Navigation guard of the window: it only ever shows the app's own server
//! (`http://127.0.0.1:<port>`). A link to anywhere else (Hugging Face, a card site, the project
//! page) opens in the system browser, and the window stays on the app; anything that is neither
//! the app nor a web address is refused.

use tauri::Url;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Nav {
    /// The app's own page: shown in the window.
    Stay,
    /// A web page elsewhere: handed to the system browser.
    External,
    /// Anything else (`file:`, `javascript:`, other schemes): refused.
    Block,
}

/// Where `url` goes, for a window that shows `own` (the server's local address).
pub(crate) fn decide(url: &Url, own: &Url) -> Nav {
    if url.origin() == own.origin() {
        Nav::Stay
    } else if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() && url.username().is_empty() && url.password().is_none() {
        Nav::External
    } else {
        Nav::Block
    }
}

/// Opens a web address in the system's default browser.
#[cfg(windows)]
pub(crate) fn open_external(url: &Url) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url.as_str()));
    // SAFETY: both strings are NUL-terminated UTF-16 buffers that live across the call; no window
    // handle, parameters or directory are passed. `decide` only lets http(s) addresses through.
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

#[cfg(not(windows))]
pub(crate) fn open_external(url: &Url) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(url.as_str()).spawn();
}

/// `on_navigation`: true keeps the navigation in the window.
pub(crate) fn guard(url: &Url, own: &Url) -> bool {
    match decide(url, own) {
        Nav::Stay => true,
        Nav::External => {
            open_external(url);
            false
        }
        Nav::Block => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(url: &str) -> Nav {
        let own: Url = "http://127.0.0.1:51234/?token=abc".parse().unwrap();
        decide(&url.parse().unwrap(), &own)
    }

    #[test]
    fn only_the_own_server_stays() {
        for url in ["http://127.0.0.1:51234/", "http://127.0.0.1:51234/?token=abc#/chars", "http://127.0.0.1:51234/api/characters/1/avatar"] {
            assert_eq!(d(url), Nav::Stay, "{url}");
        }
        for url in [
            "https://huggingface.co/org/model",
            "https://chub.ai/characters/x",
            "http://127.0.0.1:51235/",
            "http://localhost:51234/",
            "http://192.168.1.20:7860/?token=x",
            "https://127.0.0.1:51234/",
        ] {
            assert_eq!(d(url), Nav::External, "{url}");
        }
        for url in ["file:///C:/Windows/win.ini", "javascript:alert(1)", "ms-settings:privacy", "data:text/html,x", "https://user:pw@example.com/"] {
            assert_eq!(d(url), Nav::Block, "{url}");
        }
    }
}
