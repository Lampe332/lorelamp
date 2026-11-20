//! Export of cards (one file, or a zip of many) and personas (SillyTavern backup zip with
//! pictures, RisuAI persona PNG), plus the thumbnail of a character's picture.

use std::io::{Cursor, Write};
use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rp_core::formats;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use super::{picture, picture_with};
use crate::error::{ApiError, ApiResult};
use crate::import::{rp_persona_ext, RP_PERSONA_KEY};
use crate::logs::{file_stamp, Entry, Level};
use crate::store::{ExportCard, Persona};
use crate::{imaging, AppState};

/// Most characters or personas one zip export takes.
const MAX_EXPORT_IDS: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Png,
    JsonV2,
    JsonV3,
}

impl Format {
    fn parse(s: Option<&str>) -> Result<Format, ApiError> {
        match s.unwrap_or("png") {
            "png" => Ok(Format::Png),
            "json_v2" => Ok(Format::JsonV2),
            "json_v3" => Ok(Format::JsonV3),
            _ => Err(ApiError::bad_request("bad_format")),
        }
    }

    fn code(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::JsonV2 => "json_v2",
            Format::JsonV3 => "json_v3",
        }
    }
}

/// A picture file as PNG (converted when it is JPEG/WebP/GIF). None: no picture or unreadable.
fn png_of(path: Option<PathBuf>) -> Option<Vec<u8>> {
    imaging::to_png(&std::fs::read(path?).ok()?)
}

/// The card as file bytes and the file extension.
fn card_file(c: &ExportCard, format: Format) -> (Vec<u8>, &'static str) {
    match format {
        Format::JsonV2 => (formats::write_json_v2(&c.raw), "json"),
        Format::JsonV3 => (formats::write_json_v3(&c.raw), "json"),
        Format::Png => {
            let png = png_of(c.avatar.as_ref().map(|(p, _)| p.clone()));
            // A damaged picture must not stop the export: then the placeholder is used.
            let bytes = formats::write_png(&c.raw, png.as_deref())
                .or_else(|_| formats::write_png(&c.raw, None))
                .expect("the placeholder is a valid PNG");
            (bytes, "png")
        }
    }
}

/// A file name from a card or persona name: no path or reserved characters, at most 80 chars.
fn safe_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .take(80)
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim().to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// `attachment` with an ASCII fallback and the UTF-8 name (RFC 6266 / 5987).
fn disposition(name: &str) -> String {
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    let encoded: String = name
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

fn download(bytes: Vec<u8>, mime: &'static str, name: &str) -> Response {
    ([(header::CONTENT_TYPE, mime.to_string()), (header::CONTENT_DISPOSITION, disposition(name))], bytes).into_response()
}

fn mime_of(ext: &str) -> &'static str {
    if ext == "png" {
        "image/png"
    } else {
        "application/json"
    }
}

/// `name.ext`, or `name (2).ext` … when the zip already has that name.
fn unique(taken: &mut std::collections::HashSet<String>, name: &str, ext: &str) -> String {
    let mut file = format!("{name}.{ext}");
    let mut n = 2;
    while !taken.insert(file.to_lowercase()) {
        file = format!("{name} ({n}).{ext}");
        n += 1;
    }
    file
}

fn zip_bytes(files: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>, ApiError> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        // PNGs are compressed already.
        let method = if name.ends_with(".png") { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        zip.start_file(name, SimpleFileOptions::default().compression_method(method)).map_err(ApiError::storage)?;
        zip.write_all(&bytes).map_err(ApiError::storage)?;
    }
    Ok(zip.finish().map_err(ApiError::storage)?.into_inner())
}

fn today() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    file_stamp(ms, 0)[..10].replace('-', "")
}

fn log_export(st: &AppState, what: &str, data: Value) {
    st.log().write(Entry::new(Level::Info, "server", "export", what.to_string()).data(data));
}

#[derive(Deserialize)]
pub(super) struct CardQuery {
    format: Option<String>,
    chat: Option<i64>,
}

/// `?format=png|json_v2|json_v3` (default png) `&chat=` (the card as that chat plays it). PNG
/// carries `chara` (V2) and `ccv3` (V3) and the picture (or a grey placeholder).
pub(super) async fn get_card_export(State(st): State<AppState>, Path(id): Path<i64>, Query(q): Query<CardQuery>) -> ApiResult {
    let format = Format::parse(q.format.as_deref())?;
    let hub = st.hub.clone();
    let (bytes, ext, name) = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let c = hub.store(|s| s.export_card(id, q.chat))?.map_err(ApiError::not_found)?;
        let (bytes, ext) = card_file(&c, format);
        Ok((bytes, ext, c.name))
    })
    .await??;
    log_export(&st, "character exported", json!({ "id": id, "format": format.code(), "chat": q.chat.is_some() }));
    Ok(download(bytes, mime_of(ext), &format!("{}.{ext}", safe_name(&name, "character"))))
}

#[derive(Deserialize)]
pub(super) struct BulkBody {
    ids: Vec<i64>,
    format: Option<String>,
}

