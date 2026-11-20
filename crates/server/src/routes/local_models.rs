//! "Meine Modelle": the model files in the model folders (list, folders, use, open in Explorer,
//! delete in the app's own folder) and the memory estimate of a model at each context step.

use std::collections::HashSet;
use std::path::{Path as FsPath, PathBuf};
use crate::system::{main_gpu, memory_now};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use rp_core::gguf::GgufInfo;
use rp_core::llama_args::without_flags;
use rp_core::vram::{self, Load};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::models::{activate, log_model, MAX_PATH_CHARS};
use crate::engine::local::{self, Folders, Origin};
use crate::engine::model::{gguf_cached, summarize};
use crate::error::{ApiError, ApiResult};
use crate::fetch::pick::{self, Fit, Pc};
use crate::store::{self, ProfileEdit, Store, PROFILE_NAME_MAX};
use crate::AppState;

const MAX_EXTRA_FOLDERS: usize = 20;
/// Context of a profile made by "Benutzen" (or the trained one, when smaller): the one the picks
/// place a model at.
const USE_CTX: u64 = pick::BASE_CTX;
/// Load arguments that belong to one model and are not copied to a profile for another (the
/// KV type is set again: the placement is computed with the app's).
const MODEL_SPECIFIC: [&str; 9] = ["-c", "--ctx-size", "--n-cpu-moe", "-ncmoe", "--mmproj", "-ctk", "--cache-type-k", "-ctv", "--cache-type-v"];
/// The spellings of `-ngl`.
const GPU_LAYERS: [&str; 3] = ["-ngl", "--gpu-layers", "--n-gpu-layers"];

fn folders_of(s: &Store, data_dir: &FsPath) -> store::Res<Folders> {
    let (app, extra) = s.model_folders()?;
    Ok(Folders {
        app_default: app.is_none(),
        app: app.map(PathBuf::from).unwrap_or_else(|| local::default_app_dir(data_dir)),
        lmstudio: local::lmstudio_dir(),
        extra: extra.into_iter().map(PathBuf::from).collect(),
    })
}

fn folders_json(f: &Folders) -> Value {
    let entry = |p: &PathBuf| json!({ "path": p.display().to_string(), "exists": p.is_dir() });
    json!({
        "app": { "path": f.app.display().to_string(), "exists": f.app.is_dir(), "default": f.app_default },
        "lmstudio": f.lmstudio.as_ref().map(entry),
        "extra": f.extra.iter().map(entry).collect::<Vec<_>>(),
    })
}

fn same_file(a: &str, b: &FsPath) -> bool {
    local::path_key(FsPath::new(a)) == local::path_key(b)
}

/// A `.gguf` path from a request body, or 400 `bad_model_file`.
fn gguf_path(path: &str) -> Result<PathBuf, ApiError> {
    let p = path.trim();
    if p.is_empty() || p.chars().count() > MAX_PATH_CHARS || p.contains('\0') || !p.to_ascii_lowercase().ends_with(".gguf") {
        return Err(ApiError::bad_request("bad_model_file"));
    }
    Ok(PathBuf::from(p))
}

/// `GET /models/local`: `{folders: {app, lmstudio, extra}, models: [...]}`; each model with the
/// profiles that use it and whether the app may delete it (only in its own folder).
pub(super) async fn get_local(State(st): State<AppState>) -> ApiResult {
    let (folders, profiles) = st.hub.store(|s| -> store::Res<_> { Ok((folders_of(s, &st.data_dir)?, s.profiles()?)) })?;
    let roots = folders.roots();
    let models = tokio::task::spawn_blocking(move || local::scan(&roots)).await?;
    let models: Vec<Value> = models
        .into_iter()
        .map(|m| {
            let path = PathBuf::from(&m.path);
            let used: Vec<Value> = profiles
                .iter()
                .filter(|p| same_file(&p.model_file, &path))
                .map(|p| json!({ "id": p.id, "name": p.name, "active": p.active }))
                .collect();
            let deletable = m.origin == Origin::App;
            let mut v = serde_json::to_value(m).unwrap_or_default();
            v["profiles"] = json!(used);
            v["deletable"] = json!(deletable);
            v
        })
        .collect();
    Ok(Json(json!({ "folders": folders_json(&folders), "models": models })).into_response())
}

