//! The settings tab "Allgemein": how sending works, what the app opens with, when it asks before
//! it deletes, how long an undo stays, how often it backs up by itself, starting with Windows and
//! the data folder. Plain values in the table `settings`; starting with Windows lives in the
//! registry (`autostart.rs`). Every change tells all devices (`{"type":"general"}`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::autostart;
use crate::backup::{self, Rhythm};
use crate::error::{ApiError, ApiResult};
use crate::lan::ViaLan;
use crate::logs::{Entry, Level};
use crate::AppState;

/// Which key sends a message; the first is the default.
const SEND_KEYS: [&str; 2] = ["enter", "ctrl_enter"];
/// What the app shows when it is opened; the first is the default.
const START_PAGES: [&str; 3] = ["home", "last_chat", "characters"];
/// How long "Rückgängig" stays on screen, in seconds.
const UNDO_SECS: [u32; 4] = [5, 8, 15, 30];
const DEFAULT_UNDO_SECS: u32 = 8;

fn pc_only(via: &Option<Extension<ViaLan>>) -> Result<(), ApiError> {
    match via {
        Some(_) => Err(ApiError::new(StatusCode::FORBIDDEN, "pc_only")),
        None => Ok(()),
    }
}

/// Everything the tab shows. `data_dir` and `autostart` belong to the PC: a phone gets null.
async fn snapshot(st: &AppState, at_pc: bool) -> Result<Value, ApiError> {
    let stored = st.hub.store(|s| -> crate::store::Res<_> {
        Ok((s.general("send_key")?, s.general("start_page")?, s.general("confirm_delete")?, s.general("confirm_rewind")?, s.general("undo_secs")?, s.general(backup::RHYTHM_KEY)?))
    })?;
    let (send_key, start_page, confirm_delete, confirm_rewind, undo_secs, rhythm) = stored;
    let one_of = |v: Option<String>, all: &[&'static str]| all.iter().copied().find(|a| v.as_deref() == Some(a)).unwrap_or(all[0]);
    let rhythm = rhythm.as_deref().and_then(Rhythm::parse).unwrap_or_default();
    let dir = st.data_dir.to_path_buf();
    let next = tokio::task::spawn_blocking(move || backup::next_automatic(&dir, crate::logs::now_ms())).await??;
    let autostart = if at_pc { tokio::task::spawn_blocking(autostart::state).await? } else { None };
    Ok(json!({
        "send_key": one_of(send_key, &SEND_KEYS),
        "start_page": one_of(start_page, &START_PAGES),
        "confirm_delete": confirm_delete.as_deref() == Some("1"),
        "confirm_rewind": confirm_rewind.as_deref() == Some("1"),
        "undo_secs": undo_secs.and_then(|v| v.parse().ok()).filter(|v| UNDO_SECS.contains(v)).unwrap_or(DEFAULT_UNDO_SECS),
        "backup_rhythm": rhythm.as_str(),
        "backup_next_ms": next,
        "data_dir": at_pc.then(|| st.data_dir.display().to_string()),
        "autostart": autostart.map(|a| json!({ "on": a.on, "minimized": a.minimized })),
    }))
}

/// `GET /general`: `{send_key, start_page, confirm_delete, confirm_rewind, undo_secs,
/// backup_rhythm, backup_next_ms, data_dir, autostart: {on, minimized} | null}`.
/// `backup_next_ms`: when the app backs up by itself next (in the past: within minutes; null:
/// never). `data_dir` is null for a phone, `autostart` null where the app cannot start with
/// Windows (a plain server, a phone).
pub(super) async fn get_general(State(st): State<AppState>, via: Option<Extension<ViaLan>>) -> ApiResult {
    Ok(Json(snapshot(&st, via.is_none()).await?).into_response())
}

/// A change; fields left out stay as they are.
#[derive(Deserialize)]
pub(super) struct GeneralBody {
    send_key: Option<String>,
    start_page: Option<String>,
    confirm_delete: Option<bool>,
    confirm_rewind: Option<bool>,
    undo_secs: Option<u32>,
    backup_rhythm: Option<String>,
}

/// The key/value pairs to store, or the error code of a bad request.
fn check(body: &GeneralBody) -> Result<Vec<(&'static str, String)>, &'static str> {
    let mut out = Vec::new();
    let flag = |b: bool| if b { "1" } else { "0" }.to_string();
    if let Some(k) = &body.send_key {
        if !SEND_KEYS.contains(&k.as_str()) {
            return Err("bad_send_key");
        }
        out.push(("send_key", k.clone()));
    }
    if let Some(p) = &body.start_page {
        if !START_PAGES.contains(&p.as_str()) {
            return Err("bad_start_page");
        }
        out.push(("start_page", p.clone()));
    }
    if let Some(b) = body.confirm_delete {
        out.push(("confirm_delete", flag(b)));
    }
    if let Some(b) = body.confirm_rewind {
        out.push(("confirm_rewind", flag(b)));
    }
    if let Some(s) = body.undo_secs {
        if !UNDO_SECS.contains(&s) {
            return Err("bad_undo_secs");
        }
        out.push(("undo_secs", s.to_string()));
    }
    if let Some(r) = &body.backup_rhythm {
        if Rhythm::parse(r).is_none() {
            return Err("bad_rhythm");
        }
        out.push((backup::RHYTHM_KEY, r.clone()));
    }
    if out.is_empty() {
        return Err("bad_settings");
    }
    Ok(out)
}

/// `PUT /general` with any of `{send_key, start_page, confirm_delete, confirm_rewind, undo_secs,
/// backup_rhythm}`: stores them and answers like `GET`. 400 `bad_send_key`, `bad_start_page`,
/// `bad_undo_secs`, `bad_rhythm`, `bad_settings` (nothing to change); then nothing is stored.
pub(super) async fn put_general(State(st): State<AppState>, via: Option<Extension<ViaLan>>, Json(body): Json<GeneralBody>) -> ApiResult {
    let changes = check(&body).map_err(ApiError::bad_request)?;
    st.hub.store(|s| -> crate::store::Res<()> {
        for (key, value) in &changes {
            s.set_general(key, value)?;
        }
        Ok(())
    })?;
    let what: serde_json::Map<String, Value> = changes.into_iter().map(|(k, v)| (k.to_string(), Value::String(v))).collect();
    st.log().write(Entry::new(Level::Info, "server", "settings", "general settings changed").data(Value::Object(what)));
    st.hub.send_event(&json!({ "type": "general" }));
    Ok(Json(snapshot(&st, via.is_none()).await?).into_response())
}

#[derive(Deserialize)]
pub(super) struct AutostartBody {
    on: bool,
    #[serde(default)]
    minimized: bool,
}

/// `PUT /general/autostart` `{on, minimized}`: starts the app with Windows (or no longer).
/// Answers like `GET /general`. 403 `pc_only` for a phone, 501 `unsupported` where the app cannot
/// start with Windows.
pub(super) async fn put_autostart(State(st): State<AppState>, via: Option<Extension<ViaLan>>, Json(body): Json<AutostartBody>) -> ApiResult {
    pc_only(&via)?;
    if tokio::task::spawn_blocking(autostart::state).await?.is_none() {
        return Err(ApiError::new(StatusCode::NOT_IMPLEMENTED, "unsupported"));
    }
    let want = autostart::State { on: body.on, minimized: body.on && body.minimized };
    tokio::task::spawn_blocking(move || autostart::set(want)).await?.map_err(ApiError::storage)?;
    st.log().write(Entry::new(Level::Info, "server", "settings", "start with Windows set").data(json!({ "on": want.on, "minimized": want.minimized })));
    st.hub.send_event(&json!({ "type": "general" }));
    Ok(Json(snapshot(&st, true).await?).into_response())
}

/// `POST /general/open-data-folder`: shows the data folder in Windows Explorer. 204; 403
/// `pc_only` for a phone, 501 `unsupported` on other systems.
pub(super) async fn open_data_folder(State(st): State<AppState>, via: Option<Extension<ViaLan>>) -> ApiResult {
    pc_only(&via)?;
    open_folder(&st.data_dir)?;
    st.log().write(Entry::new(Level::Info, "server", "settings", "data folder opened"));
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(windows)]
fn open_folder(dir: &std::path::Path) -> Result<(), ApiError> {
    std::process::Command::new("explorer.exe").arg(dir).spawn().map(drop).map_err(ApiError::storage)
}

#[cfg(not(windows))]
fn open_folder(_: &std::path::Path) -> Result<(), ApiError> {
    Err(ApiError::new(StatusCode::NOT_IMPLEMENTED, "unsupported"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> GeneralBody {
        GeneralBody { send_key: None, start_page: None, confirm_delete: None, confirm_rewind: None, undo_secs: None, backup_rhythm: None }
    }

    #[test]
    fn general_checks() {
        assert_eq!(check(&body()), Err("bad_settings"));
        assert_eq!(check(&GeneralBody { send_key: Some("ctrl_enter".into()), ..body() }), Ok(vec![("send_key", "ctrl_enter".into())]));
        assert_eq!(check(&GeneralBody { send_key: Some("tab".into()), ..body() }), Err("bad_send_key"));
        assert_eq!(check(&GeneralBody { start_page: Some("last_chat".into()), ..body() }), Ok(vec![("start_page", "last_chat".into())]));
        assert_eq!(check(&GeneralBody { start_page: Some("logs".into()), ..body() }), Err("bad_start_page"));
        assert_eq!(
            check(&GeneralBody { confirm_delete: Some(true), confirm_rewind: Some(false), ..body() }),
            Ok(vec![("confirm_delete", "1".into()), ("confirm_rewind", "0".into())])
        );
        assert_eq!(check(&GeneralBody { undo_secs: Some(15), ..body() }), Ok(vec![("undo_secs", "15".into())]));
        assert_eq!(check(&GeneralBody { undo_secs: Some(7), ..body() }), Err("bad_undo_secs"));
        assert_eq!(check(&GeneralBody { backup_rhythm: Some("weekly".into()), ..body() }), Ok(vec![(backup::RHYTHM_KEY, "weekly".into())]));
        assert_eq!(check(&GeneralBody { backup_rhythm: Some("hourly".into()), ..body() }), Err("bad_rhythm"));
        // One bad value refuses the whole change.
        assert_eq!(check(&GeneralBody { send_key: Some("enter".into()), undo_secs: Some(1), ..body() }), Err("bad_undo_secs"));
    }
}
