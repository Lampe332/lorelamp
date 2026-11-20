//! Import by link (`POST /import/link`): the pasted link is matched to a card site by host and
//! path. Chub and RisuRealm have public download APIs, so the card (with its picture) is fetched
//! there — one user action, at most two requests (Chub: card, then picture), HTTPS only,
//! redirects only to the same API hosts, with a timeout and a size limit, an honest user agent,
//! no cookies and no account. Other known sites do not allow it (private definitions, bot
//! protection, terms that forbid automated access, no public download API): for them the
//! answer explains how to get the file, and the page is never requested. Any other address
//! goes the general way (`link_web`): a card file behind the link, or a card linked from or
//! embedded in the page, fetched under the rules of `netguard`.
//!
//! What was fetched goes through the same import as a file (`import::prepare` → store, with
//! duplicates and thumbnail). Logs get the source and status only, never the link.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use reqwest::{redirect, StatusCode, Url};
use serde_json::{json, Value};

use crate::chat::Hub;
use crate::error::ApiError;
use crate::import::{self, FileResult, Prepared};
use crate::imaging;
use crate::store::NewItem;

/// `Lorelamp/<version> (+https://github.com/Lampe332/lorelamp)`.
pub fn user_agent() -> String {
    format!("Lorelamp/{} (+https://github.com/Lampe332/lorelamp)", env!("CARGO_PKG_VERSION"))
}

/// Whole request (connect, answer, body) of one fetch.
const TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Most redirects followed (each only to an allowed host).
const MAX_REDIRECTS: usize = 3;
/// Largest card JSON from Chub.
pub const MAX_CARD_JSON: usize = 8 << 20;
/// Largest picture or card PNG.
pub const MAX_CARD_IMAGE: usize = rp_core::formats::MAX_IMAGE as usize;
/// Longest link accepted.
const MAX_LINK: usize = 2048;

/// A card site the app knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Chub,
    RisuRealm,
    JanitorAi,
    JannyAi,
    CharacterTavern,
    HammerAi,
    Pygmalion,
    AiCharacterCards,
    Perchance,
    Wyvern,
    Backyard,
    CharaVault,
    /// Any other address: the general way.
    Web,
}

impl Source {
    pub fn code(self) -> &'static str {
        match self {
            Source::Chub => "chub",
            Source::RisuRealm => "risurealm",
            Source::JanitorAi => "janitorai",
            Source::JannyAi => "jannyai",
            Source::CharacterTavern => "character_tavern",
            Source::HammerAi => "hammerai",
            Source::Pygmalion => "pygmalion",
            Source::AiCharacterCards => "aicharactercards",
            Source::Perchance => "perchance",
            Source::Wyvern => "wyvern",
            Source::Backyard => "backyard",
            Source::CharaVault => "charavault",
            Source::Web => "web",
        }
    }

    /// Why the app does not fetch from this site (None: it does).
    pub fn file_only_reason(self) -> Option<&'static str> {
        match self {
            Source::Chub | Source::RisuRealm | Source::Web => None,
            // The definition is hidden unless the creator shows it; the data API is disallowed.
            Source::JanitorAi => Some("definition_private"),
            // Cloudflare challenge on every page and the API; never bypassed.
            Source::JannyAi => Some("bot_protection"),
            // Their terms forbid automated access / scraping / bots (read again 2026-11-16:
            // Character Tavern §4 "No bots", HammerAI §8.1, Pygmalion §7, Perchance "you will not
            // access the Services through automated or non-human means", WyvernChat §3A,
            // Backyard §7).
            Source::CharacterTavern | Source::HammerAi | Source::Pygmalion | Source::Perchance | Source::Wyvern | Source::Backyard => {
                Some("terms_forbid_automation")
            }
            // No documented download API: the old `wp-json/pngapi` endpoint is gone (404) and the
            // new site has no documented public download.
            Source::AiCharacterCards => Some("no_public_api"),
            // robots.txt disallows `/cards/` and `/api/` for everyone.
            Source::CharaVault => Some("robots_disallow"),
        }
    }

    fn from_host(host: &str) -> Option<Source> {
        let host = host.strip_prefix("www.").unwrap_or(host);
        Some(match host {
            "chub.ai" | "venus.chub.ai" | "characterhub.org" => Source::Chub,
            "realm.risuai.net" => Source::RisuRealm,
            "janitorai.com" => Source::JanitorAi,
            "jannyai.com" => Source::JannyAi,
            "character-tavern.com" => Source::CharacterTavern,
            "hammerai.com" => Source::HammerAi,
            "pygmalion.chat" => Source::Pygmalion,
            "aicharactercards.com" => Source::AiCharacterCards,
            "perchance.org" => Source::Perchance,
            "wyvern.chat" | "app.wyvern.chat" => Source::Wyvern,
            "backyard.ai" => Source::Backyard,
            "charavault.net" => Source::CharaVault,
            _ => return None,
        })
    }
}

