//! The general way of the link import, for any address the app has no own rule for:
//!
//! 1. The link is a card file itself (PNG, JSON, CHARX, a zip of cards, …): taken as it is.
//!    Links to a file's page on GitHub, Hugging Face, Pastebin and Dropbox are turned into the
//!    file's own address first.
//! 2. The link is a web page: a card embedded in the page (JSON in a `<script>`), else a card
//!    file the page links to or shows (`<a href>`, `<img src>`, `og:image`), tried in the order
//!    of how likely they are, at most [`MAX_CANDIDATES`] of them.
//!
//! One user action, one deadline ([`BUDGET`]) for everything, every request through
//! [`netguard`](crate::netguard). A page that answers with a bot check or wants a login is
//! never worked around: the caller tells the user to open the page and drop the file.

use std::time::Duration;

use reqwest::Url;
use rp_core::formats::{self, Content};
use serde_json::Value;

use crate::imaging;
use crate::import;
use crate::link::{FetchError, MAX_CARD_IMAGE};
use crate::netguard::Guard;

/// Everything one pasted link may take.
pub const BUDGET: Duration = Duration::from_secs(30);
/// Largest web page read for links and embedded cards.
pub const MAX_PAGE: usize = 4 << 20;
/// Most files of a page that are fetched to look for a card.
pub const MAX_CANDIDATES: usize = 6;
/// Most cards taken from one zip behind a link.
pub const MAX_ZIP_CARDS: usize = 50;
/// Most bytes the entries of one zip behind a link may unpack to together.
pub const MAX_ZIP_UNPACKED: u64 = 256 << 20;

const ACCEPT_ANY: &str = "image/png, application/json, text/html;q=0.9, */*;q=0.5";
const ACCEPT_FILE: &str = "image/png, application/json, */*;q=0.5";

/// Why the general way brought no card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebError {
    Fetch(FetchError),
    /// Neither a card file nor a page with one.
    NoCard,
}

impl From<FetchError> for WebError {
    fn from(e: FetchError) -> WebError {
        WebError::Fetch(e)
    }
}

/// The address of the file behind a link to a file's page on a few well-known hosts.
pub fn file_address(url: &Url) -> Url {
    let host = url.host_str().unwrap_or("").trim_start_matches("www.").to_ascii_lowercase();
    let parts: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let built = match (host.as_str(), parts.as_slice()) {
        // github.com/{owner}/{repo}/blob/{ref}/{path…} → raw.githubusercontent.com/{owner}/{repo}/{ref}/{path…}
        ("github.com", [owner, repo, "blob", rest @ ..]) if !rest.is_empty() => {
            Url::parse(&format!("https://raw.githubusercontent.com/{owner}/{repo}/{}", rest.join("/"))).ok()
        }
        // huggingface.co/…/blob/{ref}/{path…} → …/resolve/{ref}/{path…}
        ("huggingface.co", p) if p.contains(&"blob") => {
            let path: Vec<&str> = p.iter().map(|s| if *s == "blob" { "resolve" } else { s }).collect();
            Url::parse(&format!("https://huggingface.co/{}", path.join("/"))).ok()
        }
        // pastebin.com/{id} → pastebin.com/raw/{id}
        ("pastebin.com", [id]) if id.len() >= 6 && id.bytes().all(|b| b.is_ascii_alphanumeric()) => Url::parse(&format!("https://pastebin.com/raw/{id}")).ok(),
        // Dropbox share links show a page unless `dl=1`.
        ("dropbox.com", _) if url.query_pairs().any(|(k, _)| k == "dl") => {
            let mut out = url.clone();
            let kept: Vec<(String, String)> = url.query_pairs().filter(|(k, _)| k != "dl").map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            out.query_pairs_mut().clear().extend_pairs(kept).append_pair("dl", "1");
            Some(out)
        }
        _ => None,
    };
    built.unwrap_or_else(|| url.clone())
}

// ---------------------------------------------------------------------------------------------
// Card files

