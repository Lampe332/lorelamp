//! `GET /search/messages`: full text search over the messages of all chats (or of one).
//! The query is never logged (the request log records the path only).

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::ApiResult;
use crate::AppState;

/// Hits answered when `limit` is left out, and the most a request can ask for.
const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 200;

#[derive(Deserialize)]
pub(super) struct SearchQuery {
    #[serde(default)]
    q: String,
    chat_id: Option<i64>,
    limit: Option<i64>,
}

/// `?q=…&chat_id=…&limit=…` → `{hits: [...]}`; fewer than 2 letters find nothing.
pub(super) async fn get_message_search(State(st): State<AppState>, Query(q): Query<SearchQuery>) -> ApiResult {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let typed = q.q.trim();
    let hits = if typed.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        vec![]
    } else {
        st.hub.store(|s| s.search_messages(typed, q.chat_id, limit))?
    };
    Ok(Json(serde_json::json!({ "hits": hits })).into_response())
}
