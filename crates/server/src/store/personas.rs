//! The user's personas (table `personas`): which one is active, which one a chat or a character
//! is bound to, their pictures, and the check of a card against one.

use std::path::PathBuf;

use rp_core::prompt::{Priority, UserFigure};
use rp_core::user_check::{self, Report};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{clamp_pos, now, Res, Store};

/// Which of the user's personas is active (a `personas.id`; global, for every device).
pub(super) const ACTIVE_PERSONA: &str = "active_persona";

/// One of the user's own profiles. The active one is `{{user}}` and its description goes into
/// every prompt.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Persona {
    pub id: i64,
    pub name: String,
    pub description: String,
    /// Optional extra fields for the prompt (empty = not set).
    pub pronouns: String,
    pub appearance: String,
    pub personality: String,
    pub background: String,
    /// The description is repeated with every reply instead of standing in the static head.
    pub every_turn: bool,
    /// Who wins when a card describes `{{user}}` differently (a character can override it).
    pub card_priority: Priority,
    pub has_avatar: bool,
    /// Changes whenever the picture changes; part of the image URL so devices refetch it.
    pub avatar_rev: i64,
    /// Which part of the picture shows when it is cropped: x and y in percent (CSS object-position).
    pub avatar_pos: [f64; 2],
    pub created_at: i64,
    pub updated_at: i64,
}

/// The columns `Persona` is read from.
const PERSONA: &str = "id, name, description, avatar_mime IS NOT NULL, avatar_rev, avatar_x, avatar_y, created_at, updated_at,
    pronouns, appearance, personality, background, every_turn, card_priority";

impl Persona {
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Persona> {
        Ok(Persona {
            id: r.get(0)?,
            name: r.get(1)?,
            description: r.get(2)?,
            has_avatar: r.get(3)?,
            avatar_rev: r.get(4)?,
            avatar_pos: [r.get(5)?, r.get(6)?],
            created_at: r.get(7)?,
            updated_at: r.get(8)?,
            pronouns: r.get(9)?,
            appearance: r.get(10)?,
            personality: r.get(11)?,
            background: r.get(12)?,
            every_turn: r.get(13)?,
            card_priority: Priority::parse(&r.get::<_, String>(14)?).unwrap_or_default(),
        })
    }

    /// What the prompt tells about this persona (besides its name).
    pub fn figure(&self) -> UserFigure {
        UserFigure {
            pronouns: self.pronouns.clone(),
            appearance: self.appearance.clone(),
            personality: self.personality.clone(),
            background: self.background.clone(),
            description: self.description.clone(),
            every_turn: self.every_turn,
            priority: self.card_priority,
        }
    }
}

/// A new persona (`name` required) or a change to one: fields left out (`None`) stay as they are.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PersonaEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    pub pronouns: Option<String>,
    pub appearance: Option<String>,
    pub personality: Option<String>,
    pub background: Option<String>,
    pub every_turn: Option<bool>,
    /// "persona" | "card" (checked by the server before it gets here).
    pub card_priority: Option<String>,
}

impl PersonaEdit {
    pub fn name(name: &str) -> PersonaEdit {
        PersonaEdit { name: Some(name.into()), ..PersonaEdit::default() }
    }
}

/// Result of binding a chat or a character to a persona.
#[derive(Debug, PartialEq)]
pub enum Bind {
    Done,
    /// No such chat or character.
    NoTarget,
    /// No such persona.
    NoPersona,
}

/// Result of `Store::user_check`.
#[derive(Debug, PartialEq)]
pub enum UserCheck {
    Done { persona_id: i64, report: Report },
    NoCharacter,
    NoPersona,
}

/// Result of deleting a persona.
#[derive(Debug, PartialEq)]
pub enum PersonaDelete {
    /// Deleted; the id of the persona that is active now.
    Done(i64),
    Missing,
    /// The last persona stays: there is always one.
    Last,
}

impl Store {
    /// A persona's picture.
    pub(super) fn persona_avatar_path(&self, id: i64) -> PathBuf {
        self.dir.join("personas").join(format!("{id}.avatar"))
    }

    pub fn personas(&self) -> Res<Vec<Persona>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {PERSONA} FROM personas ORDER BY name COLLATE NOCASE, id"))?;
        let rows = stmt.query_map([], Persona::read)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn persona(&self, id: i64) -> Res<Option<Persona>> {
        Ok(self.conn.query_row(&format!("SELECT {PERSONA} FROM personas WHERE id = ?1"), params![id], Persona::read).optional()?)
    }