/// The card files in fetched bytes: the file itself, or the cards of a plain zip (read entry by
/// entry within the import budget, all entries together at most [`MAX_ZIP_UNPACKED`]). Runs on
/// the blocking pool.
fn card_files(bytes: Vec<u8>) -> Vec<Vec<u8>> {
    let holds_card = |f: &formats::ImportedFile| matches!(&f.content, Content::Cards(c) if !c.is_empty());
    let is_card = |b: &[u8]| formats::detect_and_parse(b).is_ok_and(|f| holds_card(&f));
    match formats::detect_and_parse(&bytes) {
        Ok(f) if holds_card(&f) => vec![bytes],
        Ok(_) => vec![],
        Err(_) if bytes.starts_with(b"PK\x03\x04") => {
            let mut cards = vec![];
            let unzipped = std::sync::atomic::AtomicU64::new(0);
            // A zip that breaks off or goes over the limits keeps the cards read until then.
            let _ = import::each_entry(std::io::Cursor::new(&bytes), formats::MAX_INPUT as u64, &unzipped, MAX_ZIP_UNPACKED, |entry| {
                if let Ok((entry, _room)) = entry.read {
                    if is_card(&entry) {
                        cards.push(entry);
                    }
                }
                cards.len() < MAX_ZIP_CARDS
            });
            cards
        }
        Err(_) => vec![],
    }
}

/// Is this JSON value a character card? Stricter than the import of a file the user picked: a
/// web page is full of JSON with a `name`.
fn card_like(v: &Value) -> bool {
    let text = |v: &Value, key: &str| v[key].as_str().is_some_and(|s| !s.trim().is_empty());
    if v["spec"].as_str().is_some_and(|s| s.starts_with("chara_card_v")) && v["data"].is_object() {
        return text(&v["data"], "name");
    }
    text(v, "name") && v["first_mes"].is_string() && v["description"].is_string() && (v["personality"].is_string() || v["scenario"].is_string() || v["mes_example"].is_string())
}

/// The first card inside a JSON value: the value itself, somewhere below it, or in a string
/// that holds JSON again. `budget` bounds the work on a huge page.
fn find_card(v: &Value, depth: usize, budget: &mut usize) -> Option<Value> {
    if depth > 24 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    match v {
        Value::Object(map) => {
            if card_like(v) {
                return Some(v.clone());
            }
            map.values().find_map(|child| find_card(child, depth + 1, budget))
        }
        Value::Array(list) => list.iter().find_map(|child| find_card(child, depth + 1, budget)),
        Value::String(s) if s.len() > 40 && s.trim_start().starts_with('{') && (s.contains("first_mes") || s.contains("chara_card_v")) => {
            let inner: Value = serde_json::from_str(s).ok()?;
            find_card(&inner, depth + 1, budget)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Reading a page

/// One opening tag: its name, attributes and, for `<script>`, its content.
struct Tag<'a> {
    name: String,
    attrs: Vec<(String, String)>,
    inner: &'a str,
}

impl Tag<'_> {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#34;", "\"").replace("&#39;", "'").replace("&#x27;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

/// The opening tags of a page, tolerant of broken markup (at most 20 000).
fn tags(html: &str) -> Vec<Tag<'_>> {
    let b = html.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() && out.len() < 20_000 {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if html[i..].starts_with("<!--") {
            i = html[i..].find("-->").map_or(b.len(), |end| i + end + 3);
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
            j += 1;
        }
        if j == start {
            i += 1;
            continue;
        }
        let name = html[start..j].to_ascii_lowercase();
        let mut attrs = vec![];
        // Attributes until the closing '>'.
        loop {
            while j < b.len() && (b[j].is_ascii_whitespace() || b[j] == b'/') {
                j += 1;
            }
            if j >= b.len() || b[j] == b'>' {
                break;
            }
            let key_start = j;
            while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>') {
                j += 1;
            }
            let key = html[key_start..j].to_ascii_lowercase();
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let mut value = String::new();
            if j < b.len() && b[j] == b'=' {
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && matches!(b[j], b'"' | b'\'') {
                    let quote = b[j];
                    let from = j + 1;
                    j = from;
                    while j < b.len() && b[j] != quote {
                        j += 1;
                    }
                    value = unescape(&html[from..j]);
                    j = (j + 1).min(b.len());
                } else {
                    let from = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    value = unescape(&html[from..j]);
                }
            }
            if key.is_empty() {
                j += 1;
            } else {
                attrs.push((key, value));
            }
        }
        j = (j + 1).min(b.len());
        let mut inner = "";
        if name == "script" || name == "style" {
            let close = format!("</{name}");
            let rest = &html[j..];
            let end = rest.to_ascii_lowercase().find(&close).unwrap_or(rest.len());
            inner = &rest[..end];
            j += end;
        }
        out.push(Tag { name, attrs, inner });
        i = j;
    }
    out
}

