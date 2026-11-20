//! Downloads, links, catalog and search through the real server, against a local stand-in for
//! Hugging Face (axum) that answers the model API, redirects downloads to a "CDN" path, honours
//! `Range` + `If-Range` with 206, and answers 429 once when asked to.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{start, Config, Running, WebUi};

const ETAG: &str = "\"blob-v1\"";
const CHUNK: usize = 4096;

fn bytes(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
}

/// (path, content) per repo.
type Repos = HashMap<String, Vec<(String, Arc<Vec<u8>>)>>;
/// (path, Range header) per download request.
type Hits = Vec<(String, Option<String>)>;

#[derive(Clone, Default)]
struct Hf {
    /// repo → (path, content)
    repos: Arc<Mutex<Repos>>,
    /// Every download request: (path, Range header).
    hits: Arc<Mutex<Hits>>,
    api_hits: Arc<Mutex<Vec<String>>>,
    /// 429 answers still to give on downloads.
    flaky: Arc<AtomicU32>,
    /// Pause per chunk in microseconds.
    delay_us: Arc<AtomicU64>,
}

impl Hf {
    fn add(&self, repo: &str, path: &str, content: Vec<u8>) {
        self.repos.lock().unwrap().entry(repo.into()).or_default().push((path.into(), Arc::new(content)));
    }
    fn file(&self, repo: &str, path: &str) -> Option<Arc<Vec<u8>>> {
        self.repos.lock().unwrap().get(repo)?.iter().find(|f| f.0 == path).map(|f| f.1.clone())
    }
    fn ranges(&self, path: &str) -> Vec<Option<String>> {
        self.hits.lock().unwrap().iter().filter(|h| h.0.ends_with(path)).map(|h| h.1.clone()).collect()
    }
}

async fn api_repo(State(s): State<Hf>, UrlPath((org, name)): UrlPath<(String, String)>) -> Response {
    let repo = format!("{org}/{name}");
    s.api_hits.lock().unwrap().push(repo.clone());
    let files = s.repos.lock().unwrap().get(&repo).cloned();
    let Some(files) = files else { return StatusCode::NOT_FOUND.into_response() };
    let siblings: Vec<Value> = std::iter::once(json!({ "rfilename": "README.md", "size": 12 }))
        .chain(files.iter().map(|(p, c)| json!({ "rfilename": p, "size": c.len(), "lfs": { "sha256": sha(c), "size": c.len(), "pointerSize": 130 } })))
        .collect();
    let gated = if org == "locked" { json!("manual") } else { json!(false) };
    Json(json!({ "id": repo, "gated": gated, "lastModified": "2026-11-01T00:00:00.000Z", "tags": ["gguf", "license:apache-2.0"], "cardData": { "license": "apache-2.0" }, "siblings": siblings }))
        .into_response()
}

async fn api_search(State(s): State<Hf>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    s.api_hits.lock().unwrap().push(format!("search:{}", q.get("search").cloned().unwrap_or_default()));
    Json(json!([
        { "id": "bartowski/Tiny-GGUF", "downloads": 900, "likes": 3, "gguf": { "total": 1_000_000_000u64, "architecture": "llama", "context_length": 8192 } },
        { "id": "someone/Hype-GGUF", "downloads": 5_000_000, "likes": 3000 },
        { "id": "unsloth/Locked-GGUF", "downloads": 10, "gated": "auto" },
        { "id": "llmfan46/Qwen3.5-9B-Ultra-Uncensored-Heretic-GGUF", "downloads": 50 },
    ]))
}

async fn resolve(UrlPath((org, name, path)): UrlPath<(String, String, String)>) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, format!("/cdn/{org}/{name}/{path}"))]).into_response()
}

