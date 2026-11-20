//! The activity log over HTTP: events from the UI, the Logs tab reading and clearing it, and the
//! export for bug reports.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ApiResult;
use crate::logs::{self, Entry, Level};
use crate::reqlog::{device_of, quick_success};
use crate::store;
use crate::AppState;

#[derive(Deserialize)]
pub(super) struct UiEvent {
    ts: i64,
    level: String,
    category: String,
    message: String,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    chat_id: Option<i64>,
    #[serde(default)]
    character_id: Option<i64>,
    /// The browser merged this many equal events in a row (the last at `last_ts`).
    #[serde(default)]
    repeat: Option<i64>,
    #[serde(default)]
    last_ts: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct UiBatch {
    events: Vec<UiEvent>,
}

/// Most events one batch from the UI may carry.
const UI_BATCH_MAX: usize = 500;

/// An API call the browser reports that succeeded quickly: left out like on the server side
/// (`reqlog::quick_success`), so failures and slow calls stay. Older UIs still send them all.
fn quick_api_call(ev: &UiEvent) -> bool {
    let field = |k: &str| ev.data.as_ref().and_then(|d| d.get(k)).and_then(Value::as_u64);
    match (ev.category.as_str(), field("status"), field("ms")) {
        ("api", Some(status), Some(ms)) => status > 0 && quick_success(status.min(999) as u16, ms.into()),
        _ => false,
    }
}

/// Events from the UI (clicks, page changes, API calls, errors), sent in batches.
pub(super) async fn post_logs(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(batch): Json<UiBatch>,
) -> ApiResult {
    let device = device_of(Some(&addr), &headers);
    let now = logs::now_ms();
    let n = batch.events.len();
    for ev in batch.events.into_iter().take(UI_BATCH_MAX).filter(|ev| !quick_api_call(ev)) {
        let category: String = ev.category.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(24).collect();
        let mut e = Entry::new(Level::parse(&ev.level), "ui", if category.is_empty() { "other" } else { &category }, ev.message);
        // The device clock decides the order of its own events, unless it is far off.
        e.ts = if (ev.ts - now).abs() < 86_400_000 { ev.ts } else { now };
        e.repeat = ev.repeat.unwrap_or(1).clamp(1, 1_000_000);
        e.last_ts = ev.last_ts.filter(|t| e.repeat > 1 && *t >= e.ts && *t - e.ts < 86_400_000);
        e.data = ev.data;
        e.chat_id = ev.chat_id;
        e.character_id = ev.character_id;
        st.log().write(e.device(device.clone()));
    }
    if n > UI_BATCH_MAX {
        st.log().write(Entry::new(Level::Warn, "server", "logs", format!("dropped {} UI events of one batch", n - UI_BATCH_MAX)).device(device));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub(super) async fn get_logs(State(st): State<AppState>, Query(q): Query<logs::Query>) -> ApiResult {
    let dir = st.data_dir.clone();
    let page = tokio::task::spawn_blocking(move || logs::open_read(&dir).and_then(|c| logs::query(&c, &q))).await??;
    Ok(Json(page).into_response())
}

pub(super) async fn delete_logs(State(st): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> ApiResult {
    // Entries still on their way in would land after the clearing; write them first.
    let log = st.log().clone();
    let dir = st.data_dir.clone();
    let n = tokio::task::spawn_blocking(move || {
        log.flush(std::time::Duration::from_secs(2));
        logs::open_read(&dir).and_then(|c| logs::clear(&c))
    })
    .await??;
    st.log().write(
        Entry::new(Level::Info, "server", "logs", format!("log cleared ({n} entries)"))
            .device(device_of(Some(&addr), &headers))
            .data(json!({ "removed": n })),
    );
    Ok(Json(json!({ "removed": n })).into_response())
}

#[derive(Deserialize)]
pub(super) struct ExportParams {
    /// Minutes east of UTC on the device that asks (times in the file are its local time).
    #[serde(default)]
    tz: i32,
    /// Screen of that device, e.g. "1920x1080@1.25".
    #[serde(default)]
    screen: String,
}

/// The whole log as a text file to send in with a bug report.
pub(super) async fn export_logs(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(p): Query<ExportParams>,
) -> ApiResult {
    let tz = p.tz.clamp(-14 * 60, 14 * 60);
    let screen: String = p.screen.chars().filter(|c| c.is_ascii_digit() || matches!(c, 'x' | '.' | '@')).take(24).collect();
    let device = device_of(Some(&addr), &headers);
    let log = st.log().clone();
    let dir = st.data_dir.clone();
    let token = st.token.to_string();
    let lan_token = st.lan.token();
    let text = tokio::task::spawn_blocking(move || -> store::Res<String> {
        log.flush(std::time::Duration::from_secs(2));
        let conn = logs::open_read(&dir)?;
        let mut head = vec![
            ("Exportiert von".to_string(), device),
            ("Bildschirm".into(), if screen.is_empty() { "(unbekannt)".into() } else { screen }),
        ];
        head.extend(logs::setup_lines(&dir));
        let llama = [("llama-server.log", 200), ("llama-server.prev.log", 60)]
            .map(|(name, n)| (format!("{name} (letzte {n} Zeilen)"), logs::tail(&dir.join(name), n)));
        // The phones' token is blanked the same way.
        Ok(logs::export(&conn, &head, &llama, tz, logs::home_dir().as_deref(), &token)?.replace(&*lan_token, "<token>"))
    })
    .await??;
    let name = format!("lorelamp-logs-{}.txt", logs::file_stamp(logs::now_ms(), tz));
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        text,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(category: &str, data: Value) -> UiEvent {
        serde_json::from_value(json!({ "ts": 1, "level": "debug", "category": category, "message": "GET /api/x", "data": data })).unwrap()
    }

    /// API calls the browser reports are kept only when they failed, got no answer or were slow.
    #[test]
    fn quick_api_calls_from_the_ui_are_left_out() {
        assert!(quick_api_call(&ev("api", json!({ "status": 200, "ms": 40 }))));
        assert!(quick_api_call(&ev("api", json!({ "status": 204, "ms": 999 }))));
        assert!(!quick_api_call(&ev("api", json!({ "status": 404, "ms": 40 }))), "failures stay");
        assert!(!quick_api_call(&ev("api", json!({ "status": 0, "ms": 40 }))), "no answer stays");
        assert!(!quick_api_call(&ev("api", json!({ "status": 200, "ms": 1_200 }))), "slow calls stay");
        assert!(!quick_api_call(&ev("click", json!({ "status": 200, "ms": 40 }))), "only API calls");
        assert!(!quick_api_call(&ev("api", Value::Null)));
    }
}
