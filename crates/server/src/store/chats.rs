//! Chats (table `chats`): the overviews, titles, the chat's own version of a card, and the soft
//! delete with undo.

use rp_core::card::Card;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use super::{now, Character, Res, Store};

/// The columns `ChatInfo` is read from, for `SELECT {CHAT} FROM chats ch`. The last one is the
/// effective persona: the chat's own, else the active one (same fallback as
/// `Store::active_persona_id`).
const CHAT: &str = "ch.id, ch.character_id, ch.created_at, ch.updated_at,
    (SELECT COUNT(*) FROM messages m WHERE m.chat_id = ch.id AND m.deleted_at IS NULL),
    COALESCE((SELECT substr(m.content, 1, 240) FROM messages m WHERE m.chat_id = ch.id AND m.deleted_at IS NULL
              AND m.role = 'assistant' ORDER BY m.id DESC LIMIT 1), ''),
    ch.card_json IS NOT NULL, ch.title,
    (SELECT p.id FROM personas p WHERE p.id = ch.persona_id),
    COALESCE((SELECT p.id FROM personas p WHERE p.id = ch.persona_id),
             COALESCE((SELECT p.id FROM personas p
                       WHERE p.id = CAST((SELECT value FROM settings WHERE key = 'active_persona') AS INTEGER)),
                      (SELECT MIN(id) FROM personas)))";

/// How many columns `CHAT` has.
const CHAT_COLS: usize = 10;

/// One chat as the overviews show it.
#[derive(Debug, Clone, Serialize)]
pub struct ChatInfo {
    pub id: i64,
    pub character_id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub messages: i64,
    /// Start of the character's newest message (what the overviews quote).
    pub last: String,
    /// This chat plays with its own version of the card.
    pub custom_card: bool,
    /// The name the user gave it; empty = unnamed (shown by its date).
    pub title: String,
    /// The persona this chat is bound to; null = it follows the active persona.
    pub persona_id: Option<i64>,
    /// The persona this chat plays with right now (`persona_id`, else the active one).
    pub effective_persona_id: i64,
}

impl ChatInfo {
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatInfo> {
        Ok(ChatInfo {
            id: r.get(0)?,
            character_id: r.get(1)?,
            created_at: r.get(2)?,
            updated_at: r.get(3)?,
            messages: r.get(4)?,
            last: r.get(5)?,
            custom_card: r.get(6)?,
            title: r.get(7)?,
            persona_id: r.get(8)?,
            effective_persona_id: r.get(9)?,
        })
    }
}

/// A recently used chat together with what is needed to show its character.
#[derive(Debug, Clone, Serialize)]
pub struct Recent {
    pub chat: ChatInfo,
    pub name: String,
    pub has_avatar: bool,
    pub avatar_rev: i64,
    pub avatar_pos: [f64; 2],
}