async fn cdn(State(s): State<Hf>, UrlPath((org, name, path)): UrlPath<(String, String, String)>, h: HeaderMap) -> Response {
    let range = h.get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);
    s.hits.lock().unwrap().push((path.clone(), range.clone()));
    if s.flaky.load(Ordering::SeqCst) > 0 {
        s.flaky.fetch_sub(1, Ordering::SeqCst);
        return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "0")]).into_response();
    }
    let Some(data) = s.file(&format!("{org}/{name}"), &path) else { return StatusCode::NOT_FOUND.into_response() };
    let len = data.len();
    let if_range_ok = h.get(header::IF_RANGE).is_none_or(|v| v == ETAG);
    let start = range.and_then(|r| r.strip_prefix("bytes=")?.strip_suffix('-')?.parse::<usize>().ok()).filter(|_| if_range_ok);
    let (status, from) = match start {
        Some(n) if n < len => (StatusCode::PARTIAL_CONTENT, n),
        Some(_) => return (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{len}"))]).into_response(),
        None => (StatusCode::OK, 0),
    };
    let delay = Duration::from_micros(s.delay_us.load(Ordering::SeqCst));
    let chunks: Vec<Vec<u8>> = data[from..].chunks(CHUNK).map(<[u8]>::to_vec).collect();
    let body = futures_util::stream::iter(chunks).then(move |c| async move {
        tokio::time::sleep(delay).await;
        Ok::<_, std::io::Error>(c)
    });
    let mut b = Response::builder()
        .status(status)
        .header(header::ETAG, ETAG)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, (len - from).to_string());
    if status == StatusCode::PARTIAL_CONTENT {
        b = b.header(header::CONTENT_RANGE, format!("bytes {from}-{}/{len}", len - 1));
    }
    b.body(Body::from_stream(body)).unwrap()
}