/// What a link points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `chub.ai/characters/{creator}/{slug}` (also `characterhub.org`, `venus.chub.ai`).
    Chub { creator: String, slug: String },
    /// `realm.risuai.net/character/{id}`.
    Risu { id: String },
    /// A page of a site that allows no direct fetch.
    FileOnly { source: Source, page_url: String },
    /// Any other address (a card file, or a page that links or embeds one).
    Web { url: Url },
}

impl Target {
    pub fn source(&self) -> Source {
        match self {
            Target::Chub { .. } => Source::Chub,
            Target::Risu { .. } => Source::RisuRealm,
            Target::FileOnly { source, .. } => *source,
            Target::Web { .. } => Source::Web,
        }
    }

    /// The card's page, stored as `source_url`.
    pub fn page_url(&self) -> String {
        match self {
            Target::Chub { creator, slug } => format!("https://chub.ai/characters/{creator}/{slug}"),
            Target::Risu { id } => format!("https://realm.risuai.net/character/{id}"),
            Target::FileOnly { page_url, .. } => page_url.clone(),
            Target::Web { url } => url.to_string(),
        }
    }
}

/// Characters a Chub creator name or project slug can have.
fn chub_part(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')) && s != "." && s != ".."
}

fn risu_id(s: &str) -> bool {
    (8..=80).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Paths on chub.ai that are not a creator name.
const CHUB_RESERVED: &[&str] = &["characters", "lorebooks", "presets", "extensions", "users", "search", "chats", "tos", "privacy"];

/// Recognises a pasted link. Errors: `bad_url` (no http(s) link), `bad_link` (a known site, but
/// not a character page). A site the app does not know goes the general way ([`Target::Web`]).
pub fn detect(input: &str) -> Result<Target, &'static str> {
    let input = input.trim();
    if input.is_empty() || input.len() > MAX_LINK {
        return Err("bad_url");
    }
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let url = Url::parse(&with_scheme).map_err(|_| "bad_url")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("bad_url");
    }
    let host = url.host_str().ok_or("bad_url")?.to_ascii_lowercase();
    let Some(source) = Source::from_host(&host) else {
        // A real host name or an address, never a bare word.
        if !host.contains('.') && !host.contains(':') {
            return Err("bad_url");
        }
        let mut page = url.clone();
        page.set_fragment(None);
        return Ok(Target::Web { url: page });
    };
    let parts: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    match source {
        Source::Chub => {
            let (creator, slug) = match parts.as_slice() {
                ["characters", creator, slug, ..] => (*creator, *slug),
                [creator, slug] if !CHUB_RESERVED.contains(&creator.to_ascii_lowercase().as_str()) => (*creator, *slug),
                _ => return Err("bad_link"),
            };
            if !chub_part(creator) || !chub_part(slug) {
                return Err("bad_link");
            }
            Ok(Target::Chub { creator: creator.to_string(), slug: slug.to_string() })
        }
        Source::RisuRealm => match parts.as_slice() {
            ["character", id] if risu_id(id) => Ok(Target::Risu { id: id.to_ascii_lowercase() }),
            _ => Err("bad_link"),
        },
        _ => {
            let mut page = url.clone();
            let _ = page.set_scheme("https");
            page.set_fragment(None);
            Ok(Target::FileOnly { source, page_url: page.to_string() })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Where the APIs are

/// Base URLs of the APIs. Tests point them at a local server.
#[derive(Debug, Clone)]
pub struct Endpoints {
    /// Chub card API (`/api/characters/{creator}/{slug}?full=true`).
    pub chub_api: Url,
    /// Host of Chub's pictures (`max_res_url`).
    pub chub_images: Url,
    /// RisuRealm (`/api/v1/download/png-v3/{id}`).
    pub risu: Url,
    /// ONE origin the general way may fetch from although it is local and plain HTTP: the
    /// stand-in site of the tests (`RP_LINK_TEST_ORIGIN=http://127.0.0.1:<port>` for a debug
    /// test server). Never set in the app.
    pub test_origin: Option<Url>,
}

/// `RP_LINK_TEST_ORIGIN`, accepted only for this PC itself and only by a debug build (the test
/// servers): a release build never reads it, so nothing can loosen the address check there.
#[cfg(debug_assertions)]
fn test_origin() -> Option<Url> {
    let url = Url::parse(&std::env::var("RP_LINK_TEST_ORIGIN").ok()?).ok()?;
    (url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))).then_some(url)
}

