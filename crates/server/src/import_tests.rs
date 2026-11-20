//! Import and export through the real server, over HTTP (multipart uploads, zips, round trips).

use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use image::{DynamicImage, ImageFormat, RgbImage};
use reqwest::multipart::{Form, Part};
use rp_core::formats::{self, Content};
use serde_json::{json, Value};

use crate::{start, Config, Running, WebUi};

const MARA_JSON: &[u8] = include_bytes!("../../../tests/fixtures/mara_voss.json");
const ILSE_PNG: &[u8] = include_bytes!("../../../tests/fixtures/ilse_brandt.png");
const CHARX: &[u8] = include_bytes!("../../../tests/fixtures/formats/moos.charx");
const RISU_PERSONA: &[u8] = include_bytes!("../../../tests/fixtures/formats/persona_risu.png");
const ST_BACKUP: &[u8] = include_bytes!("../../../tests/fixtures/formats/personas_20261115.json");

struct Server {
    running: Running,
    base: String,
    auth: String,
    c: reqwest::Client,
    dir: std::path::PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.running.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn server(name: &str) -> Server {
    let dir = std::env::temp_dir().join(format!("rp-app-io-{name}-{}", std::process::id()));
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
    async fn get(&self, path: &str) -> reqwest::Response {
        self.c.get(format!("{}{path}", self.base)).header("authorization", &self.auth).send().await.unwrap()
    }

    async fn post_json(&self, path: &str, body: Value) -> reqwest::Response {
        self.c.post(format!("{}{path}", self.base)).header("authorization", &self.auth).json(&body).send().await.unwrap()
    }

    async fn upload(&self, path: &str, files: Vec<(String, Vec<u8>)>) -> Value {
        let mut form = Form::new();
        for (name, bytes) in files {
            form = form.part("files", Part::bytes(bytes).file_name(name));
        }
        let res = self.c.post(format!("{}{path}", self.base)).header("authorization", &self.auth).multipart(form).send().await.unwrap();
        assert!(res.status().is_success(), "{path}: {}", res.status());
        res.json().await.unwrap()
    }

    async fn bytes(&self, path: &str) -> (String, Vec<u8>) {
        let res = self.get(path).await;
        assert!(res.status().is_success(), "{path}: {}", res.status());
        let ct = res.headers()["content-type"].to_str().unwrap().to_string();
        (ct, res.bytes().await.unwrap().to_vec())
    }
}

fn picture(w: u32, h: u32, format: ImageFormat) -> Vec<u8> {
    let img = RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, ((x ^ y) % 256) as u8]));
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(img).write_to(&mut out, format).unwrap();
    out.into_inner()
}

/// The fixture card with its own name and description (so every one is a new card).
fn sfw_card(i: usize) -> Value {
    let mut raw: Value = serde_json::from_slice(MARA_JSON).unwrap();
    raw["data"]["name"] = json!(format!("Testfigur {i}"));
    raw["data"]["description"] = json!(format!("{} Nummer {i}.", raw["data"]["description"].as_str().unwrap()));
    raw
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut f = zip.by_index(i).unwrap();
            let mut buf = vec![];
            f.read_to_end(&mut buf).unwrap();
            (f.name().to_string(), buf)
        })
        .collect()
}

/// Keywords of the tEXt chunks of a PNG.
fn text_keys(png: &[u8]) -> Vec<String> {
    let mut keys = vec![];
    let mut at = 8;
    while at + 8 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        if &png[at + 4..at + 8] == b"tEXt" {
            let data = &png[at + 8..at + 8 + len];
            keys.push(String::from_utf8_lossy(&data[..data.iter().position(|&b| b == 0).unwrap()]).to_string());
        }
        at += 12 + len;
    }
    keys
}

fn statuses(v: &Value) -> Vec<(String, String)> {
    v["results"].as_array().unwrap().iter().map(|r| (r["file"].as_str().unwrap().into(), r["status"].as_str().unwrap().into())).collect()
}

fn the_card(bytes: &[u8]) -> rp_core::card::Card {
    match formats::detect_and_parse(bytes).unwrap().content {
        Content::Cards(mut c) => c.remove(0).card,
        Content::Persona { .. } => panic!("not a card"),
    }
}

