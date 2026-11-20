//! Download and API tests against a real local HTTP server (axum) that answers Range,
//! If-Range, redirects and 429 like a CDN would.

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::StreamExt;
use rp_models::download::http_client;
use rp_models::{download_file, CancellationToken, Error, Expected, HfClient, Retry};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ETAG: &str = "\"v1\"";
const LEN: usize = 300_000;
const CHUNK: usize = 8192;

#[derive(Debug, Clone)]
struct Hit {
    path: String,
    range: Option<String>,
    if_range: Option<String>,
}

#[derive(Clone)]
struct Srv {
    data: Arc<Vec<u8>>,
    hits: Arc<Mutex<Vec<Hit>>>,
    flaky_left: Arc<AtomicU32>,
    cut_left: Arc<AtomicU32>,
}

fn data() -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    (0..LEN)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
}

fn hdr(h: &HeaderMap, name: header::HeaderName) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// Serves the blob with Range/If-Range support in small, slightly delayed chunks.
/// `cut` sends only that many body bytes although the full length is announced.
fn serve(s: &Srv, path: &str, h: &HeaderMap, honor_range: bool, cut: Option<usize>) -> Response {
    s.hits.lock().unwrap().push(Hit {
        path: path.to_string(),
        range: hdr(h, header::RANGE),
        if_range: hdr(h, header::IF_RANGE),
    });
    let len = s.data.len();
    let range_start = hdr(h, header::RANGE)
        .and_then(|r| r.strip_prefix("bytes=")?.strip_suffix('-')?.parse::<usize>().ok());
    let if_range_ok = hdr(h, header::IF_RANGE).is_none_or(|v| v == ETAG);
    let (status, start) = match range_start {
        Some(n) if honor_range && if_range_ok => {
            if n >= len {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                    .body(Body::empty())
                    .unwrap();
            }
            (StatusCode::PARTIAL_CONTENT, n)
        }
        _ => (StatusCode::OK, 0),
    };
    let end = cut.map_or(len, |c| (start + c).min(len));
    let chunks: Vec<Vec<u8>> = s.data[start..end].chunks(CHUNK).map(<[u8]>::to_vec).collect();
    let body = futures_util::stream::iter(chunks).then(|c| async move {
        tokio::time::sleep(Duration::from_millis(1)).await;
        Ok::<_, std::io::Error>(c)
    });
    let mut b = Response::builder()
        .status(status)
        .header(header::ETAG, ETAG)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, (len - start).to_string());
    if status == StatusCode::PARTIAL_CONTENT {
        b = b.header(header::CONTENT_RANGE, format!("bytes {start}-{}/{len}", len - 1));
    }
    b.body(Body::from_stream(body)).unwrap()
}

async fn files(State(s): State<Srv>, UrlPath(name): UrlPath<String>, h: HeaderMap) -> Response {
    serve(&s, &format!("/files/{name}"), &h, true, None)
}

async fn norange(State(s): State<Srv>, UrlPath(name): UrlPath<String>, h: HeaderMap) -> Response {
    serve(&s, &format!("/norange/{name}"), &h, false, None)
}

async fn redirect(UrlPath(name): UrlPath<String>) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, format!("/files/{name}"))]).into_response()
}

async fn flaky(State(s): State<Srv>, UrlPath(name): UrlPath<String>, h: HeaderMap) -> Response {
    let left = s.flaky_left.load(Ordering::SeqCst);
    if left > 0 {
        s.flaky_left.store(left - 1, Ordering::SeqCst);
        s.hits.lock().unwrap().push(Hit { path: "429".into(), range: None, if_range: None });
        return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "0")]).into_response();
    }
    serve(&s, &format!("/flaky/{name}"), &h, true, None)
}

async fn cut(State(s): State<Srv>, UrlPath(name): UrlPath<String>, h: HeaderMap) -> Response {
    let first = s.cut_left.fetch_sub(1, Ordering::SeqCst) > 0;
    serve(&s, &format!("/cut/{name}"), &h, true, first.then_some(LEN / 2))
}

