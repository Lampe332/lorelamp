//! Reading the log for the Logs tab: filters, free-text search, pages and live updates.

use rusqlite::{named_params, Connection};
use serde::{Deserialize, Serialize};

use super::{Entry, Res, PAGE_MAX};

/// What the Logs tab asks for: newest first, one page at a time.
#[derive(Debug, Default, Deserialize)]
pub struct Query {
    /// all | problems | clicks | chat | model | server
    #[serde(default)]
    pub filter: String,
    /// Free text, matched case-insensitively against message, category, source, device and data.
    #[serde(default)]
    pub q: String,
    /// Older than this entry (the last one shown): "Ältere laden".
    pub before_ts: Option<i64>,
    pub before_id: Option<i64>,
    /// Written after this id: live updates.
    pub after_id: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub entries: Vec<Entry>,
    /// More entries match below the last one.
    pub has_more: bool,
    /// Newest id in the log; the next live update asks for what came after it.
    pub max_id: i64,
    /// Entries in the log / entries that match the filter and search.
    pub total: i64,
    pub matching: i64,
}

fn filter_sql(filter: &str) -> &'static str {
    match filter {
        "problems" => "level IN ('warn', 'error')",
        "clicks" => "category = 'click'",
        "chat" => "(source = 'chat' OR (source = 'ui' AND category = 'chat'))",
        "model" => "(source = 'engine' OR category IN ('generate', 'model'))",
        "server" => "source = 'server'",
        _ => "1",
    }
}

/// The columns `read_entry` reads.
pub(super) const COLS: &str = "id, ts, level, source, category, message, data, chat_id, character_id, device, repeat, last_ts";

/// A merged entry shows its count in the message (`… (×12)`), so the Logs tab and the export
/// show it without knowing about merging.
pub(super) fn read_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let data: Option<String> = r.get(6)?;
    let repeat: i64 = r.get(10)?;
    let mut message: String = r.get(5)?;
    if repeat > 1 {
        message.push_str(&format!(" (×{repeat})"));
    }
    Ok(Entry {
        id: r.get(0)?,
        ts: r.get(1)?,
        level: r.get(2)?,
        source: r.get(3)?,
        category: r.get(4)?,
        message,
        data: data.and_then(|d| serde_json::from_str(&d).ok()),
        chat_id: r.get(7)?,
        character_id: r.get(8)?,
        device: r.get(9)?,
        repeat,
        last_ts: r.get(11)?,
    })
}

pub fn query(conn: &Connection, q: &Query) -> Res<Page> {
    let limit = q.limit.unwrap_or(200).clamp(1, PAGE_MAX);
    let search = q.q.trim().to_lowercase();
    let matches = format!(
        "{} AND (:q = '' OR instr(lower(message || ' ' || category || ' ' || source || ' ' || device || ' ' || COALESCE(data, '')), :q) > 0)",
        filter_sql(&q.filter)
    );
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM logs WHERE {matches}
           AND (:bts IS NULL OR ts < :bts OR (ts = :bts AND id < :bid))
           AND (:after IS NULL OR id > :after)
         ORDER BY ts DESC, id DESC LIMIT :limit"
    ))?;
    let mut entries: Vec<Entry> = st
        .query_map(
            named_params! {
                ":q": search, ":bts": q.before_ts, ":bid": q.before_id.unwrap_or(i64::MAX),
                ":after": q.after_id, ":limit": limit + 1,
            },
            read_entry,
        )?
        .collect::<Result<_, _>>()?;
    let has_more = entries.len() as i64 > limit;
    entries.truncate(limit as usize);
    let max_id = conn.query_row("SELECT COALESCE(MAX(id), 0) FROM logs", [], |r| r.get(0))?;
    let total = conn.query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0))?;
    let matching =
        conn.query_row(&format!("SELECT COUNT(*) FROM logs WHERE {matches}"), named_params! { ":q": search }, |r| r.get(0))?;
    Ok(Page { entries, has_more, max_id, total, matching })
}
