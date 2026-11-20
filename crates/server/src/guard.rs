//! Who may talk to the server: the Host/Origin check against DNS rebinding (every request) and
//! the API token (every `/api` request but `POST /api/pair`, see below).
//!
//! Two listeners, two tokens: the local one (127.0.0.1, the desktop window; its token is new at
//! every start) and, while "Handy verbinden" is on, the LAN one (`lan::Lan`; requests over it carry
//! `lan::ViaLan` and need the phones' token, never the local one).
//!
//! The one route without a token is `POST /api/pair` (a phone trades the 6-digit code for the
//! phones' token; `routes/lan.rs`). It is registered outside the token layer in `routes::api`, so
//! nothing else can slip through; the Host/Origin check below applies to it as to everything.

use std::net::IpAddr;

use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::lan::{reachable_ip, ViaLan};
use crate::AppState;

/// DNS-rebinding protection: the Host must be `localhost` or an IP literal
/// (an attacker-controlled DNS name can never match). Non-loopback IPs are
/// only accepted in LAN mode and only from private, link-local and the shared (VPN) range.
pub fn host_allowed(host_header: &str, lan: bool) -> bool {
    let host = if let Some(rest) = host_header.strip_prefix('[') {
        match rest.split_once(']') {
            Some((h, _)) => h,
            None => return false,
        }
    } else if host_header.matches(':').count() > 1 {
        host_header // bare IPv6 without brackets
    } else {
        host_header.rsplit_once(':').map_or(host_header, |(h, _)| h)
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() => true,
        Ok(ip @ IpAddr::V4(_)) => lan && reachable_ip(ip),
        _ => false,
    }
}

fn origin_host(origin: &str) -> &str {
    origin.split_once("://").map_or(origin, |(_, rest)| rest).split('/').next().unwrap_or("")
}

/// The page of the phone app (Tauri on Android) before it is connected: it may only pair.
pub(crate) fn is_app_origin(origin: &str) -> bool {
    matches!(origin, "http://tauri.localhost" | "https://tauri.localhost")
}

/// `POST /api/pair` (or its CORS preflight).
pub(crate) fn is_pair(req: &Request) -> bool {
    req.uri().path() == "/api/pair" && (req.method() == Method::POST || req.method() == Method::OPTIONS)
}

pub(crate) async fn check_host(State(st): State<AppState>, req: Request, next: Next) -> Response {
    // Over the LAN listener only while it is on (a connection still open after the switch went off
    // gets nothing more); the local listener accepts only loopback names.
    let lan = req.extensions().get::<ViaLan>().is_some();
    if lan && !st.lan.is_on() {
        return (StatusCode::FORBIDDEN, "phones are switched off").into_response();
    }
    let h = req.headers();
    let host_ok = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| host_allowed(v, lan));
    let origin_ok = h
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|o| host_allowed(origin_host(o), lan) || (is_pair(&req) && is_app_origin(o)));
    if host_ok && origin_ok {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "forbidden host").into_response()
    }
}

/// Token from `Authorization: Bearer <t>` or `?token=<t>`.
pub fn extract_token<'a>(auth_header: Option<&'a str>, query: Option<&'a str>) -> Option<&'a str> {
    if let Some(t) = auth_header.and_then(|v| v.strip_prefix("Bearer ")) {
        return Some(t.trim());
    }
    query?.split('&').find_map(|kv| kv.strip_prefix("token="))
}

/// Compares in constant time (for equal lengths), so the answer time tells nothing.
pub(crate) fn token_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) async fn require_token(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let auth = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let lan = req.extensions().get::<ViaLan>().is_some();
    let ok = extract_token(auth, req.uri().query()).is_some_and(|t| {
        if lan {
            st.lan.is_on() && token_eq(t, &st.lan.token())
        } else {
            token_eq(t, &st.token)
        }
    });
    if ok {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "missing or wrong token").into_response()
    }
}

/// The routes that act on the PC itself (files and folders, processes, engines, models, backups,
/// the system, the phones' switch; `routes::pc_only`): over the LAN listener 403 `pc_only`. A phone
/// chats, reads and edits characters, personas and chats, imports card files and changes settings
/// that only concern the app.
pub(crate) async fn require_pc(req: Request, next: Next) -> Response {
    let via = req.extensions().get::<ViaLan>().copied();
    match pc_only(via) {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}

/// The same answer for a part of a route a phone may not use (`ViaLan` from the request).
pub(crate) fn pc_only(via: Option<ViaLan>) -> Result<(), ApiError> {
    match via {
        Some(_) => Err(ApiError::new(StatusCode::FORBIDDEN, "pc_only")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_from_header_or_query() {
        assert_eq!(extract_token(Some("Bearer abc"), None), Some("abc"));
        assert_eq!(extract_token(None, Some("x=1&token=abc")), Some("abc"));
        assert_eq!(extract_token(None, Some("x=1")), None);
        assert_eq!(extract_token(Some("Basic abc"), None), None);
    }

    #[test]
    fn token_compare() {
        assert!(token_eq("abc", "abc"));
        assert!(!token_eq("abc", "abd"));
        assert!(!token_eq("abc", "abcd"));
    }

    #[test]
    fn host_checks() {
        assert!(host_allowed("localhost:7860", false));
        assert!(host_allowed("127.0.0.1:7860", false));
        assert!(host_allowed("[::1]:7860", false));
        assert!(!host_allowed("192.168.1.13:7860", false));
        assert!(host_allowed("192.168.1.13:7860", true));
        assert!(host_allowed("100.80.1.2:7860", true));
        assert!(!host_allowed("100.80.1.2:7860", false));
        assert!(!host_allowed("8.8.8.8", true));
        assert!(!host_allowed("evil.example.com", true));
        assert!(!host_allowed("evil.example.com:7860", false));
        assert!(!host_allowed("localhost.evil.com", true));
        assert!(!host_allowed("tauri.localhost", true));
    }

    #[test]
    fn app_origin_is_exact() {
        assert!(is_app_origin("http://tauri.localhost"));
        assert!(!is_app_origin("http://tauri.localhost.evil.com"));
        assert!(!is_app_origin("http://evil.tauri.localhost"));
    }
}
