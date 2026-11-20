//! `POST /pictures/from-link {url}`: the picture behind a link the user pasted into a picture field
//! (character or persona), fetched by the server. The page itself may only talk to this server
//! (CSP `connect-src 'self'`), so the picture comes this way. Every request goes through `netguard`
//! (HTTPS on 443 only, public addresses pinned, redirects re-checked, no proxy, one 30 s budget);
//! the answer is handed on byte for byte only when it is a PNG, JPEG, WebP or GIF by its first
//! bytes and decodes within the limits of `imaging`. Nothing is stored here: the editor stages the
//! picture until the user saves it.

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use reqwest::Url;
use serde::Deserialize;

use crate::error::{ApiError, ApiResult};
use crate::link::MAX_CARD_IMAGE;
use crate::netguard::Guard;
use crate::{imaging, import, link_web, store, AppState};

/// What the request may say it takes; the answer is judged by its bytes anyway.
const ACCEPT: &str = "image/png,image/jpeg,image/webp,image/gif,image/*;q=0.8";

#[derive(Deserialize)]
pub(super) struct Link {
    url: String,
}

/// The address, if it is an http(s) link at all (`bad_url` otherwise).
fn parse(text: &str) -> Result<Url, ApiError> {
    let mut url = Url::parse(text.trim()).map_err(|_| ApiError::bad_request("bad_url"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::bad_request("bad_url"));
    }
    url.set_fragment(None);
    Ok(url)
}

