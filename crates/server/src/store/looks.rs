//! How a chat looks (migration 18): the reading settings of the app (setting `chat_look`), what one
//! chat changes of them (`chat_looks.look`), and backdrop pictures of a chat and of a character
//! (default for its chats). The pictures are JPEG files in `<data>/backdrops/`, never in the database.

use std::path::PathBuf;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{clamp_pos, Res, Store};

/// Reading faces the web UI knows (font stacks of the device, nothing is downloaded).
pub const LOOK_FONTS: [&str; 7] = ["sitka", "georgia", "palatino", "cambria", "sans", "verdana", "mono"];

/// How messages are set: flowing like a book, the character's words in a bubble, both sides as
/// bubbles at their own edge of a centred column (theirs left, yours right), the same across the
/// whole width of the chat ("wide"), or tight without pictures.
pub const LOOK_STYLES: [&str; 5] = ["book", "bubbles", "sides", "wide", "compact"];

/// Named inks for spoken words and actions: the character's colour, the story's own ink, the
/// quiet ink of narration. Any other ink is a hue: `h0` … `h359` (the theme sets how light it is).
pub const LOOK_INKS: [&str; 3] = ["figure", "text", "soft"];

/// Named fills of a bubble: the character's colour, the theme's plain surface, or no bubble at
/// all. Any other fill is a hue, as for inks.
pub const LOOK_FILLS: [&str; 3] = ["figure", "plain", "none"];

/// Shapes of the small pictures at the messages.
pub const LOOK_AVATAR_SHAPES: [&str; 3] = ["round", "soft", "square"];

/// How a new message comes on screen.
pub const LOOK_ENTERS: [&str; 4] = ["none", "fade", "rise", "pop"];

/// What shows while the character has not written a word yet.
pub const LOOK_TYPING: [&str; 3] = ["dots", "name", "none"];

fn hue_ok(value: &str) -> bool {
    let hue = |h: &str| !h.is_empty() && h.len() <= 3 && h.bytes().all(|b| b.is_ascii_digit()) && h.parse::<u16>().is_ok_and(|h| h < 360);
    value.strip_prefix('h').is_some_and(hue)
}

fn ink_ok(ink: &str) -> bool {
    LOOK_INKS.contains(&ink) || hue_ok(ink)
}

fn fill_ok(fill: &str) -> bool {
    LOOK_FILLS.contains(&fill) || hue_ok(fill)
}

/// The setting that holds the app-wide reading settings (JSON of [`Look`]).
const LOOK_KEY: &str = "chat_look";

/// Reading settings. Every field is optional: app-wide, an empty field is the app's own default;
/// in a chat, an empty field follows the app-wide value.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Look {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Type size in px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Line height as a factor of the size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<f64>,
    /// Width of the text column in px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Justified text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justify: Option<bool>,
    /// Spoken words in the character's colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<bool>,
    /// New text of an answer fades in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animate: Option<bool>,
    /// The measurements under an answer (tokens per second, cache).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timings: Option<bool>,
    /// The ink of spoken words ("…"): one of [`LOOK_INKS`] or a hue `h0` … `h359`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub say: Option<String>,
    /// Spoken words in a heavier weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub say_bold: Option<bool>,
    /// The ink of actions (*…*), as `say`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<String>,
    /// Actions in italics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act_italic: Option<bool>,
    /// The inks of speech and actions in the user's own messages too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<bool>,
    /// The pictures of both sides at their messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatars: Option<bool>,
    /// Space between paragraphs, in lines of the type size (em).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub para: Option<f64>,
    /// The ink of plain narration (text outside quotes and asterisks), as `say`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story: Option<String>,
    /// The ink of thoughts (`…` in backticks), as `say`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub think: Option<String>,
    /// Widest a bubble may grow, in percent of its row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bubble: Option<f64>,
    /// Space between the messages and the chat's edge in the wide layout, px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<f64>,
    /// Space below each message, px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
    /// Roundness of the bubbles' corners, px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner: Option<f64>,
    /// The corner next to the speaker is pointed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<bool>,
    /// A soft shadow under the bubbles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    /// How much the bubbles cover what lies behind them, percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<f64>,
    /// How much a backdrop is blurred behind the bubbles, px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frost: Option<f64>,
    /// Fill of the character's bubbles: one of [`LOOK_FILLS`] or a hue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_fill: Option<String>,
    /// Fill of the user's own bubbles, as `ai_fill`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub me_fill: Option<String>,
    /// Size of the pictures at the messages, px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub av_size: Option<f64>,
    /// One of [`LOOK_AVATAR_SHAPES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub av_shape: Option<String>,
    /// Names above the messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<bool>,
    /// The time a message was written, beside the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times: Option<bool>,
    /// Both-sides layouts: yours left, theirs right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip: Option<bool>,
    /// One of [`LOOK_ENTERS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter: Option<String>,
    /// One of [`LOOK_TYPING`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typing: Option<String>,
}

