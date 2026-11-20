//! Link import through the real server against a real local HTTP server that answers like the
//! Chub and RisuRealm APIs (saved SFW answers built from the fixtures), plus `#[ignore]` tests
//! against the real sites with public SFW cards.

use std::sync::{Arc, Mutex};

use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use reqwest::Url;
use serde_json::{json, Value};

use crate::link::Endpoints;
use crate::{start, Config, Running, WebUi};

const CHUB_ANSWER: &str = include_str!("../../../tests/fixtures/link/chub_character.json");
const ILSE_PNG: &[u8] = include_bytes!("../../../tests/fixtures/ilse_brandt.png");
const RISU_ID: &str = "0f3c2a1e-5b6d-4e7f-8a9b-0c1d2e3f4a5b";
const RISU_BLOCKED: &str = "0f3c2a1e-0000-4000-8000-00000000dead";

// ---------------------------------------------------------------------------------------------
// The local card site

#[derive(Clone, Default)]
struct Site {
    base: Arc<Mutex<String>>,
    /// Every request: "path?query user-agent".
    seen: Arc<Mutex<Vec<String>>>,
}

impl Site {
    fn note(&self, path: String, query: Option<String>, headers: &HeaderMap) {
        let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        self.seen.lock().unwrap().push(format!("{path}?{} {ua}", query.unwrap_or_default()));
    }
}