/// A card embedded in the page's scripts.
fn embedded_card(tags: &[Tag<'_>]) -> Option<Value> {
    let mut budget = 400_000;
    for tag in tags.iter().filter(|t| t.name == "script" && !t.inner.trim().is_empty()) {
        let kind = tag.attr("type").unwrap_or("").to_ascii_lowercase();
        let text = tag.inner.trim();
        let json = if kind.contains("json") {
            serde_json::from_str::<Value>(text).ok()
        } else if text.contains("chara_card_v") {
            // `window.x = {…};`: the outermost braces.
            match (text.find('{'), text.rfind('}')) {
                (Some(a), Some(z)) if a < z => serde_json::from_str::<Value>(&text[a..=z]).ok(),
                _ => None,
            }
        } else {
            None
        };
        if let Some(card) = json.and_then(|v| find_card(&v, 0, &mut budget)) {
            return Some(card);
        }
    }
    None
}

/// The picture a page names as its own (`og:image`, `twitter:image`).
fn page_image(tags: &[Tag<'_>], base: &Url) -> Option<Url> {
    tags.iter()
        .filter(|t| t.name == "meta")
        .find(|t| matches!(t.attr("property").or(t.attr("name")), Some("og:image" | "og:image:secure_url" | "twitter:image")))
        .and_then(|t| t.attr("content"))
        .and_then(|c| base.join(c.trim()).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https"))
}

const GOOD_WORDS: &[&str] = &["card", "chara", "character", "tavern", "download", "export", "v2", "v3"];
const BAD_WORDS: &[&str] = &["logo", "icon", "favicon", "sprite", "banner", "emoji", "badge", "manifest", "package", "tsconfig", "schema", "avatar-default", "placeholder"];

/// The files of a page that may be a card, most likely first.
fn candidates(tags: &[Tag<'_>], page: &Url) -> Vec<Url> {
    let base = tags.iter().find(|t| t.name == "base").and_then(|t| t.attr("href")).and_then(|h| page.join(h).ok()).unwrap_or_else(|| page.clone());
    let mut found: Vec<(i32, Url)> = vec![];
    let mut add = |raw: &str, mut score: i32, any_type: bool| {
        let Ok(mut url) = base.join(raw.trim()) else { return };
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        url.set_fragment(None);
        let path = url.path().to_ascii_lowercase();
        let ext = path.rsplit('.').next().unwrap_or("");
        if !any_type && !matches!(ext, "png" | "json" | "charx") {
            return;
        }
        let whole = url.as_str().to_ascii_lowercase();
        score += 2 * GOOD_WORDS.iter().filter(|w| whole.contains(*w)).count().min(2) as i32;
        score -= 4 * BAD_WORDS.iter().filter(|w| path.contains(*w)).count() as i32;
        if url.host_str() == page.host_str() {
            score += 1;
        }
        if ext == "charx" {
            score += 3;
        }
        if score >= 0 && !found.iter().any(|(_, u)| *u == url) {
            found.push((score, url));
        }
    };
    for tag in tags {
        match tag.name.as_str() {
            "a" => {
                if let Some(href) = tag.attr("href") {
                    add(href, if tag.attr("download").is_some() { 4 } else { 1 }, false);
                }
            }
            "img" => {
                if let Some(src) = tag.attr("src") {
                    add(src, 0, false);
                }
            }
            "meta" if matches!(tag.attr("property").or(tag.attr("name")), Some("og:image" | "og:image:secure_url" | "twitter:image")) => {
                if let Some(content) = tag.attr("content") {
                    add(content, 2, true);
                }
            }
            "link" if tag.attr("rel").is_some_and(|r| r.eq_ignore_ascii_case("alternate")) && tag.attr("type").is_some_and(|t| t.contains("json")) => {
                if let Some(href) = tag.attr("href") {
                    add(href, 2, true);
                }
            }
            _ => {}
        }
    }
    found.sort_by_key(|(score, _)| -score);
    found.into_iter().take(MAX_CANDIDATES).map(|(_, url)| url).collect()
}

fn looks_like_html(content_type: &str, body: &[u8]) -> bool {
    if content_type.contains("html") {
        return true;
    }
    let head = String::from_utf8_lossy(&body[..body.len().min(512)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<!doctype html") || head.starts_with("<html") || head.starts_with("<head")
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, WebError> {
    tokio::task::spawn_blocking(work).await.map_err(|_| WebError::Fetch(FetchError::Failed))
}

/// Fetches whatever is behind `url` and answers the card files found (the first is the one to
/// show; more than one only for a zip of cards). `exempt`: the test origin, see `netguard`.
pub async fn fetch(exempt: Option<Url>, url: &Url) -> Result<Vec<Vec<u8>>, WebError> {
    let guard = Guard::new(exempt, BUDGET);
    let first = guard.get(file_address(url), ACCEPT_ANY, MAX_CARD_IMAGE).await?;
    let (page_url, content_type, body) = (first.url, first.content_type, first.body);

    if !looks_like_html(&content_type, &body) {
        let files = blocking(move || card_files(body)).await?;
        return if files.is_empty() { Err(WebError::NoCard) } else { Ok(files) };
    }
    if body.len() > MAX_PAGE {
        return Err(WebError::NoCard);
    }

    let html = String::from_utf8_lossy(&body).into_owned();
    let base = page_url.clone();
    let (embedded, image, links) = blocking(move || {
        let tags = tags(&html);
        (embedded_card(&tags), page_image(&tags, &base), candidates(&tags, &base))
    })
    .await?;

    if let Some(card) = embedded {
        // The page's picture becomes the card's; a page without one gives a card without.
        let picture = match image {
            Some(image_url) => guard.get(image_url, "image/*", MAX_CARD_IMAGE).await.ok().map(|got| got.body),
            None => None,
        };
        let file = blocking(move || {
            let png = picture.as_deref().and_then(imaging::to_png);
            match png {
                Some(png) => formats::write_png(&card, Some(&png)).ok(),
                None => serde_json::to_vec(&card).ok(),
            }
        })
        .await?;
        return file.map(|f| vec![f]).ok_or(WebError::NoCard);
    }

    let mut blocked = None;
    for link in links {
        match guard.get(link, ACCEPT_FILE, MAX_CARD_IMAGE).await {
            Ok(got) => {
                let body = got.body;
                // A linked JSON file must really be a card (a page links many JSON files).
                let files = blocking(move || {
                    if body.starts_with(b"\x89PNG") || body.starts_with(b"PK\x03\x04") {
                        return card_files(body);
                    }
                    match serde_json::from_slice::<Value>(&body) {
                        Ok(v) if card_like(&v) => card_files(body),
                        _ => vec![],
                    }
                })
                .await?;
                if !files.is_empty() {
                    return Ok(files);
                }
            }
            Err(FetchError::Timeout) => return Err(WebError::Fetch(FetchError::Timeout)),
            Err(e @ (FetchError::Blocked | FetchError::RateLimited)) => blocked = Some(e),
            Err(_) => {}
        }
    }
    match blocked {
        Some(e) => Err(WebError::Fetch(e)),
        None => Err(WebError::NoCard),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn file_pages_become_file_addresses() {
        for (from, to) in [
            ("https://github.com/someone/cards/blob/main/sfw/mara.png", "https://raw.githubusercontent.com/someone/cards/main/sfw/mara.png"),
            ("https://www.github.com/someone/cards/blob/v1.2/a%20b.json", "https://raw.githubusercontent.com/someone/cards/v1.2/a%20b.json"),
            ("https://huggingface.co/datasets/someone/cards/blob/main/mara.png", "https://huggingface.co/datasets/someone/cards/resolve/main/mara.png"),
            ("https://pastebin.com/AbCd1234", "https://pastebin.com/raw/AbCd1234"),
            ("https://www.dropbox.com/scl/fi/abc/mara.png?rlkey=k&dl=0", "https://www.dropbox.com/scl/fi/abc/mara.png?rlkey=k&dl=1"),
            // Everything else stays as it is.
            ("https://github.com/someone/cards", "https://github.com/someone/cards"),
            ("https://files.catbox.moe/abc123.png", "https://files.catbox.moe/abc123.png"),
            ("https://example.com/blob/main/x.png", "https://example.com/blob/main/x.png"),
        ] {
            assert_eq!(file_address(&url(from)).as_str(), to, "{from}");
        }
    }

    #[test]
    fn only_real_cards_count() {
        assert!(card_like(&json!({ "spec": "chara_card_v2", "data": { "name": "Mara" } })));
        assert!(card_like(&json!({ "name": "Mara", "description": "", "first_mes": "Hallo", "personality": "" })));
        assert!(!card_like(&json!({ "spec": "chara_card_v2", "data": { "name": " " } })));
        assert!(!card_like(&json!({ "name": "A web app", "description": "manifest", "icons": [] })));
        assert!(!card_like(&json!({ "@type": "Person", "name": "Someone" })));
    }

    #[test]
    fn finds_a_card_deep_inside_page_data() {
        let card = json!({ "spec": "chara_card_v3", "data": { "name": "Mara", "first_mes": "Moin." } });
        let nested = json!({ "props": { "pageProps": { "items": [1, { "card": card }] } } });
        assert_eq!(find_card(&nested, 0, &mut 1000), Some(card.clone()));
        // JSON inside a string (pages often serialize twice).
        let twice = json!({ "state": serde_json::to_string(&json!({ "x": card })).unwrap() });
        assert_eq!(find_card(&twice, 0, &mut 1000), Some(card));
        assert_eq!(find_card(&json!({ "name": "no card" }), 0, &mut 1000), None);
        assert_eq!(find_card(&nested, 0, &mut 2), None, "the budget ends the search");
    }

    #[test]
    fn reads_tags_of_untidy_markup() {
        let html = r#"<!-- <a href="/no.png"> --><BASE href="/sub/"><a class=x href='a&amp;b.png' download>Karte</a>
            <img src=pic.png><script type="application/json" id="d">{"a":"</b>"}</script><meta property="og:image" content="https://cdn.example/o">"#;
        let tags = tags(html);
        let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["base", "a", "img", "script", "meta"]);
        assert_eq!(tags[1].attr("href"), Some("a&b.png"));
        assert!(tags[1].attr("download").is_some());
        assert_eq!(tags[2].attr("src"), Some("pic.png"));
        assert_eq!(tags[3].inner, r#"{"a":"</b>"}"#);
        assert_eq!(page_image(&tags, &url("https://example.com/")).unwrap().as_str(), "https://cdn.example/o");
    }

    #[test]
    fn candidates_are_ranked_and_limited() {
        let html = r#"<base href="/cards/"><img src="/img/logo.png"><img src="shot.png">
            <a href="mara.card.png" download>Download</a><a href="/about">x</a><a href="notes.txt">x</a>
            <a href="javascript:void(0)">x</a><a href="mara.charx">CHARX</a><a href="mara.card.png#again">again</a>
            <meta property="og:image" content="https://cdn.example/og/123">"#;
        let found: Vec<String> = candidates(&tags(html), &url("https://example.com/page")).into_iter().map(String::from).collect();
        assert_eq!(found[0], "https://example.com/cards/mara.card.png");
        assert!(found.contains(&"https://example.com/cards/mara.charx".to_string()));
        assert!(found.contains(&"https://cdn.example/og/123".to_string()), "og:image counts without an ending");
        assert!(found.contains(&"https://example.com/cards/shot.png".to_string()));
        assert!(!found.iter().any(|u| u.contains("logo") || u.contains("about") || u.contains("notes") || u.contains("javascript")), "{found:?}");
        assert_eq!(found.iter().filter(|u| u.ends_with("mara.card.png")).count(), 1);

        let many: String = (0..40).map(|i| format!(r#"<a href="c{i}.png">x</a>"#)).collect();
        assert_eq!(candidates(&tags(&many), &url("https://example.com/")).len(), MAX_CANDIDATES);
    }

    #[test]
    fn embedded_cards_in_scripts() {
        let card = json!({ "spec": "chara_card_v2", "spec_version": "2.0", "data": { "name": "Mara", "first_mes": "Moin." } });
        let next = format!(r#"<script id="__NEXT_DATA__" type="application/json">{}</script>"#, json!({ "props": { "card": card } }));
        assert_eq!(embedded_card(&tags(&next)), Some(card.clone()));
        let assigned = format!("<script>window.__CARD__ = {card};</script>");
        assert_eq!(embedded_card(&tags(&assigned)), Some(card));
        let ld = r#"<script type="application/ld+json">{"@type":"Person","name":"Someone","description":"x"}</script>"#;
        assert_eq!(embedded_card(&tags(ld)), None);
    }
}
