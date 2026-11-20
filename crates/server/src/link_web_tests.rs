//! The general way of the link import through the real server, against a real local web site
//! (card files, pages that link or embed a card, redirects), and the address rules: nothing on
//! this PC or in the home network is ever requested, also not after a redirect or through a
//! link found on a page. Plus `#[ignore]` tests against real public files.

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};

use crate::link_tests::{server, Server};

const ILSE_PNG: &[u8] = include_bytes!("../../../tests/fixtures/ilse_brandt.png");
const MARA_JSON: &str = include_str!("../../../tests/fixtures/mara_voss.json");
const MOOS_JSON: &str = include_str!("../../../tests/fixtures/moos.json");

#[derive(Clone, Default)]
struct Site {
    /// This site's base and the base of the other local site (which must never be reached).
    base: Arc<Mutex<(String, String)>>,
    /// Every requested path.
    seen: Arc<Mutex<Vec<String>>>,
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn html(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], format!("<!doctype html><html><head><title>x</title></head><body>{body}</body></html>")).into_response()
}

fn to(location: String) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

async fn site_file(State(site): State<Site>, Path((kind, name)): Path<(String, String)>) -> Response {
    site.seen.lock().unwrap().push(format!("/{kind}/{name}"));
    let (_, other) = site.base.lock().unwrap().clone();
    let png = |bytes: Vec<u8>| ([(header::CONTENT_TYPE, "image/png")], bytes).into_response();
    match (kind.as_str(), name.as_str()) {
        ("files", "ilse.png") => png(ILSE_PNG.to_vec()),
        // A card PNG served without a telling name or type.
        ("files", "download") => ([(header::CONTENT_TYPE, "application/octet-stream")], ILSE_PNG).into_response(),
        ("files", "mara.json") => ([(header::CONTENT_TYPE, "application/json")], MARA_JSON).into_response(),
        ("files", "cards.zip") => ([(header::CONTENT_TYPE, "application/zip")], zip_of(&[("a/mara.json", MARA_JSON.as_bytes()), ("readme.txt", b"hi"), ("moos.json", MOOS_JSON.as_bytes())])).into_response(),
        ("files", "notes.txt") => "just some notes".into_response(),
        ("files", "site.json") => axum::Json(json!({ "name": "A web app", "description": "not a card" })).into_response(),
        ("files", "huge.png") => png(vec![0; crate::link::MAX_CARD_IMAGE + 1]),
        ("img", "shot.png") | ("img", "logo.png") => png(rp_core::formats::placeholder_png()),
        ("img", "portrait.png") => png(ILSE_PNG.to_vec()),
        // A page that links the card among other things.
        ("page", "linked") => html(
            r#"<img src="/img/logo.png"><img src="/img/shot.png"><a href="/files/site.json">data</a>
               <a href="/files/notes.txt">notes</a><a href="/files/ilse.png" download>Karte herunterladen</a>"#
                .to_string(),
        ),
        // A page that carries the card in its data, with a picture of its own.
        ("page", "embedded") => {
            let card: Value = serde_json::from_str(MARA_JSON).unwrap();
            let data = json!({ "props": { "pageProps": { "character": { "id": 7, "card": card } } } });
            html(format!(r#"<meta property="og:image" content="/img/portrait.png"><script id="__NEXT_DATA__" type="application/json">{data}</script>"#))
        }
        ("page", "empty") => html(r#"<p>Nothing here.</p><img src="/img/shot.png"><a href="/files/site.json">x</a>"#.to_string()),
        // Links that point into the home network or at this PC.
        ("page", "inward") => html(format!(
            r#"<a href="{other}/files/ilse.png" download>card</a><a href="https://192.168.1.10/card.png">card</a>
               <a href="https://[::1]/card.png">card</a><meta property="og:image" content="https://10.0.0.5/card.png">"#
        )),
        ("page", "challenge") => (StatusCode::FORBIDDEN, [("cf-mitigated", "challenge")], "Just a moment...").into_response(),
        ("page", "login") => StatusCode::UNAUTHORIZED.into_response(),
        ("redirect", "ok") => to("/files/ilse.png".into()),
        ("redirect", "other-port") => to(format!("{other}/files/ilse.png")),
        ("redirect", "private") => to("https://10.0.0.5/card.png".into()),
        ("redirect", "linklocal") => to("https://169.254.169.254/latest/meta-data/".into()),
        ("redirect", "loopback6") => to("https://[::1]/card.png".into()),
        ("redirect", "http-public") => to("http://example.com:8080/card.png".into()),
        ("redirect", "loop") => to("/redirect/loop".into()),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn one_site() -> (Site, String) {
    let site = Site::default();
    let app = Router::new().route("/{kind}/{name}", get(site_file)).with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (site, base)
}

/// The site the server may fetch from (the exempt test origin) and a second local one that
/// stands for "something else on this PC": it must never see a request.
async fn sites() -> (Site, String, Site) {
    let (site, base) = one_site().await;
    let (other, other_base) = one_site().await;
    *site.base.lock().unwrap() = (base.clone(), other_base);
    (site, base, other)
}

async fn preview(s: &Server, url: &str) -> (StatusCode, Value) {
    s.post("/import/link/preview", json!({ "url": url })).await
}

fn file_only(v: &Value, reason: &str) -> bool {
    v["status"] == "file_only" && v["source"] == "web" && v["reason_code"] == reason && v["page_url"].is_string()
}

// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn direct_card_files_are_previewed_and_imported() {
    let (site, base, _) = sites().await;
    let s = Server::with_site("web-files", &base).await;

    // PNG card: preview, then import with the pasted address as its source.
    let link = format!("{base}/files/ilse.png");
    let (status, v) = preview(&s, &link).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["status"].as_str(), v["source"].as_str(), v["page_url"].as_str()), (Some("preview"), Some("web"), Some(link.as_str())), "{v}");
    assert_eq!((v["card"]["name"].as_str(), v["has_image"].as_bool(), v["more_cards"].as_u64()), (Some("Kapitän Ilse Brandt"), Some(true), Some(0)), "{v}");
    assert_eq!(s.characters().await, 0, "a preview stores nothing");
    let (status, done) = s.post("/import/link/confirm", json!({ "preview_id": v["preview_id"] })).await;
    assert_eq!((status, done["status"].as_str(), done["source"].as_str()), (StatusCode::OK, Some("imported"), Some("web")), "{done}");
    let id = done["character_id"].as_i64().unwrap();
    assert_eq!(s.source_url(id).as_deref(), Some(link.as_str()));
    assert!(s.has_picture(id).await);

    // The same card under another address (no file ending, a plain download type): a duplicate.
    let (_, v) = preview(&s, &format!("{base}/files/download")).await;
    assert_eq!((v["status"].as_str(), v["duplicate_of"].as_i64()), (Some("preview"), Some(id)), "{v}");

    // JSON card, through a redirect on the same site, with the one-step route.
    let (status, v) = s.post("/import/link", json!({ "url": format!("{base}/files/mara.json#top") })).await;
    assert_eq!((status, v["status"].as_str(), v["source"].as_str()), (StatusCode::OK, Some("imported"), Some("web")), "{v}");
    let (_, v) = preview(&s, &format!("{base}/redirect/ok")).await;
    assert_eq!((v["status"].as_str(), v["duplicate_of"].as_i64()), (Some("preview"), Some(id)), "{v}");

    // One request per pasted link (the redirect: two), with the honest user agent's path only.
    let seen = site.seen.lock().unwrap().clone();
    assert_eq!(seen, ["/files/ilse.png", "/files/download", "/files/mara.json", "/redirect/ok", "/files/ilse.png"]);
}

#[tokio::test]
async fn a_zip_of_cards_behind_a_link_brings_them_all() {
    let (_, base, _) = sites().await;
    let s = Server::with_site("web-zip", &base).await;
    let (status, v) = preview(&s, &format!("{base}/files/cards.zip")).await;
    assert_eq!((status, v["status"].as_str(), v["more_cards"].as_u64()), (StatusCode::OK, Some("preview"), Some(1)), "{v}");
    assert_eq!(v["card"]["name"], "Mara Voss");
    let (_, done) = s.post("/import/link/confirm", json!({ "preview_id": v["preview_id"] })).await;
    assert_eq!(done["status"], "imported", "{done}");
    assert_eq!(done["more"], json!({ "imported": 1, "duplicates": 0, "errors": 0 }), "{done}");
    assert_eq!(s.characters().await, 2);
}

#[tokio::test]
async fn pages_with_a_linked_or_embedded_card() {
    let (site, base, _) = sites().await;
    let s = Server::with_site("web-pages", &base).await;

    // Linked: the download link wins; the logo, the screenshot and the site's JSON are no cards.
    let link = format!("{base}/page/linked");
    let (status, v) = preview(&s, &link).await;
    assert_eq!((status, v["status"].as_str(), v["page_url"].as_str()), (StatusCode::OK, Some("preview"), Some(link.as_str())), "{v}");
    assert_eq!((v["card"]["name"].as_str(), v["has_image"].as_bool()), (Some("Kapitän Ilse Brandt"), Some(true)), "{v}");
    let seen = site.seen.lock().unwrap().clone();
    assert_eq!(seen, ["/page/linked", "/files/ilse.png"], "the likeliest file first, nothing after the hit");

    // Embedded: the card from the page's data, the page's picture as the card's.
    site.seen.lock().unwrap().clear();
    let link = format!("{base}/page/embedded");
    let (_, v) = preview(&s, &link).await;
    assert_eq!((v["status"].as_str(), v["card"]["name"].as_str(), v["has_image"].as_bool()), (Some("preview"), Some("Mara Voss"), Some(true)), "{v}");
    assert_eq!(*site.seen.lock().unwrap(), ["/page/embedded", "/img/portrait.png"]);
    let (_, done) = s.post("/import/link/confirm", json!({ "preview_id": v["preview_id"] })).await;
    let id = done["character_id"].as_i64().unwrap();
    assert_eq!(s.source_url(id).as_deref(), Some(link.as_str()));
    assert!(s.has_picture(id).await);

    // No card: the user is sent to the page, nothing is stored. At most the page plus
    // MAX_CANDIDATES files are read.
    site.seen.lock().unwrap().clear();
    for page in ["page/empty", "files/notes.txt"] {
        let (status, v) = preview(&s, &format!("{base}/{page}")).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert!(file_only(&v, "no_card_found"), "{page}: {v}");
    }
    assert!(site.seen.lock().unwrap().len() <= 2 + crate::link_web::MAX_CANDIDATES);
    assert_eq!(s.characters().await, 1);
}

#[tokio::test]
async fn bot_checks_and_failures_are_not_worked_around() {
    let (site, base, _) = sites().await;
    let s = Server::with_site("web-blocked", &base).await;
    for page in ["page/challenge", "page/login"] {
        let (status, v) = preview(&s, &format!("{base}/{page}")).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert!(file_only(&v, "bot_protection"), "{page}: {v}");
    }
    assert_eq!(site.seen.lock().unwrap().len(), 2, "one request each, no second try");
    for (page, status, code) in [
        ("files/gone.png", StatusCode::NOT_FOUND, "source_not_found"),
        ("files/huge.png", StatusCode::BAD_GATEWAY, "source_too_large"),
        ("redirect/loop", StatusCode::BAD_GATEWAY, "source_redirect"),
    ] {
        let (got, v) = preview(&s, &format!("{base}/{page}")).await;
        assert_eq!((got, v["error"].as_str()), (status, Some(code)), "{page}: {v}");
    }
}

#[tokio::test]
async fn nothing_on_this_pc_or_in_the_home_network_is_requested() {
    let (site, base, other) = sites().await;
    let s = Server::with_site("web-ssrf", &base).await;
    let (other_base, app_port) = (site.base.lock().unwrap().1.clone(), s.running.addr.port());

    // Pasted directly.
    for link in [
        "https://127.0.0.1/card.png".to_string(),
        "http://127.0.0.1/card.png".to_string(),
        "https://10.1.2.3/card.png".to_string(),
        "https://192.168.178.1/card.png".to_string(),
        "https://169.254.169.254/latest/meta-data/".to_string(),
        "https://[::1]/card.png".to_string(),
        "https://[fd12:3456:789a::1]/card.png".to_string(),
        "https://[fe80::1]/card.png".to_string(),
        "https://[::ffff:192.168.1.1]/card.png".to_string(),
        "https://example.com:8443/card.png".to_string(),
        // The app's own API and another program on this PC.
        format!("http://127.0.0.1:{app_port}/api/library"),
        format!("{other_base}/files/ilse.png"),
    ] {
        let (status, v) = preview(&s, &link).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("address_not_allowed")), "{link}: {v}");
        let (status, v) = s.post("/import/link", json!({ "url": link })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("address_not_allowed")), "{link}: {v}");
    }

    // Reached through a redirect of an allowed site.
    for page in ["other-port", "private", "linklocal", "loopback6", "http-public"] {
        let (status, v) = preview(&s, &format!("{base}/redirect/{page}")).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("address_not_allowed")), "{page}: {v}");
    }

    // Reached through links on an allowed page: skipped, so the page has no card.
    let (status, v) = preview(&s, &format!("{base}/page/inward")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(file_only(&v, "no_card_found"), "{v}");

    assert!(other.seen.lock().unwrap().is_empty(), "the other local program saw a request: {:?}", other.seen.lock().unwrap());
    assert_eq!(s.characters().await, 0);
}

// ---------------------------------------------------------------------------------------------
// Against the real internet (run by hand: `cargo test -p rp-server real_web -- --ignored`)

/// A public SFW card PNG in SillyTavern's own repository (its default assistant), by page link.
const REAL_GITHUB: &str = "https://github.com/SillyTavern/SillyTavern/blob/release/default/content/default_Seraphina.png";

#[tokio::test]
#[ignore = "fetches from github.com"]
async fn real_web_github_file_page() {
    let s = server("real-web-github").await;
    let (status, v) = preview(&s, REAL_GITHUB).await;
    assert_eq!((status, v["status"].as_str(), v["source"].as_str()), (StatusCode::OK, Some("preview"), Some("web")), "{}", v["error"]);
    assert_eq!(v["has_image"], true);
    println!("github: preview with picture");
}