async fn stand_in() -> (Hf, String) {
    let hf = Hf::default();
    let app = Router::new()
        .route("/api/models", get(api_search))
        .route("/api/models/{org}/{name}", get(api_repo))
        .route("/{org}/{name}/resolve/main/{*path}", get(resolve))
        .route("/cdn/{org}/{name}/{*path}", get(cdn))
        .with_state(hf.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (hf, base)
}

struct Srv {
    running: Running,
    base: String,
    auth: String,
    c: reqwest::Client,
    data: std::path::PathBuf,
}

async fn server(dir: &std::path::Path, hf_base: &str) -> Srv {
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
    running.set_hf_base(hf_base).unwrap();
    let base = format!("http://127.0.0.1:{}/api", running.addr.port());
    let auth = format!("Bearer {}", running.token);
    Srv { running, base, auth, c: reqwest::Client::new(), data: dir.join("data") }
}

impl Srv {
    async fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = self.c.request(method, format!("{}{path}", self.base)).header("authorization", &self.auth);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.unwrap();
        let status = StatusCode::from_u16(res.status().as_u16()).unwrap();
        (status, res.json().await.unwrap_or(Value::Null))
    }
    async fn get(&self, path: &str) -> Value {
        self.send(reqwest::Method::GET, path, None).await.1
    }
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(reqwest::Method::POST, path, Some(body)).await
    }
    async fn download(&self, id: &str) -> Value {
        let v = self.get("/models/downloads").await;
        v["downloads"].as_array().unwrap().iter().find(|d| d["id"] == id).cloned().unwrap_or(Value::Null)
    }
    /// Polls the download list until `ok` holds for this download (at most 60 s).
    async fn wait(&self, id: &str, what: &str, ok: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..1200 {
            let d = self.download(id).await;
            if ok(&d) {
                return d;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for {what}: {}", self.download(id).await);
    }
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rp-app-fetch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resolve_links() {
    let (hf, base) = stand_in().await;
    for (p, n) in [("M-Q3_K_M.gguf", 300), ("M-Q4_K_M.gguf", 400), ("M-Q6_K.gguf", 600), ("Q8_0/M-Q8_0-00001-of-00002.gguf", 450), ("Q8_0/M-Q8_0-00002-of-00002.gguf", 450), ("mmproj-M-f16.gguf", 50)] {
        hf.add("org/M-GGUF", p, bytes(n, n as usize));
    }
    hf.add("locked/Secret-GGUF", "S-Q4_K_M.gguf", bytes(1, 10));
    let dir = temp("resolve");
    let s = server(&dir, &base).await;

    let (status, v) = s.post("/models/resolve", json!({ "url": "https://huggingface.co/org/M-GGUF/blob/main/M-Q4_K_M.gguf" })).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["repo"].as_str(), v["file"].as_str(), v["license"].as_str()), (Some("org/M-GGUF"), Some("M-Q4_K_M.gguf"), Some("apache-2.0")));
    assert_eq!(v["groups"]["small"]["recommended"], "M-Q3_K_M.gguf");
    assert_eq!(v["groups"]["balanced"]["choices"][0]["size"], 400);
    assert_eq!(v["groups"]["large"]["choices"].as_array().unwrap().len(), 2, "Q6_K and the two-part Q8_0: {v}");
    assert_eq!(v["groups"]["large"]["choices"][1]["files"].as_array().unwrap().len(), 2);
    assert_eq!(v["mmproj"][0]["path"], "mmproj-M-f16.gguf");
    assert_eq!(v["groups"]["balanced"]["choices"][0]["files"][0]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(hf.api_hits.lock().unwrap().len(), 1, "one request per user action");

    for (url, status, code) in [
        ("https://huggingface.co/locked/Secret-GGUF", StatusCode::FORBIDDEN, "gated"),
        ("org/Nope-GGUF", StatusCode::NOT_FOUND, "repo_missing"),
        ("https://example.com/org/M-GGUF", StatusCode::BAD_REQUEST, "bad_url"),
        ("https://huggingface.co/datasets/org/M", StatusCode::BAD_REQUEST, "bad_url"),
    ] {
        let (got, v) = s.post("/models/resolve", json!({ "url": url })).await;
        assert_eq!((got, v["error"].as_str()), (status, Some(code)), "{url}");
    }

    // Live search: only GGUF repos of known quanters / catalog owners, no gated ones.
    let v = s.get("/models/search?q=tiny").await;
    let ids: Vec<&str> = v["results"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["bartowski/Tiny-GGUF", "llmfan46/Qwen3.5-9B-Ultra-Uncensored-Heretic-GGUF"]);
    assert_eq!((v["results"][0]["curated"].as_bool(), v["results"][1]["curated"].as_bool()), (Some(false), Some(true)));
    assert_eq!(v["results"][0]["context"], 8192);
    assert_eq!(s.send(reqwest::Method::GET, "/models/search?q=x", None).await.0, StatusCode::BAD_REQUEST);
    s.running.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// A multi-part model with its projector: 429 once, pause mid-way, resume with Range (206),
/// SHA-256 checked, files in `<models>/<org>/<name>/`, then a hash error throws the file away,
/// and the checks of the request.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_pause_resume_and_hash() {
    let (hf, base) = stand_in().await;
    let part1 = bytes(7, 300_000);
    let part2 = bytes(8, 200_000);
    let proj = bytes(9, 30_000);
    hf.add("org/Big-GGUF", "Q8_0/Big-Q8_0-00001-of-00002.gguf", part1.clone());
    hf.add("org/Big-GGUF", "Q8_0/Big-Q8_0-00002-of-00002.gguf", part2.clone());
    hf.add("org/Big-GGUF", "mmproj-Big-f16.gguf", proj.clone());
    hf.delay_us.store(4_000, Ordering::SeqCst); // 4 KB every 4 ms ≈ 1 MB/s: slow enough to pause
    hf.flaky.store(1, Ordering::SeqCst);
    let dir = temp("dl");
    let s = server(&dir, &base).await;

    // Part 1 names the whole set: part 2 is added by the server.
    let (status, job) = s.post("/models/downloads", json!({ "repo": "org/Big-GGUF", "files": ["Q8_0/Big-Q8_0-00001-of-00002.gguf", "mmproj-Big-f16.gguf"] })).await;
    assert_eq!(status, StatusCode::CREATED, "{job}");
    let id = job["id"].as_str().unwrap().to_string();
    assert_eq!((job["total"].as_u64(), job["files"].as_array().unwrap().len(), job["name"].as_str()), (Some(530_000), 3, Some("Big-Q8_0")));
    let (status, v) = s.post("/models/downloads", json!({ "repo": "org/Big-GGUF", "file": "mmproj-Big-f16.gguf" })).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::CONFLICT, Some("already_downloading")));

    let d = s.wait(&id, "bytes arriving", |d| d["state"] == "running" && d["done"].as_u64().unwrap_or(0) > 60_000).await;
    assert!(d["total"] == 530_000);
    let (status, v) = s.post(&format!("/models/downloads/{id}/pause"), json!({})).await;
    assert_eq!((status, v["state"].as_str()), (StatusCode::OK, Some("paused")));
    let folder = std::path::PathBuf::from(d["folder"].as_str().unwrap());
    assert_eq!(folder, s.data.join("models").join("org").join("Big-GGUF"));
    let part = folder.join("Q8_0").join("Big-Q8_0-00001-of-00002.gguf.part");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let kept = std::fs::metadata(&part).unwrap().len();
    assert!(kept > 0 && kept < 300_000, "the part file stays: {kept}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(std::fs::metadata(&part).unwrap().len(), kept, "nothing written while paused");

    hf.delay_us.store(0, Ordering::SeqCst);
    let (status, _) = s.post(&format!("/models/downloads/{id}/resume"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let d = s.wait(&id, "done", |d| d["state"] == "done").await;
    assert_eq!(d["done"], 530_000);
    assert_eq!(std::fs::read(folder.join("Q8_0").join("Big-Q8_0-00001-of-00002.gguf")).unwrap(), part1);
    assert_eq!(std::fs::read(folder.join("Q8_0").join("Big-Q8_0-00002-of-00002.gguf")).unwrap(), part2);
    assert_eq!(std::fs::read(folder.join("mmproj-Big-f16.gguf")).unwrap(), proj);
    assert!(!part.exists());
    let ranges = hf.ranges("Big-Q8_0-00001-of-00002.gguf");
    assert_eq!(ranges[0], None, "first request without Range (answered 429)");
    assert!(ranges.iter().any(|r| r.as_deref() == Some(&format!("bytes={kept}-"))), "resumed at the kept length: {ranges:?}");

    // Already there.
    let (status, v) = s.post("/models/downloads", json!({ "repo": "org/Big-GGUF", "file": "mmproj-Big-f16.gguf" })).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::CONFLICT, Some("exists")));
    // A finished one leaves the list, the model stays.
    assert_eq!(s.send(reqwest::Method::DELETE, &format!("/models/downloads/{id}"), None).await.0, StatusCode::NO_CONTENT);
    assert!(s.download(&id).await.is_null());
    assert!(folder.join("mmproj-Big-f16.gguf").exists());

    // Wrong SHA-256 (as a stale catalog would give it): no request to the API, the file is thrown away.
    hf.add("org/Small-GGUF", "S-Q4_K_M.gguf", bytes(3, 50_000));
    let api_before = hf.api_hits.lock().unwrap().len();
    let (status, job) = s.post("/models/downloads", json!({ "repo": "org/Small-GGUF", "file": "S-Q4_K_M.gguf", "size": 50_000, "sha256": "0".repeat(64) })).await;
    assert_eq!(status, StatusCode::CREATED, "{job}");
    assert_eq!(hf.api_hits.lock().unwrap().len(), api_before, "size and SHA-256 given: no API request");
    let id = job["id"].as_str().unwrap().to_string();
    let d = s.wait(&id, "failed", |d| d["state"] == "failed").await;
    assert_eq!((d["error"].as_str(), d["done"].as_u64()), (Some("hash_mismatch"), Some(0)));
    let small = s.data.join("models").join("org").join("Small-GGUF");
    assert!(!small.join("S-Q4_K_M.gguf").exists() && !small.join("S-Q4_K_M.gguf.part").exists());
    assert_eq!(s.send(reqwest::Method::DELETE, &format!("/models/downloads/{id}"), None).await.0, StatusCode::NO_CONTENT);

    // Request checks.
    for (body, status, code) in [
        (json!({ "repo": "../x", "file": "a.gguf" }), StatusCode::BAD_REQUEST, "bad_repo"),
        (json!({ "repo": "org/Big-GGUF", "file": "../a.gguf" }), StatusCode::BAD_REQUEST, "bad_file"),
        (json!({ "repo": "org/Big-GGUF", "file": "notes.txt" }), StatusCode::BAD_REQUEST, "bad_file"),
        (json!({ "repo": "org/Big-GGUF" }), StatusCode::BAD_REQUEST, "bad_file"),
        (json!({ "repo": "org/Big-GGUF", "file": "a.gguf", "sha256": "xyz", "size": 3 }), StatusCode::BAD_REQUEST, "bad_sha256"),
        (json!({ "repo": "org/Big-GGUF", "file": "Missing-Q4_K_M.gguf" }), StatusCode::NOT_FOUND, "file_missing"),
        (json!({ "repo": "locked/Secret-GGUF", "file": "S-Q4_K_M.gguf" }), StatusCode::NOT_FOUND, "repo_missing"),
        (json!({ "repo": "org/Huge-GGUF", "file": "H.gguf", "size": 1u64 << 60, "sha256": "a".repeat(64) }), StatusCode::INSUFFICIENT_STORAGE, "no_space"),
    ] {
        let (got, v) = s.post("/models/downloads", body.clone()).await;
        assert_eq!((got, v["error"].as_str()), (status, Some(code)), "{body}");
    }
    hf.add("locked/Secret-GGUF", "S-Q4_K_M.gguf", bytes(1, 10));
    let (got, v) = s.post("/models/downloads", json!({ "repo": "locked/Secret-GGUF", "file": "S-Q4_K_M.gguf" })).await;
    assert_eq!((got, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("gated")));
    for path in ["/models/downloads/nope/pause", "/models/downloads/nope/resume"] {
        assert_eq!(s.post(path, json!({})).await.0, StatusCode::NOT_FOUND);
    }
    assert!(s.get("/models/downloads").await["downloads"].as_array().unwrap().is_empty());
    s.running.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// At most two at a time; a cancelled one deletes its part; a download that ran when the app
/// stopped goes on after the next start (with Range).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_cancel_and_restart() {
    let (hf, base) = stand_in().await;
    for i in 0..3u32 {
        hf.add("org/Q-GGUF", &format!("Q{i}-Q4_K_M.gguf"), bytes(20 + i, 400_000));
    }
    hf.delay_us.store(5_000, Ordering::SeqCst);
    let dir = temp("queue");
    let s = server(&dir, &base).await;
    let mut ids = Vec::new();
    for i in 0..3 {
        let (status, job) = s.post("/models/downloads", json!({ "repo": "org/Q-GGUF", "file": format!("Q{i}-Q4_K_M.gguf") })).await;
        assert_eq!(status, StatusCode::CREATED);
        ids.push(job["id"].as_str().unwrap().to_string());
    }
    s.wait(&ids[0], "first running", |d| d["state"] == "running" && d["done"].as_u64().unwrap_or(0) > 0).await;
    let states: Vec<String> = s.get("/models/downloads").await["downloads"].as_array().unwrap().iter().map(|d| d["state"].as_str().unwrap().to_string()).collect();
    assert_eq!(states, ["running", "running", "queued"]);

    // Cancel the second: its part goes, the third starts.
    let folder = s.data.join("models").join("org").join("Q-GGUF");
    assert_eq!(s.send(reqwest::Method::DELETE, &format!("/models/downloads/{}", ids[1]), None).await.0, StatusCode::NO_CONTENT);
    s.wait(&ids[1], "removed", |d| d.is_null()).await;
    assert!(!folder.join("Q1-Q4_K_M.gguf.part").exists());
    s.wait(&ids[2], "third running", |d| d["state"] == "running").await;
    assert_eq!(s.send(reqwest::Method::DELETE, "/models/downloads/nope", None).await.0, StatusCode::NOT_FOUND);

    // Stop the app mid-way.
    s.wait(&ids[0], "some bytes", |d| d["done"].as_u64().unwrap_or(0) > 50_000).await;
    s.running.stop();
    s.running.stop_downloads().await;
    let kept = std::fs::metadata(folder.join("Q0-Q4_K_M.gguf.part")).unwrap().len();
    assert!(kept > 0 && kept < 400_000);
    let saved: Value = serde_json::from_slice(&std::fs::read(s.data.join("downloads.json")).unwrap()).unwrap();
    assert!(saved.as_array().unwrap().iter().all(|j| j["state"] == "running"), "{saved}");

    hf.delay_us.store(0, Ordering::SeqCst);
    let s = server(&dir, &base).await;
    let d = s.wait(&ids[0], "done after restart", |d| d["state"] == "done").await;
    assert_eq!(d["done"], 400_000);
    s.wait(&ids[2], "third done", |d| d["state"] == "done").await;
    assert_eq!(std::fs::read(folder.join("Q0-Q4_K_M.gguf")).unwrap(), bytes(20, 400_000));
    assert!(hf.ranges("Q0-Q4_K_M.gguf").iter().any(|r| r.as_deref() == Some(&format!("bytes={kept}-"))), "resumed with Range");
    s.running.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// The catalog: built-in data first, the live check once per repo and 24 h, live sizes and
/// hashes win, a file gone from the repo is marked, installed files are found.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catalog_live_check() {
    let (hf, base) = stand_in().await;
    let cat = super::catalog::embedded();
    let m = cat.models.iter().find(|m| m.quants.small.is_some()).unwrap();
    let balanced = m.quants.balanced.clone().unwrap();
    let small = m.quants.small.clone().unwrap();
    // The stand-in has the balanced file (tiny) but not the small one; every other repo is 404.
    hf.add(&m.repo, &balanced.file, bytes(5, 1234));
    let dir = temp("catalog");
    let s = server(&dir, &base).await;
    let large = m.quants.large.clone().unwrap();
    let installed = s.data.join("models").join("x").join("y");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join(&large.file), b"GGUF").unwrap();

    let v = s.get("/models/catalog").await;
    assert_eq!(v["models"].as_array().unwrap().len(), cat.models.len());
    let entry = |v: &Value| v["models"].as_array().unwrap().iter().find(|e| e["id"] == m.id.as_str()).unwrap().clone();
    assert_eq!(entry(&v)["quants"]["balanced"]["size_bytes"], balanced.size_bytes, "built-in data before the check");
    assert_eq!(entry(&v)["quants"]["large"]["installed"], true);
    assert_eq!(v["source"].as_str(), Some(base.as_str()));
    let mut tries = 0;
    let v = loop {
        let v = s.get("/models/catalog").await;
        if !v["syncing"].as_bool().unwrap() {
            break v;
        }
        tries += 1;
        assert!(tries < 200, "catalog check did not end");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let e = entry(&v);
    assert_eq!((v["live"].as_u64(), e["live"].as_bool()), (Some(1), Some(true)));
    assert_eq!(e["quants"]["balanced"]["size_bytes"], 1234);
    assert_eq!(e["quants"]["balanced"]["sha256"], sha(&bytes(5, 1234)));
    assert_eq!(e["quants"]["small"]["missing"], true, "{} is gone from the repo", small.file);
    assert_eq!(e["license"], "apache-2.0");
    // The live check never touches the researched fields.
    for key in ["base", "rp", "de", "uncensored", "writing", "confidence", "sources", "params_active_b"] {
        assert_eq!(e[key], serde_json::to_value(m).unwrap()[key], "{key} after the live check");
    }
    assert_eq!(e["q"], (m.quality() * 10.0).round() / 10.0);
    assert_eq!(e["checked"].as_str().unwrap(), super::catalog::date_of(super::now()));
    let other = v["models"].as_array().unwrap().iter().find(|e| e["repo"] != m.repo.as_str()).unwrap();
    assert_eq!(other["live"], false, "no live data: built-in values");
    let mut repos: Vec<&str> = cat.models.iter().map(|m| m.repo.as_str()).collect();
    repos.sort();
    repos.dedup();
    let hits = hf.api_hits.lock().unwrap().len();
    assert_eq!(hits, repos.len(), "one request per repo");

    // Within 24 h: no new requests, also after a restart (the cache is a file).
    s.get("/models/catalog").await;
    s.running.stop();
    let s = server(&dir, &base).await;
    let v = s.get("/models/catalog").await;
    assert_eq!(v["syncing"], false);
    assert_eq!(entry(&v)["quants"]["balanced"]["size_bytes"], 1234);
    assert_eq!(hf.api_hits.lock().unwrap().len(), hits);
    if v["pc"].is_object() {
        let fit = entry(&v)["quants"]["balanced"]["fit"].clone();
        assert!(["fits", "cpu_help", "too_big"].contains(&fit.as_str().unwrap()), "{fit}");
        for col in ["roleplay", "tempo", "context"] {
            assert!(v["best"][col].is_array(), "{col}");
        }
        assert!(v["best"]["balanced"].is_null(), "Ausgewogen is gone");
        let u = s.get("/models/catalog?uncensored=1").await;
        assert_eq!(u["uncensored_only"], true);
        for p in u["best"]["roleplay"].as_array().unwrap() {
            let m = cat.models.iter().find(|m| m.id == p["id"].as_str().unwrap()).unwrap();
            assert!(m.uncensored >= 1, "{}", m.id);
        }
    }
    s.running.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// N10: "installiert" looks into LM Studio's folder the way "Meine Modelle" does: with
/// `RP_LMSTUDIO_DIR` empty (as every test server runs) nothing of the real LM Studio folder counts,
/// and an empty data folder has no catalog file installed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catalog_installed_follows_lmstudio_dir() {
    std::env::set_var("RP_LMSTUDIO_DIR", ""); // the value every test server uses
    let (_hf, base) = stand_in().await;
    let dir = temp("lmstudio");
    let s = server(&dir, &base).await;
    let v = s.get("/models/catalog").await;
    let installed: Vec<String> = v["models"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| ["small", "balanced", "large"].map(|t| (m["quants"][t]["installed"] == true).then(|| m["quants"][t]["file"].to_string())))
        .flatten()
        .collect();
    assert!(installed.is_empty(), "installed without a file in the data folder: {installed:?}");
    s.running.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// Every device hears of a download through the hub: progress at most every 250 ms, a final
/// "done", then "models".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_events() {
    let (hf, base) = stand_in().await;
    hf.add("org/E-GGUF", "E-Q4_K_M.gguf", bytes(11, 600_000));
    hf.delay_us.store(2_000, Ordering::SeqCst);
    let dir = temp("events");
    std::fs::create_dir_all(&dir).unwrap();
    let log = crate::Logger::start(&dir);
    let store = crate::store::Store::open(&dir).unwrap();
    let hub = Arc::new(crate::chat::Hub::new(store, "http://127.0.0.1:1".into(), log));
    let mut rx = hub.subscribe();
    let fetch = super::Fetch::start(hub.clone(), &dir);
    fetch.set_hf_base(&base).unwrap();
    let job = fetch
        .add(super::downloads::AddRequest { repo: "org/E-GGUF".into(), file: Some("E-Q4_K_M.gguf".into()), files: None, sha256: None, size: None })
        .await
        .unwrap();
    let id = job["id"].as_str().unwrap().to_string();
    let mut progress = Vec::new();
    let mut saw_done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let msg = tokio::time::timeout_at(deadline, rx.recv()).await.expect("events in time").unwrap();
        let v: Value = serde_json::from_str(&msg).unwrap();
        match v["type"].as_str() {
            Some("download") if v["id"] == id.as_str() => {
                assert!(v["total"] == 600_000 && v["done"].is_u64() && v["speed"].is_number());
                if v["state"] == "running" {
                    progress.push((std::time::Instant::now(), v["done"].as_u64().unwrap()));
                }
                saw_done |= v["state"] == "done";
            }
            Some("models") => break,
            _ => {}
        }
    }
    assert!(saw_done);
    assert!(progress.len() >= 3, "progress while it runs: {}", progress.len());
    let ticks: Vec<_> = progress.windows(2).filter(|w| w[1].1 > w[0].1).map(|w| w[1].0 - w[0].0).collect();
    assert!(ticks.iter().all(|d| *d >= Duration::from_millis(240)), "at most every 250 ms: {ticks:?}");
    assert_eq!(std::fs::read(dir.join("models").join("org").join("E-GGUF").join("E-Q4_K_M.gguf")).unwrap(), bytes(11, 600_000));
    fetch.shutdown().await;
    drop(fetch);
    let _ = std::fs::remove_dir_all(dir);
}