/// Like Hugging Face for a Xet file: 302 with a body of its own (`Content-Length` 1300) and
/// `X-Linked-Size` / `X-Linked-ETag` naming the real file, to `to` on the same server.
fn hub_redirect(s: &Srv, to: &str, name: &str) -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, format!("/{to}/{name}"))
        .header(header::CONTENT_LENGTH, "1300")
        .header("x-linked-size", LEN.to_string())
        .header("x-linked-etag", format!("\"{}\"", sha(&s.data)))
        .body(Body::from(vec![b'x'; 1300]))
        .unwrap()
}

async fn hub_stall(State(s): State<Srv>, UrlPath(name): UrlPath<String>) -> Response {
    hub_redirect(&s, "stall", &name)
}

async fn hub_files(State(s): State<Srv>, UrlPath(name): UrlPath<String>) -> Response {
    hub_redirect(&s, "files", &name)
}

/// A CDN that stops sending: the first answer brings 1 KB and then nothing, every resume only
/// its headers (as the Xet bridge did while it rebuilt an uncached range).
async fn stall(State(s): State<Srv>, UrlPath(name): UrlPath<String>, h: HeaderMap) -> Response {
    s.hits.lock().unwrap().push(Hit { path: format!("/stall/{name}"), range: hdr(&h, header::RANGE), if_range: hdr(&h, header::IF_RANGE) });
    let start = hdr(&h, header::RANGE).and_then(|r| r.strip_prefix("bytes=")?.strip_suffix('-')?.parse::<usize>().ok()).unwrap_or(0);
    let head: Vec<u8> = if start == 0 { s.data[..1024].to_vec() } else { Vec::new() };
    let body = futures_util::stream::once(async move { Ok::<_, std::io::Error>(head) }).chain(futures_util::stream::pending());
    let mut b = Response::builder()
        .status(if start == 0 { StatusCode::OK } else { StatusCode::PARTIAL_CONTENT })
        .header(header::ETAG, ETAG)
        .header(header::CONTENT_LENGTH, (LEN - start).to_string());
    if start > 0 {
        b = b.header(header::CONTENT_RANGE, format!("bytes {start}-{}/{LEN}", LEN - 1));
    }
    b.body(Body::from_stream(body)).unwrap()
}

async fn api_search() -> Json<serde_json::Value> {
    Json(serde_json::json!([
        {"id": "org/open-GGUF", "downloads": 1234, "likes": 7, "lastModified": "2026-11-01T00:00:00.000Z",
         "gated": false, "gguf": {"total": 4_600_000_000u64, "architecture": "gemma4", "context_length": 131072, "chat_template": "{{x}}"}},
        {"id": "locked/model-GGUF", "downloads": 3, "gated": "manual"}
    ]))
}

async fn api_info(UrlPath((org, name)): UrlPath<(String, String)>) -> Response {
    if org == "locked" {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({
        "id": format!("{org}/{name}"), "gated": false, "lastModified": "2026-11-01T00:00:00.000Z",
        "tags": ["gguf", "license:apache-2.0"], "cardData": {"license": "gemma"}
    }))
    .into_response()
}

async fn api_tree(State(s): State<Srv>, UrlPath((org, _name)): UrlPath<(String, String)>) -> Response {
    if org == "locked" {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!([
        {"type": "directory", "oid": "abc", "size": 0, "path": "sub"},
        {"type": "file", "oid": "deadbeef", "size": 30, "path": "README.md"},
        {"type": "file", "oid": "1111", "size": 134, "path": "sub/model-Q4_K_M.gguf",
         "lfs": {"oid": sha(&s.data).to_uppercase(), "size": LEN, "pointerSize": 134}}
    ]))
    .into_response()
}

async fn resolve(
    State(s): State<Srv>,
    UrlPath((org, _name, path)): UrlPath<(String, String, String)>,
    h: HeaderMap,
) -> Response {
    if org == "locked" {
        return StatusCode::FORBIDDEN.into_response();
    }
    serve(&s, &format!("/resolve/{path}"), &h, true, None)
}

