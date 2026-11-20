//! Characters: import, card edits, soft delete with undo, and the profile picture.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rp_core::card::Card;
use serde::Deserialize;
use serde_json::json;

use super::{picture, Position};
use crate::error::{ApiError, ApiResult};
use crate::logs::{Entry, Level};
use crate::store;
use crate::AppState;

/// Body = the card file itself (PNG with embedded card, or JSON).
pub(super) async fn import_character(State(st): State<AppState>, body: Bytes) -> ApiResult {
    let imported = rp_core::card::parse(&body).map_err(|e| ApiError::bad_request(e.code()))?;
    let png = imported.is_png.then_some(&body[..]);
    let info = st.hub.store(|s| s.add_character(&imported, png))?;
    st.log().write(
        Entry::new(Level::Info, "server", "import", "character imported")
            .character(info.id)
            .data(json!({ "png": imported.is_png, "bytes": body.len() })),
    );
    st.hub.library_changed();
    refresh_thumb(&st, info.id);
    Ok(Json(info).into_response())
}

/// Makes the thumbnail of the character's current picture in the background.
fn refresh_thumb(st: &AppState, id: i64) {
    let hub = st.hub.clone();
    tokio::task::spawn_blocking(move || crate::thumbs::refresh(&hub, id));
}

/// Answer to a change of a character that answers whether it exists: 204, or 404
/// `character_missing`. Every device hears that the library changed.
fn character_changed(st: &AppState, result: store::Res<bool>) -> ApiResult {
    if !result? {
        return Err(ApiError::not_found("character_missing"));
    }
    st.hub.library_changed();
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub(super) async fn delete_character(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let result = st.hub.store(|s| s.set_deleted(id, true));
    character_changed(&st, result)
}

pub(super) async fn restore_character(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let result = st.hub.store(|s| s.set_deleted(id, false));
    character_changed(&st, result)
}

fn character_json(c: store::Character) -> Response {
    Json(json!({ "info": c.info, "card": c.card })).into_response()
}

pub(super) async fn get_character(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let c = st.hub.store(|s| s.character(id))?.ok_or(ApiError::not_found("character_missing"))?;
    Ok(character_json(c))
}

/// Tidies edited card fields. Err: the name is missing or too long (400 `bad_name`).
pub(super) fn clean_card(mut card: Card) -> Result<Card, ApiError> {
    card.name = card.name.trim().to_string();
    if card.name.is_empty() || card.name.chars().count() > 120 {
        return Err(ApiError::bad_request("bad_name"));
    }
    card.tags = card.tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    card.alternate_greetings.retain(|g| !g.trim().is_empty());
    Ok(card)
}

/// The own short text of a PUT body: `None` = field left out (unchanged), `Some(None)` = back to
/// the automatic one (`null` or `""`), `Some(Some(t))` = trimmed text. 400 `bad_tagline` when it
/// is not a string or longer than [`store::TAGLINE_MAX`].
fn tagline(body: &serde_json::Value) -> Result<Option<Option<String>>, ApiError> {
    let bad = || ApiError::bad_request("bad_tagline");
    match body.get("tagline") {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(t)) => {
            let t = t.trim();
            if t.chars().count() > store::TAGLINE_MAX {
                return Err(bad());
            }
            Ok(Some((!t.is_empty()).then(|| t.to_string())))
        }
        Some(_) => Err(bad()),
    }
}

/// Body = the edited card fields, plus optionally `tagline` (the own short text under the card,
/// ≤ 160 characters, `""`/null = automatic). Answers with the saved state.
pub(super) async fn put_character(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let tagline = tagline(&body)?;
    let card: Card = serde_json::from_value(body).map_err(|_| ApiError::bad_request("bad_card"))?;
    let card = clean_card(card)?;
    let c = st
        .hub
        .store(|s| -> store::Res<_> {
            if let Some(t) = &tagline {
                if !s.set_tagline(id, t.as_deref())? {
                    return Ok(None);
                }
            }
            s.update_character(id, &card)
        })?
        .ok_or(ApiError::not_found("character_missing"))?;
    st.hub.library_changed();
    Ok(character_json(c))
}