async fn chub_character(State(site): State<Site>, Path((creator, slug)): Path<(String, String)>, RawQuery(q): RawQuery, headers: HeaderMap) -> Response {
    site.note(format!("/api/characters/{creator}/{slug}"), q, &headers);
    let base = site.base.lock().unwrap().clone();
    match (creator.as_str(), slug.as_str()) {
        ("rp-app", "mara-voss-0001") => ([(header::CONTENT_TYPE, "application/json")], CHUB_ANSWER.replace("{base}", &base)).into_response(),
        // A picture on a host that is not Chub's picture host.
        ("rp-app", "foreign-picture") => {
            let answer = CHUB_ANSWER.replace("{base}", "https://pictures.example.invalid");
            ([(header::CONTENT_TYPE, "application/json")], answer).into_response()
        }
        // Redirects to another origin (localhost instead of 127.0.0.1).
        ("rp-app", "moved") => {
            let to = base.replace("127.0.0.1", "localhost") + "/api/characters/rp-app/mara-voss-0001?full=true";
            (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
        }
        // Over the size limit of a card JSON.
        ("rp-app", "huge") => ([(header::CONTENT_TYPE, "application/json")], vec![b' '; crate::link::MAX_CARD_JSON + 1]).into_response(),
        _ => (StatusCode::NOT_FOUND, axum::Json(json!({ "errors": "Not Found" }))).into_response(),
    }
}

async fn chub_picture(State(site): State<Site>, Path((creator, slug, file)): Path<(String, String, String)>, headers: HeaderMap) -> Response {
    site.note(format!("/avatars/{creator}/{slug}/{file}"), None, &headers);
    ([(header::CONTENT_TYPE, "image/png")], ILSE_PNG).into_response()
}

async fn risu_download(State(site): State<Site>, Path((format, id)): Path<(String, String)>, RawQuery(q): RawQuery, headers: HeaderMap) -> Response {
    site.note(format!("/api/v1/download/{format}/{id}"), q, &headers);
    match (format.as_str(), id.as_str()) {
        ("png-v3", RISU_ID) => ([(header::CONTENT_TYPE, "image/png")], ILSE_PNG).into_response(),
        ("png-v3", RISU_BLOCKED) => (StatusCode::FORBIDDEN, axum::Json(json!({ "error": "Blocked", "message": "Blocked from download" }))).into_response(),
        _ => (StatusCode::NOT_FOUND, axum::Json(json!({ "error": "Not Found", "message": "Data not found" }))).into_response(),
    }
}

/// Starts the local site; answers its base URL (`http://127.0.0.1:<port>`).
async fn card_site() -> (Site, String) {
    let site = Site::default();
    let app = Router::new()
        .route("/api/characters/{creator}/{slug}", get(chub_character))
        .route("/avatars/{creator}/{slug}/{file}", get(chub_picture))
        .route("/api/v1/download/{format}/{id}", get(risu_download))
        .with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    *site.base.lock().unwrap() = base.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (site, base)
}

// ---------------------------------------------------------------------------------------------
// The app's server

pub(crate) struct Server {
    pub(crate) running: Running,
    pub(crate) base: String,
    pub(crate) auth: String,
    pub(crate) c: reqwest::Client,
    pub(crate) dir: std::path::PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.running.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) async fn server(name: &str) -> Server {
    let dir = std::env::temp_dir().join(format!("rp-app-link-{name}-{}", std::process::id()));
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
    let auth = format!("Bearer {}", running.token);
    Server { running, base, auth, c: reqwest::Client::new(), dir }
}

impl Server {
    /// Server whose link import goes to the local card site.
    pub(crate) async fn with_site(name: &str, site_base: &str) -> Server {
        let s = server(name).await;
        let url = Url::parse(&format!("{site_base}/")).unwrap();
        s.running.link_sources().set(Endpoints { chub_api: url.clone(), chub_images: url.clone(), risu: url.clone(), test_origin: Some(url) });
        s
    }

    async fn link(&self, url: &str) -> (StatusCode, Value) {
        let res = self.c.post(format!("{}/import/link", self.base)).header("authorization", &self.auth).json(&json!({ "url": url })).send().await.unwrap();
        let status = StatusCode::from_u16(res.status().as_u16()).unwrap();
        (status, res.json().await.unwrap())
    }

    pub(crate) async fn get(&self, path: &str) -> reqwest::Response {
        self.c.get(format!("{}{path}", self.base)).header("authorization", &self.auth).send().await.unwrap()
    }

    pub(crate) fn source_url(&self, id: i64) -> Option<String> {
        let db = rusqlite::Connection::open(self.dir.join("data").join("rp-app.db")).unwrap();
        db.query_row("SELECT source_url FROM characters WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }

    /// Picture and thumbnail of a character are there and are images.
    pub(crate) async fn has_picture(&self, id: i64) -> bool {
        let avatar = self.get(&format!("/characters/{id}/avatar")).await;
        let thumb = self.get(&format!("/characters/{id}/thumb")).await;
        let image = |r: &reqwest::Response| r.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("image/"));
        avatar.status() == 200 && image(&avatar) && thumb.status() == 200 && image(&thumb)
    }
}

fn character_id(v: &Value) -> i64 {
    v["character_id"].as_i64().unwrap_or_else(|| panic!("no character_id in {v}"))
}

// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn chub_link_imports_card_with_picture() {
    let (site, base) = card_site().await;
    let s = Server::with_site("chub", &base).await;
    let (status, v) = s.link("https://chub.ai/characters/rp-app/mara-voss-0001").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["status"].as_str(), v["source"].as_str()), (Some("imported"), Some("chub")), "{v}");
    let id = character_id(&v);
    assert_eq!(s.source_url(id).as_deref(), Some("https://chub.ai/characters/rp-app/mara-voss-0001"));
    assert!(s.has_picture(id).await);

    // The card is mapped the way Chub's fields mean it (personality → description, …).
    let c: Value = s.get(&format!("/characters/{id}")).await.json().await.unwrap();
    let text = c.to_string();
    assert!(text.contains("Mara Voss") && text.contains("Leuchtturmwärterin auf der Hallig Süderoog"), "{c}");

    // Exactly two requests (card, picture), with the honest user agent and the documented query.
    let seen = site.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen[0].starts_with("/api/characters/rp-app/mara-voss-0001?full=true Lorelamp/"), "{seen:?}");
    assert!(seen[1].starts_with("/avatars/rp-app/mara-voss-0001/chara_card_v2.png? Lorelamp/"), "{seen:?}");
    assert!(seen.iter().all(|l| l.ends_with("(+https://github.com/Lampe332/lorelamp)")));

    // The same link again is a duplicate (same import path as files).
    let (_, again) = s.link("https://www.characterhub.org/characters/rp-app/mara-voss-0001").await;
    assert_eq!((again["status"].as_str(), again["character_id"].as_i64()), (Some("duplicate"), Some(id)), "{again}");
}

#[tokio::test]
async fn risu_link_imports_card_with_picture() {
    let (site, base) = card_site().await;
    let s = Server::with_site("risu", &base).await;
    let (status, v) = s.link(&format!("https://realm.risuai.net/character/{RISU_ID}")).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["status"].as_str(), v["source"].as_str()), (Some("imported"), Some("risurealm")), "{v}");
    let id = character_id(&v);
    assert_eq!(s.source_url(id), Some(format!("https://realm.risuai.net/character/{RISU_ID}")));
    assert!(s.has_picture(id).await);
    let seen = site.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(seen[0].starts_with(&format!("/api/v1/download/png-v3/{RISU_ID}?non_commercial=true Lorelamp/")), "{seen:?}");
}

