//! Characters (table `characters`): imported cards, their pictures and soft delete with undo.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use rp_core::card::{self, Card, Imported};
use rp_core::prompt::{self, Priority};
use rp_core::user_check::{self, Fit};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use super::{clamp_pos, now, Persona, Res, Store};

/// The columns after the second one of `Row` (the second is the card's text or its `card_key`).
const META: &str = "c.has_png, c.avatar_mime, c.avatar_rev,
    (SELECT COUNT(*) FROM chats WHERE character_id = c.id AND deleted_at IS NULL), c.avatar_x, c.avatar_y,
    (SELECT p.id FROM personas p WHERE p.id = c.default_persona_id), c.user_priority, c.favorite, c.created_at,
    (SELECT MAX(updated_at) FROM chats WHERE character_id = c.id AND deleted_at IS NULL),
    (SELECT group_concat(t.name, char(31)) FROM character_tags ct JOIN tags t ON t.id = ct.tag_id WHERE ct.character_id = c.id), c.tagline";

/// Longest own short text of a character (characters, after trimming).
pub const TAGLINE_MAX: usize = 160;

/// What the library shows of a card that depends on its text alone, worked out once per version
/// of the text (parsing a large card takes milliseconds each).
struct Derived {
    name: String,
    /// First non-empty line of creator notes, personality or scenario (before `{{user}}` etc.).
    blurb_src: String,
    tags: Vec<String>,
}

/// One card version in the cache: None = the text is not a card (left out of the library), and
/// the user check against the last few personas (by `persona_print`).
struct Cached {
    derived: Option<Arc<Derived>>,
    checks: Vec<(u64, Fit)>,
}

/// User checks kept per card version (one per persona the library was shown with).
const CHECKS_KEPT: usize = 4;

/// Cache of `Derived` by (character id, `card_key`). `card_key` is random and set anew by a
/// trigger with every write of `raw_json` (migration 20), so an entry can never outlive the text
/// it was made from — also across a restored backup or two databases in one process.
static CACHE: LazyLock<Mutex<HashMap<(i64, i64), Cached>>> = LazyLock::new(Default::default);

fn cache() -> std::sync::MutexGuard<'static, HashMap<(i64, i64), Cached>> {
    CACHE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Stands for everything the user check reads of a persona.
fn persona_print(p: &Persona) -> u64 {
    let mut h = DefaultHasher::new();
    format!("{:?}", (&p.name, p.figure())).hash(&mut h);
    h.finish()
}

fn derive(card: &Card) -> Derived {
    let blurb_src = [&card.creator_notes, &card.personality, &card.scenario]
        .into_iter()
        .find_map(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("");
    Derived { name: card.name.clone(), blurb_src: blurb_src.into(), tags: card.tags.clone() }
}

/// The short text: the blurb source with `{{char}}`/`{{user}}` resolved, at most 140 characters.
fn blurb(d: &Derived, user_name: &str) -> String {
    let mut blurb: String = prompt::resolve(&d.blurb_src, &d.name, user_name).chars().take(140).collect();
    if d.blurb_src.chars().count() > 140 {
        blurb.push('…');
    }
    blurb
}

/// A library entry from its row, what its text says and the user check.
fn assemble(row: Row, d: &Derived, user_name: &str, user_check_status: Fit) -> CharInfo {
    let mut own_tags: Vec<String> = row.own_tags.as_deref().map_or(vec![], |t| t.split(char::from(31u8)).map(String::from).collect());
    own_tags.sort_by_key(|t| t.to_lowercase());
    CharInfo {
        id: row.id,
        name: d.name.clone(),
        blurb: blurb(d, user_name),
        tagline: row.tagline,
        tags: d.tags.clone(),
        has_avatar: row.has_png || row.avatar_mime.is_some(),
        custom_avatar: row.avatar_mime.is_some(),
        avatar_rev: row.avatar_rev,
        avatar_pos: row.avatar_pos,
        chats: row.chats,
        default_persona_id: row.default_persona_id,
        user_priority: row.user_priority.as_deref().and_then(Priority::parse),
        user_check_status,
        favorite: row.favorite,
        own_tags,
        created_at: row.created_at,
        last_chat_at: row.last_chat_at,
    }
}

/// How many live chats a character has and when one last changed (what a finished reply changes
/// in the library; `GET /library?part=chats`).
#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub id: i64,
    pub chats: i64,
    pub last_chat_at: Option<i64>,
}

