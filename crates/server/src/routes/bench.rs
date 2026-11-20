//! The benchmark of a model profile (`crate::bench`): start, cancel, read the last result, and
//! apply one of its three recommendations to the profile.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use rp_core::bench::{self, Candidate, Purpose, Row};
use rp_core::llama_args;
use serde::Deserialize;
use serde_json::json;

use super::models::log_model;
use crate::bench::{Job, Params};
use crate::error::{ApiError, ApiResult};
use crate::store::{self, ProfileEdit};
use crate::AppState;

/// `POST /models/profiles/{id}/benchmark`: starts the benchmark (202 `{started: true}`); progress
/// comes over the socket (`{"type":"bench",…}`). 409 `busy` while a reply is being written,
/// `benchmark` while one runs, `unmanaged` / `other_window` / `foreign` when this app does not run
/// the model server itself, `port_busy` when another program holds its port; 404 `profile_missing`, `model_missing`; 422 `unreadable`.
pub(super) async fn post_benchmark(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let engine = match &st.engine {
        None => return Err(ApiError::conflict("unmanaged")),
        Some(e) if !e.managing() => return Err(ApiError::conflict("other_window")),
        Some(e) if e.foreign() => return Err(ApiError::conflict("foreign")),
        Some(e) if matches!(e.problem(), Some(crate::engine::Problem::PortBusy { .. })) => return Err(ApiError::conflict("port_busy")),
        Some(e) => e.clone(),
    };
    let found = st.hub.store(|s| -> store::Res<_> {
        let Some(p) = s.profile(id)? else { return Ok(None) };
        let backend = s.engine(p.engine_id)?.map(|e| e.backend).unwrap_or_default();
        Ok(s.profile_spec(id)?.map(|spec| (spec, backend, p.bench_stamp())))
    })?;
    let (spec, backend, measured_with) = found.ok_or_else(|| ApiError::not_found("profile_missing"))?;
    // Whether the model computes on a GPU: a real one, a GPU build, layers on it.
    let gpu = crate::system::computing_gpu(crate::system::main_gpu(), &backend, &spec.args);
    let model = spec.model.clone();
    let info = tokio::task::spawn_blocking(move || model.is_file().then(|| crate::engine::model::gguf_cached(&model))).await?;
    let info = match info {
        None => return Err(ApiError::not_found("model_missing")),
        Some(None) => return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "unreadable")),
        Some(Some(info)) => info,
    };
    st.hub.bench_begin(id).map_err(ApiError::conflict)?;
    st.bench.begin();
    log_model(&st, "benchmark requested", json!({ "profile_id": id }));
    let job = Job {
        hub: st.hub.clone(),
        engine,
        status: st.bench.clone(),
        data_dir: st.data_dir.to_path_buf(),
        profile_id: id,
        spec,
        info,
        params: Params::from_env(gpu.is_some()),
        gpu,
        measured_with,
    };
    tokio::spawn(crate::bench::run(job));
    Ok((StatusCode::ACCEPTED, Json(json!({ "started": true }))).into_response())
}

/// `GET /models/profiles/{id}/benchmark`: `{running, result, results}`: the newest progress event
/// while a benchmark of this profile runs (else null), the last stored result (null: never
/// measured), and every kept result, newest first, one per kind of memory `conditions` (a run with
/// the apps open next to one with them closed).
pub(super) async fn get_benchmark(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let running = st.hub.bench_run().filter(|r| r.profile_id == id).map(|_| st.bench.last().unwrap_or(json!({ "type": "bench", "profile_id": id, "stage": "loading", "done": 0, "total": 0, "rows": [] })));
    let (result, results) = st.hub.store(|s| -> store::Res<_> { Ok((s.bench_result(id)?, s.bench_results(id)?)) })?;
    Ok(Json(json!({ "running": running, "result": result, "results": results })).into_response())
}

/// `POST /models/profiles/{id}/benchmark/cancel`: 204; the benchmark stops after the step it is in
/// and the active profile runs again. 409 `not_running`.
pub(super) async fn post_cancel(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    if st.hub.bench_run().is_none_or(|r| r.profile_id != id) {
        return Err(ApiError::conflict("not_running"));
    }
    st.bench.cancel();
    log_model(&st, "benchmark cancelled", json!({ "profile_id": id }));
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub(super) struct ApplyBody {
    purpose: String,
}

/// `POST /models/profiles/{id}/benchmark/apply` `{purpose: "speed"|"balanced"|"context"}`: writes
/// that recommendation's context, KV cache type and `--n-cpu-moe` into the profile's start options
/// (the others stay) and marks it measured. `{profile, engine}`; `engine` is `restarting` for the
/// active profile, else `saved`. 400 `bad_purpose`, 404 `no_result` (nothing measured for that
/// purpose), 409 `benchmark` while one runs, `busy` while a reply is being written (active profile).
pub(super) async fn post_apply(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<ApplyBody>) -> ApiResult {
    let purpose = Purpose::parse(&body.purpose).ok_or_else(|| ApiError::bad_request("bad_purpose"))?;
    if st.hub.bench_run().is_some() {
        return Err(ApiError::conflict("benchmark"));
    }
    let busy = st.hub.generating();
    let outcome = st.hub.store(|s| -> store::Res<Result<(store::Profile, Candidate), ApiError>> {
        let Some(p) = s.profile(id)? else { return Ok(Err(ApiError::not_found("profile_missing"))) };
        let Some(c) = s.bench_result(id)?.as_ref().and_then(|r| pick(r, purpose)) else { return Ok(Err(ApiError::not_found("no_result"))) };
        if p.active && busy {
            return Ok(Err(ApiError::conflict("busy")));
        }
        // Through the same allowlist as an edit (a profile saved before it loses what it refuses).
        let args = llama_args::allowed_args(&bench::args_with(&p.load_args, &c)).args;
        let edit = ProfileEdit { load_args: Some(args), measured: Some(true), ..ProfileEdit::default() };
        Ok(s.update_profile(id, &edit)?.map(|p| (p, c)).ok_or_else(|| ApiError::not_found("profile_missing")))
    });
    let (profile, c) = outcome??;
    let engine = if profile.active {
        if let Some(e) = &st.engine {
            e.wake();
        }
        "restarting"
    } else {
        "saved"
    };
    log_model(&st, "benchmark result applied", json!({ "profile_id": id, "purpose": body.purpose, "ctx": c.ctx, "kv": c.kv, "n_cpu_moe": c.n_cpu_moe }));
    st.hub.models_changed();
    Ok(Json(json!({ "profile": profile, "engine": engine })).into_response())
}

/// The candidate a stored result recommends for `purpose`.
fn pick(result: &serde_json::Value, purpose: Purpose) -> Option<Candidate> {
    let rows: Vec<Row> = serde_json::from_value(result["rows"].clone()).ok()?;
    let picks: Vec<bench::Pick> = serde_json::from_value(result["picks"].clone()).ok()?;
    let p = picks.into_iter().find(|p| p.purpose == purpose)?;
    rows.get(p.row).map(Row::candidate)
}
