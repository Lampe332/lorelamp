//! The welcome tour of a new installation (web UI `src/tutorial/`): which step it is at, or that it
//! is over. One value in the table `settings` (key `tutorial`), so the PC and every phone share it.
//! Never stored: a data folder without characters and model profiles is new (the tour starts), any
//! other one is not (an installation from before the tour never sees it by itself).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::error::{ApiError, ApiResult};
use crate::logs::{Entry, Level};
use crate::store::{Res, Store};
use crate::AppState;

const KEY: &str = "tutorial";
/// The steps in order, then the end.
const STEPS: [&str; 5] = ["welcome", "model", "figure", "persona", "chat"];
const DONE: &str = "done";

/// The stored step, or for a value never stored: `welcome` in a new data folder, else `done`.
fn current(s: &Store) -> Res<&'static str> {
    if let Some(v) = s.general(KEY)? {
        if let Some(step) = known(&v) {
            return Ok(step);
        }
    }
    Ok(if s.characters()?.is_empty() && s.profiles()?.is_empty() { STEPS[0] } else { DONE })
}

fn known(v: &str) -> Option<&'static str> {
    STEPS.iter().copied().chain([DONE]).find(|s| *s == v)
}

/// `GET /tutorial`: `{step}`, one of `welcome`, `model`, `figure`, `persona`, `chat`, `done`.
pub(super) async fn get_tutorial(State(st): State<AppState>) -> ApiResult {
    let step = st.hub.store(|s| current(s))?;
    Ok(Json(json!({ "step": step })).into_response())
}

#[derive(Deserialize)]
pub(super) struct TutorialBody {
    step: String,
}

/// `PUT /tutorial {step}`: the step the tour is at now (`done`: over; `welcome`: start again).
/// 400 `bad_step` for anything else.
pub(super) async fn put_tutorial(State(st): State<AppState>, Json(body): Json<TutorialBody>) -> ApiResult {
    let step = known(&body.step).ok_or_else(|| ApiError::bad_request("bad_step"))?;
    st.hub.store(|s| s.set_general(KEY, step))?;
    if step == DONE || step == STEPS[0] {
        st.log().write(Entry::new(Level::Info, "server", "settings", if step == DONE { "tutorial done" } else { "tutorial started" }));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{sample, temp_store};

    #[test]
    fn new_folder_starts_the_tour_and_a_stored_step_wins() {
        let (mut s, _dir) = temp_store();
        assert_eq!(current(&s).unwrap(), "welcome");
        s.set_general(KEY, "persona").unwrap();
        assert_eq!(current(&s).unwrap(), "persona");
        s.set_general(KEY, "done").unwrap();
        assert_eq!(current(&s).unwrap(), "done");
        // A value this version does not know counts as never stored.
        s.set_general(KEY, "later").unwrap();
        assert_eq!(current(&s).unwrap(), "welcome");
        assert_eq!(known("chat"), Some("chat"));
        assert_eq!(known("Chat"), None);
    }

    #[test]
    fn a_folder_with_characters_has_seen_it() {
        let (mut s, _dir) = temp_store();
        s.add_character(&sample(r#"{"name":"A"}"#), None).unwrap();
        assert_eq!(current(&s).unwrap(), "done");
    }
}
