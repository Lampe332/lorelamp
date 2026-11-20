//! The card builder's routes through the real server, over HTTP: saving a draft the way an import
//! stores a card, reads it back, the PNG export taken in again unchanged,
//! and a draft without a model.

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::{start, Config, WebUi};

fn draft(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "personality": "dry, patient",
        "scenario": "A storm night at the lighthouse.",
        "first_mes": "*The door creaks.* \"You're late, {{user}}.\"",
        "mes_example": "<START>\n{{user}}: Hi.\n{{char}}: Hm.",
        "system_prompt": "",
        "post_history_instructions": "",
        "creator_notes": "A quiet story by the sea.",
        "alternate_greetings": ["Morning. Coffee is on the stove."],
        "tags": ["Drama", "Coast"],
        "lore": [{ "name": "The lamp", "keys": ["lamp"], "content": "The lamp has burned for ninety years.", "constant": false, "enabled": true }],
        "creator": "Tester",
        "version": "1.0",
    })
}

#[tokio::test]
async fn builder_over_http() {
    let dir = std::env::temp_dir().join(format!("rp-app-builder-{}", std::process::id()));
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
    let post = |path: &str, body: Value| c.post(format!("{base}{path}")).header("authorization", &auth).json(&body).send();
    let get = |path: String| c.get(format!("{base}{path}")).header("authorization", &auth).send();

    // Saved like an import: the library lists it, the card and the lorebook read back the same.
    let d = draft("Ilse Brand", "Ilse keeps the lighthouse. She is 41 and has seen many storms.");
    let res = post("/builder/save", json!({ "draft": d })).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let info: Value = res.json().await.unwrap();
    let id = info["id"].as_i64().unwrap();
    assert_eq!(info["name"].as_str(), Some("Ilse Brand"));
    let lib: Value = get("/library".into()).await.unwrap().json().await.unwrap();
    assert!(lib["characters"].as_array().unwrap().iter().any(|ch| ch["id"] == id));
    let back: Value = get(format!("/builder/from/{id}")).await.unwrap().json().await.unwrap();
    for key in ["name", "description", "first_mes", "mes_example", "alternate_greetings", "tags", "lore", "creator", "version"] {
        assert_eq!(back["draft"][key], d[key], "{key}");
    }

    // The PNG export, taken in again, is the same card (lorebook included).
    let png = get(format!("/characters/{id}/export?format=png")).await.unwrap().bytes().await.unwrap();
    let again: Value = c.post(format!("{base}/characters/import")).header("authorization", &auth).body(png.to_vec()).send().await.unwrap().json().await.unwrap();
    let again_id = again["id"].as_i64().unwrap();
    let copy: Value = get(format!("/builder/from/{again_id}")).await.unwrap().json().await.unwrap();
    assert_eq!(copy["draft"], back["draft"]);

    // No name: refused; an unknown character: 404.
    let bad = post("/builder/save", json!({ "draft": draft("  ", "x") })).await.unwrap();
    assert_eq!((bad.status(), bad.json::<Value>().await.unwrap()["error"].clone()), (StatusCode::BAD_REQUEST, json!("bad_name")));
    assert_eq!(get("/builder/from/9999".into()).await.unwrap().status(), StatusCode::NOT_FOUND);

    // No model server: the draft says so, nothing is written, and the slot is free again.
    let res = post("/builder/draft", json!({ "plan": { "idea": "A lighthouse keeper." }, "draft": d, "field": "scenario" })).await.unwrap();
    assert_eq!((res.status(), res.json::<Value>().await.unwrap()["error"].clone()), (StatusCode::SERVICE_UNAVAILABLE, json!("no_model")));
    let res = post("/builder/draft", json!({ "field": "nonsense" })).await.unwrap();
    assert!(res.status().is_client_error());
    let res = post("/builder/draft", json!({ "field": "name" })).await.unwrap();
    assert_eq!(res.json::<Value>().await.unwrap()["error"], "no_model", "the first draft gave the slot back");

    drop(running);
    let _ = std::fs::remove_dir_all(&dir);
}
