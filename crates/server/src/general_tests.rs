//! The general settings (`routes/general.rs`) through the real server: defaults, changes, refused
//! values, and what a phone in the Wi-Fi may not see or do (the data folder, starting with
//! Windows). The phones' listener binds loopback only in tests.

use std::path::PathBuf;

use axum::http::StatusCode;
use serde_json::{json, Value};

use super::*;

fn fresh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rp-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn call(method: &str, port: u16, token: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let c = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/api{path}");
    let req = match method {
        "PUT" => c.put(url),
        "POST" => c.post(url),
        _ => c.get(url),
    }
    .header("authorization", format!("Bearer {token}"));
    let res = match body {
        Some(b) => req.json(&b),
        None => req,
    }
    .send()
    .await
    .unwrap();
    (res.status(), res.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn general_settings_over_http() {
    let dir = fresh("general");
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
    let (port, token) = (running.addr.port(), running.token.clone());

    // Defaults; the PC itself sees its data folder. This test process is no desktop app.
    let (st, v) = call("GET", port, &token, "/general", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        (v["send_key"].as_str(), v["start_page"].as_str(), v["confirm_delete"].as_bool(), v["confirm_rewind"].as_bool(), v["undo_secs"].as_u64(), v["backup_rhythm"].as_str()),
        (Some("enter"), Some("home"), Some(false), Some(false), Some(8), Some("daily"))
    );
    assert_eq!(v["data_dir"].as_str(), Some(dir.join("data").display().to_string().as_str()));
    assert!(v["autostart"].is_null());
    assert!(v["backup_next_ms"].is_i64());

    // A change answers the new state and is kept.
    let (st, v) = call("PUT", port, &token, "/general", Some(json!({ "send_key": "ctrl_enter", "confirm_delete": true, "undo_secs": 30, "backup_rhythm": "off" }))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!((v["send_key"].as_str(), v["confirm_delete"].as_bool(), v["undo_secs"].as_u64(), v["backup_rhythm"].as_str()), (Some("ctrl_enter"), Some(true), Some(30), Some("off")));
    assert!(v["backup_next_ms"].is_null(), "off: no automatic backup is planned");
    assert_eq!(v["start_page"].as_str(), Some("home"), "fields left out stay");

    // One bad value refuses the whole change.
    let (st, v) = call("PUT", port, &token, "/general", Some(json!({ "start_page": "characters", "backup_rhythm": "hourly" }))).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_rhythm")));
    let (st, v) = call("PUT", port, &token, "/general", Some(json!({}))).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_settings")));
    let (_, v) = call("GET", port, &token, "/general", None).await;
    assert_eq!((v["start_page"].as_str(), v["backup_rhythm"].as_str()), (Some("home"), Some("off")));

    // No desktop app: starting with Windows is not offered.
    let (st, v) = call("PUT", port, &token, "/general/autostart", Some(json!({ "on": true }))).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::NOT_IMPLEMENTED, Some("unsupported")));

    // A phone: the settings themselves, but neither the folder's path nor anything that acts on the PC.
    let (st, v) = call("PUT", port, &token, "/lan", Some(json!({ "enabled": true }))).await;
    assert_eq!(st, StatusCode::OK);
    let phone_port = v["port"].as_u64().unwrap() as u16;
    let phone_token = running.lan_ctl.token().to_string();
    let (st, v) = call("GET", phone_port, &phone_token, "/general", None).await;
    assert_eq!((st, v["send_key"].as_str()), (StatusCode::OK, Some("ctrl_enter")));
    assert!(v["data_dir"].is_null() && v["autostart"].is_null());
    let (st, v) = call("PUT", phone_port, &phone_token, "/general", Some(json!({ "send_key": "enter" }))).await;
    assert_eq!((st, v["send_key"].as_str(), v["data_dir"].is_null()), (StatusCode::OK, Some("enter"), true));
    let (st, v) = call("POST", phone_port, &phone_token, "/general/open-data-folder", None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("pc_only")));
    let (st, v) = call("PUT", phone_port, &phone_token, "/general/autostart", Some(json!({ "on": true }))).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("pc_only")));

    drop(running);
    let _ = std::fs::remove_dir_all(dir);
}
