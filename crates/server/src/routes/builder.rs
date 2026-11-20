//! The card builder (`#/chars/new`): the local model writes one field from the plan, a draft is
//! saved as a new character through the same store call as an imported card (content hash,
//! thumbnail), and a stored character is read back as a draft to build on.
//!
//! Shared with phones: building and saving a card is what a phone may do with an imported one too.

use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use rp_core::builder::{self, Draft, Field, Plan};
use rp_core::prompt::ChatMsg;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{ApiError, ApiResult};
use crate::logs::{Entry, Level};
use crate::AppState;

/// How long a draft waits for a model that is loading (as a message does).
const LOAD_WAIT: Duration = Duration::from_secs(600);
/// The longest one field may take to write (a CPU model writes slowly).
const WRITE_TIMEOUT: Duration = Duration::from_secs(300);

fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(reqwest::Client::new)
}

#[derive(Deserialize)]
pub(super) struct DraftBody {
    #[serde(default)]
    plan: Plan,
    #[serde(default)]
    draft: Draft,
    field: Field,
}

/// Body `{plan, draft, field}`: the local model writes `field` (one generation at a time,
/// like a reply). Answers `{field, value}`: a text, or a list for `tags`. 409 `busy` (the model
/// writes), `benchmark`; 503 `no_model`; 502 `draft_failed`, `draft_empty`.
pub(super) async fn post_draft(State(st): State<AppState>, Json(body): Json<DraftBody>) -> ApiResult {
    let lang = st.hub.store(|s| s.language())?;
    let msgs = builder::draft_messages(lang, &body.plan, &body.draft, body.field);
    let _job = st.hub.side_begin().map_err(ApiError::conflict)?;
    let started = Instant::now();
    let result = write(&st, &msgs, body.field).await;
    let field = serde_json::to_value(body.field).unwrap_or(Value::Null);
    let facts = |extra: Value| json!({ "field": field, "ms": started.elapsed().as_millis() as u64, "extra": extra });
    match result {
        Ok(answer) => {
            let size = match &answer {
                builder::Answer::Text(t) => t.chars().count(),
                builder::Answer::List(l) => l.len(),
            };
            st.log().write(Entry::new(Level::Info, "builder", "draft", "card field drafted").data(facts(json!({ "size": size }))));
            Ok(Json(json!({ "field": field, "value": answer })).into_response())
        }
        Err(code) => {
            st.log().write(Entry::new(Level::Warn, "builder", "draft", format!("card field draft failed: {code}")).data(facts(Value::Null)));
            let status = if code == "no_model" { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::BAD_GATEWAY };
            Err(ApiError::new(status, code))
        }
    }
}

/// The model's text for the field, tidied. Err: `no_model`, `draft_failed`, `draft_empty`.
async fn write(st: &AppState, msgs: &[ChatMsg], field: Field) -> Result<builder::Answer, &'static str> {
    // An unloaded model is loaded again, as a message would do it.
    let woke = st.hub.load();
    let deadline = Instant::now() + LOAD_WAIT;
    loop {
        match st.hub.engine_status().await {
            "ready" => break,
            status if Instant::now() < deadline && (status == "loading" || (woke && status == "offline")) => tokio::time::sleep(Duration::from_millis(500)).await,
            _ => return Err("no_model"),
        }
    }
    let llama = st.hub.llama_url();
    let template: Value = http()
        .post(format!("{llama}/apply-template"))
        .json(&json!({ "messages": msgs, "chat_template_kwargs": { "enable_thinking": false } }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| "no_model")?
        .json()
        .await
        .map_err(|_| "draft_failed")?;
    let prompt = template["prompt"].as_str().ok_or("draft_failed")?;
    let body = json!({ "prompt": prompt, "n_predict": field.max_tokens(), "temperature": 0.85, "top_p": 0.95, "min_p": 0.05, "stream": false });
    let answer: Value = http()
        .post(format!("{llama}/completion"))
        .json(&body)
        .timeout(WRITE_TIMEOUT)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| "draft_failed")?
        .json()
        .await
        .map_err(|_| "draft_failed")?;
    let text = answer["content"].as_str().ok_or("draft_failed")?;
    builder::clean(field, text)
}

#[derive(Deserialize)]
pub(super) struct SaveBody {
    draft: Draft,
}

/// Body `{draft, from?}`: stores the draft as a new character (Character Card V3), the way an
/// imported card is stored. Answers the library entry of the new character. 400 `bad_name`.
pub(super) async fn post_save(State(st): State<AppState>, Json(body): Json<SaveBody>) -> ApiResult {
    super::characters::clean_card(body.draft.card())?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let raw = builder::card_json(&body.draft, now);
    // Through the same parser as every imported card, so what is stored is what an import stores.
    let imported = rp_core::card::parse(raw.to_string().as_bytes()).map_err(|e| ApiError::bad_request(e.code()))?;
    let info = st.hub.store(|s| s.add_character(&imported, None))?;
    let lore = body.draft.lore.iter().filter(|e| !e.content.trim().is_empty()).count();
    st.log().write(
        Entry::new(Level::Info, "builder", "save", "character built")
            .character(info.id)
            .data(json!({ "lore_entries": lore, "greetings": imported.card.alternate_greetings.len() })),
    );
    st.hub.library_changed();
    Ok(Json(info).into_response())
}

/// `{draft}`: a stored character as a draft to build on (lorebook, creator and version included).
/// 404 `character_missing`.
pub(super) async fn get_from(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let card = st.hub.store(|s| s.export_card(id, None))?.map_err(ApiError::not_found)?;
    let draft = builder::draft_from_raw(&card.raw).ok_or(ApiError::not_found("character_missing"))?;
    Ok(Json(json!({ "draft": draft })).into_response())
}
