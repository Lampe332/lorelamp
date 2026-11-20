//! Import of many files (`POST /import`), of personas (`POST /personas/import`) and a card as a
//! persona (`POST /characters/{id}/as-persona`). The log gets counts only, never file names.

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use rand::distr::{Alphanumeric, SampleString};
use rp_core::formats::MAX_INPUT;
use serde::Deserialize;
use serde_json::json;

use crate::error::{ApiError, ApiResult};
use crate::import::{self, FileResult, Job, MAX_UPLOAD};
use crate::link;
use crate::link_web::{self, WebError};
use crate::logs::{Entry, Level};
use crate::store::{self, NewItem, NewPersona};
use crate::{preview, sillytavern, AppState};

#[derive(Deserialize)]
pub(super) struct ImportQuery {
    job: Option<String>,
}

/// The client's job id (so it can match progress events before the answer arrives), else a new one.
fn job_id(q: Option<String>) -> Result<String, ApiError> {
    match q {
        Some(id) if !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') => Ok(id),
        Some(_) => Err(ApiError::bad_request("bad_job")),
        None => Ok(Alphanumeric.sample_string(&mut rand::rng(), 12)),
    }
}

fn upload_too_large() -> ApiError {
    ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large")
}

/// One uploaded file, written chunk by chunk to `path` (the job's temporary folder, so its size
/// does not take RAM); 413 `too_large` when the whole upload is over [`MAX_UPLOAD`].
async fn spool_field(field: &mut axum::extract::multipart::Field<'_>, path: &std::path::Path, uploaded: &mut u64) -> Result<(), ApiError> {
    use tokio::io::AsyncWriteExt;
    let mut out = tokio::io::BufWriter::new(tokio::fs::File::create(path).await.map_err(ApiError::storage)?);
    while let Some(chunk) = field.chunk().await.map_err(|_| ApiError::bad_request("bad_upload"))? {
        *uploaded += chunk.len() as u64;
        if *uploaded > MAX_UPLOAD {
            return Err(upload_too_large());
        }
        out.write_all(&chunk).await.map_err(ApiError::storage)?;
    }
    out.flush().await.map_err(ApiError::storage)
}

/// One uploaded file read into memory: `Ok(None)` when it is over the size limit of a single
/// file (the rest of it is read and dropped), 413 `too_large` when the whole upload is over
/// `limit`.
async fn read_field(field: &mut axum::extract::multipart::Field<'_>, uploaded: &mut u64, limit: u64) -> Result<Option<Vec<u8>>, ApiError> {
    let mut buf = Vec::new();
    let mut over = false;
    while let Some(chunk) = field.chunk().await.map_err(|_| ApiError::bad_request("bad_upload"))? {
        *uploaded += chunk.len() as u64;
        if *uploaded > limit {
            return Err(upload_too_large());
        }
        if over || buf.len() + chunk.len() > MAX_INPUT {
            over = true;
            buf = Vec::new();
        } else {
            buf.extend_from_slice(&chunk);
        }
    }
    Ok((!over).then_some(buf))
}

fn file_name(field: &axum::extract::multipart::Field<'_>, n: usize) -> String {
    field.file_name().filter(|f| !f.is_empty()).map_or_else(|| format!("file{}", n + 1), str::to_string)
}

fn answer(st: &AppState, what: &str, job: Option<&str>, results: Vec<FileResult>) -> ApiResult {
    let (imported, duplicates, errors) = import::counts(&results);
    st.log().write(
        Entry::new(Level::Info, "server", "import", what.to_string())
            .data(json!({ "imported": imported, "duplicates": duplicates, "errors": errors })),
    );
    if imported > 0 {
        st.hub.library_changed();
    }
    let mut body = json!({ "results": results, "imported": imported, "duplicates": duplicates, "errors": errors });
    if let Some(job) = job {
        body["job"] = json!(job);
    }
    Ok(Json(body).into_response())
}

/// Multipart, any number of file fields (cards in every format, zips full of cards, RisuAI
/// persona PNGs), `?job=` optional. Files are read while the upload still runs; progress goes
/// to every device as `{"type":"import","job","done","total","finished"}`. Answers
/// `{job, results: [{file, status: "imported"|"duplicate"|"error", code?, character_id?,
/// persona_id?}], imported, duplicates, errors}`. A file over 128 MiB is `too_large` unless it
/// is a zip of files; a zip entry over 128 MiB or unpacking more than 100× its packed size is
/// `too_large`, so is the rest of the zips once they unpacked 4 GiB; the whole upload at most
/// 1 GiB (else 413 `too_large`). Imports from several devices at once take turns.
pub(super) async fn post_import(State(st): State<AppState>, Query(q): Query<ImportQuery>, mut mp: Multipart) -> ApiResult {
    let id = job_id(q.job)?;
    let mut job = Job::start(st.hub.clone(), id.clone()).await;
    let mut uploaded = 0u64;
    let mut n = 0;
    while let Some(mut field) = mp.next_field().await.map_err(|_| ApiError::bad_request("bad_upload"))? {
        let name = file_name(&field, n);
        let path = job.spool_file(n).map_err(ApiError::storage)?;
        spool_field(&mut field, &path, &mut uploaded).await?;
        job.add_spooled((n, 0), name, path);
        n += 1;
    }
    let results = job.finish().await?;
    answer(&st, "files imported", Some(&id), results)
}

