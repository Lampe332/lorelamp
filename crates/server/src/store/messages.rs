//! Messages (table `messages`): adding, finishing a streamed reply, editing, and rewind with one
//! step of undo.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{now, Res, Store};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Timings {
    pub tok_s: f64,
    pub processed: u64,
    pub cached: u64,
    pub predicted: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Msg {
    pub id: i64,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timings: Option<Timings>,
    /// The persona the user wrote this message as (user messages since persona binding; else null).
    pub persona_id: Option<i64>,
    /// All variants of a reply (`content` is the one at `swipe_id`); left out when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swipes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swipe_id: Option<usize>,
    /// When it was written (Unix seconds).
    pub at: i64,
}

/// Variants as stored: a JSON array of strings and the index of the active one (both or neither).
fn read_swipes(json: Option<String>, id: Option<i64>) -> (Option<Vec<String>>, Option<usize>) {
    let swipes: Option<Vec<String>> = json.and_then(|j| serde_json::from_str(&j).ok());
    match (swipes, id.and_then(|i| usize::try_from(i).ok())) {
        (Some(s), Some(i)) if i < s.len() => (Some(s), Some(i)),
        _ => (None, None),
    }
}

impl Store {
    pub fn messages(&self, chat_id: i64) -> Res<Vec<Msg>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, timings, persona_id, swipes, swipe_id, created_at FROM messages WHERE chat_id = ?1 AND deleted_at IS NULL ORDER BY id",
        )?;
        let rows = stmt.query_map(params![chat_id], |r| {
            let timings: Option<String> = r.get(3)?;
            let (swipes, swipe_id) = read_swipes(r.get(5)?, r.get(6)?);
            Ok(Msg {
                id: r.get(0)?,
                role: r.get(1)?,
                content: r.get(2)?,
                timings: timings.and_then(|t| serde_json::from_str(&t).ok()),
                persona_id: r.get(4)?,
                swipes,
                swipe_id,
                at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The active messages of a chat for an export, in order: the user's carry the name of the
    /// persona they were written as (None when it is gone or was never recorded).
    pub fn export_lines(&self, chat_id: i64) -> Res<Vec<crate::export::Line>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.role, m.content, m.created_at, p.name FROM messages m LEFT JOIN personas p ON p.id = m.persona_id
             WHERE m.chat_id = ?1 AND m.deleted_at IS NULL ORDER BY m.id",
        )?;
        let rows = stmt.query_map(params![chat_id], |r| {
            let is_user = r.get::<_, String>(0)? == "user";
            Ok(crate::export::Line { is_user, text: r.get(1)?, sent: r.get(2)?, name: if is_user { r.get(3)? } else { None } })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Adds a message at the end. The story goes on from here, so a rewind can no longer be undone.
    pub fn add_message(&mut self, chat_id: i64, role: &str, content: &str) -> Res<Msg> {
        self.insert_message(chat_id, role, content, None)
    }

    /// Adds the user's message, written as this persona.
    pub fn add_user_message(&mut self, chat_id: i64, content: &str, persona_id: i64) -> Res<Msg> {
        self.insert_message(chat_id, "user", content, Some(persona_id))
    }

    fn insert_message(&mut self, chat_id: i64, role: &str, content: &str, persona_id: Option<i64>) -> Res<Msg> {
        let t = now();
        self.conn.execute("DELETE FROM messages WHERE chat_id = ?1 AND deleted_at IS NOT NULL", params![chat_id])?;
        self.conn.execute(
            "INSERT INTO messages (chat_id, role, content, created_at, persona_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chat_id, role, content, t, persona_id],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute("UPDATE chats SET updated_at = ?2 WHERE id = ?1", params![chat_id, t])?;
        Ok(Msg { id, role: role.into(), content: content.into(), timings: None, persona_id, swipes: None, swipe_id: None, at: t })
    }

    pub fn finish_message(&mut self, id: i64, content: &str, timings: Option<&Timings>) -> Res<()> {
        let timings = timings.map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "UPDATE messages SET content = ?2, timings = ?3 WHERE id = ?1",
            params![id, content, timings],
        )?;
        Ok(())
    }

    pub fn delete_message(&mut self, id: i64) -> Res<()> {
        self.conn.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// The chat a message belongs to (None: no such message, or its chat is gone).
    pub fn message_chat(&self, id: i64) -> Res<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT m.chat_id FROM messages m JOIN chats ch ON ch.id = m.chat_id
                 WHERE m.id = ?1 AND m.deleted_at IS NULL AND ch.deleted_at IS NULL",
                params![id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Replaces the text of a message (its measurements stay: they describe how it was written);
    /// with variants, the active one changes with it.
    pub fn edit_message(&mut self, id: i64, content: &str) -> Res<()> {
        self.conn.execute(
            "UPDATE chats SET updated_at = ?2 WHERE id = (SELECT chat_id FROM messages WHERE id = ?1)",
            params![id, now()],
        )?;
        let stored: Option<(Option<String>, Option<i64>)> =
            self.conn.query_row("SELECT swipes, swipe_id FROM messages WHERE id = ?1", params![id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let swipes = match stored.map(|(s, i)| read_swipes(s, i)) {
            Some((Some(mut s), Some(i))) => {
                s[i] = content.to_string();
                Some(serde_json::to_string(&s)?)
            }
            _ => None,
        };
        self.conn.execute("UPDATE messages SET content = ?2, swipes = COALESCE(?3, swipes) WHERE id = ?1", params![id, content, swipes])?;
        Ok(())
    }

    /// The variants of a message and the index of the active one (None: it has none).
    pub fn message_swipes(&self, id: i64) -> Res<Option<(Vec<String>, usize)>> {
        let stored: Option<(Option<String>, Option<i64>)> =
            self.conn.query_row("SELECT swipes, swipe_id FROM messages WHERE id = ?1", params![id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        Ok(match stored.map(|(s, i)| read_swipes(s, i)) {
            Some((Some(s), Some(i))) => Some((s, i)),
            _ => None,
        })
    }

    /// Makes variant `index` the active one: `content` becomes its text. The caller checks that
    /// the message has variants and the index is among them.
    pub fn set_swipe(&mut self, id: i64, index: usize, text: &str) -> Res<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE messages SET content = ?2, swipe_id = ?3 WHERE id = ?1", params![id, text, index as i64])?;
        tx.execute("UPDATE chats SET updated_at = ?2 WHERE id = (SELECT chat_id FROM messages WHERE id = ?1)", params![id, now()])?;
        tx.commit()?;
        Ok(())
    }

    /// Removes a message and everything after it; answers how many. One step of undo
    /// (`undo_rewind`) until the chat gets a new message, another rewind, or the app restarts.
    pub fn rewind(&mut self, chat_id: i64, from_id: i64) -> Res<usize> {
        let t = now();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM messages WHERE chat_id = ?1 AND deleted_at IS NOT NULL", params![chat_id])?;
        let n = tx.execute("UPDATE messages SET deleted_at = ?3 WHERE chat_id = ?1 AND id >= ?2", params![chat_id, from_id, t])?;
        tx.execute("UPDATE chats SET updated_at = ?2 WHERE id = ?1", params![chat_id, t])?;
        tx.commit()?;
        Ok(n)
    }

    /// Brings back what the last rewind removed; answers how many messages (0: too late).
    pub fn undo_rewind(&mut self, chat_id: i64) -> Res<usize> {
        Ok(self.conn.execute("UPDATE messages SET deleted_at = NULL WHERE chat_id = ?1 AND deleted_at IS NOT NULL", params![chat_id])?)
    }
}
