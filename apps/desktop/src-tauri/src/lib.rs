//! Tauri shell: one-click start of the whole app.
//! 1. Runs rp-server in-process on a free loopback port, with the model supervisor on
//!    (`rp_server::engine`): it starts llama-server from the active model profile, restarts it
//!    if it dies, switches it when another profile is activated and reports problems to the UI.
//! 2. Opens the window on it (the auth token travels in the URL; the page is the same one a phone uses).
//!
//! Closing the window (X) ends everything: a llama-server that this app started is stopped, also
//! while it still loads; should the app end hard, Windows ends it through a job object
//! (`rp_server::engine::job`). Model servers started elsewhere are never stopped.

mod links;
mod startup;
mod window;
mod window_state;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rp_server::{Entry, Level, Logger};
use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// How long the normal exit may take after the window is closed before the app ends itself.
const EXIT_GRACE: Duration = Duration::from_secs(4);

pub fn run() {
    let data_dir = rp_server::default_data_dir();
    // The activity log first, so everything from here on is in it.
    let log = Logger::start(&data_dir);
    log.info("server", "app", format!("app starting (version {})", env!("CARGO_PKG_VERSION")));

    // Own tokio runtime for the server; it lives as long as the app.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    // The web UI is built into the program; RP_WEB_DIR serves a folder instead (development).
    let web = std::env::var("RP_WEB_DIR")
        .map(|d| rp_server::WebUi::Dir(PathBuf::from(d)))
        .unwrap_or(rp_server::WebUi::Embedded);
    let lan = std::env::var("RP_LAN").is_ok_and(|v| v == "1");
    startup::offer();

    let running = rt
        .block_on(rp_server::start(rp_server::Config {
            port: 0,
            lan,
            web,
            // Used only while no port is set (no profile and no engine.json yet).
            llama_url: std::env::var("RP_LLAMA_URL").unwrap_or_else(|_| rp_server::DEFAULT_LLAMA_URL.into()),
            data_dir,
            engine_note: Arc::new(Mutex::new(None)),
            log: Some(log.clone()),
            manage_engine: true,
        }))
        .expect("start rp-server");
    println!("local:  {}", running.local_url());
    if let Some(u) = running.lan_url() {
        println!("phone:  {u}");
    }
    let url: tauri::Url = running.local_url().parse().expect("valid url");
    let (nav_own, popup_own) = (url.clone(), url.clone());
    let engine = running.engine.clone();
    let shutdown_engine = move || {
        if let Some(engine) = &engine {
            engine.shutdown();
        }
    };

    let download_log = log.clone();
    let setup_keeper = window::Keeper::new(&rp_server::default_data_dir());
    let event_keeper = setup_keeper.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            // Built hidden: `Keeper::restore` places it as remembered, then shows it in front.
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Lorelamp")
                .inner_size(1200.0, 820.0)
                .center()
                .visible(false)
                // Tauri's own drop handler swallows files dragged in from the OS, so the page's HTML5
                // `drop` (import page, library: files, folders, ZIPs) never fired. The page handles drops.
                .disable_drag_drop_handler()
                // Only the app's own server is shown here; other web pages open in the system browser
                // (a plain link and `target=_blank` alike), everything else is refused (`links.rs`).
                .on_navigation(move |to| links::guard(to, &nav_own))
                .on_new_window(move |to, _| {
                    if links::decide(&to, &popup_own) == links::Nav::Stay {
                        NewWindowResponse::Allow
                    } else {
                        links::guard(&to, &popup_own);
                        NewWindowResponse::Deny
                    }
                })
                // Plain downloads (the page's fallback where it cannot open a save dialog) go silently
                // into WebView2's download folder; the page asks for a place via `showSaveFilePicker`.
                .on_download(move |_, event| {
                    if let DownloadEvent::Finished { url, path, success } = event {
                        let path = path.map(|p| p.display().to_string());
                        download_log.write(
                            Entry::new(if success { Level::Info } else { Level::Error }, "server", "download",
                                if success { "download saved" } else { "download failed" })
                            .data(serde_json::json!({ "path": url.path(), "saved_to": path })),
                        );
                    }
                    true
                })
                .build()?;
            setup_keeper.restore(&win);
            startup::apply(&win);
            setup_keeper.start_saver(app.handle().clone());
            Ok(())
        })
        .on_window_event(move |win, event| event_keeper.on_event(win, event))
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    let shutdown_engine = Arc::new(shutdown_engine);
    let closing = AtomicBool::new(false);
    app.run(move |_app, event| match event {
        // Closing the window ends the app. Twice the window was gone while rp-desktop and
        // llama-server kept running without it (no `Exit` event came), so a closed window ends the
        // app by itself if the normal exit below has not done so in time.
        RunEvent::WindowEvent { event: WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed, .. } => {
            if !closing.swap(true, Ordering::SeqCst) {
                let (shutdown_engine, log) = (shutdown_engine.clone(), log.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(EXIT_GRACE);
                    log.write(Entry::new(Level::Warn, "server", "app", "window closed but the app did not exit; ending it now"));
                    shutdown_engine();
                    log.flush(Duration::from_secs(2));
                    std::process::exit(0);
                });
            }
        }
        RunEvent::Exit => {
            log.info("server", "app", "app closing");
            shutdown_engine();
            log.flush(Duration::from_secs(2));
        }
        _ => {}
    });
    drop(rt);
}