/// Real Hugging Face, by hand: `cargo test -p rp-server real_download -- --ignored --nocapture`.
/// A 47 MB GGUF and its 1 MB projector from a small public test repo (ggml-org/tinygemma3-GGUF):
/// start, pause mid-way, resume, SHA-256 checked against `lfs.sha256`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "downloads ~48 MB from huggingface.co"]
async fn real_download() {
    let dir = temp("real");
    let s = server(&dir, "https://huggingface.co/").await;
    let started = std::time::Instant::now();
    let (status, res) = s.post("/models/resolve", json!({ "url": "https://huggingface.co/ggml-org/tinygemma3-GGUF" })).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let model = res["groups"]["large"]["recommended"].as_str().expect("Q8_0 is large").to_string();
    let proj = res["mmproj"][0]["path"].as_str().unwrap().to_string();
    let (status, job) = s.post("/models/downloads", json!({ "repo": "ggml-org/tinygemma3-GGUF", "files": [model, proj] })).await;
    assert_eq!(status, StatusCode::CREATED, "{job}");
    let id = job["id"].as_str().unwrap().to_string();
    let d = s.wait(&id, "5 MB", |d| d["done"].as_u64().unwrap_or(0) > 5_000_000 || d["state"] == "done").await;
    assert_eq!(d["state"], "running", "file too small to pause: {d}");
    let (status, _) = s.post(&format!("/models/downloads/{id}/pause"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let paused = s.download(&id).await;
    println!("paused at {} of {} bytes", paused["done"], paused["total"]);
    let (status, _) = s.post(&format!("/models/downloads/{id}/resume"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let d = s.wait(&id, "done", |d| d["state"] == "done" || d["state"] == "failed").await;
    assert_eq!(d["state"], "done", "{d}");
    let folder = std::path::PathBuf::from(d["folder"].as_str().unwrap());
    let file = std::fs::read(folder.join("tinygemma3-Q8_0.gguf")).unwrap();
    assert_eq!(sha(&file), "7566ae7219c93ea2ecc692a931ee122d30c55261d0e2c3347acb8b939d2e9abd");
    println!("done: {} bytes in {:.1} s, SHA-256 matches", d["done"], started.elapsed().as_secs_f64());
    s.running.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// "Was willst du mit dem Modell machen?": the sentence is stored with the needs read from it,
/// corrected needs replace them, and the catalog comes ordered for the purpose with reasons.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn purpose_is_stored_and_orders_the_catalog() {
    std::env::set_var("RP_LMSTUDIO_DIR", "");
    let (_hf, base) = stand_in().await;
    let dir = temp("purpose");
    let s = server(&dir, &base).await;
    let put = |body: Value| s.send(reqwest::Method::PUT, "/models/purpose", Some(body));

    let empty = s.get("/models/purpose").await;
    assert_eq!((empty["text"].as_str(), empty["needs"].as_array().map(Vec::len), empty["all"].as_array().map(Vec::len)), (Some(""), Some(0), Some(12)));

    let (status, v) = put(json!({ "text": "Ich will richtig lange Chats mit 3–4 User-Charakteren" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((&v["needs"], &v["figures"], &v["parsed"]), (&json!(["long_chats", "group"]), &json!(4), &json!(["long_chats", "group"])));
    assert_eq!(v["hits"][0], json!({ "need": "long_chats", "phrase": "lange chats" }));
    assert_eq!(s.get("/models/purpose").await["needs"], json!(["long_chats", "group"]), "kept");

    let cat = s.get("/models/catalog").await;
    assert_eq!(cat["purpose"]["needs"], json!(["long_chats", "group"]));
    // The picks need a PC reading; a test machine without one has none.
    if !cat["pc"].is_null() {
        let ranked = cat["for_purpose"]["ranked"].as_array().unwrap();
        let top = cat["for_purpose"]["top"].as_array().unwrap();
        assert!(!ranked.is_empty() && top.len() == ranked.len().min(5));
        assert!(ranked.windows(2).all(|w| w[0]["score"].as_f64() >= w[1]["score"].as_f64()));
        assert!(ranked.iter().all(|r| cat["models"].as_array().unwrap().iter().any(|m| m["id"] == r["id"])));
        assert!(ranked[0]["reasons"].as_array().is_some_and(|r| !r.is_empty()), "a reason for the first: {}", ranked[0]);
    }

    // Chips corrected by hand: these needs count, in chip order; the sentence stays.
    let (_, v) = put(json!({ "text": "Ich will richtig lange Chats mit 3–4 User-Charakteren", "needs": ["uncensored", "long_chats"] })).await;
    assert_eq!((&v["needs"], &v["parsed"]), (&json!(["long_chats", "uncensored"]), &json!(["long_chats", "group"])));
    let cat = s.get("/models/catalog").await;
    if !cat["pc"].is_null() {
        let refusing: Vec<&str> = cat["models"].as_array().unwrap().iter().filter(|m| m["uncensored"] == 0).filter_map(|m| m["id"].as_str()).collect();
        let ranked = cat["for_purpose"]["ranked"].as_array().unwrap();
        assert!(ranked.iter().all(|r| !refusing.contains(&r["id"].as_str().unwrap())), "uncensored leaves out what refuses");
    }

    let (status, v) = put(json!({ "text": "x".repeat(501) })).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("text_too_long")));
    let (status, _) = put(json!({ "text": "", "needs": ["no_such_need"] })).await;
    assert!(status.is_client_error(), "an unknown need is refused: {status}");
    let (status, v) = put(json!({ "text": "" })).await;
    assert_eq!((status, &v["needs"]), (StatusCode::OK, &json!([])));

    // No profile: nothing to advise.
    let (status, v) = s.send(reqwest::Method::GET, "/models/profiles/999/advice", None).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("profile_missing")));
    let (status, v) = s.post("/models/profiles/999/advice/apply", json!({})).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("profile_missing")));
    s.running.stop();
    let _ = std::fs::remove_dir_all(dir);
}
