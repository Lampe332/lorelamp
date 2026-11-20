//! Through the real server: finding SillyTavern installations (in a temp folder via
//! `RP_ST_SEARCH_ROOTS`, never the real drives), their import, the own short text of a
//! character (`tagline`, migration 16) and the layout of the characters tab.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use reqwest::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::sillytavern::tests::{fake_install, temp};
use crate::{start, Config, Running, WebUi};

pub(crate) struct Server {
    running: Running,
    base: String,
    auth: String,
    c: reqwest::Client,
    dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.running.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) async fn server(name: &str) -> Server {
    let dir = std::env::temp_dir().join(format!("rp-app-stsrv-{name}-{}", std::process::id()));
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
    pub(crate) async fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = self.c.request(method, format!("{}{path}", self.base)).header("authorization", &self.auth);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        let text = res.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub(crate) async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(reqwest::Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(reqwest::Method::POST, path, Some(body)).await
    }
}

/// Every file below `dir` with its SHA-256 (to prove ST was only read).
fn fingerprint(dir: &Path) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.display().to_string(), format!("{:x}", Sha256::digest(std::fs::read(&p).unwrap()))));
            }
        }
    }
    out.sort();
    out
}

#[tokio::test]
async fn sillytavern_find_is_cached_until_refresh() {
    let root = temp("find");
    fake_install(&root.join("Spiele").join("SillyTavern"), true);
    std::env::set_var("RP_ST_SEARCH_ROOTS", format!("{};{}", root.join("missing").display(), root.display()));
    let s = server("find").await;

    let (status, v) = s.get("/import/sillytavern/find").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let installs = v["installs"].as_array().unwrap();
    assert_eq!(installs.len(), 1, "{v}");
    assert_eq!(installs[0]["path"], root.join("Spiele").join("SillyTavern").display().to_string());
    assert_eq!((installs[0]["user"].as_str(), installs[0]["characters"].as_u64(), installs[0]["personas"].as_u64(), installs[0]["avatars"].as_u64()), (Some("default-user"), Some(2), Some(2), Some(1)));
    assert!(v["searched_ms"].is_u64() && v["complete"] == true, "{v}");
    assert!(installs[0].get("data").is_none());

    // A new installation shows only after a refresh.
    fake_install(&root.join("alt").join("SillyTavern-old"), false);
    assert_eq!(s.get("/import/sillytavern/find").await.1["installs"].as_array().unwrap().len(), 1);
    let (_, v) = s.get("/import/sillytavern/find?refresh=1").await;
    let users: Vec<Value> = v["installs"].as_array().unwrap().iter().map(|i| i["user"].clone()).collect();
    assert_eq!(users, vec![Value::Null, json!("default-user")], "{v}");
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sillytavern_import_cards_personas_and_duplicates() {
    let root = temp("import");
    let st = root.join("SillyTavern");
    fake_install(&st, true);
    let before = fingerprint(&st);
    let s = server("stimport").await;
    let path = st.display().to_string();

    let all = |extra: Value| {
        let mut body = json!({ "path": path, "characters": ["Ilse.png", "Mara.json"], "personas": ["user-default.png", "1731670000-Wanderin.png"] });
        body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        body
    };
    let (status, v) = s.post("/import/sillytavern", all(json!({ "job": "st-1" }))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["job"].as_str(), v["imported"].as_u64(), v["duplicates"].as_u64(), v["errors"].as_u64()), (Some("st-1"), Some(4), Some(0), Some(0)), "{v}");
    let results = v["results"].as_array().unwrap();
    assert_eq!(results.iter().filter(|r| r["persona_id"].is_i64()).count(), 2);
    assert_eq!(results.iter().filter(|r| r["character_id"].is_i64()).count(), 2);
    assert!(results.iter().any(|r| r["file"] == "characters/Ilse.png"), "{v}");

    let (_, lib) = s.get("/library").await;
    assert_eq!(lib["characters"].as_array().unwrap().len(), 2);
    let (_, personas) = s.get("/personas").await;
    let named = |n: &str| personas["personas"].as_array().unwrap().iter().find(|p| p["name"] == n).cloned().unwrap_or_else(|| panic!("{n} in {personas}"));
    assert_eq!((named("Jonas")["has_avatar"].as_bool(), named("Wanderin")["has_avatar"].as_bool()), (Some(true), Some(false)));

    // The same installation again: all duplicates, with the user given explicitly.
    let (_, v) = s.post("/import/sillytavern", all(json!({ "user": "default-user" }))).await;
    assert_eq!((v["imported"].as_u64(), v["duplicates"].as_u64()), (Some(0), Some(4)), "{v}");
    assert_eq!(v["job"].as_str().map(str::len), Some(12));

    // Errors.
    for (body, code) in [
        (json!({ "path": root.display().to_string(), "characters": [] }), "not_sillytavern"),
        (json!({ "path": "SillyTavern", "characters": [] }), "not_sillytavern"),
        (all(json!({ "user": "someone-else" })), "bad_user"),
        (all(json!({ "job": "no spaces" })), "bad_job"),
        (json!({ "path": path }), "selection_required"),
    ] {
        let (status, v) = s.post("/import/sillytavern", body.clone()).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some(code)), "{body}");
    }

    // ST was only read; the log has counts, no paths.
    assert_eq!(fingerprint(&st), before);
    let mut text = String::new();
    for _ in 0..50 {
        text = s.c.get(format!("{}/logs?limit=500", s.base)).header("authorization", &s.auth).send().await.unwrap().text().await.unwrap();
        if text.contains("sillytavern imported") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(text.contains("sillytavern imported"), "{text}");
    assert!(!text.contains("rp-app-st-import"), "no paths in the log");
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}

