//! How chats look: the app-wide reading settings (`/chat-look`), what one chat changes of them
//! (`/chats/{id}/look`) and the backdrop pictures of a chat and of a character
//! (`/chats/{id}/backdrop`, `/characters/{id}/backdrop`). Every change tells the other devices
//! with `{"type":"look"}`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::picture;
use crate::error::{ApiError, ApiResult};
use crate::store::{self, BackdropOf, Frame, Look, BACKDROP_MODES};
use crate::{imaging, AppState};

/// Largest accepted backdrop file. The web UI scales pictures down before it sends them.
const MAX_BACKDROP_BYTES: usize = 12 * 1024 * 1024;

/// A stored backdrop's longer side and JPEG quality: sharp on a 1440p window, a few hundred kB.
pub(super) const BACKDROP_SIDE: u32 = 2560;
pub(super) const BACKDROP_QUALITY: u8 = 86;

pub(super) fn changed(st: &AppState) -> ApiResult {
    st.hub.send_event(&json!({ "type": "look" }));
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn chat_missing() -> ApiError {
    ApiError::not_found("chat_missing")
}

pub(super) fn owner_missing(of: BackdropOf) -> ApiError {
    ApiError::not_found(match of {
        BackdropOf::Chat(_) => "chat_missing",
        BackdropOf::Character(_) => "character_missing",
    })
}

pub(super) async fn get_look(State(st): State<AppState>) -> ApiResult {
    let look = st.hub.store(|s| s.look())?;
    Ok(Json(json!({ "look": look })).into_response())
}

/// Body = the whole app-wide settings (an empty field is the app's default again).
pub(super) async fn put_look(State(st): State<AppState>, Json(look): Json<Look>) -> ApiResult {
    let look = look.checked().map_err(ApiError::bad_request)?;
    st.hub.store(|s| s.set_look(&look))?;
    changed(&st)
}

pub(super) async fn get_chat_look(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let view = st.hub.store(|s| s.chat_look(id))?.ok_or_else(chat_missing)?;
    Ok(Json(view).into_response())
}

#[derive(Deserialize)]
pub(super) struct ChatLookEdit {
    /// What this chat changes (the whole set; an empty field follows the app again).
    look: Option<Look>,
    /// One of `BACKDROP_MODES`.
    backdrop: Option<String>,
}

pub(super) async fn put_chat_look(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<ChatLookEdit>) -> ApiResult {
    let look = body.look.map(Look::checked).transpose().map_err(ApiError::bad_request)?;
    if body.backdrop.as_deref().is_some_and(|m| !BACKDROP_MODES.contains(&m)) {
        return Err(ApiError::bad_request("bad_look"));
    }
    match st.hub.store(|s| s.set_chat_look(id, look.as_ref(), body.backdrop.as_deref())) {
        Ok(true) => changed(&st),
        Ok(false) => Err(chat_missing()),
        Err(e) if e.to_string() == "no_backdrop" => Err(ApiError::conflict("no_backdrop")),
        Err(e) => Err(e.into()),
    }
}

async fn get_backdrop(st: AppState, of: BackdropOf) -> ApiResult {
    picture(st.hub.store(|s| s.backdrop_file(of))).await
}

/// Body = the image file (PNG, JPEG, WebP or GIF). It is stored as a JPEG of at most
/// `BACKDROP_SIDE` pixels.
async fn put_backdrop(st: AppState, of: BackdropOf, body: Bytes) -> ApiResult {
    if body.len() > MAX_BACKDROP_BYTES {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large"));
    }
    store::image_mime(&body).ok_or(ApiError::bad_request("bad_image"))?;
    // Decoding and encoding a large picture takes a moment: not on the async threads.
    let jpeg = tokio::task::spawn_blocking(move || imaging::fit_jpeg(&body, BACKDROP_SIDE, BACKDROP_QUALITY))
        .await
        .map_err(ApiError::storage)?
        .ok_or(ApiError::bad_request("bad_image"))?;
    if !st.hub.store(|s| s.set_backdrop(of, Some(&jpeg)))? {
        return Err(owner_missing(of));
    }
    changed(&st)
}

async fn delete_backdrop(st: AppState, of: BackdropOf) -> ApiResult {
    if !st.hub.store(|s| s.set_backdrop(of, None))? {
        return Err(owner_missing(of));
    }
    changed(&st)
}

async fn put_frame(st: AppState, of: BackdropOf, frame: Frame) -> ApiResult {
    if !st.hub.store(|s| s.set_backdrop_frame(of, frame))? {
        return Err(ApiError::not_found("no_backdrop"));
    }
    changed(&st)
}

pub(super) async fn get_chat_backdrop(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    get_backdrop(st, BackdropOf::Chat(id)).await
}

pub(super) async fn put_chat_backdrop(State(st): State<AppState>, Path(id): Path<i64>, body: Bytes) -> ApiResult {
    put_backdrop(st, BackdropOf::Chat(id), body).await
}

pub(super) async fn delete_chat_backdrop(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    delete_backdrop(st, BackdropOf::Chat(id)).await
}

pub(super) async fn put_chat_backdrop_frame(State(st): State<AppState>, Path(id): Path<i64>, Json(frame): Json<Frame>) -> ApiResult {
    put_frame(st, BackdropOf::Chat(id), frame).await
}

/// Makes this chat's own backdrop the default of its character.
pub(super) async fn post_backdrop_to_character(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    if !st.hub.store(|s| s.backdrop_to_character(id))? {
        return Err(ApiError::not_found("no_backdrop"));
    }
    changed(&st)
}

pub(super) async fn get_character_backdrop(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    get_backdrop(st, BackdropOf::Character(id)).await
}

pub(super) async fn put_character_backdrop(State(st): State<AppState>, Path(id): Path<i64>, body: Bytes) -> ApiResult {
    put_backdrop(st, BackdropOf::Character(id), body).await
}

pub(super) async fn delete_character_backdrop(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    delete_backdrop(st, BackdropOf::Character(id)).await
}

pub(super) async fn put_character_backdrop_frame(State(st): State<AppState>, Path(id): Path<i64>, Json(frame): Json<Frame>) -> ApiResult {
    put_frame(st, BackdropOf::Character(id), frame).await
}
