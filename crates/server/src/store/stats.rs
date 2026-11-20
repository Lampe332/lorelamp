//! Figures across the whole library for the start screen.

use rusqlite::OptionalExtension;
use serde::Serialize;

use super::{Res, Store, Timings};

/// Figures across the whole library (see `Store::stats`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stats {
    pub messages: i64,
    pub last_timings: Option<Timings>,
}

/// Messages that count: not rewound, in a chat that is not deleted, of a character that is not
/// deleted. Soft-deleted rows stay in the database until the next start (undo), so every figure
/// has to leave them out itself.
const LIVE_MESSAGES: &str = "messages m JOIN chats ch ON ch.id = m.chat_id JOIN characters c ON c.id = ch.character_id
     WHERE m.deleted_at IS NULL AND ch.deleted_at IS NULL AND c.deleted_at IS NULL";

impl Store {
    /// Figures for the start screen: all live messages, and the measurements of the newest live
    /// answer that has them.
    pub fn stats(&self) -> Res<Stats> {
        let messages = self.conn.query_row(&format!("SELECT COUNT(*) FROM {LIVE_MESSAGES}"), [], |r| r.get(0))?;
        let last: Option<String> = self
            .conn
            .query_row(
                &format!("SELECT m.timings FROM {LIVE_MESSAGES} AND m.timings IS NOT NULL ORDER BY m.id DESC LIMIT 1"),
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(Stats { messages, last_timings: last.and_then(|t| serde_json::from_str(&t).ok()) })
    }
}
