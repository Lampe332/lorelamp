//! The built web UI, compiled into the program (feature `embed-ui`), so the app needs no
//! `apps/web/dist` folder at run time. Same answers as serving the folder: the file's content
//! type, `index.html` for every unknown path (the UI's hash routes, a missing icon), plus cache
//! headers: `index.html` is revalidated on every load, the hashed files in `assets/` are kept.

use std::borrow::Cow;

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::Embed)]
#[folder = "../../apps/web/dist/"]
struct Dist;

/// Vite puts a content hash into every file name below `assets/`: a new build means a new name.
const KEEP: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

/// Fallback handler for every path outside `/api`.
pub(crate) async fn serve(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "GET,HEAD")]).into_response();
    }
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() || path.ends_with('/') {
        path.push_str("index.html");
    }
    let (path, file) = match Dist::get(&path) {
        Some(file) => (path, file),
        None => match Dist::get("index.html") {
            Some(file) => ("index.html".to_string(), file),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    let etag = format!("\"{}\"", hex(&file.metadata.sha256_hash()[..16]));
    let cache = if path.starts_with("assets/") { KEEP } else { REVALIDATE };
    let mut res = if headers.get(header::IF_NONE_MATCH).is_some_and(|v| v.as_bytes() == etag.as_bytes()) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let body = match file.data {
            Cow::Borrowed(b) => Bytes::from_static(b),
            Cow::Owned(v) => Bytes::from(v),
        };
        let mut res = Response::new(Body::from(body));
        if let Ok(v) = HeaderValue::from_str(file.metadata.mimetype()) {
            res.headers_mut().insert(header::CONTENT_TYPE, v);
        }
        res
    };
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    res
}

/// One file of the built UI (`manifest.webmanifest` for `routes::lan::get_manifest`).
pub(crate) fn file(path: &str) -> Option<Vec<u8>> {
    Dist::get(path).map(|f| f.data.into_owned())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{logs, start, Config, Logger, WebUi};

    /// Through the real server with the embedded UI: page, hashed asset with its type, the
    /// fallback for unknown paths, cache headers, revalidation, and still no log entries.
    #[tokio::test]
    async fn embedded_ui_is_served() {
        let dir = std::env::temp_dir().join(format!("rp-app-embed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = Logger::start(&dir.join("data"));
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Embedded,
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
        let index = Dist::get("index.html").unwrap().data.into_owned();

        let page = client.get(format!("{base}/")).send().await.unwrap();
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(page.headers()[header::CONTENT_TYPE], "text/html");
        assert_eq!(page.headers()[header::CACHE_CONTROL], "no-cache");
        let etag = page.headers()[header::ETAG].clone();
        assert_eq!(page.bytes().await.unwrap().as_ref(), index.as_slice());

        let again = client.get(format!("{base}/index.html")).header(header::IF_NONE_MATCH, etag).send().await.unwrap();
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        for (ext, mime) in [(".js", "text/javascript"), (".css", "text/css")] {
            let name = Dist::iter().find(|f| f.starts_with("assets/") && f.ends_with(ext)).expect("asset in the build");
            let res = client.get(format!("{base}/{name}")).send().await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{name}");
            assert_eq!(res.headers()[header::CONTENT_TYPE], mime, "{name}");
            assert_eq!(res.headers()[header::CACHE_CONTROL], KEEP, "{name}");
            assert_eq!(res.bytes().await.unwrap().as_ref(), Dist::get(&name).unwrap().data.as_ref());
        }

        for (name, mime) in [("favicon.ico", "image/x-icon"), ("favicon.svg", "image/svg+xml")] {
            let res = client.get(format!("{base}/{name}")).send().await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{name}");
            assert_eq!(res.headers()[header::CONTENT_TYPE], mime, "{name}");
        }

        for path in ["/chat/12", "/no-such-icon.ico", "/assets/", "/assets/gone-123.js"] {
            let res = client.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            assert_eq!(res.headers()[header::CONTENT_TYPE], "text/html", "{path}");
            assert_eq!(res.headers()[header::CACHE_CONTROL], "no-cache", "{path}");
            assert_eq!(res.bytes().await.unwrap().as_ref(), index.as_slice(), "{path}");
        }

        let post = client.post(format!("{base}/")).send().await.unwrap();
        assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
        let api = client.get(format!("{base}/api/library")).send().await.unwrap(); // no token
        assert_eq!(api.status(), StatusCode::UNAUTHORIZED);

        log.flush(std::time::Duration::from_secs(5));
        let conn = logs::open_read(&dir.join("data")).unwrap();
        let mut http: Vec<String> = logs::query(&conn, &logs::Query { filter: "server".into(), ..Default::default() })
            .unwrap()
            .entries
            .into_iter()
            .filter(|e| e.category == "http")
            .map(|e| e.message)
            .collect();
        http.sort();
        assert_eq!(http, ["GET /api/library -> 401", "POST / -> 405"]);
        running.stop();
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }
}
