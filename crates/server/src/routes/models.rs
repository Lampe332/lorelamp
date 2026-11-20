//! Model profiles and engines: list, create, edit, delete, and activate a profile (the
//! supervisor then restarts the model server with its arguments; sampler changes apply from the
//! next message without a restart).

use std::path::Path as FsPath;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use rp_core::llama_args;
use serde_json::json;

use crate::error::{ApiError, ApiResult};
use crate::guard::pc_only;
use crate::lan::ViaLan;
use crate::logs::{Entry, Level};
use crate::store::{self, ProfileDelete, ProfileEdit};
use crate::AppState;

/// Arguments the app sets itself; a profile must not carry them.
const MANAGED_FLAGS: [&str; 6] = ["-m", "--model", "--host", "--port", "-hf", "--hf-repo"];
const MAX_ARGS: usize = 200;
const MAX_ARG_CHARS: usize = 4096;
pub(super) const MAX_PATH_CHARS: usize = 1024;

/// Profile events go to the log with ids only, never names or paths typed by the user.
pub(super) fn log_model(st: &AppState, what: &str, data: serde_json::Value) {
    st.log().write(Entry::new(Level::Info, "server", "model", what).data(data));
}

/// The edit with name and file trimmed, or the error code for a bad request. `new`: name,
/// engine and model file are required.
fn check_profile(mut body: ProfileEdit, new: bool) -> Result<ProfileEdit, &'static str> {
    body.name = body.name.map(|n| n.trim().to_string());
    body.model_file = body.model_file.map(|f| f.trim().to_string());
    if (new && body.name.is_none()) || body.name.as_ref().is_some_and(|n| n.is_empty() || n.chars().count() > store::PROFILE_NAME_MAX) {
        return Err("bad_name");
    }
    if new && body.engine_id.is_none() {
        return Err("bad_engine");
    }
    let bad_file = |f: &String| {
        f.is_empty() || f.chars().count() > MAX_PATH_CHARS || !f.to_ascii_lowercase().ends_with(".gguf") || !FsPath::new(f).is_file()
    };
    if (new && body.model_file.is_none()) || body.model_file.as_ref().is_some_and(bad_file) {
        return Err("bad_model_file");
    }
    if let Some(args) = &body.load_args {
        let managed = |a: &String| MANAGED_FLAGS.contains(&a.split('=').next().unwrap_or(a));
        if args.len() > MAX_ARGS || args.iter().any(|a| a.is_empty() || a.chars().count() > MAX_ARG_CHARS || a.contains('\0') || managed(a)) {
            return Err("bad_load_args");
        }
        // Only tuning options: no files written, nothing fetched, no other chat template.
        if !llama_args::allowed_args(args).dropped.is_empty() {
            return Err("arg_not_allowed");
        }
    }
    if body.sampler.as_ref().is_some_and(|s| s.check().is_err()) {
        return Err("bad_sampler");
    }
    if body.reasoning.as_deref().is_some_and(|r| r != "off" && r != "on") {
        return Err("bad_reasoning");
    }
    Ok(body)
}

fn profile_missing() -> ApiError {
    ApiError::not_found("profile_missing")
}

/// `GET /models/profiles`: `{profiles: [...], active_id}`.
pub(super) async fn get_profiles(State(st): State<AppState>) -> ApiResult {
    let (profiles, active) = st.hub.store(|s| -> store::Res<_> { Ok((s.profiles()?, s.active_profile()?.map(|p| p.id))) })?;
    Ok(Json(json!({ "profiles": profiles, "active_id": active })).into_response())
}

/// `POST /models/profiles`: a new, inactive profile.
pub(super) async fn post_profile(State(st): State<AppState>, Json(body): Json<ProfileEdit>) -> ApiResult {
    let body = check_profile(body, true).map_err(ApiError::bad_request)?;
    let profile = st.hub.store(|s| -> store::Res<_> {
        if s.engine(body.engine_id.unwrap_or_default())?.is_none() {
            return Ok(None);
        }
        s.add_profile(&body).map(Some)
    })?;
    let profile = profile.ok_or_else(|| ApiError::bad_request("bad_engine"))?;
    log_model(&st, "model profile created", json!({ "profile_id": profile.id, "engine_id": profile.engine_id }));
    st.hub.models_changed();
    Ok(Json(profile).into_response())
}