/// 100 generated SFW cards with a 1024x1536 picture each (plus a duplicate, a broken file, a zip
/// of three cards and a RisuAI persona) in one upload: under 10 s, thumbnails, duplicates.
#[tokio::test(flavor = "multi_thread")]
async fn hundred_cards_in_one_upload() {
    let s = server("bulk").await;
    let art = picture(1024, 1536, ImageFormat::Png);
    let mut files: Vec<(String, Vec<u8>)> =
        (0..100).map(|i| (format!("karte_{i:03}.png"), formats::write_png(&sfw_card(i), Some(&art)).unwrap())).collect();
    files.push(("doppelt.json".into(), formats::write_json_v3(&sfw_card(7))));
    files.push(("kaputt.png".into(), b"\x89PNG\r\n\x1a\nnot really".to_vec()));
    let zip_cards: Vec<Vec<u8>> = (100..103).map(|i| formats::write_json_v2(&sfw_card(i))).collect();
    let zip = zip_of(&[("a.json", &zip_cards[0]), ("ordner/b.json", &zip_cards[1]), ("c.json", &zip_cards[2]), ("__MACOSX/._a.json", b"x")]);
    files.push(("paket.zip".into(), zip.clone()));
    files.push(("persona.png".into(), RISU_PERSONA.to_vec()));

    let t = Instant::now();
    let v = s.upload("/import?job=test-1", files).await;
    let secs = t.elapsed().as_secs_f64();
    println!("100 cards (+5 files) imported in {secs:.2} s");
    assert!(secs < 10.0, "took {secs:.2} s");

    assert_eq!(v["job"], "test-1");
    assert_eq!((v["imported"].as_u64(), v["duplicates"].as_u64(), v["errors"].as_u64()), (Some(104), Some(1), Some(1)));
    let st = statuses(&v);
    assert_eq!(st[0], ("karte_000.png".into(), "imported".into()));
    assert_eq!(st[100], ("doppelt.json".into(), "duplicate".into()));
    assert_eq!(v["results"][100]["character_id"], v["results"][7]["character_id"]);
    assert_eq!((v["results"][101]["status"].as_str(), v["results"][101]["code"].as_str()), (Some("error"), Some("no_card")));
    assert_eq!(&st[102..105], &[
        ("paket.zip/a.json".into(), "imported".into()),
        ("paket.zip/ordner/b.json".into(), "imported".into()),
        ("paket.zip/c.json".into(), "imported".into()),
    ]);
    assert_eq!(st[105], ("persona.png".into(), "imported".into()));
    assert!(v["results"][105]["persona_id"].as_i64().is_some());

    let lib: Value = s.get("/library").await.json().await.unwrap();
    assert_eq!(lib["characters"].as_array().unwrap().len(), 103);

    // Thumbnail: WebP, 384 wide; a card without picture falls back to 404.
    let id = v["results"][0]["character_id"].as_i64().unwrap();
    let (ct, thumb) = s.bytes(&format!("/characters/{id}/thumb")).await;
    assert_eq!(ct, "image/webp");
    assert_eq!(image::load_from_memory(&thumb).unwrap().width(), 384);
    let json_id = v["results"][102]["character_id"].as_i64().unwrap();
    assert_eq!(s.get(&format!("/characters/{json_id}/thumb")).await.status(), 404);

    // The same zip again: all duplicates, nothing new.
    let again = s.upload("/import", vec![("paket.zip".into(), zip)]).await;
    assert_eq!((again["imported"].as_u64(), again["duplicates"].as_u64()), (Some(0), Some(3)));
    assert_eq!(again["job"].as_str().map(str::len), Some(12));
}