const THEO_CHAT: &str = "Theo/Theo - 2026-03-01@18h30m00s.jsonl";
const GRETA_CHAT: &str = "Greta Holm/Greta Holm - 2026-04-02@10h00m00s.jsonl";
const ORPHAN_CHAT: &str = "Unbekannt/Unbekannt - 2026-05-03@08h00m00s.jsonl";

/// An installation with 4 cards (Greta.json, Ilse.png, Mara.json, Theo.json), 2 personas and 3
/// chats: Theo's (folder named like the card file, with swipes), Greta's (folder named like the
/// card's name) and one whose character has no card.
fn st_fixture(dir: &Path) {
    fake_install(dir, true);
    let data = dir.join("data").join("default-user");
    let fx = |f: &str| std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sillytavern").join(f)).unwrap();
    std::fs::write(data.join("characters").join("Theo.json"), fx("theo_lindqvist.json")).unwrap();
    std::fs::write(data.join("characters").join("Greta.json"), fx("greta_holm.json")).unwrap();
    for (id, src) in [(THEO_CHAT, "chat_theo.jsonl"), (GRETA_CHAT, "chat_greta.jsonl"), (ORPHAN_CHAT, "chat_orphan.jsonl")] {
        let file = data.join("chats").join(id);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, fx(src)).unwrap();
    }
    std::fs::write(data.join("chats").join("Theo").join("notes.txt"), b"no chat").unwrap();
}

