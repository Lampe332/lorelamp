//! Link import with a look first (`POST /import/link/preview`, then `…/confirm`): the card is
//! fetched exactly like `POST /import/link`, but kept in memory only — at most [`MAX_PREVIEWS`]
//! and [`MAX_BYTES`] for [`TTL`] — until the user confirms it. Confirming imports exactly the fetched bytes through
//! the normal import (duplicates, thumbnail, `source_url`). Nothing is stored before.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::distr::{Alphanumeric, SampleString};
use rp_core::formats::{self, Content};
use serde_json::{json, Value};

use crate::imaging;
use crate::link::Source;
use crate::store::content_hash;

/// Most previews kept at once (the oldest goes first): several links can be pasted together.
pub const MAX_PREVIEWS: usize = 24;
/// Most card bytes kept for previews; over it the oldest go (the newest always stays).
pub const MAX_BYTES: usize = 192 << 20;
/// How long a preview can be confirmed.
pub const TTL: Duration = Duration::from_secs(15 * 60);
/// Longest text of a preview field (characters; longer ones end with `…`).
pub const TEXT_MAX: usize = 600;

/// A fetched card waiting for the user's yes.
#[derive(Clone)]
pub struct Preview {
    pub id: String,
    pub source: Source,
    pub page_url: String,
    /// The fetched card file, imported as is on confirm.
    pub bytes: Vec<u8>,
    /// Further card files that came with it (a zip of cards behind one link); imported with it.
    pub extra: Vec<Vec<u8>>,
    /// The picture to show (WebP thumbnail, else the original) with its content type.
    pub image: Option<(Vec<u8>, &'static str)>,
    /// What the card shows: name, creator, tags and the shortened texts.
    pub card: Value,
    /// SHA-256 of the card content, to tell a duplicate.
    pub hash: String,
    made: Instant,
}

/// The previews of a running server.
pub struct Previews {
    list: Mutex<VecDeque<Preview>>,
    ttl: Duration,
}

impl Default for Previews {
    fn default() -> Previews {
        Previews::with_ttl(TTL)
    }
}

impl Previews {
    pub fn with_ttl(ttl: Duration) -> Previews {
        Previews { list: Mutex::new(VecDeque::new()), ttl }
    }

    fn sweep(&self, list: &mut VecDeque<Preview>) {
        list.retain(|p| p.made.elapsed() < self.ttl);
    }

    /// Keeps a preview (dropping expired ones and, over the limit, the oldest).
    pub fn put(&self, preview: Preview) {
        let mut list = self.list.lock().unwrap();
        self.sweep(&mut list);
        list.push_back(preview);
        let size = |list: &VecDeque<Preview>| list.iter().map(Preview::size).sum::<usize>();
        while list.len() > MAX_PREVIEWS || (list.len() > 1 && size(&list) > MAX_BYTES) {
            list.pop_front();
        }
    }

    /// The preview, if it is still there.
    pub fn get(&self, id: &str) -> Option<Preview> {
        let mut list = self.list.lock().unwrap();
        self.sweep(&mut list);
        list.iter().find(|p| p.id == id).cloned()
    }

    /// Takes the preview out (it can be confirmed once).
    pub fn take(&self, id: &str) -> Option<Preview> {
        let mut list = self.list.lock().unwrap();
        self.sweep(&mut list);
        let at = list.iter().position(|p| p.id == id)?;
        list.remove(at)
    }
}

impl Preview {
    fn size(&self) -> usize {
        self.bytes.len() + self.extra.iter().map(Vec::len).sum::<usize>()
    }
}

fn short(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= TEXT_MAX {
        return text.to_string();
    }
    let mut out: String = text.chars().take(TEXT_MAX).collect::<String>().trim_end().to_string();
    out.push('…');
    out
}

/// Reads a fetched card file into a preview. Err: the import error code (the file holds no
/// card). Runs on the blocking pool: it decodes the picture.
pub fn read(source: Source, page_url: String, bytes: Vec<u8>) -> Result<Preview, &'static str> {
    let file = formats::detect_and_parse(&bytes).map_err(|e| e.code())?;
    let Content::Cards(cards) = file.content else { return Err("no_card") };
    let c = cards.into_iter().next().ok_or("no_card")?;
    let hash = content_hash(&c.raw).ok_or("broken")?;
    let data = if c.raw["data"].is_object() { &c.raw["data"] } else { &c.raw };
    let creator = data["creator"].as_str().unwrap_or("").trim().to_string();
    let card = json!({
        "name": c.card.name,
        "creator": creator,
        "tags": c.card.tags,
        "description": short(&c.card.description),
        "first_mes": short(&c.card.first_mes),
        "creator_notes": short(&c.card.creator_notes),
    });
    let image = c.image.map(|img| match imaging::thumb_webp(&img.bytes) {
        Some(webp) => (webp, "image/webp"),
        None => {
            let mime = crate::store::image_mime(&img.bytes).unwrap_or("application/octet-stream");
            (img.bytes, mime)
        }
    });
    Ok(Preview {
        id: Alphanumeric.sample_string(&mut rand::rng(), 16),
        source,
        page_url,
        bytes,
        extra: vec![],
        image,
        card,
        hash,
        made: Instant::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ILSE_PNG: &[u8] = include_bytes!("../../../tests/fixtures/ilse_brandt.png");

    fn preview() -> Preview {
        read(Source::RisuRealm, "https://realm.risuai.net/character/x".into(), ILSE_PNG.to_vec()).unwrap()
    }

    #[test]
    fn reads_card_and_picture() {
        let p = preview();
        assert!(!p.card["name"].as_str().unwrap().is_empty());
        assert_eq!(p.image.as_ref().map(|i| i.1), Some("image/webp"));
        assert_eq!(p.hash.len(), 64);
        assert_eq!(p.id.len(), 16);
        assert_eq!(read(Source::Chub, String::new(), b"not a card".to_vec()).err(), Some("unknown_format"));
    }

    #[test]
    fn texts_are_shortened() {
        assert_eq!(short("  kurz  "), "kurz");
        let long = "a".repeat(TEXT_MAX + 50);
        let s = short(&long);
        assert_eq!(s.chars().count(), TEXT_MAX + 1);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn limit_and_expiry() {
        let previews = Previews::default();
        let ids: Vec<String> = (0..MAX_PREVIEWS + 1)
            .map(|_| {
                let p = preview();
                let id = p.id.clone();
                previews.put(p);
                id
            })
            .collect();
        assert!(previews.get(&ids[0]).is_none(), "the oldest went first");
        assert!(previews.get(&ids[MAX_PREVIEWS]).is_some());
        assert!(previews.take(&ids[1]).is_some());
        assert!(previews.take(&ids[1]).is_none(), "confirmed once");

        let short_lived = Previews::with_ttl(Duration::from_millis(30));
        let p = preview();
        let id = p.id.clone();
        short_lived.put(p);
        assert!(short_lived.get(&id).is_some());
        std::thread::sleep(Duration::from_millis(60));
        assert!(short_lived.get(&id).is_none() && short_lived.take(&id).is_none(), "expired");
    }
}