/// Multipart: SillyTavern persona backup (`personas_*.json`) or `settings.json` plus, in the same
/// upload, the pictures from `User Avatars`; RisuAI persona PNGs; zips of the app's persona
/// export. Same answer as `POST /import` (with `persona_id`). A persona with the same name and
/// description as an existing one is a `duplicate`.
pub(super) async fn post_personas_import(State(st): State<AppState>, mut mp: Multipart) -> ApiResult {
    // Read in memory: one at a time with the other bulk imports, and a smaller upload limit.
    let _turn = import::turn().await;
    let mut files = vec![];
    let mut refused = vec![];
    let mut uploaded = 0u64;
    while let Some(mut field) = mp.next_field().await.map_err(|_| ApiError::bad_request("bad_upload"))? {
        let name = file_name(&field, files.len() + refused.len());
        match read_field(&mut field, &mut uploaded, import::MAX_PERSONA_UPLOAD).await? {
            Some(bytes) => files.push((name, bytes)),
            None => refused.push(FileResult::error(&name, "too_large")),
        }
    }
    let mut results = refused;
    results.extend(import_personas(&st, files).await?);
    answer(&st, "personas imported", None, results)
}

/// Reads persona files (as [`import::read_personas`]) and stores what they hold.
async fn import_personas(st: &AppState, files: Vec<(String, Vec<u8>)>) -> Result<Vec<FileResult>, ApiError> {
    let parts = tokio::task::spawn_blocking(move || import::read_personas(files)).await?;
    let mut items = vec![];
    let mut owners = vec![];
    let mut results: Vec<Option<FileResult>> = vec![];
    for (file, part) in parts {
        match part {
            Ok(p) => {
                owners.push((results.len(), file));
                results.push(None);
                items.push(NewItem::Persona(p));
            }
            Err(code) => results.push(Some(FileResult::error(&file, code))),
        }
    }
    let hub = st.hub.clone();
    let added = tokio::task::spawn_blocking(move || hub.store(|s| s.import_items(&items))).await??;
    for ((slot, file), added) in owners.into_iter().zip(added) {
        results[slot] = Some(FileResult::added(&file, added));
    }
    Ok(results.into_iter().flatten().collect())
}

/// The character's card as a new persona: name, description (`{{char}}` becomes the name) and
/// picture. Answers `{status: "imported"|"duplicate", persona_id}`.
pub(super) async fn post_as_persona(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let hub = st.hub.clone();
    let added = tokio::task::spawn_blocking(move || -> Result<store::Added, ApiError> {
        let c = hub.store(|s| s.character(id))?.ok_or(ApiError::not_found("character_missing"))?;
        let description = rp_core::prompt::resolve(&c.card.description, &c.card.name, "{{user}}");
        let edit = import::persona_edit(&c.card.name, &description, &serde_json::Value::Null).ok_or(ApiError::bad_request("bad_name"))?;
        let avatar = hub
            .store(|s| s.avatar(id))?
            .and_then(|(path, _)| std::fs::read(path).ok())
            .and_then(|bytes| store::image_mime(&bytes).map(|mime| (bytes, mime)));
        let added = hub.store(|s| s.import_items(&[NewItem::Persona(NewPersona { edit, avatar })]))?;
        Ok(added[0])
    })
    .await??;
    let result = FileResult::added("", added);
    if result.status == "imported" {
        crate::routes::personas::log_persona(&st, "persona from character", result.persona_id.unwrap_or_default());
        st.hub.library_changed();
    }
    let status = if result.status == "imported" { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(json!({ "status": result.status, "persona_id": result.persona_id }))).into_response())
}

#[derive(Deserialize)]
pub(super) struct LinkBody {
    url: String,
}