/// The picture's bytes with its type (`image/png`, `image/jpeg`, `image/webp`, `image/gif`); errors
/// as the link import's (`address_not_allowed`, `source_not_found`, `source_too_large`, …) or
/// 400 `bad_image` when the answer is not such a picture.
pub(super) async fn post_from_link(State(st): State<AppState>, Json(link): Json<Link>) -> ApiResult {
    let url = parse(&link.url)?;
    // The picture and its decoded copy count against the program-wide import budget.
    let _room = import::reserve(MAX_CARD_IMAGE as u64).await;
    let guard = Guard::new(st.links.get().test_origin, link_web::BUDGET);
    let got = guard.get(url, ACCEPT, MAX_CARD_IMAGE).await.map_err(|e| e.api())?;
    let mime = store::image_mime(&got.body).ok_or(ApiError::bad_request("bad_image"))?;
    let body = tokio::task::spawn_blocking(move || imaging::readable(&got.body).then_some(got.body))
        .await
        .ok()
        .flatten()
        .ok_or(ApiError::bad_request("bad_image"))?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use reqwest::Url;
    use serde_json::{json, Value};

    use crate::link::{Endpoints, MAX_CARD_IMAGE};
    use crate::link_tests::{server, Server};

    const ILSE_PNG: &[u8] = include_bytes!("../../../../tests/fixtures/ilse_brandt.png");

    /// A local web site: a picture, a page, a file too large, a broken picture behind PNG's first
    /// bytes, and redirects to addresses inside the home network and to this PC.
    async fn site() -> String {
        let jpeg = {
            let img = image::RgbImage::from_fn(30, 20, |x, y| image::Rgb([x as u8 * 8, y as u8 * 12, 90]));
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
            out.into_inner()
        };
        let app = Router::new()
            .route("/pic.png", get(|| async { ([(header::CONTENT_TYPE, "image/png")], ILSE_PNG).into_response() }))
            // Sent as octet-stream: judged by its bytes.
            .route("/photo", get(move || async move { ([(header::CONTENT_TYPE, "application/octet-stream")], jpeg).into_response() }))
            .route("/page", get(|| async { ([(header::CONTENT_TYPE, "text/html")], "<!doctype html><title>x</title>").into_response() }))
            .route("/huge.png", get(|| async { ([(header::CONTENT_TYPE, "image/png")], vec![0u8; MAX_CARD_IMAGE + 1]).into_response() }))
            .route("/broken.png", get(|| async { ([(header::CONTENT_TYPE, "image/png")], b"\x89PNG\r\n\x1a\nnot really a picture".to_vec()).into_response() }))
            .route("/to-lan", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "https://192.168.178.1/router.png")]).into_response() }))
            .route("/to-metadata", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "http://169.254.169.254/latest/meta-data/")]).into_response() }))
            .route("/to-localhost", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "https://localhost/secret.png")]).into_response() }))
            .route("/missing.png", get(|| async { StatusCode::NOT_FOUND.into_response() }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        base
    }

    async fn with_site(name: &str, base: &str) -> Server {
        let s = server(name).await;
        let url = Url::parse(&format!("{base}/")).unwrap();
        s.running.link_sources().set(Endpoints { test_origin: Some(url), ..Endpoints::default() });
        s
    }

    async fn fetch(s: &Server, url: &str) -> (StatusCode, String, Vec<u8>) {
        let res = s.c.post(format!("{}/pictures/from-link", s.base)).header("authorization", &s.auth).json(&json!({ "url": url })).send().await.unwrap();
        let status = StatusCode::from_u16(res.status().as_u16()).unwrap();
        let mime = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        (status, mime, res.bytes().await.unwrap().to_vec())
    }

    async fn refused(s: &Server, url: &str) -> (StatusCode, String) {
        let (status, _, body) = fetch(s, url).await;
        let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, v["error"].as_str().unwrap_or("").to_string())
    }

    #[tokio::test]
    async fn a_picture_comes_byte_for_byte_with_its_type() {
        let base = site().await;
        let s = with_site("pic-ok", &base).await;
        let (status, mime, body) = fetch(&s, &format!("{base}/pic.png")).await;
        assert_eq!((status, mime.as_str()), (StatusCode::OK, "image/png"));
        assert_eq!(body, ILSE_PNG);
        // The type comes from the bytes, not from what the site claims.
        let (status, mime, body) = fetch(&s, &format!("{base}/photo#frag")).await;
        assert_eq!((status, mime.as_str()), (StatusCode::OK, "image/jpeg"));
        assert!(body.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[tokio::test]
    async fn local_and_private_addresses_are_refused() {
        // No exempt origin at all: the app as it ships.
        let s = server("pic-private").await;
        for link in [
            "https://localhost/a.png",
            "http://localhost/a.png",
            "https://127.0.0.1/a.png",
            "http://127.0.0.1:8080/a.png",
            "https://192.168.1.20/a.png",
            "https://10.0.0.1/a.png",
            "https://172.16.5.4/a.png",
            "https://169.254.169.254/latest/meta-data/",
            "https://[::1]/a.png",
            "https://[fd00::5]/a.png",
            "https://[::ffff:192.168.0.1]/a.png",
            "https://example.com:8443/a.png",
            "https://user:pw@example.com/a.png",
            // Loopback and metadata written the other ways a URL parser accepts.
            "https://2130706433/a.png",
            "https://0x7f000001/a.png",
            "https://0177.0.0.1/a.png",
            "https://127.1/a.png",
            "https://0xa9fea9fe/latest/meta-data/",
            "https://[::ffff:169.254.169.254]/latest/meta-data/",
            "https://[::]/a.png",
            // Parser confusion: the host is what the URL says, not what a filter might read.
            "https://127.0.0.1#@example.com/a.png",
            "https://127.0.0.1\\@example.com/a.png",
            "https://example.com@127.0.0.1/a.png",
        ] {
            assert_eq!(refused(&s, link).await, (StatusCode::BAD_REQUEST, "address_not_allowed".into()), "{link}");
        }
        for link in ["file:///C:/Windows/win.ini", "ftp://example.com/a.png", "gopher://127.0.0.1:6379/_INFO", "javascript:alert(1)", "not a link", ""] {
            assert_eq!(refused(&s, link).await, (StatusCode::BAD_REQUEST, "bad_url".into()), "{link}");
        }
    }

    #[tokio::test]
    async fn the_exempt_test_origin_is_the_only_local_one() {
        let base = site().await;
        let s = with_site("pic-origin", &base).await;
        // The same site under another name, and the app's own server: refused.
        let other = base.replace("127.0.0.1", "localhost");
        assert_eq!(refused(&s, &format!("{other}/pic.png")).await, (StatusCode::BAD_REQUEST, "address_not_allowed".into()));
        let own = s.base.trim_end_matches("/api").to_string();
        assert_eq!(refused(&s, &format!("{own}/api/library")).await, (StatusCode::BAD_REQUEST, "address_not_allowed".into()));
    }

    #[tokio::test]
    async fn redirects_into_the_home_network_are_refused() {
        let base = site().await;
        let s = with_site("pic-redirect", &base).await;
        for path in ["/to-lan", "/to-metadata", "/to-localhost"] {
            assert_eq!(refused(&s, &format!("{base}{path}")).await, (StatusCode::BAD_REQUEST, "address_not_allowed".into()), "{path}");
        }
    }

    #[tokio::test]
    async fn too_large_pages_broken_pictures_and_gaps() {
        let base = site().await;
        let s = with_site("pic-bad", &base).await;
        assert_eq!(refused(&s, &format!("{base}/huge.png")).await, (StatusCode::BAD_GATEWAY, "source_too_large".into()));
        assert_eq!(refused(&s, &format!("{base}/page")).await, (StatusCode::BAD_REQUEST, "bad_image".into()));
        assert_eq!(refused(&s, &format!("{base}/broken.png")).await, (StatusCode::BAD_REQUEST, "bad_image".into()));
        assert_eq!(refused(&s, &format!("{base}/missing.png")).await, (StatusCode::NOT_FOUND, "source_not_found".into()));
    }
}
