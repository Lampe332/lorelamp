//! `GET /engine`: is the model server there, which model runs (file, name, architecture, quant,
//! parameters, MoE/SWA, context, build), from which profile, and the last measured speed.
//! `POST /engine/restart`: new start attempts after the supervisor gave up.
//! `POST /engine/unload` + `/engine/load` and `PUT /engine/idle`: unloading the model to free the
//! graphics memory (see `engine::idle`).

use std::path::PathBuf;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use rp_core::llama_args::ctx_from_args;
use serde::Deserialize;
use serde_json::json;

use crate::engine::idle::IDLE_CHOICES;
use crate::engine::DEFAULT_ENGINE_PORT;
use crate::engine::model::{self, ModelInfo};
use crate::error::{ApiError, ApiResult};
use crate::logs::{Entry, Level};
use crate::store;
use crate::AppState;

pub(super) async fn get_engine(State(st): State<AppState>) -> ApiResult {
    let status = st.hub.engine_status().await;
    // A note only matters while the model is not there.
    let note = if status == "ready" { None } else { st.engine_note.lock().unwrap_or_else(|p| p.into_inner()).clone() };
    let props = if status == "ready" { st.hub.props().await } else { None };
    let (props_model, props_ctx, props_build) = props.as_ref().map(model::from_props).unwrap_or_default();

    let (active, stats, spec, engines, idle_min, port) = st.hub.store(|s| -> store::Res<_> {
        Ok((s.active_profile()?, s.stats()?, s.launch_spec()?, s.engines()?, s.idle_unload_min()?, s.engine_port()?))
    })?;
    let running = st.engine.as_ref().and_then(|e| e.running());
    // What runs: what llama-server says, else what our supervisor started, else what the active
    // profile would start.
    let launch = running.or(spec);
    let file: Option<PathBuf> = props_model.or_else(|| launch.as_ref().map(|l| l.model.clone()));
    let mut info = match file {
        Some(path) => {
            let p = path.clone();
            let gguf = tokio::task::spawn_blocking(move || model::gguf_cached(&p)).await?;
            model::summarize(&path, gguf.as_deref())
        }
        None => ModelInfo::default(),
    };
    info.ctx = props_ctx.or_else(|| launch.as_ref().and_then(|l| ctx_from_args(&l.args))).or(info.ctx_train);
    let engine_version = active.as_ref().and_then(|p| engines.iter().find(|e| e.id == p.engine_id)).and_then(|e| e.version.clone());
    info.build = props_build.or_else(|| st.engine.as_ref().and_then(|e| e.version())).or(engine_version);

    Ok(Json(json!({
        "status": status,
        "note": note,
        "model": info,
        "last_timings": stats.last_timings,
        "profile": active.map(|p| json!({ "id": p.id, "name": p.name, "measured": p.measured })),
        // A benchmark holds the model server (it restarts with test settings): the profile measured.
        "bench_profile_id": st.hub.bench_run().map(|r| r.profile_id),
        "managed": st.engine.is_some(),
        "dropped_args": st.engine.as_ref().map(|e| e.dropped_args()).unwrap_or_default(),
        "unloaded": st.engine.is_some() && st.hub.unloaded(),
        "idle_unload_min": idle_min,
        "pid": st.engine.as_ref().and_then(|e| e.pid()),
        // The port the managed model server uses (setting `engine_port`, `PUT /settings`).
        "port": port.unwrap_or(DEFAULT_ENGINE_PORT),
        // Why no model server runs: `{code:"port_busy",port}` (another program holds the port)
        // or `{code:"gave_up",crashes}` (no further start until `POST /engine/restart`).
        "problem": if status == "ready" { None } else { st.engine.as_ref().and_then(|e| e.problem()) },
    }))
    .into_response())
}

/// `POST /engine/restart`: after the supervisor gave up (too many crashes), one new series of
/// start attempts; also loads an unloaded model. `restarting`: something will start.
pub(super) async fn post_restart(State(st): State<AppState>) -> ApiResult {
    managed(&st)?;
    let again = st.engine.as_ref().is_some_and(|e| e.retry());
    let loading = st.hub.load();
    st.log().write(Entry::new(Level::Info, "engine", "retry", "model server restart asked for").data(json!({ "gave_up": again, "unloaded": loading })));
    Ok(Json(json!({ "restarting": again || loading })).into_response())
}

/// Only a server that manages the model server can unload or load it (409 `unmanaged`).
fn managed(st: &AppState) -> Result<(), ApiError> {
    match &st.engine {
        Some(_) => Ok(()),
        None => Err(ApiError::conflict("unmanaged")),
    }
}

/// `POST /engine/unload`: stop the model server to free the graphics memory; the next message
/// (or `POST /engine/load`) starts it again. 409 `busy` while a reply is being written.
pub(super) async fn post_unload(State(st): State<AppState>) -> ApiResult {
    managed(&st)?;
    st.hub.unload().map_err(ApiError::conflict)?;
    st.log().write(Entry::new(Level::Info, "engine", "unload", "model unloaded by hand"));
    Ok(Json(json!({ "unloaded": true })).into_response())
}

/// `POST /engine/load`: start the model again after an unload. `loading`: it was unloaded.
pub(super) async fn post_load(State(st): State<AppState>) -> ApiResult {
    managed(&st)?;
    let loading = st.hub.load();
    if loading {
        st.log().write(Entry::new(Level::Info, "engine", "load", "model loaded again by hand"));
    }
    Ok(Json(json!({ "loading": loading })).into_response())
}

#[derive(Deserialize)]
pub(super) struct IdleBody {
    minutes: u32,
}

/// `PUT /engine/idle` `{minutes}`: unload the model after this many quiet minutes (one of
/// 0 = off, 5, 15, 30, 60; else 400 `bad_idle`).
pub(super) async fn put_idle(State(st): State<AppState>, Json(body): Json<IdleBody>) -> ApiResult {
    if !IDLE_CHOICES.contains(&body.minutes) {
        return Err(ApiError::bad_request("bad_idle"));
    }
    st.hub.store(|s| s.set_idle_unload_min(body.minutes))?;
    st.log().write(Entry::new(Level::Info, "engine", "config", "idle unload set").data(json!({ "idle_unload_min": body.minutes })));
    st.hub.models_changed();
    Ok(Json(json!({ "idle_unload_min": body.minutes })).into_response())
}
