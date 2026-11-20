//! Chat export: Markdown, plain text and SillyTavern's chat format (`.jsonl`), one chat as a file
//! or several in a ZIP. The text is built here from what the store holds; the routes only add the
//! download headers (`routes/chats.rs`).

use std::io::{Cursor, Write};

use serde_json::{json, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::logs::fmt_ts;
use crate::store::{Res, Store};

/// The three export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Markdown,
    Text,
    SillyTavern,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "md" => Some(Format::Markdown),
            "txt" => Some(Format::Text),
            "jsonl" => Some(Format::SillyTavern),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Markdown => "md",
            Format::Text => "txt",
            Format::SillyTavern => "jsonl",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Format::Markdown => "text/markdown; charset=utf-8",
            Format::Text => "text/plain; charset=utf-8",
            Format::SillyTavern => "application/jsonl; charset=utf-8",
        }
    }
}

/// One message as the export writes it. `name` is None for the user's messages whose persona is
/// gone (or that were written before personas were bound); the chat's persona fills it in.
#[derive(Debug, Clone)]
pub(crate) struct Line {
    pub is_user: bool,
    pub name: Option<String>,
    pub text: String,
    /// Unix seconds.
    pub sent: i64,
}

/// A whole chat, ready to be written in any format.
#[derive(Debug, Clone)]
pub(crate) struct Transcript {
    pub character: String,
    /// The persona the chat plays with now (its own, else the active one).
    pub persona: String,
    pub title: String,
    /// Unix seconds.
    pub created: i64,
    pub lines: Vec<Line>,
}

impl Transcript {
    fn name_of<'a>(&'a self, line: &'a Line) -> &'a str {
        match (&line.name, line.is_user) {
            (Some(n), true) => n,
            (None, true) => &self.persona,
            (_, false) => &self.character,
        }
    }
}

/// The chat as it is now: only active messages in their order, the character's name from the card
/// the chat plays with. None: no such chat (or it was deleted).
pub(crate) fn transcript(s: &Store, chat_id: i64) -> Res<Option<Transcript>> {
    let Some(info) = s.chat_info(chat_id)? else { return Ok(None) };
    let Some((character, _)) = s.chat_character_card(chat_id)? else { return Ok(None) };
    let persona = s.chat_persona(chat_id)?;
    Ok(Some(Transcript {
        character: character.card.name,
        persona: persona.name,
        title: info.title,
        created: info.created_at,
        lines: s.export_lines(chat_id)?,
    }))
}

/// Minutes east of UTC, limited to the zones that exist.
pub(crate) fn clamp_tz(tz: i32) -> i32 {
    tz.clamp(-14 * 60, 14 * 60)
}

/// `2026-11-15 14:03` in the asking device's time.
fn local_minute(seconds: i64, tz: i32) -> String {
    fmt_ts(seconds * 1000, tz)[..16].to_string()
}

/// ISO 8601 in UTC with milliseconds, as SillyTavern writes `send_date` (`Date.toISOString()`).
fn iso_utc(seconds: i64) -> String {
    let t = fmt_ts(seconds * 1000, 0);
    format!("{}T{}Z", &t[..10], &t[11..])
}

/// SillyTavern's `humanizedDateTime`: `2026-11-15@14h03m09s000ms`.
fn humanized(seconds: i64, tz: i32) -> String {
    let t = fmt_ts(seconds * 1000, tz);
    format!("{}@{}h{}m{}s{}ms", &t[..10], &t[11..13], &t[14..16], &t[17..19], &t[20..23])
}

/// A name inside `**…**` must not end the emphasis early.
fn md_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '#') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The chat in one of the formats. `tz`: minutes east of UTC for the dates in the head.
pub(crate) fn render(t: &Transcript, format: Format, tz: i32) -> String {
    match format {
        Format::Markdown => markdown(t, tz),
        Format::Text => text(t, tz),
        Format::SillyTavern => sillytavern(t, tz),
    }
}

fn markdown(t: &Transcript, tz: i32) -> String {
    let mut out = format!("# {}", md_escape(&t.character));
    if !t.title.is_empty() {
        out += &format!(" · {}", md_escape(&t.title));
    }
    out += &format!(
        "\n\n- **Figur:** {}\n- **Persona:** {}\n- **Datum:** {}\n- **Nachrichten:** {}\n\n---\n",
        md_escape(&t.character),
        md_escape(&t.persona),
        local_minute(t.created, tz),
        t.lines.len()
    );
    for line in &t.lines {
        out += &format!("\n**{}**\n\n{}\n", md_escape(t.name_of(line)), line.text.trim_end());
    }
    out
}