impl Store {
    pub fn latest_chat(&self, character_id: i64) -> Res<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM chats WHERE character_id = ?1 AND deleted_at IS NULL ORDER BY updated_at DESC, id DESC LIMIT 1",
                params![character_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// A new chat with at most one greeting (tests).
    #[cfg(test)]
    pub fn create_chat(&mut self, character_id: i64, greeting: Option<&str>) -> Res<i64> {
        let greetings: Vec<String> = greeting.into_iter().map(str::to_string).collect();
        self.create_chat_with_greetings(character_id, &greetings)
    }

    /// A new chat that opens with the first of `greetings` (none: without a greeting). With more
    /// than one, all of them are the variants of that first message (`swipes`, the first active),
    /// so the user can swipe to another greeting.
    pub fn create_chat_with_greetings(&mut self, character_id: i64, greetings: &[String]) -> Res<i64> {
        let t = now();
        let tx = self.conn.transaction()?;
        // Bound to the character's default persona if it has one (that still exists).
        tx.execute(
            "INSERT INTO chats (character_id, created_at, updated_at, persona_id)
             VALUES (?1, ?2, ?2, (SELECT p.id FROM characters c JOIN personas p ON p.id = c.default_persona_id WHERE c.id = ?1))",
            params![character_id, t],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(first) = greetings.first() {
            let swipes = (greetings.len() > 1).then(|| serde_json::to_string(greetings)).transpose()?;
            tx.execute(
                "INSERT INTO messages (chat_id, role, content, created_at, swipes, swipe_id) VALUES (?1, 'assistant', ?2, ?3, ?4, ?5)",
                params![id, first, t, swipes, swipes.is_some().then_some(0i64)],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// The character a chat belongs to (None: no such chat, or its character was deleted).
    pub fn chat_character(&self, chat_id: i64) -> Res<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT c.id FROM chats ch JOIN characters c ON c.id = ch.character_id
                 WHERE ch.id = ?1 AND ch.deleted_at IS NULL AND c.deleted_at IS NULL",
                params![chat_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The most recently used chats across all characters, newest first.
    pub fn recent(&self, limit: i64) -> Res<Vec<Recent>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CHAT}, c.name, c.has_png OR c.avatar_mime IS NOT NULL, c.avatar_rev, c.avatar_x, c.avatar_y
             FROM chats ch JOIN characters c ON c.id = ch.character_id
             WHERE ch.deleted_at IS NULL AND c.deleted_at IS NULL
             ORDER BY ch.updated_at DESC, ch.id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(Recent {
                chat: ChatInfo::read(r)?,
                name: r.get(CHAT_COLS)?,
                has_avatar: r.get(CHAT_COLS + 1)?,
                avatar_rev: r.get(CHAT_COLS + 2)?,
                avatar_pos: [r.get(CHAT_COLS + 3)?, r.get(CHAT_COLS + 4)?],
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// All chats with one character, newest first.
    pub fn chats_of(&self, character_id: i64) -> Res<Vec<ChatInfo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CHAT} FROM chats ch WHERE ch.character_id = ?1 AND ch.deleted_at IS NULL
             ORDER BY ch.updated_at DESC, ch.id DESC"
        ))?;
        let rows = stmt.query_map(params![character_id], ChatInfo::read)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn chat_info(&self, chat_id: i64) -> Res<Option<ChatInfo>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {CHAT} FROM chats ch WHERE ch.id = ?1 AND ch.deleted_at IS NULL"),
                params![chat_id],
                ChatInfo::read,
            )
            .optional()?)
    }

    /// Names a chat (empty = back to unnamed). false: no such chat.
    pub fn set_chat_title(&mut self, chat_id: i64, title: &str) -> Res<bool> {
        let n = self.conn.execute(
            "UPDATE chats SET title = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![chat_id, title],
        )?;
        Ok(n > 0)
    }

    /// Soft delete (`deleted = true`) or undo. Deleted chats are purged at the next start.
    pub fn set_chat_deleted(&mut self, chat_id: i64, deleted: bool) -> Res<bool> {
        let n = self.conn.execute(
            "UPDATE chats SET deleted_at = ?2 WHERE id = ?1",
            params![chat_id, deleted.then(now)],
        )?;
        Ok(n > 0)
    }

    /// The character as this chat plays it: with the chat's own version of the card if it has
    /// one (second value), else the character's card. None: no such chat.
    pub fn chat_character_card(&self, chat_id: i64) -> Res<Option<(Character, bool)>> {
        let Some(character_id) = self.chat_character(chat_id)? else { return Ok(None) };
        let Some(mut character) = self.character(character_id)? else { return Ok(None) };
        let own: Option<String> =
            self.conn.query_row("SELECT card_json FROM chats WHERE id = ?1", params![chat_id], |r| r.get(0))?;
        let Some(own) = own.and_then(|j| serde_json::from_str::<Card>(&j).ok()) else {
            return Ok(Some((character, false)));
        };
        character.card = own;
        Ok(Some((character, true)))
    }

    /// Gives a chat its own version of the card (`Some`) or returns it to the character's card
    /// (`None`). false: no such chat.
    pub fn set_chat_card(&mut self, chat_id: i64, card: Option<&Card>) -> Res<bool> {
        if self.chat_character(chat_id)?.is_none() {
            return Ok(false);
        }
        let json = card.map(serde_json::to_string).transpose()?;
        self.conn.execute("UPDATE chats SET card_json = ?2 WHERE id = ?1", params![chat_id, json])?;
        Ok(true)
    }

    /// A new chat with the same character and card that holds this chat's messages up to and
    /// including `upto_id`. The original stays as it is.
    pub fn fork_chat(&mut self, chat_id: i64, upto_id: i64, title: &str) -> Res<i64> {
        let t = now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO chats (character_id, created_at, updated_at, card_json, title, persona_id)
             SELECT character_id, ?2, ?2, card_json, ?3, persona_id FROM chats WHERE id = ?1",
            params![chat_id, t, title],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO messages (chat_id, role, content, timings, created_at, persona_id, swipes, swipe_id)
             SELECT ?3, role, content, timings, created_at, persona_id, swipes, swipe_id FROM messages
             WHERE chat_id = ?1 AND id <= ?2 AND deleted_at IS NULL ORDER BY id",
            params![chat_id, upto_id, id],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// The chat of this character that was imported with this content hash (None: none that is
    /// still there).
    pub fn chat_by_import_hash(&self, hash: &str, character_id: i64) -> Res<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT ch.id FROM chats ch JOIN characters c ON c.id = ch.character_id
                 WHERE ch.import_hash = ?1 AND ch.character_id = ?2 AND ch.deleted_at IS NULL AND c.deleted_at IS NULL
                 ORDER BY ch.id LIMIT 1",
                params![hash, character_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Stores a chat from another app with all its messages in one transaction, bound to the
    /// character's default persona like a new chat. A chat of the same character with the same
    /// content hash is not stored again: answers (chat id, true when it is new).
    pub fn import_chat(&mut self, character_id: i64, chat: &ImportedChat) -> Res<(i64, bool)> {
        if let Some(id) = self.chat_by_import_hash(&chat.hash, character_id)? {
            return Ok((id, false));
        }
        let updated = chat.messages.iter().map(|m| m.created_at).max().unwrap_or(chat.created_at).max(chat.created_at);
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO chats (character_id, created_at, updated_at, title, import_hash, persona_id)
             VALUES (?1, ?2, ?3, ?4, ?5, (SELECT p.id FROM characters c JOIN personas p ON p.id = c.default_persona_id WHERE c.id = ?1))",
            params![character_id, chat.created_at, updated, chat.title, chat.hash],
        )?;
        let id = tx.last_insert_rowid();
        {
            let mut insert = tx.prepare(
                "INSERT INTO messages (chat_id, role, content, created_at, swipes, swipe_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for m in &chat.messages {
                let (swipes, swipe_id) = match &m.swipes {
                    Some((list, i)) if *i < list.len() && list.len() > 1 => (Some(serde_json::to_string(list)?), Some(*i as i64)),
                    _ => (None, None),
                };
                insert.execute(params![id, m.role, m.content, m.created_at, swipes, swipe_id])?;
            }
        }
        tx.commit()?;
        Ok((id, true))
    }
}

/// A chat from another app, ready to store.
#[derive(Debug, Clone)]
pub struct ImportedChat {
    pub title: String,
    /// Unix seconds.
    pub created_at: i64,
    /// SHA-256 of the chat's content: the same chat is stored once.
    pub hash: String,
    pub messages: Vec<ImportedMsg>,
}

#[derive(Debug, Clone)]
pub struct ImportedMsg {
    /// "user" | "assistant"
    pub role: &'static str,
    /// The active text.
    pub content: String,
    /// Unix seconds.
    pub created_at: i64,
    /// All variants and the index of the active one (`content`).
    pub swipes: Option<(Vec<String>, usize)>,
}