/// `{url}`: a pasted link to a card page. Chub and RisuRealm: the card and its picture are
/// fetched from their public API and imported like a file; answers one entry of `POST /import`
/// plus `source` (`{file, status: "imported"|"duplicate"|"error", code?, character_id?, source}`).
/// Sites that allow no direct fetch: 200 `{status: "file_only", source, page_url, reason_code}`
/// (the page is not requested). Any other address goes the general way (`link_web`, source
/// `web`): a card file behind the link or a card the page links or embeds; a page without a
/// card or behind a bot check answers `file_only` with `no_card_found` / `bot_protection`, an
/// address that is not public HTTPS on port 443 answers 400 `address_not_allowed`.
/// Errors: 400 `bad_url` / `bad_link`, and when
/// the fetch fails 404 `source_not_found`, 403 `source_blocked`, 429 `source_rate_limited`,
/// 504 `source_timeout`, 502 `source_failed` / `source_redirect` / `source_too_large` /
/// `source_bad_answer`. The log gets the source and status only, never the link.
pub(super) async fn post_import_link(State(st): State<AppState>, Json(body): Json<LinkBody>) -> ApiResult {
    let (target, bytes, extra) = match fetch_link(&st, &body.url, "link import").await? {
        Fetched::Card(target, bytes, extra) => (target, bytes, extra),
        Fetched::FileOnly(answer) => return Ok(answer),
    };
    import_link(&st, target.source(), target.page_url(), bytes, extra, "link import").await
}

fn log_link(st: &AppState, what: &'static str, source: &str, status: &str) {
    st.log().write(Entry::new(Level::Info, "server", "import", what).data(json!({ "source": source, "status": status })));
}

/// What a pasted link gave.
enum Fetched {
    /// The card file, and further ones that came with it (a zip of cards behind the link).
    Card(link::Target, Vec<u8>, Vec<Vec<u8>>),
    /// The 200 answer for a site that allows no direct fetch.
    FileOnly(axum::response::Response),
}

/// Detects the link and fetches the card file (the errors of `POST /import/link`).
async fn fetch_link(st: &AppState, url: &str, what: &'static str) -> Result<Fetched, ApiError> {
    let target = link::detect(url).inspect_err(|code| log_link(st, what, "", code)).map_err(ApiError::bad_request)?;
    let source = target.source();
    if let Some(reason) = source.file_only_reason() {
        log_link(st, what, source.code(), "file_only");
        let answer = json!({ "status": "file_only", "source": source.code(), "page_url": target.page_url(), "reason_code": reason });
        return Ok(Fetched::FileOnly(Json(answer).into_response()));
    }
    let file_only = |reason: &'static str| {
        log_link(st, what, source.code(), reason);
        let answer = json!({ "status": "file_only", "source": source.code(), "page_url": target.page_url(), "reason_code": reason });
        Fetched::FileOnly(Json(answer).into_response())
    };
    let endpoints = st.links.get();
    if let link::Target::Web { url } = &target {
        // The general way. A page that blocks programs or holds no card is the user's to open.
        let mut files = match link_web::fetch(endpoints.test_origin.clone(), url).await {
            Ok(files) => files,
            Err(WebError::NoCard) => return Ok(file_only("no_card_found")),
            Err(WebError::Fetch(link::FetchError::Blocked | link::FetchError::RateLimited)) => return Ok(file_only("bot_protection")),
            Err(WebError::Fetch(e)) => {
                log_link(st, what, source.code(), e.code());
                return Err(e.api());
            }
        };
        let first = files.remove(0);
        return Ok(Fetched::Card(target, first, files));
    }
    let bytes = link::fetch_card(&endpoints, &target).await.map_err(|e| {
        log_link(st, what, source.code(), e.code());
        e.api()
    })?;
    Ok(Fetched::Card(target, bytes, vec![]))
}

/// The fetched file through the normal import; answers like `POST /import/link`. `extra`: the
/// other cards of a zip behind the link, imported the same way and counted in `more`
/// (`{imported, duplicates, errors}`, only when there are any).
async fn import_link(st: &AppState, source: link::Source, page_url: String, bytes: Vec<u8>, extra: Vec<Vec<u8>>, what: &'static str) -> ApiResult {
    let file = format!("{}.png", source.code());
    let result = link::import_fetched(&st.hub, file.clone(), bytes, page_url.clone()).await?;
    log_link(st, what, source.code(), result.code.unwrap_or(result.status));
    let mut more = vec![];
    for bytes in extra {
        more.push(link::import_fetched(&st.hub, file.clone(), bytes, page_url.clone()).await?);
    }
    if result.status == "imported" || more.iter().any(|r| r.status == "imported") {
        st.hub.library_changed();
    }
    let mut answer = serde_json::to_value(&result).map_err(|_| ApiError::bad_request("broken"))?;
    answer["source"] = json!(source.code());
    if !more.is_empty() {
        let (imported, duplicates, errors) = import::counts(&more);
        answer["more"] = json!({ "imported": imported, "duplicates": duplicates, "errors": errors });
    }
    Ok(Json(answer).into_response())
}