#[derive(Deserialize)]
pub(super) struct FoldersBody {
    /// None or empty: the default `<data>\models`.
    app_folder: Option<String>,
    #[serde(default)]
    extra: Vec<String>,
}

/// A folder from a request body: absolute, not too long.
fn folder(text: &str) -> Option<PathBuf> {
    let t = text.trim();
    // One spelling per folder: `D:/models` and `D:\models` are the same on Windows.
    let t = if cfg!(windows) { t.replace('/', "\\") } else { t.to_string() };
    let t = t.trim_end_matches(['\\', '/']);
    let t = if t.ends_with(':') { format!("{t}\\") } else { t.to_string() };
    let ok = !t.is_empty() && t.chars().count() <= MAX_PATH_CHARS && !t.contains('\0') && FsPath::new(&t).is_absolute();
    ok.then(|| PathBuf::from(t))
}

/// `PUT /models/folders` `{app_folder, extra: [...]}`: the app's folder is created when missing;
/// further folders must exist. 400 `bad_folder`. Answers like the `folders` of `GET /models/local`.
pub(super) async fn put_folders(State(st): State<AppState>, Json(body): Json<FoldersBody>) -> ApiResult {
    let app = match body.app_folder.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(t) => {
            let p = folder(t).ok_or_else(|| ApiError::bad_request("bad_folder"))?;
            std::fs::create_dir_all(&p).map_err(|_| ApiError::bad_request("bad_folder"))?;
            Some(p)
        }
    };
    if body.extra.len() > MAX_EXTRA_FOLDERS {
        return Err(ApiError::bad_request("bad_folder"));
    }
    let mut extra: Vec<PathBuf> = Vec::new();
    for e in &body.extra {
        let p = folder(e).filter(|p| p.is_dir()).ok_or_else(|| ApiError::bad_request("bad_folder"))?;
        let key = local::path_key(&p);
        let dup = extra.iter().any(|x| local::path_key(x) == key) || app.as_ref().is_some_and(|a| local::path_key(a) == key);
        if !dup {
            extra.push(p);
        }
    }
    let text: Vec<String> = extra.iter().map(|p| p.display().to_string()).collect();
    let app_text = app.as_ref().map(|p| p.display().to_string());
    let folders = st.hub.store(|s| -> store::Res<_> {
        s.set_model_folders(app_text.as_deref(), &text)?;
        folders_of(s, &st.data_dir)
    })?;
    log_model(&st, "model folders changed", json!({ "app_default": folders.app_default, "extra": folders.extra.len() }));
    st.hub.models_changed();
    Ok(Json(folders_json(&folders)).into_response())
}

#[derive(Deserialize)]
pub(super) struct PathBody {
    path: String,
}

/// `DELETE /models/local` `{path}`: deletes the model (all parts of a split one), only inside the
/// app's own folder: 403 `not_app_folder`; 404 `model_missing`; 409 `model_in_use` while the
/// active profile runs it. `{freed_bytes}`.
pub(super) async fn delete_local(State(st): State<AppState>, Json(body): Json<PathBody>) -> ApiResult {
    let path = gguf_path(&body.path)?;
    if !path.is_file() {
        return Err(ApiError::not_found("model_missing"));
    }
    let (folders, active) = st.hub.store(|s| -> store::Res<_> { Ok((folders_of(s, &st.data_dir)?, s.active_profile()?)) })?;
    if !local::inside(&folders.app, &path) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "not_app_folder"));
    }
    let files = local::model_files(&path);
    if active.is_some_and(|p| files.iter().any(|f| same_file(&p.model_file, f))) {
        return Err(ApiError::conflict("model_in_use"));
    }
    let freed = tokio::task::spawn_blocking(move || local::delete_model(&path)).await?.map_err(ApiError::storage)?;
    log_model(&st, "model file deleted", json!({ "parts": files.len(), "freed_bytes": freed }));
    st.hub.models_changed();
    Ok(Json(json!({ "freed_bytes": freed })).into_response())
}