struct Row {
    id: i64,
    raw_json: String,
    has_png: bool,
    avatar_mime: Option<String>,
    avatar_rev: i64,
    chats: i64,
    avatar_pos: [f64; 2],
    default_persona_id: Option<i64>,
    user_priority: Option<String>,
    favorite: bool,
    created_at: i64,
    last_chat_at: Option<i64>,
    own_tags: Option<String>,
    tagline: Option<String>,
}

impl Row {
    /// For `SELECT c.id, c.raw_json, {META} FROM characters c`.
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
        Row::read_with(r, r.get(1)?)
    }

    /// For `SELECT c.id, c.card_key, {META} FROM characters c`: the text stays out (`raw_json`
    /// empty), the key comes back beside the row.
    fn read_keyed(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, Row)> {
        Ok((r.get(1)?, Row::read_with(r, String::new())?))
    }

    fn read_with(r: &rusqlite::Row<'_>, raw_json: String) -> rusqlite::Result<Row> {
        Ok(Row {
            id: r.get(0)?,
            raw_json,
            has_png: r.get(2)?,
            avatar_mime: r.get(3)?,
            avatar_rev: r.get(4)?,
            chats: r.get(5)?,
            avatar_pos: [r.get(6)?, r.get(7)?],
            default_persona_id: r.get(8)?,
            user_priority: r.get(9)?,
            favorite: r.get(10)?,
            created_at: r.get(11)?,
            last_chat_at: r.get(12)?,
            own_tags: r.get(13)?,
            tagline: r.get(14)?,
        })
    }
}

/// What the library and the chat header show about a character.
#[derive(Debug, Clone, Serialize)]
pub struct CharInfo {
    pub id: i64,
    pub name: String,
    /// The automatic short text: first line of creator notes, personality or scenario.
    pub blurb: String,
    /// The user's own short text under the card; null = the automatic `blurb`.
    pub tagline: Option<String>,
    /// The card's own tags (the user's are `own_tags`).
    pub tags: Vec<String>,
    pub has_avatar: bool,
    /// The picture was chosen by the user (otherwise it is the card's own PNG).
    pub custom_avatar: bool,
    /// Changes whenever the picture changes; part of the image URL so devices refetch it.
    pub avatar_rev: i64,
    /// Which part of the picture shows when it is cropped: x and y in percent (CSS object-position).
    pub avatar_pos: [f64; 2],
    pub chats: i64,
    /// New chats with this character are bound to this persona (null: they follow the active one).
    pub default_persona_id: Option<i64>,
    /// Who wins when the card describes `{{user}}` differently; null = as the chat's persona says.
    pub user_priority: Option<Priority>,
    /// Worst verdict of the user check against the persona of a new chat (`Store::user_check`).
    pub user_check_status: Fit,
    /// Marked with a star in the library.
    pub favorite: bool,
    /// The user's own tags (not the card's `tags`), by name.
    pub own_tags: Vec<String>,
    /// When the character was imported (Unix seconds).
    pub created_at: i64,
    /// When one of its chats last changed; null: no chat.
    pub last_chat_at: Option<i64>,
}

pub struct Character {
    pub info: CharInfo,
    pub card: Card,
}

impl Store {
    /// The card's own PNG (the import file).
    pub(super) fn png_path(&self, character_id: i64) -> PathBuf {
        self.dir.join("cards").join(format!("{character_id}.png"))
    }

    /// A picture the user chose instead.
    pub(super) fn avatar_path(&self, character_id: i64) -> PathBuf {
        self.dir.join("cards").join(format!("{character_id}.avatar"))
    }

    /// Stores an imported card; `png` is the original file if the import was a PNG.
    pub fn add_character(&mut self, imported: &Imported, png: Option<&[u8]>) -> Res<CharInfo> {
        self.conn.execute(
            "INSERT INTO characters (name, raw_json, has_png, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![imported.card.name, imported.raw.to_string(), png.is_some(), now()],
        )?;
        let id = self.conn.last_insert_rowid();
        if let Some(bytes) = png {
            if let Err(e) = std::fs::write(self.png_path(id), bytes) {
                self.conn.execute("DELETE FROM characters WHERE id = ?1", params![id])?;
                return Err(e.into());
            }
        }
        self.set_content_hash(id, &imported.raw)?;
        Ok(self.character(id)?.ok_or("character vanished after insert")?.info)
    }

