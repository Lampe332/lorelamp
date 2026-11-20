//! rp-server: ONE HTTP + WebSocket API shared by the desktop window and phone browsers.
//!
//! This file holds the configuration and puts the server together: the `/api` routes
//! (`routes/`, behind the token check in `guard`), the web UI as fallback, the request log
//! (`reqlog`) and the Host check (`guard`) around everything. Storage is `store`, the chat hub
//! with the model is `chat`, the model server and its supervisor are `engine`, the activity log
//! is `logs`, database backups are `backup`; every handler answers errors with `error::ApiError`.

pub mod autostart;
mod backdrop_presets;
mod backup;
mod bench;
#[cfg(test)]
mod builder_tests;
mod chat;
pub mod engine;
mod error;
mod export;
mod fetch;
#[cfg(test)]
mod general_tests;
mod guard;
mod headers;
mod imaging;
mod import;
#[cfg(test)]
mod import_tests;
mod lan;
#[cfg(test)]
mod lan_tests;
pub mod link;
#[cfg(test)]
mod link_tests;
mod link_web;
#[cfg(test)]
mod link_web_tests;
mod netguard;
pub mod logs;
mod preview;
mod reqlog;
mod routes;
mod sillytavern;
#[cfg(test)]
mod st_tests;
mod store;
mod system;
mod thumbs;
#[cfg(feature = "embed-ui")]
mod web;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{middleware, Router};
use rand::distr::{Alphanumeric, SampleString};
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

use chat::Hub;
use store::Store;

pub use guard::{extract_token, host_allowed};
pub use logs::{Entry, Level, Logger};
pub use reqlog::device_label;

pub const DEFAULT_PORT: u16 = 7860;
pub const DEFAULT_LLAMA_URL: &str = "http://127.0.0.1:8080";

#[derive(Debug, Clone)]
pub struct Config {
    /// 0 = pick any free port.
    pub port: u16,
    /// Phones in the Wi-Fi may connect from the start (as if the switch "Handy verbinden" were on,
    /// without storing it). The switch itself is the setting `lan_enabled` (`PUT /api/lan`).
    pub lan: bool,
    /// Where the web UI comes from.
    pub web: WebUi,
    pub llama_url: String,
    /// Database and card files. Personal data: never inside the repo.
    pub data_dir: PathBuf,
    /// Set by whoever manages the model server (the desktop app) when something is wrong with
    /// it; shown next to the model status.
    pub engine_note: EngineNote,
    /// The activity log, when the caller started it already (the desktop app does, so the model
    /// supervisor can log before the server runs). None: the server starts one in `data_dir`.
    pub log: Option<Logger>,
    /// Start and supervise llama-server from the active model profile (the desktop app;
    /// `rp-server --manage-engine`). Then `llama_url` follows the profile's port.
    pub manage_engine: bool,
}

/// Source of the web UI's files (index.html + assets).
#[derive(Debug, Clone)]
pub enum WebUi {
    /// A built web UI on disk, read at run time (development: `--web-dir`).
    Dir(PathBuf),
    /// Compiled into the program from `apps/web/dist` (feature `embed-ui`).
    #[cfg(feature = "embed-ui")]
    Embedded,
}

impl WebUi {
    /// One file of the built UI, read where the UI comes from.
    fn file(&self, path: &str) -> Option<Vec<u8>> {
        match self {
            WebUi::Dir(dir) => std::fs::read(dir.join(path)).ok(),
            #[cfg(feature = "embed-ui")]
            WebUi::Embedded => web::file(path),
        }
    }

    fn describe(&self) -> String {
        match self {
            WebUi::Dir(dir) => dir.display().to_string(),
            #[cfg(feature = "embed-ui")]
            WebUi::Embedded => "embedded".into(),
        }
    }
}

/// Why no model answers, as `GET /engine` sends it in `note`: a code the UI puts in words in the
/// app's language (`shell.engineNote.*`), plus what it names — the technical detail as the system
/// says it (an exit status, an error, a path) and the port. `gave_up`: no further start attempt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Note {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub gave_up: bool,
}

impl Note {
    pub fn new(code: &'static str) -> Note {
        Note { code, detail: None, port: None, gave_up: false }
    }

    pub fn with(code: &'static str, detail: impl std::fmt::Display) -> Note {
        Note { detail: Some(detail.to_string()), ..Note::new(code) }
    }
}

pub type EngineNote = Arc<Mutex<Option<Note>>>;

/// A started server. Dropping it does not stop it; call `stop()` or exit the process.
pub struct Running {
    pub token: String,
    pub addr: SocketAddr,
    /// Phones could connect when the server had started (see `lan_url`).
    pub lan: bool,
    lan_ctl: Arc<lan::Lan>,
    /// The model supervisor (with `manage_engine`); call `shutdown` on it when the app exits.
    pub engine: Option<engine::EngineHandle>,
    task: tokio::task::JoinHandle<()>,
    /// The daily backup check.
    daily: tokio::task::JoinHandle<()>,
    links: Arc<link::Sources>,
    fetch: Arc<fetch::Fetch>,
}

impl Running {
    pub fn local_url(&self) -> String {
        format!("http://127.0.0.1:{}/?token={}", self.addr.port(), self.token)
    }
    /// The phones' address (`http://<LAN-IP>:<port>/?token=<phones' token>`) while they may connect.
    pub fn lan_url(&self) -> Option<String> {
        self.lan_ctl.url()
    }
    /// Where link import fetches cards (Chub, RisuRealm); tests point it at a local server.
    pub fn link_sources(&self) -> &link::Sources {
        &self.links
    }
    /// Where model downloads and the catalog check go (default `https://huggingface.co`, else
    /// `RP_HF_BASE`); tests point it at a local stand-in.
    pub fn set_hf_base(&self, base: &str) -> Result<(), String> {
        self.fetch.set_hf_base(base)
    }
    pub fn stop(&self) {
        self.lan_ctl.abort();
        self.task.abort();
        self.daily.abort();
    }
    /// Stops the running model downloads without changing their list (the next start goes on)
    /// and waits until their files are closed.
    pub async fn stop_downloads(&self) {
        self.fetch.shutdown().await;
    }
    /// Wait until the server task ends.
    pub async fn wait(self) {
        let _ = self.task.await;
    }
}

/// What every handler and middleware gets.
#[derive(Clone)]
struct AppState {
    /// The local listener's token (the desktop window); phones use `lan`'s own.
    token: Arc<str>,
    /// Phones in the Wi-Fi (`lan.rs`).
    lan: Arc<lan::Lan>,
    /// Where the web UI comes from (the web app manifest is read from there).
    web: WebUi,
    hub: Arc<Hub>,
    engine_note: EngineNote,
    data_dir: Arc<std::path::Path>,
    /// The model supervisor, when this server manages the model server.
    engine: Option<engine::EngineHandle>,
    /// API endpoints of the link import.
    links: Arc<link::Sources>,
    /// Fetched cards waiting for the user's yes (link import with preview).
    previews: Arc<preview::Previews>,
    /// The last search for SillyTavern installations (until `refresh=1`).
    st_found: Arc<tokio::sync::Mutex<Option<sillytavern::Found>>>,
    /// The last event of the engine download (one at a time).
    engine_job: Arc<Mutex<Option<serde_json::Value>>>,
    /// Model downloads and the catalog.
    fetch: Arc<fetch::Fetch>,
    /// The benchmark of a model profile (one at a time).
    bench: Arc<bench::Status>,
}

impl AppState {
    fn log(&self) -> &Logger {
        self.hub.log()
    }
}