#[cfg(not(debug_assertions))]
fn test_origin() -> Option<Url> {
    None
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        let url = |s: &str| Url::parse(s).expect("a valid built-in URL");
        Endpoints {
            chub_api: url("https://gateway.chub.ai/"),
            chub_images: url("https://avatars.charhub.io/"),
            risu: url("https://realm.risuai.net/"),
            test_origin: test_origin(),
        }
    }
}

/// The endpoints of a running server; [`crate::Running::link_sources`] can change them.
#[derive(Debug, Default)]
pub struct Sources(RwLock<Endpoints>);

impl Sources {
    pub fn get(&self) -> Endpoints {
        self.0.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set(&self, endpoints: Endpoints) {
        *self.0.write().unwrap_or_else(|p| p.into_inner()) = endpoints;
    }
}

// ---------------------------------------------------------------------------------------------
// Fetching

/// Why a fetch failed (each maps to one API error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// The site answered 404 / the card is gone.
    NotFound,
    /// The site refuses the download (e.g. the creator blocked it).
    Blocked,
    RateLimited,
    TooLarge,
    /// A redirect to a host that is not the API, or a non-HTTPS target.
    Redirect,
    Timeout,
    /// Network error or another status.
    Failed,
    /// The answer is not what the API documents.
    BadAnswer,
    /// The address is not on the public internet over HTTPS on the standard port.
    Private,
}

impl FetchError {
    pub fn code(self) -> &'static str {
        match self {
            FetchError::NotFound => "source_not_found",
            FetchError::Blocked => "source_blocked",
            FetchError::RateLimited => "source_rate_limited",
            FetchError::TooLarge => "source_too_large",
            FetchError::Redirect => "source_redirect",
            FetchError::Timeout => "source_timeout",
            FetchError::Failed => "source_failed",
            FetchError::BadAnswer => "source_bad_answer",
            FetchError::Private => "address_not_allowed",
        }
    }

    pub(crate) fn api(self) -> ApiError {
        use axum::http::StatusCode as S;
        let status = match self {
            FetchError::NotFound => S::NOT_FOUND,
            FetchError::Blocked => S::FORBIDDEN,
            FetchError::RateLimited => S::TOO_MANY_REQUESTS,
            FetchError::Timeout => S::GATEWAY_TIMEOUT,
            FetchError::Private => S::BAD_REQUEST,
            FetchError::TooLarge | FetchError::Redirect | FetchError::Failed | FetchError::BadAnswer => S::BAD_GATEWAY,
        };
        ApiError::new(status, self.code())
    }
}

/// Origins (scheme, host, port) a fetch may go to. A built-in endpoint is HTTPS, so its origin
/// only matches HTTPS URLs.
fn allowed(url: &Url, origins: &[Url]) -> bool {
    origins.iter().any(|o| o.origin() == url.origin())
}