    /// Saves edited card fields. None: no such character.
    pub fn update_character(&mut self, id: i64, edited: &Card) -> Res<Option<Character>> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT raw_json FROM characters WHERE id = ?1 AND deleted_at IS NULL", params![id], |r| r.get(0))
            .optional()?;
        let Some(raw) = raw else { return Ok(None) };
        let mut raw: serde_json::Value = serde_json::from_str(&raw)?;
        card::apply(&mut raw, edited);
        self.conn.execute(
            "UPDATE characters SET name = ?2, raw_json = ?3 WHERE id = ?1",
            params![id, edited.name, raw.to_string()],
        )?;
        self.set_content_hash(id, &raw)?;
        self.character(id)
    }

    /// Sets the own short text (`None` or empty after trimming: back to the automatic one).
    /// Err when it is longer than [`TAGLINE_MAX`]; Ok(false): no such character.
    pub fn set_tagline(&mut self, id: i64, tagline: Option<&str>) -> Res<bool> {
        let tagline = tagline.map(str::trim).filter(|t| !t.is_empty());
        if tagline.is_some_and(|t| t.chars().count() > TAGLINE_MAX) {
            return Err("tagline too long".into());
        }
        let n = self.conn.execute("UPDATE characters SET tagline = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![id, tagline])?;
        Ok(n > 0)
    }

    /// Sets (`Some`: bytes and content type) or removes (`None`) the user-chosen picture.
    /// false: no such character.
    pub fn set_avatar(&mut self, id: i64, image: Option<(&[u8], &str)>) -> Res<bool> {
        if self.character(id)?.is_none() {
            return Ok(false);
        }
        match image {
            Some((bytes, _)) => std::fs::write(self.avatar_path(id), bytes)?,
            None => {
                let _ = std::fs::remove_file(self.avatar_path(id));
            }
        }
        self.conn.execute(
            "UPDATE characters SET avatar_mime = ?2, avatar_rev = avatar_rev + 1 WHERE id = ?1",
            params![id, image.map(|(_, mime)| mime)],
        )?;
        Ok(true)
    }

    /// Where the picture is cropped (percent, clamped to 0..=100). false: no such character.
    pub fn set_avatar_pos(&mut self, id: i64, x: f64, y: f64) -> Res<bool> {
        let n = self.conn.execute(
            "UPDATE characters SET avatar_x = ?2, avatar_y = ?3 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, clamp_pos(x), clamp_pos(y)],
        )?;
        Ok(n > 0)
    }

    /// File and content type of the picture to show: the user's choice, else the card's PNG.
    pub fn avatar(&self, id: i64) -> Res<Option<(PathBuf, String)>> {
        let row: Option<(bool, Option<String>)> = self
            .conn
            .query_row("SELECT has_png, avatar_mime FROM characters WHERE id = ?1", params![id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(match row {
            Some((_, Some(mime))) => Some((self.avatar_path(id), mime)),
            Some((true, None)) => Some((self.png_path(id), "image/png".into())),
            _ => None,
        })
    }

    /// `active` is the active persona: its name is `{{user}}` in the blurb, and it is the persona
    /// of a new chat unless the character has a default.
    fn info(&self, row: Row, active: &Persona) -> Res<Option<Character>> {
        let Some(raw) = serde_json::from_str::<Value>(&row.raw_json).ok() else { return Ok(None) };
        let Ok(card) = card::from_json(&raw) else { return Ok(None) };
        let default = row.default_persona_id.map(|p| self.persona(p)).transpose()?.flatten();
        let check_with = default.as_ref().unwrap_or(active);
        let user_check_status = user_check::check(&card, &check_with.name, &check_with.figure()).status;
        let info = assemble(row, &derive(&card), &active.name, user_check_status);
        Ok(Some(Character { info, card }))
    }

    /// Every live character, by name. What depends on a card's text alone comes from `CACHE`;
    /// only cards new or changed since the last call (or checked against another persona) are
    /// read and worked out again.
    pub fn characters(&self) -> Res<Vec<CharInfo>> {
        let active = self.active_persona()?;
        let personas: HashMap<i64, Persona> = self.personas()?.into_iter().map(|p| (p.id, p)).collect();
        let prints: HashMap<i64, u64> = personas.values().map(|p| (p.id, persona_print(p))).collect();
        let active_print = persona_print(&active);
        // The persona a card is checked against: its default (if that still exists), else the active one.
        let check_with = |row: &Row| row.default_persona_id.and_then(|p| personas.get(&p)).unwrap_or(&active);
        let print_of = |row: &Row| row.default_persona_id.and_then(|p| prints.get(&p).copied()).unwrap_or(active_print);

        let rows: Vec<(i64, Row)> = self
            .conn
            .prepare(&format!(
                "SELECT c.id, c.card_key, {META} FROM characters c WHERE c.deleted_at IS NULL ORDER BY c.name COLLATE NOCASE, c.id"
            ))?
            .query_map([], Row::read_keyed)?
            .collect::<Result<_, _>>()?;

        enum Slot {
            Ready(Arc<Derived>, Fit),
            NotACard,
            /// Not in the cache, or not yet checked against this persona (then with what is known).
            Missing(Option<Arc<Derived>>),
        }
        let mut slots: Vec<Slot> = {
            let cache = cache();
            rows.iter()
                .map(|(key, row)| match cache.get(&(row.id, *key)) {
                    Some(Cached { derived: None, .. }) => Slot::NotACard,
                    Some(Cached { derived: Some(d), checks }) => match checks.iter().find(|(p, _)| *p == print_of(row)) {
                        Some(&(_, fit)) => Slot::Ready(d.clone(), fit),
                        None => Slot::Missing(Some(d.clone())),
                    },
                    None => Slot::Missing(None),
                })
                .collect()
        };

        // Work out what the cache lacks, without its lock. The text and its key are read together,
        // so a card edited meanwhile is cached under its new key.
        // (id, key), what the text says (None: not a card), the user check by persona print.
        type Fresh = ((i64, i64), Option<Arc<Derived>>, Option<(u64, Fit)>);
        let mut fresh: Vec<Fresh> = vec![];
        let mut text = self.conn.prepare("SELECT raw_json, card_key FROM characters WHERE id = ?1")?;
        for ((_, row), slot) in rows.iter().zip(slots.iter_mut()) {
            let Slot::Missing(known) = slot else { continue };
            let Some((raw, key)) = text.query_row(params![row.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).optional()? else {
                *slot = Slot::NotACard;
                continue;
            };
            let parsed = serde_json::from_str::<Value>(&raw).ok().and_then(|raw| card::from_json(&raw).ok());
            let Some(card) = parsed else {
                fresh.push(((row.id, key), None, None));
                *slot = Slot::NotACard;
                continue;
            };
            let d = known.take().unwrap_or_else(|| Arc::new(derive(&card)));
            let p = check_with(row);
            let fit = user_check::check(&card, &p.name, &p.figure()).status;
            fresh.push(((row.id, key), Some(d.clone()), Some((print_of(row), fit))));
            *slot = Slot::Ready(d, fit);
        }
        if !fresh.is_empty() {
            let mut cache = cache();
            for (key, derived, check) in fresh {
                let entry = cache.entry(key).or_insert_with(|| Cached { derived: derived.clone(), checks: vec![] });
                entry.derived = derived;
                if let Some((print, fit)) = check {
                    entry.checks.retain(|(p, _)| *p != print);
                    entry.checks.insert(0, (print, fit));
                    entry.checks.truncate(CHECKS_KEPT);
                }
            }
            // Old versions are dropped once the cache holds far more than this library.
            if cache.len() > 2 * rows.len() + 256 {
                let live: HashSet<(i64, i64)> = rows.iter().map(|(key, row)| (row.id, *key)).collect();
                cache.retain(|k, _| live.contains(k));
            }
        }

        Ok(rows
            .into_iter()
            .zip(slots)
            .filter_map(|((_, row), slot)| match slot {
                Slot::Ready(d, fit) => Some(assemble(row, &d, &active.name, fit)),
                Slot::NotACard | Slot::Missing(_) => None,
            })
            .collect())
    }

    /// Chat count and last chat time of every character with a live chat (by id; one without is
    /// not listed: no chat, no time).
    pub fn activity(&self) -> Res<Vec<Activity>> {
        let mut stmt = self.conn.prepare(
            "SELECT character_id, COUNT(*), MAX(updated_at) FROM chats WHERE deleted_at IS NULL GROUP BY character_id ORDER BY character_id",
        )?;
        let rows = stmt.query_map([], |r| Ok(Activity { id: r.get(0)?, chats: r.get(1)?, last_chat_at: r.get(2)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn character(&self, id: i64) -> Res<Option<Character>> {
        let row = self
            .conn
            .query_row(
                &format!("SELECT c.id, c.raw_json, {META} FROM characters c WHERE c.id = ?1 AND c.deleted_at IS NULL"),
                params![id],
                Row::read,
            )
            .optional()?;
        let active = self.active_persona()?;
        row.map_or(Ok(None), |row| self.info(row, &active))
    }

    /// Soft delete (`deleted = true`) or undo. Deleted cards are purged at the next start.
    pub fn set_deleted(&mut self, id: i64, deleted: bool) -> Res<bool> {
        let n = self.conn.execute(
            "UPDATE characters SET deleted_at = ?2 WHERE id = ?1",
            params![id, deleted.then(now)],
        )?;
        Ok(n > 0)
    }
}
