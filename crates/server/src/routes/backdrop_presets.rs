//! The ready-made backdrops: their gallery thumbnails (`GET /backdrop-presets/{id}`) and picking
//! one for a chat or as a character's default (`PUT /chats/{id}/backdrop/preset`,
//! `PUT /characters/{id}/backdrop/preset`, body `{"preset": "<id>"}`). A pick is stored like an
//! uploaded picture (a JPEG in `<data>/backdrops/`), so every device shows the same and the frame,
//! "make it the default" and removing work as before.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use super::chat_look::{changed, owner_missing, BACKDROP_QUALITY, BACKDROP_SIDE};
use crate::error::{ApiError, ApiResult};
use crate::store::BackdropOf;
use crate::{backdrop_presets, imaging, AppState};

fn unknown() -> ApiError {
    ApiError::not_found("no_preset")
}

pub(super) async fn get_thumb(Path(id): Path<String>) -> ApiResult {
    let bytes = backdrop_presets::thumb(&id).ok_or_else(unknown)?;
    // Part of the program: the same id always has the same picture until an update, which is rare.
    Ok((
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "private, max-age=86400"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
pub(super) struct Pick {
    preset: String,
}

async fn put_preset(st: AppState, of: BackdropOf, pick: Pick) -> ApiResult {
    let picture = backdrop_presets::picture(&pick.preset).ok_or_else(unknown)?;
    let jpeg = tokio::task::spawn_blocking(move || imaging::fit_jpeg(picture, BACKDROP_SIDE, BACKDROP_QUALITY))
        .await
        .map_err(ApiError::storage)?
        .ok_or(ApiError::bad_request("bad_image"))?;
    if !st.hub.store(|s| s.set_preset_backdrop(of, &jpeg, &pick.preset))? {
        return Err(owner_missing(of));
    }
    changed(&st)
}

pub(super) async fn put_chat_preset(State(st): State<AppState>, Path(id): Path<i64>, Json(pick): Json<Pick>) -> ApiResult {
    put_preset(st, BackdropOf::Chat(id), pick).await
}

pub(super) async fn put_character_preset(State(st): State<AppState>, Path(id): Path<i64>, Json(pick): Json<Pick>) -> ApiResult {
    put_preset(st, BackdropOf::Character(id), pick).await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::http::StatusCode;
    use serde_json::{json, Value};

    use crate::{start, Config, WebUi};

    /// Through the real server: thumbnails, a pick for a chat and for a character, and both still
    /// there after the server starts again on the same data folder.
    #[tokio::test]
    async fn presets_over_http_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("rp-app-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        let cfg = || Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        };
        let c = reqwest::Client::new();
        let running = start(cfg()).await.unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let auth = format!("Bearer {}", running.token);
        let card = br#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"Mara","description":"A baker.","first_mes":"Hello."}}"#;
        let character: Value = c.post(format!("{base}/characters/import")).header("authorization", &auth).body(&card[..]).send().await.unwrap().json().await.unwrap();
        let character = character["id"].as_i64().unwrap();
        let chat: Value = c.post(format!("{base}/chats")).header("authorization", &auth).json(&json!({ "character_id": character, "fresh": true })).send().await.unwrap().json().await.unwrap();
        let chat = chat["chat_id"].as_i64().unwrap();

        // Thumbnails: small WebP, unknown ids are 404, and the token is required.
        let thumb = c.get(format!("{base}/backdrop-presets/bergsee")).header("authorization", &auth).send().await.unwrap();
        assert_eq!((thumb.status(), thumb.headers()["content-type"].to_str().unwrap()), (StatusCode::OK, "image/webp"));
        assert!(thumb.bytes().await.unwrap().len() < 24 * 1024);
        let missing = c.get(format!("{base}/backdrop-presets/nope")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(c.get(format!("{base}/backdrop-presets/bergsee")).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);

        let pick = |path: String, preset: &str| c.put(format!("{base}{path}")).header("authorization", &auth).json(&json!({ "preset": preset })).send();
        assert_eq!(pick(format!("/chats/{chat}/backdrop/preset"), "taverne").await.unwrap().status(), StatusCode::NO_CONTENT);
        assert_eq!(pick(format!("/characters/{character}/backdrop/preset"), "wald").await.unwrap().status(), StatusCode::NO_CONTENT);
        let bad = pick(format!("/chats/{chat}/backdrop/preset"), "../rp-app.db").await.unwrap();
        assert_eq!(bad.status(), StatusCode::NOT_FOUND);
        assert_eq!(pick(format!("/chats/{}/backdrop/preset", chat + 100), "wald").await.unwrap().status(), StatusCode::NOT_FOUND);
        // Stored as JPEG copies in the data folder, like an uploaded picture.
        let jpeg = std::fs::read(dir.join("data").join("backdrops").join(format!("chat-{chat}.jpg"))).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8]));
        running.stop();

        let running = start(cfg()).await.unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let auth = format!("Bearer {}", running.token);
        let view: Value = c.get(format!("{base}/chats/{chat}/look")).header("authorization", &auth).send().await.unwrap().json().await.unwrap();
        let b = &view["backdrop"];
        assert_eq!((b["mode"].as_str(), b["own"]["preset"].as_str(), b["character"]["preset"].as_str()), (Some("own"), Some("taverne"), Some("wald")));
        let served = c.get(format!("{base}/chats/{chat}/backdrop")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(served.headers()["content-type"], "image/jpeg");
        assert_eq!(&served.bytes().await.unwrap()[..], &jpeg[..]);
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
