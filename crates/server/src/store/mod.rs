//! SQLite storage: characters, chats, messages, personas, settings, engines and model profiles.
//! Card PNGs live next to the database.
//!
//! `Store` owns the connection; this file opens it, runs the migrations and cleans up what the
//! last session left behind. Each table has its own file with the `impl Store` block for it.

mod backup;
mod characters;
mod chats;
mod engines;
mod fetch;
mod general;
mod import;
mod looks;
mod messages;
mod models;
mod personas;
mod purpose;
mod search;
mod settings;
mod stats;
mod tags;
mod variants;
#[cfg(test)]
pub(crate) mod tests;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

pub use characters::{Character, TAGLINE_MAX};
pub use chats::{ImportedChat, ImportedMsg};
pub use engines::{EngineDelete, NewEngine};
pub use looks::{BackdropOf, Frame, Look, BACKDROP_MODES};
pub use import::{content_hash, Added, ExportCard, NewCard, NewItem, NewPersona};
pub use messages::Timings;
pub use models::{Profile, ProfileDelete, ProfileEdit, PROFILE_NAME_MAX};
pub use personas::{Bind, Persona, PersonaDelete, PersonaEdit, UserCheck};
#[cfg(test)]
pub use search::{MARK_END, MARK_START};
pub use tags::{clean_tag, Bulk};
#[cfg(test)]
pub use tags::TagCount;

/// Any failure of the database or a file the store keeps.
pub type StoreError = Box<dyn std::error::Error + Send + Sync>;
pub type Res<T> = Result<T, StoreError>;