#[tokio::test]
async fn fetch_errors_are_reported() {
    let (site, base) = card_site().await;
    let s = Server::with_site("errors", &base).await;
    for (link, status, code) in [
        ("https://chub.ai/characters/rp-app/gone".to_string(), StatusCode::NOT_FOUND, "source_not_found"),
        ("https://chub.ai/characters/rp-app/moved".to_string(), StatusCode::BAD_GATEWAY, "source_redirect"),
        ("https://chub.ai/characters/rp-app/huge".to_string(), StatusCode::BAD_GATEWAY, "source_too_large"),
        ("https://chub.ai/characters/rp-app/foreign-picture".to_string(), StatusCode::BAD_GATEWAY, "source_redirect"),
        (format!("https://realm.risuai.net/character/{RISU_BLOCKED}"), StatusCode::FORBIDDEN, "source_blocked"),
        ("https://realm.risuai.net/character/0f3c2a1e-0000-4000-8000-000000000000".to_string(), StatusCode::NOT_FOUND, "source_not_found"),
    ] {
        let (got, v) = s.link(&link).await;
        assert_eq!((got, v["error"].as_str()), (status, Some(code)), "{link}: {v}");
    }
    // The redirect was not followed: no request reached the other origin's path twice.
    let seen = site.seen.lock().unwrap().clone();
    assert!(!seen.iter().any(|l| l.contains("mara-voss-0001")), "{seen:?}");
    let lib: Value = s.get("/library").await.json().await.unwrap();
    assert!(!lib.to_string().contains("Mara Voss"), "nothing imported");
}

#[tokio::test]
async fn file_only_and_unknown_links() {
    let (site, base) = card_site().await;
    let s = Server::with_site("fileonly", &base).await;
    for (link, source, reason) in [
        ("https://janitorai.com/characters/0b1c2d3e-0000-4000-8000-000000000000_character-a-card", "janitorai", "definition_private"),
        ("https://jannyai.com/characters/0b1c2d3e-0000-4000-8000-000000000000_character-a-card", "jannyai", "bot_protection"),
        ("https://character-tavern.com/character/someone/a_card", "character_tavern", "terms_forbid_automation"),
        ("https://hammerai.com/characters/a-card", "hammerai", "terms_forbid_automation"),
        ("https://pygmalion.chat/character/0b1c2d3e-0000-4000-8000-000000000000", "pygmalion", "terms_forbid_automation"),
        ("https://aicharactercards.com/cards/4072", "aicharactercards", "no_public_api"),
        ("https://perchance.org/ai-character-chat?data=A_Card~0123456789abcdef0123456789abcdef.gz", "perchance", "terms_forbid_automation"),
        ("https://app.wyvern.chat/characters/_abc123", "wyvern", "terms_forbid_automation"),
        ("https://backyard.ai/hub/character/abc123", "backyard", "terms_forbid_automation"),
        ("https://charavault.net/cards/a/b", "charavault", "robots_disallow"),
    ] {
        let (status, v) = s.link(link).await;
        assert_eq!(status, StatusCode::OK, "{link}");
        assert_eq!(v, json!({ "status": "file_only", "source": source, "page_url": link, "reason_code": reason }));
    }
    for (link, code) in [("just-a-word", "bad_url"), ("not a link at all", "bad_url"), ("https://chub.ai/lorebooks/a/b", "bad_link")] {
        let (status, v) = s.link(link).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some(code)), "{link}");
    }
    assert!(site.seen.lock().unwrap().is_empty(), "nothing fetched");

    // The log has the source and status, never the link.
    let mut text = String::new();
    for _ in 0..50 {
        text = s.get("/logs?limit=500").await.text().await.unwrap();
        if text.contains("pygmalion") && text.contains("bad_link") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(text.contains("file_only") && text.contains("character_tavern"), "source and status are logged: {text}");
    assert!(!text.contains("a_card") && !text.contains("A_Card") && !text.contains("lorebooks"), "no links in the log");
}

// ---------------------------------------------------------------------------------------------
// Preview first, import on confirm

impl Server {
    pub(crate) async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let res = self.c.post(format!("{}{path}", self.base)).header("authorization", &self.auth).json(&body).send().await.unwrap();
        let status = StatusCode::from_u16(res.status().as_u16()).unwrap();
        (status, res.json().await.unwrap())
    }

    pub(crate) async fn characters(&self) -> usize {
        let lib: Value = self.get("/library").await.json().await.unwrap();
        lib["characters"].as_array().unwrap().len()
    }
}