impl Look {
    /// The settings with numbers inside their limits; Err `bad_look` for an unknown face, style,
    /// ink, fill or choice, or a value that is not a number.
    pub fn checked(mut self) -> Result<Look, &'static str> {
        if self.font.as_deref().is_some_and(|f| !LOOK_FONTS.contains(&f)) || self.style.as_deref().is_some_and(|s| !LOOK_STYLES.contains(&s)) {
            return Err("bad_look");
        }
        if [&self.say, &self.act, &self.story, &self.think].into_iter().flatten().any(|ink| !ink_ok(ink)) {
            return Err("bad_look");
        }
        if [&self.ai_fill, &self.me_fill].into_iter().flatten().any(|fill| !fill_ok(fill)) {
            return Err("bad_look");
        }
        let choice = |value: &Option<String>, all: &[&str]| value.as_deref().is_none_or(|v| all.contains(&v));
        if !choice(&self.av_shape, &LOOK_AVATAR_SHAPES) || !choice(&self.enter, &LOOK_ENTERS) || !choice(&self.typing, &LOOK_TYPING) {
            return Err("bad_look");
        }
        for (value, min, max) in [
            (&mut self.size, 14.0, 28.0),
            (&mut self.line, 1.3, 2.2),
            (&mut self.width, 480.0, 1800.0),
            (&mut self.para, 0.0, 2.0),
            (&mut self.bubble, 30.0, 100.0),
            (&mut self.edge, 0.0, 200.0),
            (&mut self.gap, 0.0, 80.0),
            (&mut self.corner, 0.0, 32.0),
            (&mut self.fill, 10.0, 100.0),
            (&mut self.frost, 0.0, 24.0),
            (&mut self.av_size, 16.0, 72.0),
        ] {
            if let Some(v) = value {
                if !v.is_finite() {
                    return Err("bad_look");
                }
                // Two decimals are plenty and keep the stored text short.
                *v = (v.clamp(min, max) * 100.0).round() / 100.0;
            }
        }
        Ok(self)
    }
}

/// Which backdrop a chat shows: the character's default, its own picture, or none.
pub const BACKDROP_MODES: [&str; 3] = ["character", "own", "none"];

/// How a backdrop sits behind the text: the visible part (percent, as CSS object-position), how
/// much the theme's ground covers it (percent) and how soft it is drawn (px).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub dim: f64,
    pub blur: f64,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame { x: 50.0, y: 50.0, dim: 60.0, blur: 0.0 }
    }
}

impl Frame {
    pub fn checked(self) -> Frame {
        let num = |v: f64, min: f64, max: f64, or: f64| if v.is_finite() { v.clamp(min, max).round() } else { or };
        Frame { x: clamp_pos(self.x).round(), y: clamp_pos(self.y).round(), dim: num(self.dim, 0.0, 90.0, 60.0), blur: num(self.blur, 0.0, 24.0, 0.0) }
    }
}

/// A stored backdrop: its revision (part of the URL, so devices may keep each version), frame, and
/// the ready-made picture it was copied from (`backdrop_presets`), if it was one.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Backdrop {
    pub rev: i64,
    #[serde(flatten)]
    pub frame: Frame,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// What the `bg_frame` column holds: the frame, plus the preset the picture came from.
#[derive(Serialize, Deserialize, Default)]
struct StoredFrame {
    #[serde(flatten)]
    frame: Frame,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preset: Option<String>,
}