fn text(t: &Transcript, tz: i32) -> String {
    let mut out = t.character.clone();
    if !t.title.is_empty() {
        out += &format!(" · {}", t.title);
    }
    out += &format!("\nFigur: {}\nPersona: {}\nDatum: {}\n", t.character, t.persona, local_minute(t.created, tz));
    for line in &t.lines {
        out += &format!("\n{}:\n{}\n", t.name_of(line), line.text.trim_end());
    }
    out
}

/// SillyTavern's chat file: a head object, then one object per message, one per line and without
/// a line break at the end (as SillyTavern saves them). The names in the head are only read by
/// older versions; `chat_metadata` stays empty so SillyTavern gives the chat its own `integrity`.
fn sillytavern(t: &Transcript, tz: i32) -> String {
    let head = json!({
        "user_name": t.persona,
        "character_name": t.character,
        "create_date": humanized(t.created, tz),
        "chat_metadata": {},
    });
    let mut lines = vec![head.to_string()];
    for line in &t.lines {
        let msg: Value = json!({
            "name": t.name_of(line),
            "is_user": line.is_user,
            "is_system": false,
            "send_date": iso_utc(line.sent),
            "mes": line.text,
            "extra": {},
        });
        lines.push(msg.to_string());
    }
    lines.join("\n")
}

/// Longest file name stem (characters), so the whole path stays short on every system.
const NAME_MAX: usize = 60;

/// A file name part that is safe on Windows, macOS, Linux and phones: letters (any script) and
/// digits stay, so do space, `-`, `_`, `.`, `(`, `)` and `,`; everything else (path separators,
/// `:*?"<>|`, control characters) becomes `_`. No dots or spaces at the ends, no Windows device
/// names, never empty.
pub(crate) fn safe_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let ok = c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')' | ',');
        let c = if c.is_whitespace() { ' ' } else if ok { c } else { '_' };
        // Runs of `_` or spaces become one.
        if (c == '_' || c == ' ') && out.ends_with(c) {
            continue;
        }
        out.push(c);
    }
    let mut out: String = out.trim_matches(|c: char| c == ' ' || c == '.' || c == '_').chars().take(NAME_MAX).collect();
    out = out.trim_end_matches([' ', '.']).to_string();
    if out.is_empty() {
        return "Chat".into();
    }
    let stem = out.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if device {
        out.insert(0, '_');
    }
    out
}

/// `Mara Voss - Romanze - 2026-11-15-1403.md`: character, title (if any) and when the chat began.
pub(crate) fn file_name(t: &Transcript, format: Format, tz: i32) -> String {
    let mut stem = t.character.clone();
    if !t.title.is_empty() {
        stem += &format!(" - {}", t.title);
    }
    let stamp = crate::logs::file_stamp(t.created * 1000, tz);
    format!("{} - {stamp}.{}", safe_name(&stem), format.ext())
}