/// `PUT /models/profiles/{id}`: fields left out stay. Changing what the active profile starts
/// restarts the model server, so that is refused (409 `busy`) while a reply is being written.
/// A phone may change the name, sampler and thinking; model file, engine and start options only the
/// PC (403 `pc_only`). 400 `arg_not_allowed`: a start option outside `llama_args::allowed_args`.
pub(super) async fn put_profile(
    State(st): State<AppState>,
    via: Option<Extension<ViaLan>>,
    Path(id): Path<i64>,
    Json(body): Json<ProfileEdit>,
) -> ApiResult {
    if body.changes_launch() {
        pc_only(via.map(|Extension(v)| v))?;
    }
    let body = check_profile(body, false).map_err(ApiError::bad_request)?;
    // Asked before the store closure: `generating` takes the same lock.
    let busy = st.hub.generating();
    let outcome = st.hub.store(|s| -> store::Res<Result<store::Profile, ApiError>> {
        let Some(old) = s.profile(id)? else { return Ok(Err(profile_missing())) };
        if old.active && body.changes_launch() && busy {
            return Ok(Err(ApiError::conflict("busy")));
        }
        if body.engine_id.is_some_and(|e| e != old.engine_id) && s.engine(body.engine_id.unwrap_or_default())?.is_none() {
            return Ok(Err(ApiError::bad_request("bad_engine")));
        }
        Ok(s.update_profile(id, &body)?.ok_or_else(profile_missing))
    });
    let profile = outcome??;
    let restarts = profile.active && body.changes_launch();
    if restarts {
        if let Some(engine) = &st.engine {
            engine.wake();
        }
    }
    log_model(&st, "model profile changed", json!({ "profile_id": id, "launch_changed": body.changes_launch(), "active": profile.active }));
    st.hub.models_changed();
    Ok(Json(profile).into_response())
}

