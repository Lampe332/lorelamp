//! Changes to the messages of a chat: edit, choosing a reply variant, rewind with undo, and fork.
//! They all go through the `Hub`, which refuses them while the model writes in that chat and
//! sends every device the chat's new state.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::chats::TITLE_MAX;
use crate::error::{ApiError, ApiResult};
use crate::AppState;

#[derive(Deserialize)]
pub(super) struct MessageEdit {
    content: String,
}

/// Replaces the text of one message. Every device gets the chat's new state over its socket.
pub(super) async fn put_message(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<MessageEdit>) -> ApiResult {
    st.hub.edit_message(id, &body.content).map_err(ApiError::message)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub(super) struct SwipeChoice {
    swipe_id: i64,
}

/// Makes another of the message's reply variants the active one. Every device gets the chat's
/// new state over its socket.
pub(super) async fn put_swipe(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<SwipeChoice>) -> ApiResult {
    st.hub.set_swipe(id, body.swipe_id).map_err(ApiError::message)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Removes this message and everything after it (undo: `/chats/{id}/unrewind`).
pub(super) async fn rewind(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let (chat_id, removed) = st.hub.rewind(id).map_err(ApiError::message)?;
    Ok(Json(json!({ "chat_id": chat_id, "removed": removed })).into_response())
}

pub(super) async fn undo_rewind(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    st.hub.undo_rewind(id).map_err(ApiError::message)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub(super) struct Fork {
    /// Name of the new chat; empty = unnamed.
    #[serde(default)]
    title: String,
}

/// Starts a new chat that holds this one up to and including the message.
pub(super) async fn fork(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<Fork>) -> ApiResult {
    let title = body.title.trim();
    if title.chars().count() > TITLE_MAX {
        return Err(ApiError::bad_request("bad_title"));
    }
    let chat_id = st.hub.fork(id, title).map_err(ApiError::message)?;
    Ok(Json(json!({ "chat_id": chat_id })).into_response())
}