/// Applied in order; `PRAGMA user_version` remembers how many already ran. Never edit an entry
/// that has shipped — append a new one.
const MIGRATIONS: &[&str] = &["
CREATE TABLE characters (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  raw_json TEXT NOT NULL,
  has_png INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  deleted_at INTEGER
);
CREATE TABLE chats (
  id INTEGER PRIMARY KEY,
  character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  content TEXT NOT NULL,
  timings TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX messages_chat ON messages(chat_id, id);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
", "
ALTER TABLE characters ADD COLUMN avatar_mime TEXT;
ALTER TABLE characters ADD COLUMN avatar_rev INTEGER NOT NULL DEFAULT 0;
ALTER TABLE characters ADD COLUMN sfw_locked INTEGER NOT NULL DEFAULT 0;
ALTER TABLE characters ADD COLUMN gate_reasons TEXT NOT NULL DEFAULT '[]';
", "
ALTER TABLE characters ADD COLUMN avatar_x REAL NOT NULL DEFAULT 50;
ALTER TABLE characters ADD COLUMN avatar_y REAL NOT NULL DEFAULT 12;
ALTER TABLE chats ADD COLUMN deleted_at INTEGER;
ALTER TABLE chats ADD COLUMN card_json TEXT;
", "
ALTER TABLE chats ADD COLUMN title TEXT NOT NULL DEFAULT '';
", "
ALTER TABLE messages ADD COLUMN deleted_at INTEGER;
", "
CREATE TABLE logs (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  level TEXT NOT NULL,
  source TEXT NOT NULL,
  category TEXT NOT NULL,
  message TEXT NOT NULL,
  data TEXT,
  chat_id INTEGER,
  character_id INTEGER,
  device TEXT NOT NULL DEFAULT ''
);
CREATE INDEX logs_ts ON logs(ts, id);
", "
DROP TABLE IF EXISTS logs;
", "
CREATE TABLE personas (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  avatar_mime TEXT,
  avatar_rev INTEGER NOT NULL DEFAULT 0,
  avatar_x REAL NOT NULL DEFAULT 50,
  avatar_y REAL NOT NULL DEFAULT 12,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
INSERT INTO personas (name, created_at, updated_at)
  VALUES (COALESCE((SELECT value FROM settings WHERE key = 'user_name'), 'User'),
          CAST(strftime('%s', 'now') AS INTEGER), CAST(strftime('%s', 'now') AS INTEGER));
INSERT INTO settings (key, value) VALUES ('active_persona', CAST(last_insert_rowid() AS TEXT))
  ON CONFLICT(key) DO UPDATE SET value = excluded.value;
", "
ALTER TABLE personas ADD COLUMN pronouns TEXT NOT NULL DEFAULT '';
ALTER TABLE personas ADD COLUMN appearance TEXT NOT NULL DEFAULT '';
ALTER TABLE personas ADD COLUMN personality TEXT NOT NULL DEFAULT '';
ALTER TABLE personas ADD COLUMN background TEXT NOT NULL DEFAULT '';
ALTER TABLE chats ADD COLUMN persona_id INTEGER;
ALTER TABLE characters ADD COLUMN default_persona_id INTEGER;
ALTER TABLE messages ADD COLUMN persona_id INTEGER;
", "
ALTER TABLE personas ADD COLUMN every_turn INTEGER NOT NULL DEFAULT 0;
", "
ALTER TABLE personas ADD COLUMN card_priority TEXT NOT NULL DEFAULT 'persona';
ALTER TABLE characters ADD COLUMN user_priority TEXT;
", "
CREATE TABLE engines (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  exe TEXT NOT NULL,
  extra_path TEXT NOT NULL DEFAULT '[]',
  backend TEXT NOT NULL DEFAULT 'unknown',
  version TEXT,
  origin TEXT NOT NULL DEFAULT 'own' CHECK (origin IN ('own', 'official')),
  created_at INTEGER NOT NULL
);
CREATE TABLE model_profiles (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  engine_id INTEGER NOT NULL REFERENCES engines(id),
  model_file TEXT NOT NULL,
  load_args TEXT NOT NULL DEFAULT '[]',
  sampler TEXT NOT NULL DEFAULT '{}',
  reasoning TEXT NOT NULL DEFAULT 'off',
  measured INTEGER NOT NULL DEFAULT 0,
  active INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX model_profiles_one_active ON model_profiles(active) WHERE active = 1;
", "
ALTER TABLE characters ADD COLUMN content_hash TEXT;
ALTER TABLE characters ADD COLUMN source_url TEXT;
ALTER TABLE characters ADD COLUMN thumb_rev INTEGER;
CREATE INDEX characters_content_hash ON characters(content_hash);
", "
CREATE VIRTUAL TABLE messages_fts USING fts5(content, tokenize = 'unicode61 remove_diacritics 2');
INSERT INTO messages_fts (rowid, content) SELECT id, content FROM messages WHERE deleted_at IS NULL;
CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages WHEN new.deleted_at IS NULL BEGIN
  INSERT INTO messages_fts (rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
  DELETE FROM messages_fts WHERE rowid = old.id;
END;
CREATE TRIGGER messages_fts_update AFTER UPDATE OF content, deleted_at ON messages BEGIN
  DELETE FROM messages_fts WHERE rowid = old.id;
  INSERT INTO messages_fts (rowid, content) SELECT new.id, new.content WHERE new.deleted_at IS NULL;
END;
", "
ALTER TABLE characters ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE);
CREATE TABLE character_tags (
  character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
  tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (character_id, tag_id)
) WITHOUT ROWID;
CREATE INDEX character_tags_tag ON character_tags(tag_id);
", "
ALTER TABLE characters ADD COLUMN tagline TEXT;
", "
ALTER TABLE messages ADD COLUMN swipes TEXT;
ALTER TABLE messages ADD COLUMN swipe_id INTEGER;
ALTER TABLE chats ADD COLUMN import_hash TEXT;
CREATE INDEX chats_import_hash ON chats(import_hash);
", "
CREATE TABLE chat_looks (
  chat_id INTEGER PRIMARY KEY REFERENCES chats(id) ON DELETE CASCADE,
  look TEXT NOT NULL DEFAULT '{}',
  backdrop TEXT NOT NULL DEFAULT 'character',
  bg_rev INTEGER NOT NULL DEFAULT 0,
  bg_frame TEXT
);
CREATE TABLE character_backdrops (
  character_id INTEGER PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
  bg_rev INTEGER NOT NULL DEFAULT 0,
  bg_frame TEXT
);
",
// Migration 19: the pictures of messages ("Szene als Bild"), shipped in 0.1.0. The feature is
// gone; the table stays as it is and is no longer used.
"
CREATE TABLE message_scenes (
  message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
  file TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
", "
CREATE INDEX chats_character ON chats(character_id, deleted_at, updated_at);
CREATE INDEX chats_updated ON chats(updated_at, id);
ALTER TABLE characters ADD COLUMN card_key INTEGER NOT NULL DEFAULT 0;
UPDATE characters SET card_key = random();
CREATE TRIGGER characters_card_key_insert AFTER INSERT ON characters BEGIN
  UPDATE characters SET card_key = random() WHERE id = new.id;
END;
CREATE TRIGGER characters_card_key_update AFTER UPDATE OF raw_json ON characters BEGIN
  UPDATE characters SET card_key = random() WHERE id = new.id;
END;
", "
ALTER TABLE characters DROP COLUMN sfw_locked;
ALTER TABLE characters DROP COLUMN gate_reasons;
"];

/// The migration (1-based) that indexed the chats of a character and by time, and gave every card
/// text a random `card_key` that changes with each write of `raw_json` (key of the library's cache
/// of what is worked out from a card, `characters::Derived`).
#[cfg(test)]
const LIBRARY_INDEX_ADDED: usize = 20;

/// The migration (1-based) that added how a chat looks (`chat_looks`) and the backdrop pictures
/// of characters (`character_backdrops`).
#[cfg(test)]
const LOOKS_ADDED: usize = 18;

/// The migration (1-based) that added reply variants of messages (`swipes`, `swipe_id`) and the
/// content hash of imported chats (`import_hash`).
#[cfg(test)]
const SWIPES_ADDED: usize = 17;

/// The migration (1-based) that added the user's own short text of a character (`tagline`).
#[cfg(test)]
const TAGLINE_ADDED: usize = 16;

/// The migration (1-based) that added engines and model profiles.
#[cfg(test)]
const MODEL_PROFILES_ADDED: usize = 12;

/// The migration (1-based) that added content hashes, source URLs and thumbnails of characters.
#[cfg(test)]
const IMPORT_ADDED: usize = 13;

/// The migration (1-based) that added the full text index of messages.
#[cfg(test)]
const SEARCH_ADDED: usize = 14;

/// The migration (1-based) that added favourites and the user's own tags of characters.
#[cfg(test)]
const TAGS_ADDED: usize = 15;

/// The migration (1-based) that turned the `user_name` setting into the first persona.
#[cfg(test)]
const PERSONAS_ADDED: usize = 8;

/// The migration (1-based) that bound personas to chats, characters and messages and gave them
/// their extra fields.
#[cfg(test)]
const PERSONA_LINKS_ADDED: usize = 9;

/// The migration (1-based) that added who wins when card and persona describe `{{user}}`
/// differently (`personas.card_priority`, `characters.user_priority`).
#[cfg(test)]
const USER_PRIORITY_ADDED: usize = 11;

/// The migration (1-based) that moved the activity log out into its own file (`logs.db`).
const LOGS_MOVED_OUT: usize = 7;

/// The database file inside the data folder.
pub const DB_FILE: &str = "rp-app.db";

/// The schema version this build brings the database to.
pub fn schema_target() -> usize {
    MIGRATIONS.len()
}

/// Brings the schema up to date and answers (version before, version after). Safe when two
/// connections open the database at the same moment (the activity log and the store): each step
/// reads the version inside its own write transaction.
pub fn migrate(conn: &mut Connection) -> Res<(usize, usize)> {
    let version = |c: &Connection| c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0));
    let before = version(conn)? as usize;
    loop {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let done = version(&tx)? as usize;
        let Some(sql) = MIGRATIONS.get(done) else { break };
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", done + 1))?;
        tx.commit()?;
    }
    Ok((before, version(conn)? as usize))
}

/// Recognizes the image formats a browser can show as a profile picture.
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// Crop position of a picture in percent (CSS object-position): clamped to 0..=100, 50 when the
/// value is not a number.
fn clamp_pos(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 100.0)
    } else {
        50.0
    }
}

pub struct Store {
    conn: Connection,
    dir: PathBuf,
    /// Schema version before and after this start's migrations.
    migrated: (usize, usize),
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Store {
    pub fn open(dir: &Path) -> Res<Store> {
        std::fs::create_dir_all(dir.join("cards"))?;
        std::fs::create_dir_all(dir.join("personas"))?;
        let mut store = Store { conn: Connection::open(dir.join(DB_FILE))?, dir: dir.to_path_buf(), migrated: (0, 0) };
        store.conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        store.migrated = migrate(&mut store.conn)?;
        let (from, to) = store.migrated;
        if from > 0 && from < LOGS_MOVED_OUT && to >= LOGS_MOVED_OUT {
            // Once: give the space of the old log table back to the disk. Only space: if it
            // fails, everything still works.
            if let Err(e) = store.conn.execute_batch("VACUUM;") {
                eprintln!("vacuum after moving the log out: {e}");
            }
        }
        store.cleanup()?;
        Ok(store)
    }

    /// A second, read-only connection to the database in `dir` (WAL lets it read while the app's
    /// own connection writes). For answers that only read and take long, like `GET /library`, so
    /// they never hold the hub's lock. No migrations, no cleanup: Err when the schema is not this
    /// build's (the caller then reads through the hub).
    pub fn reader(dir: &Path) -> Res<Store> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(dir.join(DB_FILE), flags)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version as usize != schema_target() {
            return Err("database schema is not this build's".into());
        }
        Ok(Store { conn, dir: dir.to_path_buf(), migrated: (0, 0) })
    }

    /// Schema version before and after the migrations of this start.
    pub fn migrated(&self) -> (usize, usize) {
        self.migrated
    }

    /// Leftovers of the last session: deleted cards, chats and rewound messages (their undo is
    /// gone) and replies cut off by a crash.
    fn cleanup(&mut self) -> Res<()> {
        let gone: Vec<i64> = self
            .conn
            .prepare("SELECT id FROM characters WHERE deleted_at IS NOT NULL")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for id in gone {
            let _ = std::fs::remove_file(self.png_path(id));
            let _ = std::fs::remove_file(self.avatar_path(id));
            let _ = std::fs::remove_file(self.thumb_path(id));
        }
        self.conn.execute("DELETE FROM characters WHERE deleted_at IS NOT NULL", [])?;
        self.conn.execute("DELETE FROM chats WHERE deleted_at IS NOT NULL", [])?;
        self.drop_unused_tags()?;
        self.conn.execute("DELETE FROM messages WHERE deleted_at IS NOT NULL OR (role = 'assistant' AND content = '')", [])?;
        self.fill_content_hashes()?;
        self.sweep_backdrops()?;
        Ok(())
    }
}