pub(super) async fn get_avatar(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    picture(st.hub.store(|s| s.avatar(id))).await
}

/// Body = the image file (PNG, JPEG, WebP or GIF).
pub(super) async fn put_avatar(State(st): State<AppState>, Path(id): Path<i64>, body: Bytes) -> ApiResult {
    let mime = store::image_mime(&body).ok_or(ApiError::bad_request("bad_image"))?;
    let result = st.hub.store(|s| s.set_avatar(id, Some((&body, mime))));
    refresh_thumb(&st, id);
    character_changed(&st, result)
}

/// Back to the card's own picture.
pub(super) async fn delete_avatar(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let result = st.hub.store(|s| s.set_avatar(id, None));
    refresh_thumb(&st, id);
    character_changed(&st, result)
}

/// Which part of the picture shows where it is cropped.
pub(super) async fn put_avatar_position(State(st): State<AppState>, Path(id): Path<i64>, Json(pos): Json<Position>) -> ApiResult {
    let result = st.hub.store(|s| s.set_avatar_pos(id, pos.x, pos.y));
    character_changed(&st, result)
}

#[derive(Deserialize)]
pub(super) struct FavoriteBody {
    favorite: bool,
}

/// Body `{favorite: bool}`: the star in the library.
pub(super) async fn put_favorite(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<FavoriteBody>) -> ApiResult {
    let result = st.hub.store(|s| s.set_favorite(id, body.favorite));
    character_changed(&st, result)
}

#[derive(Deserialize)]
pub(super) struct TagsBody {
    tags: Vec<String>,
}

/// Body `{tags: [..]}`: replaces the user's own tags of a character; answers `{tags}` as stored
/// (trimmed, without duplicates; 400 `bad_tag` for one that is longer than `TAG_MAX`).
pub(super) async fn put_tags(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<TagsBody>) -> ApiResult {
    if body.tags.iter().any(|t| !t.trim().is_empty() && store::clean_tag(t).is_none()) {
        return Err(ApiError::bad_request("bad_tag"));
    }
    let tags = st.hub.store(|s| s.set_character_tags(id, &body.tags))?.ok_or(ApiError::not_found("character_missing"))?;
    st.hub.library_changed();
    Ok(Json(json!({ "tags": tags })).into_response())
}

/// `{tags: [{name, count}]}`: every own tag in use.
pub(super) async fn get_tags(State(st): State<AppState>) -> ApiResult {
    let tags = st.hub.store(|s| s.tags())?;
    Ok(Json(json!({ "tags": tags })).into_response())
}

/// Most characters one bulk action takes.
const MAX_BULK: usize = 1000;

#[derive(Deserialize)]
pub(super) struct BulkBody {
    ids: Vec<i64>,
    action: String,
    tag: Option<String>,
}

/// Body `{ids, action, tag?}` with action `delete` | `restore` (its undo) | `tag` | `untag` |
/// `favorite` | `unfavorite`; answers `{changed}`. 400 `bad_ids`, `bad_action`, `bad_tag`.
pub(super) async fn post_bulk(State(st): State<AppState>, Json(body): Json<BulkBody>) -> ApiResult {
    if body.ids.is_empty() || body.ids.len() > MAX_BULK {
        return Err(ApiError::bad_request("bad_ids"));
    }
    let tag = body.tag.as_deref().unwrap_or("");
    let action = match body.action.as_str() {
        "delete" => store::Bulk::Delete,
        "restore" => store::Bulk::Restore,
        "favorite" => store::Bulk::Favorite(true),
        "unfavorite" => store::Bulk::Favorite(false),
        "tag" | "untag" if store::clean_tag(tag).is_none() => return Err(ApiError::bad_request("bad_tag")),
        "tag" => store::Bulk::Tag(tag),
        "untag" => store::Bulk::Untag(tag),
        _ => return Err(ApiError::bad_request("bad_action")),
    };
    let changed = st.hub.store(|s| s.bulk_characters(&body.ids, action))?;
    st.log().write(Entry::new(Level::Info, "server", "library", "bulk action").data(json!({ "action": body.action, "count": body.ids.len(), "changed": changed })));
    st.hub.library_changed();
    Ok(Json(json!({ "changed": changed })).into_response())
}
