//! Personas: the list, create/edit/delete, the active one, their pictures, and binding chats and
//! characters to one (plus who wins when a card describes `{{user}}` differently, and the check
//! of a card against a persona).

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use rp_core::prompt::Priority;
use serde::Deserialize;
use serde_json::json;

use super::{picture, Position};
use crate::error::{ApiError, ApiResult};
use crate::logs::{Entry, Level};
use crate::store::{self, Bind, PersonaDelete, PersonaEdit, UserCheck};
use crate::AppState;

/// Longest persona description (characters).
const PERSONA_TEXT_MAX: usize = 20_000;

/// Longest pronouns, and longest appearance / personality / background (characters).
const PERSONA_PRONOUNS_MAX: usize = 40;
const PERSONA_FIELD_MAX: usize = 5_000;

/// Persona events go to the log with the id only, never name or description.
pub(crate) fn log_persona(st: &AppState, what: &str, id: i64) {
    st.log().write(Entry::new(Level::Info, "server", "persona", what).data(json!({ "id": id })));
}

/// The edit with name and pronouns trimmed, or the error code for a bad request.
fn check_persona(mut body: PersonaEdit) -> Result<PersonaEdit, &'static str> {
    let too_long = |f: &Option<String>, max: usize| f.as_ref().is_some_and(|t| t.chars().count() > max);
    body.name = body.name.map(|n| n.trim().to_string());
    body.pronouns = body.pronouns.map(|p| p.trim().to_string());
    if body.name.as_ref().is_some_and(|n| n.is_empty() || n.chars().count() > 40) {
        return Err("bad_name");
    }
    if too_long(&body.description, PERSONA_TEXT_MAX) {
        return Err("bad_description");
    }
    if too_long(&body.pronouns, PERSONA_PRONOUNS_MAX) {
        return Err("bad_pronouns");
    }
    for (field, code) in [(&body.appearance, "bad_appearance"), (&body.personality, "bad_personality"), (&body.background, "bad_background")] {
        if too_long(field, PERSONA_FIELD_MAX) {
            return Err(code);
        }
    }
    if body.card_priority.as_deref().is_some_and(|p| Priority::parse(p).is_none()) {
        return Err("bad_priority");
    }
    Ok(body)
}

fn persona_missing() -> ApiError {
    ApiError::not_found("persona_missing")
}

