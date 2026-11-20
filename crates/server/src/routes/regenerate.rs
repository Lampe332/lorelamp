//! The chat's last reply once more: `POST /messages/{id}/regenerate` writes another variant of it
//! (the earlier ones stay in `messages.swipes`, the new one becomes the active one),
//! `POST /messages/{id}/continue` lets the model go on at its end. Both go through the `Hub`:
//! refused with 409 `busy` while the model writes, and every device gets the chat's `history`
//! snapshot when the writing starts and when it ends. The text itself arrives over the socket
//! (`token`, `done`), as for any reply.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::error::{ApiError, ApiResult};
use crate::AppState;

/// `not_last`: the chat went on meanwhile; `benchmark`: the model server runs test settings.
fn refused(code: &'static str) -> ApiError {
    let status = match code {
        "message_missing" => StatusCode::NOT_FOUND,
        "busy" | "benchmark" | "not_last" => StatusCode::CONFLICT,
        "storage" => StatusCode::INTERNAL_SERVER_ERROR,
        crate::chat::CONTEXT_FULL => return ApiError::context_full(),
        _ => StatusCode::BAD_REQUEST,
    };
    ApiError::new(status, code)
}

pub(super) async fn regenerate(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    st.hub.regenerate(id).map_err(refused)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub(super) async fn continue_reply(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    st.hub.continue_reply(id).map_err(refused)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_map_to_their_status() {
        for (code, status) in [("message_missing", 404), ("busy", 409), ("benchmark", 409), ("not_last", 409), ("not_reply", 400), ("storage", 500), ("context_full", 413)] {
            assert_eq!(refused(code).into_response().status().as_u16(), status, "{code}");
        }
    }
}
