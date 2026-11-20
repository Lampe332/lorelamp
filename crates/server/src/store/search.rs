//! Full text search over the messages of all chats (FTS5 table `messages_fts`, migration 14).
//!
//! The index holds the active messages only: triggers on `messages` keep it in step with inserts,
//! edits, rewinds (soft delete), their undo and hard deletes. Deleted chats and characters are left
//! out when searching. Case, accents and umlauts do not matter (`remove_diacritics 2`).

use rusqlite::params;
use serde::Serialize;

use super::{Res, Store};

/// Start and end of a matched part in `SearchHit::snippet` (control characters a message never holds).
pub const MARK_START: char = '\u{2}';
pub const MARK_END: char = '\u{3}';

/// Most words of a query that are looked for.
const MAX_WORDS: usize = 12;

/// One message that matches a search.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub message_id: i64,
    pub chat_id: i64,
    pub character_id: i64,
    /// The character's name (the chat's figure).
    pub character: String,
    pub has_avatar: bool,
    pub avatar_rev: i64,
    pub avatar_pos: [f64; 2],
    /// The chat's name; empty = unnamed (shown by its date).
    pub chat_title: String,
    pub chat_created_at: i64,
    pub role: String,
    pub created_at: i64,
    /// A part of the message around the match; matched words between `MARK_START` and `MARK_END`.
    pub snippet: String,
}

/// The FTS5 query for what the user typed: every word must appear, each as the start of a word.
/// Words are quoted, so FTS5 syntax (`AND`, `NEAR`, `*`, `:`) typed by the user is plain text.
/// None: nothing to look for.
pub fn fts_query(typed: &str) -> Option<String> {
    let words: Vec<String> = typed
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .take(MAX_WORDS)
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

impl Store {
    /// Messages that match `typed`: across all chats by relevance, or within one chat in story order.
    pub fn search_messages(&self, typed: &str, chat_id: Option<i64>, limit: i64) -> Res<Vec<SearchHit>> {
        let Some(query) = fts_query(typed) else { return Ok(vec![]) };
        let order = if chat_id.is_some() { "m.id" } else { "f.rank, m.id DESC" };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.id, m.chat_id, c.id, c.name, c.has_png OR c.avatar_mime IS NOT NULL, c.avatar_rev, c.avatar_x, c.avatar_y,
                    ch.title, ch.created_at, m.role, m.created_at,
                    snippet(messages_fts, 0, ?4, ?5, '…', 14)
             FROM messages_fts f
             JOIN messages m ON m.id = f.rowid AND m.deleted_at IS NULL
             JOIN chats ch ON ch.id = m.chat_id AND ch.deleted_at IS NULL
             JOIN characters c ON c.id = ch.character_id AND c.deleted_at IS NULL
             WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR m.chat_id = ?2)
             ORDER BY {order} LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![query, chat_id, limit, MARK_START.to_string(), MARK_END.to_string()], |r| {
            Ok(SearchHit {
                message_id: r.get(0)?,
                chat_id: r.get(1)?,
                character_id: r.get(2)?,
                character: r.get(3)?,
                has_avatar: r.get(4)?,
                avatar_rev: r.get(5)?,
                avatar_pos: [r.get(6)?, r.get(7)?],
                chat_title: r.get(8)?,
                chat_created_at: r.get(9)?,
                role: r.get(10)?,
                created_at: r.get(11)?,
                snippet: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