/// Everything a chat needs to draw itself.
#[derive(Serialize, Debug)]
pub struct ChatLook {
    pub character_id: i64,
    /// App-wide reading settings.
    pub global: Look,
    /// What this chat changes of them.
    pub look: Look,
    pub backdrop: ChatBackdrop,
}

#[derive(Serialize, Debug)]
pub struct ChatBackdrop {
    /// One of [`BACKDROP_MODES`].
    pub mode: String,
    pub own: Option<Backdrop>,
    pub character: Option<Backdrop>,
}

/// Whose backdrop: a chat's own or a character's default.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BackdropOf {
    Chat(i64),
    Character(i64),
}

fn backdrop(rev: i64, frame: Option<String>) -> Option<Backdrop> {
    let stored: StoredFrame = serde_json::from_str(&frame?).unwrap_or_default();
    Some(Backdrop { rev, frame: stored.frame.checked(), preset: stored.preset })
}

impl Store {
    fn backdrop_dir(&self) -> PathBuf {
        self.dir.join("backdrops")
    }

    fn backdrop_path(&self, of: BackdropOf) -> PathBuf {
        self.backdrop_dir().join(match of {
            BackdropOf::Chat(id) => format!("chat-{id}.jpg"),
            BackdropOf::Character(id) => format!("char-{id}.jpg"),
        })
    }

    /// The owner exists (and is not in the bin).
    fn backdrop_owner(&self, of: BackdropOf) -> Res<bool> {
        let (sql, id) = match of {
            BackdropOf::Chat(id) => ("SELECT 1 FROM chats WHERE id = ?1 AND deleted_at IS NULL", id),
            BackdropOf::Character(id) => ("SELECT 1 FROM characters WHERE id = ?1 AND deleted_at IS NULL", id),
        };
        Ok(self.conn.query_row(sql, params![id], |_| Ok(())).optional()?.is_some())
    }

    /// The app-wide reading settings; nothing stored (or unreadable) = the app's defaults.
    pub fn look(&self) -> Res<Look> {
        let stored = self.setting(LOOK_KEY)?.and_then(|v| serde_json::from_str::<Look>(&v).ok());
        Ok(stored.and_then(|l| l.checked().ok()).unwrap_or_default())
    }

    pub fn set_look(&mut self, look: &Look) -> Res<()> {
        self.set_setting(LOOK_KEY, &serde_json::to_string(look)?)
    }

