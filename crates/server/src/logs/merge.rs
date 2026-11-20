//! The writer's side: writes a batch in one transaction, merges an entry that repeats the one
//! before it into a counter, and prunes from time to time.

use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use serde_json::Value;

use super::limits::{db_bytes, prune};
use super::{now_ms, Entry, Res, MAX_BYTES, MERGE_MS};

/// The writer also prunes after this many entries or this much time.
const PRUNE_EVERY: usize = 500;
const PRUNE_INTERVAL: Duration = Duration::from_secs(300);

/// What the writer remembers about the entry it wrote last, to merge repeats into it.
struct Last {
    id: i64,
    key: String,
    ts: i64,
}

/// Two entries merge when everything but the time (and an HTTP duration) is the same.
fn merge_key(e: &Entry) -> String {
    let data = e.data.as_ref().map(|d| {
        let mut d = d.clone();
        if let Some(o) = d.as_object_mut() {
            o.remove("ms");
        }
        d
    });
    serde_json::json!([e.level, e.source, e.category, e.message, e.device, e.chat_id, e.character_id, data]).to_string()
}

/// The writer's connection with what it remembers between batches.
pub(super) struct Sink {
    pub(super) conn: Connection,
    last: Option<Last>,
    since_prune: usize,
    pruned_at: Instant,
}

impl Sink {
    pub(super) fn new(conn: Connection) -> Sink {
        Sink { conn, last: None, since_prune: 0, pruned_at: Instant::now() }
    }

    pub(super) fn write(&mut self, batch: &[Entry]) -> Res<()> {
        let Sink { conn, last, .. } = self;
        let tx = conn.transaction()?;
        {
            let mut merge = tx.prepare_cached(
                "UPDATE logs SET repeat = repeat + ?1, last_ts = max(COALESCE(last_ts, ts), ?2) WHERE id = ?3",
            )?;
            let mut add = tx.prepare_cached(
                "INSERT INTO logs (ts, level, source, category, message, data, chat_id, character_id, device, repeat, last_ts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for e in batch {
                let key = merge_key(e);
                let end = e.last_ts.unwrap_or(e.ts).max(e.ts);
                if let Some(l) = last.as_mut() {
                    // A row that is gone (the log was cleared meanwhile) takes the entry as new.
                    if l.key == key && (e.ts - l.ts).abs() <= MERGE_MS && merge.execute(params![e.repeat.max(1), end, l.id])? == 1 {
                        l.ts = l.ts.max(end);
                        continue;
                    }
                }
                add.execute(params![
                    e.ts,
                    e.level,
                    e.source,
                    e.category,
                    e.message,
                    e.data.as_ref().map(Value::to_string),
                    e.chat_id,
                    e.character_id,
                    e.device,
                    e.repeat.max(1),
                    e.last_ts
                ])?;
                *last = Some(Last { id: tx.last_insert_rowid(), key, ts: end });
            }
        }
        tx.commit()?;
        self.since_prune += batch.len();
        if self.since_prune >= PRUNE_EVERY || self.pruned_at.elapsed() >= PRUNE_INTERVAL || db_bytes(&self.conn)? > MAX_BYTES {
            self.since_prune = 0;
            self.pruned_at = Instant::now();
            prune(&mut self.conn, now_ms())?;
        }
        Ok(())
    }
}