/// Body `{ids: [..], format?}`: a zip with one file per character (400 `bad_ids` when empty or
/// too many, 404 `character_missing` when one is not there).
pub(super) async fn post_cards_export(State(st): State<AppState>, Json(body): Json<BulkBody>) -> ApiResult {
    let format = Format::parse(body.format.as_deref())?;
    if body.ids.is_empty() || body.ids.len() > MAX_EXPORT_IDS {
        return Err(ApiError::bad_request("bad_ids"));
    }
    let count = body.ids.len();
    let hub = st.hub.clone();
    let zip = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let mut taken = std::collections::HashSet::new();
        let mut files = vec![];
        for id in body.ids {
            let c = hub.store(|s| s.export_card(id, None))?.map_err(ApiError::not_found)?;
            let (bytes, ext) = card_file(&c, format);
            files.push((unique(&mut taken, &safe_name(&c.name, "character"), ext), bytes));
        }
        zip_bytes(files)
    })
    .await??;
    log_export(&st, "characters exported", json!({ "count": count, "format": format.code() }));
    Ok(download(zip, "application/zip", &format!("characters-{}.zip", today())))
}

#[derive(Deserialize)]
pub(super) struct PersonasQuery {
    /// Comma separated ids; all personas when left out.
    ids: Option<String>,
}

/// The picture of a persona as PNG.
fn persona_png(st: &AppState, p: &Persona) -> Result<Option<Vec<u8>>, ApiError> {
    let path = st.hub.store(|s| s.persona_avatar(p.id))?.map(|(path, _)| path);
    Ok(png_of(path))
}

/// Zip in the SillyTavern backup layout: `personas_YYYYMMDD.json` (`personas`,
/// `persona_descriptions`, `default_persona`; key = picture file name) plus `User Avatars/`
/// with the pictures. `?ids=1,2` for some only (404 `persona_missing` for an unknown one).
pub(super) async fn get_personas_export(State(st): State<AppState>, Query(q): Query<PersonasQuery>) -> ApiResult {
    let all = st.hub.store(|s| s.personas())?;
    let chosen: Vec<Persona> = match q.ids.as_deref() {
        None => all,
        Some(list) => {
            let ids: Vec<i64> = list
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().parse().map_err(|_| ApiError::bad_request("bad_ids")))
                .collect::<Result<_, _>>()?;
            if ids.is_empty() || ids.len() > MAX_EXPORT_IDS {
                return Err(ApiError::bad_request("bad_ids"));
            }
            ids.iter()
                .map(|id| all.iter().find(|p| p.id == *id).cloned().ok_or(ApiError::not_found("persona_missing")))
                .collect::<Result<_, _>>()?
        }
    };
    let active = st.hub.store(|s| s.active_persona_id())?;
    let st2 = st.clone();
    let count = chosen.len();
    let zip = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let (mut names, mut descriptions, mut files) = (Map::new(), Map::new(), vec![]);
        let mut default = Value::Null;
        for p in &chosen {
            let key = format!("rp-{}.png", p.id);
            names.insert(key.clone(), json!(p.name));
            descriptions.insert(
                key.clone(),
                json!({
                    "description": p.description, "position": 0, "depth": 2, "role": 0,
                    "lorebook": "", "title": "", "connections": [], RP_PERSONA_KEY: rp_persona_ext(p),
                }),
            );
            if p.id == active {
                default = json!(key);
            }
            if let Some(png) = persona_png(&st2, p)? {
                files.push((format!("User Avatars/{key}"), png));
            }
        }
        let backup = json!({ "personas": names, "persona_descriptions": descriptions, "default_persona": default });
        let json = serde_json::to_vec_pretty(&backup).map_err(ApiError::storage)?;
        files.insert(0, (format!("personas_{}.json", today()), json));
        zip_bytes(files)
    })
    .await??;
    log_export(&st, "personas exported", json!({ "count": count }));
    Ok(download(zip, "application/zip", &format!("personas-{}.zip", today())))
}

/// One persona as RisuAI persona PNG (chunk `persona` = `{name, personaPrompt, note}` plus the
/// app's own fields under `rp_app`); without a picture Risu's grey square.
pub(super) async fn get_persona_png(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let p = st.hub.store(|s| s.persona(id))?.ok_or(ApiError::not_found("persona_missing"))?;
    let st2 = st.clone();
    let (bytes, name) = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let persona = formats::Persona {
            name: p.name.clone(),
            prompt: p.description.clone(),
            note: String::new(),
            raw: json!({ RP_PERSONA_KEY: rp_persona_ext(&p) }),
        };
        let png = persona_png(&st2, &p)?;
        let bytes = formats::write_persona_png(&persona, png.as_deref())
            .or_else(|_| formats::write_persona_png(&persona, None))
            .expect("the placeholder is a valid PNG");
        Ok((bytes, p.name))
    })
    .await??;
    log_export(&st, "persona exported", json!({ "id": id }));
    Ok(download(bytes, "image/png", &format!("{}.png", safe_name(&name, "persona"))))
}

/// The WebP thumbnail of the character's picture; while there is none, the picture itself (not
/// cached for good, so the thumbnail is fetched once it exists).
pub(super) async fn get_thumb(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    match st.hub.store(|s| s.thumb(id)) {
        Ok(Some(path)) => picture(Ok(Some((path, "image/webp".into())))).await,
        _ => picture_with(st.hub.store(|s| s.avatar(id)), false).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(safe_name("Mara: \"Voss\"/x", "c"), "Mara_ _Voss__x");
        assert_eq!(safe_name("  ..  ", "c"), "c");
        let d = disposition("Jörg.png");
        assert!(d.contains("filename=\"J_rg.png\"") && d.contains("filename*=UTF-8''J%C3%B6rg.png"), "{d}");
        let mut taken = Default::default();
        assert_eq!(unique(&mut taken, "A", "png"), "A.png");
        assert_eq!(unique(&mut taken, "a", "png"), "a (2).png");
    }
}