    fn stored_backdrop(&self, of: BackdropOf) -> Res<Option<Backdrop>> {
        let (sql, id) = match of {
            BackdropOf::Chat(id) => ("SELECT bg_rev, bg_frame FROM chat_looks WHERE chat_id = ?1", id),
            BackdropOf::Character(id) => ("SELECT bg_rev, bg_frame FROM character_backdrops WHERE character_id = ?1", id),
        };
        let row: Option<(i64, Option<String>)> = self.conn.query_row(sql, params![id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        Ok(row.and_then(|(rev, frame)| backdrop(rev, frame)))
    }

    /// How a chat looks; None = no such chat.
    pub fn chat_look(&self, chat_id: i64) -> Res<Option<ChatLook>> {
        let character_id: Option<i64> =
            self.conn.query_row("SELECT character_id FROM chats WHERE id = ?1 AND deleted_at IS NULL", params![chat_id], |r| r.get(0)).optional()?;
        let Some(character_id) = character_id else { return Ok(None) };
        let row: Option<(String, String)> =
            self.conn.query_row("SELECT look, backdrop FROM chat_looks WHERE chat_id = ?1", params![chat_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let (look, mode) = row.unwrap_or_else(|| ("{}".into(), BACKDROP_MODES[0].into()));
        let look = serde_json::from_str::<Look>(&look).ok().and_then(|l| l.checked().ok()).unwrap_or_default();
        let own = self.stored_backdrop(BackdropOf::Chat(chat_id))?;
        // A chat whose own picture is gone follows the character again.
        let mode = if !BACKDROP_MODES.contains(&mode.as_str()) || (mode == "own" && own.is_none()) { BACKDROP_MODES[0].to_string() } else { mode };
        Ok(Some(ChatLook {
            character_id,
            global: self.look()?,
            look,
            backdrop: ChatBackdrop { mode, own, character: self.stored_backdrop(BackdropOf::Character(character_id))? },
        }))
    }

    /// What this chat changes of the reading settings and/or which backdrop it shows. false = no
    /// such chat. Mode `own` without an own picture is refused (Err `no_backdrop`).
    pub fn set_chat_look(&mut self, chat_id: i64, look: Option<&Look>, mode: Option<&str>) -> Res<bool> {
        if !self.backdrop_owner(BackdropOf::Chat(chat_id))? {
            return Ok(false);
        }
        if mode == Some("own") && self.stored_backdrop(BackdropOf::Chat(chat_id))?.is_none() {
            return Err("no_backdrop".into());
        }
        self.conn.execute("INSERT OR IGNORE INTO chat_looks (chat_id) VALUES (?1)", params![chat_id])?;
        if let Some(look) = look {
            self.conn.execute("UPDATE chat_looks SET look = ?2 WHERE chat_id = ?1", params![chat_id, serde_json::to_string(look)?])?;
        }
        if let Some(mode) = mode {
            self.conn.execute("UPDATE chat_looks SET backdrop = ?2 WHERE chat_id = ?1", params![chat_id, mode])?;
        }
        Ok(true)
    }

    /// Stores (`Some`: a JPEG) or removes a backdrop. A chat with a new picture shows it; a chat
    /// that loses its own follows the character again. false = no such chat or character.
    pub fn set_backdrop(&mut self, of: BackdropOf, jpeg: Option<&[u8]>) -> Res<bool> {
        self.store_backdrop(of, jpeg, None)
    }

    /// Stores the JPEG copy of a ready-made picture as a backdrop, remembering which one it is
    /// (the id is checked by the caller). false = no such chat or character.
    pub fn set_preset_backdrop(&mut self, of: BackdropOf, jpeg: &[u8], preset: &str) -> Res<bool> {
        self.store_backdrop(of, Some(jpeg), Some(preset))
    }

    fn store_backdrop(&mut self, of: BackdropOf, jpeg: Option<&[u8]>, preset: Option<&str>) -> Res<bool> {
        if !self.backdrop_owner(of)? {
            return Ok(false);
        }
        let path = self.backdrop_path(of);
        match jpeg {
            Some(bytes) => {
                std::fs::create_dir_all(self.backdrop_dir())?;
                std::fs::write(&path, bytes)?;
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        // A new picture keeps the frame of the one before only in dim and blur: its crop starts in the middle.
        let frame = jpeg.map(|_| {
            let old = self.stored_backdrop(of).ok().flatten().map(|b| b.frame).unwrap_or_default();
            let stored = StoredFrame { frame: Frame { x: 50.0, y: 50.0, ..old }, preset: preset.map(str::to_string) };
            serde_json::to_string(&stored).unwrap_or_default()
        });
        match of {
            BackdropOf::Chat(id) => {
                self.conn.execute("INSERT OR IGNORE INTO chat_looks (chat_id) VALUES (?1)", params![id])?;
                self.conn.execute(
                    "UPDATE chat_looks SET bg_rev = bg_rev + 1, bg_frame = ?2, backdrop = ?3 WHERE chat_id = ?1",
                    params![id, frame, if jpeg.is_some() { "own" } else { "character" }],
                )?;
            }
            BackdropOf::Character(id) => {
                self.conn.execute("INSERT OR IGNORE INTO character_backdrops (character_id) VALUES (?1)", params![id])?;
                self.conn.execute("UPDATE character_backdrops SET bg_rev = bg_rev + 1, bg_frame = ?2 WHERE character_id = ?1", params![id, frame])?;
            }
        }
        Ok(true)
    }

    /// Moves, dims or softens a stored backdrop. false = there is none.
    pub fn set_backdrop_frame(&mut self, of: BackdropOf, frame: Frame) -> Res<bool> {
        // The picture stays the same, so does the preset it came from.
        let preset = self.stored_backdrop(of)?.and_then(|b| b.preset);
        let json = serde_json::to_string(&StoredFrame { frame: frame.checked(), preset })?;
        let (sql, id) = match of {
            BackdropOf::Chat(id) => ("UPDATE chat_looks SET bg_frame = ?2 WHERE chat_id = ?1 AND bg_frame IS NOT NULL", id),
            BackdropOf::Character(id) => ("UPDATE character_backdrops SET bg_frame = ?2 WHERE character_id = ?1 AND bg_frame IS NOT NULL", id),
        };
        Ok(self.conn.execute(sql, params![id, json])? > 0)
    }

    /// Makes a chat's own backdrop (picture and frame) the default of its character. false = the
    /// chat has no own picture.
    pub fn backdrop_to_character(&mut self, chat_id: i64) -> Res<bool> {
        let Some(view) = self.chat_look(chat_id)? else { return Ok(false) };
        let Some(own) = view.backdrop.own else { return Ok(false) };
        let bytes = std::fs::read(self.backdrop_path(BackdropOf::Chat(chat_id)))?;
        let to = BackdropOf::Character(view.character_id);
        self.store_backdrop(to, Some(&bytes), own.preset.as_deref())?;
        self.set_backdrop_frame(to, own.frame)?;
        Ok(true)
    }

    /// File and type of a stored backdrop, for the picture route.
    pub fn backdrop_file(&self, of: BackdropOf) -> Res<Option<(PathBuf, String)>> {
        Ok(self.stored_backdrop(of)?.map(|_| (self.backdrop_path(of), "image/jpeg".to_string())))
    }

    /// Removes backdrop files nothing points at any more (their chat or character was deleted).
    pub(super) fn sweep_backdrops(&self) -> Res<()> {
        let Ok(dir) = std::fs::read_dir(self.backdrop_dir()) else { return Ok(()) };
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let of = name.strip_suffix(".jpg").and_then(|stem| {
                let (kind, id) = stem.split_once('-')?;
                let id: i64 = id.parse().ok()?;
                match kind {
                    "chat" => Some(BackdropOf::Chat(id)),
                    "char" => Some(BackdropOf::Character(id)),
                    _ => None,
                }
            });
            // Only files this module wrote are ever removed.
            if let Some(of) = of {
                if self.stored_backdrop(of)?.is_none() {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{sample, temp_store};
    use super::*;

    fn chat(s: &mut Store) -> (i64, i64) {
        let card = sample(r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"Mara","description":"A baker.","first_mes":"Hello."}}"#);
        let character = s.add_character(&card, None).unwrap().id;
        (character, s.create_chat(character, None).unwrap())
    }

    #[test]
    fn looks_are_checked_and_a_chat_only_keeps_what_it_changes() {
        let (mut s, dir) = temp_store();
        let (character, chat) = chat(&mut s);
        assert_eq!(s.look().unwrap(), Look::default());
        let global = Look { font: Some("georgia".into()), size: Some(99.0), ..Look::default() }.checked().unwrap();
        assert_eq!(global.size, Some(28.0));
        s.set_look(&global).unwrap();
        assert!(Look { font: Some("comic".into()), ..Look::default() }.checked().is_err());
        assert!(Look { line: Some(f64::NAN), ..Look::default() }.checked().is_err());

        let own = Look { line: Some(1.9), animate: Some(false), ..Look::default() };
        assert!(s.set_chat_look(chat, Some(&own), None).unwrap());
        assert!(!s.set_chat_look(chat + 100, Some(&own), None).unwrap());
        let view = s.chat_look(chat).unwrap().unwrap();
        assert_eq!((view.character_id, view.global, view.look), (character, global, own));
        assert_eq!((view.backdrop.mode.as_str(), view.backdrop.own, view.backdrop.character), ("character", None, None));
        assert!(s.chat_look(chat + 100).unwrap().is_none());
        // Inks are a name or a hue; the new way to set messages is known.
        let inks = |say: &str, act: &str| Look { say: Some(say.into()), act: Some(act.into()), ..Look::default() }.checked();
        assert!(inks("figure", "soft").is_ok() && inks("h0", "h359").is_ok() && inks("text", "h210").is_ok());
        for bad in ["h360", "h-1", "h+12", "h", "h1000", "red", "#ff0000", ""] {
            assert!(inks(bad, "soft").is_err() && inks("figure", bad).is_err(), "{bad}");
        }
        let sides = Look { style: Some("sides".into()), para: Some(9.0), mine: Some(true), ..Look::default() }.checked().unwrap();
        assert_eq!((sides.style.as_deref(), sides.para), (Some("sides"), Some(2.0)));
        assert!(Look { para: Some(f64::INFINITY), ..Look::default() }.checked().is_err());
        // The stored text holds only the changed fields.
        let raw: String = s.conn.query_row("SELECT look FROM chat_looks WHERE chat_id = ?1", params![chat], |r| r.get(0)).unwrap();
        assert_eq!(raw, r#"{"line":1.9,"animate":false}"#);
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_wider_choice_is_checked_clamped_and_kept_per_chat() {
        let (mut s, dir) = temp_store();
        let (_, chat) = chat(&mut s);
        let wide = Look {
            style: Some("wide".into()),
            bubble: Some(140.0),
            edge: Some(-5.0),
            gap: Some(12.345),
            corner: Some(40.0),
            tail: Some(false),
            shadow: Some(true),
            fill: Some(3.0),
            frost: Some(8.0),
            ai_fill: Some("h215".into()),
            me_fill: Some("none".into()),
            story: Some("soft".into()),
            think: Some("h50".into()),
            av_size: Some(90.0),
            av_shape: Some("square".into()),
            names: Some(false),
            times: Some(true),
            flip: Some(true),
            enter: Some("rise".into()),
            typing: Some("name".into()),
            ..Look::default()
        }
        .checked()
        .unwrap();
        assert_eq!((wide.bubble, wide.edge, wide.gap, wide.corner, wide.fill, wide.av_size), (Some(100.0), Some(0.0), Some(12.35), Some(32.0), Some(10.0), Some(72.0)));
        let bad = |l: Look| l.checked().is_err();
        assert!(bad(Look { ai_fill: Some("text".into()), ..Look::default() }), "text is an ink, not a fill");
        assert!(bad(Look { me_fill: Some("h400".into()), ..Look::default() }));
        assert!(bad(Look { story: Some("plain".into()), ..Look::default() }));
        assert!(bad(Look { av_shape: Some("star".into()), ..Look::default() }));
        assert!(bad(Look { enter: Some("spin".into()), ..Look::default() }));
        assert!(bad(Look { typing: Some("".into()), ..Look::default() }));
        assert!(bad(Look { frost: Some(f64::NAN), ..Look::default() }));
        assert!(bad(Look { style: Some("grid".into()), ..Look::default() }));

        // Stored for the app and for one chat; both come back after a restart.
        s.set_look(&wide).unwrap();
        let own = Look { style: Some("book".into()), times: Some(false), gap: Some(40.0), ..Look::default() };
        assert!(s.set_chat_look(chat, Some(&own), None).unwrap());
        drop(s);
        let s = Store::open(&dir).unwrap();
        let view = s.chat_look(chat).unwrap().unwrap();
        assert_eq!((view.global, view.look), (wide, own));
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn backdrops_live_as_files_and_go_with_their_owner() {
        let (mut s, dir) = temp_store();
        let (character, chat) = chat(&mut s);
        let of = BackdropOf::Chat(chat);
        assert!(s.set_chat_look(chat, None, Some("own")).is_err(), "no own picture yet");
        assert!(s.set_backdrop(of, Some(b"jpeg one")).unwrap());
        assert!(!s.set_backdrop(BackdropOf::Chat(chat + 100), Some(b"x")).unwrap());
        let file = dir.join("backdrops").join(format!("chat-{chat}.jpg"));
        assert_eq!(std::fs::read(&file).unwrap(), b"jpeg one");
        let view = s.chat_look(chat).unwrap().unwrap();
        assert_eq!(view.backdrop.mode, "own");
        assert_eq!(view.backdrop.own, Some(Backdrop { rev: 1, frame: Frame::default(), preset: None }));

        // The frame is clamped; a new picture keeps dim and blur, and counts the revision on.
        assert!(s.set_backdrop_frame(of, Frame { x: 120.0, y: 30.4, dim: 99.0, blur: 6.0 }).unwrap());
        assert!(!s.set_backdrop_frame(BackdropOf::Character(character), Frame::default()).unwrap());
        s.set_backdrop(of, Some(b"jpeg two")).unwrap();
        let own = s.chat_look(chat).unwrap().unwrap().backdrop.own.unwrap();
        assert_eq!(own, Backdrop { rev: 2, frame: Frame { x: 50.0, y: 50.0, dim: 90.0, blur: 6.0 }, preset: None });

        // The same picture as the character's default; the chat can still show none.
        assert!(s.backdrop_to_character(chat).unwrap());
        assert!(s.set_chat_look(chat, None, Some("none")).unwrap());
        let view = s.chat_look(chat).unwrap().unwrap();
        assert_eq!((view.backdrop.mode.as_str(), view.backdrop.character.map(|b| b.frame.blur)), ("none", Some(6.0)));
        let (path, mime) = s.backdrop_file(BackdropOf::Character(character)).unwrap().unwrap();
        assert_eq!((std::fs::read(path).unwrap(), mime.as_str()), (b"jpeg two".to_vec(), "image/jpeg"));

        // Removing the own picture: the file goes, the chat follows the character again.
        assert!(s.set_backdrop(of, None).unwrap());
        assert!(!file.exists());
        let view = s.chat_look(chat).unwrap().unwrap();
        assert_eq!((view.backdrop.mode.as_str(), view.backdrop.own), ("character", None));
        assert!(s.backdrop_file(of).unwrap().is_none());

        // A deleted character takes its rows along; the next start removes the files left behind.
        s.set_backdrop(of, Some(b"jpeg three")).unwrap();
        let stray = dir.join("backdrops").join("notes.txt");
        std::fs::write(&stray, b"not ours").unwrap();
        s.conn.execute("DELETE FROM characters WHERE id = ?1", params![character]).unwrap();
        s.sweep_backdrops().unwrap();
        assert!(!file.exists() && !dir.join("backdrops").join(format!("char-{character}.jpg")).exists());
        assert!(stray.exists(), "only backdrop files are swept");
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_preset_backdrop_remembers_its_preset_across_a_restart_until_another_picture_comes() {
        let (mut s, dir) = temp_store();
        let (character, chat) = chat(&mut s);
        let of = BackdropOf::Chat(chat);
        assert!(s.set_preset_backdrop(of, b"tavern jpeg", "taverne").unwrap());
        assert!(!s.set_preset_backdrop(BackdropOf::Character(character + 100), b"x", "wald").unwrap());
        // Moving or dimming it keeps the preset; making it the character's default carries it along.
        assert!(s.set_backdrop_frame(of, Frame { x: 30.0, y: 50.0, dim: 40.0, blur: 2.0 }).unwrap());
        assert!(s.backdrop_to_character(chat).unwrap());
        assert!(s.set_preset_backdrop(BackdropOf::Character(character), b"forest jpeg", "wald").unwrap());
        drop(s);

        // A new start reads the same from the database and the files.
        let mut s = Store::open(&dir).unwrap();
        let view = s.chat_look(chat).unwrap().unwrap();
        assert_eq!(view.backdrop.mode, "own");
        assert_eq!(view.backdrop.own, Some(Backdrop { rev: 1, frame: Frame { x: 30.0, y: 50.0, dim: 40.0, blur: 2.0 }, preset: Some("taverne".into()) }));
        let theirs = view.backdrop.character.clone().unwrap();
        assert_eq!((theirs.rev, theirs.preset.as_deref(), theirs.frame.dim), (2, Some("wald"), 40.0));
        assert_eq!(std::fs::read(dir.join("backdrops").join(format!("char-{character}.jpg"))).unwrap(), b"forest jpeg");
        // The JSON the devices get names it.
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["backdrop"]["own"]["preset"], "taverne");

        // An uploaded picture is no preset any more; no picture, no preset.
        s.set_backdrop(of, Some(b"own jpeg")).unwrap();
        assert_eq!(s.chat_look(chat).unwrap().unwrap().backdrop.own.unwrap().preset, None);
        assert!(serde_json::to_value(s.chat_look(chat).unwrap().unwrap()).unwrap()["backdrop"]["own"].get("preset").is_none());
        s.set_backdrop(BackdropOf::Character(character), None).unwrap();
        assert!(s.chat_look(chat).unwrap().unwrap().backdrop.character.is_none());
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
