//! The request log: an activity log entry for each HTTP request that failed or was slow (method,
//! path, status, time, and the error code of a failure); quick successes say nothing.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use serde_json::json;

use crate::error::ErrCode;
use crate::logs::{Entry, Level};
use crate::AppState;

/// Who sent a request: "PC" (this computer) or "WLAN <ip>", plus the short id the browser sends
/// in `X-RP-Client`.
pub fn device_label(addr: Option<&SocketAddr>, client: Option<&str>) -> String {
    let base = match addr {
        Some(a) if a.ip().is_loopback() => "PC".to_string(),
        Some(a) => format!("WLAN {}", a.ip()),
        None => "?".to_string(),
    };
    let id: String = client.unwrap_or("").chars().filter(char::is_ascii_alphanumeric).take(8).collect();
    if id.is_empty() {
        base
    } else {
        format!("{base} {id}")
    }
}

pub(crate) fn device_of(addr: Option<&SocketAddr>, headers: &HeaderMap) -> String {
    device_label(addr, headers.get("x-rp-client").and_then(|v| v.to_str().ok()))
}

/// Requests that would only log themselves: the Logs tab reading the log, the UI sending its
/// events, and the status poll that runs every few seconds.
fn quiet_path(path: &str) -> bool {
    path.starts_with("/api/logs") || path == "/api/engine" || path == "/api/health"
}

/// Answers at least this slow are logged even when they succeed.
pub(crate) const SLOW_MS: u128 = 1_000;

/// A request that says nothing worth keeping: it succeeded (below 400) and answered within
/// `SLOW_MS`. Nearly all requests are these (library, chats, settings, avatars, the UI's files);
/// actions worth tracing write entries of their own (chat, backups, imports, the model).
pub(crate) fn quick_success(status: u16, ms: u128) -> bool {
    status < 400 && ms < SLOW_MS
}

/// Logs failed and slow HTTP requests: method, path (never the query, it may hold the token),
/// status, time.
pub(crate) async fn log_requests(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if quiet_path(&path) {
        return next.run(req).await;
    }
    let method = req.method().to_string();
    let addr = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let device = device_of(addr.as_ref(), req.headers());
    let started = std::time::Instant::now();
    let res = next.run(req).await;
    let status = res.status().as_u16();
    if quick_success(status, started.elapsed().as_millis()) {
        return res;
    }
    let level = match status {
        500.. => Level::Error,
        400.. => Level::Warn,
        _ if method == "GET" => Level::Debug,
        _ => Level::Info,
    };
    let mut data = json!({ "method": method, "path": path, "status": status, "ms": started.elapsed().as_millis() as u64 });
    if let Some(ErrCode(code, detail)) = res.extensions().get::<ErrCode>() {
        data["error"] = json!(code);
        if let Some(d) = detail {
            data["detail"] = json!(d);
        }
    }
    st.log().write(Entry::new(level, "server", "http", format!("{method} {path} -> {status}")).device(device).data(data));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_labels() {
        let pc: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let phone: SocketAddr = "192.168.1.23:5000".parse().unwrap();
        assert_eq!(device_label(Some(&pc), Some("ab12")), "PC ab12");
        assert_eq!(device_label(Some(&phone), None), "WLAN 192.168.1.23");
        assert_eq!(device_label(Some(&phone), Some("x<y>z")), "WLAN 192.168.1.23 xyz");
        assert!(quiet_path("/api/logs") && quiet_path("/api/logs/export") && !quiet_path("/api/library"));
    }

    #[test]
    fn only_failures_and_slow_answers_are_logged() {
        assert!(quick_success(200, 3));
        assert!(quick_success(304, 3));
        assert!(quick_success(201, 999));
        assert!(!quick_success(400, 3), "failures stay");
        assert!(!quick_success(503, 3), "failures stay");
        assert!(!quick_success(200, 1_000), "slow answers stay");
        assert!(!quick_success(204, 4_000), "slow answers stay");
    }
}