#[tokio::test]
async fn preview_stores_nothing_until_confirmed() {
    let (site, base) = card_site().await;
    let s = Server::with_site("preview", &base).await;
    let link = "https://chub.ai/characters/rp-app/mara-voss-0001";
    let (status, v) = s.post("/import/link/preview", json!({ "url": link })).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["status"].as_str(), v["source"].as_str(), v["page_url"].as_str()), (Some("preview"), Some("chub"), Some(link)), "{v}");
    assert_eq!(v["card"]["name"], "Mara Voss");
    assert_eq!(v["card"]["creator"], "rp-app");
    assert!(v["card"]["tags"].is_array() && v["card"]["description"].as_str().unwrap().chars().count() <= crate::preview::TEXT_MAX + 1, "{v}");
    for key in ["first_mes", "creator_notes"] {
        assert!(v["card"][key].is_string(), "{key}: {v}");
    }
    assert_eq!((v["has_image"].as_bool(), v["duplicate_of"].clone()), (Some(true), Value::Null));
    let id = v["preview_id"].as_str().unwrap().to_string();
    assert_eq!(s.characters().await, 0, "the preview stored nothing");
    assert!(!s.dir.join("data").join("cards").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false), "no card files");

    let image = s.get(&format!("/import/link/preview/{id}/image")).await;
    assert_eq!((image.status().as_u16(), image.headers()["content-type"].to_str().unwrap()), (200, "image/webp"));
    assert_eq!(s.get("/import/link/preview/nope/image").await.status(), 404);

    // Confirm imports exactly the fetched card, without a second fetch.
    let fetched = site.seen.lock().unwrap().len();
    let (status, v) = s.post("/import/link/confirm", json!({ "preview_id": id })).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["status"].as_str(), v["source"].as_str()), (Some("imported"), Some("chub")), "{v}");
    let char_id = character_id(&v);
    assert_eq!(s.source_url(char_id).as_deref(), Some(link));
    assert!(s.has_picture(char_id).await);
    assert_eq!(site.seen.lock().unwrap().len(), fetched);

    // A preview can be confirmed once; unknown ids are expired too.
    for preview_id in [id.as_str(), "unknown"] {
        let (status, v) = s.post("/import/link/confirm", json!({ "preview_id": preview_id })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("preview_expired")));
    }

    // The same card again: the preview names the duplicate, confirming answers duplicate.
    let (_, v) = s.post("/import/link/preview", json!({ "url": link })).await;
    assert_eq!(v["duplicate_of"].as_i64(), Some(char_id), "{v}");
    let (_, v) = s.post("/import/link/confirm", json!({ "preview_id": v["preview_id"] })).await;
    assert_eq!((v["status"].as_str(), v["character_id"].as_i64()), (Some("duplicate"), Some(char_id)));
    assert_eq!(s.characters().await, 1);
}

#[tokio::test]
async fn preview_errors_like_link_import() {
    let (site, base) = card_site().await;
    let s = Server::with_site("preview-errors", &base).await;
    let (status, v) = s.post("/import/link/preview", json!({ "url": "https://chub.ai/characters/rp-app/gone" })).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("source_not_found")));
    let (status, v) = s.post("/import/link/preview", json!({ "url": format!("https://realm.risuai.net/character/{RISU_BLOCKED}") })).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("source_blocked")));
    let (status, v) = s.post("/import/link/preview", json!({ "url": "just-a-word" })).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_url")));
    let link = "https://janitorai.com/characters/0b1c2d3e-0000-4000-8000-000000000000_character-a-card";
    let (status, v) = s.post("/import/link/preview", json!({ "url": link })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v, json!({ "status": "file_only", "source": "janitorai", "page_url": link, "reason_code": "definition_private" }));
    // RisuRealm: a PNG card.
    let (status, v) = s.post("/import/link/preview", json!({ "url": format!("https://realm.risuai.net/character/{RISU_ID}") })).await;
    assert_eq!((status, v["status"].as_str(), v["has_image"].as_bool()), (StatusCode::OK, Some("preview"), Some(true)), "{v}");
    assert_eq!(s.characters().await, 0);
    assert_eq!(site.seen.lock().unwrap().len(), 3, "two failed fetches and one preview");
}

// ---------------------------------------------------------------------------------------------
// Against the real sites (run by hand: `cargo test -p rp-server real_ -- --ignored`)

/// A public SFW card on Chub (a writing assistant).
const REAL_CHUB: &str = "https://chub.ai/characters/Luke_Jones/writing-assistant-aa9ef49358da";
/// A public SFW card on RisuRealm (a coding/utility assistant).
const REAL_RISU: &str = "https://realm.risuai.net/character/fa33d702-5e08-45a3-835d-26cdc325e8e1";

async fn real(name: &str, link: &str, source: &str) {
    let s = server(name).await;
    let (status, v) = s.link(link).await;
    assert_eq!(status, StatusCode::OK, "{}", v["error"]);
    assert_eq!((v["status"].as_str(), v["source"].as_str()), (Some("imported"), Some(source)), "status {}", v["status"]);
    let id = character_id(&v);
    assert!(s.has_picture(id).await, "picture missing");
    assert_eq!(s.source_url(id).as_deref(), Some(link));
    println!("{source}: imported with picture");
}

#[tokio::test]
#[ignore = "fetches from chub.ai"]
async fn real_chub() {
    real("real-chub", REAL_CHUB, "chub").await;
}

#[tokio::test]
#[ignore = "fetches from realm.risuai.net"]
async fn real_risu() {
    real("real-risu", REAL_RISU, "risurealm").await;
}