/// `{url}`: fetches the card like `POST /import/link` (same sources, limits, errors and
/// `file_only` answer) but stores nothing; it is kept in memory (at most 24, 15 minutes) until
/// `POST /import/link/confirm`. Answers `{status: "preview", preview_id, source, page_url,
/// card: {name, creator, tags, description, first_mes, creator_notes} (texts cut to 600
/// characters), has_image, duplicate_of: character_id|null, more_cards}` (`more_cards`: further
/// cards of a zip behind the link, imported with it); a fetched file without a card:
/// `{status: "error", code, source}`.
pub(super) async fn post_link_preview(State(st): State<AppState>, Json(body): Json<LinkBody>) -> ApiResult {
    let (target, bytes, extra) = match fetch_link(&st, &body.url, "link preview").await? {
        Fetched::Card(target, bytes, extra) => (target, bytes, extra),
        Fetched::FileOnly(answer) => return Ok(answer),
    };
    let source = target.source();
    let page_url = target.page_url();
    let read = tokio::task::spawn_blocking(move || preview::read(source, page_url, bytes)).await?;
    let p = match read {
        Ok(mut p) => {
            p.extra = extra;
            p
        }
        Err(code) => {
            log_link(&st, "link preview", source.code(), code);
            return Ok(Json(json!({ "status": "error", "code": code, "source": source.code() })).into_response());
        }
    };
    let hash = p.hash.clone();
    let duplicate_of = st.hub.store(|s| s.character_by_hash(&hash))?;
    log_link(&st, "link preview", source.code(), "preview");
    let answer = json!({
        "status": "preview",
        "preview_id": p.id,
        "source": source.code(),
        "page_url": p.page_url,
        "card": p.card,
        "has_image": p.image.is_some(),
        "duplicate_of": duplicate_of,
        "more_cards": p.extra.len(),
    });
    st.previews.put(p);
    Ok(Json(answer).into_response())
}

/// The picture of a preview (WebP thumbnail). 404 `preview_expired` / `no_avatar`.
pub(super) async fn get_link_preview_image(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let p = st.previews.get(&id).ok_or(ApiError::not_found("preview_expired"))?;
    let (bytes, mime) = p.image.ok_or(ApiError::not_found("no_avatar"))?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (axum::http::header::CACHE_CONTROL, "private, no-cache"),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
pub(super) struct ConfirmBody {
    preview_id: String,
}

/// `{preview_id}`: imports exactly the previewed file through the normal import (duplicates,
/// thumbnail, `source_url`); answers like `POST /import/link`. Unknown or expired:
/// 404 `preview_expired`.
pub(super) async fn post_link_confirm(State(st): State<AppState>, Json(body): Json<ConfirmBody>) -> ApiResult {
    let p = st.previews.take(&body.preview_id).ok_or(ApiError::not_found("preview_expired"))?;
    import_link(&st, p.source, p.page_url, p.bytes, p.extra, "link confirm").await
}

#[derive(Deserialize)]
pub(super) struct FindQuery {
    refresh: Option<String>,
}

/// SillyTavern installations on this PC (cached until `?refresh=1`):
/// `{installs: [{path, user, characters, personas, avatars}], searched_ms, complete}`.
pub(super) async fn get_sillytavern_find(State(st): State<AppState>, Query(q): Query<FindQuery>) -> ApiResult {
    let mut cached = st.st_found.lock().await;
    let refresh = q.refresh.as_deref().is_some_and(|r| r == "1" || r == "true");
    if refresh || cached.is_none() {
        let found = tokio::task::spawn_blocking(|| sillytavern::search(&sillytavern::roots(), sillytavern::BUDGET)).await?;
        st.log().write(Entry::new(Level::Info, "server", "import", "sillytavern search").data(json!({
            "installs": found.installs.len(), "ms": found.searched_ms as u64, "complete": found.complete,
        })));
        *cached = Some(found);
    }
    Ok(Json(cached.as_ref().expect("filled above")).into_response())
}

/// Only plain file names: an id or key from a request or `settings.json` never leads out of its
/// folder. No `:` (on Windows `C:x.png` joined to a folder replaces the whole path), and only what
/// `Path::file_name` gives back unchanged.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', ':'])
        && !name.contains("..")
        && std::path::Path::new(name).file_name().is_some_and(|f| f == std::ffi::OsStr::new(name))
}