async fn start() -> (String, Srv) {
    let s = Srv {
        data: Arc::new(data()),
        hits: Arc::default(),
        flaky_left: Arc::new(AtomicU32::new(0)),
        cut_left: Arc::new(AtomicU32::new(1)),
    };
    let app = Router::new()
        .route("/files/{name}", get(files))
        .route("/norange/{name}", get(norange))
        .route("/redirect/{name}", get(redirect))
        .route("/flaky/{name}", get(flaky))
        .route("/cut/{name}", get(cut))
        .route("/stall/{name}", get(stall))
        .route("/hub-stall/{name}", get(hub_stall))
        .route("/hub-files/{name}", get(hub_files))
        .route("/api/models", get(api_search))
        .route("/api/models/{org}/{name}", get(api_info))
        .route("/api/models/{org}/{name}/tree/main", get(api_tree))
        .route("/{org}/{name}/resolve/main/{*path}", get(resolve))
        .with_state(s.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), s)
}

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "rp-models-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fast() -> Retry {
    Retry { attempts: 3, base_delay: Duration::from_millis(10), max_delay: Duration::from_millis(50) }
}

fn expected(s: &Srv) -> Expected {
    Expected { size: Some(LEN as u64), sha256: Some(sha(&s.data)) }
}

fn part_of(p: &std::path::Path) -> PathBuf {
    PathBuf::from(format!("{}.part", p.display()))
}

#[tokio::test]
async fn complete_download_with_progress() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("deep/dir/m.gguf");
    let mut last = None;
    download_file(&http_client(), &format!("{base}/files/m.gguf"), &dest, &expected(&s), fast(), &CancellationToken::new(), |p| last = Some(p))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    assert!(!part_of(&dest).exists());
    let p = last.unwrap();
    assert_eq!((p.downloaded, p.total), (LEN as u64, Some(LEN as u64)));
    assert!(p.bytes_per_sec > 0.0);
    assert_eq!(s.hits.lock().unwrap()[0].range, None);
}

#[tokio::test]
async fn cancel_then_resume() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    let url = format!("{base}/files/m.gguf");
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let r = download_file(&http_client(), &url, &dest, &expected(&s), fast(), &cancel, move |p| {
        if p.downloaded > 0 {
            c2.cancel();
        }
    })
    .await;
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    let kept = std::fs::metadata(part_of(&dest)).unwrap().len();
    assert!(kept > 0 && kept < LEN as u64, "kept {kept}");
    assert!(!dest.exists());
    assert_eq!(&std::fs::read(part_of(&dest)).unwrap()[..], &s.data[..kept as usize]);

    download_file(&http_client(), &url, &dest, &expected(&s), fast(), &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    let hits = s.hits.lock().unwrap();
    let last = hits.last().unwrap();
    assert_eq!(last.range.as_deref(), Some(format!("bytes={kept}-").as_str()));
    assert_eq!(last.if_range.as_deref(), Some(ETAG));
}

#[tokio::test]
async fn broken_connection_is_resumed() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    download_file(&http_client(), &format!("{base}/cut/m.gguf"), &dest, &expected(&s), fast(), &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    let hits = s.hits.lock().unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits[1].range.is_some(), "second request must resume: {hits:?}");
}

#[tokio::test]
async fn wrong_hash_deletes_the_file() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    let exp = Expected { size: Some(LEN as u64), sha256: Some("00".repeat(32)) };
    let r = download_file(&http_client(), &format!("{base}/files/m.gguf"), &dest, &exp, fast(), &CancellationToken::new(), |_| {}).await;
    match r {
        Err(Error::HashMismatch { actual, .. }) => assert_eq!(actual, sha(&s.data)),
        other => panic!("{other:?}"),
    }
    assert!(!dest.exists() && !part_of(&dest).exists());
}

#[tokio::test]
async fn ok_instead_of_partial_starts_over() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    std::fs::write(part_of(&dest), vec![0xEE; 5000]).unwrap();
    download_file(&http_client(), &format!("{base}/norange/m.gguf"), &dest, &expected(&s), fast(), &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    assert_eq!(s.hits.lock().unwrap()[0].range.as_deref(), Some("bytes=5000-"));
}

#[tokio::test]
async fn changed_etag_starts_over() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    std::fs::write(part_of(&dest), vec![0xEE; 5000]).unwrap();
    std::fs::write(format!("{}.part.etag", dest.display()), "\"old\"").unwrap();
    download_file(&http_client(), &format!("{base}/files/m.gguf"), &dest, &expected(&s), fast(), &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    assert_eq!(s.hits.lock().unwrap()[0].if_range.as_deref(), Some("\"old\""));
}