/// `POST /models/local/reveal` `{path}`: shows the file in Windows Explorer. 404 `model_missing`
/// for a file outside the model folders; 501 `unsupported` on other systems.
pub(super) async fn reveal_local(State(st): State<AppState>, Json(body): Json<PathBody>) -> ApiResult {
    let path = gguf_path(&body.path)?;
    let folders = st.hub.store(|s| folders_of(s, &st.data_dir))?;
    let known = tokio::task::spawn_blocking({
        let path = path.clone();
        move || path.is_file() && folders.roots().iter().any(|(_, root)| local::inside(root, &path))
    })
    .await?;
    if !known {
        return Err(ApiError::not_found("model_missing"));
    }
    reveal(&path)?;
    log_model(&st, "model folder opened", json!({}));
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(windows)]
fn reveal(path: &FsPath) -> Result<(), ApiError> {
    use std::os::windows::process::CommandExt;
    // `/select,"<path>"` exactly so: Explorer does not parse ordinary argument quoting.
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
        .map(drop)
        .map_err(ApiError::storage)
}

#[cfg(not(windows))]
fn reveal(_: &FsPath) -> Result<(), ApiError> {
    Err(ApiError::new(StatusCode::NOT_IMPLEMENTED, "unsupported"))
}

/// A profile name that no other profile has: the model's name, else "Name (2)", ….
fn free_name(base: &str, taken: &[String]) -> String {
    let base: String = base.trim().chars().take(PROFILE_NAME_MAX - 4).collect();
    let base = if base.is_empty() { "Modell".to_string() } else { base };
    let free = |n: &String| !taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    if free(&base) {
        return base;
    }
    (2..).map(|i| format!("{base} ({i})")).find(free).unwrap_or(base)
}

/// How "Benutzen" placed a new model on this PC (the answer's `placement`).
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Placement {
    /// `fits`, `cpu_help` or `too_big` (then no placement flag is set).
    fit: Fit,
    /// `--n-cpu-moe` written (0: none).
    n_cpu_moe: u64,
    /// `-ngl` written (None: the template's stays).
    n_gpu_layers: Option<u64>,
    ctx: u64,
    gpu_gb: f64,
    ram_gb: f64,
    /// `catalog`: sized the way the picks size it; `file`: from the file's own header.
    source: &'static str,
    /// `n_cpu_moe_unsupported`: the engine's `--help` lacks `--n-cpu-moe`, so whole layers stay
    /// in RAM instead (`-ngl`).
    fallback: Option<&'static str>,
}

/// Where a new model goes: its memory shape, the PC, and the flags the engine knows (None: its
/// `--help` was not readable; the supervisor checks the flags again before the start).
struct Target<'a> {
    info: &'a GgufInfo,
    source: &'static str,
    pc: &'a Pc,
    known: Option<&'a HashSet<String>>,
}

/// The arguments for a new profile of another model: those of the template without what belongs
/// to its model, with a context the new model was trained for (at most [`USE_CTX`]), the app's KV
/// type ([`pick::KV`]) and the placement the picks compute for this PC ([`pick::place`]):
/// `--n-cpu-moe N` for a MoE model with experts in RAM, `-ngl N` for a dense one with layers in
/// RAM (or for a MoE one when the engine lacks `--n-cpu-moe`). A template with `-ngl 0` runs on
/// the CPU and stays so.
fn args_for(template: &[String], ctx_train: Option<u64>, target: Option<Target>) -> (Vec<String>, Option<Placement>) {
    let mut args = without_flags(template, &MODEL_SPECIFIC);
    let ctx = ctx_train.filter(|&c| c >= 512).map_or(USE_CTX, |c| c.min(USE_CTX));
    args.extend(["-c", &ctx.to_string(), "-ctk", pick::KV, "-ctv", pick::KV].map(String::from));
    let Some(t) = target else { return (args, None) };
    let cpu_only = rp_core::bench::gpu_layers(template) == Some(0);
    let pc = if cpu_only { Pc { backend: Some("cpu".into()), ..t.pc.clone() } } else { t.pc.clone() };
    let mut p = pick::place(t.info, &pc, ctx);
    let mut fallback = None;
    let knows_ncmoe = t.known.is_none_or(|k| k.contains("--n-cpu-moe") || k.contains("-ncmoe"));
    if p.n_cpu_moe > 0 && !knows_ncmoe {
        p = pick::by_layers(t.info, &pc, ctx);
        fallback = Some("n_cpu_moe_unsupported");
    }
    if p.fit != Fit::TooBig {
        if p.n_cpu_moe > 0 {
            args.extend(["--n-cpu-moe".to_string(), p.n_cpu_moe.to_string()]);
        }
        if let Some(n) = p.gpu_layers {
            args = without_flags(&args, &GPU_LAYERS);
            args.extend(["-ngl".to_string(), n.to_string()]);
        }
    }
    let gb = |b: u64| (b as f64 / 1_073_741_824.0 * 10.0).round() / 10.0;
    let written = p.fit != Fit::TooBig;
    let placement = Placement {
        fit: p.fit,
        n_cpu_moe: if written { p.n_cpu_moe } else { 0 },
        n_gpu_layers: p.gpu_layers.filter(|_| written),
        ctx,
        gpu_gb: gb(p.gpu_bytes),
        ram_gb: gb(p.ram_bytes),
        source: t.source,
        fallback,
    };
    (args, Some(placement))
}