/// `DELETE /models/profiles/{id}`: 204; 409 `profile_active` for the active one.
pub(super) async fn delete_profile(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    match st.hub.store(|s| s.delete_profile(id))? {
        ProfileDelete::Missing => Err(profile_missing()),
        ProfileDelete::Active => Err(ApiError::conflict("profile_active")),
        ProfileDelete::Done => {
            log_model(&st, "model profile deleted", json!({ "profile_id": id }));
            st.hub.models_changed();
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}

/// `POST /models/profiles/{id}/activate`: `{profile, engine}`. `engine` says what happens to
/// the model server: `restarting` (ours, other arguments), `unchanged` (same arguments, only the
/// sampler differs), `unmanaged` (this server does not manage one, e.g. `rp-server` without
/// `--manage-engine`), `other_window` (another app window manages it), `port_busy` (another
/// program holds the port: nothing starts until it is free or changed), `foreign` (a server
/// started elsewhere holds the port). 409 `busy` while a reply is being written: stopping the
/// server then would cut that reply off.
pub(super) async fn activate_profile(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    if st.hub.generating() {
        return Err(ApiError::conflict("busy"));
    }
    let (profile, engine) = activate(&st, id)?;
    Ok(Json(json!({ "profile": profile, "engine": engine })).into_response())
}

/// Makes the profile the active one and wakes the supervisor; what that does to the model server
/// (see [`activate_profile`]). The caller checked that no reply is being written.
pub(super) fn activate(st: &AppState, id: i64) -> Result<(store::Profile, &'static str), ApiError> {
    let (found, profile, spec) = st.hub.store(|s| -> store::Res<_> {
        let found = s.activate_profile(id)?;
        Ok((found, s.profile(id)?, s.launch_spec()?))
    })?;
    let (true, Some(profile)) = (found, profile) else { return Err(profile_missing()) };
    let engine = match &st.engine {
        None => "unmanaged",
        Some(e) if !e.managing() => "other_window",
        Some(e) if e.foreign() => "foreign",
        Some(e) if matches!(e.problem(), Some(crate::engine::Problem::PortBusy { .. })) => {
            e.wake();
            "port_busy"
        }
        Some(e) => {
            let same = e.running().zip(spec).is_some_and(|(running, wanted)| running.same_launch(&wanted));
            e.wake();
            if same {
                "unchanged"
            } else {
                "restarting"
            }
        }
    };
    log_model(st, "model profile activated", json!({ "profile_id": id, "engine": engine }));
    st.hub.models_changed();
    Ok((profile, engine))
}

/// `GET /models/engines`: `{engines: [...]}`. A build whose version is not known yet is asked
/// once (`--version`, cached per program file).
pub(super) async fn get_engines(State(st): State<AppState>) -> ApiResult {
    let engines = st.hub.store(|s| s.engines())?;
    let mut out = Vec::with_capacity(engines.len());
    for mut e in engines {
        let exe = std::path::PathBuf::from(&e.exe);
        let exists = exe.is_file();
        if e.version.is_none() && exists {
            let extra: Vec<std::path::PathBuf> = e.extra_path.iter().map(Into::into).collect();
            let probe = tokio::task::spawn_blocking(move || crate::engine::probe::probe(&exe, &extra)).await?;
            if let Ok(p) = &*probe {
                if let Some(v) = &p.version {
                    st.hub.store(|s| s.set_engine_version(e.id, v))?;
                    e.version = Some(v.clone());
                }
            }
        }
        let mut v = serde_json::to_value(&e).map_err(ApiError::storage)?;
        v["exists"] = json!(exists);
        v["selftest"] = json!(st.hub.store(|s| s.engine_selftest(e.id))?);
        out.push(v);
    }
    Ok(Json(json!({ "engines": out })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_core::sampler::Sampler;

    #[test]
    fn profile_checks() {
        let file = std::env::temp_dir().join(format!("rp-app-check-{}.gguf", std::process::id()));
        std::fs::write(&file, b"GGUF").unwrap();
        let f = file.display().to_string();
        let new = |name: Option<&str>, engine: Option<i64>, model: Option<&str>| ProfileEdit {
            name: name.map(Into::into),
            engine_id: engine,
            model_file: model.map(Into::into),
            ..ProfileEdit::default()
        };
        assert_eq!(check_profile(new(Some("  Orion 32k "), Some(1), Some(&f)), true).unwrap().name.as_deref(), Some("Orion 32k"));
        assert_eq!(check_profile(new(None, Some(1), Some(&f)), true).unwrap_err(), "bad_name");
        assert_eq!(check_profile(new(Some(" "), Some(1), Some(&f)), true).unwrap_err(), "bad_name");
        assert_eq!(check_profile(new(Some(&"x".repeat(61)), Some(1), Some(&f)), true).unwrap_err(), "bad_name");
        assert_eq!(check_profile(new(Some("a"), None, Some(&f)), true).unwrap_err(), "bad_engine");
        assert_eq!(check_profile(new(Some("a"), Some(1), None), true).unwrap_err(), "bad_model_file");
        assert_eq!(check_profile(new(Some("a"), Some(1), Some("Z:\\missing.gguf")), true).unwrap_err(), "bad_model_file");
        assert!(check_profile(ProfileEdit::default(), false).is_ok(), "an edit may leave everything out");
        let edit = |e: ProfileEdit| check_profile(e, false);
        let args = |a: &[&str]| ProfileEdit { load_args: Some(a.iter().map(|s| s.to_string()).collect()), ..ProfileEdit::default() };
        assert!(edit(args(&["-c", "4096", "-ngl", "0"])).is_ok());
        assert_eq!(edit(args(&["--port", "9931"])).unwrap_err(), "bad_load_args");
        assert_eq!(edit(args(&["--model=x.gguf"])).unwrap_err(), "bad_load_args");
        assert_eq!(edit(args(&[""])).unwrap_err(), "bad_load_args");
        // H1/H2: only the allowlist (files, network, downloads and other chat templates refused).
        assert!(edit(args(&["-c", "8192", "-ngl", "99", "--n-cpu-moe", "8", "-ctk", "q8_0", "-fa", "on", "--temp", "0.8", "--jinja"])).is_ok());
        for bad in [
            &["--log-file", "C:\\x.bat"][..],
            &["-hfr", "someone/evil-model"],
            &["--rpc", "203.0.113.5:50052"],
            &["-mu", "https://example.invalid/x.gguf"],
            &["--slot-save-path", "C:\\x"],
            &["--chat-template", "{{ messages[1].content }}"],
            &["--chat-template-file", "C:\\t.jinja"],
            &["--chat-template-kwargs", "{}"],
            &["--override-kv", "tokenizer.chat_template=str:x"],
            &["--mmproj", "\\\\host\\share\\p.gguf"],
        ] {
            assert_eq!(edit(args(bad)).unwrap_err(), "arg_not_allowed", "{bad:?}");
        }
        let sampler = ProfileEdit { sampler: Some(Sampler { top_p: Some(1.5), ..Sampler::default() }), ..ProfileEdit::default() };
        assert_eq!(edit(sampler).unwrap_err(), "bad_sampler");
        let reasoning = ProfileEdit { reasoning: Some("auto".into()), ..ProfileEdit::default() };
        assert_eq!(edit(reasoning).unwrap_err(), "bad_reasoning");
        let _ = std::fs::remove_file(file);
    }
}