#[tokio::test]
async fn redirect_keeps_range_header() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    std::fs::write(part_of(&dest), &s.data[..1000]).unwrap();
    download_file(&http_client(), &format!("{base}/redirect/m.gguf"), &dest, &expected(&s), fast(), &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    let hits = s.hits.lock().unwrap();
    assert_eq!(hits[0].path, "/files/m.gguf");
    assert_eq!(hits[0].range.as_deref(), Some("bytes=1000-"));
}

/// The real failure: 1 KB arrived, then the CDN went silent. That is a network problem and the
/// part file stays for later; it used to be reported as "other size" (`SizeMismatch`).
#[tokio::test]
async fn stalled_cdn_is_not_a_size_mismatch() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    let impatient = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .read_timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let r = download_file(&impatient, &format!("{base}/hub-stall/m.gguf"), &dest, &expected(&s), fast(), &CancellationToken::new(), |_| {}).await;
    assert!(matches!(r, Err(Error::Stalled { expected, actual: 1024 }) if expected == LEN as u64), "{r:?}");
    assert_eq!(std::fs::metadata(part_of(&dest)).unwrap().len(), 1024, "part file kept for a resume");
    let ranges: Vec<_> = s.hits.lock().unwrap().iter().map(|h| h.range.clone()).collect();
    assert_eq!(ranges[0], None);
    assert!(ranges[1..].iter().all(|r| r.as_deref() == Some("bytes=1024-")), "{ranges:?}");
}

/// The download waits as long as the Xet CDN was seen to take for a cold range (221 s).
#[test]
fn read_timeout_outlasts_a_cold_cdn() {
    assert!(rp_models::download::READ_TIMEOUT >= Duration::from_secs(240));
}

/// An out-of-date size and hash (old catalog) give way to what Hugging Face names on the
/// redirect; the bytes are checked against that SHA-256.
#[tokio::test]
async fn stale_expected_size_takes_the_hub_values() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    let stale = Expected { size: Some(LEN as u64 + 4096), sha256: Some("0".repeat(64)) };
    let mut last = None;
    download_file(&http_client(), &format!("{base}/hub-files/m.gguf"), &dest, &stale, fast(), &CancellationToken::new(), |p| last = Some(p))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    assert_eq!(last.unwrap().total, Some(LEN as u64));
    // Without the Hub's word a different size is still refused before any byte is written.
    let other = t.0.join("o.gguf");
    let r = download_file(&http_client(), &format!("{base}/files/o.gguf"), &other, &stale, fast(), &CancellationToken::new(), |_| {}).await;
    assert!(matches!(r, Err(Error::SizeMismatch { actual, .. }) if actual == LEN as u64), "{r:?}");
}

#[tokio::test]
async fn retries_after_429() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let dest = t.0.join("m.gguf");
    s.flaky_left.store(2, Ordering::SeqCst);
    download_file(&http_client(), &format!("{base}/flaky/m.gguf"), &dest, &expected(&s), fast(), &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    assert_eq!(s.hits.lock().unwrap().len(), 3);

    s.flaky_left.store(100, Ordering::SeqCst);
    let r = download_file(&http_client(), &format!("{base}/flaky/x.gguf"), &t.0.join("x.gguf"), &expected(&s), fast(), &CancellationToken::new(), |_| {}).await;
    assert!(matches!(r, Err(Error::Status { status: 429, .. })), "{r:?}");
}

