//! Lorelamp for Android. The app does one thing: it finds Lorelamp on a PC in the same network
//! (QR code, or search plus a six-digit code), remembers the address and then shows the PC's
//! interface in its WebView.
//!
//! The start page (`../ui`) is the only page with access to the commands below. The PC's
//! interface is a remote page: no capability names it, so it cannot call anything here. It
//! comes back to the start page by navigating to `http://tauri.localhost/?rescan=1`.

mod link;
mod net;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use link::{Fail, Found, Link, Offer};

/// An address handed in from outside (`lorelamp://connect?u=…`) that the start page has not
/// picked up yet.
#[derive(Default)]
struct Pending(Mutex<Option<String>>);

fn store_path(app: &AppHandle) -> Result<PathBuf, Fail> {
    Ok(app.path().app_data_dir().map_err(|_| Fail::Storage)?.join("connection.json"))
}

fn load(app: &AppHandle) -> Option<Link> {
    let text = std::fs::read_to_string(store_path(app).ok()?).ok()?;
    let url = serde_json::from_str::<serde_json::Value>(&text).ok()?.get("url")?.as_str()?.to_owned();
    link::parse_link(&url, net::LOOPBACK).ok()
}

fn save(app: &AppHandle, link: &Link) -> Result<(), Fail> {
    let path = store_path(app)?;
    let dir = path.parent().ok_or(Fail::Storage)?;
    std::fs::create_dir_all(dir).map_err(|_| Fail::Storage)?;
    // Written next to the file and moved over it, so a kill in between leaves the old one whole.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::json!({ "url": link.url() }).to_string()).map_err(|_| Fail::Storage)?;
    std::fs::rename(&tmp, &path).map_err(|_| Fail::Storage)
}

#[derive(Serialize)]
struct Startup {
    /// A link from outside that waits for the user's yes: which PC, and which remembered one a yes
    /// replaces.
    offer: Option<Offer>,
    /// A link from outside that failed the address check: why (it is dropped).
    offer_fail: Option<Fail>,
    /// Whether a PC is remembered.
    saved: bool,
}

fn pending_link(pending: &Pending) -> Option<String> {
    pending.0.lock().ok().and_then(|p| p.clone())
}

fn drop_pending(pending: &Pending) {
    if let Ok(mut p) = pending.0.lock() {
        *p = None;
    }
}

#[tauri::command]
fn startup(app: AppHandle, pending: State<'_, Pending>) -> Startup {
    let saved = load(&app);
    let (mut offer, mut offer_fail) = (None, None);
    if let Some(raw) = pending_link(&pending) {
        match link::offer(&raw, saved.as_ref(), net::LOOPBACK) {
            Ok((_, o)) => offer = Some(o),
            Err(e) => {
                offer_fail = Some(e);
                drop_pending(&pending);
            }
        }
    }
    Startup { offer, offer_fail, saved: saved.is_some() }
}

/// The user said yes to the link from outside: checked again, asked at the PC, remembered.
#[tauri::command]
async fn accept_link(app: AppHandle) -> Result<String, Fail> {
    let raw = {
        let pending = app.state::<Pending>();
        let raw = pending_link(&pending).ok_or(Fail::NotALink)?;
        drop_pending(&pending);
        raw
    };
    let (link, _) = link::offer(&raw, None, net::LOOPBACK)?;
    adopt(&app, link).await
}

/// The user said no: the link is forgotten, the remembered PC stays.
#[tauri::command]
fn decline_link(pending: State<'_, Pending>) {
    drop_pending(&pending);
}

/// Checks an address, asks the PC, remembers it and returns what the WebView has to open.
async fn adopt(app: &AppHandle, link: Link) -> Result<String, Fail> {
    net::check(&link).await?;
    save(app, &link)?;
    Ok(link.open_url())
}

/// A scanned QR code (the user pointed the camera at it, so it connects at once).
#[tauri::command]
async fn connect(app: AppHandle, raw: String) -> Result<String, Fail> {
    let raw = link::deep_link_target(&raw).unwrap_or(raw);
    adopt(&app, link::parse_link(&raw, net::LOOPBACK)?).await
}

/// The remembered PC.
#[tauri::command]
async fn open_saved(app: AppHandle) -> Result<String, Fail> {
    let link = load(&app).ok_or(Fail::NoneSaved)?;
    net::check(&link).await?;
    Ok(link.open_url())
}

/// Looks for Lorelamp in the network for about two seconds.
#[tauri::command]
async fn discover() -> Vec<Found> {
    tauri::async_runtime::spawn_blocking(|| net::discover(Duration::from_millis(2000))).await.unwrap_or_default()
}

/// The code way: `addr` is `IP:Port` (from the search or typed), `code` the six digits.
#[tauri::command]
async fn pair(app: AppHandle, addr: String, code: String) -> Result<String, Fail> {
    let (host, port) = link::parse_addr(&addr, net::LOOPBACK)?;
    let code = link::parse_code(&code)?;
    let link = net::pair(host, port, &code).await?;
    adopt(&app, link).await
}

/// An address arrived from outside: keep it and show the start page, which asks before it connects.
#[cfg(mobile)]
fn take_link(app: &AppHandle, url: &str) {
    if link::deep_link_target(url).is_none() {
        return;
    }
    if let Ok(mut pending) = app.state::<Pending>().0.lock() {
        *pending = Some(url.to_owned());
    }
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(home) = "http://tauri.localhost/".parse() {
            let _ = window.navigate(home);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init()).plugin(tauri_plugin_deep_link::init());
    builder
        .manage(Pending::default())
        .invoke_handler(tauri::generate_handler![startup, accept_link, decline_link, connect, open_saved, discover, pair])
        .setup(|_app| {
            #[cfg(mobile)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                // Started by a link: the start page picks it up when it asks for `startup`.
                if let Ok(Some(urls)) = _app.deep_link().get_current() {
                    if let Some(url) = urls.first().filter(|u| link::deep_link_target(u.as_str()).is_some()) {
                        *_app.state::<Pending>().0.lock().unwrap() = Some(url.to_string());
                    }
                }
                let handle = _app.handle().clone();
                _app.deep_link().on_open_url(move |event| {
                    if let Some(url) = event.urls().first() {
                        take_link(&handle, url.as_str());
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Lorelamp");
}