pub async fn start(cfg: Config) -> std::io::Result<Running> {
    let log = cfg.log.clone().unwrap_or_else(|| Logger::start(&cfg.data_dir));
    // A migration changes the database for good: back it up first, and do not start without.
    let started = std::time::Instant::now();
    match backup::before_migration(&cfg.data_dir, store::schema_target()) {
        Ok(Some(made)) => backup::log_made(&log, &made, started.elapsed().as_millis()),
        Ok(None) => {}
        Err(e) => {
            log.write(Entry::new(Level::Error, "server", "backup", format!("backup before the migration failed: {e}")));
            log.flush(std::time::Duration::from_secs(1));
            return Err(std::io::Error::other(format!("backup before the migration failed: {e}")));
        }
    }
    let mut store = match Store::open(&cfg.data_dir) {
        Ok(store) => store,
        Err(e) => {
            log.write(Entry::new(Level::Error, "server", "db", format!("cannot open the data folder: {e}")));
            log.flush(std::time::Duration::from_secs(1));
            return Err(std::io::Error::other(format!("data folder {}: {e}", cfg.data_dir.display())));
        }
    };
    let (from, to) = store.migrated();
    if from != to {
        log.write(
            Entry::new(Level::Info, "server", "db", format!("database migrated from version {from} to {to}"))
                .data(json!({ "from": from, "to": to })),
        );
    }
    take_over_engine_json(&mut store, &cfg.data_dir, &log);
    // A managed model server runs on the port of the settings (never llama.cpp's own default).
    let llama_url = match (cfg.manage_engine, store.engine_port()) {
        (true, Ok(Some(port))) => format!("http://127.0.0.1:{port}"),
        _ => cfg.llama_url.clone(),
    };
    let token = Alphanumeric.sample_string(&mut rand::rng(), 32);
    // The local listener is loopback only; phones get their own one (`lan.rs`).
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), cfg.port)).await?;
    let addr = listener.local_addr()?;

    let hub = Arc::new(Hub::new(store, llama_url.clone(), log.clone()).with_idle_minute(idle_minute()));
    let engine = cfg.manage_engine.then(|| start_supervisor(&hub, &cfg, &log));
    // Thumbnails for pictures that have none yet (the library from before they existed).
    let (for_thumbs, thumb_log) = (hub.clone(), log.clone());
    tokio::task::spawn_blocking(move || {
        let made = thumbs::backfill(&for_thumbs);
        if made > 0 {
            thumb_log.write(Entry::new(Level::Info, "server", "thumbs", "thumbnails made").data(json!({ "count": made })));
        }
    });
    let fetch = fetch::Fetch::start(hub.clone(), &cfg.data_dir);
    let state = AppState {
        token: token.as_str().into(),
        lan: Arc::new(lan::Lan::new(hub.clone(), addr.port())),
        web: cfg.web.clone(),
        hub,
        engine_note: cfg.engine_note.clone(),
        data_dir: cfg.data_dir.as_path().into(),
        engine: engine.clone(),
        links: Arc::new(link::Sources::default()),
        previews: Arc::new(preview::Previews::default()),
        st_found: Arc::new(tokio::sync::Mutex::new(None)),
        engine_job: Arc::new(Mutex::new(None)),
        fetch: fetch.clone(),
        bench: Arc::new(bench::Status::default()),
    };
    let links = state.links.clone();
    let lan_ctl = state.lan.clone();

    let app = Router::new()
        .nest("/api", routes::api(&state))
        .route("/manifest.webmanifest", axum::routing::get(routes::manifest));
    let app = match &cfg.web {
        WebUi::Dir(dir) => app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")))),
        #[cfg(feature = "embed-ui")]
        WebUi::Embedded => app.fallback(web::serve),
    };
    let app = app
        .layer(middleware::from_fn_with_state(state.clone(), reqlog::log_requests))
        .layer(middleware::from_fn_with_state(state.clone(), guard::check_host))
        .layer(middleware::from_fn_with_state(headers::Policy::new(&cfg.web), headers::add))
        .with_state(state);
    // The same app for phones, every request marked as coming over the LAN listener.
    lan_ctl.set_app(app.clone().layer(axum::Extension(lan::ViaLan)));
    let lan_wanted = cfg.lan || lan_ctl.stored_enabled();

    log.write(Entry::new(Level::Info, "server", "start", format!("server started on port {}", addr.port())).data(json!({
        "version": env!("CARGO_PKG_VERSION"), "port": addr.port(), "lan": cfg.lan, "llama": llama_url,
        "web_dir": cfg.web.describe(), "data_dir": cfg.data_dir.display().to_string(), "manage_engine": cfg.manage_engine,
    })));
    if lan_wanted {
        if let Err(e) = lan_ctl.enable().await {
            log.write(Entry::new(Level::Warn, "server", "lan", format!("phones could not be let in: {e}")));
        }
    }
    let lan_on = lan_ctl.is_on();
    let daily = backup::spawn_daily(cfg.data_dir.clone(), log.clone());
    let task = tokio::spawn(async move {
        let app = app.into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, app).await {
            log.write(Entry::new(Level::Error, "server", "stop", format!("server stopped: {e}")));
            eprintln!("server error: {e}");
        }
    });
    Ok(Running { token, addr, lan: lan_on, lan_ctl, engine, task, daily, links, fetch })
}

/// Once: `engine.json` from before model profiles becomes an engine and the active profile (the
/// file stays as it is, as emergency fallback).
fn take_over_engine_json(store: &mut Store, data_dir: &std::path::Path, log: &Logger) {
    if store.engine_json_imported().unwrap_or(true) {
        return;
    }
    let cfg = match engine::load_engine_json(data_dir) {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return,
        Err(e) => {
            log.write(Entry::new(Level::Warn, "engine", "config", format!("engine.json not readable, not taken over: {e}")));
            return;
        }
    };
    match store.import_engine_json(&cfg) {
        Ok(Some((engine_id, profile_id))) => log.write(
            Entry::new(Level::Info, "engine", "config", "engine.json taken over as engine and model profile").data(json!({
                "engine_id": engine_id, "profile_id": profile_id, "port": cfg.port,
                "model_file": cfg.model.file_name().map(|f| f.to_string_lossy().to_string()),
            })),
        ),
        Ok(None) => {}
        Err(e) => log.write(Entry::new(Level::Error, "engine", "config", format!("engine.json could not be taken over: {e}"))),
    }
}

/// The supervisor runs what `desired_now` says and tells every device about changes.
fn start_supervisor(hub: &Arc<Hub>, cfg: &Config, log: &Logger) -> engine::EngineHandle {
    let (for_desired, for_events) = (hub.clone(), hub.clone());
    let data_dir = cfg.data_dir.clone();
    let desired: engine::DesiredFn = Arc::new(move || desired_now(&for_desired, &data_dir));
    let events: engine::EventFn = Arc::new(move |event: &'static str, profile_id: Option<i64>, port: u16| {
        // Requests go where the model server now listens (the setting `engine_port` may change).
        if matches!(event, "starting" | "foreign") {
            for_events.set_llama_port(port);
        }
        for_events.engine_event(event, profile_id, Some(port));
    });
    let handle = engine::EngineHandle::start(cfg.data_dir.clone(), cfg.engine_note.clone(), log.clone(), desired, events);
    let waker = handle.clone();
    hub.set_waker(move || waker.wake());
    handle
}

/// One minute of the setting "Modell nach … entladen": 60 s, or `RP_IDLE_MINUTE_SECS` seconds on a
/// test server (so a test sees the quiet time end without waiting minutes).
fn idle_minute() -> std::time::Duration {
    let secs = std::env::var("RP_IDLE_MINUTE_SECS").ok().and_then(|v| v.parse::<u64>().ok()).filter(|&s| s > 0);
    std::time::Duration::from_secs(secs.unwrap_or(60))
}

/// The active profile; without one `engine.json` as emergency fallback.
fn desired_now(hub: &Hub, data_dir: &std::path::Path) -> engine::Desired {
    // A benchmark runs its candidates instead (no quiet-time unloading meanwhile); when it ends,
    // the active profile comes back as it was.
    if let Some(run) = hub.bench_run() {
        let unloaded = run.spec.is_none();
        return engine::Desired { spec: Ok(run.spec), busy: false, unloaded };
    }
    let minutes = hub.store(|s| s.idle_unload_min()).unwrap_or(0);
    let unloaded = hub.idle_check(minutes);
    let busy = hub.generating();
    let spec = match hub.store(|s| s.launch_spec()) {
        Ok(Some(spec)) => Ok(Some(spec)),
        Ok(None) => engine::load_engine_json(data_dir).map(|cfg| cfg.map(|cfg| engine::LaunchSpec::from_engine_json(&cfg))),
        Err(e) => Err(e.to_string()),
    };
    engine::Desired { spec, busy, unloaded }
}

