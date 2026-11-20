//! The `/api` routes, one file per area. This file puts them together and holds what several
//! areas share (pictures, crop positions).

mod backdrop_presets;
mod backups;
mod bench;
mod builder;
mod characters;
mod chat_look;
mod chats;
mod engine;
mod export;
mod fetch;
mod general;
mod import;
mod lan;
pub(crate) use lan::get_manifest as manifest;
mod library;
mod local_models;
mod logs;
mod messages;
mod picture_link;
mod regenerate;
mod models;
pub(crate) mod personas;
mod purpose;
mod search;
mod system;
mod tutorial;
mod ws;

use std::path::PathBuf;

use axum::extract::DefaultBodyLimit;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{middleware, Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::error::{ApiError, ApiResult};
use crate::guard::{require_pc, require_token};
use crate::store::Res;
use crate::AppState;

/// Largest accepted card file (PNG cards with big artwork run to several MB).
const MAX_CARD_BYTES: usize = 32 * 1024 * 1024;

/// Every `/api` route, behind the token check (but `POST /pair`); the ones of [`pc_only`] answer a
/// phone 403 `pc_only`.
pub(crate) fn api(state: &AppState) -> Router<AppState> {
    shared()
        .merge(pc_only().route_layer(middleware::from_fn(require_pc)))
        .layer(DefaultBodyLimit::max(MAX_CARD_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        // The one route without a token (a phone trades the pairing code for it), added after the
        // token layer so that layer does not cover it; see `guard`.
        .route("/pair", post(lan::post_pair).options(lan::options_pair).layer(DefaultBodyLimit::max(1024)))
}

/// What acts on the PC itself, refused over the phones' listener as one group (`guard::require_pc`):
/// the model server's configuration (profiles created, deleted, activated; benchmark and advice that
/// rewrite them; the idle time), engines (download, delete), model files and folders (download,
/// delete, show in Explorer, use, read a file's header by path), the PC scan, SillyTavern on this PC's
/// drives, backups (restore, delete, limit), start with Windows, the data folder and the phones' switch.
/// A phone keeps reading all of it and may still load or unload the model the PC chose (a message
/// does the same). Two routes check one part themselves: `PUT /models/profiles/{id}` (model file,
/// engine, start options) and `PUT /settings` (`engine_port`).
fn pc_only() -> Router<AppState> {
    Router::new()
        .route("/engine/idle", put(engine::put_idle))
        .route("/models/profiles", post(models::post_profile))
        .route("/models/profiles/{id}", delete(models::delete_profile))
        .route("/models/profiles/{id}/activate", post(models::activate_profile))
        .route("/models/profiles/{id}/benchmark", post(bench::post_benchmark))
        .route("/models/profiles/{id}/benchmark/cancel", post(bench::post_cancel))
        .route("/models/profiles/{id}/benchmark/apply", post(bench::post_apply))
        .route("/models/profiles/{id}/advice/apply", post(purpose::post_apply))
        .route("/models/engines/download", post(system::post_engine_download))
        .route("/models/engines/{id}", delete(system::delete_engine))
        .route("/system/scan", post(system::post_scan))
        .route("/models/downloads", post(fetch::post_download))
        .route("/models/downloads/{id}", delete(fetch::delete_download))
        .route("/models/downloads/{id}/pause", post(fetch::pause_download))
        .route("/models/downloads/{id}/resume", post(fetch::resume_download))
        .route("/models/local", delete(local_models::delete_local))
        .route("/models/local/use", post(local_models::use_local))
        .route("/models/local/reveal", post(local_models::reveal_local))
        .route("/models/folders", put(local_models::put_folders))
        .route("/models/estimate", post(local_models::post_estimate))
        .route("/general/autostart", put(general::put_autostart))
        .route("/general/open-data-folder", post(general::open_data_folder))
        .route("/import/sillytavern/find", get(import::get_sillytavern_find))
        .route("/import/sillytavern/contents", get(import::get_sillytavern_contents))
        .route("/import/sillytavern/image", get(import::get_sillytavern_image))
        .route("/import/sillytavern", post(import::post_sillytavern_import))
        .route("/backups/limit", put(backups::put_limit))
        .route("/backups/delete", post(backups::delete_many))
        .route("/backups/{name}", delete(backups::delete_backup))
        .route("/backups/{name}/restore", post(backups::restore_backup))
        .route("/lan", get(lan::get_lan).put(lan::put_lan))
        .route("/lan/disconnect", post(lan::post_disconnect))
        .route("/lan/code", post(lan::post_code))
}

/// Everything a phone may use as well.
fn shared() -> Router<AppState> {
    Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        .route("/engine", get(engine::get_engine))
        .route("/engine/unload", post(engine::post_unload))
        .route("/engine/load", post(engine::post_load))
        .route("/engine/restart", post(engine::post_restart))
        .route("/models/profiles", get(models::get_profiles))
        .route("/models/profiles/{id}", put(models::put_profile))
        .route("/models/profiles/{id}/benchmark", get(bench::get_benchmark))
        .route("/models/purpose", get(purpose::get_purpose).put(purpose::put_purpose))
        .route("/models/profiles/{id}/advice", get(purpose::get_advice))
        .route("/models/engines", get(models::get_engines))
        .route("/system/scan", get(system::get_scan))
        .route("/system/engine-releases", get(system::get_engine_releases))
        .route("/system/engine-download", get(system::get_engine_download))
        .route("/models/downloads", get(fetch::get_downloads))
        .route("/models/resolve", post(fetch::post_resolve))
        .route("/models/catalog", get(fetch::get_catalog))
        .route("/models/search", get(fetch::get_search))
        .route("/models/local", get(local_models::get_local))
        .route("/library", get(library::get_library))
        .route("/settings", put(library::put_settings))
        .route("/general", get(general::get_general).put(general::put_general))
        .route("/tutorial", get(tutorial::get_tutorial).put(tutorial::put_tutorial))
        .route("/characters/import", post(characters::import_character))
        .route("/builder/draft", post(builder::post_draft))
        .route("/builder/save", post(builder::post_save))
        .route("/builder/from/{id}", get(builder::get_from))
        // Bulk uploads: their own limits while streaming (`import::MAX_UPLOAD`, per file `MAX_INPUT`).
        .route("/import", post(import::post_import).layer(DefaultBodyLimit::disable()))
        .route("/personas/import", post(import::post_personas_import).layer(DefaultBodyLimit::disable()))
        .route("/import/link", post(import::post_import_link))
        .route("/import/link/preview", post(import::post_link_preview))
        .route("/import/link/preview/{id}/image", get(import::get_link_preview_image))
        .route("/import/link/confirm", post(import::post_link_confirm))
        // A phone sets persona and character pictures too; every fetch goes through `netguard`.
        .route("/pictures/from-link", post(picture_link::post_from_link))
        .route("/characters/export", post(export::post_cards_export))
        .route("/characters/bulk", post(characters::post_bulk))
        .route("/characters/{id}/favorite", put(characters::put_favorite))
        .route("/characters/{id}/tags", put(characters::put_tags))
        .route("/tags", get(characters::get_tags))
        .route("/search/messages", get(search::get_message_search))
        .route("/characters/{id}/export", get(export::get_card_export))
        .route("/characters/{id}/thumb", get(export::get_thumb))
        .route("/characters/{id}/as-persona", post(import::post_as_persona))
        .route("/personas/export", get(export::get_personas_export))
        .route("/personas/{id}/export.png", get(export::get_persona_png))
        .route(
            "/characters/{id}",
            get(characters::get_character).put(characters::put_character).delete(characters::delete_character),
        )
        .route("/characters/{id}/restore", post(characters::restore_character))
        .route(
            "/characters/{id}/avatar",
            get(characters::get_avatar).put(characters::put_avatar).delete(characters::delete_avatar),
        )
        .route("/characters/{id}/avatar-position", put(characters::put_avatar_position))
        .route("/characters/{id}/chats", get(chats::get_chats))
        .route("/characters/{id}/default-persona", put(personas::put_default_persona))
        .route("/characters/{id}/user-priority", put(personas::put_user_priority))
        .route("/characters/{id}/user-check", get(personas::get_user_check))
        .route("/personas", get(personas::get_personas).post(personas::post_persona))
        .route("/personas/{id}", put(personas::put_persona).delete(personas::delete_persona))
        .route("/personas/{id}/activate", put(personas::activate_persona))
        .route(
            "/personas/{id}/avatar",
            get(personas::get_persona_avatar).put(personas::put_persona_avatar).delete(personas::delete_persona_avatar),
        )
        .route("/personas/{id}/avatar-position", put(personas::put_persona_avatar_position))
        .route("/chats", post(chats::post_chat))
        .route("/chats/export", post(chats::export_chats))
        .route("/chats/{id}/export", get(chats::export_chat))
        .route("/chats/{id}", put(chats::put_chat).delete(chats::delete_chat))
        .route("/chats/{id}/restore", post(chats::restore_chat))
        .route("/chats/{id}/card", get(chats::get_chat_card).put(chats::put_chat_card).delete(chats::delete_chat_card))
        .route("/chats/{id}/persona", put(personas::put_chat_persona))
        .route("/chats/{id}/unrewind", post(messages::undo_rewind))
        .route("/chat-look", get(chat_look::get_look).put(chat_look::put_look))
        .route("/chats/{id}/look", get(chat_look::get_chat_look).put(chat_look::put_chat_look))
        .route(
            "/chats/{id}/backdrop",
            get(chat_look::get_chat_backdrop).put(chat_look::put_chat_backdrop).delete(chat_look::delete_chat_backdrop),
        )
        .route("/chats/{id}/backdrop/frame", put(chat_look::put_chat_backdrop_frame))
        .route("/chats/{id}/backdrop/to-character", post(chat_look::post_backdrop_to_character))
        .route(
            "/characters/{id}/backdrop",
            get(chat_look::get_character_backdrop).put(chat_look::put_character_backdrop).delete(chat_look::delete_character_backdrop),
        )
        .route("/characters/{id}/backdrop/frame", put(chat_look::put_character_backdrop_frame))
        .route("/backdrop-presets/{id}", get(backdrop_presets::get_thumb))
        .route("/chats/{id}/backdrop/preset", put(backdrop_presets::put_chat_preset))
        .route("/characters/{id}/backdrop/preset", put(backdrop_presets::put_character_preset))
        .route("/messages/{id}", put(messages::put_message))
        .route("/messages/{id}/swipe", put(messages::put_swipe))
        .route("/messages/{id}/rewind", post(messages::rewind))
        .route("/messages/{id}/fork", post(messages::fork))
        .route("/messages/{id}/regenerate", post(regenerate::regenerate))
        .route("/messages/{id}/continue", post(regenerate::continue_reply))
        .route("/backups", get(backups::get_backups).post(backups::post_backup))
        .route("/logs", get(logs::get_logs).post(logs::post_logs).delete(logs::delete_logs))
        .route("/logs/export", get(logs::export_logs))
        .route("/ws", get(ws::ws_handler))
}

/// Body of the routes that move the visible part of a cropped picture.
#[derive(Deserialize)]
struct Position {
    x: f64,
    y: f64,
}

/// A stored picture (`store` answers file and type), or 404 `no_avatar` — also when the store
/// fails, so a broken picture never breaks the page around it.
async fn picture(stored: Res<Option<(PathBuf, String)>>) -> ApiResult {
    picture_with(stored, true).await
}

/// [`picture`]; `immutable` false: the browser asks again each time (a stand-in that will be
/// replaced under the same URL).
async fn picture_with(stored: Res<Option<(PathBuf, String)>>, immutable: bool) -> ApiResult {
    let Ok(Some((path, mime))) = stored else { return Err(ApiError::not_found("no_avatar")) };
    let bytes = tokio::fs::read(path).await.map_err(|_| ApiError::not_found("no_avatar"))?;
    // The URL carries the picture's revision, so devices may keep each version for good.
    // `nosniff`: the stored type is what the browser gets, never a guess from the bytes.
    Ok((
        [
            (header::CONTENT_TYPE, mime.as_str()),
            (header::CACHE_CONTROL, if immutable { "private, max-age=31536000, immutable" } else { "private, no-cache" }),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}
