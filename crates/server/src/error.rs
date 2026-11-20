//! The one error answer of the API: a status code and `{"error": "<code>"}`, plus the code (and
//! for storage errors the detail) for the request log.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// The error code (and detail) of a failed response, for the request log (`reqlog`).
#[derive(Clone)]
pub(crate) struct ErrCode(pub String, pub Option<String>);

/// A failed request. Every handler answers `Result<Response, ApiError>`.
#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    code: &'static str,
    /// What went wrong inside, for the log only; never sent to the client.
    detail: Option<String>,
}

pub(crate) type ApiResult = Result<Response, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str) -> ApiError {
        ApiError { status, code, detail: None }
    }

    pub fn bad_request(code: &'static str) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, code)
    }

    pub fn not_found(code: &'static str) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, code)
    }

    pub fn conflict(code: &'static str) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, code)
    }

    /// The database or a file could not be read or written: 500 `storage`.
    pub fn storage(e: impl std::fmt::Display) -> ApiError {
        eprintln!("storage error: {e}");
        ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, code: "storage", detail: Some(e.to_string()) }
    }

    /// A refused change of a chat's messages (the codes `Hub` answers with).
    pub fn message(code: &'static str) -> ApiError {
        let status = match code {
            "message_missing" | "chat_missing" => StatusCode::NOT_FOUND,
            "busy" | "undo_gone" => StatusCode::CONFLICT,
            "storage" => StatusCode::INTERNAL_SERVER_ERROR,
            crate::chat::CONTEXT_FULL => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        };
        ApiError::new(status, code)
    }

    /// 413 `context_full`: the chat's prompt does not fit into the model's context even with the
    /// history cut down to the newest message (over the socket the same code comes as `error` of
    /// the `done` event).
    pub fn context_full() -> ApiError {
        ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, crate::chat::CONTEXT_FULL)
    }
}

impl From<crate::store::StoreError> for ApiError {
    fn from(e: crate::store::StoreError) -> ApiError {
        ApiError::storage(e)
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> ApiError {
        ApiError::storage(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut res = (self.status, Json(json!({ "error": self.code }))).into_response();
        res.extensions_mut().insert(ErrCode(self.code.to_string(), self.detail));
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_codes_map_to_their_status() {
        for (code, status) in [
            ("message_missing", 404),
            ("chat_missing", 404),
            ("busy", 409),
            ("undo_gone", 409),
            ("storage", 500),
            ("empty", 400),
            ("no_swipes", 400),
            ("swipe_out_of_range", 400),
            ("context_full", 413),
        ] {
            assert_eq!(ApiError::message(code).into_response().status().as_u16(), status, "{code}");
        }
    }

    /// The overflow code W1-B's chat shows its own words for: 413 with `{"error":"context_full"}`.
    #[tokio::test]
    async fn context_full_is_413_with_its_code() {
        let res = ApiError::context_full().into_response();
        assert_eq!(res.status().as_u16(), 413);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), json!({ "error": "context_full" }));
    }
}