/// A path to a network share (`\\host\share`, `\\?\UNC\…`, `//host/…`): opening it would reach
/// out to another computer (and on Windows send the login there).
fn network_path(path: &str) -> bool {
    let unc = path.len() >= 2 && path.as_bytes()[..2].iter().all(|b| matches!(b, b'\\' | b'/'));
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        unc || matches!(std::path::Path::new(path).components().next(), Some(Component::Prefix(p)) if !matches!(p.kind(), Prefix::Disk(_)))
    }
    #[cfg(not(windows))]
    unc
}

/// The installation's user data that a request names. `path` must be the absolute path of a
/// SillyTavern folder on a drive of this PC (no `..` in it, no network share), `user` one of its
/// `data/<user>` folders (left out: the only one, else `default-user`). 400 `not_sillytavern` /
/// `bad_user`.
async fn st_install(path: &str, user: Option<&str>) -> Result<sillytavern::Install, ApiError> {
    let path = path.trim();
    let local = !network_path(path);
    let path = std::path::PathBuf::from(path);
    let ok = local && path.is_absolute() && !path.components().any(|c| matches!(c, std::path::Component::ParentDir));
    let mut installs = tokio::task::spawn_blocking(move || if ok { sillytavern::installs_at(&path) } else { vec![] }).await?;
    if installs.is_empty() {
        return Err(ApiError::bad_request("not_sillytavern"));
    }
    let chosen = match user {
        Some(user) => installs.iter().position(|i| i.user.as_deref() == Some(user)),
        None if installs.len() == 1 => Some(0),
        None => installs.iter().position(|i| i.user.as_deref() == Some("default-user")),
    };
    Ok(installs.swap_remove(chosen.ok_or(ApiError::bad_request("bad_user"))?))
}

/// The personas of `settings.json`: (avatar key, the persona as it would be imported or None
/// when it has no usable name).
fn st_persona_list(settings: &serde_json::Value) -> Vec<(String, Option<store::PersonaEdit>)> {
    let pu = &settings["power_user"];
    sillytavern::persona_keys(settings)
        .into_iter()
        .map(|key| {
            let d = &pu["persona_descriptions"][key.as_str()];
            let edit = pu["personas"][key.as_str()]
                .as_str()
                .and_then(|n| import::persona_edit(n, d["description"].as_str().unwrap_or(""), &d[import::RP_PERSONA_KEY]));
            (key, edit)
        })
        .collect()
}

/// A chat file as the app stores it. Err `empty`: no messages.
fn st_chat(file: &sillytavern::ChatFile, chat: rp_core::st_chat::StChat, hash: String) -> Result<store::ImportedChat, &'static str> {
    if chat.messages.is_empty() {
        return Err("empty");
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let created = chat.created.or_else(|| chat.messages.iter().find_map(|m| m.sent)).unwrap_or(now);
    let mut last = created;
    let messages = chat
        .messages
        .into_iter()
        .map(|m| {
            // A line without a readable date keeps the time of the one before.
            last = m.sent.unwrap_or(last);
            store::ImportedMsg {
                role: if m.is_user { "user" } else { "assistant" },
                content: m.text,
                created_at: last,
                swipes: (!m.swipes.is_empty()).then_some((m.swipes, m.swipe_id)),
            }
        })
        .collect();
    let name = file.id.rsplit('/').next().unwrap_or(&file.id);
    let stem = name.strip_suffix(".jsonl").unwrap_or(name).trim();
    let title = stem.chars().take(super::chats::TITLE_MAX).collect::<String>().trim().to_string();
    Ok(store::ImportedChat { title, created_at: created, hash, messages })
}

#[derive(Deserialize)]
pub(super) struct StQuery {
    path: String,
    user: Option<String>,
    /// Minutes east of UTC of the asking device (SillyTavern writes some dates in local time).
    tz: Option<i32>,
    card: Option<String>,
    persona: Option<String>,
}

