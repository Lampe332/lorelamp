//! Tests of the store: migrations, characters, chats, messages, personas and settings.

use std::sync::atomic::{AtomicU32, Ordering};

use rp_core::card::{self, Imported};
use rp_core::prompt::Priority;
use rp_core::user_check::Fit;
use rusqlite::params;

use super::stats::Stats;
use super::*;

static N: AtomicU32 = AtomicU32::new(0);

pub fn temp_store() -> (Store, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "rp-app-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    (Store::open(&dir).unwrap(), dir)
}

pub fn sample(json: &str) -> Imported {
    card::parse(json.as_bytes()).unwrap()
}

/// A database from before the log moved out: the log table goes, its space goes back to the
/// disk, and every character, chat and message stays.
#[test]
fn the_old_log_table_goes_and_the_data_stays() {
    let dir = std::env::temp_dir().join(format!("rp-app-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cards")).unwrap();
    {
        let conn = Connection::open(dir.join(DB_FILE)).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
        for sql in &MIGRATIONS[..LOGS_MOVED_OUT - 1] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(&format!("PRAGMA user_version = {}", LOGS_MOVED_OUT - 1)).unwrap();
        let mut s = Store { conn, dir: dir.clone(), migrated: (0, 0) };
        let id = old_character(&s, "Mara");
        old_chat(&s, id, &["Hi.", "Hallo"]);
        let filler = "x".repeat(4_000);
        let tx = s.conn.transaction().unwrap();
        for i in 0..2_000 {
            tx.execute("INSERT INTO logs (ts, level, source, category, message, data) VALUES (?1, 'info', 'ui', 'click', 'k', ?2)", params![i, filler])
                .unwrap();
        }
        tx.commit().unwrap();
        s.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    }
    let before = std::fs::metadata(dir.join(DB_FILE)).unwrap().len();
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (LOGS_MOVED_OUT - 1, schema_target()));
    let logs: i64 = s.conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'logs'", [], |r| r.get(0)).unwrap();
    assert_eq!(logs, 0);
    let chars = s.characters().unwrap();
    assert_eq!(chars.len(), 1);
    let chat = s.chats_of(chars[0].id).unwrap()[0].id;
    assert_eq!(s.messages(chat).unwrap().iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["Hi.", "Hallo"]);
    drop(s);
    let after = std::fs::metadata(dir.join(DB_FILE)).unwrap().len();
    assert!(after < before / 4, "{before} -> {after} bytes");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn character_roundtrip() {
    let (mut s, dir) = temp_store();
    let mara = s
        .add_character(&sample(r#"{"name":"Mara","description":"Mara is 34 years old.","creator_notes":"Keeper of {{char}}'s light\nline two","tags":["Sea"]}"#), None)
        .unwrap();
    assert_eq!(mara.blurb, "Keeper of Mara's light");
    let second = s.add_character(&sample(r#"{"name":"Mia","description":"A quiet traveler."}"#), Some(b"png-bytes")).unwrap();
    assert_eq!(std::fs::read(s.avatar(second.id).unwrap().unwrap().0).unwrap(), b"png-bytes");

    let names: Vec<_> = s.characters().unwrap().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["Mara", "Mia"]);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn chats_messages_and_recent() {
    let (mut s, dir) = temp_store();
    let a = s.add_character(&sample(r#"{"name":"A"}"#), None).unwrap().id;
    let b = s.add_character(&sample(r#"{"name":"B"}"#), None).unwrap().id;
    assert_eq!(s.latest_chat(a).unwrap(), None);
    let chat_a = s.create_chat(a, Some("Hello.")).unwrap();
    let chat_b = s.create_chat(b, None).unwrap();
    assert_eq!(s.latest_chat(a).unwrap(), Some(chat_a));
    assert_eq!(s.chat_character(chat_b).unwrap(), Some(b));
    assert_eq!(s.chat_character(999).unwrap(), None);

    let reply = s.add_message(chat_a, "assistant", "").unwrap();
    let t = Timings { tok_s: 90.5, processed: 12, cached: 300, predicted: 40 };
    s.finish_message(reply.id, "Done.", Some(&t)).unwrap();
    let msgs = s.messages(chat_a).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!((msgs[0].content.as_str(), msgs[1].content.as_str()), ("Hello.", "Done."));
    assert_eq!(msgs[1].timings, Some(t));
    assert_eq!(s.character(a).unwrap().unwrap().info.chats, 1);
    let recent = s.recent(8).unwrap();
    assert_eq!(recent.len(), 2);
    let of_a = recent.iter().find(|r| r.chat.id == chat_a).unwrap(); // order within one second is by id
    assert_eq!((of_a.chat.messages, of_a.chat.last.as_str(), of_a.name.as_str()), (2, "Done.", "A"));

    // Stats: messages of live chats only, and the newest measured answer.
    s.add_message(chat_b, "user", "Hi.").unwrap();
    assert_eq!(s.stats().unwrap(), Stats { messages: 3, last_timings: msgs[1].timings.clone() });
    assert!(s.stats().unwrap().last_timings.is_some());
    s.set_chat_deleted(chat_b, true).unwrap();
    assert_eq!(s.stats().unwrap().messages, 2);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Deleted chats and characters wait for the next start (undo), but from the moment they are
/// deleted their messages count no more, neither in the number nor in the newest measurements;
/// an undo brings both back.
#[test]
fn stats_leave_out_deleted_chats_and_characters_at_once() {
    let (mut s, dir) = temp_store();
    let a = s.add_character(&sample(r#"{"name":"A"}"#), None).unwrap().id;
    let b = s.add_character(&sample(r#"{"name":"B"}"#), None).unwrap().id;
    let measured = |s: &mut Store, chat: i64, tok_s: f64| {
        let m = s.add_message(chat, "assistant", "").unwrap();
        let t = Timings { tok_s, processed: 1, cached: 2, predicted: 3 };
        s.finish_message(m.id, "Reply.", Some(&t)).unwrap();
        t
    };
    let chat_a = s.create_chat(a, Some("Hi.")).unwrap();
    let older = measured(&mut s, chat_a, 50.0);
    let chat_b = s.create_chat(b, Some("Hallo.")).unwrap();
    let newer = measured(&mut s, chat_b, 90.0);
    assert_eq!(s.stats().unwrap(), Stats { messages: 4, last_timings: Some(newer.clone()) });

    s.set_chat_deleted(chat_b, true).unwrap();
    assert_eq!(s.stats().unwrap(), Stats { messages: 2, last_timings: Some(older.clone()) }, "deleted chat");
    s.set_chat_deleted(chat_b, false).unwrap();
    assert_eq!(s.stats().unwrap(), Stats { messages: 4, last_timings: Some(newer.clone()) }, "undo");

    s.set_deleted(b, true).unwrap();
    assert_eq!(s.stats().unwrap(), Stats { messages: 2, last_timings: Some(older) }, "deleted character");
    s.set_deleted(a, true).unwrap();
    assert_eq!(s.stats().unwrap(), Stats { messages: 0, last_timings: None }, "nothing left");
    s.set_deleted(b, false).unwrap();
    assert_eq!(s.stats().unwrap(), Stats { messages: 2, last_timings: Some(newer) }, "undo");

    // A rewound measured answer leaves the figures too.
    let reply = s.messages(chat_b).unwrap()[1].id;
    s.rewind(chat_b, reply).unwrap();
    assert_eq!(s.stats().unwrap(), Stats { messages: 1, last_timings: None });
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn delete_undo_and_purge_on_next_start() {
    let (mut s, dir) = temp_store();
    let keep = s.add_character(&sample(r#"{"name":"Keep"}"#), None).unwrap().id;
    let gone = s.add_character(&sample(r#"{"name":"Gone"}"#), Some(b"x")).unwrap().id;
    let chat = s.create_chat(gone, Some("Hi")).unwrap();
    let crashed = s.create_chat(keep, None).unwrap();
    s.add_message(crashed, "assistant", "").unwrap(); // a reply that never finished

    assert!(s.set_deleted(keep, true).unwrap());
    assert!(s.character(keep).unwrap().is_none());
    assert!(s.set_deleted(keep, false).unwrap()); // undo
    assert!(s.character(keep).unwrap().is_some());

    s.set_deleted(gone, true).unwrap();
    assert_eq!(s.chat_character(chat).unwrap(), None);
    assert!(s.recent(8).unwrap().iter().all(|r| r.chat.character_id != gone));
    let png = s.avatar(gone).unwrap().unwrap().0;
    drop(s);

    let s = Store::open(&dir).unwrap(); // next start
    assert!(!png.exists());
    assert!(s.messages(chat).unwrap().is_empty(), "chats of a purged card are gone");
    assert!(s.messages(crashed).unwrap().is_empty(), "empty reply removed");
    assert_eq!(s.characters().unwrap().len(), 1);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn editing_keeps_other_fields() {
    let (mut s, dir) = temp_store();
    let id = s
        .add_character(&sample(r#"{"spec":"chara_card_v2","data":{"name":"Mia","description":"A quiet traveler.","extensions":{"x":1}}}"#), None)
        .unwrap()
        .id;
    let mut card = s.character(id).unwrap().unwrap().card;
    card.name = "Mia Renamed".into();
    card.description = "A more confident traveler.".into();
    card.tags = vec!["Drama".into()];
    let saved = s.update_character(id, &card).unwrap().unwrap();
    assert_eq!(saved.card, card);
    assert_eq!(saved.info.name, "Mia Renamed");
    assert_eq!(saved.card.description, "A more confident traveler.");
    assert!(s.update_character(999, &card).unwrap().is_none());
    drop(s);

    let s = Store::open(&dir).unwrap();
    assert_eq!(s.character(id).unwrap().unwrap().card.description, "A more confident traveler.");
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn avatar_choice_replaces_and_restores_the_card_picture() {
    let (mut s, dir) = temp_store();
    let with_png = s.add_character(&sample(r#"{"name":"A"}"#), Some(b"card-png")).unwrap();
    let plain = s.add_character(&sample(r#"{"name":"B"}"#), None).unwrap();
    assert!(with_png.has_avatar && !with_png.custom_avatar);
    assert!(!plain.has_avatar);
    assert!(s.avatar(plain.id).unwrap().is_none());

    assert!(s.set_avatar(plain.id, Some((b"jpeg-bytes", "image/jpeg"))).unwrap());
    let info = s.character(plain.id).unwrap().unwrap().info;
    assert!(info.has_avatar && info.custom_avatar);
    assert_eq!(info.avatar_rev, 1);
    let (path, mime) = s.avatar(plain.id).unwrap().unwrap();
    assert_eq!((std::fs::read(path).unwrap().as_slice(), mime.as_str()), (&b"jpeg-bytes"[..], "image/jpeg"));

    s.set_avatar(with_png.id, Some((b"new", "image/webp"))).unwrap();
    s.set_avatar(with_png.id, None).unwrap(); // back to the card's own picture
    let info = s.character(with_png.id).unwrap().unwrap().info;
    assert!(info.has_avatar && !info.custom_avatar);
    assert_eq!(info.avatar_rev, 2);
    assert_eq!(s.avatar(with_png.id).unwrap().unwrap().1, "image/png");
    assert!(!s.set_avatar(999, None).unwrap());
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn chat_overview_delete_and_undo() {
    let (mut s, dir) = temp_store();
    let a = s.add_character(&sample(r#"{"name":"A","first_mes":"Hello there."}"#), None).unwrap().id;
    let first = s.create_chat(a, Some("Hello there.")).unwrap();
    let second = s.create_chat(a, Some("Hello there.")).unwrap();
    s.add_message(first, "user", "Hi").unwrap();

    // Newest first; within the same second the newer chat (higher id) leads.
    let list = s.chats_of(a).unwrap();
    assert_eq!(list.iter().map(|c| (c.id, c.messages)).collect::<Vec<_>>(), [(second, 1), (first, 2)]);
    // The overview quotes the character, not the user's own last line.
    assert_eq!(list[1].last, "Hello there.");
    assert!(!list[1].custom_card);
    assert_eq!(s.character(a).unwrap().unwrap().info.chats, 2);

    assert_eq!(list[0].title, "", "new chats are unnamed");
    assert!(s.set_chat_title(first, "Romanze").unwrap());
    assert_eq!(s.chat_info(first).unwrap().unwrap().title, "Romanze");
    assert_eq!(s.recent(8).unwrap().iter().find(|r| r.chat.id == first).unwrap().name, "A");
    assert!(!s.set_chat_title(999, "x").unwrap());

    assert!(s.set_chat_deleted(first, true).unwrap());
    assert_eq!(s.chats_of(a).unwrap().len(), 1);
    assert_eq!(s.latest_chat(a).unwrap(), Some(second));
    assert_eq!(s.chat_character(first).unwrap(), None, "a deleted chat cannot be opened");
    assert_eq!(s.character(a).unwrap().unwrap().info.chats, 1);
    assert!(s.set_chat_deleted(first, false).unwrap()); // undo
    assert_eq!(s.chats_of(a).unwrap().len(), 2);

    s.set_chat_deleted(second, true).unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap(); // next start purges it
    assert!(s.messages(second).unwrap().is_empty());
    assert_eq!(s.messages(first).unwrap().len(), 2);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_chat_can_play_its_own_version_of_the_card() {
    let (mut s, dir) = temp_store();
    let a = s.add_character(&sample(r#"{"name":"Mara","description":"Mara is 34 years old."}"#), None).unwrap().id;
    let plain = s.create_chat(a, None).unwrap();
    let own = s.create_chat(a, None).unwrap();

    let (c, custom) = s.chat_character_card(plain).unwrap().unwrap();
    assert!(!custom);
    assert_eq!(c.card.description, "Mara is 34 years old.");

    let mut card = c.card.clone();
    card.scenario = "A different story.".into();
    assert!(s.set_chat_card(own, Some(&card)).unwrap());
    let (c, custom) = s.chat_character_card(own).unwrap().unwrap();
    assert!(custom && c.card.scenario == "A different story.");
    assert!(s.chat_info(own).unwrap().unwrap().custom_card);
    assert_eq!(s.chat_character_card(plain).unwrap().unwrap().0.card.scenario, "", "other chats keep the character's card");
    assert_eq!(s.character(a).unwrap().unwrap().card.scenario, "", "and so does the character");

    assert!(s.set_chat_card(own, None).unwrap());
    assert!(!s.chat_character_card(own).unwrap().unwrap().1);
    assert!(!s.set_chat_card(999, None).unwrap());

    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn edit_rewind_undo_and_fork() {
    let (mut s, dir) = temp_store();
    let a = s.add_character(&sample(r#"{"name":"A"}"#), None).unwrap().id;
    let chat = s.create_chat(a, Some("Hello.")).unwrap();
    let mut card = s.character(a).unwrap().unwrap().card;
    card.scenario = "Own story.".into();
    s.set_chat_card(chat, Some(&card)).unwrap();
    let ids: Vec<i64> = ["one", "two", "three"]
        .iter()
        .enumerate()
        .map(|(i, text)| s.add_message(chat, if i % 2 == 0 { "user" } else { "assistant" }, text).unwrap().id)
        .collect();
    let texts = |s: &Store, chat: i64| s.messages(chat).unwrap().into_iter().map(|m| m.content).collect::<Vec<_>>();

    assert_eq!(s.message_chat(ids[0]).unwrap(), Some(chat));
    assert_eq!(s.message_chat(999).unwrap(), None);
    s.edit_message(ids[0], "one, edited").unwrap();
    assert_eq!(texts(&s, chat), ["Hello.", "one, edited", "two", "three"]);

    // A fork copies the chat up to one message, with its own card; the original is untouched.
    let fork = s.fork_chat(chat, ids[1], "Other way").unwrap();
    assert_eq!(texts(&s, fork), ["Hello.", "one, edited", "two"]);
    assert_eq!(s.chat_info(fork).unwrap().unwrap().title, "Other way");
    assert_eq!(s.chat_character_card(fork).unwrap().unwrap().0.card.scenario, "Own story.");
    s.add_message(fork, "user", "elsewhere").unwrap();
    assert_eq!(texts(&s, chat).len(), 4);

    // Rewind removes a message and all after it, in lists and counts too; undo brings them back.
    assert_eq!(s.rewind(chat, ids[1]).unwrap(), 2);
    assert_eq!(texts(&s, chat), ["Hello.", "one, edited"]);
    assert_eq!(s.message_chat(ids[2]).unwrap(), None);
    let info = s.chat_info(chat).unwrap().unwrap();
    assert_eq!((info.messages, info.last.as_str()), (2, "Hello."));
    assert_eq!(s.undo_rewind(chat).unwrap(), 2);
    assert_eq!(texts(&s, chat), ["Hello.", "one, edited", "two", "three"]);

    // Once the story goes on, the removed messages are gone for good.
    s.rewind(chat, ids[2]).unwrap();
    s.add_message(chat, "assistant", "new three").unwrap();
    assert_eq!(s.undo_rewind(chat).unwrap(), 0);
    assert_eq!(texts(&s, chat), ["Hello.", "one, edited", "two", "new three"]);

    // And so are those of a rewind that is still open at the next start.
    s.rewind(chat, ids[0]).unwrap();
    drop(s);
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(s.undo_rewind(chat).unwrap(), 0);
    assert_eq!(texts(&s, chat), ["Hello."]);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn avatar_position_is_clamped() {
    let (mut s, dir) = temp_store();
    let a = s.add_character(&sample(r#"{"name":"A"}"#), None).unwrap();
    assert_eq!(a.avatar_pos, [50.0, 12.0]);
    assert!(s.set_avatar_pos(a.id, 130.0, -4.0).unwrap());
    assert_eq!(s.character(a.id).unwrap().unwrap().info.avatar_pos, [100.0, 0.0]);
    assert!(s.set_avatar_pos(a.id, f64::NAN, 30.5).unwrap());
    assert_eq!(s.character(a.id).unwrap().unwrap().info.avatar_pos, [50.0, 30.5]);
    assert!(!s.set_avatar_pos(999, 1.0, 1.0).unwrap());
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn image_formats() {
    assert_eq!(image_mime(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
    assert_eq!(image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
    assert_eq!(image_mime(b"GIF89a"), Some("image/gif"));
    assert_eq!(image_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    assert_eq!(image_mime(b"<svg onload=alert(1)>"), None);
    assert_eq!(image_mime(b""), None);
}

/// A card stored the way any older schema can hold it (`add_character` needs the newest one).
fn old_character(s: &Store, name: &str) -> i64 {
    let raw = serde_json::json!({ "name": name, "first_mes": "Hi." }).to_string();
    s.conn
        .execute("INSERT INTO characters (name, raw_json, has_png, created_at) VALUES (?1, ?2, 0, 1)", params![name, raw])
        .unwrap();
    s.conn.last_insert_rowid()
}

/// A chat with messages (alternating, the character first) stored the way any older schema
/// can hold it.
fn old_chat(s: &Store, character_id: i64, texts: &[&str]) -> i64 {
    s.conn.execute("INSERT INTO chats (character_id, created_at, updated_at) VALUES (?1, 1, 1)", params![character_id]).unwrap();
    let chat = s.conn.last_insert_rowid();
    for (i, text) in texts.iter().enumerate() {
        let role = if i % 2 == 0 { "assistant" } else { "user" };
        s.conn.execute("INSERT INTO messages (chat_id, role, content, created_at) VALUES (?1, ?2, ?3, 1)", params![chat, role, text]).unwrap();
    }
    chat
}

/// A database that stopped at schema `version`, after `fill` put data into it.
fn old_db(version: usize, fill: impl FnOnce(&mut Store)) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rp-app-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cards")).unwrap();
    let conn = Connection::open(dir.join(DB_FILE)).unwrap();
    for sql in &MIGRATIONS[..version] {
        conn.execute_batch(sql).unwrap();
    }
    conn.execute_batch(&format!("PRAGMA user_version = {version}")).unwrap();
    let mut s = Store { conn, dir: dir.clone(), migrated: (0, 0) };
    fill(&mut s);
    dir
}

/// The old `user_name` becomes the one active persona; cards, chats and messages stay.
#[test]
fn migration_turns_the_user_name_into_the_first_persona() {
    let mut ids = (0, 0);
    let dir = old_db(PERSONAS_ADDED - 1, |s| {
        s.set_setting("user_name", "Lukas").unwrap();
        let c = old_character(s, "Mara");
        let chat = old_chat(s, c, &["Hi.", "Hallo"]);
        ids = (c, chat);
    });
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (PERSONAS_ADDED - 1, schema_target()));
    let all = s.personas().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].name.as_str(), all[0].description.as_str(), all[0].has_avatar), ("Lukas", "", false));
    assert_eq!(s.active_persona_id().unwrap(), all[0].id);
    assert_eq!(s.user_name().unwrap(), "Lukas");
    assert_eq!(s.setting("user_name").unwrap().as_deref(), Some("Lukas"), "the old setting is left alone");
    let chars = s.characters().unwrap();
    assert_eq!((chars.len(), chars[0].id, chars[0].name.as_str()), (1, ids.0, "Mara"));
    assert_eq!(s.chats_of(ids.0).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [ids.1]);
    assert_eq!(s.messages(ids.1).unwrap().iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["Hi.", "Hallo"]);
    drop(s);
    let s = Store::open(&dir).unwrap(); // runs only once
    assert_eq!(s.personas().unwrap().len(), 1);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();

    // Never named: the first persona is "User".
    let dir = old_db(PERSONAS_ADDED - 1, |_| {});
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.personas().unwrap().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["User"]);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Persona links and fields arrive empty: personas, cards, chats and messages stay as they were,
/// and every chat keeps following the active persona.
#[test]
fn migration_adds_persona_links_and_keeps_the_data() {
    let mut ids = (0, 0, 0);
    let dir = old_db(PERSONA_LINKS_ADDED - 1, |s| {
        s.conn.execute("UPDATE personas SET name = 'Lukas', description = 'Kapitän.'", []).unwrap();
        s.conn.execute("INSERT INTO personas (name, description, created_at, updated_at) VALUES ('Bea', '', 1, 1)", []).unwrap();
        let c = old_character(s, "Mara");
        let chat = s.conn.execute("INSERT INTO chats (character_id, created_at, updated_at, title) VALUES (?1, 1, 1, 'Alt')", params![c]).map(|_| s.conn.last_insert_rowid()).unwrap();
        for (role, text) in [("assistant", "Hi."), ("user", "Hallo")] {
            s.conn.execute("INSERT INTO messages (chat_id, role, content, created_at) VALUES (?1, ?2, ?3, 1)", params![chat, role, text]).unwrap();
        }
        ids = (c, chat, s.conn.query_row("SELECT MIN(id) FROM personas", [], |r| r.get(0)).unwrap());
    });
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (PERSONA_LINKS_ADDED - 1, schema_target()));
    let lukas = s.persona(ids.2).unwrap().unwrap();
    assert_eq!((lukas.name.as_str(), lukas.description.as_str()), ("Lukas", "Kapitän."));
    assert_eq!([lukas.pronouns, lukas.appearance, lukas.personality, lukas.background], ["", "", "", ""].map(String::from));
    assert_eq!(s.personas().unwrap().len(), 2);
    assert_eq!(s.active_persona_id().unwrap(), ids.2);
    let chars = s.characters().unwrap();
    assert_eq!((chars.len(), chars[0].name.as_str(), chars[0].default_persona_id), (1, "Mara", None));
    let chat = s.chat_info(ids.1).unwrap().unwrap();
    assert_eq!((chat.title.as_str(), chat.messages, chat.persona_id, chat.effective_persona_id), ("Alt", 2, None, ids.2));
    let msgs = s.messages(ids.1).unwrap();
    assert_eq!(msgs.iter().map(|m| (m.content.as_str(), m.persona_id)).collect::<Vec<_>>(), [("Hi.", None), ("Hallo", None)]);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The priority columns arrive with their defaults; personas, characters, chats and
/// messages stay as they were.
#[test]
fn migration_adds_priorities_and_keeps_the_data() {
    let mut ids = (0, 0, 0);
    let dir = old_db(USER_PRIORITY_ADDED - 1, |s| {
        s.conn
            .execute("UPDATE personas SET name = 'Lukas', description = 'Kapitän.', pronouns = 'er/ihm', appearance = 'Bart.', every_turn = 1", [])
            .unwrap();
        let c = old_character(s, "Mara");
        let p: i64 = s.conn.query_row("SELECT MIN(id) FROM personas", [], |r| r.get(0)).unwrap();
        s.conn.execute("UPDATE characters SET default_persona_id = ?1", params![p]).unwrap();
        let chat = old_chat(s, c, &["Hi.", "Hallo"]);
        s.conn.execute("UPDATE chats SET persona_id = ?1, title = 'Alt'", params![p]).unwrap();
        ids = (c, chat, p);
    });
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (USER_PRIORITY_ADDED - 1, schema_target()));
    let p = s.persona(ids.2).unwrap().unwrap();
    assert_eq!(
        (p.name.as_str(), p.description.as_str(), p.pronouns.as_str(), p.appearance.as_str(), p.every_turn, p.card_priority),
        ("Lukas", "Kapitän.", "er/ihm", "Bart.", true, Priority::Persona)
    );
    let c = s.character(ids.0).unwrap().unwrap().info;
    assert_eq!((c.name.as_str(), c.default_persona_id, c.user_priority, c.chats), ("Mara", Some(ids.2), None, 1));
    let chat = s.chat_info(ids.1).unwrap().unwrap();
    assert_eq!((chat.title.as_str(), chat.messages, chat.persona_id), ("Alt", 2, Some(ids.2)));
    let msgs = s.messages(ids.1).unwrap();
    assert_eq!(msgs.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["Hi.", "Hallo"]);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Persona and character priority are stored; the user check runs against the given persona,
/// else the persona of a new chat, and its status shows in the character info.
#[test]
fn priorities_and_user_check() {
    let (mut s, dir) = temp_store();
    let id = s.add_character(&sample(r#"{"name":"Eris","description":"{{user}} ist ihr Stiefsohn. Eris hat rote Haare."}"#), None).unwrap().id;
    let user = s.active_persona_id().unwrap();
    let lena = s
        .add_persona(&PersonaEdit { pronouns: Some("sie/ihr".into()), card_priority: Some("card".into()), ..PersonaEdit::name("Lena") })
        .unwrap();
    assert_eq!(lena.card_priority, Priority::Card);
    assert_eq!(lena.figure().priority, Priority::Card);
    let keep = s.update_persona(lena.id, &PersonaEdit { name: Some("Lena".into()), ..PersonaEdit::default() }).unwrap().unwrap();
    assert_eq!(keep.card_priority, Priority::Card, "a field left out stays");
    assert_eq!(s.add_persona(&PersonaEdit::name("Kai")).unwrap().card_priority, Priority::Persona);

    // The active persona has no pronouns: nothing conflicts.
    assert_eq!(s.character(id).unwrap().unwrap().info.user_check_status, Fit::Ok);
    let UserCheck::Done { persona_id, report } = s.user_check(id, None).unwrap() else { panic!() };
    assert_eq!((persona_id, report.status), (user, Fit::Ok));
    let UserCheck::Done { persona_id, report } = s.user_check(id, Some(lena.id)).unwrap() else { panic!() };
    assert_eq!((persona_id, report.status), (lena.id, Fit::Conflict));
    assert!(report.items.iter().all(|i| !i.excerpt.contains("rote Haare")), "the character's hair is not about {{{{user}}}}");
    // A default persona is the one of a new chat.
    s.set_default_persona(id, Some(lena.id)).unwrap();
    assert_eq!(s.characters().unwrap()[0].user_check_status, Fit::Conflict);
    assert!(matches!(s.user_check(id, None).unwrap(), UserCheck::Done { persona_id, .. } if persona_id == lena.id));
    assert_eq!(s.user_check(id, Some(999)).unwrap(), UserCheck::NoPersona);
    assert_eq!(s.user_check(999, None).unwrap(), UserCheck::NoCharacter);

    assert_eq!(s.character(id).unwrap().unwrap().info.user_priority, None);
    assert!(s.set_user_priority(id, Some(Priority::Card)).unwrap());
    assert_eq!(s.character(id).unwrap().unwrap().info.user_priority, Some(Priority::Card));
    assert!(s.set_user_priority(id, None).unwrap());
    assert_eq!(s.characters().unwrap()[0].user_priority, None);
    assert!(!s.set_user_priority(999, None).unwrap());
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

/// Default persona per character, chat binding, effective persona, forks, and what a deleted
/// persona leaves behind.
#[test]
fn personas_bind_to_chats_characters_and_messages() {
    let (mut s, dir) = temp_store();
    let user = s.active_persona_id().unwrap();
    let bea = s.add_persona(&PersonaEdit::name("Bea")).unwrap().id;
    let carl = s.add_persona(&PersonaEdit::name("Carl")).unwrap().id;
    let a = s.add_character(&sample(r#"{"name":"A"}"#), None).unwrap().id;
    let b = s.add_character(&sample(r#"{"name":"B"}"#), None).unwrap().id;

    // Without a default a new chat follows the active persona.
    let free = s.create_chat(b, None).unwrap();
    let info = s.chat_info(free).unwrap().unwrap();
    assert_eq!((info.persona_id, info.effective_persona_id), (None, user));
    assert_eq!(s.persona_for_new_chat(b).unwrap().id, user);

    assert_eq!(s.set_default_persona(a, Some(bea)).unwrap(), Bind::Done);
    assert_eq!(s.set_default_persona(a, Some(999)).unwrap(), Bind::NoPersona);
    assert_eq!(s.set_default_persona(999, None).unwrap(), Bind::NoTarget);
    assert_eq!(s.character(a).unwrap().unwrap().info.default_persona_id, Some(bea));
    assert_eq!(s.persona_for_new_chat(a).unwrap().id, bea);
    let bound = s.create_chat(a, None).unwrap();
    let info = s.chat_info(bound).unwrap().unwrap();
    assert_eq!((info.persona_id, info.effective_persona_id), (Some(bea), bea));

    // Switching the active persona moves only the free chat.
    s.activate_persona(carl).unwrap();
    assert_eq!(s.chat_persona(bound).unwrap().id, bea);
    assert_eq!(s.chat_persona(free).unwrap().id, carl);
    assert_eq!(s.chat_info(free).unwrap().unwrap().effective_persona_id, carl);
    assert_eq!(s.recent(8).unwrap().iter().find(|r| r.chat.id == bound).unwrap().chat.persona_id, Some(bea));
    assert_eq!(s.chats_of(a).unwrap()[0].effective_persona_id, bea);

    assert_eq!(s.set_chat_persona(free, Some(user)).unwrap(), Bind::Done);
    assert_eq!(s.chat_persona(free).unwrap().id, user);
    assert_eq!(s.set_chat_persona(free, Some(999)).unwrap(), Bind::NoPersona);
    assert_eq!(s.set_chat_persona(999, None).unwrap(), Bind::NoTarget);
    assert_eq!(s.set_chat_persona(free, None).unwrap(), Bind::Done);
    assert_eq!(s.chat_persona(free).unwrap().id, carl);

    // Messages remember their persona; edits and forks keep it.
    let m = s.add_user_message(bound, "Hallo", bea).unwrap();
    assert_eq!(m.persona_id, Some(bea));
    s.add_message(bound, "assistant", "Hi.").unwrap();
    s.edit_message(m.id, "Hallo!").unwrap();
    assert_eq!(s.messages(bound).unwrap().iter().map(|m| m.persona_id).collect::<Vec<_>>(), [Some(bea), None]);
    let fork = s.fork_chat(bound, m.id, "").unwrap();
    assert_eq!(s.messages(fork).unwrap()[0].persona_id, Some(bea));
    assert_eq!(s.chat_info(fork).unwrap().unwrap().persona_id, Some(bea));

    // A deleted persona leaves no dangling ids; its chats follow the active one again.
    assert_eq!(s.delete_persona(bea).unwrap(), PersonaDelete::Done(carl));
    assert_eq!(s.character(a).unwrap().unwrap().info.default_persona_id, None);
    for chat in [bound, fork] {
        let info = s.chat_info(chat).unwrap().unwrap();
        assert_eq!((info.persona_id, info.effective_persona_id), (None, carl));
        assert!(s.messages(chat).unwrap().iter().all(|m| m.persona_id.is_none()));
    }
    let left: i64 = s
        .conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM chats WHERE persona_id = ?1) + (SELECT COUNT(*) FROM characters WHERE default_persona_id = ?1)
                  + (SELECT COUNT(*) FROM messages WHERE persona_id = ?1)",
            params![bea],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn persona_fields_are_stored_and_kept() {
    let (mut s, dir) = temp_store();
    let p = s
        .add_persona(&PersonaEdit {
            pronouns: Some("sie/ihr".into()),
            appearance: Some("Groß.".into()),
            ..PersonaEdit::name("Bea")
        })
        .unwrap();
    assert_eq!((p.pronouns.as_str(), p.appearance.as_str(), p.personality.as_str(), p.background.as_str()), ("sie/ihr", "Groß.", "", ""));
    let p = s.update_persona(p.id, &PersonaEdit { background: Some("Hafen.".into()), appearance: Some(String::new()), ..PersonaEdit::default() }).unwrap().unwrap();
    assert_eq!((p.name.as_str(), p.pronouns.as_str(), p.appearance.as_str(), p.background.as_str()), ("Bea", "sie/ihr", "", "Hafen."));
    let f = p.figure();
    assert_eq!((f.pronouns.as_str(), f.background.as_str()), ("sie/ihr", "Hafen."));
    assert!(!f.every_turn, "the static head is the default");
    let p = s.update_persona(p.id, &PersonaEdit { every_turn: Some(true), ..PersonaEdit::default() }).unwrap().unwrap();
    assert!(p.every_turn && p.figure().every_turn);
    assert_eq!(p.background, "Hafen.", "other fields stay");
    assert!(s.add_persona(&PersonaEdit::default()).is_err(), "a name is required");
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn personas_crud_and_the_last_one_stays() {
    let (mut s, dir) = temp_store();
    let first = s.active_persona().unwrap();
    assert_eq!(first.name, "User");
    let b = s.add_persona(&PersonaEdit { description: Some("{{user}} mag Tee.".into()), ..PersonaEdit::name("Bea") }).unwrap();
    let c = s.add_persona(&PersonaEdit::name("Carl")).unwrap();
    assert_eq!(s.personas().unwrap().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Bea", "Carl", "User"]);
    assert_eq!(s.active_persona_id().unwrap(), first.id, "adding does not switch");

    let edited = s.update_persona(b.id, &PersonaEdit { description: Some("Neu.".into()), ..PersonaEdit::default() }).unwrap().unwrap();
    assert_eq!((edited.name.as_str(), edited.description.as_str()), ("Bea", "Neu."));
    assert!(s.update_persona(999, &PersonaEdit::name("x")).unwrap().is_none());

    assert!(s.activate_persona(b.id).unwrap());
    assert!(!s.activate_persona(999).unwrap());
    assert_eq!(s.user_name().unwrap(), "Bea");
    assert_eq!(s.set_user_name("Beatrice").unwrap(), b.id);
    assert_eq!(s.persona(b.id).unwrap().unwrap().name, "Beatrice", "the name setting renames the active persona");
    assert_eq!(s.persona(first.id).unwrap().unwrap().name, "User");

    // Picture with crop position; deleting the persona removes its file.
    assert!(s.set_persona_avatar(b.id, Some((b"jpeg", "image/jpeg"))).unwrap());
    assert!(s.set_persona_avatar_pos(b.id, 130.0, 20.0).unwrap());
    let p = s.persona(b.id).unwrap().unwrap();
    assert_eq!((p.has_avatar, p.avatar_rev, p.avatar_pos), (true, 1, [100.0, 20.0]));
    let (file, mime) = s.persona_avatar(b.id).unwrap().unwrap();
    assert_eq!((std::fs::read(&file).unwrap().as_slice(), mime.as_str()), (&b"jpeg"[..], "image/jpeg"));
    assert!(s.persona_avatar(c.id).unwrap().is_none());
    assert!(!s.set_persona_avatar(999, None).unwrap());

    // Deleting the active one makes another active; the last one stays.
    assert_eq!(s.delete_persona(b.id).unwrap(), PersonaDelete::Done(first.id));
    assert!(!file.exists());
    assert_eq!(s.delete_persona(b.id).unwrap(), PersonaDelete::Missing);
    assert_eq!(s.delete_persona(first.id).unwrap(), PersonaDelete::Done(c.id));
    assert_eq!(s.delete_persona(c.id).unwrap(), PersonaDelete::Last, "there is always one");
    assert_eq!(s.user_name().unwrap(), "Carl");
    drop(s);
    let s = Store::open(&dir).unwrap(); // the choice is stored
    assert_eq!(s.active_persona_id().unwrap(), c.id);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn user_name_setting() {
    let (mut s, dir) = temp_store();
    assert_eq!(s.user_name().unwrap(), "User");
    s.set_user_name("Lukas").unwrap();
    s.set_user_name("Alex").unwrap();
    assert_eq!(s.user_name().unwrap(), "Alex");
    assert_eq!(s.theme().unwrap(), None);
    s.set_theme("papier").unwrap();
    s.set_theme("glut").unwrap();
    assert_eq!(s.theme().unwrap().as_deref(), Some("glut"));
    assert_eq!(s.user_name().unwrap(), "Alex", "settings keep their own keys");
    assert_eq!(s.home_layout().unwrap(), None);
    s.set_home_layout("bento").unwrap();
    assert_eq!(s.home_layout().unwrap().as_deref(), Some("bento"));
    assert_eq!(s.theme().unwrap().as_deref(), Some("glut"), "the layout has its own key");
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Schema 11 → 12: the new tables arrive empty and the data stays; `engine.json` is taken over
/// once, 1:1, as engine "Eigener Build" plus the active profile (port and stop-on-exit as
/// settings), and never again on later starts.
#[test]
fn migration_adds_model_profiles_and_takes_over_engine_json_once() {
    let mut ids = (0, 0);
    let dir = old_db(MODEL_PROFILES_ADDED - 1, |s| {
        let c = old_character(s, "Mara");
        ids = (c, old_chat(s, c, &["Hi.", "Hallo"]));
    });
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (MODEL_PROFILES_ADDED - 1, schema_target()));
    assert!(s.profiles().unwrap().is_empty() && s.engines().unwrap().is_empty());
    assert_eq!((s.launch_spec().unwrap(), s.engine_port().unwrap()), (None, None));
    assert_eq!(s.sampling().unwrap(), (rp_core::sampler::Sampler::default(), false));
    assert_eq!(s.messages(ids.1).unwrap().iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["Hi.", "Hallo"]);

    let cfg: crate::engine::EngineConfig = serde_json::from_str(
        r#"{ "llama_server": "D:\\llm\\b1\\llama-server.exe", "extra_path": ["C:\\rocm\\bin"], "model": "C:\\m\\Orion-Q4_K_M.gguf",
             "port": 8081, "args": ["-c", "131072", "-ngl", "999", "-fa", "on"], "stop_on_exit": false }"#,
    )
    .unwrap();
    let (engine_id, profile_id) = s.import_engine_json(&cfg).unwrap().unwrap();
    assert_eq!(s.import_engine_json(&cfg).unwrap(), None, "only once");
    let e = s.engine(engine_id).unwrap().unwrap();
    assert_eq!(
        (e.name.as_str(), e.exe.as_str(), e.extra_path.clone(), e.origin.as_str(), e.version.clone()),
        ("Eigener Build", "D:\\llm\\b1\\llama-server.exe", vec!["C:\\rocm\\bin".to_string()], "own", None)
    );
    let p = s.active_profile().unwrap().unwrap();
    assert_eq!((p.id, p.name.as_str(), p.engine_id, p.model_file.as_str()), (profile_id, "Orion-Q4_K_M", engine_id, "C:\\m\\Orion-Q4_K_M.gguf"));
    assert_eq!(p.load_args, ["-c", "131072", "-ngl", "999", "-fa", "on"]);
    assert_eq!((p.sampler.clone(), p.reasoning.as_str(), p.measured), (rp_core::sampler::Sampler::legacy(), "off", false));
    let spec = s.launch_spec().unwrap().unwrap();
    assert_eq!((spec.port, spec.profile_id, spec.args.clone()), (8081, Some(profile_id), cfg.args.clone()));
    assert_eq!((spec.exe.clone(), spec.model.clone(), spec.extra_path.clone()), (cfg.llama_server.clone(), cfg.model.clone(), cfg.extra_path.clone()));
    assert_eq!(s.engine_port().unwrap(), Some(8081));
    drop(s);

    let mut s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (schema_target(), schema_target()));
    assert_eq!(s.import_engine_json(&cfg).unwrap(), None, "a second start takes nothing over");
    assert_eq!((s.profiles().unwrap().len(), s.engines().unwrap().len()), (1, 1));
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// One active profile at a time; the active one cannot be deleted; edits keep what they leave out.
#[test]
fn profiles_activate_edit_and_delete() {
    let (mut s, dir) = temp_store();
    let cfg: crate::engine::EngineConfig = serde_json::from_str(r#"{ "llama_server": "a.exe", "model": "m.gguf" }"#).unwrap();
    let (engine, first) = s.import_engine_json(&cfg).unwrap().unwrap();
    let new = ProfileEdit {
        name: Some("Zwei".into()),
        engine_id: Some(engine),
        model_file: Some("n.gguf".into()),
        load_args: Some(vec!["-c".into(), "4096".into()]),
        ..ProfileEdit::default()
    };
    let second = s.add_profile(&new).unwrap();
    assert_eq!((second.active, second.sampler.clone(), second.reasoning.as_str()), (false, rp_core::sampler::Sampler::default(), "off"));
    assert!(s.activate_profile(second.id).unwrap());
    assert!(!s.activate_profile(999).unwrap());
    let active: Vec<i64> = s.profiles().unwrap().into_iter().filter(|p| p.active).map(|p| p.id).collect();
    assert_eq!(active, [second.id]);
    assert_eq!(s.delete_profile(second.id).unwrap(), ProfileDelete::Active);
    assert_eq!(s.delete_profile(999).unwrap(), ProfileDelete::Missing);
    let edit = ProfileEdit { reasoning: Some("on".into()), ..ProfileEdit::default() };
    let p = s.update_profile(second.id, &edit).unwrap().unwrap();
    assert_eq!((p.name.as_str(), p.load_args.clone(), p.reasoning.as_str()), ("Zwei", vec!["-c".to_string(), "4096".to_string()], "on"));
    assert!(s.sampling().unwrap().1, "reasoning on");
    assert!(s.update_profile(999, &edit).unwrap().is_none());
    // A benchmark marks the start options measured; typing other ones by hand takes that back.
    let set = ProfileEdit { load_args: Some(vec!["-c".into(), "8192".into()]), measured: Some(true), ..ProfileEdit::default() };
    assert!(s.update_profile(second.id, &set).unwrap().unwrap().measured);
    assert!(s.update_profile(second.id, &edit).unwrap().unwrap().measured, "a sampler change keeps it");
    let typed = ProfileEdit { load_args: Some(vec!["-c".into(), "4096".into()]), ..ProfileEdit::default() };
    assert!(!s.update_profile(second.id, &typed).unwrap().unwrap().measured);
    s.set_bench_result(second.id, &serde_json::json!({ "rows": [] })).unwrap();
    assert_eq!(s.bench_result(second.id).unwrap(), Some(serde_json::json!({ "rows": [] })));
    // A new result measured with apps open is the last one; the older one (no conditions) stays.
    let open = serde_json::json!({ "rows": [], "conditions": { "ram_available_mib": 9000, "vram_free_mib": 16000, "commit_free_mib": 40000 } });
    s.set_bench_result(second.id, &open).unwrap();
    assert_eq!(s.bench_result(second.id).unwrap(), Some(open.clone()));
    assert_eq!(s.bench_results(second.id).unwrap(), vec![open, serde_json::json!({ "rows": [] })]);
    s.set_bench_result(first, &serde_json::json!({ "rows": [] })).unwrap();
    assert_eq!(s.delete_profile(first).unwrap(), ProfileDelete::Done);
    assert!(s.bench_results(first).unwrap().is_empty(), "both keys go with the profile");
    assert_eq!(s.launch_spec().unwrap().unwrap().port, 8080, "engine.json without port: 8080");
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

/// Schema 12 → 13: existing cards get their content hash (so a re-import is a duplicate) and are
/// listed for a thumbnail; an edit changes the hash; a thumbnail of an old picture is not kept.
#[test]
fn migration_adds_content_hash_and_thumbnails() {
    let mut id = 0;
    let dir = old_db(IMPORT_ADDED - 1, |s| {
        id = old_character(s, "Mara");
        s.conn.execute("UPDATE characters SET has_png = 1 WHERE id = ?1", params![id]).unwrap();
        std::fs::write(s.png_path(id), include_bytes!("../../../../tests/fixtures/ilse_brandt.png")).unwrap();
    });
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (IMPORT_ADDED - 1, schema_target()));
    let hash = content_hash(&serde_json::json!({ "name": "Mara", "first_mes": "Hi." })).unwrap();
    assert_eq!(hash.len(), 64);
    assert_eq!(s.character_by_hash(&hash).unwrap(), Some(id));
    // The same content in V2 form, with other line ends and spaces: the same hash.
    let v2 = serde_json::json!({ "spec": "chara_card_v2", "data": { "name": " Mara", "first_mes": "Hi.\r\n", "tags": [] } });
    assert_eq!(content_hash(&v2).as_deref(), Some(hash.as_str()));

    assert_eq!(s.thumbs_missing().unwrap(), vec![id]);
    assert_eq!(s.thumb(id).unwrap(), None);
    let (_, rev) = s.thumb_source(id).unwrap().unwrap();
    assert!(!s.set_thumb(id, rev + 1, Some(b"old")).unwrap(), "a thumbnail of another revision is not kept");
    assert!(s.set_thumb(id, rev, Some(b"webp")).unwrap());
    assert!(s.thumbs_missing().unwrap().is_empty());
    assert_eq!(s.thumb(id).unwrap(), Some(s.thumb_path(id)));
    s.set_avatar(id, Some((b"\x89PNG new", "image/png"))).unwrap();
    assert_eq!((s.thumb(id).unwrap(), s.thumbs_missing().unwrap()), (None, vec![id]));

    let mut card = s.character(id).unwrap().unwrap().card;
    card.description = "Neu.".into();
    s.update_character(id, &card).unwrap();
    assert_eq!(s.character_by_hash(&hash).unwrap(), None);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The message ids a search finds, in the order it answers them.
fn found(s: &Store, q: &str, chat: Option<i64>) -> Vec<i64> {
    s.search_messages(q, chat, 50).unwrap().into_iter().map(|h| h.message_id).collect()
}

/// The index follows every change of a message: insert, streamed reply, edit, rewind and its undo,
/// a fork, a deleted chat or character; case, accents and umlauts do not matter; typed FTS syntax
/// is plain text.
#[test]
fn message_search_follows_every_change() {
    let (mut s, dir) = temp_store();
    let a = s.add_character(&sample(r#"{"name":"Mara"}"#), None).unwrap().id;
    let chat = s.create_chat(a, Some("Willkommen im Leuchtturm.")).unwrap();
    let m1 = s.add_message(chat, "user", "Ich bringe dir Äpfel und eine Laterne.").unwrap().id;
    let reply = s.add_message(chat, "assistant", "").unwrap().id;
    assert_eq!(found(&s, "laterne", None), vec![m1]);
    s.finish_message(reply, "Die Laterne brennt die ganze Nacht.", None).unwrap();
    assert_eq!(found(&s, "LATERNE", Some(chat)), vec![m1, reply], "one chat: in story order");
    assert_eq!(found(&s, "apfel", None), vec![m1], "umlauts fold");
    assert_eq!(found(&s, "Äpf", None), vec![m1], "a word start is enough");
    assert_eq!(found(&s, "laterne nacht", None), vec![reply], "every word must match");
    assert!(found(&s, "\"laterne\" OR NEAR(*", None).is_empty(), "FTS syntax is plain text");

    let hit = &s.search_messages("brennt", None, 5).unwrap()[0];
    assert_eq!((hit.chat_id, hit.character_id, hit.character.as_str(), hit.role.as_str()), (chat, a, "Mara", "assistant"));
    assert!(hit.snippet.contains(&format!("{MARK_START}brennt{MARK_END}")), "{}", hit.snippet);

    s.edit_message(m1, "Ich bringe dir Birnen.").unwrap();
    assert!(found(&s, "apfel", None).is_empty());
    assert_eq!(found(&s, "birnen", None), vec![m1]);

    s.rewind(chat, m1).unwrap();
    assert!(found(&s, "birnen", None).is_empty(), "rewound messages are out");
    s.undo_rewind(chat).unwrap();
    assert_eq!(found(&s, "birnen", None), vec![m1], "and back after the undo");

    let fork = s.fork_chat(chat, m1, "").unwrap();
    assert_eq!(found(&s, "birnen", None).len(), 2, "a fork's copies are found too");
    s.set_chat_deleted(fork, true).unwrap();
    assert_eq!(found(&s, "birnen", None), vec![m1], "a deleted chat is left out");
    s.set_deleted(a, true).unwrap();
    assert!(found(&s, "birnen", None).is_empty(), "so is a deleted character");
    s.set_deleted(a, false).unwrap();
    s.delete_message(m1).unwrap();
    assert!(found(&s, "birnen", None).is_empty());
    let indexed = |s: &Store| -> i64 { s.conn.query_row("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'birnen'", [], |r| r.get(0)).unwrap() };
    assert_eq!(indexed(&s), 1, "the deleted fork's copy stays indexed until the purge");
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert_eq!(indexed(&s), 0, "the purge at the next start takes it out of the index");
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fts_query_quotes_every_word() {
    use super::search::fts_query;
    assert_eq!(fts_query("  "), None);
    assert_eq!(fts_query("- * :"), None);
    assert_eq!(fts_query("Mara NEAR"), Some("\"Mara\"* \"NEAR\"*".into()));
    assert_eq!(fts_query("sag \"hi\""), Some("\"sag\"* \"\"\"hi\"\"\"*".into()));
}

/// Migration 14 indexes the messages already there (only the active ones).
#[test]
fn migration_indexes_existing_messages() {
    let mut chat = 0;
    let dir = old_db(SEARCH_ADDED - 1, |s| {
        let id = old_character(s, "Mara");
        chat = old_chat(s, id, &["Hi.", "Erzähl vom Meer", "Das Meer ist grün."]);
        s.conn.execute("UPDATE messages SET deleted_at = 5 WHERE content = 'Hi.'", []).unwrap();
    });
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (SEARCH_ADDED - 1, schema_target()));
    assert_eq!(s.search_messages("meer", Some(chat), 10).unwrap().len(), 2);
    assert_eq!(s.search_messages("erzahl", None, 10).unwrap().len(), 1);
    assert!(s.search_messages("hi", None, 10).unwrap().is_empty());
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Favourites, own tags (cleaned, case-insensitive, unused ones dropped), the last chat time and
/// every bulk action.
#[test]
fn favorites_tags_and_bulk_actions() {
    let (mut s, dir) = temp_store();
    let a = s.add_character(&sample(r#"{"name":"Ada","tags":["card tag"]}"#), None).unwrap().id;
    let b = s.add_character(&sample(r#"{"name":"Bo"}"#), None).unwrap().id;
    let info = |s: &Store, id: i64| s.character(id).unwrap().unwrap().info;
    assert!(!info(&s, a).favorite && info(&s, a).own_tags.is_empty() && info(&s, a).last_chat_at.is_none());
    assert_eq!(info(&s, a).tags, ["card tag"], "the card's own tags stay apart");

    assert!(s.set_favorite(a, true).unwrap());
    assert!(!s.set_favorite(999, true).unwrap());
    assert!(info(&s, a).favorite);

    let set = s.set_character_tags(a, &["  Fantasy ".into(), "fantasy".into(), "".into(), "Meer  und Wind".into()]).unwrap().unwrap();
    assert_eq!(set, ["Fantasy", "Meer und Wind"]);
    assert_eq!(s.set_character_tags(b, &["FANTASY".into()]).unwrap().unwrap(), ["Fantasy"], "joins the existing tag");
    assert_eq!(s.set_character_tags(999, &[]).unwrap(), None);
    assert_eq!(s.tags().unwrap(), vec![TagCount { name: "Fantasy".into(), count: 2 }, TagCount { name: "Meer und Wind".into(), count: 1 }]);
    s.set_character_tags(a, &["Fantasy".into()]).unwrap();
    assert_eq!(s.tags().unwrap().len(), 1, "an unused tag goes");

    assert_eq!(s.bulk_characters(&[a, b, 999], Bulk::Tag("Neu")).unwrap(), 2);
    assert_eq!(info(&s, b).own_tags, ["Fantasy", "Neu"]);
    assert_eq!(s.bulk_characters(&[a, b], Bulk::Untag("neu")).unwrap(), 2);
    assert!(s.tags().unwrap().iter().all(|t| t.name != "Neu"));
    assert_eq!(s.bulk_characters(&[a, b], Bulk::Favorite(true)).unwrap(), 1, "a was one already");
    assert_eq!(s.bulk_characters(&[a, b], Bulk::Favorite(false)).unwrap(), 2);
    assert!(s.bulk_characters(&[a], Bulk::Tag("  ")).is_err());

    let chat = s.create_chat(b, Some("Hi.")).unwrap();
    let at = s.chat_info(chat).unwrap().unwrap().updated_at;
    assert_eq!(info(&s, b).last_chat_at, Some(at));
    assert!(info(&s, b).created_at > 0);

    assert_eq!(s.bulk_characters(&[a, b], Bulk::Delete).unwrap(), 2);
    assert!(s.characters().unwrap().is_empty());
    assert!(s.tags().unwrap().is_empty(), "deleted characters do not count");
    assert_eq!(s.bulk_characters(&[a], Bulk::Restore).unwrap(), 1);
    assert_eq!(s.characters().unwrap().len(), 1);
    assert_eq!(s.tags().unwrap(), vec![TagCount { name: "Fantasy".into(), count: 1 }]);
    s.bulk_characters(&[a], Bulk::Delete).unwrap();
    drop(s);
    // The purge at the next start takes the deleted characters' tags with them.
    let s = Store::open(&dir).unwrap();
    let rows: i64 = s.conn.query_row("SELECT (SELECT COUNT(*) FROM tags) + (SELECT COUNT(*) FROM character_tags)", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Migration 15 keeps every character, none a favourite, without own tags.
#[test]
fn migration_adds_favorites_and_tags() {
    let dir = old_db(TAGS_ADDED - 1, |s| {
        old_character(s, "Mara");
    });
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (TAGS_ADDED - 1, schema_target()));
    let all = s.characters().unwrap();
    assert_eq!(all.len(), 1);
    assert!(!all[0].favorite && all[0].own_tags.is_empty() && all[0].created_at == 1);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Migration 16 on an existing database: every character stays, none has an own short text yet;
/// on a fresh one the column is there too. The text is trimmed, limited and can go back to null.
#[test]
fn migration_adds_tagline() {
    let mut id = 0;
    let dir = old_db(TAGLINE_ADDED - 1, |s| {
        id = old_character(s, "Mara");
    });
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (TAGLINE_ADDED - 1, schema_target()));
    let all = s.characters().unwrap();
    assert_eq!((all.len(), all[0].tagline.clone()), (1, None));
    assert!(s.set_tagline(id, Some("  Hütet den Leuchtturm ")).unwrap());
    assert_eq!(s.character(id).unwrap().unwrap().info.tagline.as_deref(), Some("Hütet den Leuchtturm"));
    assert!(s.set_tagline(id, Some(&"ä".repeat(TAGLINE_MAX))).unwrap());
    assert!(s.set_tagline(id, Some(&"ä".repeat(TAGLINE_MAX + 1))).is_err());
    assert!(s.set_tagline(id, Some("   ")).unwrap());
    assert_eq!(s.character(id).unwrap().unwrap().info.tagline, None);
    assert!(!s.set_tagline(999, Some("x")).unwrap());
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();

    let (mut fresh, dir) = temp_store();
    assert_eq!(fresh.migrated(), (0, schema_target()));
    assert_eq!(schema_target(), LIBRARY_INDEX_ADDED + 1);
    let c = fresh.add_character(&sample(r#"{"name":"Ilse","first_mes":"Moin."}"#), None).unwrap();
    assert_eq!(c.tagline, None);
    assert!(fresh.set_tagline(c.id, Some("Kurz")).unwrap());
    assert_eq!(fresh.characters().unwrap()[0].tagline.as_deref(), Some("Kurz"));
    drop(fresh);
    std::fs::remove_dir_all(dir).unwrap();
}

fn imported(hash: &str) -> ImportedChat {
    let msg = |role: &'static str, content: &str, t: i64, swipes: Option<(Vec<&str>, usize)>| ImportedMsg {
        role,
        content: content.into(),
        created_at: t,
        swipes: swipes.map(|(s, i)| (s.into_iter().map(str::to_string).collect(), i)),
    };
    ImportedChat {
        title: "Fähre".into(),
        created_at: 100,
        hash: hash.into(),
        messages: vec![
            msg("assistant", "Moin.", 100, Some((vec!["Hallo.", "Moin."], 1))),
            msg("user", "Wohin?", 160, None),
            msg("assistant", "Nach Amrum.", 220, Some((vec!["Nach Amrum.", "Nach Föhr.", "Kaffee."], 0))),
            msg("assistant", "Einzeln.", 230, Some((vec!["Einzeln."], 0))),
        ],
    }
}

/// An imported chat keeps order, times and every reply variant with the active one; the same
/// content is stored once; edits change the active variant; a fork keeps the variants.
#[test]
fn imported_chat_with_swipes() {
    let (mut s, dir) = temp_store();
    let c = s.add_character(&sample(r#"{"name":"Theo","first_mes":"Moin."}"#), None).unwrap();
    let (chat, new) = s.import_chat(c.id, &imported("abc")).unwrap();
    assert!(new);
    assert_eq!(s.import_chat(c.id, &imported("abc")).unwrap(), (chat, false));
    assert_eq!(s.chat_by_import_hash("abc", c.id).unwrap(), Some(chat));
    assert_eq!(s.chats_of(c.id).unwrap().len(), 1);
    let info = s.chat_info(chat).unwrap().unwrap();
    assert_eq!((info.title.as_str(), info.created_at, info.updated_at, info.messages), ("Fähre", 100, 230, 4));

    let m = s.messages(chat).unwrap();
    type Row<'a> = (&'a str, &'a str, Option<Vec<String>>, Option<usize>);
    let got: Vec<Row> =
        m.iter().map(|m| (m.role.as_str(), m.content.as_str(), m.swipes.clone(), m.swipe_id)).collect();
    let v = |l: &[&str]| Some(l.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    assert_eq!(
        got,
        vec![
            ("assistant", "Moin.", v(&["Hallo.", "Moin."]), Some(1)),
            ("user", "Wohin?", None, None),
            ("assistant", "Nach Amrum.", v(&["Nach Amrum.", "Nach Föhr.", "Kaffee."]), Some(0)),
            ("assistant", "Einzeln.", None, None),
        ]
    );
    let times: Vec<i64> = s
        .conn
        .prepare("SELECT created_at FROM messages WHERE chat_id = ?1 ORDER BY id")
        .unwrap()
        .query_map(params![chat], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(times, vec![100, 160, 220, 230]);
    let json = serde_json::to_value(&m[0]).unwrap();
    assert_eq!((json["swipes"].as_array().map(Vec::len), json["swipe_id"].as_u64()), (Some(2), Some(1)));
    assert!(serde_json::to_value(&m[1]).unwrap().get("swipes").is_none());

    s.edit_message(m[2].id, "Doch nach Sylt.").unwrap();
    s.edit_message(m[1].id, "Wohin genau?").unwrap();
    let m = s.messages(chat).unwrap();
    assert_eq!((m[2].content.as_str(), m[2].swipes.clone()), ("Doch nach Sylt.", v(&["Doch nach Sylt.", "Nach Föhr.", "Kaffee."])));
    assert_eq!((m[1].content.as_str(), m[1].swipes.clone()), ("Wohin genau?", None));

    let fork = s.fork_chat(chat, m[2].id, "Zweig").unwrap();
    let f = s.messages(fork).unwrap();
    assert_eq!((f.len(), f[0].swipe_id, f[2].swipes.clone()), (3, Some(1), m[2].swipes.clone()));
    assert_eq!(s.chat_by_import_hash("abc", c.id).unwrap(), Some(chat), "a fork is no import");

    // A deleted chat no longer counts: the same file can come in again.
    s.set_chat_deleted(chat, true).unwrap();
    assert_eq!(s.chat_by_import_hash("abc", c.id).unwrap(), None);
    assert!(s.import_chat(c.id, &imported("abc")).unwrap().1);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The same chat file for another character is a chat of its own, not a duplicate.
#[test]
fn imported_chat_is_a_duplicate_only_for_its_character() {
    let (mut s, dir) = temp_store();
    let theo = s.add_character(&sample(r#"{"name":"Theo"}"#), None).unwrap();
    let ilse = s.add_character(&sample(r#"{"name":"Ilse"}"#), None).unwrap();
    let (first, new) = s.import_chat(theo.id, &imported("same")).unwrap();
    assert!(new);
    let (second, new) = s.import_chat(ilse.id, &imported("same")).unwrap();
    assert!(new, "another character: stored");
    assert_ne!(first, second);
    assert_eq!((s.chats_of(theo.id).unwrap().len(), s.chats_of(ilse.id).unwrap().len()), (1, 1));
    assert_eq!(s.import_chat(ilse.id, &imported("same")).unwrap(), (second, false));
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Migration 17 on an existing database: chats and messages stay, without variants.
#[test]
fn migration_adds_swipes() {
    let mut chat = 0;
    let dir = old_db(SWIPES_ADDED - 1, |s| {
        let c = old_character(s, "Mara");
        chat = old_chat(s, c, &["Hi.", "Hallo"]);
    });
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (SWIPES_ADDED - 1, schema_target()));
    let m = s.messages(chat).unwrap();
    let got: Vec<(&str, bool, Option<usize>)> = m.iter().map(|m| (m.content.as_str(), m.swipes.is_none(), m.swipe_id)).collect();
    assert_eq!(got, vec![("Hi.", true, None), ("Hallo", true, None)]);
    assert_eq!(s.chat_by_import_hash("x", 1).unwrap(), None);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Migration 18 on an existing database: characters, chats and messages stay; no chat has a look
/// of its own or a backdrop yet, and both can be stored afterwards.
#[test]
fn migration_adds_chat_looks_and_backdrops() {
    let mut chat = 0;
    let dir = old_db(LOOKS_ADDED - 1, |s| {
        let c = old_character(s, "Mara");
        chat = old_chat(s, c, &["Hi.", "Hallo"]);
    });
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (LOOKS_ADDED - 1, schema_target()));
    assert_eq!(s.messages(chat).unwrap().len(), 2);
    let view = s.chat_look(chat).unwrap().unwrap();
    assert_eq!((view.look, view.global), (Look::default(), Look::default()));
    assert_eq!((view.backdrop.mode.as_str(), view.backdrop.own, view.backdrop.character), ("character", None, None));
    let own = Look { size: Some(21.0), ..Look::default() };
    assert!(s.set_chat_look(chat, Some(&own), None).unwrap());
    assert!(s.set_backdrop(BackdropOf::Chat(chat), Some(b"jpeg")).unwrap());
    let view = s.chat_look(chat).unwrap().unwrap();
    assert_eq!((view.look, view.backdrop.mode.as_str(), view.backdrop.own.map(|b| b.rev)), (own, "own", Some(1)));
    // A deleted chat takes its look along.
    s.conn.execute("DELETE FROM chats WHERE id = ?1", params![chat]).unwrap();
    let left: i64 = s.conn.query_row("SELECT COUNT(*) FROM chat_looks", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The library as the uncached path makes it: one `character(id)` per listed card, by name.
fn fresh_library(s: &Store) -> Vec<serde_json::Value> {
    let ids: Vec<i64> = s.characters().unwrap().iter().map(|c| c.id).collect();
    ids.iter().map(|&id| serde_json::to_value(s.character(id).unwrap().unwrap().info).unwrap()).collect()
}

fn library(s: &Store) -> Vec<serde_json::Value> {
    s.characters().unwrap().into_iter().map(|c| serde_json::to_value(c).unwrap()).collect()
}

fn card_key(s: &Store, id: i64) -> i64 {
    s.conn.query_row("SELECT card_key FROM characters WHERE id = ?1", params![id], |r| r.get(0)).unwrap()
}

fn index_names(s: &Store) -> Vec<String> {
    let mut stmt = s.conn.prepare("SELECT name FROM sqlite_master WHERE type IN ('index', 'trigger') ORDER BY name").unwrap();
    stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
}

/// Migration 20 on a schema-19 database: the chats of a character and the recent chats are read
/// by index, every card gets a key, and the library lists exactly what it listed before. Later
/// migrations also run when the database is opened.
#[test]
fn migration_indexes_the_chats_and_keys_the_cards() {
    let (mut mara, mut ilse) = (0, 0);
    let dir = old_db(LIBRARY_INDEX_ADDED - 1, |s| {
        mara = old_character(s, "Mara");
        ilse = old_character(s, "Ilse");
        old_chat(s, mara, &["Hi.", "Hallo"]);
        old_chat(s, mara, &["Na?"]);
        s.conn.execute("INSERT INTO characters (name, raw_json, has_png, created_at) VALUES ('kaputt', '{nope', 0, 1)", []).unwrap();
    });
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (LIBRARY_INDEX_ADDED - 1, schema_target()));
    let names = index_names(&s);
    for name in ["chats_character", "chats_updated", "characters_card_key_insert", "characters_card_key_update"] {
        assert!(names.iter().any(|n| n == name), "{name} in {names:?}");
    }
    let plan: Vec<String> = s
        .conn
        .prepare("EXPLAIN QUERY PLAN SELECT COUNT(*) FROM chats WHERE character_id = ?1 AND deleted_at IS NULL")
        .unwrap()
        .query_map(params![mara], |r| r.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(plan.iter().any(|p| p.contains("chats_character")), "{plan:?}");
    assert!(card_key(&s, mara) != 0 && card_key(&s, ilse) != 0 && card_key(&s, mara) != card_key(&s, ilse));

    // Same entries as the uncached path; the broken card stays out.
    let listed = library(&s);
    assert_eq!(listed.iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Ilse", "Mara"]);
    assert_eq!(listed, fresh_library(&s));
    assert_eq!((listed[1]["chats"].as_i64(), listed[1]["last_chat_at"].as_i64()), (Some(2), Some(1)));

    // A new text gets a new key; what does not touch the text keeps it.
    let before = card_key(&s, mara);
    assert!(s.set_tagline(mara, Some("Kurz")).unwrap());
    s.set_favorite(mara, true).unwrap();
    assert_eq!(card_key(&s, mara), before);
    let mut card = s.character(mara).unwrap().unwrap().card;
    card.creator_notes = "Wacht über den Hafen von {{user}}.".into();
    s.update_character(mara, &card).unwrap().unwrap();
    assert_ne!(card_key(&s, mara), before);
    let listed = library(&s);
    assert_eq!(listed[1]["blurb"], format!("Wacht über den Hafen von {}.", s.active_persona().unwrap().name));
    assert_eq!(listed, fresh_library(&s));
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A new database comes with the current schema; every inserted card has a key at once.
#[test]
fn a_fresh_database_keys_every_card() {
    let (mut s, dir) = temp_store();
    assert_eq!(s.migrated(), (0, schema_target()));
    let a = s.add_character(&sample(r#"{"name":"Ilse","first_mes":"Moin."}"#), None).unwrap().id;
    let b = s.add_character(&sample(r#"{"name":"Ilse","first_mes":"Moin."}"#), None).unwrap().id;
    assert!(card_key(&s, a) != 0 && card_key(&s, b) != 0 && card_key(&s, a) != card_key(&s, b));
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The current schema removes two obsolete card columns without losing existing cards.
#[test]
fn migration_removes_unused_character_columns() {
    let mut id = 0;
    let dir = old_db(LIBRARY_INDEX_ADDED, |s| id = old_character(s, "Mara"));
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.migrated(), (LIBRARY_INDEX_ADDED, schema_target()));
    let columns: Vec<String> = s
        .conn
        .prepare("PRAGMA table_info(characters)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!columns.iter().any(|name| name == "sfw_locked" || name == "gate_reasons"), "{columns:?}");
    assert_eq!(s.character(id).unwrap().unwrap().card.name, "Mara");
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The cached library is always what the cards say now: after edits of a card, of the persona it
/// is checked against, and of the active persona's name (the blurb's `{{user}}`).
#[test]
fn the_cached_library_follows_every_change() {
    let (mut s, dir) = temp_store();
    let eris = s
        .add_character(&sample(r#"{"name":"Eris","description":"{{user}} ist ihr Stiefsohn. Eris hat rote Haare.","creator_notes":"Grüßt {{user}} am Tor."}"#), None)
        .unwrap()
        .id;
    let ole = s.add_character(&sample(r#"{"name":"Ole","personality":"Ruhig.","tags":["Hafen"]}"#), None).unwrap().id;
    let cold = library(&s);
    assert_eq!(cold, fresh_library(&s));
    assert_eq!(library(&s), cold, "a second call (all from the cache) is the same");

    let lena = s.add_persona(&PersonaEdit { pronouns: Some("sie/ihr".into()), ..PersonaEdit::name("Lena") }).unwrap();
    s.set_default_persona(eris, Some(lena.id)).unwrap();
    assert_eq!(library(&s), fresh_library(&s));
    let user = s.active_persona_id().unwrap();
    s.update_persona(user, &PersonaEdit::name("Kai")).unwrap().unwrap();
    s.update_persona(lena.id, &PersonaEdit { pronouns: Some("er/ihm".into()), ..PersonaEdit::default() }).unwrap().unwrap();
    let now = library(&s);
    assert_eq!(now, fresh_library(&s));
    assert_eq!(now.iter().find(|c| c["id"] == eris).unwrap()["blurb"], "Grüßt Kai am Tor.");
    s.set_default_persona(eris, None).unwrap();
    s.activate_persona(lena.id).unwrap();
    assert_eq!(library(&s), fresh_library(&s));

    let mut card = s.character(ole).unwrap().unwrap().card;
    card.personality = "Laut.".into();
    card.tags = vec!["Markt".into()];
    s.update_character(ole, &card).unwrap().unwrap();
    let now = library(&s);
    let o = now.iter().find(|c| c["id"] == ole).unwrap();
    assert_eq!((o["blurb"].as_str(), o["tags"].clone()), (Some("Laut."), serde_json::json!(["Markt"])));
    assert_eq!(now, fresh_library(&s));

    assert!(s.set_deleted(ole, true).unwrap());
    assert_eq!(library(&s).len(), 1);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The second connection of `GET /library` reads the same library and figures; it refuses a
/// database of another schema (the route then reads through the hub).
#[test]
fn the_reader_sees_what_the_store_wrote() {
    let (mut s, dir) = temp_store();
    let id = s.add_character(&sample(r#"{"name":"Ilse","first_mes":"Moin."}"#), None).unwrap().id;
    let chat = s.create_chat(id, Some("Moin.")).unwrap();
    let reader = Store::reader(&dir).unwrap();
    assert_eq!(library(&reader), library(&s));
    let a = reader.activity().unwrap();
    assert_eq!((a.len(), a[0].id, a[0].chats, a[0].last_chat_at.is_some()), (1, id, 1, true));
    assert_eq!(reader.recent(12).unwrap()[0].chat.id, chat);
    assert!(reader.conn.execute("UPDATE characters SET favorite = 1", []).is_err(), "read-only");
    drop((reader, s));
    std::fs::remove_dir_all(dir).unwrap();

    let old = old_db(LIBRARY_INDEX_ADDED - 1, |_| {});
    assert!(Store::reader(&old).is_err());
    std::fs::remove_dir_all(old).unwrap();
}
