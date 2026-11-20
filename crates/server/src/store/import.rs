//! Import in bulk (one transaction for many cards and personas, duplicates by content hash),
//! thumbnails, and what an export needs.

use std::path::PathBuf;

use rp_core::card;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{now, PersonaEdit, Res, Store};

/// SHA-256 (hex) of the card's content, the same whichever format it came in: the fields the
/// app uses plus lorebook, creator and version, with line ends and outer spaces normalised.
/// None: no card.
pub fn content_hash(raw: &Value) -> Option<String> {
    let c = card::from_json(raw).ok()?;
    let data = if raw["data"].is_object() { &raw["data"] } else { raw };
    let t = |s: &str| s.replace("\r\n", "\n").trim().to_string();
    let list = |l: &[String]| l.iter().map(|s| t(s)).collect::<Vec<_>>();
    let norm = json!({
        "name": t(&c.name),
        "description": t(&c.description),
        "personality": t(&c.personality),
        "scenario": t(&c.scenario),
        "first_mes": t(&c.first_mes),
        "mes_example": t(&c.mes_example),
        "system_prompt": t(&c.system_prompt),
        "post_history_instructions": t(&c.post_history_instructions),
        "creator_notes": t(&c.creator_notes),
        "alternate_greetings": list(&c.alternate_greetings),
        "tags": list(&c.tags),
        "character_book": data.get("character_book").filter(|b| !b.is_null()),
        "creator": t(data["creator"].as_str().unwrap_or("")),
        "character_version": t(data["character_version"].as_str().unwrap_or("")),
    });
    // serde_json keeps object keys sorted, so the text is canonical.
    let bytes = serde_json::to_vec(&norm).ok()?;
    Some(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// One card of an import, ready to store.
pub struct NewCard {
    pub name: String,
    pub raw: Value,
    /// The card's PNG: the import file for a PNG card, else its picture converted to PNG.
    pub png: Option<Vec<u8>>,
    pub thumb: Option<Vec<u8>>,
    pub hash: String,
    pub source_url: Option<String>,
}

/// One persona of an import (name and texts already checked).
pub struct NewPersona {
    pub edit: PersonaEdit,
    /// Picture bytes and content type.
    pub avatar: Option<(Vec<u8>, &'static str)>,
}

pub enum NewItem {
    Card(NewCard),
    Persona(NewPersona),
}

/// What became of one item: a new row, or the id of the one that already holds the same content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    Character(i64),
    DuplicateCharacter(i64),
    Persona(i64),
    DuplicatePersona(i64),
}

/// A character as an export writes it.
pub struct ExportCard {
    /// For the file name only (never logged).
    pub name: String,
    /// The card JSON, with the chat's version of the fields when a chat was given.
    pub raw: Value,
    /// Picture file and its content type.
    pub avatar: Option<(PathBuf, String)>,
}

impl Store {
    pub(super) fn thumb_path(&self, character_id: i64) -> PathBuf {
        self.dir.join("cards").join(format!("{character_id}.thumb.webp"))
    }

    /// Cards stored before content hashes existed (or whose JSON a newer build reads differently).
    pub(super) fn fill_content_hashes(&mut self) -> Res<()> {
        let rows: Vec<(i64, String)> = self
            .conn
            .prepare("SELECT id, raw_json FROM characters WHERE content_hash IS NULL")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (id, raw) in rows {
            let hash = serde_json::from_str::<Value>(&raw).ok().and_then(|raw| content_hash(&raw));
            self.conn.execute("UPDATE characters SET content_hash = ?2 WHERE id = ?1", params![id, hash.unwrap_or_default()])?;
        }
        Ok(())
    }

    pub(super) fn set_content_hash(&mut self, id: i64, raw: &Value) -> Res<()> {
        self.conn.execute("UPDATE characters SET content_hash = ?2 WHERE id = ?1", params![id, content_hash(raw).unwrap_or_default()])?;
        Ok(())
    }

    /// A character (not deleted) with this content.
    pub fn character_by_hash(&self, hash: &str) -> Res<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM characters WHERE content_hash = ?1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
                params![hash],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// A persona with the same name and description (spaces around them do not count).
    pub fn persona_like(&self, name: &str, description: &str) -> Res<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM personas WHERE trim(name) = ?1 AND trim(description) = ?2 ORDER BY id LIMIT 1",
                params![name.trim(), description.trim()],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Inserts one card with its files. `written` collects the files so a failed batch can take
    /// them back.
    fn insert_card(&mut self, c: &NewCard, written: &mut Vec<PathBuf>) -> Res<i64> {
        self.conn.execute(
            "INSERT INTO characters (name, raw_json, has_png, created_at, content_hash, source_url, thumb_rev)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![c.name, c.raw.to_string(), c.png.is_some(), now(), c.hash, c.source_url, c.thumb.as_ref().map(|_| 0)],
        )?;
        let id = self.conn.last_insert_rowid();
        if let Some(png) = &c.png {
            let path = self.png_path(id);
            std::fs::write(&path, png)?;
            written.push(path);
        }
        if let Some(thumb) = &c.thumb {
            let path = self.thumb_path(id);
            std::fs::write(&path, thumb)?;
            written.push(path);
        }
        Ok(id)
    }

    fn insert_persona(&mut self, p: &NewPersona, written: &mut Vec<PathBuf>) -> Res<i64> {
        let id = self.add_persona(&p.edit)?.id;
        if let Some((bytes, mime)) = &p.avatar {
            let path = self.persona_avatar_path(id);
            std::fs::write(&path, bytes)?;
            written.push(path);
            self.conn.execute("UPDATE personas SET avatar_mime = ?2, avatar_rev = avatar_rev + 1 WHERE id = ?1", params![id, mime])?;
        }
        Ok(id)
    }

    /// Stores all items in one transaction. A card whose content is already there (also
    /// earlier in the same batch) is not stored again, likewise a persona with the same name and
    /// description. On an error nothing is kept.
    pub fn import_items(&mut self, items: &[NewItem]) -> Res<Vec<Added>> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut written = vec![];
        let mut run = || -> Res<Vec<Added>> {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(match item {
                    NewItem::Card(c) => match self.character_by_hash(&c.hash)? {
                        Some(id) => Added::DuplicateCharacter(id),
                        None => Added::Character(self.insert_card(c, &mut written)?),
                    },
                    NewItem::Persona(p) => {
                        let name = p.edit.name.as_deref().unwrap_or("");
                        match self.persona_like(name, p.edit.description.as_deref().unwrap_or(""))? {
                            Some(id) => Added::DuplicatePersona(id),
                            None => Added::Persona(self.insert_persona(p, &mut written)?),
                        }
                    }
                });
            }
            Ok(out)
        };
        match run() {
            Ok(out) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(out)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                for path in written {
                    let _ = std::fs::remove_file(path);
                }
                Err(e)
            }
        }
    }

    /// Characters whose thumbnail is missing or older than their picture.
    pub fn thumbs_missing(&self) -> Res<Vec<i64>> {
        Ok(self
            .conn
            .prepare(
                "SELECT id FROM characters WHERE deleted_at IS NULL AND (has_png = 1 OR avatar_mime IS NOT NULL)
                   AND (thumb_rev IS NULL OR thumb_rev != avatar_rev) ORDER BY id",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?)
    }

    /// The picture to make a thumbnail from and its revision.
    pub fn thumb_source(&self, id: i64) -> Res<Option<(PathBuf, i64)>> {
        let rev: Option<i64> =
            self.conn.query_row("SELECT avatar_rev FROM characters WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        let Some(rev) = rev else { return Ok(None) };
        Ok(self.avatar(id)?.map(|(path, _)| (path, rev)))
    }

    /// Saves a thumbnail made from revision `rev` of the picture (or `None`: the picture cannot
    /// be read, so the original is shown and the thumbnail is not tried again). false: the
    /// picture changed meanwhile, the thumbnail was not kept.
    pub fn set_thumb(&mut self, id: i64, rev: i64, thumb: Option<&[u8]>) -> Res<bool> {
        let current: Option<i64> =
            self.conn.query_row("SELECT avatar_rev FROM characters WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        if current != Some(rev) {
            return Ok(false);
        }
        match thumb {
            Some(bytes) => std::fs::write(self.thumb_path(id), bytes)?,
            None => {
                let _ = std::fs::remove_file(self.thumb_path(id));
            }
        }
        self.conn.execute("UPDATE characters SET thumb_rev = ?2 WHERE id = ?1", params![id, rev])?;
        Ok(true)
    }

    /// The current thumbnail file, if there is one.
    pub fn thumb(&self, id: i64) -> Res<Option<PathBuf>> {
        let ok: Option<bool> = self
            .conn
            .query_row(
                "SELECT thumb_rev = avatar_rev FROM characters WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |r| Ok(r.get::<_, Option<bool>>(0)?.unwrap_or(false)),
            )
            .optional()?;
        let path = self.thumb_path(id);
        Ok((ok == Some(true) && path.exists()).then_some(path))
    }

    /// A character for export; with `chat`, the card as that chat plays it. Err code
    /// `character_missing` or `chat_missing` (the chat is not this character's).
    pub fn export_card(&self, id: i64, chat: Option<i64>) -> Res<Result<ExportCard, &'static str>> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT raw_json FROM characters WHERE id = ?1 AND deleted_at IS NULL", params![id], |r| r.get(0))
            .optional()?;
        let Some(raw) = raw else { return Ok(Err("character_missing")) };
        let mut raw: Value = serde_json::from_str(&raw)?;
        let mut name = card::from_json(&raw).map(|c| c.name).unwrap_or_default();
        if let Some(chat_id) = chat {
            if self.chat_character(chat_id)? != Some(id) {
                return Ok(Err("chat_missing"));
            }
            let (c, custom) = self.chat_character_card(chat_id)?.ok_or("chat vanished")?;
            if custom {
                card::apply(&mut raw, &c.card);
                name = c.card.name;
            }
        }
        Ok(Ok(ExportCard { name, raw, avatar: self.avatar(id)? }))
    }
}