/// What one SillyTavern installation holds, without importing anything (`?path=&user=&tz=`;
/// `path`/`user` as `POST /import/sillytavern`). Answers `{path, user, characters: [{id, name,
/// has_image, duplicate_of, error, chats}], personas: [{id, name, has_image, duplicate_of,
/// error}], chats: [{id, folder, card, messages, last_date, duplicate_of, error}]}`: `id` is the
/// card's file name, the persona's avatar key, the chat's `<folder>/<file>.jsonl`; `duplicate_of`
/// the app's character, persona or chat that has the same content; `card` the id of the card a
/// chat belongs to (null: none); `last_date` Unix seconds. ST is only read.
pub(super) async fn get_sillytavern_contents(State(st): State<AppState>, Query(q): Query<StQuery>) -> ApiResult {
    let install = st_install(&q.path, q.user.as_deref()).await?;
    let tz = crate::export::clamp_tz(q.tz.unwrap_or(0));
    let data = install.data.clone();
    let (cards, personas, chats) = tokio::task::spawn_blocking(move || {
        let cards = sillytavern::cards(&data);
        let personas: Vec<(String, Option<store::PersonaEdit>, bool)> = sillytavern::settings(&data)
            .map(|s| st_persona_list(&s))
            .unwrap_or_default()
            .into_iter()
            .map(|(key, edit)| {
                let has_image = plain(&key) && data.join(sillytavern::AVATARS).join(&key).is_file();
                (key, edit, has_image)
            })
            .collect();
        let chats: Vec<_> = sillytavern::chat_files(&data)
            .into_iter()
            .map(|f| {
                let read = sillytavern::read_chat(&f.path, tz).map(|(c, hash)| {
                    let last = c.messages.iter().rev().find_map(|m| m.sent).or(c.created);
                    (c.messages.len(), last, hash, c.character_name)
                });
                let card = sillytavern::match_card(&f.folder, read.as_ref().map_or("", |r| r.3.as_str()), &cards);
                (f, read, card)
            })
            .collect();
        (cards, personas, chats)
    })
    .await?;

    let hub = st.hub.clone();
    let body = tokio::task::spawn_blocking(move || {
        hub.store(|s| -> store::Res<serde_json::Value> {
            let mut card_list = vec![];
            let mut in_app = vec![];
            for (n, c) in cards.iter().enumerate() {
                let duplicate_of = match &c.hash {
                    Some(h) => s.character_by_hash(h)?,
                    None => None,
                };
                in_app.push(duplicate_of);
                let count = chats.iter().filter(|(_, _, card)| *card == Some(n)).count();
                card_list.push(json!({ "id": c.id, "name": c.name, "has_image": c.has_image, "duplicate_of": duplicate_of, "error": c.error, "chats": count }));
            }
            let mut persona_list = vec![];
            for (key, edit, has_image) in &personas {
                let (name, duplicate_of, error) = match edit {
                    Some(e) => {
                        let (name, description) = (e.name.clone().unwrap_or_default(), e.description.clone().unwrap_or_default());
                        let dup = s.persona_like(&name, &description)?;
                        (Some(name), dup, None)
                    }
                    None => (None, None, Some("bad_name")),
                };
                persona_list.push(json!({ "id": key, "name": name, "has_image": has_image, "duplicate_of": duplicate_of, "error": error }));
            }
            let mut chat_list = vec![];
            for (f, read, card) in &chats {
                let card_id = card.map(|i| cards[i].id.clone());
                // Already imported: to the character its card is in the app as.
                let character = card.and_then(|i| in_app[i]);
                chat_list.push(match read {
                    Ok((count, last, hash, _)) => json!({
                        "id": f.id, "folder": f.folder, "card": card_id, "messages": count, "last_date": last,
                        "duplicate_of": match character { Some(c) => s.chat_by_import_hash(hash, c)?, None => None }, "error": null,
                    }),
                    Err(code) => json!({
                        "id": f.id, "folder": f.folder, "card": card_id, "messages": 0, "last_date": null, "duplicate_of": null, "error": code,
                    }),
                });
            }
            Ok(json!({ "path": install.path, "user": install.user, "characters": card_list, "personas": persona_list, "chats": chat_list }))
        })
    })
    .await??;
    st.log().write(Entry::new(Level::Info, "server", "import", "sillytavern contents").data(json!({
        "characters": body["characters"].as_array().map_or(0, Vec::len),
        "personas": body["personas"].as_array().map_or(0, Vec::len),
        "chats": body["chats"].as_array().map_or(0, Vec::len),
    })));
    Ok(Json(body).into_response())
}