/// Where the app keeps its data (database, logs, `engine.json`); a relative `rp-app` in the
/// current directory if the variable it is based on is missing.
/// - Windows: `%APPDATA%\rp-app`
/// - macOS: `~/Library/Application Support/rp-app`
/// - other Unix (Linux, BSD): `$XDG_DATA_HOME/rp-app`, else `~/.local/share/rp-app`
///
/// Android and iOS give an app its folder only through the platform API (Tauri's path
/// resolver), so a mobile shell passes `Config::data_dir` itself.
pub fn default_data_dir() -> PathBuf {
    data_base_dir().unwrap_or_default().join("rp-app")
}

#[cfg(windows)]
fn data_base_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn data_base_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Library").join("Application Support"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn data_base_dir() -> Option<PathBuf> {
    // XDG Base Directory spec: only an absolute XDG_DATA_HOME counts.
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(xdg);
    }
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".local").join("share"))
}

#[cfg(not(any(windows, unix)))]
fn data_base_dir() -> Option<PathBuf> {
    None
}

/// Best-effort LAN IPv4 of this machine (UDP "connect" sends no packet).
pub fn lan_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// Through the real server, over HTTP.
#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::Value;

    use super::*;

    #[cfg(windows)]
    #[test]
    fn data_dir_is_appdata_on_windows() {
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
        assert_eq!(default_data_dir(), appdata.join("rp-app"));
    }

    /// Through the real server: persona routes, the library fields, and the old name setting
    /// renaming the active persona.
    #[tokio::test]
    async fn personas_over_http() {
        let dir = std::env::temp_dir().join(format!("rp-app-personas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let lib = || async { c.get(format!("{base}/library")).header("authorization", &auth).send().await.unwrap().json::<Value>().await.unwrap() };

        let v = lib().await;
        let first = v["active_persona_id"].as_i64().unwrap();
        assert_eq!((v["user_name"].as_str(), v["personas"].as_array().unwrap().len()), (Some("User"), 1));

        let res = c.put(format!("{base}/settings")).header("authorization", &auth).json(&json!({ "user_name": " Lukas " })).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let v = lib().await;
        assert_eq!((v["user_name"].as_str(), v["personas"][0]["name"].as_str()), (Some("Lukas"), Some("Lukas")));

        let res = c.post(format!("{base}/personas")).header("authorization", &auth).json(&json!({ "name": "Bea", "description": "Kapitänin." })).send().await.unwrap();
        let bea: Value = res.json().await.unwrap();
        let bea_id = bea["id"].as_i64().unwrap();
        assert_eq!((bea["description"].as_str(), bea["has_avatar"].as_bool()), (Some("Kapitänin."), Some(false)));
        let bad = c.post(format!("{base}/personas")).header("authorization", &auth).json(&json!({ "description": "x" })).send().await.unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        let res = c.put(format!("{base}/personas/{bea_id}/activate")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let v = lib().await;
        assert_eq!((v["user_name"].as_str(), v["active_persona_id"].as_i64()), (Some("Bea"), Some(bea_id)));

        // Extra fields: limits, partial update, empty = cleared.
        let long = c.post(format!("{base}/personas")).header("authorization", &auth).json(&json!({ "name": "X", "pronouns": "x".repeat(41) })).send().await.unwrap();
        assert_eq!(long.json::<Value>().await.unwrap()["error"], "bad_pronouns");
        let long = c.put(format!("{base}/personas/{bea_id}")).header("authorization", &auth).json(&json!({ "background": "x".repeat(5_001) })).send().await.unwrap();
        assert_eq!(long.json::<Value>().await.unwrap()["error"], "bad_background");
        let res = c
            .put(format!("{base}/personas/{bea_id}"))
            .header("authorization", &auth)
            .json(&json!({ "pronouns": " sie/ihr ", "appearance": "Groß.", "personality": "Ruhig." }))
            .send()
            .await
            .unwrap();
        let p: Value = res.json().await.unwrap();
        assert_eq!((p["pronouns"].as_str(), p["appearance"].as_str(), p["description"].as_str()), (Some("sie/ihr"), Some("Groß."), Some("Kapitänin.")));
        let p: Value = c.put(format!("{base}/personas/{bea_id}")).header("authorization", &auth).json(&json!({ "personality": "" })).send().await.unwrap().json().await.unwrap();
        assert_eq!((p["personality"].as_str(), p["background"].as_str()), (Some(""), Some("")));

        // Default persona of a character binds new chats; a chat can be rebound or freed.
        let ch: Value = c.post(format!("{base}/characters/import")).header("authorization", &auth).body(r#"{"name":"Mara"}"#).send().await.unwrap().json().await.unwrap();
        let ch_id = ch["id"].as_i64().unwrap();
        assert_eq!(ch["default_persona_id"], Value::Null);
        let put = |url: String, body: Value| {
            let req = c.put(url).header("authorization", &auth).json(&body);
            async move { req.send().await.unwrap().status() }
        };
        assert_eq!(put(format!("{base}/characters/{ch_id}/default-persona"), json!({ "persona_id": bea_id })).await, StatusCode::NO_CONTENT);
        assert_eq!(put(format!("{base}/characters/{ch_id}/default-persona"), json!({ "persona_id": 999 })).await, StatusCode::NOT_FOUND);
        assert_eq!(put(format!("{base}/characters/999/default-persona"), json!({ "persona_id": null })).await, StatusCode::NOT_FOUND);
        let chat: Value = c.post(format!("{base}/chats")).header("authorization", &auth).json(&json!({ "character_id": ch_id })).send().await.unwrap().json().await.unwrap();
        let chat_id = chat["chat_id"].as_i64().unwrap();
        let chats: Value = c.get(format!("{base}/characters/{ch_id}/chats")).header("authorization", &auth).send().await.unwrap().json().await.unwrap();
        assert_eq!((chats["chats"][0]["persona_id"].as_i64(), chats["chats"][0]["effective_persona_id"].as_i64()), (Some(bea_id), Some(bea_id)));
        assert_eq!(lib().await["characters"][0]["default_persona_id"].as_i64(), Some(bea_id));
        assert_eq!(put(format!("{base}/chats/{chat_id}/persona"), json!({ "persona_id": null })).await, StatusCode::NO_CONTENT);
        assert_eq!(put(format!("{base}/chats/{chat_id}/persona"), json!({ "persona_id": 999 })).await, StatusCode::NOT_FOUND);
        assert_eq!(put(format!("{base}/chats/999/persona"), json!({ "persona_id": null })).await, StatusCode::NOT_FOUND);
        assert_eq!(lib().await["recent"][0]["chat"]["persona_id"], Value::Null);
        assert_eq!(put(format!("{base}/chats/{chat_id}/persona"), json!({ "persona_id": bea_id })).await, StatusCode::NO_CONTENT);

        // Card vs. persona: priority on the persona and per character, and the user check.
        assert_eq!(lib().await["personas"].as_array().unwrap().iter().find(|p| p["id"] == bea_id).unwrap()["card_priority"], "persona");
        let bad = c.put(format!("{base}/personas/{bea_id}")).header("authorization", &auth).json(&json!({ "card_priority": "x" })).send().await.unwrap();
        assert_eq!((bad.status(), bad.json::<Value>().await.unwrap()["error"].clone()), (StatusCode::BAD_REQUEST, json!("bad_priority")));
        let bad = c.post(format!("{base}/personas")).header("authorization", &auth).json(&json!({ "name": "Y", "card_priority": "" })).send().await.unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        let p: Value = c.put(format!("{base}/personas/{bea_id}")).header("authorization", &auth).json(&json!({ "card_priority": "card" })).send().await.unwrap().json().await.unwrap();
        assert_eq!((p["card_priority"].as_str(), p["pronouns"].as_str()), (Some("card"), Some("sie/ihr")));
        let info = |v: Value| v["characters"].as_array().unwrap().iter().find(|x| x["id"] == ch_id).unwrap().clone();
        assert_eq!((info(lib().await)["user_priority"].clone(), info(lib().await)["user_check_status"].clone()), (Value::Null, json!("ok")));
        assert_eq!(put(format!("{base}/characters/{ch_id}/user-priority"), json!({ "priority": "card" })).await, StatusCode::NO_CONTENT);
        assert_eq!(info(lib().await)["user_priority"], "card");
        assert_eq!(put(format!("{base}/characters/{ch_id}/user-priority"), json!({ "priority": null })).await, StatusCode::NO_CONTENT);
        assert_eq!(info(lib().await)["user_priority"], Value::Null);
        assert_eq!(put(format!("{base}/characters/{ch_id}/user-priority"), json!({ "priority": "both" })).await, StatusCode::BAD_REQUEST);
        assert_eq!(put(format!("{base}/characters/999/user-priority"), json!({ "priority": null })).await, StatusCode::NOT_FOUND);
        let check = |q: &str| {
            let req = c.get(format!("{base}/characters/{ch_id}/user-check{q}")).header("authorization", &auth);
            async move { req.send().await.unwrap() }
        };
        let v: Value = check("").await.json().await.unwrap();
        assert_eq!((v["status"].as_str(), v["persona_id"].as_i64(), v["items"].as_array().map(Vec::len)), (Some("ok"), Some(bea_id), Some(0)));
        let mut edited = serde_json::to_value(rp_core::card::Card { name: "Mara".into(), ..Default::default() }).unwrap();
        edited["description"] = json!("{{user}} ist ihr Ehemann und hat einen Bart.");
        assert_eq!(c.put(format!("{base}/characters/{ch_id}")).header("authorization", &auth).json(&edited).send().await.unwrap().status(), StatusCode::OK);
        let v: Value = check("").await.json().await.unwrap();
        assert_eq!(v["status"], "conflict", "{v}");
        let kinds: Vec<&str> = v["items"].as_array().unwrap().iter().map(|i| i["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["gender", "appearance", "role"]);
        assert_eq!((v["items"][0]["verdict"].as_str(), v["items"][0]["persona_text"].as_str()), (Some("conflict"), Some("sie/ihr")));
        assert_eq!(v["items"][1]["verdict"], "overlap", "Bea has an appearance: {v}");
        assert_eq!(info(lib().await)["user_check_status"], "conflict");
        let v: Value = check(&format!("?persona_id={first}")).await.json().await.unwrap();
        assert_eq!((v["status"].as_str(), v["persona_id"].as_i64()), (Some("ok"), Some(first)));
        assert_eq!(check("?persona_id=999").await.status(), StatusCode::NOT_FOUND);
        assert_eq!(c.get(format!("{base}/characters/999/user-check")).header("authorization", &auth).send().await.unwrap().status(), StatusCode::NOT_FOUND);

        let png = b"\x89PNG\r\n\x1a\nrest";
        let res = c.put(format!("{base}/personas/{bea_id}/avatar")).header("authorization", &auth).body(&png[..]).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let got = c.get(format!("{base}/personas/{bea_id}/avatar")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(got.headers()["content-type"], "image/png");
        assert_eq!(&got.bytes().await.unwrap()[..], &png[..]);

        let del = c.delete(format!("{base}/personas/{bea_id}")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(del.json::<Value>().await.unwrap()["active_persona_id"].as_i64(), Some(first));
        let v = lib().await;
        assert_eq!((v["recent"][0]["chat"]["persona_id"].clone(), v["recent"][0]["chat"]["effective_persona_id"].as_i64()), (Value::Null, Some(first)));
        assert_eq!(v["characters"][0]["default_persona_id"], Value::Null);
        let last = c.delete(format!("{base}/personas/{first}")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(last.status(), StatusCode::CONFLICT);
        let gone = c.put(format!("{base}/personas/{bea_id}/activate")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(gone.status(), StatusCode::NOT_FOUND);
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Through the real server: `engine.json` becomes the first profile once, profiles are
    /// checked, edited, activated and deleted, and `/engine` tells which model runs from which
    /// profile (here without a model server: file and context come from the profile).
    #[tokio::test]
    async fn model_profiles_over_http() {
        let dir = std::env::temp_dir().join(format!("rp-app-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let model = dir.join("Mini-Q4_K_M.gguf");
        std::fs::write(&model, b"GGUF but cut").unwrap();
        let engine_json = json!({
            "llama_server": dir.join("missing").join("llama-server.exe"), "model": model,
            "port": 7869, "args": ["-c", "2048", "-ngl", "0"], "stop_on_exit": false,
        });
        std::fs::write(dir.join("data").join("engine.json"), engine_json.to_string()).unwrap();
        let cfg = || Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        };
        let running = start(cfg()).await.unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let get = |path: &str| {
            let req = c.get(format!("{base}{path}")).header("authorization", &auth);
            async move { req.send().await.unwrap().json::<Value>().await.unwrap() }
        };
        let send = |method: reqwest::Method, path: &str, body: Option<Value>| {
            let mut req = c.request(method, format!("{base}{path}")).header("authorization", &auth);
            if let Some(b) = body {
                req = req.json(&b);
            }
            async move {
                let res = req.send().await.unwrap();
                let status = res.status();
                (status, res.json::<Value>().await.unwrap_or(Value::Null))
            }
        };

        let v = get("/models/profiles").await;
        let first = &v["profiles"][0];
        let first_id = first["id"].as_i64().unwrap();
        assert_eq!(v["profiles"].as_array().unwrap().len(), 1);
        assert_eq!(v["active_id"].as_i64(), Some(first_id));
        assert_eq!((first["name"].as_str(), first["active"].as_bool(), first["reasoning"].as_str()), (Some("Mini-Q4_K_M"), Some(true), Some("off")));
        assert_eq!((first["load_args"].clone(), first["sampler"].clone()), (json!(["-c", "2048", "-ngl", "0"]), json!({ "temperature": 0.8, "n_predict": 800 })));
        assert_eq!(first["model_file"].as_str(), model.to_str());
        let engines = get("/models/engines").await;
        let e = &engines["engines"][0];
        assert_eq!((e["name"].as_str(), e["origin"].as_str(), e["exists"].as_bool(), e["backend"].as_str()), (Some("Eigener Build"), Some("own"), Some(false), Some("unknown")));
        let engine_id = e["id"].as_i64().unwrap();

        let v = get("/engine").await;
        assert_eq!((v["status"].as_str(), v["managed"].as_bool(), v["last_timings"].clone()), (Some("offline"), Some(false), Value::Null));
        assert_eq!((v["model"]["file"].as_str(), v["model"]["name"].as_str(), v["model"]["ctx"].as_u64()), (Some("Mini-Q4_K_M.gguf"), Some("Mini-Q4_K_M"), Some(2048)));
        assert_eq!((v["model"]["arch"].clone(), v["model"]["build"].clone()), (Value::Null, Value::Null), "a cut file is unknown, not an error");
        assert_eq!(v["profile"], json!({ "id": first_id, "name": "Mini-Q4_K_M", "measured": false }));

        // Checks with their codes.
        let f = model.to_str().unwrap();
        for (body, code) in [
            (json!({ "engine_id": engine_id, "model_file": f }), "bad_name"),
            (json!({ "name": "x", "engine_id": 999, "model_file": f }), "bad_engine"),
            (json!({ "name": "x", "engine_id": engine_id, "model_file": "C:\\nope.gguf" }), "bad_model_file"),
            (json!({ "name": "x", "engine_id": engine_id, "model_file": f, "load_args": ["--port", "1"] }), "bad_load_args"),
            (json!({ "name": "x", "engine_id": engine_id, "model_file": f, "sampler": { "temperature": 9 } }), "bad_sampler"),
            (json!({ "name": "x", "engine_id": engine_id, "model_file": f, "reasoning": "maybe" }), "bad_reasoning"),
        ] {
            let (status, v) = send(reqwest::Method::POST, "/models/profiles", Some(body)).await;
            assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some(code)));
        }

        let (status, second) = send(
            reqwest::Method::POST,
            "/models/profiles",
            Some(json!({ "name": "Groß", "engine_id": engine_id, "model_file": f, "load_args": ["-c", "4096", "-ngl", "0"], "sampler": { "temperature": 1.0, "min_p": 0.05 } })),
        )
        .await;
        assert_eq!((status, second["active"].as_bool(), second["measured"].as_bool()), (StatusCode::OK, Some(false), Some(false)));
        let second_id = second["id"].as_i64().unwrap();
        let (status, v) = send(reqwest::Method::PUT, &format!("/models/profiles/{second_id}"), Some(json!({ "name": "Groß 4k", "measured": true }))).await;
        assert_eq!((status, v["name"].as_str(), v["measured"].as_bool(), v["sampler"]["min_p"].as_f64()), (StatusCode::OK, Some("Groß 4k"), Some(true), Some(0.05)));
        let (status, v) = send(reqwest::Method::PUT, "/models/profiles/999", Some(json!({ "name": "x" }))).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("profile_missing")));
        let (status, v) = send(reqwest::Method::DELETE, &format!("/models/profiles/{first_id}"), None).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::CONFLICT, Some("profile_active")));

        let (status, v) = send(reqwest::Method::POST, &format!("/models/profiles/{second_id}/activate"), None).await;
        assert_eq!((status, v["engine"].as_str(), v["profile"]["active"].as_bool()), (StatusCode::OK, Some("unmanaged"), Some(true)));
        let (status, _) = send(reqwest::Method::POST, "/models/profiles/999/activate", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let v = get("/engine").await;
        assert_eq!((v["model"]["ctx"].as_u64(), v["profile"]["id"].as_i64()), (Some(4096), Some(second_id)));
        assert_eq!(get("/models/profiles").await["active_id"].as_i64(), Some(second_id));
        let (status, _) = send(reqwest::Method::DELETE, &format!("/models/profiles/{first_id}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // A second start does not take engine.json over again (the deleted profile stays gone).
        running.stop();
        let running = start(cfg()).await.unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let v: Value = c.get(format!("{base}/models/profiles")).header("authorization", format!("Bearer {}", running.token)).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["profiles"].as_array().unwrap().iter().map(|p| p["id"].as_i64().unwrap()).collect::<Vec<_>>(), [second_id]);
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Through the real server: "Meine Modelle" lists the app folder and added folders, deletes
    /// only in the app folder and never the running model, "Benutzen" makes a profile once and
    /// reuses it, and the estimate answers per context step.
    #[tokio::test]
    async fn local_models_over_http() {
        use crate::engine::local::tests::gguf_bytes;
        std::env::set_var("RP_LMSTUDIO_DIR", ""); // never the real LM Studio folder
        let dir = std::env::temp_dir().join(format!("rp-app-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["web", "data/models/vendor", "extra"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let mini = dir.join("data/models/vendor/Mini-Q4_K_M.gguf");
        std::fs::write(&mini, gguf_bytes("Mini", "llama", 8192, 4096)).unwrap();
        let other = dir.join("extra/Other-Q4_K_M.gguf");
        std::fs::write(&other, gguf_bytes("Other", "llama", 4096, 64)).unwrap();
        let engine_json = json!({ "llama_server": dir.join("missing.exe"), "model": mini, "port": 7868, "args": ["-c", "2048", "-ngl", "0"] });
        std::fs::write(dir.join("data/engine.json"), engine_json.to_string()).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let send = |method: reqwest::Method, path: &str, body: Option<Value>| {
            let mut req = c.request(method, format!("{base}{path}")).header("authorization", &auth);
            if let Some(b) = body {
                req = req.json(&b);
            }
            async move {
                let res = req.send().await.unwrap();
                let status = res.status();
                (status, res.json::<Value>().await.unwrap_or(Value::Null))
            }
        };
        let list = || send(reqwest::Method::GET, "/models/local", None);

        let (status, v) = list().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!((v["folders"]["app"]["default"].as_bool(), v["folders"]["lmstudio"].clone()), (Some(true), Value::Null));
        assert!(v["folders"]["app"]["path"].as_str().unwrap().ends_with("models"));
        let models = v["models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!((models[0]["name"].as_str(), models[0]["origin"].as_str(), models[0]["deletable"].as_bool()), (Some("Mini"), Some("app"), Some(true)));
        assert_eq!(models[0]["profiles"][0]["active"].as_bool(), Some(true), "used by the profile from engine.json");

        let (status, v) = send(reqwest::Method::PUT, "/models/folders", Some(json!({ "app_folder": null, "extra": ["relative\\path"] }))).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_folder")));
        let extra = dir.join("extra").display().to_string();
        let (status, v) = send(reqwest::Method::PUT, "/models/folders", Some(json!({ "app_folder": "", "extra": [extra, extra] }))).await;
        assert_eq!((status, v["extra"].as_array().map(Vec::len)), (StatusCode::OK, Some(1)), "a folder given twice counts once");
        let (_, v) = list().await;
        let names: Vec<_> = v["models"].as_array().unwrap().iter().map(|m| (m["name"].as_str().unwrap().to_string(), m["deletable"].as_bool().unwrap())).collect();
        assert_eq!(names, [("Mini".to_string(), true), ("Other".to_string(), false)]);

        // Delete: only in the app folder, never the running model.
        let path = |p: &std::path::Path| json!({ "path": p.display().to_string() });
        let (status, v) = send(reqwest::Method::DELETE, "/models/local", Some(path(&other))).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_app_folder")));
        let (status, v) = send(reqwest::Method::DELETE, "/models/local", Some(path(&mini))).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::CONFLICT, Some("model_in_use")));
        let (status, v) = send(reqwest::Method::DELETE, "/models/local", Some(json!({ "path": dir.join("data/models/none.gguf") }))).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("model_missing")));
        let (status, v) = send(reqwest::Method::POST, "/models/local/reveal", Some(json!({ "path": dir.join("web/x.gguf") }))).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("model_missing")));

        // "Benutzen": a new profile with the active one's options and the new model's context, then the same one again.
        let (status, v) = send(reqwest::Method::POST, "/models/local/use", Some(path(&other))).await;
        assert_eq!((status, v["created"].as_bool(), v["engine"].as_str()), (StatusCode::OK, Some(true), Some("unmanaged")));
        assert_eq!((v["profile"]["name"].as_str(), v["profile"]["active"].as_bool()), (Some("Other"), Some(true)));
        assert_eq!(v["profile"]["load_args"], json!(["-ngl", "0", "-c", "4096", "-ctk", "q8_0", "-ctv", "q8_0"]));
        let made = v["profile"]["id"].as_i64();
        let (_, v) = send(reqwest::Method::POST, "/models/local/use", Some(path(&other))).await;
        assert_eq!((v["created"].as_bool(), v["profile"]["id"].as_i64()), (Some(false), made));

        // The memory estimate: one step per context up to the trained one.
        let (status, v) = send(reqwest::Method::POST, "/models/estimate", Some(json!({ "path": mini, "cache_k": "q8_0", "cache_v": "q8_0", "ctx": 3072 }))).await;
        assert_eq!(status, StatusCode::OK);
        let steps = v["steps"].as_array().unwrap();
        assert_eq!(steps.iter().map(|s| s["ctx"].as_u64().unwrap()).collect::<Vec<_>>(), [2048, 3072, 4096, 8192], "the profile's own context is a step too");
        assert!(steps.iter().all(|s| s["gpu_bytes"].as_u64().unwrap() >= rp_core::vram::RESERVE_BYTES && s["fit"].is_string()));
        let (status, v) = send(reqwest::Method::POST, "/models/estimate", Some(json!({ "path": mini, "cache_k": "q9" }))).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_cache_type")));

        // Now Mini is not running any more and may go (from the app folder).
        let size = std::fs::metadata(&mini).unwrap().len();
        let (status, v) = send(reqwest::Method::DELETE, "/models/local", Some(path(&mini))).await;
        assert_eq!((status, v["freed_bytes"].as_u64()), (StatusCode::OK, Some(size)));
        assert!(!mini.exists() && other.exists());
        let (_, v) = list().await;
        assert_eq!(v["models"].as_array().unwrap().len(), 1);
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Through the real server: the daily backup at start, "back up now", restoring one (the
    /// state after it is gone, a `before-restore` backup holds it) and the refused names.
    #[tokio::test]
    async fn backups_over_http() {
        let dir = std::env::temp_dir().join(format!("rp-app-backups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        let log = Logger::start(&dir.join("data"));
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            manage_engine: false,
            log: Some(log.clone()),
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let list = || async { c.get(format!("{base}/backups")).header("authorization", &auth).send().await.unwrap().json::<Value>().await.unwrap()["backups"].clone() };
        let reasons = |v: &Value| v.as_array().unwrap().iter().map(|b| b["reason"].as_str().unwrap().to_string()).collect::<Vec<_>>();

        // The daily backup runs at start (in the background).
        let mut tries = 0;
        while !reasons(&list().await).contains(&"daily".to_string()) {
            tries += 1;
            assert!(tries < 50, "no daily backup at start");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(c.get(format!("{base}/backups")).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);

        let names = |v: Value| v["characters"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let lib = || async { c.get(format!("{base}/library")).header("authorization", &auth).send().await.unwrap().json::<Value>().await.unwrap() };
        c.post(format!("{base}/characters/import")).header("authorization", &auth).body(r#"{"name":"Mara"}"#).send().await.unwrap();
        let res = c.post(format!("{base}/backups")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let made: Value = res.json().await.unwrap();
        let name = made["name"].as_str().unwrap().to_string();
        assert_eq!(made["reason"], "manual");
        assert!(made["size"].as_u64().unwrap() > 0 && made["time"].as_i64().unwrap() > 0);
        assert_eq!(list().await[0]["name"], name.as_str(), "newest first");

        c.post(format!("{base}/characters/import")).header("authorization", &auth).body(r#"{"name":"Ilse"}"#).send().await.unwrap();
        assert_eq!(names(lib().await).len(), 2);
        let res = c.post(format!("{base}/backups/{name}/restore")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v: Value = res.json().await.unwrap();
        assert_eq!((v["restored"].as_str(), v["safety"]["reason"].as_str()), (Some(name.as_str()), Some("before-restore")));
        assert_eq!(names(lib().await), ["Mara"]);
        assert_eq!(reasons(&list().await).iter().filter(|r| *r == "before-restore").count(), 1);

        for (bad, status) in [
            ("..%2Frp-app.db", StatusCode::BAD_REQUEST),
            ("..%5Crp-app.db", StatusCode::BAD_REQUEST),
            ("rp-app.db", StatusCode::BAD_REQUEST),
            ("rp-app-2020-01-01-000000-manual.db", StatusCode::NOT_FOUND),
        ] {
            let res = c.post(format!("{base}/backups/{bad}/restore")).header("authorization", &auth).send().await.unwrap();
            assert_eq!(res.status(), status, "{bad}");
        }

        // The list carries kind, creation time, schema and the sizes for the Backups tab.
        let all: Value = c.get(format!("{base}/backups")).header("authorization", &auth).send().await.unwrap().json().await.unwrap();
        let first = &all["backups"][0];
        assert_eq!((first["kind"].as_str(), first["reason"].as_str()), (Some("restore"), Some("before-restore")));
        assert!(first["created_at"].as_i64().unwrap() > 0 && first["schema"].as_i64().unwrap() > 0);
        let sizes: u64 = all["backups"].as_array().unwrap().iter().map(|b| b["size"].as_u64().unwrap()).sum();
        assert_eq!(all["total_bytes"].as_u64(), Some(sizes));
        assert_eq!(all["limit_bytes"].as_u64(), Some(1024 * 1024 * 1024));
        assert!(all["db_bytes"].as_u64().unwrap() > 0 && all["disk_free"].as_u64().unwrap() > 0);

        // Deleting: the manual backup goes; nothing outside the backups folder can be reached.
        std::fs::write(dir.join("data/rp-app.db.backup-by-hand"), b"x").unwrap();
        for (bad, status) in [
            ("..%2Frp-app.db", StatusCode::BAD_REQUEST),
            ("..%5C..%5Crp-app.db", StatusCode::BAD_REQUEST),
            ("..", StatusCode::BAD_REQUEST),
            ("rp-app.db", StatusCode::BAD_REQUEST),
            ("logs.db", StatusCode::BAD_REQUEST),
            ("rp-app.db.backup-by-hand", StatusCode::BAD_REQUEST),
            ("rp-app-2020-01-01-000000-manual.db", StatusCode::NOT_FOUND),
        ] {
            let res = c.delete(format!("{base}/backups/{bad}")).header("authorization", &auth).send().await.unwrap();
            assert!(res.status() == status || (bad == ".." && res.status().is_client_error()), "{bad}: {}", res.status());
        }
        assert!(dir.join("data/rp-app.db").is_file() && dir.join("data/rp-app.db.backup-by-hand").is_file());
        assert_eq!(c.delete(format!("{base}/backups/{name}")).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let res = c.delete(format!("{base}/backups/{name}")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.json::<Value>().await.unwrap()["deleted"], name.as_str());
        assert!(!list().await.as_array().unwrap().iter().any(|b| b["name"] == name.as_str()));
        assert!(!dir.join("data/backups").join(&name).exists());
        assert_eq!(all["near_full"], false);

        // The limit: only the four choices; it is kept and the list gives it.
        let put_limit = |v: Value| {
            let (c, auth) = (c.clone(), auth.clone());
            let url = format!("{base}/backups/limit");
            async move { c.put(url).header("authorization", auth).json(&v).send().await.unwrap() }
        };
        let res = put_limit(serde_json::json!({ "limit_bytes": 123 })).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.json::<Value>().await.unwrap()["error"], "bad_limit");
        let res = put_limit(serde_json::json!({ "limit_bytes": 2u64 << 30 })).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.json::<Value>().await.unwrap(), serde_json::json!({ "limit_bytes": 2u64 << 30, "pruned": 0 }));
        let all: Value = c.get(format!("{base}/backups")).header("authorization", &auth).send().await.unwrap().json().await.unwrap();
        assert_eq!(all["limit_bytes"].as_u64(), Some(2 << 30));

        // Deleting several: a foreign name refuses the whole list; then exactly the named ones go.
        let delete_many = |v: Value| {
            let (c, auth) = (c.clone(), auth.clone());
            let url = format!("{base}/backups/delete");
            async move { c.post(url).header("authorization", auth).json(&v).send().await.unwrap() }
        };
        let left: Vec<String> = list().await.as_array().unwrap().iter().filter(|b| b["reason"] != "hand").map(|b| b["name"].as_str().unwrap().to_string()).collect();
        assert!(left.len() >= 2, "{left:?}");
        let count = || async { list().await.as_array().unwrap().iter().filter(|b| b["reason"] != "hand").count() };
        let res = delete_many(serde_json::json!({ "names": [left[0], "../rp-app.db"] })).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(count().await, left.len());
        assert!(dir.join("data/rp-app.db").is_file());
        for body in [serde_json::json!({}), serde_json::json!({ "names": [] }), serde_json::json!({ "older_than_days": 0 }), serde_json::json!({ "names": [left[0]], "older_than_days": 3 })] {
            let res = delete_many(body.clone()).await;
            assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(res.json::<Value>().await.unwrap()["error"], "bad_delete");
        }
        let res = delete_many(serde_json::json!({ "older_than_days": 30 })).await;
        assert_eq!(res.json::<Value>().await.unwrap()["deleted"], serde_json::json!([]), "all are from today");
        let res = delete_many(serde_json::json!({ "names": [left[0]] })).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.json::<Value>().await.unwrap()["deleted"], serde_json::json!([left[0]]));
        assert_eq!(count().await, left.len() - 1);

        log.flush(std::time::Duration::from_secs(5));
        let conn = logs::open_read(&dir.join("data")).unwrap();
        let msgs: Vec<String> = logs::query(&conn, &logs::Query::default()).unwrap().entries.into_iter().filter(|e| e.category == "backup").map(|e| e.message).collect();
        for m in ["backup created (daily)", "backup created (manual)", "backup created (before-restore)", "backup restored", "restore refused: bad_name", "backup deleted", "delete refused: bad_name", "backup limit set", "backups deleted"] {
            assert!(msgs.iter().any(|x| x == m), "{m} in {msgs:?}");
        }
        running.stop();
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Through the real server: a window start (page, script, style, icon) leaves no entries,
    /// a failed request does, and the first start logs the new database.
    #[tokio::test]
    async fn window_start_is_not_logged_but_failures_are() {
        let dir = std::env::temp_dir().join(format!("rp-app-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let web = dir.join("web");
        std::fs::create_dir_all(web.join("assets")).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
        std::fs::write(web.join("assets/app.js"), "1").unwrap();
        std::fs::write(web.join("assets/app.css"), "a{}").unwrap();
        let log = Logger::start(&dir.join("data"));
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(web),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: Some(log.clone()),
            manage_engine: false,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}", running.addr.port());
        let client = reqwest::Client::new();
        for path in ["/", "/assets/app.js", "/assets/app.css", "/favicon.ico"] {
            assert!(client.get(format!("{base}{path}")).send().await.unwrap().status().is_success(), "{path}");
        }
        let missing = client.get(format!("{base}/api/library")).send().await.unwrap(); // no token
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        log.flush(std::time::Duration::from_secs(5));

        let conn = logs::open_read(&dir.join("data")).unwrap();
        let http: Vec<String> = logs::query(&conn, &logs::Query { filter: "server".into(), ..Default::default() })
            .unwrap()
            .entries
            .into_iter()
            .filter(|e| e.category == "http")
            .map(|e| e.message)
            .collect();
        assert_eq!(http, ["GET /api/library -> 401"]);
        let all = logs::query(&conn, &logs::Query::default()).unwrap();
        assert!(all.entries.iter().any(|e| e.category == "db" && e.message.starts_with("database migrated from version 0")));
        running.stop();
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Through the real server: message search, favourites, own tags and bulk actions.
    #[tokio::test]
    async fn search_tags_and_bulk_over_http() {
        let dir = std::env::temp_dir().join(format!("rp-app-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let get = |path: String| {
            let req = c.get(format!("{base}{path}")).header("authorization", &auth);
            async move { req.send().await.unwrap().json::<Value>().await.unwrap() }
        };
        let send = |method: reqwest::Method, path: &str, body: Value| {
            let req = c.request(method, format!("{base}{path}")).header("authorization", &auth).json(&body);
            async move {
                let res = req.send().await.unwrap();
                let status = res.status();
                (status, res.json::<Value>().await.unwrap_or(Value::Null))
            }
        };

        let mut ids = vec![];
        for (name, greeting) in [("Mara", "Der Leuchtturm steht am Meer."), ("Ilse", "Die Bücherei öffnet um acht.")] {
            let card = json!({ "name": name, "first_mes": greeting }).to_string();
            let ch: Value = c.post(format!("{base}/characters/import")).header("authorization", &auth).body(card).send().await.unwrap().json().await.unwrap();
            ids.push(ch["id"].as_i64().unwrap());
        }
        let (_, opened) = send(reqwest::Method::POST, "/chats", json!({ "character_id": ids[0], "fresh": true })).await;
        let chat = opened["chat_id"].as_i64().unwrap();

        let hits = get("/search/messages?q=leucht".into()).await;
        let hit = &hits["hits"][0];
        assert_eq!((hit["chat_id"].as_i64(), hit["character"].as_str(), hit["character_id"].as_i64()), (Some(chat), Some("Mara"), Some(ids[0])));
        assert_eq!(hit["snippet"].as_str(), Some("Der \u{2}Leuchtturm\u{3} steht am Meer."));
        assert_eq!(get("/search/messages?q=b%C3%BCcherei".into()).await["hits"].as_array().unwrap().len(), 0, "no chat with Ilse yet");
        assert_eq!(get(format!("/search/messages?q=meer&chat_id={}", chat + 1)).await["hits"].as_array().unwrap().len(), 0);
        assert_eq!(get("/search/messages?q=l".into()).await["hits"].as_array().unwrap().len(), 0, "one letter finds nothing");

        let (status, _) = send(reqwest::Method::PUT, &format!("/characters/{}/favorite", ids[1]), json!({ "favorite": true })).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, v) = send(reqwest::Method::PUT, &format!("/characters/{}/tags", ids[0]), json!({ "tags": ["Meer", " meer ", "Ruhig"] })).await;
        assert_eq!((status, v["tags"].clone()), (StatusCode::OK, json!(["Meer", "Ruhig"])));
        let (status, v) = send(reqwest::Method::PUT, &format!("/characters/{}/tags", ids[0]), json!({ "tags": ["x".repeat(41)] })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_tag")));
        assert_eq!(send(reqwest::Method::PUT, "/characters/999/tags", json!({ "tags": [] })).await.0, StatusCode::NOT_FOUND);

        let (status, v) = send(reqwest::Method::POST, "/characters/bulk", json!({ "ids": ids, "action": "tag", "tag": "Neu" })).await;
        assert_eq!((status, v["changed"].as_i64()), (StatusCode::OK, Some(2)));
        assert_eq!(get("/tags".into()).await["tags"], json!([{ "name": "Meer", "count": 1 }, { "name": "Neu", "count": 2 }, { "name": "Ruhig", "count": 1 }]));
        for (body, code) in [
            (json!({ "ids": [], "action": "delete" }), "bad_ids"),
            (json!({ "ids": ids, "action": "explode" }), "bad_action"),
            (json!({ "ids": ids, "action": "untag" }), "bad_tag"),
        ] {
            let (status, v) = send(reqwest::Method::POST, "/characters/bulk", body).await;
            assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some(code)));
        }
        let lib = get("/library".into()).await;
        let mara = lib["characters"].as_array().unwrap().iter().find(|c| c["name"] == "Mara").unwrap().clone();
        assert_eq!((mara["own_tags"].clone(), mara["favorite"].as_bool()), (json!(["Meer", "Neu", "Ruhig"]), Some(false)));
        assert!(mara["last_chat_at"].as_i64().is_some() && mara["created_at"].as_i64().is_some());

        let (_, v) = send(reqwest::Method::POST, "/characters/bulk", json!({ "ids": ids, "action": "delete" })).await;
        assert_eq!(v["changed"].as_i64(), Some(2));
        assert_eq!(get("/library".into()).await["characters"].as_array().unwrap().len(), 0);
        assert_eq!(get("/search/messages?q=meer".into()).await["hits"].as_array().unwrap().len(), 0, "deleted characters are not found");
        let (_, v) = send(reqwest::Method::POST, "/characters/bulk", json!({ "ids": ids, "action": "restore" })).await;
        assert_eq!(v["changed"].as_i64(), Some(2));
        assert_eq!(get("/search/messages?q=meer".into()).await["hits"].as_array().unwrap().len(), 1);

        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The benchmark's refusals without a model server of its own.
    #[tokio::test]
    async fn benchmark_refusals() {
        let dir = std::env::temp_dir().join(format!("rp-app-bench-refuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let post = |path: &str, body: Value| {
            let req = c.post(format!("{base}{path}")).header("authorization", &auth).json(&body);
            async move {
                let r = req.send().await.unwrap();
                (r.status(), r.json::<Value>().await.unwrap_or(Value::Null))
            }
        };
        let (status, v) = post("/models/profiles/1/benchmark", json!({})).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::CONFLICT, Some("unmanaged")));
        let (status, v) = post("/models/profiles/1/benchmark/cancel", json!({})).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::CONFLICT, Some("not_running")));
        let (status, v) = post("/models/profiles/1/benchmark/apply", json!({ "purpose": "fast" })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_purpose")));
        let (status, v) = post("/models/profiles/1/benchmark/apply", json!({ "purpose": "speed" })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("profile_missing")));
        let v: Value = c.get(format!("{base}/models/profiles/1/benchmark")).header("authorization", &auth).send().await.unwrap().json().await.unwrap();
        assert_eq!(v, json!({ "running": null, "result": null, "results": [] }));
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Through the real server with `--manage-engine`: another program holds the model server's
    /// port → `/engine` says so (code + port) and nothing starts; the port is changed over
    /// `PUT /settings` → the model server starts there. The program here is this test binary,
    /// which exits at once on llama-server's arguments (a model server that does not come up).
    #[tokio::test]
    async fn a_busy_engine_port_is_reported_and_can_be_changed() {
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); // listens, never answers
        let busy = other.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(format!("rp-app-engine-port-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let model = dir.join("Mini-Q4_K_M.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        let engine_json = json!({ "llama_server": std::env::current_exe().unwrap(), "model": model, "port": busy, "args": ["-ngl", "0"] });
        std::fs::write(dir.join("data").join("engine.json"), engine_json.to_string()).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: true,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let get = || {
            let req = c.get(format!("{base}/engine")).header("authorization", &auth);
            async move { req.send().await.unwrap().json::<Value>().await.unwrap() }
        };
        let put = |body: Value| {
            let req = c.put(format!("{base}/settings")).header("authorization", &auth).json(&body);
            async move {
                let r = req.send().await.unwrap();
                (r.status(), r.json::<Value>().await.unwrap_or(Value::Null))
            }
        };
        let started = dir.join("data").join("llama-server.log"); // written by every start
        let mut e = Value::Null;
        for _ in 0..100 {
            e = get().await;
            if !e["problem"].is_null() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(e["problem"], json!({ "code": "port_busy", "port": busy }), "{e}");
        assert_eq!((e["port"].as_u64(), e["pid"].clone()), (Some(busy as u64), Value::Null));
        assert_eq!(e["note"], json!({ "code": "port_busy", "port": busy }), "{e}");
        tokio::time::sleep(std::time::Duration::from_millis(3500)).await; // one more check
        assert!(!started.exists(), "no start attempt on a busy port");
        assert!(engine::port_in_use(busy), "the other program keeps its port");

        let (status, v) = put(json!({ "engine_port": 80 })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_port")));
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (status, _) = put(json!({ "engine_port": free })).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        for _ in 0..100 {
            if started.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(started.exists(), "the model server starts on the new port");
        let e = get().await;
        assert_eq!(e["port"].as_u64(), Some(free as u64));
        assert_ne!(e["problem"]["code"], "port_busy", "{e}");
        assert!(engine::port_in_use(busy), "the other program was never stopped");

        running.engine.as_ref().unwrap().shutdown();
        running.stop();
        drop(other);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A whole benchmark with a small model on the CPU, started by the server itself
    /// (`--manage-engine`): shortened steps, then "Tempo" applied and run. Needs
    /// `RP_TEST_LLAMA_CPU` (a llama-server CPU build) and `RP_TEST_BENCH_MODEL` (a small GGUF);
    /// the model server gets port 7889. Never with a GPU build.
    #[tokio::test]
    #[ignore]
    async fn benchmark_with_a_cpu_model() {
        let exe = std::env::var("RP_TEST_LLAMA_CPU").expect("RP_TEST_LLAMA_CPU not set");
        let model = std::env::var("RP_TEST_BENCH_MODEL").expect("RP_TEST_BENCH_MODEL not set");
        assert!(!engine::port_in_use(7889), "something already runs on port 7889");
        std::env::set_var("RP_BENCH_CTX", "2048,4096");
        std::env::set_var("RP_BENCH_PROMPT_TOKENS", "256");
        std::env::set_var("RP_BENCH_GEN_TOKENS", "16");
        std::env::set_var("RP_BENCH_BUDGET_SECS", "150");
        let dir = std::env::temp_dir().join(format!("rp-app-bench-cpu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let cfg = json!({ "llama_server": exe, "model": model, "port": 7889, "args": ["-ngl", "0", "-c", "4096", "-np", "1"] });
        std::fs::write(dir.join("data").join("engine.json"), cfg.to_string()).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: true,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let get = |path: String| {
            let req = c.get(format!("{base}{path}")).header("authorization", &auth);
            async move { req.send().await.unwrap().json::<Value>().await.unwrap() }
        };
        let post = |path: String, body: Value| {
            let req = c.post(format!("{base}{path}")).header("authorization", &auth).json(&body);
            async move {
                let r = req.send().await.unwrap();
                (r.status(), r.json::<Value>().await.unwrap_or(Value::Null))
            }
        };
        let ready = |want_ctx: Option<u64>| {
            let get = &get;
            async move {
                for _ in 0..240 {
                    let e = get("/engine".into()).await;
                    if e["status"] == "ready" && want_ctx.is_none_or(|c| e["model"]["ctx"].as_u64() == Some(c)) {
                        return e;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                panic!("model server not ready");
            }
        };
        let e = ready(None).await;
        let id = e["profile"]["id"].as_i64().unwrap();
        assert_eq!(e["profile"]["measured"], false);

        let t = std::time::Instant::now();
        let (status, _) = post(format!("/models/profiles/{id}/benchmark"), json!({})).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let (status, v) = post(format!("/models/profiles/{id}/benchmark"), json!({})).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::CONFLICT, Some("benchmark")), "one at a time");
        let (status, v) = post(format!("/models/profiles/{id}/benchmark/apply"), json!({ "purpose": "speed" })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::CONFLICT, Some("benchmark")));
        let mut result = Value::Null;
        for _ in 0..400 {
            let v = get(format!("/models/profiles/{id}/benchmark")).await;
            if v["running"].is_null() {
                result = v["result"].clone();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let took = t.elapsed();
        println!("benchmark took {:.1} s: {}", took.as_secs_f64(), serde_json::to_string_pretty(&result).unwrap());
        let rows = result["rows"].as_array().expect("a stored result").clone();
        assert_eq!(rows.len(), 4, "2 steps × 2 KV types, no MoE search on the CPU");
        assert!(rows.iter().all(|r| r["outcome"] == "ok" && r["tg"].as_f64().unwrap() > 0.0 && r["pp"].as_f64().unwrap() > 0.0));
        // Each candidate: at least three measured runs after the warmup, median and spread kept.
        for r in &rows {
            let tg: Vec<f64> = serde_json::from_value(r["tg_runs"].clone()).unwrap();
            assert!(tg.len() >= rp_core::bench::MIN_RUNS && tg.len() <= rp_core::bench::MAX_RUNS, "{r}");
            let median = rp_core::bench::median(&tg).unwrap();
            assert!((r["tg"].as_f64().unwrap() - median).abs() <= 0.06, "the median of the runs (rounded to 0.1): {r}");
            assert!(r["spread"].as_f64().is_some() && r["uncertain"].is_boolean(), "{r}");
        }
        assert!(result["conditions"]["ram_available_mib"].as_u64().is_some(), "the memory at the start is stored");
        assert_eq!(get(format!("/models/profiles/{id}/benchmark")).await["results"][0], result, "the newest of the kept results");
        assert_eq!(result["picks"].as_array().unwrap().len(), 3);
        assert!(took.as_secs() < 150);

        // The active profile runs again as before (its own 4k context).
        let e = ready(Some(4096)).await;
        assert_eq!(e["profile"]["measured"].as_bool(), Some(false));

        let (status, v) = post(format!("/models/profiles/{id}/benchmark/apply"), json!({ "purpose": "speed" })).await;
        assert_eq!((status, v["engine"].as_str(), v["profile"]["measured"].as_bool()), (StatusCode::OK, Some("restarting"), Some(true)));
        let args: Vec<String> = serde_json::from_value(v["profile"]["load_args"].clone()).unwrap();
        let picked = &rows[result["picks"][0]["row"].as_u64().unwrap() as usize];
        let want = picked["ctx"].as_u64().unwrap();
        assert_eq!(rp_core::llama_args::ctx_from_args(&args), Some(want));
        assert!(args.windows(2).any(|w| w[0] == "-ngl" && w[1] == "0"), "the other options stay: {args:?}");
        let e = ready(Some(want)).await;
        assert_eq!(e["profile"]["measured"].as_bool(), Some(true));

        running.engine.as_ref().unwrap().shutdown();
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