#[tokio::test]
async fn too_little_space_is_refused_before_download() {
    let (base, s) = start().await;
    let t = Tmp::new();
    let exp = Expected { size: Some(u64::MAX / 4), sha256: None };
    let r = download_file(&http_client(), &format!("{base}/files/m.gguf"), &t.0.join("m.gguf"), &exp, fast(), &CancellationToken::new(), |_| {}).await;
    assert!(matches!(r, Err(Error::NoSpace { .. })), "{r:?}");
    assert!(s.hits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn hf_api_and_download() {
    let (base, s) = start().await;
    let hf = HfClient::with_base(&base).unwrap().with_retry(fast());

    let found = hf.search("gemma", 10).await.unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].gguf.as_ref().unwrap().architecture.as_deref(), Some("gemma4"));
    assert_eq!(found[1].gated.as_deref(), Some("manual"));

    let info = hf.repo_info("org/open-GGUF").await.unwrap();
    assert_eq!(info.license.as_deref(), Some("gemma"));
    assert_eq!(info.gated, None);
    assert!(info.tags.contains(&"gguf".to_string()));

    let files = hf.files("org/open-GGUF").await.unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].sha256, None);
    let gguf = &files[1];
    assert_eq!((gguf.path.as_str(), gguf.size), ("sub/model-Q4_K_M.gguf", LEN as u64));
    assert_eq!(gguf.sha256.as_deref(), Some(sha(&s.data).as_str()));

    let t = Tmp::new();
    let dest = t.0.join("model-Q4_K_M.gguf");
    hf.download("org/open-GGUF", gguf, &dest, &CancellationToken::new(), |_| {}).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *s.data);
    assert_eq!(s.hits.lock().unwrap()[0].path, "/resolve/sub/model-Q4_K_M.gguf");

    assert!(matches!(hf.files("locked/model-GGUF").await, Err(Error::Gated(r)) if r == "locked/model-GGUF"));
    let r = hf.download("locked/model-GGUF", gguf, &t.0.join("x.gguf"), &CancellationToken::new(), |_| {}).await;
    assert!(matches!(r, Err(Error::Gated(r)) if r == "locked/model-GGUF"));
}

/// Real Hugging Face, metadata only (no model download).
#[tokio::test]
#[ignore]
async fn real_hf_file_list() {
    let hf = HfClient::new();
    let repo = "unsloth/gemma-4-E2B-it-GGUF";
    let info = hf.repo_info(repo).await.unwrap();
    println!("info: gated={:?} license={:?} modified={:?} tags={}", info.gated, info.license, info.last_modified, info.tags.len());
    let files = hf.files(repo).await.unwrap();
    for f in files.iter().filter(|f| f.path.ends_with(".gguf")) {
        println!("  {} {} {}", f.path, f.size, f.sha256.as_deref().unwrap_or("-"));
    }
    assert!(files.iter().any(|f| f.path.ends_with(".gguf") && f.sha256.as_ref().is_some_and(|h| h.len() == 64)));
    let found = hf.search("gemma-4-E2B", 3).await.unwrap();
    for m in &found {
        println!("search: {} dl={} gated={:?} gguf={:?}", m.id, m.downloads, m.gated, m.gguf);
    }
    assert!(!found.is_empty());
}

/// Real Hugging Face (Xet storage, 302 to the CDN): the first ~2 MB of a catalog model, then
/// cancel. The announced total must be the real file size from the tree, not a body length.
#[tokio::test]
#[ignore]
async fn real_hf_download_head() {
    const REPO: &str = "llmfan46/Mistral-Small-3.2-24B-Instruct-2506-ultra-uncensored-heretic-GGUF";
    const FILE: &str = "Mistral-Small-3.2-24B-Instruct-2506-ultra-uncensored-heretic-Q5_K_M.gguf";
    const STOP: u64 = 2 * 1024 * 1024;
    let hf = HfClient::new();
    let (_, files) = hf.repo_files(REPO).await.unwrap();
    let file = files.iter().find(|f| f.path == FILE).unwrap().clone();
    println!("tree: size={} sha256={:?}", file.size, file.sha256);
    assert!(file.size > 15_000_000_000);
    let t = Tmp::new();
    let dest = t.0.join(FILE);
    // First call: stop after the first block. Second call: resume it (Range + If-Range).
    for stop in [1, STOP] {
        let cancel = CancellationToken::new();
        let mut seen = None;
        let r = hf
            .download(REPO, &file, &dest, &cancel, |p| {
                seen = Some(p);
                if p.downloaded >= stop {
                    cancel.cancel();
                }
            })
            .await;
        let p = seen.expect("progress");
        println!("result={r:?} downloaded={} total={:?}", p.downloaded, p.total);
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        assert_eq!(p.total, Some(file.size));
    }
    let head = std::fs::read(part_of(&dest)).unwrap();
    assert!(head.len() as u64 >= STOP && &head[..4] == b"GGUF");
}
