//! Backups of the database over HTTP: the list with sizes, "back up now", the folder's limit,
//! deleting one or several and restoring one (see `crate::backup`). Every outcome goes into the activity log with names and
//! sizes only.

use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::backup::{self, Reason};
use crate::error::{ApiError, ApiResult};
use crate::logs::{Entry, Level};
use crate::AppState;

fn refused(code: &'static str) -> ApiError {
    match code {
        "bad_name" | "bad_backup" => ApiError::bad_request(code),
        "backup_missing" => ApiError::not_found(code),
        "busy" | "backup_newer" | "backup_limit" => ApiError::conflict(code),
        _ => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "storage"),
    }
}

/// `{backups, total_bytes, limit_bytes, near_full, db_bytes, disk_free}` (see `backup::Overview`).
pub(super) async fn get_backups(State(st): State<AppState>) -> ApiResult {
    let dir = st.data_dir.to_path_buf();
    let limit = st.hub.store(|s| s.backup_limit());
    let overview = tokio::task::spawn_blocking(move || backup::overview(&dir, limit)).await??;
    Ok(Json(overview).into_response())
}

#[derive(Deserialize)]
pub(super) struct LimitBody {
    limit_bytes: u64,
}

/// `PUT /backups/limit` `{limit_bytes}`: the backups folder's limit (one of
/// `backup::LIMIT_CHOICES`, else 400 `bad_limit`). A smaller limit prunes right away (oldest
/// automatic ones first, manual ones never). Answers `{limit_bytes, pruned}`.
pub(super) async fn put_limit(State(st): State<AppState>, Json(body): Json<LimitBody>) -> ApiResult {
    if !backup::LIMIT_CHOICES.contains(&body.limit_bytes) {
        return Err(ApiError::bad_request("bad_limit"));
    }
    st.hub.store(|s| s.set_backup_limit(body.limit_bytes))?;
    let dir = st.data_dir.to_path_buf();
    let limit = body.limit_bytes;
    let pruned = tokio::task::spawn_blocking(move || backup::apply_limit(&dir, limit)).await??;
    st.log().write(Entry::new(Level::Info, "server", "backup", "backup limit set").data(json!({ "limit_bytes": limit, "pruned": pruned })));
    Ok(Json(json!({ "limit_bytes": limit, "pruned": pruned })).into_response())
}

/// Most days `older_than_days` takes (ten years).
const MAX_DAYS: u32 = 3650;

#[derive(Deserialize)]
pub(super) struct DeleteBody {
    names: Option<Vec<String>>,
    older_than_days: Option<u32>,
}

/// `POST /backups/delete` with exactly one of `{names: [..]}` (those backups of the backups
/// folder, also manual ones; all or none) or `{older_than_days: N}` (1–3650: every automatic
/// backup made more than N days ago; manual ones stay). Answers `{deleted: [names]}`.
/// 400 `bad_delete` (neither or both, an empty list, N out of range), `bad_name`; 404
/// `backup_missing`.
pub(super) async fn delete_many(State(st): State<AppState>, Json(body): Json<DeleteBody>) -> ApiResult {
    let dir = st.data_dir.to_path_buf();
    let gone = match (body.names, body.older_than_days) {
        (Some(names), None) if !names.is_empty() => match tokio::task::spawn_blocking(move || backup::remove_many(&dir, &names)).await? {
            Ok(gone) => gone,
            Err(code) => {
                st.log().write(Entry::new(Level::Warn, "server", "backup", format!("delete refused: {code}")));
                return Err(refused(code));
            }
        },
        (None, Some(days)) if (1..=MAX_DAYS).contains(&days) => {
            tokio::task::spawn_blocking(move || backup::remove_older(&dir, days, crate::logs::now_ms())).await??
        }
        _ => return Err(ApiError::bad_request("bad_delete")),
    };
    let bytes: u64 = gone.iter().map(|b| b.size).sum();
    let names: Vec<&str> = gone.iter().map(|b| b.name.as_str()).collect();
    st.log().write(Entry::new(Level::Info, "server", "backup", "backups deleted").data(json!({ "names": names, "count": names.len(), "bytes": bytes })));
    Ok(Json(json!({ "deleted": names })).into_response())
}

/// Deletes one backup of the backups folder by its name from the list (also a manual one; never
/// a copy made by hand next to the database). Answers `{deleted: name}`.
pub(super) async fn delete_backup(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult {
    let dir = st.data_dir.to_path_buf();
    let target = name.clone();
    match tokio::task::spawn_blocking(move || backup::remove(&dir, &target)).await? {
        Ok(gone) => {
            st.log().write(Entry::new(Level::Info, "server", "backup", "backup deleted").data(json!({
                "name": gone.name, "reason": gone.reason, "bytes": gone.size,
            })));
            Ok(Json(json!({ "deleted": gone.name })).into_response())
        }
        Err(code) => {
            st.log().write(Entry::new(Level::Warn, "server", "backup", format!("delete refused: {code}")));
            Err(refused(code))
        }
    }
}

/// Backs up now (`manual`; never pruned). Answers the new backup. 409 `backup_limit` when it would
/// not fit into the backups folder's limit next to the other manual ones.
pub(super) async fn post_backup(State(st): State<AppState>) -> ApiResult {
    let dir = st.data_dir.to_path_buf();
    let started = Instant::now();
    match tokio::task::spawn_blocking(move || backup::create(&dir, Reason::Manual)).await? {
        Ok(made) => {
            backup::log_made(st.log(), &made, started.elapsed().as_millis());
            Ok((StatusCode::CREATED, Json(made.0)).into_response())
        }
        Err(e) if e.downcast_ref::<backup::LimitReached>().is_some() => {
            st.log().write(Entry::new(Level::Warn, "server", "backup", "backup refused: backup_limit"));
            Err(refused("backup_limit"))
        }
        Err(e) => {
            st.log().write(Entry::new(Level::Error, "server", "backup", format!("backup failed: {e}")));
            Err(ApiError::storage(e))
        }
    }
}

/// Restores a backup by its name from the list; the current state is backed up first
/// (`before-restore`). 409 `busy` while the model writes an answer.
pub(super) async fn restore_backup(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult {
    let (path, reason) = backup::locate(&st.data_dir, &name).map_err(|code| {
        st.log().write(Entry::new(Level::Warn, "server", "backup", format!("restore refused: {code}")));
        refused(code)
    })?;
    let hub = st.hub.clone();
    let started = Instant::now();
    match tokio::task::spawn_blocking(move || hub.restore_backup(&path)).await? {
        Ok(safety) => {
            let ms = started.elapsed().as_millis();
            backup::log_made(st.log(), &(safety.clone(), 0), ms);
            st.log().write(Entry::new(Level::Info, "server", "backup", "backup restored").data(json!({
                "name": name, "reason": reason, "safety": safety.name, "ms": ms as u64,
            })));
            st.hub.library_changed();
            Ok(Json(json!({ "restored": name, "safety": safety })).into_response())
        }
        Err(code) => {
            st.log().write(Entry::new(Level::Warn, "server", "backup", format!("restore refused: {code}")).data(json!({ "name": name })));
            Err(refused(code))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manual backup over the limit is a conflict (409 `backup_limit`), a bad name a 400.
    #[test]
    fn refusals_have_their_status() {
        for (code, status) in [
            ("backup_limit", StatusCode::CONFLICT),
            ("busy", StatusCode::CONFLICT),
            ("bad_name", StatusCode::BAD_REQUEST),
            ("backup_missing", StatusCode::NOT_FOUND),
            ("storage", StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            assert_eq!(refused(code).into_response().status(), status, "{code}");
        }
    }
}
