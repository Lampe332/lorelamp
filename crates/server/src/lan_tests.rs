//! "Handy verbinden" through the real server: the switch, the phones' token, the pairing code and
//! the search in the Wi-Fi. In tests the phones' listener binds loopback only (`lan::BIND`), so
//! nothing here is reachable from the network.

use std::path::Path;
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

async fn server(dir: &Path) -> Running {
    std::fs::create_dir_all(dir.join("web")).unwrap();
    start(Config {
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
    .unwrap()
}

fn fresh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rp-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

struct Pc {
    c: reqwest::Client,
    base: String,
    auth: String,
}

impl Pc {
    fn of(r: &Running) -> Pc {
        Pc { c: reqwest::Client::new(), base: format!("http://127.0.0.1:{}/api", r.addr.port()), auth: format!("Bearer {}", r.token) }
    }
    async fn lan(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let url = format!("{}{path}", self.base);
        let req = match method {
            "PUT" => self.c.put(url),
            "POST" => self.c.post(url),
            _ => self.c.get(url),
        };
        let req = req.header("authorization", &self.auth);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let res = req.send().await.unwrap();
        (res.status(), res.json().await.unwrap_or(Value::Null))
    }
}

/// Status of `GET <port>/api/health` with `token` (None: without) and an optional Host.
async fn health(port: u16, token: Option<&str>, host: Option<&str>) -> StatusCode {
    let mut req = reqwest::Client::new().get(format!("http://127.0.0.1:{port}/api/health"));
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    if let Some(h) = host {
        req = req.header("host", h);
    }
    req.send().await.unwrap().status()
}

#[tokio::test]
async fn switch_tokens_and_restart() {
    let dir = fresh("lan-switch");
    let running = server(&dir).await;
    let pc = Pc::of(&running);

    let (st, v) = pc.lan("GET", "/lan", None).await;
    assert_eq!((st, v["enabled"].as_bool(), v["url"].is_null(), v["code"].is_null()), (StatusCode::OK, Some(false), true, true));
    assert!(running.lan_url().is_none());

    let (st, v) = pc.lan("PUT", "/lan", Some(json!({ "enabled": true }))).await;
    assert_eq!((st, v["enabled"].as_bool()), (StatusCode::OK, Some(true)));
    let port = v["port"].as_u64().unwrap() as u16;
    assert_ne!(port, running.addr.port());
    let lan_token = running.lan_ctl.token().to_string();
    assert_ne!(lan_token, running.token);
    if let Some(url) = v["url"].as_str() {
        // The QR code's content (only when the PC has a LAN address at all).
        assert!(url.starts_with("http://") && url.ends_with(&format!(":{port}/?token={lan_token}")), "{url}");
        assert_eq!(running.lan_url().as_deref(), Some(url));
    }

    // Over the phones' listener: their token only, never the desktop window's.
    assert_eq!(health(port, Some(&lan_token), None).await, StatusCode::OK);
    assert_eq!(health(port, None, None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(health(port, Some(&running.token), None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(health(port, Some(&lan_token), Some(&format!("192.168.1.13:{port}"))).await, StatusCode::OK);
    assert_eq!(health(port, Some(&lan_token), Some("evil.example.com")).await, StatusCode::FORBIDDEN);
    // The local listener stays as it was: the window's token, loopback names only.
    let local = running.addr.port();
    assert_eq!(health(local, Some(&running.token), None).await, StatusCode::OK);
    assert_eq!(health(local, Some(&lan_token), None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(health(local, Some(&running.token), Some(&format!("192.168.1.13:{local}"))).await, StatusCode::FORBIDDEN);
    // A phone cannot change the switch.
    let res = reqwest::Client::new().get(format!("http://127.0.0.1:{port}/api/lan")).header("authorization", format!("Bearer {lan_token}")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // An open phone connection (keep-alive) is cut when the switch goes off.
    let mut phone = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let ask = format!("GET /api/health HTTP/1.1
Host: 127.0.0.1:{port}
Authorization: Bearer {lan_token}

");
    phone.write_all(ask.as_bytes()).await.unwrap();
    let mut buf = [0u8; 1024];
    let n = phone.read(&mut buf).await.unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));
    let (st, v) = pc.lan("PUT", "/lan", Some(json!({ "enabled": false }))).await;
    assert_eq!((st, v["enabled"].as_bool(), v["code"].is_null()), (StatusCode::OK, Some(false), true));
    let read = tokio::time::timeout(Duration::from_secs(5), phone.read(&mut buf)).await.expect("the phone connection is still open");
    assert!(read.map_or(true, |n| n == 0), "the phone connection is still open");
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "the phones' port still takes connections");
    assert!(running.lan_url().is_none());

    // On again: same port and token; "Alle Handys trennen" makes a new token, the window goes on.
    let (_, v) = pc.lan("PUT", "/lan", Some(json!({ "enabled": true }))).await;
    assert_eq!(v["port"].as_u64(), Some(u64::from(port)));
    assert_eq!(health(port, Some(&lan_token), None).await, StatusCode::OK);
    let (st, _) = pc.lan("POST", "/lan/disconnect", None).await;
    assert_eq!(st, StatusCode::OK);
    let new_token = running.lan_ctl.token().to_string();
    assert_ne!(new_token, lan_token);
    assert_eq!(health(port, Some(&lan_token), None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(health(port, Some(&new_token), None).await, StatusCode::OK);
    assert_eq!(health(local, Some(&running.token), None).await, StatusCode::OK);

    // A new start follows the stored switch, with the same port and the same (new) token.
    running.lan_ctl.disable().await; // frees the port now (the stored switch stays on)
    running.stop();
    let again = server(&dir).await;
    assert!(again.lan);
    assert_eq!((again.lan_ctl.port(), again.lan_ctl.token().to_string()), (port, new_token.clone()));
    assert_eq!(health(port, Some(&new_token), None).await, StatusCode::OK);
    let pc = Pc::of(&again);
    pc.lan("PUT", "/lan", Some(json!({ "enabled": false }))).await;
    again.stop();
    let off = server(&dir).await;
    assert!(!off.lan && off.lan_url().is_none());
    off.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

async fn pair(port: u16, code: &str, origin: Option<&str>) -> (StatusCode, Value, Option<String>) {
    let mut req = reqwest::Client::new().post(format!("http://127.0.0.1:{port}/api/pair")).json(&json!({ "code": code }));
    if let Some(o) = origin {
        req = req.header("origin", o);
    }
    let res = req.send().await.unwrap();
    let cors = res.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()).map(str::to_string);
    (res.status(), res.json().await.unwrap_or(Value::Null), cors)
}

/// [`pair`] from another loopback address (another device, as far as the server can tell).
async fn pair_from(port: u16, code: &str, from: &str) -> (StatusCode, Value) {
    let c = reqwest::Client::builder().local_address(from.parse::<std::net::IpAddr>().unwrap()).build().unwrap();
    let res = c.post(format!("http://127.0.0.1:{port}/api/pair")).json(&json!({ "code": code })).send().await.unwrap();
    (res.status(), res.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn pairing_code() {
    let dir = fresh("lan-pair");
    let running = server(&dir).await;
    let pc = Pc::of(&running);
    let lan = running.lan_ctl.clone();

    // Off: no code, and pairing says so.
    let (st, v, _) = pair(running.addr.port(), "123456", None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::CONFLICT, Some("lan_off")));

    let (_, v) = pc.lan("PUT", "/lan", Some(json!({ "enabled": true }))).await;
    let port = v["port"].as_u64().unwrap() as u16;
    let code = v["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 6);
    assert!((590..=600).contains(&v["code_expires_in"].as_u64().unwrap()));
    let wrong = if code == "000000" { "000001" } else { "000000" };

    // Wrong, then right (from the phone app's page: CORS for it), then the code is used up.
    let (st, v, _) = pair(port, wrong, None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("wrong_code")));
    let (st, v, cors) = pair(port, &code, Some("http://tauri.localhost")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["token"].as_str(), Some(&*lan.token()));
    assert_eq!(cors.as_deref(), Some("http://tauri.localhost"));
    assert_eq!(health(port, v["token"].as_str(), None).await, StatusCode::OK);
    let (_, v) = pc.lan("GET", "/lan", None).await;
    let next = v["code"].as_str().unwrap().to_string();
    let (st, _, _) = pair(port, &code, None).await;
    assert_eq!(st, if next == code { StatusCode::OK } else { StatusCode::UNAUTHORIZED });
    // Another page may not pair.
    let (st, _, _) = pair(port, &next, Some("http://evil.example.com")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // "Neuer Code" replaces it; a code past its 10 minutes is refused.
    let (_, v) = pc.lan("POST", "/lan/code", None).await;
    let code = v["code"].as_str().unwrap().to_string();
    lan.expire_code();
    let (st, v, _) = pair(port, &code, None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("wrong_code")));

    // M2: five wrong in a row make that address wait (even with the right code); another address,
    // the real phone, pairs with the same code at once.
    lan.unlock(); // forget the two wrong ones above
    let (_, v) = pc.lan("GET", "/lan", None).await;
    let code = v["code"].as_str().unwrap().to_string();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for _ in 0..4 {
        assert_eq!(pair(port, wrong, None).await.0, StatusCode::UNAUTHORIZED);
    }
    let (st, v, _) = pair(port, wrong, None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_many")));
    assert_eq!(pair(port, &code, None).await.0, StatusCode::TOO_MANY_REQUESTS, "the disturber's address waits");
    let (_, v) = pc.lan("GET", "/lan", None).await;
    assert_eq!((v["locked_for"].as_u64(), v["code"].as_str()), (Some(0), Some(code.as_str())), "nobody else waits, the code stays");
    let (st, v) = pair_from(port, &code, "127.0.0.2").await;
    assert_eq!((st, v["token"].as_str()), (StatusCode::OK, Some(&*lan.token())), "the phone at another address pairs");

    // Many addresses taking turns: 20 wrong codes in all replace the code and pause pairing for
    // everyone for 30 s (not longer: then the new code works again).
    let (_, v) = pc.lan("GET", "/lan", None).await;
    let code = v["code"].as_str().unwrap().to_string();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let mut answers = Vec::new();
    for i in 0..20u8 {
        let from = std::net::IpAddr::from([192, 168, 1, 100 + i / 4]);
        answers.push(lan.pair(wrong, from));
    }
    assert_eq!(answers.iter().filter(|a| **a == crate::lan::Paired::Wrong).count(), 19, "five addresses, four each: {answers:?}");
    assert_eq!(answers.last(), Some(&crate::lan::Paired::TooMany));
    let (_, v) = pc.lan("GET", "/lan", None).await;
    assert!(v["locked_for"].as_u64().unwrap() > 25);
    let newer = v["code"].as_str().unwrap().to_string();
    assert_ne!(newer, code, "a new code");
    assert_eq!(pair_from(port, &newer, "127.0.0.3").await.0, StatusCode::TOO_MANY_REQUESTS);
    lan.unlock();
    assert_eq!(pair(port, &newer, None).await.0, StatusCode::OK);

    // No token needed for pairing, but only that route is open.
    assert_eq!(health(port, None, None).await, StatusCode::UNAUTHORIZED);

    // Off again: 409, and the old code is gone.
    pc.lan("PUT", "/lan", Some(json!({ "enabled": false }))).await;
    let (st, v, _) = pair(running.addr.port(), &newer, None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::CONFLICT, Some("lan_off")));
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn search_in_the_wifi() {
    let dir = fresh("lan-search");
    let running = server(&dir).await;
    let pc = Pc::of(&running);
    let (_, v) = pc.lan("PUT", "/lan", Some(json!({ "enabled": true }))).await;
    let http_port = v["port"].as_u64().unwrap();
    assert_eq!(v["search"].as_bool(), Some(true));
    let udp_port = running.lan_ctl.discovery_bound();
    assert_ne!(udp_port, 0);

    let phone = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut buf = [0u8; 512];
    phone.send_to(b"LORELAMP_DISCOVER 1", ("127.0.0.1", udp_port)).await.unwrap();
    let (n, _) = tokio::time::timeout(Duration::from_secs(3), phone.recv_from(&mut buf)).await.expect("no answer").unwrap();
    assert!(n < 256);
    let answer: Value = serde_json::from_slice(&buf[..n]).unwrap();
    assert_eq!((answer["app"].as_str(), answer["v"].as_u64(), answer["port"].as_u64()), (Some("lorelamp"), Some(1), Some(http_port)));
    assert!(!answer["name"].as_str().unwrap().is_empty());

    // Anything else gets no answer.
    for other in [&b"LORELAMP_DISCOVER 2"[..], b"LORELAMP_DISCOVER 1\n", b"hello"] {
        phone.send_to(other, ("127.0.0.1", udp_port)).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(400), phone.recv_from(&mut buf)).await.is_err(), "answered {other:?}");
    }

    // Off: the socket is closed, nothing answers.
    pc.lan("PUT", "/lan", Some(json!({ "enabled": false }))).await;
    let _ = phone.send_to(b"LORELAMP_DISCOVER 1", ("127.0.0.1", udp_port)).await;
    let got = tokio::time::timeout(Duration::from_millis(400), phone.recv_from(&mut buf)).await;
    assert!(!matches!(got, Ok(Ok(_))), "answered while off");
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Zum Startbildschirm": the manifest's start address carries a valid token of the listener asked.
#[tokio::test]
async fn manifest_start_address() {
    let dir = fresh("lan-manifest");
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(dir.join("web").join("manifest.webmanifest"), r#"{"name":"Lorelamp","start_url":"/"}"#).unwrap();
    let running = server(&dir).await;
    let get = |port: u16, token: &str| {
        let url = format!("http://127.0.0.1:{port}/manifest.webmanifest?token={token}");
        async move {
            let res = reqwest::get(url).await.unwrap();
            assert_eq!(res.headers()["content-type"], "application/manifest+json");
            res.json::<Value>().await.unwrap()["start_url"].as_str().unwrap().to_string()
        }
    };
    let local = running.addr.port();
    assert_eq!(get(local, &running.token).await, format!("/?token={}", running.token));
    assert_eq!(get(local, "wrong").await, "/");
    let (_, v) = Pc::of(&running).lan("PUT", "/lan", Some(json!({ "enabled": true }))).await;
    let port = v["port"].as_u64().unwrap() as u16;
    let lan_token = running.lan_ctl.token().to_string();
    assert_eq!(get(port, &lan_token).await, format!("/?token={lan_token}"));
    // The window's token never goes out to a phone.
    assert_eq!(get(port, &running.token).await, "/");
    running.lan_ctl.disable().await;
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// `method path` with `token` on `port`, a raw JSON body (or none). Status and JSON answer.
async fn send(port: u16, token: &str, method: &str, path: &str, body: Option<&str>) -> (StatusCode, Value) {
    let url = format!("http://127.0.0.1:{port}/api{path}");
    let c = reqwest::Client::new();
    let req = match method {
        "POST" => c.post(url),
        "PUT" => c.put(url),
        "DELETE" => c.delete(url),
        _ => c.get(url),
    };
    let req = req.header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(b) => req.header("content-type", "application/json").body(b.to_string()),
        None => req,
    };
    let res = req.send().await.unwrap();
    (res.status(), res.json().await.unwrap_or(Value::Null))
}

/// H3: everything that acts on the PC itself answers a phone 403 `pc_only`, from one route group
/// (`routes::pc_only`); the PC's own window passes. A phone keeps chatting, reading and editing.
#[tokio::test]
async fn phones_cannot_act_on_the_pc() {
    let dir = fresh("lan-pc-only");
    let running = server(&dir).await;
    let pc = Pc::of(&running);
    let (_, v) = pc.lan("PUT", "/lan", Some(json!({ "enabled": true }))).await;
    let port = v["port"].as_u64().unwrap() as u16;
    let phone = running.lan_ctl.token().to_string();
    let local = running.addr.port();
    let not_json = Some("not json");
    let folder_body = json!({ "app_folder": dir.join("made").join("by").join("phone").display().to_string() }).to_string();

    // (group, method, path, body). The body never matters to the phone: the group answers first.
    let pc_only: &[(&str, &str, &str, Option<&str>)] = &[
        ("engine", "PUT", "/engine/idle", Some(r#"{"minutes":30}"#)),
        ("profiles", "POST", "/models/profiles", Some("{}")),
        ("profiles", "DELETE", "/models/profiles/1", None),
        ("profiles", "POST", "/models/profiles/1/activate", None),
        ("profiles", "PUT", "/models/profiles/1", Some(r#"{"load_args":["-c","4096"]}"#)),
        ("profiles", "PUT", "/models/profiles/1", Some(r#"{"model_file":"C:\\x.gguf"}"#)),
        ("profiles", "PUT", "/models/profiles/1", Some(r#"{"engine_id":2}"#)),
        ("benchmark", "POST", "/models/profiles/1/benchmark", Some("{}")),
        ("benchmark", "POST", "/models/profiles/1/benchmark/cancel", None),
        ("benchmark", "POST", "/models/profiles/1/benchmark/apply", Some(r#"{"purpose":"speed"}"#)),
        ("benchmark", "POST", "/models/profiles/1/advice/apply", None),
        ("engines", "POST", "/models/engines/download", Some("{}")),
        ("engines", "DELETE", "/models/engines/1", None),
        ("system", "POST", "/system/scan", None),
        ("settings", "PUT", "/settings", Some(r#"{"engine_port":8090}"#)),
        ("downloads", "POST", "/models/downloads", Some(r#"{"repo":"a/b","file":"x.gguf"}"#)),
        ("downloads", "DELETE", "/models/downloads/x", None),
        ("downloads", "POST", "/models/downloads/x/pause", None),
        ("downloads", "POST", "/models/downloads/x/resume", None),
        ("model files", "DELETE", "/models/local", Some(r#"{"path":"C:\\x.gguf"}"#)),
        ("model files", "POST", "/models/local/use", Some(r#"{"path":"C:\\x.gguf"}"#)),
        ("model files", "POST", "/models/local/reveal", Some(r#"{"path":"C:\\x.gguf"}"#)),
        ("model files", "PUT", "/models/folders", Some(folder_body.as_str())),
        ("model files", "POST", "/models/estimate", Some(r#"{"path":"\\\\host\\share\\x.gguf"}"#)),
        ("windows", "PUT", "/general/autostart", Some(r#"{"on":true}"#)),
        ("windows", "POST", "/general/open-data-folder", None),
        ("sillytavern", "GET", "/import/sillytavern/find", None),
        ("sillytavern", "GET", "/import/sillytavern/contents?path=C:\\st", None),
        ("sillytavern", "GET", "/import/sillytavern/image?path=C:\\st&card=x", None),
        ("sillytavern", "POST", "/import/sillytavern", Some(r#"{"path":"C:\\st","characters":[]}"#)),
        ("backups", "PUT", "/backups/limit", Some(r#"{"limit_bytes":524288000}"#)),
        ("backups", "POST", "/backups/delete", Some(r#"{"older_than_days":0}"#)),
        ("backups", "DELETE", "/backups/rp-app-x.db", None),
        ("backups", "POST", "/backups/rp-app-x.db/restore", None),
        ("phones", "GET", "/lan", None),
        ("phones", "PUT", "/lan", Some(r#"{"enabled":false}"#)),
        ("phones", "POST", "/lan/disconnect", None),
        ("phones", "POST", "/lan/code", None),
    ];
    for (group, method, path, body) in pc_only {
        let (st, v) = send(port, &phone, method, path, *body).await;
        assert_eq!((st, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("pc_only")), "phone: {group}: {method} {path}");
    }
    assert!(!dir.join("made").exists() && running.lan_ctl.is_on(), "nothing happened on the PC");

    // The PC's window passes the group (a body the handler refuses keeps it from doing anything;
    // the data folder is not opened, the PC scan runs for real).
    let at_pc: &[(&str, &str, Option<&str>)] = &[
        ("PUT", "/engine/idle", not_json),
        ("POST", "/models/profiles", not_json),
        ("DELETE", "/models/profiles/999", None),
        ("POST", "/models/profiles/999/activate", None),
        ("PUT", "/models/profiles/999", Some(r#"{"load_args":["-c","4096"]}"#)),
        ("POST", "/models/profiles/999/benchmark/apply", not_json),
        ("POST", "/models/profiles/999/advice/apply", None),
        ("POST", "/models/engines/download", not_json),
        ("DELETE", "/models/engines/999", None),
        ("POST", "/system/scan", None),
        ("PUT", "/settings", Some(r#"{"engine_port":80}"#)),
        ("POST", "/models/downloads", not_json),
        ("DELETE", "/models/downloads/x", None),
        ("DELETE", "/models/local", not_json),
        ("POST", "/models/local/reveal", not_json),
        ("PUT", "/models/folders", not_json),
        ("POST", "/models/estimate", not_json),
        ("PUT", "/general/autostart", not_json),
        ("GET", "/import/sillytavern/contents", None),
        ("POST", "/import/sillytavern", not_json),
        ("PUT", "/backups/limit", not_json),
        ("POST", "/backups/delete", not_json),
        ("DELETE", "/backups/rp-app-x.db", None),
        ("POST", "/backups/rp-app-x.db/restore", None),
        ("GET", "/lan", None),
    ];
    for (method, path, body) in at_pc {
        let (st, v) = send(local, &running.token, method, path, *body).await;
        assert!(st != StatusCode::FORBIDDEN && st != StatusCode::UNAUTHORIZED && v["error"] != "pc_only", "PC: {method} {path} -> {st} {v}");
    }

    // A phone keeps chatting, reading and editing what belongs to the app.
    let (st, v) = send(port, &phone, "POST", "/personas", Some(r#"{"name":"Mira"}"#)).await;
    assert!(st.is_success(), "{st} {v}");
    let allowed: &[(&str, &str, Option<&str>, StatusCode)] = &[
        ("GET", "/library", None, StatusCode::OK),
        ("GET", "/models/profiles", None, StatusCode::OK),
        ("GET", "/models/engines", None, StatusCode::OK),
        ("GET", "/models/local", None, StatusCode::OK),
        ("GET", "/models/downloads", None, StatusCode::OK),
        ("GET", "/system/scan", None, StatusCode::OK),
        ("GET", "/engine", None, StatusCode::OK),
        ("GET", "/backups", None, StatusCode::OK),
        ("POST", "/backups", None, StatusCode::CREATED),
        ("GET", "/general", None, StatusCode::OK),
        ("PUT", "/general", Some(r#"{"send_key":"ctrl_enter"}"#), StatusCode::OK),
        ("PUT", "/settings", Some(r#"{"theme":"papier"}"#), StatusCode::NO_CONTENT),
        ("GET", "/models/purpose", None, StatusCode::OK),
        ("GET", "/personas", None, StatusCode::OK),
        ("GET", "/logs", None, StatusCode::OK),
        // A sampler change is not a start option: no 403 (there is no profile 999).
        ("PUT", "/models/profiles/999", Some(r#"{"sampler":{"temperature":0.7},"name":"x"}"#), StatusCode::NOT_FOUND),
    ];
    for (method, path, body, want) in allowed {
        let (st, v) = send(port, &phone, method, path, *body).await;
        assert_eq!(st, *want, "phone: {method} {path} {v}");
    }
    running.lan_ctl.disable().await;
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
