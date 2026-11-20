//! "PC prüfen" and official engines: the hardware scan (kept in `settings`), the llama.cpp
//! release list from GitHub, the download of one build (progress over the socket as
//! `{"type":"engine_download",…}`) and removing a build the app installed.

use std::path::{Path as FsPath, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::install::{self, Phase, Release};
use crate::error::{ApiError, ApiResult};
use crate::logs::{Entry, Level};
use crate::store::{EngineDelete, NewEngine};
use crate::AppState;

/// The release list is asked for at most this often (GitHub allows 60 requests an hour without
/// a token).
const RELEASES_FRESH: Duration = Duration::from_secs(10 * 60);

fn log(st: &AppState, level: Level, what: &str, data: Value) {
    st.log().write(Entry::new(level, "server", "system", what).data(data));
}

/// `POST /system/scan`: reads the PC now, stores and answers the result.
pub(super) async fn post_scan(State(st): State<AppState>) -> ApiResult {
    let own = st.engine.as_ref().and_then(|e| e.pid());
    let scan = tokio::task::spawn_blocking(move || crate::system::scan(own)).await?;
    let scan = match scan {
        Ok(v) => v,
        Err(hw::Error::Unsupported) => return Err(ApiError::new(StatusCode::NOT_IMPLEMENTED, "scan_unsupported")),
        Err(e) => {
            log(&st, Level::Error, "PC scan failed", json!({ "error": e.to_string() }));
            return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "scan_failed"));
        }
    };
    st.hub.store(|s| s.set_system_scan(&scan))?;
    let gpu = scan["backend"]["gpu_index"].as_u64().map(|i| &scan["gpus"][i as usize]);
    log(
        &st,
        Level::Info,
        "PC scanned",
        json!({
            "gpus": scan["gpus"].as_array().map_or(0, Vec::len),
            "gpu": gpu.map(|g| g["name"].clone()),
            "driver": gpu.map(|g| g["driver"].clone()),
            "driver_issue": gpu.map(|g| g["driver_issue"].clone()),
            "backend": scan["backend"]["suggested"],
            "reason": scan["backend"]["reason_code"],
            "rocm_runtime": scan["backend"]["rocm_runtime_found"],
        }),
    );
    Ok(Json(scan).into_response())
}

/// `GET /system/scan`: `{scan}`, the last stored scan or null.
pub(super) async fn get_scan(State(st): State<AppState>) -> ApiResult {
    let mut scan = st.hub.store(|s| s.system_scan())?;
    if let Some(scan) = scan.as_mut() {
        crate::system::upgrade(scan);
    }
    Ok(Json(json!({ "scan": scan })).into_response())
}

type Cached = Mutex<Option<(Instant, Vec<Release>)>>;

fn cache() -> &'static Cached {
    static C: OnceLock<Cached> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Where the release list (and with it every engine download and its SHA-256) comes from: GitHub. A
/// test server (a debug build, `test_hooks`) reads it from a local stand-in in `RP_GITHUB_API`; a
/// release build never reads that variable, so nothing set on the PC can swap the source.
fn github_api(env: Option<String>, test_hooks: bool) -> String {
    env.filter(|_| test_hooks).unwrap_or_else(|| install::GITHUB_API.into())
}

async fn releases(fresh: bool) -> Result<Vec<Release>, ApiError> {
    if !fresh {
        if let Some((at, r)) = cache().lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            if at.elapsed() < RELEASES_FRESH {
                return Ok(r.clone());
            }
        }
    }
    let client = rp_models::download::http_client();
    let api = github_api(std::env::var("RP_GITHUB_API").ok(), cfg!(debug_assertions));
    let list = install::fetch_releases(&client, api.trim_end_matches('/')).await.map_err(|e| {
        eprintln!("release list: {e}");
        ApiError::new(StatusCode::BAD_GATEWAY, "github_unreachable")
    })?;
    *cache().lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), list.clone()));
    Ok(list)
}

/// `GET /system/engine-releases`: `{releases: [{tag, published_at, assets: [{name, variant,
/// backend, size, sha256, url, runtime}]}], arch}`; 502 `github_unreachable`.
pub(super) async fn get_engine_releases() -> ApiResult {
    let list = releases(false).await?;
    Ok(Json(json!({ "releases": list, "arch": install::arch() })).into_response())
}

/// `GET /system/engine-download`: `{job}`, the last download event (null: none since the start).
pub(super) async fn get_engine_download(State(st): State<AppState>) -> ApiResult {
    let job = st.engine_job.lock().unwrap_or_else(|p| p.into_inner()).clone();
    Ok(Json(json!({ "job": job })).into_response())
}

#[derive(Deserialize)]
pub(super) struct DownloadBody {
    tag: String,
    asset: String,
}

fn engines_dir(st: &AppState) -> PathBuf {
    st.data_dir.join("engines")
}

/// Sets the job's state and tells every device.
fn report(st: &AppState, job: Value) {
    *st.engine_job.lock().unwrap_or_else(|p| p.into_inner()) = Some(job.clone());
    st.hub.send_event(&job);
}

