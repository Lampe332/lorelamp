//! `GET /library` (everything the start screen and the library show) and `PUT /settings` (also
//! `engine_port`, the managed model server's port, and `language`, the app's language).

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde::Deserialize;
use rp_core::prompt::Language;
use serde_json::{json, Value};

use super::personas::log_persona;
use crate::error::{ApiError, ApiResult};
use crate::guard::pc_only;
use crate::lan::ViaLan;
use crate::logs::{Entry, Level};
use crate::store;
use crate::AppState;

/// Colour themes of the web UI (`theme.ts`); the first is the default.
const THEMES: [&str; 6] = ["graphit", "glut", "mitternacht", "smaragd", "bordeaux", "papier"];

/// Layouts of the start screen (`theme.ts`); the first is the default.
const HOME_LAYOUTS: [&str; 5] = ["bisher", "dashboard", "bento", "buehne", "kommando"];

/// Layouts of the characters tab; the first is the default.
const LIBRARY_LAYOUTS: [&str; 3] = ["poster", "compact", "list"];

#[derive(Deserialize)]
pub(super) struct LibraryQuery {
    /// `chats`: only what a finished reply changes (recent chats, figures, and as `activity` the
    /// chat count and last chat time of every character that has a chat); anything else or none:
    /// everything.
    part: Option<String>,
}

/// Read on a connection of its own on a blocking thread, so a large library never holds the hub's
/// lock (chat, saving, streaming) nor a runtime worker; through the hub only when that connection
/// cannot be had (e.g. during a restore's migration).
pub(super) async fn get_library(State(st): State<AppState>, Query(q): Query<LibraryQuery>) -> ApiResult {
    let only_chats = q.part.as_deref() == Some("chats");
    let read = move |s: &store::Store| if only_chats { chats_part(s) } else { library(s) };
    let dir = st.data_dir.clone();
    let lib = match tokio::task::spawn_blocking(move || store::Store::reader(&dir).and_then(|s| read(&s))).await {
        Ok(Ok(lib)) => lib,
        _ => st.hub.store(|s| read(s))?,
    };
    Ok(Json(lib).into_response())
}

/// Everything the start screen and the library show.
fn library(s: &store::Store) -> store::Res<Value> {
    let theme = s.theme()?.filter(|t| THEMES.contains(&t.as_str())).unwrap_or_else(|| THEMES[0].into());
    let layout = s.home_layout()?.filter(|l| HOME_LAYOUTS.contains(&l.as_str())).unwrap_or_else(|| HOME_LAYOUTS[0].into());
    let library_layout = s.library_layout()?.filter(|l| LIBRARY_LAYOUTS.contains(&l.as_str())).unwrap_or_else(|| LIBRARY_LAYOUTS[0].into());
    Ok(json!({
        "characters": s.characters()?,
        "recent": s.recent(12)?,
        "user_name": s.user_name()?,
        "personas": s.personas()?,
        "active_persona_id": s.active_persona_id()?,
        "theme": theme,
        "home_layout": layout,
        "library_layout": library_layout,
        "language": s.language()?,
        "stats": s.stats()?,
    }))
}

/// What a finished reply changes: no card texts, tags or personas, a small part of the library.
fn chats_part(s: &store::Store) -> store::Res<Value> {
    Ok(json!({ "recent": s.recent(12)?, "stats": s.stats()?, "activity": s.activity()? }))
}

/// A change to the app settings; fields left out stay as they are.
#[derive(Deserialize)]
pub(super) struct Settings {
    user_name: Option<String>,
    theme: Option<String>,
    home_layout: Option<String>,
    library_layout: Option<String>,
    /// The app's language (`de`, `en`, `es`, `ru`): the UI on every device and the language the
    /// figures answer in from the next reply on.
    language: Option<String>,
    /// Port of the managed model server (`ENGINE_PORTS`); a wider type so a bad number is
    /// `bad_port`, not a parse error.
    engine_port: Option<i64>,
}

/// Ports the model server may use (no privileged ones).
const ENGINE_PORTS: std::ops::RangeInclusive<i64> = 1024..=65535;

/// What `check_settings` lets through: the trimmed name, the theme, the start layout, the
/// layout of the characters tab, the model server's port and the language.
type Checked<'a> = (Option<&'a str>, Option<&'a str>, Option<&'a str>, Option<&'a str>, Option<u16>, Option<Language>);

