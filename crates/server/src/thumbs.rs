//! Thumbnails of character pictures: made for the existing library at start (only where one is
//! missing or older than the picture) and again whenever a picture changes. Runs on the
//! blocking pool; the store lock is held only for the database steps, never while decoding.

use crate::chat::Hub;
use crate::imaging;

/// Makes (or replaces) the thumbnail of one character. true: a thumbnail was kept.
pub fn refresh(hub: &Hub, id: i64) -> bool {
    let Ok(Some((path, rev))) = hub.store(|s| s.thumb_source(id)) else { return false };
    let thumb = std::fs::read(path).ok().and_then(|bytes| imaging::thumb_webp(&bytes));
    let made = thumb.is_some();
    matches!(hub.store(|s| s.set_thumb(id, rev, thumb.as_deref())), Ok(true)) && made
}

/// Every missing thumbnail, one after the other. Answers how many were made.
pub fn backfill(hub: &Hub) -> usize {
    let ids = hub.store(|s| s.thumbs_missing()).unwrap_or_default();
    ids.into_iter().filter(|&id| refresh(hub, id)).count()
}
