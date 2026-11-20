//! What the user wants to do with a model (`rp_core::purpose`): the stored sentence with its
//! needs, and "Beste Einstellung finden" for it: the measured row of the last benchmark that fits
//! the needs plus the sampler values derived from them, shown first and then written into the
//! profile.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use rp_core::bench::{self, Candidate, Row};
use rp_core::llama_args;
use rp_core::purpose::{self, Derived, Need, Purpose};
use rp_core::sampler::Sampler;
use serde::Deserialize;
use serde_json::{json, Value};

use super::models::log_model;
use crate::error::{ApiError, ApiResult};
use crate::fetch::catalog;
use crate::store::{self, Profile, ProfileEdit, Store};
use crate::AppState;

/// Longest sentence kept (characters).
const MAX_TEXT: usize = 500;

fn purpose_json(p: &Purpose) -> Value {
    let parsed = purpose::parse(&p.text);
    json!({ "text": p.text, "needs": p.needs, "figures": p.figures, "parsed": parsed.needs, "hits": parsed.hits, "all": Need::ALL })
}

/// `GET /models/purpose`: `{text, needs, figures, parsed, hits, all}`. `needs` are the ones in
/// effect (read from the sentence or corrected by hand), `parsed` what the sentence alone says,
/// `hits` the words behind each parsed need, `all` every need there is, in chip order.
pub(super) async fn get_purpose(State(st): State<AppState>) -> ApiResult {
    let p = st.hub.store(|s| s.use_purpose())?;
    Ok(Json(purpose_json(&p)).into_response())
}

#[derive(Deserialize)]
pub(super) struct PurposeBody {
    text: String,
    /// The needs chosen by hand; left out: read them from the sentence.
    #[serde(default)]
    needs: Option<Vec<Need>>,
}

/// `PUT /models/purpose` `{text, needs?}`: stores the sentence; without `needs` they are read from
/// it, with `needs` those count (the chips the user corrected). Answers like `GET`. 400
/// `text_too_long` above 500 characters. Every device reloads the suggestions (`{"type":"catalog"}`).
pub(super) async fn put_purpose(State(st): State<AppState>, Json(body): Json<PurposeBody>) -> ApiResult {
    if body.text.chars().count() > MAX_TEXT {
        return Err(ApiError::bad_request("text_too_long"));
    }
    let p = match &body.needs {
        Some(needs) => Purpose::with_needs(&body.text, needs),
        None => Purpose::from_text(&body.text),
    };
    st.hub.store(|s| s.set_use_purpose(&p))?;
    st.hub.notify(&json!({ "type": "catalog" }));
    Ok(Json(purpose_json(&p)).into_response())
}

/// What "Beste Einstellung finden" proposes for a profile and the stored purpose.
struct Advice {
    profile: Profile,
    purpose: Purpose,
    /// Which of the benchmark's three recommendations fits the needs.
    wanted: bench::Purpose,
    /// The measured row taken (the wanted one, else the nearest that was measured) and its place.
    row: Option<(usize, Row, bench::Purpose)>,
    measured_at: Option<i64>,
    sampler: Sampler,
    derived: Vec<Derived>,
    author_source: Option<String>,
}

impl Advice {
    fn candidate(&self) -> Option<Candidate> {
        self.row.as_ref().map(|r| r.1.candidate())
    }

    /// The start options with the measured row, through the same allowlist as an edit (what a profile
    /// saved before the allowlist still carries beyond it is left out).
    fn load_args(&self) -> Option<Vec<String>> {
        self.candidate().map(|c| llama_args::allowed_args(&bench::args_with(&self.profile.load_args, &c)).args)
    }

    /// The profile already has every proposed value.
    fn same(&self) -> bool {
        self.sampler == self.profile.sampler && self.load_args().is_none_or(|a| a == self.profile.load_args)
    }

    fn json(&self) -> Value {
        let measured = self.row.as_ref().map(|(at, r, used)| json!({ "row": at, "purpose": used, "ctx": r.ctx, "kv": r.kv, "n_cpu_moe": r.n_cpu_moe, "tg": r.tg, "pp": r.pp, "uncertain": r.uncertain, "at": self.measured_at }));
        json!({
            "purpose": purpose_json(&self.purpose),
            "bench_purpose": self.wanted,
            "measured": measured,
            "load_args": self.load_args(),
            "sampler": self.sampler,
            "derived": self.derived,
            "author_source": self.author_source,
            "same": self.same(),
        })
    }
}