/// What "Benutzen" finds: a profile that uses the file already, or the template and the engine
/// (id, program, extra PATH folders, backend) of a new one.
enum Found {
    Used(i64),
    New { template: Option<Box<store::Profile>>, engine: (i64, String, Vec<String>, String) },
}

/// `POST /models/local/use` `{path}`: runs this model. A profile that already uses the file is
/// activated; else a new one is made (start options, sampler and engine copied from the active
/// profile, context of the new model, placed on this PC as the picks place it: [`args_for`]) and
/// activated. `{profile, created, engine, placement}` (`engine` as for activating; `placement`
/// only for a new profile on a known PC, see [`Placement`]). 409 `busy` while a reply is written,
/// `no_engine` without any engine.
pub(super) async fn use_local(State(st): State<AppState>, Json(body): Json<PathBody>) -> ApiResult {
    let path = gguf_path(&body.path)?;
    if !path.is_file() {
        return Err(ApiError::bad_request("bad_model_file"));
    }
    if st.hub.generating() {
        return Err(ApiError::conflict("busy"));
    }
    let p = path.clone();
    let (info, gguf) = tokio::task::spawn_blocking(move || {
        let gguf = gguf_cached(&p);
        (summarize(&p, gguf.as_deref()), gguf)
    })
    .await?;
    let found = st.hub.store(|s| -> store::Res<Result<Found, ApiError>> {
        let profiles = s.profiles()?;
        let mut users: Vec<&store::Profile> = profiles.iter().filter(|p| same_file(&p.model_file, &path)).collect();
        users.sort_by_key(|p| !p.active);
        if let Some(p) = users.first() {
            return Ok(Ok(Found::Used(p.id)));
        }
        let template = profiles.iter().find(|p| p.active).or(profiles.first()).cloned().map(Box::new);
        let engine = match &template {
            Some(t) => s.engine(t.engine_id)?,
            None => s.engines()?.into_iter().next(),
        };
        let Some(e) = engine else { return Ok(Err(ApiError::conflict("no_engine"))) };
        Ok(Ok(Found::New { template, engine: (e.id, e.exe, e.extra_path, e.backend) }))
    })??;
    let (id, created, placement) = match found {
        Found::Used(id) => (id, false, None),
        Found::New { template, engine: (engine_id, exe, extra, backend) } => {
            // The PC as the picks see it, with the engine's own backend where it is known.
            let pc = st.fetch.pc().await.map(|mut pc| {
                if matches!(backend.as_str(), "cpu" | "cuda" | "rocm" | "vulkan") {
                    pc.backend = Some(backend.clone());
                }
                pc
            });
            let (exe, extra): (PathBuf, Vec<PathBuf>) = (exe.into(), extra.iter().map(Into::into).collect());
            let probe = tokio::task::spawn_blocking(move || crate::engine::probe::probe(&exe, &extra)).await?;
            let known = probe.as_ref().as_ref().ok().map(|p| &p.flags);
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let (shape, source) = match st.fetch.catalog_info(&name) {
                Some(i) => (Some(i), "catalog"),
                None => (gguf.as_deref().cloned(), "file"),
            };
            let target = shape.as_ref().zip(pc.as_ref()).map(|(info, pc)| Target { info, source, pc, known });
            let (args, placement) = args_for(template.as_ref().map(|t| t.load_args.as_slice()).unwrap_or_default(), info.ctx_train, target);
            let id = st.hub.store(|s| -> store::Res<i64> {
                let taken: Vec<String> = s.profiles()?.into_iter().map(|p| p.name).collect();
                let edit = ProfileEdit {
                    name: Some(free_name(info.name.as_deref().unwrap_or_default(), &taken)),
                    engine_id: Some(engine_id),
                    model_file: Some(path.display().to_string()),
                    load_args: Some(args),
                    sampler: template.as_ref().map(|t| t.sampler.clone()),
                    reasoning: Some("off".into()),
                    measured: Some(false),
                };
                Ok(s.add_profile(&edit)?.id)
            })?;
            log_model(&st, "model profile created", json!({ "profile_id": id, "from": "local", "placement": placement }));
            (id, true, placement)
        }
    };
    let (profile, engine) = activate(&st, id)?;
    Ok(Json(json!({ "profile": profile, "created": created, "engine": engine, "placement": placement })).into_response())
}