/// Answer to a change of a persona that answers whether it exists: 204 after `what` went to the
/// log, or 404 `persona_missing`.
fn persona_changed(st: &AppState, what: &str, id: i64, result: store::Res<bool>) -> ApiResult {
    if !result? {
        return Err(persona_missing());
    }
    log_persona(st, what, id);
    st.hub.library_changed();
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Body of the routes that bind a chat or a character to a persona.
#[derive(Deserialize)]
pub(super) struct PersonaRef {
    persona_id: Option<i64>,
}

/// Answer to a binding: 204, 404 `missing` (no such chat or character) or 404 `persona_missing`.
fn bound(st: &AppState, result: store::Res<Bind>, missing: &'static str) -> ApiResult {
    match result? {
        Bind::Done => {
            st.hub.library_changed();
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        Bind::NoTarget => Err(ApiError::not_found(missing)),
        Bind::NoPersona => Err(persona_missing()),
    }
}

/// Body `{persona_id: number|null}`: binds the chat to that persona, or (null) lets it follow
/// the active one. Every device gets the chat's `history` snapshot and a `library` event.
pub(super) async fn put_chat_persona(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<PersonaRef>) -> ApiResult {
    let result = st.hub.set_chat_persona(id, body.persona_id);
    if matches!(result, Ok(Bind::Done)) {
        st.log().write(Entry::new(Level::Info, "server", "persona", "chat persona set").chat(id).data(json!({ "persona_id": body.persona_id })));
    }
    bound(&st, result, "chat_missing")
}

/// Body `{persona_id: number|null}`: new chats with this character are bound to that persona
/// (null: they follow the active one). Existing chats keep their binding.
pub(super) async fn put_default_persona(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<PersonaRef>) -> ApiResult {
    let result = st.hub.store(|s| s.set_default_persona(id, body.persona_id));
    if matches!(result, Ok(Bind::Done)) {
        st.log().write(
            Entry::new(Level::Info, "server", "persona", "default persona set").character(id).data(json!({ "persona_id": body.persona_id })),
        );
    }
    bound(&st, result, "character_missing")
}

/// Body `{priority: null|"persona"|"card"}`: who wins when this character's card describes
/// `{{user}}` differently from the chat's persona; null = as the persona says.
#[derive(Deserialize)]
pub(super) struct PriorityRef {
    priority: Option<String>,
}

pub(super) async fn put_user_priority(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<PriorityRef>) -> ApiResult {
    let priority = match body.priority.as_deref().map(Priority::parse) {
        None => None,
        Some(Some(p)) => Some(p),
        Some(None) => return Err(ApiError::bad_request("bad_priority")),
    };
    if !st.hub.store(|s| s.set_user_priority(id, priority))? {
        return Err(ApiError::not_found("character_missing"));
    }
    st.log().write(
        Entry::new(Level::Info, "server", "persona", "user priority set")
            .character(id)
            .data(json!({ "priority": priority.map(Priority::as_str) })),
    );
    st.hub.library_changed();
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub(super) struct UserCheckQuery {
    persona_id: Option<i64>,
}

/// What the card says about `{{user}}` and how it fits the persona `?persona_id=`, else the
/// persona of a new chat with this character: `{status, persona_id, items: [...]}`.
pub(super) async fn get_user_check(State(st): State<AppState>, Path(id): Path<i64>, Query(q): Query<UserCheckQuery>) -> ApiResult {
    match st.hub.store(|s| s.user_check(id, q.persona_id))? {
        UserCheck::Done { persona_id, report } => {
            Ok(Json(json!({ "status": report.status, "persona_id": persona_id, "items": report.items })).into_response())
        }
        UserCheck::NoCharacter => Err(ApiError::not_found("character_missing")),
        UserCheck::NoPersona => Err(persona_missing()),
    }
}

pub(super) async fn get_personas(State(st): State<AppState>) -> ApiResult {
    let list = st.hub.store(|s| -> store::Res<_> {
        Ok(json!({ "personas": s.personas()?, "active_persona_id": s.active_persona_id()? }))
    })?;
    Ok(Json(list).into_response())
}

/// Body `{name, description?, pronouns?, appearance?, personality?, background?, every_turn?,
/// card_priority?: "persona"|"card"}` (else 400 `bad_priority`). Answers with
/// the new persona (not active yet).
pub(super) async fn post_persona(State(st): State<AppState>, Json(body): Json<PersonaEdit>) -> ApiResult {
    let edit = check_persona(body).map_err(ApiError::bad_request)?;
    if edit.name.is_none() {
        return Err(ApiError::bad_request("bad_name"));
    }
    let p = st.hub.store(|s| s.add_persona(&edit))?;
    log_persona(&st, "persona created", p.id);
    st.hub.library_changed();
    Ok(Json(p).into_response())
}

/// Body: any of the fields of `post_persona`; fields left out stay, "" empties one (not the name).
/// Answers with the saved persona.
pub(super) async fn put_persona(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<PersonaEdit>) -> ApiResult {
    let edit = check_persona(body).map_err(ApiError::bad_request)?;
    let p = st.hub.store(|s| s.update_persona(id, &edit))?.ok_or_else(persona_missing)?;
    log_persona(&st, "persona changed", id);
    st.hub.library_changed();
    Ok(Json(p).into_response())
}

/// Deletes for good. 409 `last_persona` for the last one; answers `{active_persona_id}`.
pub(super) async fn delete_persona(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    match st.hub.store(|s| s.delete_persona(id))? {
        PersonaDelete::Done(active) => {
            log_persona(&st, "persona deleted", id);
            st.hub.library_changed();
            Ok(Json(json!({ "active_persona_id": active })).into_response())
        }
        PersonaDelete::Missing => Err(persona_missing()),
        PersonaDelete::Last => Err(ApiError::conflict("last_persona")),
    }
}

/// Makes this persona `{{user}}` on every device.
pub(super) async fn activate_persona(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let result = st.hub.store(|s| s.activate_persona(id));
    persona_changed(&st, "persona switched", id, result)
}

pub(super) async fn get_persona_avatar(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    picture(st.hub.store(|s| s.persona_avatar(id))).await
}

/// Body = the image file (PNG, JPEG, WebP or GIF).
pub(super) async fn put_persona_avatar(State(st): State<AppState>, Path(id): Path<i64>, body: Bytes) -> ApiResult {
    let mime = store::image_mime(&body).ok_or(ApiError::bad_request("bad_image"))?;
    let result = st.hub.store(|s| s.set_persona_avatar(id, Some((&body, mime))));
    persona_changed(&st, "persona picture changed", id, result)
}

pub(super) async fn delete_persona_avatar(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let result = st.hub.store(|s| s.set_persona_avatar(id, None));
    persona_changed(&st, "persona picture changed", id, result)
}

pub(super) async fn put_persona_avatar_position(State(st): State<AppState>, Path(id): Path<i64>, Json(pos): Json<Position>) -> ApiResult {
    let result = st.hub.store(|s| s.set_persona_avatar_pos(id, pos.x, pos.y));
    persona_changed(&st, "persona picture changed", id, result)
}
