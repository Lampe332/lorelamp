//! "Handy verbinden": the switch for phones in the home network, the QR code's address, the
//! pairing code and "Alle Handys trennen" (`crate::lan`). Only the PC itself changes these; a phone
//! gets 403 `pc_only` (`routes::pc_only`). `POST /pair` is the one route without a token (see `guard`);
//! wrong codes are counted per sender address (`Lan::pair`).

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::error::{ApiError, ApiResult};
use crate::guard::{is_app_origin, token_eq};
use crate::lan::{Paired, ViaLan};
use crate::AppState;

/// `GET /lan`: `{enabled, port, addresses: [{ip, adapter}], ip, url, code, code_expires_in,
/// locked_for, problem, search}`. `url` (the QR code's content) is
/// `http://<ip>:<port>/?token=<token>`; `code` is the 6-digit pairing code (null while off).
pub(super) async fn get_lan(State(st): State<AppState>) -> ApiResult {
    get_status(&st).await
}

#[derive(Deserialize)]
pub(super) struct LanBody {
    enabled: Option<bool>,
    /// The address for the QR code when the PC has several.
    ip: Option<String>,
}

/// `PUT /lan {enabled?, ip?}`: switches phones on or off (kept for the next start) and/or picks
/// the address. Answers the status; 409 `port_busy` (no free port), 400 `bad_ip`.
pub(super) async fn put_lan(State(st): State<AppState>, Json(body): Json<LanBody>) -> ApiResult {
    if let Some(ip) = &body.ip {
        let lan = st.lan.clone();
        let ip = ip.clone();
        if !tokio::task::spawn_blocking(move || lan.choose(&ip)).await? {
            return Err(ApiError::bad_request("bad_ip"));
        }
    }
    match body.enabled {
        Some(true) => {
            st.lan.enable().await.map_err(ApiError::conflict)?;
            st.hub.store(|s| s.set_lan_enabled(true))?;
        }
        Some(false) => {
            st.lan.disable().await;
            st.hub.store(|s| s.set_lan_enabled(false))?;
        }
        None => {}
    }
    get_status(&st).await
}

async fn get_status(st: &AppState) -> ApiResult {
    // The adapter list asks Windows; off the async threads.
    let lan = st.lan.clone();
    Ok(Json(tokio::task::spawn_blocking(move || lan.status()).await?).into_response())
}

/// `POST /lan/disconnect`: "Alle Handys trennen" (new phones' token, every phone connection cut,
/// old QR codes refused). Answers the status.
pub(super) async fn post_disconnect(State(st): State<AppState>) -> ApiResult {
    st.lan.new_token();
    get_status(&st).await
}

/// `POST /lan/code`: "Neuer Code". Answers the status.
pub(super) async fn post_code(State(st): State<AppState>) -> ApiResult {
    st.lan.renew_code();
    get_status(&st).await
}

#[derive(Deserialize)]
pub(super) struct PairBody {
    code: String,
}

/// The phone app's page (`tauri.localhost`) may read the answer.
fn with_cors(mut res: Response, headers: &HeaderMap) -> Response {
    if let Some(origin) = headers.get(header::ORIGIN).filter(|o| o.to_str().is_ok_and(is_app_origin)) {
        let h = res.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    res
}

/// `POST /pair {code}` (no token): 200 `{token}`; 401 `wrong_code`; 429 `too_many` (this address sent
/// 5 wrong ones and waits, 30 s and longer each time; or 20 wrong ones from all addresses replaced the
/// code and pause pairing for 30 s); 409 `lan_off`. The phone then opens
/// `http://<ip>:<port>/?token=<token>`, the same address as the QR code.
pub(super) async fn post_pair(
    State(st): State<AppState>,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PairBody>,
) -> Response {
    let res = match st.lan.pair(&body.code, from.ip()) {
        Paired::Token(token) => Json(json!({ "token": &*token })).into_response(),
        Paired::Wrong => ApiError::new(StatusCode::UNAUTHORIZED, "wrong_code").into_response(),
        Paired::TooMany => ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many").into_response(),
        Paired::Off => ApiError::conflict("lan_off").into_response(),
    };
    with_cors(res, &headers)
}

/// The CORS preflight of `POST /pair` from the phone app's page.
pub(super) async fn options_pair(headers: HeaderMap) -> Response {
    let mut res = StatusCode::NO_CONTENT.into_response();
    if headers.get(header::ORIGIN).is_some_and(|o| o.to_str().is_ok_and(is_app_origin)) {
        let h = res.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("POST"));
        h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type"));
        h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    }
    with_cors(res, &headers)
}

#[derive(Deserialize)]
pub(crate) struct ManifestQuery {
    token: Option<String>,
}

/// `GET /manifest.webmanifest` (outside `/api`, no token needed): the UI's web app manifest. With
/// `?token=` of this listener (the phones' one over the LAN) its start address carries the token, so
/// Lorelamp put on a phone's home screen opens connected. A wrong token changes nothing.
pub(crate) async fn get_manifest(State(st): State<AppState>, via: Option<Extension<ViaLan>>, Query(q): Query<ManifestQuery>) -> Response {
    let Some(bytes) = st.web.file("manifest.webmanifest") else { return StatusCode::NOT_FOUND.into_response() };
    let Ok(mut manifest) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return StatusCode::NOT_FOUND.into_response() };
    let valid = q.token.as_deref().filter(|t| match via {
        Some(_) => st.lan.is_on() && token_eq(t, &st.lan.token()),
        None => token_eq(t, &st.token),
    });
    if let (Some(t), Some(m)) = (valid, manifest.as_object_mut()) {
        m.insert("start_url".into(), json!(format!("/?token={t}")));
    }
    ([(header::CONTENT_TYPE, "application/manifest+json"), (header::CACHE_CONTROL, "no-store")], manifest.to_string()).into_response()
}
