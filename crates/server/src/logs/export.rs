//! The export: the whole log as a readable text file to send in with a bug report, with a head
//! about the setup. Paths under the home folder are shortened and the token never appears.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::read::{read_entry, COLS};
use super::{disk_bytes, now_ms, Entry, Res, MAX_DISK_BYTES};
use crate::store;

/// `2026-11-13 14:03:22.123` for Unix milliseconds, shifted by `tz_min` minutes east of UTC.
pub fn fmt_ts(ms: i64, tz_min: i32) -> String {
    let ms = ms + tz_min as i64 * 60_000;
    let (day, in_day) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (y, m, d) = civil(day);
    let s = in_day / 1000;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{:03}", s / 3600, s / 60 % 60, s % 60, in_day % 1000)
}

/// `2026-11-13-1403` for file names.
pub fn file_stamp(ms: i64, tz_min: i32) -> String {
    let t = fmt_ts(ms, tz_min);
    format!("{}-{}{}", &t[..10], &t[11..13], &t[14..16])
}

/// Days since 1970-01-01 to (year, month, day) — H. Hinnant's civil_from_days.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub(super) fn tz_label(tz_min: i32) -> String {
    let sign = if tz_min < 0 { '-' } else { '+' };
    format!("UTC{sign}{:02}:{:02}", tz_min.abs() / 60, tz_min.abs() % 60)
}

/// Replaces `needle` ignoring ASCII case (byte lengths stay the same, so indexes line up).
fn replace_ci(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let hay = text.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (i, _) in hay.match_indices(&needle) {
        out.push_str(&text[last..i]);
        out.push_str(with);
        last = i + needle.len();
    }
    out.push_str(&text[last..]);
    out
}

/// The user's home folder becomes `~` in every spelling a path takes in the log: with `\`,
/// with `/`, and JSON-escaped (`\\`).
pub fn shorten_paths(text: &str, home: &str) -> String {
    let home = home.trim_end_matches(['\\', '/']);
    if home.len() < 3 {
        return text.to_string();
    }
    let plain = home.replace('/', "\\");
    let mut out = replace_ci(text, &plain.replace('\\', "\\\\"), "~");
    out = replace_ci(&out, &plain, "~");
    replace_ci(&out, &plain.replace('\\', "/"), "~")
}

/// Last `n` lines of a text file (reads at most the last 256 KB). None: no such file.
pub fn tail(path: &Path, n: usize) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(256 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = vec![];
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(n);
    Some(lines[from..].join("\n"))
}

/// Windows version as `ver` prints it (other systems: the OS name).
pub fn os_version() -> String {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("cmd").args(["/C", "ver"]).creation_flags(CREATE_NO_WINDOW).output();
        if let Ok(out) = out {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
        "Windows (Version unbekannt)".into()
    }
    #[cfg(not(windows))]
    {
        std::env::consts::OS.into()
    }
}

/// The user's home folder (`USERPROFILE`, else `HOME`).
pub fn home_dir() -> Option<String> {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()
}

/// What the head of an export says about the setup, read from the data folder.
pub fn setup_lines(dir: &Path) -> Vec<(String, String)> {
    let mut lines = vec![
        ("App-Version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
        ("System".into(), os_version()),
    ];
    let version: i64 = Connection::open_with_flags(dir.join(store::DB_FILE), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
        .unwrap_or(-1);
    lines.push(("Datenbank".into(), format!("Schema {version} von {}", store::schema_target())));
    lines.push(("Log-Speicher".into(), format!("{} KB (Grenze {} MB)", disk_bytes(dir) / 1024, MAX_DISK_BYTES / 1024 / 1024)));
    lines.push(("Datenordner".into(), dir.display().to_string()));
    match std::fs::read_to_string(dir.join("engine.json")) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(cfg)) => {
                let model = cfg.get("model").and_then(Value::as_str).map(|m| {
                    m.rsplit(['\\', '/']).next().unwrap_or(m).to_string()
                });
                lines.push(("Modell".into(), model.unwrap_or_else(|| "(nicht eingetragen)".into())));
                for (key, value) in cfg {
                    let shown = match value {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    lines.push((format!("engine.json {key}"), shown));
                }
            }
            Ok(_) => lines.push(("engine.json".into(), "(kein JSON-Objekt)".into())),
            Err(e) => lines.push(("engine.json".into(), format!("(nicht lesbar: {e})"))),
        },
        Err(_) => lines.push(("engine.json".into(), "(nicht vorhanden)".into())),
    }
    lines
}

/// One readable line per entry.
fn entry_line(e: &Entry, tz_min: i32) -> String {
    let mut line = format!(
        "{}  {:<5}  {:<20}  {:<18}  {}",
        fmt_ts(e.ts, tz_min),
        e.level.to_uppercase(),
        format!("{}/{}", e.source, e.category),
        if e.device.is_empty() { "-" } else { &e.device },
        e.message
    );
    if let (true, Some(t)) = (e.repeat > 1, e.last_ts) {
        line.push_str(&format!("  bis {}", &fmt_ts(t, tz_min)[11..]));
    }
    if let Some(c) = e.chat_id {
        line.push_str(&format!("  chat={c}"));
    }
    if let Some(c) = e.character_id {
        line.push_str(&format!("  figur={c}"));
    }
    if let Some(d) = &e.data {
        line.push_str("  ");
        line.push_str(&d.to_string());
    }
    line
}

/// The whole export as text: a head about the setup, every entry oldest first, then the tail of
/// the model server's logs. Paths under `home` are shortened and `token` never appears.
pub fn export(
    conn: &Connection,
    head: &[(String, String)],
    llama_logs: &[(String, Option<String>)],
    tz_min: i32,
    home: Option<&str>,
    token: &str,
) -> Res<String> {
    let mut out = String::from("Lorelamp Aktivitäts-Log\n=======================\n");
    out.push_str(&format!("{:<22} {} ({})\n", "Exportiert", fmt_ts(now_ms(), tz_min), tz_label(tz_min)));
    for (k, v) in head {
        out.push_str(&format!("{:<22} {v}\n", k));
    }
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0))?;
    out.push_str(&format!("{:<22} {n} (älteste zuerst)\n\n", "Einträge"));
    out.push_str("Zeit                     Stufe  Quelle/Kategorie      Gerät               Text\n");
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM logs ORDER BY ts, id"))?;
    for e in st.query_map([], read_entry)? {
        out.push_str(&entry_line(&e?, tz_min));
        out.push('\n');
    }
    for (title, text) in llama_logs {
        out.push_str(&format!("\n--- {title} ---\n"));
        out.push_str(text.as_deref().unwrap_or("(nicht vorhanden)"));
        out.push('\n');
    }
    if let Some(home) = home {
        out = shorten_paths(&out, home);
    }
    if token.len() >= 8 {
        out = out.replace(token, "<token>");
    }
    Ok(out)
}
