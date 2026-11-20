//! Model downloads, pasted Hugging Face links, the catalog ("Vorschläge") and the live search
//! ("Mehr finden"). The work is done in `crate::fetch`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::error::{ApiError, ApiResult};
use crate::fetch::downloads::{hf_error, AddRequest};
use crate::fetch::{catalog, quant};
use crate::AppState;

/// `GET /models/downloads`: `{downloads: [...], folder}` (where new downloads go).
pub(super) async fn get_downloads(State(st): State<AppState>) -> ApiResult {
    Ok(Json(json!({ "downloads": st.fetch.downloads(), "folder": st.fetch.models_dir().display().to_string() })).into_response())
}

/// `POST /models/downloads` `{repo, file | files, sha256?, size?}`: 201 with the download.
/// 400 `bad_repo`/`bad_file`/`bad_sha256`, 403 `gated`, 404 `repo_missing`/`file_missing`,
/// 409 `exists`/`already_downloading`, 507 `no_space`, 502 `hf_unreachable`.
pub(super) async fn post_download(State(st): State<AppState>, Json(body): Json<AddRequest>) -> ApiResult {
    let job = st.fetch.add(body).await?;
    Ok((StatusCode::CREATED, Json(job)).into_response())
}

pub(super) async fn pause_download(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    Ok(Json(st.fetch.pause(&id)?).into_response())
}

pub(super) async fn resume_download(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    Ok(Json(st.fetch.resume(&id)?).into_response())
}

/// `DELETE /models/downloads/{id}`: 204; stops it and deletes its `.part` files.
pub(super) async fn delete_download(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    st.fetch.delete(&id)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub(super) struct Link {
    url: String,
}

/// `POST /models/resolve {url}`: a pasted Hugging Face link (or `org/name`) →
/// `{repo, file, license, groups: {small, balanced, large: {recommended, choices}}, other, mmproj}`;
/// each choice `{key, quant, tier, files: [{path, size, sha256}], size}`. One request.
/// 400 `bad_url`, 403 `gated`, 404 `repo_missing`, 502 `hf_unreachable`.
pub(super) async fn post_resolve(State(st): State<AppState>, Json(body): Json<Link>) -> ApiResult {
    if body.url.len() > 2048 {
        return Err(ApiError::bad_request("bad_url"));
    }
    let (repo, file) = quant::parse_link(&body.url).ok_or_else(|| ApiError::bad_request("bad_url"))?;
    let (info, files) = st.fetch.hf().repo_files(&repo).await.map_err(hf_error)?;
    if info.gated.is_some() {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "gated"));
    }
    let g = quant::group(&files);
    Ok(Json(json!({
        "repo": info.id, "file": file, "license": info.license, "last_modified": info.last_modified,
        "groups": { "small": g.small, "balanced": g.balanced, "large": g.large },
        "other": g.other, "mmproj": g.mmproj,
    }))
    .into_response())
}

/// `GET /models/catalog`: see `fetch::catalog`. Starts the live check of due repos in the
/// background (`{"type":"catalog"}` when done). `?uncensored=1`: the best picks only among models
/// with `uncensored ≥ 1` (the model list stays complete).
pub(super) async fn get_catalog(State(st): State<AppState>, Query(q): Query<CatalogQuery>) -> ApiResult {
    let uncensored = q.uncensored.as_deref().is_some_and(|v| matches!(v, "1" | "true"));
    Ok(Json(st.fetch.catalog_view(uncensored).await).into_response())
}

#[derive(Deserialize)]
pub(super) struct CatalogQuery {
    uncensored: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Search {
    q: String,
}

/// `GET /models/search?q=`: GGUF repos of the known quanters (and the catalog's owners), most
/// downloaded first, gated ones left out: `{results: [{id, downloads, likes, last_modified,
/// params, arch, context, curated}]}`. One request. 400 `bad_query`.
pub(super) async fn get_search(State(st): State<AppState>, Query(q): Query<Search>) -> ApiResult {
    let query = q.q.trim();
    if query.chars().count() < 2 || query.chars().count() > 80 {
        return Err(ApiError::bad_request("bad_query"));
    }
    let cat = catalog::embedded();
    let curated: Vec<&str> = cat.models.iter().map(|m| m.repo.as_str()).collect();
    let owners: Vec<&str> = quant::QUANTERS.iter().copied().chain(cat.models.iter().filter_map(|m| m.repo.split_once('/').map(|x| x.0))).collect();
    let found = st.fetch.hf().search(query, 60).await.map_err(hf_error)?;
    let results: Vec<_> = found
        .into_iter()
        .filter(|m| m.gated.is_none() && m.id.split_once('/').is_some_and(|(o, _)| owners.iter().any(|x| x.eq_ignore_ascii_case(o))))
        .take(20)
        .map(|m| {
            json!({
                "id": m.id, "downloads": m.downloads, "likes": m.likes, "last_modified": m.last_modified,
                "params": m.gguf.as_ref().and_then(|g| g.total), "arch": m.gguf.as_ref().and_then(|g| g.architecture.clone()),
                "context": m.gguf.as_ref().and_then(|g| g.context_length), "curated": curated.contains(&m.id.as_str()),
            })
        })
        .collect();
    Ok(Json(json!({ "results": results })).into_response())
}