/// `POST /models/engines/download {tag, asset}`: 202 `{job}`, then the download runs in the
/// background. 409 `download_running`, `engine_exists`, `rocm_runtime_missing`; 404
/// `release_missing` (not among the listed releases or assets).
pub(super) async fn post_engine_download(State(st): State<AppState>, Json(body): Json<DownloadBody>) -> ApiResult {
    let running = |j: &Option<Value>| j.as_ref().is_some_and(|j| !matches!(j["phase"].as_str(), Some("done" | "failed")));
    if running(&st.engine_job.lock().unwrap_or_else(|p| p.into_inner())) {
        return Err(ApiError::conflict("download_running"));
    }
    let list = releases(false).await?;
    let asset = list
        .iter()
        .find(|r| r.tag == body.tag)
        .and_then(|r| r.assets.iter().find(|a| a.name == body.asset))
        .cloned()
        .ok_or_else(|| ApiError::not_found("release_missing"))?;
    let tag = body.tag;
    let dir = install::target_dir(&engines_dir(&st), &tag, &asset);
    let taken = st.hub.store(|s| s.engines())?.iter().any(|e| FsPath::new(&e.exe).starts_with(&dir));
    if taken || dir.exists() {
        return Err(ApiError::conflict("engine_exists"));
    }
    let rocm = if asset.backend == "rocm" {
        let found = tokio::task::spawn_blocking(hw::rocm_runtime).await?.ok().flatten();
        Some(found.ok_or_else(|| ApiError::conflict("rocm_runtime_missing"))?)
    } else {
        None
    };
    let base = json!({ "type": "engine_download", "tag": tag, "asset": asset.name, "variant": asset.variant, "backend": asset.backend });
    let with = move |extra: Value| {
        let mut v = base.clone();
        if let (Some(v), Value::Object(extra)) = (v.as_object_mut(), extra) {
            v.extend(extra);
        }
        v
    };
    // Claim the job before anything awaits, so a second request sees it.
    {
        let mut job = st.engine_job.lock().unwrap_or_else(|p| p.into_inner());
        if running(&job) {
            return Err(ApiError::conflict("download_running"));
        }
        *job = Some(with(json!({ "phase": "download", "downloaded": 0, "total": asset.total_size() })));
    }
    let first = with(json!({ "phase": "download", "downloaded": 0, "total": asset.total_size() }));
    st.hub.send_event(&first);
    log(&st, Level::Info, "engine download started", json!({ "tag": tag, "asset": asset.name, "bytes": asset.total_size() }));

    let (task_st, task_with) = (st.clone(), with.clone());
    tokio::spawn(async move {
        let (st, with) = (task_st, task_with);
        let client = rp_models::download::http_client();
        let started = Instant::now();
        let progress = |s: install::Step| {
            let phase = match s.phase {
                Phase::Download => "download",
                Phase::Unpack => "unpack",
                Phase::Test => "test",
            };
            report(&st, with(json!({ "phase": phase, "downloaded": s.done, "total": s.total })));
        };
        let result = install::install(&client, &engines_dir(&st), &tag, &asset, rocm.as_deref(), progress).await;
        let outcome = result.and_then(|got| {
            let name = match asset.backend.as_str() {
                "cuda" => format!("llama.cpp {tag} (CUDA {})", asset.variant.trim_start_matches("cuda-")),
                _ => format!("llama.cpp {tag}"),
            };
            let exe = got.exe.display().to_string();
            let extra: Vec<String> = got.extra_path.iter().map(|p| p.display().to_string()).collect();
            let selftest = serde_json::to_value(&got.selftest).unwrap_or_default();
            let added = st.hub.store(|s| {
                s.add_official_engine(
                    &NewEngine { name: &name, exe: &exe, extra_path: &extra, backend: &asset.backend, version: got.selftest.version.as_deref() },
                    &selftest,
                )
            });
            match added {
                Ok(id) => Ok((id, got.selftest)),
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&got.dir);
                    Err(install::Failure { code: "storage", detail: e.to_string() })
                }
            }
        });
        match outcome {
            Ok((id, selftest)) => {
                log(
                    &st,
                    Level::Info,
                    "engine installed",
                    json!({ "engine_id": id, "tag": tag, "backend": asset.backend, "devices": selftest.devices.len(), "ran": selftest.ran, "ms": started.elapsed().as_millis() as u64 }),
                );
                report(&st, with(json!({ "phase": "done", "engine_id": id, "selftest": selftest, "downloaded": asset.total_size(), "total": asset.total_size() })));
                st.hub.models_changed();
            }
            Err(f) => {
                log(&st, Level::Warn, "engine install failed", json!({ "tag": tag, "asset": asset.name, "error": f.code, "detail": f.detail }));
                report(&st, with(json!({ "phase": "failed", "error": f.code })));
            }
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "job": first }))).into_response())
}

/// `DELETE /models/engines/{id}`: 204, the build's folder goes too. 409 `engine_own` (a build of
/// the user, never removed) or `engine_in_use` (a profile uses it).
pub(super) async fn delete_engine(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    match st.hub.store(|s| s.delete_engine(id))? {
        EngineDelete::Missing => Err(ApiError::not_found("engine_missing")),
        EngineDelete::Own => Err(ApiError::conflict("engine_own")),
        EngineDelete::InUse => Err(ApiError::conflict("engine_in_use")),
        EngineDelete::Done(exe) => {
            // Only a folder the app made itself, directly below `<data>/engines`.
            let root = engines_dir(&st);
            let dir = FsPath::new(&exe).ancestors().find(|a| a.parent() == Some(root.as_path())).map(FsPath::to_path_buf);
            if let Some(dir) = dir {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    log(&st, Level::Warn, "engine folder not removed", json!({ "engine_id": id, "error": e.to_string() }));
                }
            }
            log(&st, Level::Info, "engine removed", json!({ "engine_id": id }));
            st.hub.models_changed();
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N6: the test stand-in only in a debug build; a release build always asks GitHub.
    #[test]
    fn release_list_source() {
        let stand_in = Some("http://127.0.0.1:7879".to_string());
        assert_eq!(github_api(stand_in.clone(), true), "http://127.0.0.1:7879");
        assert_eq!(github_api(stand_in, false), install::GITHUB_API);
        assert_eq!(github_api(None, true), install::GITHUB_API);
    }
}