/// The picture of a card (`?path=&user=&card=<id>`) or persona (`&persona=<id>`) of a SillyTavern
/// installation as a WebP thumbnail (else as it is). 400 `bad_image` (neither or both given),
/// 404 `not_found` (no such card/persona) / `no_avatar`.
pub(super) async fn get_sillytavern_image(Query(q): Query<StQuery>) -> ApiResult {
    let install = st_install(&q.path, q.user.as_deref()).await?;
    let data = install.data;
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ApiError> {
        match (q.card, q.persona) {
            (Some(id), None) => {
                let path = sillytavern::card_files(&data.join("characters"))
                    .into_iter()
                    .find(|p| sillytavern::file_name(p) == id)
                    .ok_or(ApiError::not_found("not_found"))?;
                sillytavern::read_card(&path).ok().and_then(|(_, _, image)| image).ok_or(ApiError::not_found("no_avatar"))
            }
            (None, Some(key)) => {
                let known = sillytavern::settings(&data).is_some_and(|s| sillytavern::persona_keys(&s).contains(&key));
                if !known || !plain(&key) {
                    return Err(ApiError::not_found("not_found"));
                }
                let path = data.join(sillytavern::AVATARS).join(&key);
                match std::fs::metadata(&path) {
                    Ok(m) if m.is_file() && m.len() <= MAX_INPUT as u64 => std::fs::read(&path).map_err(|_| ApiError::not_found("no_avatar")),
                    _ => Err(ApiError::not_found("no_avatar")),
                }
            }
            _ => Err(ApiError::bad_request("bad_image")),
        }
    })
    .await??;
    let (bytes, mime) = match crate::imaging::thumb_webp(&bytes) {
        Some(webp) => (webp, "image/webp"),
        None => {
            let mime = store::image_mime(&bytes).ok_or(ApiError::not_found("no_avatar"))?;
            (bytes, mime)
        }
    };
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (axum::http::header::CACHE_CONTROL, "private, no-cache"),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
pub(super) struct StBody {
    path: String,
    user: Option<String>,
    job: Option<String>,
    tz: Option<i32>,
    characters: Option<Vec<String>>,
    personas: Option<Vec<String>>,
    chats: Option<Vec<String>>,
}

/// The persona files for the chosen keys: `settings.json` cut down to them plus their pictures
/// from `User Avatars`; and the keys that are not there.
fn st_persona_files(data: &std::path::Path, wanted: &[String]) -> (Vec<(String, Vec<u8>)>, Vec<String>) {
    let Some(mut settings) = sillytavern::settings(data) else { return (vec![], wanted.to_vec()) };
    let keys = sillytavern::persona_keys(&settings);
    let (found, missing): (Vec<String>, Vec<String>) = wanted.iter().cloned().partition(|k| keys.contains(k));
    if found.is_empty() {
        return (vec![], missing);
    }
    for list in ["personas", "persona_descriptions"] {
        if let Some(map) = settings["power_user"][list].as_object_mut() {
            map.retain(|k, _| found.contains(k));
        }
    }
    let mut files = vec![("settings.json".to_string(), serde_json::to_vec(&settings).expect("a JSON value always serializes"))];
    for key in found.iter().filter(|k| plain(k)) {
        let file = data.join(sillytavern::AVATARS).join(key);
        if let Ok(bytes) = std::fs::read(&file) {
            if bytes.len() <= MAX_INPUT {
                files.push((format!("{}/{key}", sillytavern::AVATARS), bytes));
            }
        }
    }
    (files, missing)
}

/// Every id once, in the order given.
fn unique(ids: Option<Vec<String>>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    ids.unwrap_or_default().into_iter().filter(|id| seen.insert(id.clone())).collect()
}