/// Progress events go to every device while a job runs; the last one says `finished`.
#[tokio::test(flavor = "multi_thread")]
async fn import_job_reports_progress() {
    let dir = std::env::temp_dir().join(format!("rp-app-progress-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = crate::store::Store::open(&dir).unwrap();
    let hub = Arc::new(crate::chat::Hub::new(store, "http://127.0.0.1:1".into(), crate::Logger::start(&dir)));
    let mut rx = hub.subscribe();
    let mut job = crate::import::Job::start(hub.clone(), "j1".into()).await;
    for i in 0..3 {
        let path = dir.join(format!("{i}.json"));
        std::fs::write(&path, formats::write_json_v2(&sfw_card(i))).unwrap();
        job.add_file((i, 0), format!("{i}.json"), path);
    }
    let results = job.finish().await.unwrap();
    assert_eq!(results.len(), 3);
    let mut last = Value::Null;
    while let Ok(ev) = rx.try_recv() {
        let ev: Value = serde_json::from_str(&ev).unwrap();
        if ev["type"] == "import" {
            assert_eq!(ev["job"], "j1");
            last = ev;
        }
    }
    assert_eq!(last, json!({ "type": "import", "job": "j1", "done": 3, "total": 3, "finished": true }));
    drop(hub);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Exported cards (PNG with both chunks, JSON V2/V3, zip of several) read back as the same card
/// and are recognised as duplicates; a JPEG picture becomes the PNG of the export; the chat's
/// version of the card is exported on request.
#[tokio::test(flavor = "multi_thread")]
async fn card_export_round_trip() {
    let s = server("export").await;
    let v = s.upload("/import", vec![("ilse.png".into(), ILSE_PNG.to_vec()), ("moos.charx".into(), CHARX.to_vec())]).await;
    assert_eq!(v["imported"], 2, "{v}");
    let ilse = v["results"][0]["character_id"].as_i64().unwrap();
    let moos = v["results"][1]["character_id"].as_i64().unwrap();
    let original = the_card(ILSE_PNG);

    let (ct, png) = s.bytes(&format!("/characters/{ilse}/export?format=png")).await;
    assert_eq!(ct, "image/png");
    let chunks = text_keys(&png);
    assert!(chunks.contains(&"chara".to_string()) && chunks.contains(&"ccv3".to_string()), "{chunks:?}");
    assert_eq!(the_card(&png), original);
    let (_, v2) = s.bytes(&format!("/characters/{ilse}/export?format=json_v2")).await;
    let (_, v3) = s.bytes(&format!("/characters/{ilse}/export?format=json_v3")).await;
    assert_eq!(serde_json::from_slice::<Value>(&v2).unwrap()["spec"], "chara_card_v2");
    assert_eq!(serde_json::from_slice::<Value>(&v3).unwrap()["spec"], "chara_card_v3");
    assert_eq!((the_card(&v2), the_card(&v3)), (original.clone(), original.clone()));
    let back = s.upload("/import", vec![("a.png".into(), png), ("b.json".into(), v2), ("c.json".into(), v3)]).await;
    assert_eq!(back["duplicates"], 3);
    assert!(back["results"].as_array().unwrap().iter().all(|r| r["character_id"] == ilse));
    assert_eq!(s.get(&format!("/characters/{ilse}/export?format=gif")).await.status(), 400);
    assert_eq!(s.get("/characters/999/export").await.status(), 404);

    // A JPEG picture: the exported PNG carries it (converted) and the card.
    let jpeg = picture(300, 450, ImageFormat::Jpeg);
    let res = s.c.put(format!("{}/characters/{moos}/avatar", s.base)).header("authorization", &s.auth).body(jpeg).send().await.unwrap();
    assert_eq!(res.status(), 204);
    let (_, png) = s.bytes(&format!("/characters/{moos}/export")).await;
    let img = image::load_from_memory_with_format(&png, ImageFormat::Png).unwrap();
    assert_eq!((img.width(), img.height()), (300, 450));
    assert_eq!(the_card(&png).name, the_card(CHARX).name);

    // The chat's version of the card.
    let chat: Value = s.post_json("/chats", json!({ "character_id": ilse })).await.json().await.unwrap();
    let chat_id = chat["chat_id"].as_i64().unwrap();
    let mut edited = original.clone();
    edited.description = "Nur in diesem Chat: Ilse ist Bäckerin.".into();
    let res = s.c.put(format!("{}/chats/{chat_id}/card", s.base)).header("authorization", &s.auth).json(&edited).send().await.unwrap();
    assert_eq!(res.status(), 204);
    let (_, json) = s.bytes(&format!("/characters/{ilse}/export?format=json_v3&chat={chat_id}")).await;
    assert_eq!(the_card(&json).description, edited.description);
    assert_eq!(s.get(&format!("/characters/{moos}/export?chat={chat_id}")).await.status(), 404);

    // Several at once: one zip.
    let res = s.post_json("/characters/export", json!({ "ids": [ilse, moos, ilse], "format": "png" })).await;
    assert_eq!(res.headers()["content-type"], "application/zip");
    let files = unzip(&res.bytes().await.unwrap());
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].0, format!("{}.png", original.name));
    assert_eq!(files[2].0, format!("{} (2).png", original.name));
    assert_eq!(the_card(&files[1].1).name, the_card(CHARX).name);
    assert_eq!(s.post_json("/characters/export", json!({ "ids": [] })).await.status(), 400);
    assert_eq!(s.post_json("/characters/export", json!({ "ids": [ilse, 999] })).await.status(), 404);
}

fn persona_named<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["personas"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap_or_else(|| panic!("no persona {name}"))
}

/// The persona fields that must survive an export and an import.
fn persona_fields(p: &Value) -> Value {
    json!([p["name"], p["description"], p["pronouns"], p["appearance"], p["personality"], p["background"], p["every_turn"], p["card_priority"], p["has_avatar"]])
}

/// Personas: SillyTavern backup zip and RisuAI PNG round trips, duplicates, the SillyTavern
/// backup and `settings.json` with `User Avatars`, and a card taken over as a persona.
#[tokio::test(flavor = "multi_thread")]
async fn persona_round_trips() {
    let s = server("personas").await;
    let bea: Value = s
        .post_json("/personas", json!({ "name": "Bea", "description": "Kapitänin eines Fischkutters.", "pronouns": "sie/ihr",
            "appearance": "graue Locken", "personality": "ruhig", "background": "aus Büsum", "every_turn": true, "card_priority": "card" }))
        .await
        .json()
        .await
        .unwrap();
    let bea_id = bea["id"].as_i64().unwrap();
    let res = s
        .c
        .put(format!("{}/personas/{bea_id}/avatar", s.base))
        .header("authorization", &s.auth)
        .body(picture(64, 64, ImageFormat::Jpeg))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    let before = persona_fields(persona_named(&s.get("/personas").await.json().await.unwrap(), "Bea"));

    // SillyTavern backup zip: JSON in ST's layout plus `User Avatars/`.
    let (ct, zip) = s.bytes(&format!("/personas/export?ids={bea_id}")).await;
    assert_eq!(ct, "application/zip");
    let files = unzip(&zip);
    assert!(files[0].0.starts_with("personas_") && files[0].0.ends_with(".json"));
    let backup: Value = serde_json::from_slice(&files[0].1).unwrap();
    let key = format!("rp-{bea_id}.png");
    assert_eq!(backup["personas"][&key], "Bea");
    assert_eq!(backup["persona_descriptions"][&key]["description"], "Kapitänin eines Fischkutters.");
    assert_eq!(files[1].0, format!("User Avatars/{key}"));
    assert!(files[1].1.starts_with(b"\x89PNG"));

    let dup = s.upload("/personas/import", vec![("personas.zip".into(), zip.clone())]).await;
    assert_eq!((dup["duplicates"].as_u64(), dup["results"][0]["persona_id"].as_i64()), (Some(1), Some(bea_id)));

    // Risu PNG of the same persona.
    let (_, risu) = s.bytes(&format!("/personas/{bea_id}/export.png")).await;
    match formats::detect_and_parse(&risu).unwrap().content {
        Content::Persona { persona, .. } => assert_eq!((persona.name.as_str(), persona.prompt.as_str()), ("Bea", "Kapitänin eines Fischkutters.")),
        Content::Cards(_) => panic!("a card"),
    }

    let del = s.c.delete(format!("{}/personas/{bea_id}", s.base)).header("authorization", &s.auth).send().await.unwrap();
    assert_eq!(del.status(), 200);
    let v = s.upload("/personas/import", vec![("personas.zip".into(), zip)]).await;
    assert_eq!(v["imported"], 1, "{v}");
    let list: Value = s.get("/personas").await.json().await.unwrap();
    assert_eq!(persona_fields(persona_named(&list, "Bea")), before);

    let id = persona_named(&list, "Bea")["id"].as_i64().unwrap();
    s.c.delete(format!("{}/personas/{id}", s.base)).header("authorization", &s.auth).send().await.unwrap();
    let v = s.upload("/personas/import", vec![("bea.png".into(), risu)]).await;
    assert_eq!(v["imported"], 1, "{v}");
    assert_eq!(persona_fields(persona_named(&s.get("/personas").await.json().await.unwrap(), "Bea")), before);

    // SillyTavern's own backup (no pictures inside) with one picture from `User Avatars`.
    let avatar = picture(32, 32, ImageFormat::Png);
    let v = s.upload("/personas/import", vec![("personas_20261115.json".into(), ST_BACKUP.to_vec()), ("User Avatars/user-default.png".into(), avatar.clone())]).await;
    assert_eq!((v["imported"].as_u64(), v["results"].as_array().unwrap().len()), (Some(2), 2), "{v}");
    let list: Value = s.get("/personas").await.json().await.unwrap();
    let jonas = persona_named(&list, "Jonas");
    assert_eq!((jonas["description"].as_str(), jonas["has_avatar"].as_bool()), (Some("Jonas ist Zimmermann aus Husum, ruhig und hilfsbereit."), Some(true)));
    assert_eq!(persona_named(&list, "Wanderin")["has_avatar"], false);
    let (_, pic) = s.bytes(&format!("/personas/{}/avatar", jonas["id"])).await;
    assert_eq!(pic, avatar);

    // The same personas inside SillyTavern's settings.json: duplicates now.
    let backup: Value = serde_json::from_slice(ST_BACKUP).unwrap();
    let settings = json!({ "username": "x", "power_user": backup });
    let v = s.upload("/personas/import", vec![("settings.json".into(), serde_json::to_vec(&settings).unwrap()), ("x.json".into(), b"{}".to_vec())]).await;
    assert_eq!((v["duplicates"].as_u64(), v["errors"].as_u64()), (Some(2), Some(1)), "{v}");
    assert_eq!(v["results"][0]["code"], "no_persona");

    // A card as a persona: once new, then a duplicate.
    let c = s.upload("/import", vec![("ilse.png".into(), ILSE_PNG.to_vec())]).await;
    let ilse = c["results"][0]["character_id"].as_i64().unwrap();
    let res = s.post_json(&format!("/characters/{ilse}/as-persona"), json!({})).await;
    assert_eq!(res.status(), 201);
    let first: Value = res.json().await.unwrap();
    let res = s.post_json(&format!("/characters/{ilse}/as-persona"), json!({})).await;
    assert_eq!(res.status(), 200);
    let second: Value = res.json().await.unwrap();
    assert_eq!((second["status"].as_str(), second["persona_id"].clone()), (Some("duplicate"), first["persona_id"].clone()));
    let card = the_card(ILSE_PNG);
    let list: Value = s.get("/personas").await.json().await.unwrap();
    let p = persona_named(&list, &card.name);
    assert!(!p["description"].as_str().unwrap().contains("{{char}}"));
    assert_eq!(p["has_avatar"], true);
    assert_eq!(s.post_json("/characters/999/as-persona", json!({})).await.status(), 404);
}

fn stored_zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in files {
        zip.start_file(name.as_str(), options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A zip of `entries` files of `mib` MiB zeros each: a few hundred KB that unpack to a lot.
fn bomb(entries: usize, mib: usize) -> Vec<u8> {
    let zeros = vec![0u8; mib << 20];
    let names: Vec<String> = (0..entries).map(|i| format!("e{i}.json")).collect();
    zip_of(&names.iter().map(|n| (n.as_str(), zeros.as_slice())).collect::<Vec<_>>())
}

fn codes(v: &Value) -> Vec<(String, String)> {
    v["results"].as_array().unwrap().iter().map(|r| (r["file"].as_str().unwrap().into(), r["code"].as_str().unwrap_or("").into())).collect()
}

/// Zip bombs: entries that unpack far more than they pack are `too_large` without being
/// inflated; the request still answers and the other files of the upload are imported.
#[tokio::test(flavor = "multi_thread")]
async fn zip_bomb_is_refused_without_unpacking() {
    let s = server("bomb").await;
    let b = bomb(3, 64);
    assert!(b.len() < 1 << 20, "{}", b.len());
    let v = s.upload("/import", vec![("a.zip".into(), b.clone()), ("b.zip".into(), b), ("ok.json".into(), formats::write_json_v2(&sfw_card(1)))]).await;
    assert_eq!((v["imported"].as_u64(), v["errors"].as_u64()), (Some(1), Some(6)), "{v}");
    let c = codes(&v);
    assert_eq!(c[0], ("a.zip/e0.json".into(), "too_large".into()));
    assert!(c[..6].iter().all(|(_, code)| code == "too_large"), "{c:?}");
    assert!(crate::import::WORK_PEAK.load(std::sync::atomic::Ordering::SeqCst) <= crate::import::WORK_BUDGET as usize);
}

/// Many small entries are read one by one; more than `MAX_ZIP_ENTRIES` is `too_large` for the
/// zip as a whole; a zip inside a zip is not unpacked (`no_card`), the cards beside it are.
#[tokio::test(flavor = "multi_thread")]
async fn many_entries_and_nested_zips() {
    let s = server("entries").await;
    let small: Vec<(String, Vec<u8>)> = (0..3000).map(|i| (format!("n/{i}.txt"), b"hallo".to_vec())).collect();
    let mut with_card = small.clone();
    with_card.push(("karte.json".into(), formats::write_json_v2(&sfw_card(5))));
    let v = s.upload("/import", vec![("viele.zip".into(), stored_zip(&with_card))]).await;
    assert_eq!((v["imported"].as_u64(), v["errors"].as_u64()), (Some(1), Some(3000)), "{}", v["imported"]);
    assert_eq!(codes(&v)[0], ("viele.zip/n/0.txt".into(), "unknown_format".into()));

    let too_many: Vec<(String, Vec<u8>)> = (0..=rp_core::formats::MAX_ZIP_ENTRIES).map(|i| (format!("{i}.txt"), vec![b'x'])).collect();
    let v = s.upload("/import", vec![("zuviel.zip".into(), stored_zip(&too_many))]).await;
    assert_eq!(codes(&v), [("zuviel.zip".to_string(), "too_large".to_string())]);

    let inner = zip_of(&[("tief.json", &formats::write_json_v2(&sfw_card(6)))]);
    let outer = zip_of(&[("innen.zip", &inner), ("oben.json", &formats::write_json_v2(&sfw_card(7))), ("bombe.zip", &bomb(1, 64))]);
    let v = s.upload("/import", vec![("aussen.zip".into(), outer)]).await;
    assert_eq!(statuses(&v), [
        ("aussen.zip/innen.zip".to_string(), "error".to_string()),
        ("aussen.zip/oben.json".to_string(), "imported".to_string()),
        ("aussen.zip/bombe.zip".to_string(), "error".to_string()),
    ]);
    assert_eq!(codes(&v)[0].1, "no_card");
}

/// A card whose picture claims 100 000 x 100 000 pixels is imported without a thumbnail (the
/// picture is never decoded), the server stays up.
#[tokio::test(flavor = "multi_thread")]
async fn huge_picture_in_a_card() {
    let s = server("hugepic").await;
    let mut pic = picture(1, 1, ImageFormat::Png);
    pic[16..20].copy_from_slice(&100_000u32.to_be_bytes());
    pic[20..24].copy_from_slice(&100_000u32.to_be_bytes());
    let card = formats::write_png(&sfw_card(9), Some(&pic)).unwrap();
    let v = s.upload("/import", vec![("riesig.png".into(), card)]).await;
    assert_eq!(v["imported"], 1, "{v}");
    let id = v["results"][0]["character_id"].as_i64().unwrap();
    // No thumbnail was made: the original file is served in its place.
    let (ct, _) = s.bytes(&format!("/characters/{id}/thumb")).await;
    assert_eq!(ct, "image/png");
    assert_eq!(s.get("/health").await.status(), 200);
}

/// Several uploads at once (each with cards and a bomb): all answer with every card, they take
/// turns, and the shared budget is never exceeded.
#[tokio::test(flavor = "multi_thread")]
async fn parallel_uploads_share_the_budget() {
    let s = server("parallel").await;
    let b = bomb(2, 64);
    let uploads = (0..5).map(|u| {
        let cards: Vec<(String, Vec<u8>)> = (0..20).map(|i| (format!("{i}.json"), formats::write_json_v2(&sfw_card(1000 + u * 100 + i)))).collect();
        vec![("karten.zip".to_string(), stored_zip(&cards)), ("bombe.zip".to_string(), b.clone())]
    });
    let answers = futures_util::future::join_all(uploads.map(|files| s.upload("/import", files))).await;
    for v in &answers {
        assert_eq!((v["imported"].as_u64(), v["errors"].as_u64()), (Some(20), Some(2)), "{}", v["errors"]);
    }
    let lib: Value = s.get("/library").await.json().await.unwrap();
    assert_eq!(lib["characters"].as_array().unwrap().len(), 100);
    assert!(crate::import::WORK_PEAK.load(std::sync::atomic::Ordering::SeqCst) <= crate::import::WORK_BUDGET as usize);
}

/// A real large collection: 600 card PNGs (~220 KB each) in one zip of ~130 MB, more than a single
/// card file may be. All are imported (stored in more than one transaction), in upload order.
#[tokio::test(flavor = "multi_thread")]
async fn large_collection_in_one_zip() {
    let s = server("collection").await;
    // A noisy picture packs badly, like a real portrait.
    let mut x = 1u32;
    let img = RgbImage::from_fn(256, 288, |_, _| {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        image::Rgb([(x >> 16) as u8, (x >> 8) as u8, (x >> 24) as u8])
    });
    let mut art = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(img).write_to(&mut art, ImageFormat::Png).unwrap();
    let art = art.into_inner();
    let cards: Vec<(String, Vec<u8>)> = (0..600).map(|i| (format!("sammlung/karte_{i:03}.png"), formats::write_png(&sfw_card(i), Some(&art)).unwrap())).collect();
    let zip = stored_zip(&cards);
    assert!(zip.len() > rp_core::formats::MAX_INPUT, "{}", zip.len());
    drop(cards);
    let mb = zip.len() / 1_000_000;
    let t = Instant::now();
    let v = s.upload("/import", vec![("sammlung.zip".into(), zip)]).await;
    println!("600 cards ({mb} MB zip) imported in {:.1} s", t.elapsed().as_secs_f64());
    assert_eq!((v["imported"].as_u64(), v["errors"].as_u64()), (Some(600), Some(0)), "{}", v["errors"]);
    let st = statuses(&v);
    assert_eq!(st[0], ("sammlung.zip/sammlung/karte_000.png".into(), "imported".into()));
    assert_eq!(st[599], ("sammlung.zip/sammlung/karte_599.png".into(), "imported".into()));
    let lib: Value = s.get("/library").await.json().await.unwrap();
    assert_eq!(lib["characters"].as_array().unwrap().len(), 600);
    assert!(crate::import::WORK_PEAK.load(std::sync::atomic::Ordering::SeqCst) <= crate::import::WORK_BUDGET as usize);
}

/// `RP_TEST_CARDS_DIR=<folder>`: imports the first 100 card files found there (sorted, also in
/// sub folders) into a temp data folder and prints counts and time only, never names or text.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_cards_from_folder() {
    let Some(dir) = std::env::var_os("RP_TEST_CARDS_DIR") else { panic!("set RP_TEST_CARDS_DIR") };
    let mut paths = vec![];
    let mut todo = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                ["png", "json", "charx", "yaml", "yml", "byaf", "gz", "zip"].contains(&x.to_ascii_lowercase().as_str())
            }) {
                paths.push(p);
            }
        }
    }
    paths.sort();
    // Only files that hold a card (the folder also has plain pictures and notes).
    let mut files: Vec<(String, Vec<u8>)> = vec![];
    let mut skipped = 0;
    for p in &paths {
        if files.len() == 100 {
            break;
        }
        let bytes = std::fs::read(p).unwrap();
        if matches!(formats::detect_and_parse(&bytes), Ok(f) if matches!(f.content, Content::Cards(_))) {
            files.push((format!("f{}", files.len()), bytes));
        } else {
            skipped += 1;
        }
    }
    let mb = files.iter().map(|(_, b)| b.len()).sum::<usize>() as f64 / 1e6;
    let files_len = files.len();
    let s = server("realcards").await;
    let t = Instant::now();
    let v = s.upload("/import", files).await;
    let secs = t.elapsed().as_secs_f64();
    let mut codes = std::collections::BTreeMap::new();
    for r in v["results"].as_array().unwrap() {
        if let Some(code) = r["code"].as_str() {
            *codes.entry(code.to_string()).or_insert(0) += 1;
        }
    }
    println!(
        "real cards: {} card files ({skipped} other files skipped), {mb:.1} MB, {secs:.2} s; imported {} duplicates {} errors {} {codes:?}",
        files_len,
        v["imported"],
        v["duplicates"],
        v["errors"]
    );
    assert!(secs < 10.0, "took {secs:.2} s");
}
