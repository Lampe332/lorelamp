//! Ordering the library (migration 15): favourites and the user's own tags of characters
//! (tables `tags` + `character_tags`), plus actions on many characters at once.
//!
//! These tags belong to the app, not to the card: a card's own `tags` stay part of the card and its
//! export, these only sort the library.

use rusqlite::params;
use serde::Serialize;

use super::{now, Res, Store};

/// Longest tag (characters) and most tags per character.
pub const TAG_MAX: usize = 40;
pub const TAGS_PER_CHARACTER: usize = 30;

/// A tag and how many (not deleted) characters carry it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TagCount {
    pub name: String,
    pub count: i64,
}

/// One action on many characters (`POST /characters/bulk`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bulk<'a> {
    /// Soft delete with undo (`Restore`).
    Delete,
    Restore,
    Tag(&'a str),
    Untag(&'a str),
    Favorite(bool),
}

/// A tag as stored: trimmed, inner spaces collapsed. None: empty or too long.
pub fn clean_tag(tag: &str) -> Option<String> {
    let t = tag.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty() && t.chars().count() <= TAG_MAX).then_some(t)
}

impl Store {
    /// The id of a tag, created when it is new. Names match without regard to case, so "fantasy"
    /// joins an existing "Fantasy".
    fn tag_id(&self, name: &str) -> Res<i64> {
        self.conn.execute("INSERT INTO tags (name) VALUES (?1) ON CONFLICT(name) DO NOTHING", params![name])?;
        Ok(self.conn.query_row("SELECT id FROM tags WHERE name = ?1", params![name], |r| r.get(0))?)
    }

    /// Tags no character carries any more.
    pub(super) fn drop_unused_tags(&self) -> Res<()> {
        self.conn.execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM character_tags)", [])?;
        Ok(())
    }

    /// The user's tags of a character, by name.
    pub fn character_tags(&self, id: i64) -> Res<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.name FROM character_tags ct JOIN tags t ON t.id = ct.tag_id WHERE ct.character_id = ?1 ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![id], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Replaces the user's tags of a character (cleaned, duplicates dropped, at most
    /// `TAGS_PER_CHARACTER`). None: no such character. Answers the stored tags.
    pub fn set_character_tags(&mut self, id: i64, tags: &[String]) -> Res<Option<Vec<String>>> {
        if !self.character_exists(id)? {
            return Ok(None);
        }
        let mut clean: Vec<String> = vec![];
        for t in tags.iter().filter_map(|t| clean_tag(t)) {
            if !clean.iter().any(|c| c.to_lowercase() == t.to_lowercase()) && clean.len() < TAGS_PER_CHARACTER {
                clean.push(t);
            }
        }
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM character_tags WHERE character_id = ?1", params![id])?;
        for t in &clean {
            tx.execute("INSERT INTO tags (name) VALUES (?1) ON CONFLICT(name) DO NOTHING", params![t])?;
            tx.execute(
                "INSERT OR IGNORE INTO character_tags (character_id, tag_id) VALUES (?1, (SELECT id FROM tags WHERE name = ?2))",
                params![id, t],
            )?;
        }
        tx.execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM character_tags)", [])?;
        tx.commit()?;
        Ok(Some(self.character_tags(id)?))
    }

    fn character_exists(&self, id: i64) -> Res<bool> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM characters WHERE id = ?1 AND deleted_at IS NULL", params![id], |r| r.get::<_, i64>(0))? > 0)
    }

    /// Marks or unmarks a character as a favourite. false: no such character.
    pub fn set_favorite(&mut self, id: i64, favorite: bool) -> Res<bool> {
        let n = self.conn.execute("UPDATE characters SET favorite = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![id, favorite])?;
        Ok(n > 0)
    }

    /// All tags in use, by name, with how many characters carry each.
    pub fn tags(&self) -> Res<Vec<TagCount>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.name, COUNT(c.id) FROM tags t
             JOIN character_tags ct ON ct.tag_id = t.id
             JOIN characters c ON c.id = ct.character_id AND c.deleted_at IS NULL
             GROUP BY t.id ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| Ok(TagCount { name: r.get(0)?, count: r.get(1)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Applies one action to many characters in one transaction; answers how many changed.
    /// Unknown ids are skipped. Err: a tag that is empty or too long.
    pub fn bulk_characters(&mut self, ids: &[i64], action: Bulk<'_>) -> Res<usize> {
        let tag = match action {
            Bulk::Tag(t) | Bulk::Untag(t) => Some(clean_tag(t).ok_or("bad_tag")?),
            _ => None,
        };
        let tag_id = match (&action, &tag) {
            (Bulk::Tag(_), Some(t)) => Some(self.tag_id(t)?),
            (Bulk::Untag(_), Some(t)) => self.conn.query_row("SELECT id FROM tags WHERE name = ?1", params![t], |r| r.get(0)).ok(),
            _ => None,
        };
        let t = now();
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        for &id in ids {
            changed += match action {
                Bulk::Delete => tx.execute("UPDATE characters SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![id, t])?,
                Bulk::Restore => tx.execute("UPDATE characters SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL", params![id])?,
                Bulk::Favorite(on) => tx.execute(
                    "UPDATE characters SET favorite = ?2 WHERE id = ?1 AND deleted_at IS NULL AND favorite <> ?2",
                    params![id, on],
                )?,
                Bulk::Tag(_) => {
                    let alive: i64 = tx.query_row("SELECT COUNT(*) FROM characters WHERE id = ?1 AND deleted_at IS NULL", params![id], |r| r.get(0))?;
                    let full: i64 = tx.query_row("SELECT COUNT(*) FROM character_tags WHERE character_id = ?1", params![id], |r| r.get(0))?;
                    if alive == 0 || full as usize >= TAGS_PER_CHARACTER {
                        0
                    } else {
                        tx.execute("INSERT OR IGNORE INTO character_tags (character_id, tag_id) VALUES (?1, ?2)", params![id, tag_id])?
                    }
                }
                Bulk::Untag(_) => match tag_id {
                    Some(tag) => tx.execute("DELETE FROM character_tags WHERE character_id = ?1 AND tag_id = ?2", params![id, tag])?,
                    None => 0,
                },
            };
        }
        tx.commit()?;
        if matches!(action, Bulk::Tag(_) | Bulk::Untag(_)) {
            self.drop_unused_tags()?;
        }
        Ok(changed)
    }
}
