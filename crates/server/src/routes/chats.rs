//! Chats: open or start one, the overview of a character's chats, titles, soft delete with undo,
//! the chat's own version of the card, and the export (one file, or several in a ZIP).

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use rp_core::card::Card;
use serde::Deserialize;
use serde_json::json;

use super::characters::clean_card;
use crate::error::{ApiError, ApiResult};
use crate::export::{self, Format, Transcript};
use crate::logs::{self, Entry, Level};
use crate::store;
use crate::AppState;

/// Longest chat title (characters); also for the title of a fork.
pub(super) const TITLE_MAX: usize = 80;

pub(super) async fn get_chats(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let chats = st.hub.store(|s| s.chats_of(id))?;
    Ok(Json(json!({ "chats": chats })).into_response())
}

/// Answer to a change of a chat that answers whether it exists: 204, or 404 `chat_missing`.
fn chat_changed(st: &AppState, result: store::Res<bool>) -> ApiResult {
    if !result? {
        return Err(ApiError::not_found("chat_missing"));
    }
    st.hub.library_changed();
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub(super) struct ChatEdit {
    title: String,
}

/// Names a chat. An empty title makes it unnamed again.
pub(super) async fn put_chat(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<ChatEdit>) -> ApiResult {
    let title = body.title.trim();
    if title.chars().count() > TITLE_MAX {
        return Err(ApiError::bad_request("bad_title"));
    }
    let result = st.hub.store(|s| s.set_chat_title(id, title));
    chat_changed(&st, result)
}

/// 409 `busy` while the model writes in it.
pub(super) async fn delete_chat(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let result = st.hub.delete_chat(id).map_err(ApiError::conflict)?;
    chat_changed(&st, result)
}

pub(super) async fn restore_chat(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let result = st.hub.store(|s| s.set_chat_deleted(id, false));
    chat_changed(&st, result)
}

/// The card this chat plays with, and whether it is the chat's own version.
pub(super) async fn get_chat_card(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let (c, custom) = st.hub.store(|s| s.chat_character_card(id))?.ok_or(ApiError::not_found("chat_missing"))?;
    Ok(Json(json!({ "info": c.info, "card": c.card, "custom": custom })).into_response())
}

/// Body = card fields for this chat only; the character's card stays as it is.
pub(super) async fn put_chat_card(State(st): State<AppState>, Path(id): Path<i64>, Json(card): Json<Card>) -> ApiResult {
    let card = clean_card(card)?;
    let result = st.hub.store(|s| s.set_chat_card(id, Some(&card)));
    chat_changed(&st, result)
}

/// Back to the character's card.
pub(super) async fn delete_chat_card(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let result = st.hub.store(|s| s.set_chat_card(id, None));
    chat_changed(&st, result)
}

#[derive(Deserialize)]
pub(super) struct OpenChat {
    character_id: i64,
    /// Start a new chat even if one exists.
    #[serde(default)]
    fresh: bool,
}

/// The latest chat with the character, or a new one: `{chat_id}`.
pub(super) async fn post_chat(State(st): State<AppState>, Json(body): Json<OpenChat>) -> ApiResult {
    let chat_id = st.hub.open_chat(body.character_id, body.fresh)?.ok_or(ApiError::not_found("character_missing"))?;
    st.hub.library_changed();
    Ok(Json(json!({ "chat_id": chat_id })).into_response())
}

#[derive(Deserialize)]
pub(super) struct ExportQuery {
    /// `md`, `txt` or `jsonl`.
    format: String,
    /// Minutes east of UTC on the device that asks (dates in the file and its name are its local time).
    #[serde(default)]
    tz: i32,
}

/// A download: the bytes with their type and file name; never cached.
fn download(mime: &str, name: &str, body: Vec<u8>) -> ApiResult {
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::CONTENT_DISPOSITION, export::disposition(name)),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        body,
    )
        .into_response())
}

/// One chat as a file: `?format=md|txt|jsonl`. Logged without names or text.
pub(super) async fn export_chat(State(st): State<AppState>, Path(id): Path<i64>, Query(q): Query<ExportQuery>) -> ApiResult {
    let format = Format::parse(&q.format).ok_or(ApiError::bad_request("bad_format"))?;
    let tz = export::clamp_tz(q.tz);
    let chat = st.hub.store(|s| export::transcript(s, id))?.ok_or(ApiError::not_found("chat_missing"))?;
    let body = export::render(&chat, format, tz);
    st.log().write(
        Entry::new(Level::Info, "server", "export", "chat exported")
            .chat(id)
            .data(json!({ "format": format.ext(), "messages": chat.lines.len() })),
    );
    download(format.mime(), &export::file_name(&chat, format, tz), body.into_bytes())
}

#[derive(Deserialize)]
pub(super) struct ExportMany {
    ids: Vec<i64>,
    format: String,
    #[serde(default)]
    tz: i32,
}

/// Several chats in one ZIP (each id once, in the order given). 404 `chat_missing` if one is gone.
pub(super) async fn export_chats(State(st): State<AppState>, Json(body): Json<ExportMany>) -> ApiResult {
    let format = Format::parse(&body.format).ok_or(ApiError::bad_request("bad_format"))?;
    let tz = export::clamp_tz(body.tz);
    let mut ids: Vec<i64> = Vec::with_capacity(body.ids.len());
    for id in body.ids {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() || ids.len() > export::ZIP_MAX {
        return Err(ApiError::bad_request("bad_selection"));
    }
    let chats = st.hub.store(|s| ids.iter().map(|&id| export::transcript(s, id)).collect::<store::Res<Vec<Option<Transcript>>>>())?;
    let chats: Vec<Transcript> = chats.into_iter().collect::<Option<_>>().ok_or(ApiError::not_found("chat_missing"))?;
    let now = logs::now_ms() / 1000;
    let messages: usize = chats.iter().map(|c| c.lines.len()).sum();
    let bytes = tokio::task::spawn_blocking(move || export::archive(&chats, format, tz, now)).await??;
    st.log().write(
        Entry::new(Level::Info, "server", "export", "chats exported")
            .data(json!({ "format": format.ext(), "chats": ids.len(), "messages": messages })),
    );
    let name = format!("Chats - {}.zip", logs::file_stamp(now * 1000, tz));
    download("application/zip", &name, bytes)
}