/// The trimmed name, the theme and the layouts to store, or the error code for a bad request.
fn check_settings(body: &Settings) -> Result<Checked<'_>, &'static str> {
    let name = body.user_name.as_deref().map(str::trim);
    if name.is_some_and(|n| n.is_empty() || n.chars().count() > 40) {
        return Err("bad_name");
    }
    let theme = body.theme.as_deref();
    if theme.is_some_and(|t| !THEMES.contains(&t)) {
        return Err("bad_theme");
    }
    let layout = body.home_layout.as_deref();
    if layout.is_some_and(|l| !HOME_LAYOUTS.contains(&l)) {
        return Err("bad_layout");
    }
    let library = body.library_layout.as_deref();
    if library.is_some_and(|l| !LIBRARY_LAYOUTS.contains(&l)) {
        return Err("bad_layout");
    }
    let port = match body.engine_port {
        Some(p) if !ENGINE_PORTS.contains(&p) => return Err("bad_port"),
        p => p.map(|p| p as u16),
    };
    let language = match body.language.as_deref() {
        Some(l) => Some(Language::parse(l).ok_or("bad_language")?),
        None => None,
    };
    if name.is_none() && theme.is_none() && layout.is_none() && library.is_none() && port.is_none() && language.is_none() {
        return Err("bad_settings");
    }
    Ok((name, theme, layout, library, port, language))
}

/// 403 `pc_only` for `engine_port` from a phone (the model server's port belongs to the PC).
pub(super) async fn put_settings(State(st): State<AppState>, via: Option<Extension<ViaLan>>, Json(body): Json<Settings>) -> ApiResult {
    if body.engine_port.is_some() {
        pc_only(via.map(|Extension(v)| v))?;
    }
    let (name, theme, layout, library, port, language) = check_settings(&body).map_err(ApiError::bad_request)?;
    let renamed = st.hub.store(|s| -> store::Res<Option<i64>> {
        let renamed = name.map(|name| s.set_user_name(name)).transpose()?;
        if let Some(theme) = theme {
            s.set_theme(theme)?;
        }
        if let Some(layout) = layout {
            s.set_home_layout(layout)?;
        }
        if let Some(library) = library {
            s.set_library_layout(library)?;
        }
        if let Some(port) = port {
            s.set_engine_port(port)?;
        }
        if let Some(language) = language {
            s.set_language(language)?;
        }
        Ok(renamed)
    })?;
    if let Some(id) = renamed {
        log_persona(&st, "persona renamed", id);
    }
    if let Some(language) = language {
        st.log().write(Entry::new(Level::Info, "server", "settings", "language set").data(json!({ "language": language })));
    }
    if let Some(port) = port {
        st.log().write(Entry::new(Level::Info, "engine", "config", "model server port set").data(json!({ "port": port })));
        // The supervisor moves the model server at its next check (never while a reply is written).
        if let Some(engine) = &st.engine {
            engine.wake();
        }
        st.hub.models_changed();
    }
    st.hub.library_changed();
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_checks() {
        let body = |name: Option<&str>, theme: Option<&str>| Settings { user_name: name.map(Into::into), theme: theme.map(Into::into), home_layout: None, library_layout: None, engine_port: None, language: None };
        let layout = |l: &str| Settings { user_name: None, theme: None, home_layout: Some(l.into()), library_layout: None, engine_port: None, language: None };
        let library = |l: &str| Settings { user_name: None, theme: None, home_layout: None, library_layout: Some(l.into()), engine_port: None, language: None };
        let port = |p: i64| Settings { user_name: None, theme: None, home_layout: None, library_layout: None, engine_port: Some(p), language: None };
        assert_eq!(check_settings(&body(Some("  Alex "), None)), Ok((Some("Alex"), None, None, None, None, None)));
        assert_eq!(check_settings(&body(None, Some("papier"))), Ok((None, Some("papier"), None, None, None, None)));
        assert_eq!(check_settings(&body(Some("Alex"), Some("glut"))), Ok((Some("Alex"), Some("glut"), None, None, None, None)));
        assert_eq!(check_settings(&body(Some("   "), None)), Err("bad_name"));
        assert_eq!(check_settings(&body(None, Some("neon"))), Err("bad_theme"));
        assert_eq!(check_settings(&body(None, None)), Err("bad_settings"));
        for id in HOME_LAYOUTS {
            assert_eq!(check_settings(&layout(id)), Ok((None, None, Some(id), None, None, None)));
        }
        for id in LIBRARY_LAYOUTS {
            assert_eq!(check_settings(&library(id)), Ok((None, None, None, Some(id), None, None)));
        }
        assert_eq!(check_settings(&library("bisher")), Err("bad_layout"));
        assert_eq!(check_settings(&library("")), Err("bad_layout"));
        assert_eq!(check_settings(&layout("grid")), Err("bad_layout"));
        assert_eq!(check_settings(&layout("")), Err("bad_layout"));
        assert_eq!(check_settings(&port(8081)), Ok((None, None, None, None, Some(8081), None)));
        for bad in [0, 80, 1023, 65536, -1] {
            assert_eq!(check_settings(&port(bad)), Err("bad_port"), "{bad}");
        }
        let language = |l: &str| Settings { user_name: None, theme: None, home_layout: None, library_layout: None, engine_port: None, language: Some(l.into()) };
        for l in Language::ALL {
            assert_eq!(check_settings(&language(l.as_str())), Ok((None, None, None, None, None, Some(l))));
        }
        for bad in ["", "DE", "fr", "de-DE", "deutsch"] {
            assert_eq!(check_settings(&language(bad)), Err("bad_language"), "{bad}");
        }
    }
}