/// GET with the limits: only to `origins`, redirects only within them, body at most `limit`.
async fn fetch(url: Url, origins: &[Url], accept: &str, limit: usize) -> Result<Vec<u8>, FetchError> {
    if !allowed(&url, origins) {
        return Err(FetchError::Redirect);
    }
    let allow: Vec<Url> = origins.to_vec();
    let policy = redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS || !allowed(attempt.url(), &allow) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    let client = reqwest::Client::builder()
        .user_agent(user_agent())
        .redirect(policy)
        .timeout(TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .https_only(origins.iter().all(|o| o.scheme() == "https"))
        .build()
        .map_err(|_| FetchError::Failed)?;
    let mut res = client.get(url).header("accept", accept).send().await.map_err(|e| if e.is_timeout() { FetchError::Timeout } else { FetchError::Failed })?;
    match res.status() {
        s if s.is_success() => {}
        s if s.is_redirection() => return Err(FetchError::Redirect),
        StatusCode::NOT_FOUND | StatusCode::GONE => return Err(FetchError::NotFound),
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => return Err(FetchError::Blocked),
        StatusCode::TOO_MANY_REQUESTS => return Err(FetchError::RateLimited),
        _ => return Err(FetchError::Failed),
    }
    if res.content_length().is_some_and(|n| n > limit as u64) {
        return Err(FetchError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| if e.is_timeout() { FetchError::Timeout } else { FetchError::Failed })? {
        if body.len() + chunk.len() > limit {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn join(base: &Url, path: &str) -> Result<Url, FetchError> {
    base.join(path).map_err(|_| FetchError::Failed)
}

/// Chub's card JSON (`node.definition`) as a V2 card, the way SillyTavern maps it.
pub fn chub_card(answer: &Value, creator: &str) -> Option<Value> {
    let node = &answer["node"];
    let d = &node["definition"];
    let s = |key: &str| d[key].as_str().unwrap_or("").to_string();
    let name = d["name"].as_str().or(node["name"].as_str())?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let mut data = json!({
        "name": name,
        "description": s("personality"),
        "personality": s("tavern_personality"),
        "scenario": s("scenario"),
        "first_mes": s("first_message"),
        "mes_example": s("example_dialogs"),
        "creator_notes": s("description"),
        "system_prompt": s("system_prompt"),
        "post_history_instructions": s("post_history_instructions"),
        "alternate_greetings": d["alternate_greetings"].as_array().map(|a| a.iter().filter(|g| g.is_string()).cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "tags": node["topics"].as_array().map(|a| a.iter().filter(|t| t.is_string()).cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "creator": creator,
        "character_version": "",
        "extensions": if d["extensions"].is_object() { d["extensions"].clone() } else { json!({}) },
    });
    if d["embedded_lorebook"].is_object() {
        data["character_book"] = d["embedded_lorebook"].clone();
    }
    Some(json!({ "spec": "chara_card_v2", "spec_version": "2.0", "data": data }))
}

/// Chub: card JSON, then its full-size picture; written as a card PNG (or JSON without picture).
async fn fetch_chub(ep: &Endpoints, creator: &str, slug: &str) -> Result<Vec<u8>, FetchError> {
    let mut url = join(&ep.chub_api, &format!("api/characters/{creator}/{slug}"))?;
    url.set_query(Some("full=true"));
    let body = fetch(url, std::slice::from_ref(&ep.chub_api), "application/json", MAX_CARD_JSON).await?;
    let answer: Value = serde_json::from_slice(&body).map_err(|_| FetchError::BadAnswer)?;
    let card = chub_card(&answer, creator).ok_or(FetchError::BadAnswer)?;
    let Some(image_url) = answer["node"]["max_res_url"].as_str().filter(|u| !u.is_empty()) else {
        return Ok(serde_json::to_vec(&card).expect("a JSON value always serializes"));
    };
    let image_url = Url::parse(image_url).map_err(|_| FetchError::BadAnswer)?;
    let image = fetch(image_url, std::slice::from_ref(&ep.chub_images), "image/*", MAX_CARD_IMAGE).await?;
    tokio::task::spawn_blocking(move || {
        let png = imaging::to_png(&image).ok_or(FetchError::BadAnswer)?;
        rp_core::formats::write_png(&card, Some(&png)).map_err(|_| FetchError::BadAnswer)
    })
    .await
    .map_err(|_| FetchError::Failed)?
}

/// RisuRealm: the documented download endpoint, PNG V3 (card + picture in one file).
async fn fetch_risu(ep: &Endpoints, id: &str) -> Result<Vec<u8>, FetchError> {
    let mut url = join(&ep.risu, &format!("api/v1/download/png-v3/{id}"))?;
    url.set_query(Some("non_commercial=true"));
    fetch(url, std::slice::from_ref(&ep.risu), "image/png", MAX_CARD_IMAGE).await
}

/// Fetches the card file of a direct source (not for file-only targets).
pub async fn fetch_card(ep: &Endpoints, target: &Target) -> Result<Vec<u8>, FetchError> {
    match target {
        Target::Chub { creator, slug } => fetch_chub(ep, creator, slug).await,
        Target::Risu { id } => fetch_risu(ep, id).await,
        Target::FileOnly { .. } | Target::Web { .. } => Err(FetchError::Failed),
    }
}

/// The fetched file through the normal import (duplicates, thumbnail), with the
/// card's page as `source_url`. One result, like one entry of `POST /import`.
pub(crate) async fn import_fetched(hub: &Arc<Hub>, file: String, bytes: Vec<u8>, page_url: String) -> Result<FileResult, ApiError> {
    let permit = import::reserve(bytes.len() as u64).await;
    let prepared = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        import::prepare(&bytes)
    })
    .await?;
    let mut items = match prepared {
        Prepared::Items(items) if !items.is_empty() => items,
        Prepared::Items(_) => return Ok(FileResult::error(&file, "no_card")),
        Prepared::Error(code) => return Ok(FileResult::error(&file, code)),
    };
    items.truncate(1);
    for item in &mut items {
        if let NewItem::Card(card) = item {
            card.source_url = Some(page_url.clone());
        }
    }
    let hub = hub.clone();
    let added = tokio::task::spawn_blocking(move || hub.store(|s| s.import_items(&items))).await??;
    Ok(FileResult::added(&file, added[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chub(c: &str, s: &str) -> Target {
        Target::Chub { creator: c.into(), slug: s.into() }
    }

    #[test]
    fn chub_links() {
        for link in [
            "https://chub.ai/characters/Some_One/a-card-1234abcd",
            "https://www.chub.ai/characters/Some_One/a-card-1234abcd",
            "http://chub.ai/characters/Some_One/a-card-1234abcd/",
            "chub.ai/characters/Some_One/a-card-1234abcd",
            "https://venus.chub.ai/characters/Some_One/a-card-1234abcd",
            "https://characterhub.org/characters/Some_One/a-card-1234abcd",
            "https://www.characterhub.org/characters/Some_One/a-card-1234abcd",
            "https://chub.ai/characters/Some_One/a-card-1234abcd/main?tab=chats#x",
            "https://chub.ai/Some_One/a-card-1234abcd",
            "  https://CHUB.AI/characters/Some_One/a-card-1234abcd  ",
        ] {
            assert_eq!(detect(link), Ok(chub("Some_One", "a-card-1234abcd")), "{link}");
        }
        assert_eq!(chub("a", "b").page_url(), "https://chub.ai/characters/a/b");
        assert_eq!(detect("https://chub.ai/lorebooks/someone/a-book"), Err("bad_link"));
        assert_eq!(detect("https://chub.ai/characters/someone"), Err("bad_link"));
        assert_eq!(detect("https://chub.ai/"), Err("bad_link"));
        assert_eq!(detect("https://chub.ai/search/x"), Err("bad_link"));
        assert_eq!(detect("https://chub.ai/characters/a%2F..%2Fb/c"), Err("bad_link"));
    }

    #[test]
    fn risu_links() {
        let uuid = "fa33d702-5e08-45a3-835d-26cdc325e8e1";
        let hex = "7adb0ed8d81855c820b3506980fb40f054ceef010ff0c4bab73730c0ebe92279";
        assert_eq!(detect(&format!("https://realm.risuai.net/character/{uuid}")), Ok(Target::Risu { id: uuid.into() }));
        assert_eq!(detect(&format!("https://realm.risuai.net/character/{uuid}/")), Ok(Target::Risu { id: uuid.into() }));
        assert_eq!(detect(&format!("realm.risuai.net/character/{hex}")), Ok(Target::Risu { id: hex.into() }));
        assert_eq!(detect(&format!("https://realm.risuai.net/character/{}", uuid.to_uppercase())), Ok(Target::Risu { id: uuid.into() }));
        assert_eq!(Target::Risu { id: uuid.into() }.page_url(), format!("https://realm.risuai.net/character/{uuid}"));
        assert_eq!(detect("https://realm.risuai.net/character/not-hex-zz"), Err("bad_link"));
        assert_eq!(detect("https://realm.risuai.net/"), Err("bad_link"));
        assert_eq!(detect(&format!("https://realm.risuai.net/module/{uuid}")), Err("bad_link"));
    }

    #[test]
    fn file_only_links() {
        for (link, source, reason) in [
            ("https://janitorai.com/characters/0b1c2d3e-0000-4000-8000-000000000000_character-a-card", Source::JanitorAi, "definition_private"),
            ("https://www.janitorai.com/characters/0b1c2d3e-0000-4000-8000-000000000000_character-a-card", Source::JanitorAi, "definition_private"),
            ("https://jannyai.com/characters/0b1c2d3e-0000-4000-8000-000000000000_character-a-card", Source::JannyAi, "bot_protection"),
            ("https://character-tavern.com/character/someone/a_card", Source::CharacterTavern, "terms_forbid_automation"),
            ("https://www.hammerai.com/characters/a-card", Source::HammerAi, "terms_forbid_automation"),
            ("https://pygmalion.chat/character/0b1c2d3e-0000-4000-8000-000000000000", Source::Pygmalion, "terms_forbid_automation"),
            ("https://aicharactercards.com/cards/4072", Source::AiCharacterCards, "no_public_api"),
            ("https://perchance.org/ai-character-chat?data=A_Card~0123456789abcdef0123456789abcdef.gz", Source::Perchance, "terms_forbid_automation"),
            ("https://app.wyvern.chat/characters/_abc123", Source::Wyvern, "terms_forbid_automation"),
            ("https://backyard.ai/hub/character/abc123", Source::Backyard, "terms_forbid_automation"),
            ("https://charavault.net/cards/a/b", Source::CharaVault, "robots_disallow"),
        ] {
            let target = detect(link).unwrap();
            assert_eq!(target.source(), source, "{link}");
            assert_eq!(source.file_only_reason(), Some(reason));
            assert!(target.page_url().starts_with("https://"), "{link}");
            assert!(matches!(target, Target::FileOnly { .. }));
        }
        assert_eq!(detect("http://character-tavern.com/character/a/b#top").unwrap().page_url(), "https://character-tavern.com/character/a/b");
        assert_eq!(Source::Chub.file_only_reason(), None);
        assert_eq!(Source::RisuRealm.file_only_reason(), None);
    }

    #[test]
    fn unknown_and_bad() {
        let web = |link: &str| match detect(link) {
            Ok(Target::Web { url }) => url.to_string(),
            other => panic!("{link}: {other:?}"),
        };
        assert_eq!(web("https://example.com/characters/a/b?x=1#top"), "https://example.com/characters/a/b?x=1");
        assert_eq!(web("notchub.ai/characters/a/b"), "https://notchub.ai/characters/a/b");
        assert_eq!(web("https://chub.ai.example.com/a.png"), "https://chub.ai.example.com/a.png");
        assert_eq!(Source::Web.file_only_reason(), None);
        assert_eq!(detect("just-a-word"), Err("bad_url"));
        assert_eq!(detect("not a link at all"), Err("bad_url"));
        assert_eq!(detect(""), Err("bad_url"));
        assert_eq!(detect("   "), Err("bad_url"));
        assert_eq!(detect("ftp://chub.ai/characters/a/b"), Err("bad_url"));
        assert_eq!(detect("javascript:alert(1)"), Err("bad_url"));
        assert_eq!(detect(&format!("https://chub.ai/characters/a/{}", "b".repeat(3000))), Err("bad_url"));
    }

    #[test]
    fn chub_mapping() {
        let answer = json!({ "node": { "topics": ["A", 3], "definition": {
            "name": "N", "personality": "desc", "tavern_personality": "pers", "description": "notes",
            "first_message": "hi", "alternate_greetings": ["x"], "embedded_lorebook": { "entries": [] }, "extensions": null,
        }}});
        let card = chub_card(&answer, "someone").unwrap();
        let d = &card["data"];
        assert_eq!(card["spec"], "chara_card_v2");
        assert_eq!((d["name"].as_str(), d["description"].as_str(), d["personality"].as_str()), (Some("N"), Some("desc"), Some("pers")));
        assert_eq!(d["creator_notes"], "notes");
        assert_eq!(d["first_mes"], "hi");
        assert_eq!(d["tags"], json!(["A"]));
        assert_eq!(d["creator"], "someone");
        assert_eq!(d["extensions"], json!({}));
        assert!(d["character_book"].is_object());
        assert!(chub_card(&json!({ "node": { "definition": { "name": " " } } }), "x").is_none());
    }

    #[test]
    fn origins() {
        let ep = Endpoints::default();
        assert!(allowed(&Url::parse("https://avatars.charhub.io/avatars/a/b/x.png").unwrap(), std::slice::from_ref(&ep.chub_images)));
        assert!(!allowed(&Url::parse("http://avatars.charhub.io/avatars/a/b/x.png").unwrap(), std::slice::from_ref(&ep.chub_images)));
        assert!(!allowed(&Url::parse("https://evil.example/x.png").unwrap(), &[ep.chub_images]));
        assert!(user_agent().starts_with("Lorelamp/") && user_agent().ends_with("(+https://github.com/Lampe332/lorelamp)"));
    }
}