    /// The active persona's id. Should the setting point nowhere, the oldest persona counts.
    pub fn active_persona_id(&self) -> Res<i64> {
        if let Some(id) = self.setting(ACTIVE_PERSONA)?.and_then(|v| v.parse::<i64>().ok()) {
            if self.persona(id)?.is_some() {
                return Ok(id);
            }
        }
        let first: Option<i64> = self.conn.query_row("SELECT MIN(id) FROM personas", [], |r| r.get(0))?;
        first.ok_or_else(|| "no persona".into())
    }

    pub fn active_persona(&self) -> Res<Persona> {
        let id = self.active_persona_id()?;
        self.persona(id)?.ok_or_else(|| "active persona vanished".into())
    }

    /// A new persona; `p.name` is required, fields left out are empty.
    pub fn add_persona(&mut self, p: &PersonaEdit) -> Res<Persona> {
        let name = p.name.as_deref().ok_or("persona without a name")?;
        let text = |f: &Option<String>| f.clone().unwrap_or_default();
        self.conn.execute(
            "INSERT INTO personas (name, description, pronouns, appearance, personality, background, every_turn, card_priority, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?8, ?9, ?7, ?7)",
            params![
                name,
                text(&p.description),
                text(&p.pronouns),
                text(&p.appearance),
                text(&p.personality),
                text(&p.background),
                now(),
                p.every_turn.unwrap_or(false),
                p.card_priority.as_deref().and_then(Priority::parse).unwrap_or_default().as_str()
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.persona(id)?.ok_or_else(|| "persona vanished after insert".into())
    }

    /// Changes the given fields. None: no such persona.
    pub fn update_persona(&mut self, id: i64, p: &PersonaEdit) -> Res<Option<Persona>> {
        let n = self.conn.execute(
            "UPDATE personas SET name = COALESCE(?2, name), description = COALESCE(?3, description),
               pronouns = COALESCE(?4, pronouns), appearance = COALESCE(?5, appearance),
               personality = COALESCE(?6, personality), background = COALESCE(?7, background),
               every_turn = COALESCE(?9, every_turn), card_priority = COALESCE(?10, card_priority), updated_at = ?8
             WHERE id = ?1",
            params![
                id,
                p.name,
                p.description,
                p.pronouns,
                p.appearance,
                p.personality,
                p.background,
                now(),
                p.every_turn,
                p.card_priority.as_deref().and_then(Priority::parse).map(Priority::as_str)
            ],
        )?;
        if n == 0 {
            return Ok(None);
        }
        self.persona(id)
    }

    /// false: no such persona.
    pub fn activate_persona(&mut self, id: i64) -> Res<bool> {
        if self.persona(id)?.is_none() {
            return Ok(false);
        }
        self.set_setting(ACTIVE_PERSONA, &id.to_string())?;
        Ok(true)
    }

    /// Deletes for good, with its picture. The last persona cannot go; if the active one goes,
    /// the oldest remaining one becomes active.
    pub fn delete_persona(&mut self, id: i64) -> Res<PersonaDelete> {
        if self.persona(id)?.is_none() {
            return Ok(PersonaDelete::Missing);
        }
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM personas", [], |r| r.get(0))?;
        if count <= 1 {
            return Ok(PersonaDelete::Last);
        }
        let was_active = self.active_persona_id()? == id;
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM personas WHERE id = ?1", params![id])?;
        tx.execute("UPDATE chats SET persona_id = NULL WHERE persona_id = ?1", params![id])?;
        tx.execute("UPDATE characters SET default_persona_id = NULL WHERE default_persona_id = ?1", params![id])?;
        tx.execute("UPDATE messages SET persona_id = NULL WHERE persona_id = ?1", params![id])?;
        if was_active {
            let next: i64 = tx.query_row("SELECT MIN(id) FROM personas", [], |r| r.get(0))?;
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![ACTIVE_PERSONA, next.to_string()],
            )?;
        }
        tx.commit()?;
        let _ = std::fs::remove_file(self.persona_avatar_path(id));
        Ok(PersonaDelete::Done(self.active_persona_id()?))
    }

    /// The persona a chat plays with: its own if it is bound to one, else the active one.
    pub fn chat_persona(&self, chat_id: i64) -> Res<Persona> {
        let bound: Option<Option<i64>> =
            self.conn.query_row("SELECT persona_id FROM chats WHERE id = ?1", params![chat_id], |r| r.get(0)).optional()?;
        if let Some(p) = bound.flatten().map(|id| self.persona(id)).transpose()?.flatten() {
            return Ok(p);
        }
        self.active_persona()
    }

    /// The persona a new chat with this character plays with: the character's default, else the
    /// active one.
    pub fn persona_for_new_chat(&self, character_id: i64) -> Res<Persona> {
        let default: Option<Option<i64>> = self
            .conn
            .query_row("SELECT default_persona_id FROM characters WHERE id = ?1", params![character_id], |r| r.get(0))
            .optional()?;
        if let Some(p) = default.flatten().map(|id| self.persona(id)).transpose()?.flatten() {
            return Ok(p);
        }
        self.active_persona()
    }

    /// Binds a chat to a persona (`Some`) or lets it follow the active one again (`None`).
    pub fn set_chat_persona(&mut self, chat_id: i64, persona_id: Option<i64>) -> Res<Bind> {
        if self.chat_character(chat_id)?.is_none() {
            return Ok(Bind::NoTarget);
        }
        if let Some(id) = persona_id {
            if self.persona(id)?.is_none() {
                return Ok(Bind::NoPersona);
            }
        }
        self.conn.execute("UPDATE chats SET persona_id = ?2 WHERE id = ?1", params![chat_id, persona_id])?;
        Ok(Bind::Done)
    }

    /// Sets (`Some`) or clears (`None`) the persona new chats with this character are bound to.
    /// Existing chats keep their binding.
    pub fn set_default_persona(&mut self, character_id: i64, persona_id: Option<i64>) -> Res<Bind> {
        if self.character(character_id)?.is_none() {
            return Ok(Bind::NoTarget);
        }
        if let Some(id) = persona_id {
            if self.persona(id)?.is_none() {
                return Ok(Bind::NoPersona);
            }
        }
        self.conn.execute("UPDATE characters SET default_persona_id = ?2 WHERE id = ?1", params![character_id, persona_id])?;
        Ok(Bind::Done)
    }

    /// Sets (`Some`) or clears (`None`: as the persona says) who wins when this character's card
    /// describes `{{user}}` differently. false: no such character.
    pub fn set_user_priority(&mut self, character_id: i64, priority: Option<Priority>) -> Res<bool> {
        if self.character(character_id)?.is_none() {
            return Ok(false);
        }
        self.conn.execute(
            "UPDATE characters SET user_priority = ?2 WHERE id = ?1",
            params![character_id, priority.map(Priority::as_str)],
        )?;
        Ok(true)
    }

    /// What the character's card says about `{{user}}`, checked against `persona_id`, else
    /// against the persona of a new chat with this character.
    pub fn user_check(&self, character_id: i64, persona_id: Option<i64>) -> Res<UserCheck> {
        let Some(character) = self.character(character_id)? else { return Ok(UserCheck::NoCharacter) };
        let persona = match persona_id {
            Some(id) => match self.persona(id)? {
                Some(p) => p,
                None => return Ok(UserCheck::NoPersona),
            },
            None => self.persona_for_new_chat(character_id)?,
        };
        let report = user_check::check(&character.card, &persona.name, &persona.figure());
        Ok(UserCheck::Done { persona_id: persona.id, report })
    }

    /// Sets (`Some`: bytes and content type) or removes (`None`) the picture. false: no such persona.
    pub fn set_persona_avatar(&mut self, id: i64, image: Option<(&[u8], &str)>) -> Res<bool> {
        if self.persona(id)?.is_none() {
            return Ok(false);
        }
        match image {
            Some((bytes, _)) => std::fs::write(self.persona_avatar_path(id), bytes)?,
            None => {
                let _ = std::fs::remove_file(self.persona_avatar_path(id));
            }
        }
        self.conn.execute(
            "UPDATE personas SET avatar_mime = ?2, avatar_rev = avatar_rev + 1, updated_at = ?3 WHERE id = ?1",
            params![id, image.map(|(_, mime)| mime), now()],
        )?;
        Ok(true)
    }

    /// Where the picture is cropped (percent, clamped to 0..=100). false: no such persona.
    pub fn set_persona_avatar_pos(&mut self, id: i64, x: f64, y: f64) -> Res<bool> {
        let n = self.conn.execute(
            "UPDATE personas SET avatar_x = ?2, avatar_y = ?3 WHERE id = ?1",
            params![id, clamp_pos(x), clamp_pos(y)],
        )?;
        Ok(n > 0)
    }

    /// File and content type of the persona's picture, if it has one.
    pub fn persona_avatar(&self, id: i64) -> Res<Option<(PathBuf, String)>> {
        let mime: Option<Option<String>> =
            self.conn.query_row("SELECT avatar_mime FROM personas WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        Ok(mime.flatten().map(|m| (self.persona_avatar_path(id), m)))
    }
}