#[derive(Deserialize)]
pub(super) struct EstimateBody {
    path: String,
    #[serde(default)]
    cache_k: Option<String>,
    #[serde(default)]
    cache_v: Option<String>,
    #[serde(default)]
    gpu_layers: Option<u64>,
    #[serde(default)]
    n_cpu_moe: Option<u64>,
    /// A context to include besides the steps (the one a profile has now).
    #[serde(default)]
    ctx: Option<u64>,
}

/// `POST /models/estimate` `{path, cache_k?, cache_v?, gpu_layers?, n_cpu_moe?, ctx?}`: the
/// estimate ("ca.") for every context step from 2k to the trained context (and `ctx`), with the verdict against the
/// VRAM free now (WDDM counters): `{layers, moe, ctx_train, gpu, vram_bytes, vram_free_bytes,
/// ram_free_bytes, steps: [{ctx, gpu_bytes, cpu_bytes, fit}]}`; `fit` is `fits`, `cpu_help`,
/// `too_big` or `unknown`. 404 `model_missing`, 422 `unreadable` (no layer count in the file).
pub(super) async fn post_estimate(State(st): State<AppState>, Json(body): Json<EstimateBody>) -> ApiResult {
    let path = gguf_path(&body.path)?;
    let bad_type = |t: &Option<String>| t.as_deref().is_some_and(|t| vram::kv_type_bytes(t).is_none());
    if bad_type(&body.cache_k) || bad_type(&body.cache_v) {
        return Err(ApiError::bad_request("bad_cache_type"));
    }
    let pid = st.engine.as_ref().and_then(|e| e.pid());
    let out = tokio::task::spawn_blocking(move || -> Result<Value, ApiError> {
        if !path.is_file() {
            return Err(ApiError::not_found("model_missing"));
        }
        let info = gguf_cached(&path).ok_or_else(|| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "unreadable"))?;
        let (vram_bytes, free, ram) = memory_now(pid);
        let load = Load {
            ctx: 0,
            cache_k: body.cache_k.unwrap_or_else(|| "f16".into()),
            cache_v: body.cache_v.unwrap_or_else(|| "f16".into()),
            gpu_layers: body.gpu_layers,
            n_cpu_moe: body.n_cpu_moe.unwrap_or(0),
        };
        let mut ctxs = vram::ctx_steps(info.context_length);
        if let Some(c) = body.ctx.filter(|&c| (256..=1 << 22).contains(&c) && !ctxs.contains(&c)) {
            ctxs.push(c);
            ctxs.sort_unstable();
        }
        let mut steps = Vec::new();
        for ctx in ctxs {
            let e = vram::estimate(&info, &Load { ctx, ..load.clone() }).ok_or_else(|| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "unreadable"))?;
            steps.push(json!({ "ctx": ctx, "gpu_bytes": e.gpu_bytes, "cpu_bytes": e.cpu_bytes, "fit": vram::fit(&e, free, ram) }));
        }
        Ok(json!({
            "layers": info.block_count,
            "moe": info.is_moe(),
            "ctx_train": info.context_length,
            "gpu": main_gpu().map(|g| g.name.clone()),
            "vram_bytes": vram_bytes,
            "vram_free_bytes": free,
            "ram_free_bytes": ram,
            "steps": steps,
        }))
    })
    .await??;
    Ok(Json(out).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_args_of_a_new_profile() {
        let taken = vec!["Mini".to_string(), "mini (2)".to_string()];
        assert_eq!(free_name("Mini", &taken), "Mini (3)");
        assert_eq!(free_name("Other", &taken), "Other");
        assert_eq!(free_name("  ", &taken), "Modell");
        assert!(free_name(&"x".repeat(80), &[]).chars().count() <= PROFILE_NAME_MAX);
        let t = strings(&["-ngl", "0", "-c", "131072", "--n-cpu-moe", "20", "-np", "1", "-ctk", "f16"]);
        assert_eq!(args_for(&t, Some(4096), None).0, strings(&["-ngl", "0", "-np", "1", "-c", "4096", "-ctk", "q8_0", "-ctv", "q8_0"]));
        assert_eq!(args_for(&t, Some(131072), None).0, strings(&["-ngl", "0", "-np", "1", "-c", "8192", "-ctk", "q8_0", "-ctv", "q8_0"]));
        assert_eq!(args_for(&[], None, None).0, strings(&["-c", "8192", "-ctk", "q8_0", "-ctv", "q8_0"]));
    }

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn value(args: &[String], flag: &str) -> Option<u64> {
        args.windows(2).find(|w| w[0] == flag).and_then(|w| w[1].parse().ok())
    }

    /// A `--help` text of a llama-server build from before `--n-cpu-moe` (the flags only).
    const OLD_HELP: &str = "-h, --help\n--version\n-t, --threads N\n-c, --ctx-size N\n-n, --n-predict N\n-b, --batch-size N\n\
        -ub, --ubatch-size N\n-fa, --flash-attn [on|off|auto]\n-ctk, --cache-type-k TYPE\n-ctv, --cache-type-v TYPE\n\
        -ngl, --gpu-layers, --n-gpu-layers N\n-m, --model FNAME\n--port PORT\n-np, --parallel N\n";

    /// H1: "Benutzen" of a model the picks place with experts in RAM writes the same `--n-cpu-moe`
    /// into the new profile, with the context and KV type the picks computed with; on a 24 GB
    /// card nothing goes to RAM; an engine without `--n-cpu-moe` gets whole layers in RAM (`-ngl`)
    /// and the answer says so; a template that runs on the CPU stays on the CPU.
    #[test]
    fn use_places_the_model_like_the_pick() {
        let catalog = crate::fetch::catalog::embedded();
        let all: Vec<pick::Sized> = catalog
            .models
            .iter()
            .map(|m| pick::Sized { model: m.clone(), shape: catalog.shapes[&m.shape].clone(), sizes: m.quants.tiers().map(|(_, q)| q.map(|q| q.size_bytes)), installed: [false; 3], measured: [None; 3] })
            .collect();
        let pc8 = Pc::profile(8, 32, Some("nvidia"), "cuda");
        let pick = pick::best(&all, &pc8, false).roleplay[0].clone();
        assert_eq!((pick.fit, pick.n_cpu_moe > 0), (Fit::Cpu, true), "{pick:?}");
        let m = catalog.models.iter().find(|m| m.id == pick.id).unwrap();
        let file = m.quants.tiers().into_iter().find(|(t, _)| *t == pick.tier).and_then(|(_, q)| q).unwrap().file.clone();
        let info = crate::fetch::catalog::info_of(&catalog, &Default::default(), &file).unwrap();
        let template = strings(&["-ngl", "999", "-fa", "on", "-c", "65536", "--n-cpu-moe", "3"]);
        let target = |pc, known| Some(Target { info: &info, source: "catalog", pc, known });

        let (args, placement) = args_for(&template, m.context_max, target(&pc8, None));
        assert_eq!(value(&args, "--n-cpu-moe"), Some(pick.n_cpu_moe), "{args:?}");
        assert_eq!((value(&args, "-c"), value(&args, "-ngl")), (Some(8192), Some(999)));
        assert!(args.windows(2).any(|w| w[0] == "-ctk" && w[1] == "q8_0"), "{args:?}");
        let placement = placement.unwrap();
        assert_eq!((placement.fit, placement.n_cpu_moe, placement.n_gpu_layers, placement.fallback), (Fit::Cpu, pick.n_cpu_moe, None, None));
        assert_eq!((placement.gpu_gb, placement.ram_gb), (pick.gpu_gb, pick.ram_gb));

        // 24 GB: all on the GPU, no --n-cpu-moe (also not the template's).
        let pc24 = Pc::profile(24, 32, Some("nvidia"), "cuda");
        let (args, placement) = args_for(&template, m.context_max, target(&pc24, None));
        assert_eq!((value(&args, "--n-cpu-moe"), placement.unwrap().fit), (None, Fit::Fits), "{args:?}");

        // A build without --n-cpu-moe: whole layers in RAM instead, said in the answer.
        let old = rp_core::llama_args::parse_help(OLD_HELP);
        assert!(!old.contains("--n-cpu-moe") && old.contains("-ngl"));
        let (args, placement) = args_for(&template, m.context_max, target(&pc8, Some(&old)));
        let placement = placement.unwrap();
        let layers = catalog.shapes[&m.shape].layers;
        assert_eq!(value(&args, "--n-cpu-moe"), None, "{args:?}");
        assert_eq!(placement.fallback, Some("n_cpu_moe_unsupported"));
        let ngl = value(&args, "-ngl").unwrap();
        assert!(ngl < layers && Some(ngl) == placement.n_gpu_layers && args.iter().filter(|a| *a == "-ngl").count() == 1, "{args:?}");
        let e = vram::estimate(&info, &Load { ctx: 8192, cache_k: "q8_0".into(), cache_v: "q8_0".into(), gpu_layers: Some(ngl), n_cpu_moe: 0 }).unwrap();
        assert!(e.gpu_bytes <= pc8.vram_budget(), "the layers on the GPU fit its budget");

        // A template with -ngl 0 runs on the CPU: no placement flag.
        let (args, placement) = args_for(&strings(&["-ngl", "0"]), m.context_max, target(&pc8, None));
        let placement = placement.unwrap();
        assert_eq!(args, strings(&["-ngl", "0", "-c", "8192", "-ctk", "q8_0", "-ctv", "q8_0"]));
        assert_eq!((placement.gpu_gb, placement.n_cpu_moe, placement.n_gpu_layers), (0.0, 0, None), "placed as on a PC without a GPU");
    }

    /// M5: a dense model that only fits with layers in RAM gets `-ngl N` (the pill's "CPU-Hilfe"
    /// is a setting now): the VRAM part of N layers fits the budget, the RAM part the RAM.
    #[test]
    fn dense_model_gets_its_layers() {
        let catalog = crate::fetch::catalog::embedded();
        let m = catalog.models.iter().find(|m| m.id == "gemma-4-31b-it").unwrap();
        let file = &m.quants.large.as_ref().unwrap().file;
        let info = crate::fetch::catalog::info_of(&catalog, &Default::default(), file).unwrap();
        let pc = Pc::profile(12, 32, Some("amd"), "rocm");
        let (args, placement) = args_for(&strings(&["-ngl", "999", "-fa", "on"]), m.context_max, Some(Target { info: &info, source: "catalog", pc: &pc, known: None }));
        let placement = placement.unwrap();
        let ngl = value(&args, "-ngl").unwrap();
        assert_eq!((placement.fit, placement.n_gpu_layers, placement.n_cpu_moe), (Fit::Cpu, Some(ngl), 0), "{args:?}");
        assert!(ngl > 0 && ngl < catalog.shapes[&m.shape].layers);
        let at = |n| vram::estimate(&info, &Load { ctx: 8192, cache_k: "q8_0".into(), cache_v: "q8_0".into(), gpu_layers: Some(n), n_cpu_moe: 0 }).unwrap();
        assert!(at(ngl).gpu_bytes <= pc.vram_budget() && at(ngl + 1).gpu_bytes > pc.vram_budget(), "the most layers that fit");
        assert!(pc.ram_fits(at(ngl).cpu_bytes));
        assert_eq!(args.iter().filter(|a| *a == "-ngl").count(), 1, "{args:?}");
    }

    /// H1 through the real server and a real database: on a PC stored as 8 GB VRAM / 32 GB RAM,
    /// "Benutzen" of the catalog file that "Bestes Modell" recommends with experts in RAM makes a
    /// profile with exactly the pick's `--n-cpu-moe`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn use_over_http_takes_the_picks_placement() {
        use crate::engine::local::tests::gguf_bytes;
        use crate::{start, Config, WebUi};
        use std::sync::{Arc, Mutex};
        std::env::set_var("RP_LMSTUDIO_DIR", ""); // never the real LM Studio folder
        let dir = std::env::temp_dir().join(format!("rp-app-use-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["web", "data/models"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let catalog = crate::fetch::catalog::embedded();
        let orion = catalog.models.iter().find(|m| m.id == "orion-26b-a4b-v1-1").unwrap();
        let file = orion.quants.balanced.as_ref().unwrap().file.clone();
        let model = dir.join("data/models").join(&file);
        std::fs::write(&model, gguf_bytes("Orion", "gemma4", 262_144, 64)).unwrap();
        let mini = dir.join("data/models/Mini-Q4_K_M.gguf");
        std::fs::write(&mini, gguf_bytes("Mini", "llama", 8192, 64)).unwrap();
        let engine_json = json!({ "llama_server": dir.join("missing.exe"), "model": mini, "port": 7866, "args": ["-c", "4096", "-ngl", "999", "-fa", "on"] });
        std::fs::write(dir.join("data/engine.json"), engine_json.to_string()).unwrap();
        // The PC as "PC prüfen" stores it.
        let scan = json!({ "gpus": [{ "name": "Test GPU", "vendor": "nvidia", "vram_total": 8u64 << 30 }], "ram_total": 32u64 << 30, "backend": { "suggested": "cuda" } });
        store::Store::open(&dir.join("data")).unwrap().set_system_scan(&scan).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);
        let catalog_view: Value = c.get(format!("{base}/models/catalog")).header("authorization", &auth).send().await.unwrap().json().await.unwrap();
        let pick = &catalog_view["best"]["roleplay"][0];
        assert_eq!((pick["id"].as_str(), pick["tier"].as_str(), pick["installed"].as_bool(), pick["fit"].as_str()), (Some(orion.id.as_str()), Some("balanced"), Some(true), Some("cpu_help")), "{pick}");
        let n = pick["n_cpu_moe"].as_u64().unwrap();
        assert!(n > 0);

        let v: Value = c.post(format!("{base}/models/local/use")).header("authorization", &auth).json(&json!({ "path": model })).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["created"].as_bool(), Some(true), "{v}");
        let args: Vec<String> = serde_json::from_value(v["profile"]["load_args"].clone()).unwrap();
        assert_eq!(value(&args, "--n-cpu-moe"), Some(n), "{args:?}");
        assert_eq!((value(&args, "-c"), value(&args, "-ngl")), (Some(8192), Some(999)), "{args:?}");
        assert_eq!((v["placement"]["fit"].as_str(), v["placement"]["n_cpu_moe"].as_u64(), v["placement"]["source"].as_str()), (Some("cpu_help"), Some(n), Some("catalog")), "{v}");
        assert_eq!((v["placement"]["gpu_gb"].clone(), v["placement"]["ram_gb"].clone()), (pick["gpu_gb"].clone(), pick["ram_gb"].clone()));
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn folders_get_one_spelling() {
        assert_eq!(folder(" D:/ki/models/ "), Some(PathBuf::from("D:\\ki\\models")));
        assert_eq!(folder("D:\\"), Some(PathBuf::from("D:\\")));
        assert_eq!(folder("relative\\models"), None);
        assert_eq!(folder(""), None);
    }
}