/// `{path, user?, job?, tz?, characters?: [id], personas?: [id], chats?: [id]}` (ids as in
/// `GET /import/sillytavern/contents`): imports exactly the chosen cards and personas through the
/// normal import (duplicates) and the chosen chats with all messages, their times and
/// all reply variants (the one chosen in SillyTavern stays active). A chat goes to the character
/// of its card: imported now or already in the app (same content). Answer and progress like
/// `POST /import`; results are `characters/<id>`, `personas/<id>` (`settings.json` when stored)
/// and `chats/<id>` with `chat_id` + `character_id` + `system_skipped` (lines SillyTavern marks
/// `is_system`, not taken over). Per item errors: `not_found` (no such id), `no_character` (the
/// chat's character is neither chosen nor in the app, or several cards share its name and none is
/// named like its folder), `empty`, `broken`, `too_large`; a chat already imported for the same
/// character is a `duplicate`. 400 `selection_required` (none of the
/// three lists given), `not_sillytavern`, `bad_user`, `bad_job`. ST is only read.
pub(super) async fn post_sillytavern_import(State(st): State<AppState>, Json(body): Json<StBody>) -> ApiResult {
    let id = job_id(body.job)?;
    if body.characters.is_none() && body.personas.is_none() && body.chats.is_none() {
        return Err(ApiError::bad_request("selection_required"));
    }
    let install = st_install(&body.path, body.user.as_deref()).await?;
    let data = install.data;
    let tz = crate::export::clamp_tz(body.tz.unwrap_or(0));
    let (want_cards, want_personas, want_chats) = (unique(body.characters), unique(body.personas), unique(body.chats));
    let mut results = vec![];

    // Personas first, so the job's last progress event comes when the cards are stored.
    if !want_personas.is_empty() {
        let read = data.clone();
        let (files, missing) = tokio::task::spawn_blocking(move || st_persona_files(&read, &want_personas)).await?;
        results.extend(missing.iter().map(|k| FileResult::error(&format!("personas/{k}"), "not_found")));
        if !files.is_empty() {
            results.extend(import_personas(&st, files).await?);
        }
    }

    let read = data.clone();
    let card_paths = tokio::task::spawn_blocking(move || sillytavern::card_files(&read.join("characters"))).await?;
    let mut job = Job::start(st.hub.clone(), id.clone()).await;
    for (n, want) in want_cards.iter().enumerate() {
        let name = format!("characters/{want}");
        let Some(file) = card_paths.iter().find(|p| sillytavern::file_name(p) == *want) else {
            results.push(FileResult::error(&name, "not_found"));
            continue;
        };
        // Read when the import budget has room (ST's file is only read).
        match tokio::fs::metadata(file).await {
            Ok(meta) if meta.len() > MAX_INPUT as u64 => job.refuse((n, 0), name, "too_large"),
            Ok(_) => job.add_file((n, 0), name, file.clone()),
            Err(_) => job.refuse((n, 0), name, "broken"),
        }
    }
    let card_results = job.finish().await?;

    if !want_chats.is_empty() {
        // The character of each card file this import stored or found as a duplicate.
        let stored: Vec<(String, i64)> = card_results
            .iter()
            .filter_map(|r| Some((r.file.strip_prefix("characters/")?.to_string(), r.character_id?)))
            .collect();
        let hub = st.hub.clone();
        let chat_results = tokio::task::spawn_blocking(move || -> Result<Vec<FileResult>, ApiError> {
            let files = sillytavern::chat_files(&data);
            let cards = sillytavern::cards(&data);
            let mut out = vec![];
            for want in want_chats {
                let name = format!("chats/{want}");
                let Some(file) = files.iter().find(|f| f.id == want) else {
                    out.push(FileResult::error(&name, "not_found"));
                    continue;
                };
                let (chat, hash) = match sillytavern::read_chat(&file.path, tz) {
                    Ok(read) => read,
                    Err(code) => {
                        out.push(FileResult::error(&name, code));
                        continue;
                    }
                };
                let card = sillytavern::match_card(&file.folder, &chat.character_name, &cards).map(|i| &cards[i]);
                let character = match card {
                    Some(c) => match stored.iter().find(|(id, _)| *id == c.id) {
                        Some((_, character)) => Some(*character),
                        None => match &c.hash {
                            Some(h) => hub.store(|s| s.character_by_hash(h))?,
                            None => None,
                        },
                    },
                    None => None,
                };
                let Some(character) = character else {
                    out.push(FileResult::error(&name, "no_character"));
                    continue;
                };
                let skipped = chat.system_skipped;
                let imported = match st_chat(file, chat, hash) {
                    Ok(c) => c,
                    Err(code) => {
                        out.push(FileResult::error(&name, code));
                        continue;
                    }
                };
                let (chat_id, new) = hub.store(|s| s.import_chat(character, &imported))?;
                out.push(FileResult::chat(&name, new, character, chat_id, skipped));
            }
            Ok(out)
        })
        .await??;
        results.extend(card_results);
        results.extend(chat_results);
    } else {
        results.extend(card_results);
    }
    answer(&st, "sillytavern imported", Some(&id), results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_only() {
        for ok in ["user-default.png", "1731670000-Wanderin.png", "a.b", "Ünïcode name.webp"] {
            assert!(plain(ok), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a\\b", "../x.png", "C:x.png", "C:", "x:y", "\\\\host\\share\\x.png"] {
            assert!(!plain(bad), "{bad}");
        }
    }

    /// A persona key in `settings.json` like `C:Cargo.toml` (on Windows a path relative to the
    /// current folder of drive C) never reads a file outside `User Avatars`.
    #[test]
    fn persona_key_with_a_drive_stays_in_its_folder() {
        let data = crate::sillytavern::tests::temp("drive-key");
        let key = "C:Cargo.toml";
        let settings = json!({ "power_user": { "personas": { key: "Jonas" }, "persona_descriptions": { key: { "description": "x" } } } });
        std::fs::write(data.join("settings.json"), serde_json::to_vec(&settings).unwrap()).unwrap();
        std::fs::create_dir_all(data.join(sillytavern::AVATARS)).unwrap();
        // The test runs in the crate's folder: on drive C the key reaches its Cargo.toml.
        let (files, missing) = st_persona_files(&data, &[key.to_string()]);
        assert!(missing.is_empty());
        assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["settings.json"]);
        std::fs::remove_dir_all(data).unwrap();
    }
}