/// The value of `Content-Disposition` for a download: a plain ASCII name for old clients and the
/// exact UTF-8 name (RFC 6266 / 5987) for everyone else.
pub(crate) fn disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .flat_map(|c| match c {
            'ä' => "ae".chars().collect::<Vec<_>>(),
            'ö' => "oe".chars().collect(),
            'ü' => "ue".chars().collect(),
            'Ä' => "Ae".chars().collect(),
            'Ö' => "Oe".chars().collect(),
            'Ü' => "Ue".chars().collect(),
            'ß' => "ss".chars().collect(),
            c if c.is_ascii() && c != '"' && c != '\\' && !c.is_ascii_control() => vec![c],
            _ => vec!['_'],
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            encoded.push(b as char);
        } else {
            encoded += &format!("%{b:02X}");
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// Most chats one ZIP may hold.
pub(crate) const ZIP_MAX: usize = 500;

/// Several chats in one ZIP; equal file names get ` (2)`, ` (3)` … `now`: Unix seconds, the time
/// stamp of the entries.
pub(crate) fn archive(chats: &[Transcript], format: Format, tz: i32, now: i64) -> Res<Vec<u8>> {
    let t = fmt_ts(now * 1000, tz);
    let num = |a: usize, b: usize| t[a..b].parse::<u16>().unwrap_or(0);
    let stamp = zip::DateTime::from_date_and_time(num(0, 4), num(5, 7) as u8, num(8, 10) as u8, num(11, 13) as u8, num(14, 16) as u8, num(17, 19) as u8)
        .unwrap_or_default();
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).last_modified_time(stamp);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let mut used: Vec<String> = Vec::new();
    for chat in chats {
        let name = file_name(chat, format, tz);
        let mut unique = name.clone();
        let mut n = 2;
        while used.iter().any(|u| u.eq_ignore_ascii_case(&unique)) {
            let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
            unique = format!("{stem} ({n}).{ext}");
            n += 1;
        }
        zip.start_file(unique.as_str(), options)?;
        zip.write_all(render(chat, format, tz).as_bytes())?;
        used.push(unique);
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    use axum::http::StatusCode;

    use super::*;
    use crate::store::tests::{sample, temp_store};
    use crate::store::PersonaEdit;
    use crate::{start, Config, WebUi};

    /// A chat as SillyTavern reads it back when importing a `.jsonl` (`src/endpoints/chats.js`,
    /// `/import`, and `getChat` in `public/script.js`): the first line is the head and must carry
    /// `user_name`, `name` or `chat_metadata`; every further non-empty line is one message.
    struct StChat {
        head: Value,
        messages: Vec<Value>,
    }

    fn read_st(data: &str) -> Result<StChat, String> {
        let mut lines = data.split('\n');
        let head: Value = serde_json::from_str(lines.next().unwrap_or("")).map_err(|e| format!("head: {e}"))?;
        if head.get("user_name").is_none() && head.get("name").is_none() && head.get("chat_metadata").is_none() {
            return Err("Incorrect chat format .jsonl".into());
        }
        let mut messages = Vec::new();
        for (i, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let m: Value = serde_json::from_str(line).map_err(|e| format!("line {}: {e}", i + 2))?;
            // The fields SillyTavern's ChatMessage uses, with their types.
            if !m["name"].is_string() || !m["mes"].is_string() || !m["is_user"].is_boolean() || !m["is_system"].is_boolean() || !m["extra"].is_object() {
                return Err(format!("line {}: not a chat message", i + 2));
            }
            let date = m["send_date"].as_str().ok_or("send_date")?;
            // `new Date(send_date)` must understand it: ISO 8601 in UTC.
            if date.len() != 24 || &date[10..11] != "T" || !date.ends_with('Z') {
                return Err(format!("line {}: send_date {date}", i + 2));
            }
            messages.push(m);
        }
        Ok(StChat { head, messages })
    }

    fn sample_chat() -> Transcript {
        let line = |is_user: bool, name: Option<&str>, text: &str, sent: i64| Line { is_user, name: name.map(Into::into), text: text.into(), sent };
        Transcript {
            character: "Kapitän Ilse".into(),
            persona: "Lukas".into(),
            title: "Sturm".into(),
            created: 1_763_215_380, // 2025-11-15 14:03 UTC
            lines: vec![
                line(false, None, "*Der Wind dreht.* Willkommen an Bord, Lukas.", 1_763_215_380),
                line(true, Some("Bea"), "Danke!\n\n\"Wohin geht es?\"", 1_763_215_441),
                line(true, None, "Ich halte mich fest.", 1_763_215_500),
                line(false, None, "Nach Norden. {\"json\": true}\nZeile zwei", 1_763_215_560),
            ],
        }
    }

    #[test]
    fn sillytavern_round_trip_keeps_count_order_text_and_sides() {
        let chat = sample_chat();
        let file = render(&chat, Format::SillyTavern, 60);
        assert!(!file.ends_with('\n'));
        let back = read_st(&file).unwrap();
        assert_eq!(back.head["user_name"], "Lukas");
        assert_eq!(back.head["character_name"], "Kapitän Ilse");
        assert_eq!(back.head["create_date"], "2025-11-15@15h03m00s000ms");
        assert_eq!(back.head["chat_metadata"], json!({}));
        assert_eq!(back.messages.len(), chat.lines.len());
        for (m, l) in back.messages.iter().zip(&chat.lines) {
            assert_eq!(m["mes"].as_str(), Some(l.text.as_str()));
            assert_eq!(m["is_user"].as_bool(), Some(l.is_user));
            assert_eq!(m["is_system"].as_bool(), Some(false));
        }
        let names: Vec<&str> = back.messages.iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Kapitän Ilse", "Bea", "Lukas", "Kapitän Ilse"]);
        assert_eq!(back.messages[1]["send_date"], "2025-11-15T14:04:01.000Z");
    }

    #[test]
    fn markdown_and_text_carry_head_and_every_message() {
        let chat = sample_chat();
        let md = render(&chat, Format::Markdown, 60);
        assert!(md.starts_with("# Kapitän Ilse · Sturm\n"));
        assert!(md.contains("- **Figur:** Kapitän Ilse\n- **Persona:** Lukas\n- **Datum:** 2025-11-15 15:03\n"));
        assert!(md.contains("\n**Bea**\n\nDanke!\n\n\"Wohin geht es?\"\n"));
        assert_eq!(md.matches("\n**Kapitän Ilse**\n").count(), 2);
        let txt = render(&chat, Format::Text, 0);
        assert!(txt.starts_with("Kapitän Ilse · Sturm\nFigur: Kapitän Ilse\nPersona: Lukas\nDatum: 2025-11-15 14:03\n"));
        assert!(txt.contains("\nLukas:\nIch halte mich fest.\n"));
        assert!(!txt.contains("**"));
        assert_eq!(md_escape("*a_b*"), "\\*a\\_b\\*");
    }

    #[test]
    fn download_names_are_safe() {
        for (raw, want) in [
            ("Mara Voss", "Mara Voss"),
            ("../../etc/passwd", "etc_passwd"),
            ("..\\..\\Windows\\System32", "Windows_System32"),
            ("C:\\Users\\x", "C_Users_x"),
            ("a/b", "a_b"),
            ("Wer? *Ich* <du> | \"wir\"", "Wer_ _Ich_ _du_ _ _wir"),
            ("Zeile\r\nzwei\u{0}", "Zeile zwei"),
            ("...", "Chat"),
            ("", "Chat"),
            ("   ", "Chat"),
            ("CON", "_CON"),
            ("nul.txt", "_nul.txt"),
            ("com1", "_com1"),
            ("Console", "Console"),
            ("Kapitän Ilse Brandt", "Kapitän Ilse Brandt"),
            ("名前", "名前"),
            ("ends with dot.", "ends with dot"),
        ] {
            let got = safe_name(raw);
            assert_eq!(got, want, "{raw:?}");
            assert!(!got.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) && !got.chars().any(char::is_control), "{raw:?}");
        }
        assert_eq!(safe_name(&"x".repeat(500)).chars().count(), NAME_MAX);

        let mut chat = sample_chat();
        chat.character = "../Mara:Voss".into();
        assert_eq!(file_name(&chat, Format::Markdown, 0), "Mara_Voss - Sturm - 2025-11-15-1403.md");
        chat.title.clear();
        assert_eq!(file_name(&chat, Format::SillyTavern, 60), "Mara_Voss - 2025-11-15-1503.jsonl");

        let d = disposition("Kapitän Ilse - 2025-11-15-1403.md");
        assert_eq!(d, "attachment; filename=\"Kapitaen Ilse - 2025-11-15-1403.md\"; filename*=UTF-8''Kapit%C3%A4n%20Ilse%20-%202025-11-15-1403.md");
    }

    #[test]
    fn zip_names_stay_unique() {
        let chat = sample_chat();
        let bytes = archive(&[chat.clone(), chat.clone(), chat], Format::Text, 0, 1_763_215_380).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names: Vec<String> = archive.file_names().map(String::from).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            ["Kapitän Ilse - Sturm - 2025-11-15-1403 (2).txt", "Kapitän Ilse - Sturm - 2025-11-15-1403 (3).txt", "Kapitän Ilse - Sturm - 2025-11-15-1403.txt"]
        );
        let mut text = String::new();
        archive.by_index(0).unwrap().read_to_string(&mut text).unwrap();
        assert!(text.contains("Ich halte mich fest."));
    }

    /// Through the real server: only active messages in order, the persona each message was
    /// written as, the chat's own card name, the file name, the ZIP, and the errors.
    #[tokio::test]
    async fn export_over_http() {
        let (mut s, dir) = temp_store();
        let character = s.add_character(&sample(r#"{"name":"Mara Voss","first_mes":"Hallo."}"#), None).unwrap().id;
        let lukas = s.active_persona_id().unwrap();
        let bea = s.add_persona(&PersonaEdit::name("Bea")).unwrap().id;
        let chat = s.create_chat(character, Some("Hallo.")).unwrap();
        s.add_user_message(chat, "Ich bin Bea.", bea).unwrap();
        s.add_message(chat, "assistant", "Schön.").unwrap();
        let gone = s.add_user_message(chat, "Verworfen", lukas).unwrap().id;
        s.rewind(chat, gone).unwrap();
        let other = s.create_chat(character, Some("Hallo.")).unwrap();
        s.set_chat_title(other, "Zweiter/Teil").unwrap();
        let mut card = s.character(character).unwrap().unwrap().card;
        card.name = "Mara (eigene)".into();
        s.set_chat_card(other, Some(&card)).unwrap();
        drop(s);

        std::fs::create_dir_all(dir.join("web")).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.clone(),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}/api", running.addr.port());
        let c = reqwest::Client::new();
        let auth = format!("Bearer {}", running.token);

        let res = c.get(format!("{base}/chats/{chat}/export?format=jsonl&tz=60")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let cd = res.headers()["content-disposition"].to_str().unwrap().to_string();
        assert!(cd.starts_with("attachment; filename=\"Mara Voss - ") && cd.ends_with(".jsonl"), "{cd}");
        assert_eq!(res.headers()["cache-control"], "no-store");
        let back = read_st(&res.text().await.unwrap()).unwrap();
        let got: Vec<(&str, bool, &str)> =
            back.messages.iter().map(|m| (m["name"].as_str().unwrap(), m["is_user"].as_bool().unwrap(), m["mes"].as_str().unwrap())).collect();
        assert_eq!(got, [("Mara Voss", false, "Hallo."), ("Bea", true, "Ich bin Bea."), ("Mara Voss", false, "Schön.")]);
        assert_eq!(back.head["user_name"], "User");

        let md = c.get(format!("{base}/chats/{other}/export?format=md")).header("authorization", &auth).send().await.unwrap();
        assert_eq!(md.headers()["content-type"], "text/markdown; charset=utf-8");
        let cd = md.headers()["content-disposition"].to_str().unwrap().to_string();
        assert!(cd.contains("filename=\"Mara (eigene) - Zweiter_Teil - "), "{cd}");
        assert!(md.text().await.unwrap().starts_with("# Mara (eigene) · Zweiter/Teil\n"));

        let zip = c.post(format!("{base}/chats/export")).header("authorization", &auth).json(&json!({ "ids": [chat, other, chat], "format": "txt" })).send().await.unwrap();
        assert_eq!(zip.status(), StatusCode::OK);
        assert_eq!(zip.headers()["content-type"], "application/zip");
        let mut archive = zip::ZipArchive::new(Cursor::new(zip.bytes().await.unwrap().to_vec())).unwrap();
        assert_eq!(archive.len(), 2);
        let mut first = String::new();
        archive.by_index(0).unwrap().read_to_string(&mut first).unwrap();
        assert!(first.contains("\nBea:\nIch bin Bea.\n") && !first.contains("Verworfen"));

        for (q, status) in [(format!("/chats/{chat}/export?format=pdf"), StatusCode::BAD_REQUEST), ("/chats/999/export?format=md".into(), StatusCode::NOT_FOUND)] {
            assert_eq!(c.get(format!("{base}{q}")).header("authorization", &auth).send().await.unwrap().status(), status, "{q}");
        }
        let none = c.post(format!("{base}/chats/export")).header("authorization", &auth).json(&json!({ "ids": [], "format": "md" })).send().await.unwrap();
        assert_eq!(none.status(), StatusCode::BAD_REQUEST);
        let missing = c.post(format!("{base}/chats/export")).header("authorization", &auth).json(&json!({ "ids": [chat, 999], "format": "md" })).send().await.unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        // The token never travels in the address for these, but the header is required.
        let anon = reqwest::get(format!("{base}/chats/{chat}/export?format=md")).await.unwrap();
        assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);

        running.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