/// (characters, personas, chats, messages) in the server's database.
fn db_counts(s: &Server) -> [i64; 4] {
    let db = rusqlite::Connection::open(s.dir.join("data").join(crate::store::DB_FILE)).unwrap();
    ["characters", "personas", "chats", "messages"].map(|t| db.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap())
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[tokio::test]
async fn sillytavern_contents_lists_without_importing() {
    let root = temp("contents");
    let st = root.join("SillyTavern");
    st_fixture(&st);
    let before = fingerprint(&st);
    let s = server("stcontents").await;
    let q = format!("path={}&tz=60", enc(&st.display().to_string()));
    let counts = db_counts(&s);

    let (status, v) = s.get(&format!("/import/sillytavern/contents?{q}")).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(db_counts(&s), counts, "nothing imported");
    assert_eq!((v["path"].as_str(), v["user"].as_str()), (Some(st.display().to_string().as_str()), Some("default-user")));
    let cards: Vec<(&str, &str, bool, u64)> = v["characters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c["name"].as_str().unwrap(), c["has_image"].as_bool().unwrap(), c["chats"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        cards,
        vec![("Greta.json", "Greta Holm", false, 1), ("Ilse.png", "Kapitän Ilse Brandt", true, 0), ("Mara.json", "Mara Voss", false, 0), ("Theo.json", "Theo Lindqvist", false, 1)],
        "{v}"
    );
    assert!(v["characters"].as_array().unwrap().iter().all(|c| c["duplicate_of"].is_null() && c["error"].is_null()));
    let personas: Vec<(&str, &str, bool)> =
        v["personas"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap(), p["has_image"].as_bool().unwrap())).collect();
    assert_eq!(personas, vec![("1731670000-Wanderin.png", "Wanderin", false), ("user-default.png", "Jonas", true)], "{v}");
    let chats: Vec<(&str, &str, Value, u64, Value)> = v["chats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c["folder"].as_str().unwrap(), c["card"].clone(), c["messages"].as_u64().unwrap(), c["last_date"].clone()))
        .collect();
    assert_eq!(
        chats,
        vec![
            (GRETA_CHAT, "Greta Holm", json!("Greta.json"), 2, json!(1_775_120_460)),
            (THEO_CHAT, "Theo", json!("Theo.json"), 5, json!(1_772_386_440)),
            (ORPHAN_CHAT, "Unbekannt", Value::Null, 1, json!(1_777_788_000)),
        ],
        "{v}"
    );

    // Pictures: a card's and a persona's as WebP; nothing outside the listed ids.
    let image = |extra: String| {
        let url = format!("{}/import/sillytavern/image?{q}&{extra}", s.base);
        let req = s.c.get(url).header("authorization", &s.auth);
        async move {
            let res = req.send().await.unwrap();
            let kind = res.headers().get("content-type").map(|h| h.to_str().unwrap().to_string());
            (res.status(), kind, res.text().await.unwrap_or_default())
        }
    };
    let (status, kind, _) = image("card=Ilse.png".into()).await;
    assert_eq!((status, kind.as_deref()), (StatusCode::OK, Some("image/webp")));
    let (status, kind, _) = image("persona=user-default.png".into()).await;
    assert_eq!((status, kind.as_deref()), (StatusCode::OK, Some("image/webp")));
    for (extra, status, code) in [
        ("card=Theo.json".to_string(), StatusCode::NOT_FOUND, "no_avatar"),
        ("persona=1731670000-Wanderin.png".into(), StatusCode::NOT_FOUND, "no_avatar"),
        (format!("card={}", enc("../settings.json")), StatusCode::NOT_FOUND, "not_found"),
        (format!("persona={}", enc("../settings.json")), StatusCode::NOT_FOUND, "not_found"),
        (format!("card={}", enc("notes.txt")), StatusCode::NOT_FOUND, "not_found"),
        (String::new(), StatusCode::BAD_REQUEST, "bad_image"),
    ] {
        let (got, _, body) = image(extra.clone()).await;
        assert_eq!((got, body.contains(code)), (status, true), "{extra}: {body}");
    }

    // After an import the listing shows what is already there.
    let (_, r) = s.post("/import/sillytavern", json!({ "path": st.display().to_string(), "characters": ["Theo.json"], "personas": ["user-default.png"], "chats": [THEO_CHAT] })).await;
    assert_eq!(r["imported"], 3, "{r}");
    let (_, v) = s.get(&format!("/import/sillytavern/contents?{q}")).await;
    let theo = v["characters"].as_array().unwrap().iter().find(|c| c["id"] == "Theo.json").unwrap();
    assert!(theo["duplicate_of"].is_i64(), "{v}");
    assert!(v["personas"].as_array().unwrap().iter().find(|p| p["id"] == "user-default.png").unwrap()["duplicate_of"].is_i64());
    assert!(v["chats"].as_array().unwrap().iter().find(|c| c["id"] == THEO_CHAT).unwrap()["duplicate_of"].is_i64());
    assert!(v["chats"].as_array().unwrap().iter().find(|c| c["id"] == GRETA_CHAT).unwrap()["duplicate_of"].is_null());

    assert_eq!(fingerprint(&st), before, "ST was only read");
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sillytavern_import_selection_and_chats() {
    let root = temp("select");
    let st = root.join("SillyTavern");
    st_fixture(&st);
    let before = fingerprint(&st);
    let s = server("stselect").await;
    let path = st.display().to_string();
    let [c0, p0, ch0, m0] = db_counts(&s);

    // 2 of 4 cards, 1 persona, 3 chats: only Theo's chat has its character.
    let (status, v) = s
        .post(
            "/import/sillytavern",
            json!({ "path": path, "tz": 60, "characters": ["Theo.json", "Mara.json"], "personas": ["user-default.png"], "chats": [THEO_CHAT, GRETA_CHAT, ORPHAN_CHAT] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["imported"].as_u64(), v["duplicates"].as_u64(), v["errors"].as_u64()), (Some(4), Some(0), Some(2)), "{v}");
    let results = v["results"].as_array().unwrap();
    let result = |file: &str| results.iter().find(|r| r["file"] == file).cloned().unwrap_or_else(|| panic!("{file} in {v}"));
    for orphan in [GRETA_CHAT, ORPHAN_CHAT] {
        let r = result(&format!("chats/{orphan}"));
        assert_eq!((r["status"].as_str(), r["code"].as_str()), (Some("error"), Some("no_character")), "{r}");
    }
    let theo = result(&format!("chats/{THEO_CHAT}"));
    assert_eq!(theo["status"], "imported");
    assert_eq!(theo["character_id"], result("characters/Theo.json")["character_id"]);
    let chat_id = theo["chat_id"].as_i64().unwrap();
    assert_eq!(db_counts(&s), [c0 + 2, p0 + 1, ch0 + 1, m0 + 5]);
    let (_, lib) = s.get("/library").await;
    let mut names: Vec<&str> = lib["characters"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["Mara Voss", "Theo Lindqvist"]);

    // The round trip: every message in order, with its time, all variants and the chosen one.
    let db = rusqlite::Connection::open(s.dir.join("data").join(crate::store::DB_FILE)).unwrap();
    type Row = (String, String, i64, Option<String>, Option<i64>);
    let rows: Vec<Row> = db
        .prepare("SELECT role, content, created_at, swipes, swipe_id FROM messages WHERE chat_id = ?1 ORDER BY id")
        .unwrap()
        .query_map([chat_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let swipes = |l: &[&str]| Some(serde_json::to_string(l).unwrap());
    assert_eq!(
        rows,
        vec![
            ("assistant".into(), "Willkommen an Bord.".into(), 1_772_386_200, swipes(&["Willkommen an Bord.", "Leinen los!"]), Some(0)),
            ("user".into(), "Wohin fahren wir heute?".into(), 1_772_386_260, None, None),
            (
                "assistant".into(),
                "Rüber nach Amrum, wenn der Wind mitspielt.".into(),
                1_772_386_320,
                swipes(&["Nach Föhr.", "Rüber nach Amrum, wenn der Wind mitspielt.", "Erst mal Kaffee."]),
                Some(1)
            ),
            ("user".into(), "Klingt gut.".into(), 1_772_386_380, None, None),
            ("assistant".into(), "Dann halt dich fest.".into(), 1_772_386_440, None, None),
        ]
    );
    let (title, created): (String, i64) = db.query_row("SELECT title, created_at FROM chats WHERE id = ?1", [chat_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((title.as_str(), created), ("Theo - 2026-03-01@18h30m00s", 1_772_386_200));
    // The app's own SillyTavern export gives the same times back.
    let text = s.c.get(format!("{}/chats/{chat_id}/export?format=jsonl", s.base)).header("authorization", &s.auth).send().await.unwrap().text().await.unwrap();
    let dates: Vec<String> = text.lines().skip(1).map(|l| serde_json::from_str::<Value>(l).unwrap()["send_date"].as_str().unwrap().to_string()).collect();
    assert_eq!(dates, ["17:30", "17:31", "17:32", "17:33", "17:34"].map(|t| format!("2026-03-01T{t}:00.000Z")));

    // The same chat again: nothing new.
    let (_, v) = s.post("/import/sillytavern", json!({ "path": path, "tz": 60, "chats": [THEO_CHAT] })).await;
    assert_eq!((v["imported"].as_u64(), v["duplicates"].as_u64()), (Some(0), Some(1)), "{v}");
    assert_eq!(v["results"][0]["chat_id"], chat_id);
    assert_eq!(db_counts(&s), [c0 + 2, p0 + 1, ch0 + 1, m0 + 5]);

    // Greta's card on its own, then her chat finds the character already in the app (by name of
    // the folder, by content of the card).
    let (_, v) = s.post("/import/sillytavern", json!({ "path": path, "characters": ["Greta.json"] })).await;
    assert_eq!(v["imported"], 1, "{v}");
    let (_, v) = s.post("/import/sillytavern", json!({ "path": path, "chats": [GRETA_CHAT, ORPHAN_CHAT] })).await;
    assert_eq!((v["imported"].as_u64(), v["errors"].as_u64()), (Some(1), Some(1)), "{v}");
    assert_eq!(v["results"][1]["code"], "no_character");
    assert_eq!(db_counts(&s), [c0 + 3, p0 + 1, ch0 + 2, m0 + 7]);

    // Ids that are not in the listing (also ones that try to leave the folder).
    let (status, v) = s
        .post(
            "/import/sillytavern",
            json!({ "path": path, "characters": ["nope.png", "../settings.json", "notes.txt"], "personas": ["../../secret.png"], "chats": ["../../settings.json", "Theo/notes.txt"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["errors"].as_u64(), v["imported"].as_u64()), (Some(6), Some(0)), "{v}");
    assert!(v["results"].as_array().unwrap().iter().all(|r| r["code"] == "not_found"), "{v}");

    // Paths outside an installation.
    let outside = [
        json!({ "path": root.display().to_string(), "chats": [THEO_CHAT] }),
        json!({ "path": st.join("data").join("default-user").display().to_string(), "chats": [THEO_CHAT] }),
        json!({ "path": st.join("data").join("..").display().to_string(), "chats": [THEO_CHAT] }),
        json!({ "path": root.join("..").join(root.file_name().unwrap()).join("SillyTavern").display().to_string(), "chats": [THEO_CHAT] }),
        json!({ "path": "SillyTavern", "chats": [THEO_CHAT] }),
    ];
    for body in outside {
        let (status, v) = s.post("/import/sillytavern", body.clone()).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("not_sillytavern")), "{body}");
        let q = format!("/import/sillytavern/contents?path={}", enc(body["path"].as_str().unwrap()));
        assert_eq!(s.get(&q).await.1["error"], "not_sillytavern", "{body}");
    }
    for user in ["../..", "default-user/../..", "someone"] {
        let (status, v) = s.post("/import/sillytavern", json!({ "path": path, "user": user, "chats": [THEO_CHAT] })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_user")), "{user}");
    }
    assert_eq!(db_counts(&s), [c0 + 3, p0 + 1, ch0 + 2, m0 + 7]);
    assert_eq!(fingerprint(&st), before, "ST was only read");
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}

/// Lines SillyTavern marks `is_system` (hidden from its prompt, no part of the story) are not
/// taken over; the chat's result says how many were left out.
#[tokio::test]
async fn sillytavern_import_leaves_out_system_lines() {
    let root = temp("system");
    let st = root.join("SillyTavern");
    st_fixture(&st);
    let id = "Theo/Theo - versteckt.jsonl";
    let lines = [
        json!({ "user_name": "Jonas", "character_name": "Theo Lindqvist", "create_date": "2026-03-01@18h30m00s", "chat_metadata": {} }),
        json!({ "name": "Theo Lindqvist", "is_user": false, "is_system": false, "send_date": 1_772_386_200_000_i64, "mes": "Moin." }),
        json!({ "name": "Jonas", "is_user": true, "is_system": false, "send_date": 1_772_386_260_000_i64, "mes": "Hallo." }),
        json!({ "name": "System", "is_user": false, "is_system": true, "send_date": 1_772_386_270_000_i64, "mes": "Ausgeblendete Notiz." }),
        json!({ "name": "Theo Lindqvist", "is_user": false, "send_date": 1_772_386_320_000_i64, "mes": "Na?" }),
    ];
    let text = lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
    std::fs::write(st.join("data").join("default-user").join("chats").join(id), text).unwrap();
    let s = server("stsystem").await;
    let path = st.display().to_string();

    let (_, v) = s.get(&format!("/import/sillytavern/contents?path={}", enc(&path))).await;
    let listed = v["chats"].as_array().unwrap().iter().find(|c| c["id"] == id).cloned().unwrap();
    assert_eq!(listed["messages"], 3, "{listed}");

    let (status, v) = s.post("/import/sillytavern", json!({ "path": path, "characters": ["Theo.json"], "chats": [id] })).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let chat = v["results"].as_array().unwrap().iter().find(|r| r["file"] == format!("chats/{id}")).cloned().unwrap();
    assert_eq!((chat["status"].as_str(), chat["system_skipped"].as_u64()), (Some("imported"), Some(1)), "{chat}");
    let card = v["results"].as_array().unwrap().iter().find(|r| r["file"] == "characters/Theo.json").cloned().unwrap();
    assert!(card.get("system_skipped").is_none(), "{card}");
    let db = rusqlite::Connection::open(s.dir.join("data").join(crate::store::DB_FILE)).unwrap();
    let rows: Vec<(String, String)> = db
        .prepare("SELECT role, content FROM messages WHERE chat_id = ?1 ORDER BY id")
        .unwrap()
        .query_map([chat["chat_id"].as_i64().unwrap()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let want = [("assistant", "Moin."), ("user", "Hallo."), ("assistant", "Na?")].map(|(r, c)| (r.to_string(), c.to_string()));
    assert_eq!(rows, want);
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}

/// A network path (`\\host\share`, `\\?\UNC\…`) is never opened as an installation, also when it
/// leads to one (here through the drive's admin share on this PC).
#[cfg(windows)]
#[tokio::test]
async fn sillytavern_refuses_network_paths() {
    let root = temp("unc");
    let st = root.join("SillyTavern");
    st_fixture(&st);
    let s = server("stunc").await;
    let local = st.display().to_string();
    let (drive, rest) = local.split_once(":\\").unwrap();
    let counts = db_counts(&s);
    for path in [
        format!("\\\\localhost\\{drive}$\\{rest}"),
        format!("\\\\?\\UNC\\localhost\\{drive}$\\{rest}"),
        format!("//localhost/{drive}$/{}", rest.replace('\\', "/")),
        format!("  \\\\127.0.0.1\\{drive}$\\{rest}"),
    ] {
        let (status, v) = s.post("/import/sillytavern", json!({ "path": path, "characters": ["Theo.json"] })).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("not_sillytavern")), "{path}: {v}");
        let (status, v) = s.get(&format!("/import/sillytavern/contents?path={}", enc(&path))).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("not_sillytavern")), "{path}: {v}");
    }
    assert_eq!(db_counts(&s), counts, "nothing imported");
    // The same installation by its local path.
    assert_eq!(s.get(&format!("/import/sillytavern/contents?path={}", enc(&local))).await.0, StatusCode::OK);
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}

/// The old layout (`public/`) without `settings.json`: cards only.
#[tokio::test]
async fn sillytavern_import_old_layout() {
    let root = temp("old");
    let st = root.join("SillyTavern-1.10");
    fake_install(&st, false);
    let s = server("stold").await;
    let (status, v) = s.post("/import/sillytavern", json!({ "path": st.display().to_string(), "characters": ["Ilse.png", "Mara.json"] })).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["imported"].as_u64(), v["errors"].as_u64()), (Some(2), Some(0)), "{v}");
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn tagline_and_library_layout() {
    let s = server("tagline").await;
    let card = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/mara_voss.json")).unwrap();
    let res = s.c.post(format!("{}/characters/import", s.base)).header("authorization", &s.auth).body(card).send().await.unwrap();
    let id = res.json::<Value>().await.unwrap()["id"].as_i64().unwrap();
    let put = |body: Value| async { s.send(reqwest::Method::PUT, &format!("/characters/{id}"), Some(body)).await };
    let (_, c) = s.get(&format!("/characters/{id}")).await;
    let mut card = c["card"].clone();
    assert_eq!(c["info"]["tagline"], Value::Null);
    let automatic = c["info"]["blurb"].clone();

    // Own text (trimmed), kept when left out, back to automatic with "" or null.
    card["tagline"] = json!("  Hütet den Leuchtturm  ");
    let (status, v) = put(card.clone()).await;
    assert_eq!((status, v["info"]["tagline"].as_str()), (StatusCode::OK, Some("Hütet den Leuchtturm")), "{v}");
    assert_eq!(v["info"]["blurb"], automatic, "the automatic text stays available");
    card.as_object_mut().unwrap().remove("tagline");
    assert_eq!(put(card.clone()).await.1["info"]["tagline"], "Hütet den Leuchtturm");
    let (_, lib) = s.get("/library").await;
    let entry = &lib["characters"][0];
    assert_eq!(entry["tagline"], "Hütet den Leuchtturm");
    assert!(entry["tags"].is_array() && entry["own_tags"].is_array(), "card tags and own tags: {entry}");
    assert_eq!(entry["tags"], c["card"]["tags"]);

    card["tagline"] = json!("ä".repeat(160));
    assert_eq!(put(card.clone()).await.0, StatusCode::OK);
    for bad in [json!("x".repeat(161)), json!(5)] {
        card["tagline"] = bad;
        let (status, v) = put(card.clone()).await;
        assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_tagline")));
    }
    card["tagline"] = json!("   ");
    assert_eq!(put(card.clone()).await.1["info"]["tagline"], Value::Null);
    card["tagline"] = json!("wieder");
    put(card.clone()).await;
    card["tagline"] = Value::Null;
    assert_eq!(put(card.clone()).await.1["info"]["tagline"], Value::Null);
    card["tagline"] = json!("x");
    assert_eq!(s.send(reqwest::Method::PUT, "/characters/999", Some(card.clone())).await.0, StatusCode::NOT_FOUND);

    // Layout of the characters tab.
    assert_eq!(s.get("/library").await.1["library_layout"], "poster");
    for layout in ["compact", "list", "poster"] {
        let (status, _) = s.send(reqwest::Method::PUT, "/settings", Some(json!({ "library_layout": layout }))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(s.get("/library").await.1["library_layout"], layout);
    }
    let (status, v) = s.send(reqwest::Method::PUT, "/settings", Some(json!({ "library_layout": "grid" }))).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_layout")));
}