fn advice(s: &Store, id: i64) -> store::Res<Option<Advice>> {
    let Some(profile) = s.profile(id)? else { return Ok(None) };
    let purpose = s.use_purpose()?;
    let wanted = purpose::bench_purpose(&purpose.needs);
    let result = s.bench_result(id)?;
    let row = result.as_ref().and_then(|r| {
        let rows: Vec<Row> = serde_json::from_value(r["rows"].clone()).ok()?;
        let picks: Vec<bench::Pick> = serde_json::from_value(r["picks"].clone()).ok()?;
        // The wanted recommendation; a run that has none for it (nothing above 15 tokens/s for
        // "Kontext") falls back to the balanced one, then the fastest.
        [wanted, bench::Purpose::Balanced, bench::Purpose::Speed].into_iter().find_map(|p| {
            let at = picks.iter().find(|x| x.purpose == p)?.row;
            rows.get(at).map(|row| (at, row.clone(), p))
        })
    });
    let file = std::path::Path::new(&profile.model_file).file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    let cat = catalog::embedded();
    let author = catalog::model_of_file(&cat, &file).and_then(|m| m.author_sampler.clone());
    let (sampler, derived) = purpose::sampler_for(&purpose.needs, purpose.figures, &profile.sampler, author.as_ref());
    let author_source = author.as_ref().and_then(|a| a.get("source")).and_then(Value::as_str).map(str::to_string);
    let measured_at = result.as_ref().and_then(|r| r.get("at")).and_then(Value::as_i64);
    Ok(Some(Advice { profile, purpose, wanted, row, measured_at, sampler, derived, author_source }))
}

/// `GET /models/profiles/{id}/advice`: what "Setzen" would write for the stored purpose:
/// `{purpose, bench_purpose, measured, load_args, sampler, derived, author_source, same}`.
/// `measured` is the benchmark row taken (`{row, purpose, ctx, kv, n_cpu_moe, tg, pp, uncertain, at}`;
/// null: never measured, only the sampler is proposed), `load_args` the start options with it,
/// `derived` the sampler values that follow from the purpose or the model's author
/// (`[{field, value, why}]`, `why`: author / long_chats / group / story / fast / short_replies /
/// long_replies), `sampler` the whole sampler after it, `same` whether the profile has it all.
/// 404 `profile_missing`.
pub(super) async fn get_advice(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let a = st.hub.store(|s| advice(s, id))?.ok_or_else(|| ApiError::not_found("profile_missing"))?;
    Ok(Json(a.json()).into_response())
}

/// `POST /models/profiles/{id}/advice/apply`: writes the advice into the profile: context, KV
/// cache type and `--n-cpu-moe` of the measured row (when there is one; the profile then counts as
/// measured) and the sampler. `{profile, engine, advice}`; `engine` is `restarting` when the
/// active profile got new start options, else `saved`. 404 `profile_missing`, 409 `benchmark`
/// while one runs, `busy` while a reply is being written and the start options would change, 400
/// `bad_sampler` (never from the rules themselves).
pub(super) async fn post_apply(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    if st.hub.bench_run().is_some() {
        return Err(ApiError::conflict("benchmark"));
    }
    let busy = st.hub.generating();
    let outcome = st.hub.store(|s| -> store::Res<Result<(Profile, Value, bool), ApiError>> {
        let Some(a) = advice(s, id)? else { return Ok(Err(ApiError::not_found("profile_missing"))) };
        if a.sampler.check().is_err() {
            return Ok(Err(ApiError::bad_request("bad_sampler")));
        }
        let args = a.load_args().filter(|args| *args != a.profile.load_args);
        if a.profile.active && busy && args.is_some() {
            return Ok(Err(ApiError::conflict("busy")));
        }
        let restart = a.profile.active && args.is_some();
        let edit = ProfileEdit { load_args: args, sampler: Some(a.sampler.clone()), measured: a.row.is_some().then_some(true), ..ProfileEdit::default() };
        let shown = a.json();
        Ok(s.update_profile(id, &edit)?.map(|p| (p, shown, restart)).ok_or_else(|| ApiError::not_found("profile_missing")))
    });
    let (profile, shown, restart) = outcome??;
    if restart {
        if let Some(e) = &st.engine {
            e.wake();
        }
    }
    log_model(&st, "purpose advice applied", json!({ "profile_id": id, "needs": shown["purpose"]["needs"], "measured": shown["measured"].is_object(), "derived": shown["derived"].as_array().map_or(0, Vec::len) }));
    st.hub.models_changed();
    Ok(Json(json!({ "profile": profile, "engine": if restart { "restarting" } else { "saved" }, "advice": shown })).into_response())
}
