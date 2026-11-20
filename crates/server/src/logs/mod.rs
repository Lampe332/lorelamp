//! Activity log: everything that happens in the app, with a millisecond timestamp, so that the
//! owner (or a user who sends in an exported file) can see what went wrong.
//!
//! Writing never slows the app down and never fails the caller: entries go through a bounded
//! queue to a writer thread with its own database connection; a full queue or a failed write
//! drops them. Reading, clearing and exporting open their own connection too, so none of it
//! waits for the chat hub.
//!
//! Privacy (fixed): entries carry ids, counts, sizes and timings — never message or card text,
//! never what was typed into a field, never the API token. The export also replaces the user's
//! home folder in paths with `~` and blanks the token should it appear anywhere.
//!
//! Size (fixed): the log lives in its own file (`logs.db`) next to the real data, so chats and
//! cards never grow or slow down because of it. The writer merges an entry that repeats the one
//! before it within `MERGE_MS` into a counter, and keeps the file within `KEEP_DAYS`, `KEEP_MAX`
//! and `MAX_BYTES` while the app runs; freed pages go back to the disk (incremental vacuum).
//!
//! Files: this one holds the entry type, the logger and the writer thread; `merge` writes
//! batches and merges repeats, `limits` keeps the file small, `read` answers the Logs tab and
//! `export` builds the text file for bug reports.

mod export;
mod limits;
mod merge;
mod read;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;

use crate::store::Res;
use merge::Sink;

pub use export::{export, file_stamp, fmt_ts, home_dir, os_version, setup_lines, shorten_paths, tail};
pub use limits::{clear, disk_bytes, prune};
pub use read::{query, Page, Query};

/// The log's own database file inside the data folder.
pub const LOG_FILE: &str = "logs.db";
/// Entries older than this are deleted.
pub const KEEP_DAYS: i64 = 7;
/// At most this many entries are kept (the oldest go first).
pub const KEEP_MAX: i64 = 20_000;
/// Largest the log database may get; the oldest entries go first. One write batch (≤ 4 MB) and
/// the write-ahead file (held to `WAL_BYTES` between batches) come on top: never above 14 MB.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// What the log may take on disk at most, all files together.
pub const MAX_DISK_BYTES: u64 = 14 * 1024 * 1024;
const WAL_BYTES: u64 = 2 * 1024 * 1024;
/// An entry equal to the one before it, at most this much later, raises its counter instead.
pub const MERGE_MS: i64 = 60_000;
/// Largest page one request may ask for.
pub const PAGE_MAX: i64 = 500;
/// Entries waiting for the writer; more are dropped rather than holding anyone up.
const QUEUE: usize = 10_000;
/// Entries written in one transaction.
const BATCH: usize = 500;
const MAX_MESSAGE: usize = 500;
const MAX_DATA: usize = 8_000;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }

    /// Unknown names count as info.
    pub fn parse(s: &str) -> Level {
        match s {
            "debug" => Level::Debug,
            "warn" => Level::Warn,
            "error" => Level::Error,
            _ => Level::Info,
        }
    }
}

/// One log entry. `source` is where it happened: ui, server, chat or engine.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Entry {
    pub id: i64,
    /// Unix milliseconds.
    pub ts: i64,
    pub level: String,
    pub source: String,
    pub category: String,
    /// Short and readable; English or German.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    pub chat_id: Option<i64>,
    pub character_id: Option<i64>,
    /// "PC" or "WLAN <ip>", plus the browser's short id; empty for the app's own events.
    pub device: String,
    /// How often it happened in a row (merged repeats); 1 = once.
    #[serde(skip_serializing_if = "is_once")]
    pub repeat: i64,
    /// Unix milliseconds of the last repeat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ts: Option<i64>,
}

fn is_once(n: &i64) -> bool {
    *n <= 1
}

impl Entry {
    pub fn new(level: Level, source: &str, category: &str, message: impl Into<String>) -> Entry {
        Entry {
            id: 0,
            ts: now_ms(),
            level: level.as_str().into(),
            source: source.into(),
            category: category.into(),
            message: message.into(),
            data: None,
            chat_id: None,
            character_id: None,
            device: String::new(),
            repeat: 1,
            last_ts: None,
        }
    }
    pub fn data(mut self, data: Value) -> Entry {
        self.data = Some(data);
        self
    }
    pub fn chat(mut self, id: i64) -> Entry {
        self.chat_id = Some(id);
        self
    }
    pub fn character(mut self, id: i64) -> Entry {
        self.character_id = Some(id);
        self
    }
    pub fn device(mut self, device: impl Into<String>) -> Entry {
        self.device = device.into();
        self
    }
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

enum Cmd {
    Write(Box<Entry>),
    Flush(mpsc::Sender<()>),
}

/// Cheap to clone; every clone feeds the same writer.
#[derive(Clone)]
pub struct Logger {
    tx: Option<SyncSender<Cmd>>,
}

impl std::fmt::Debug for Logger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.tx.is_some() { "Logger" } else { "Logger(off)" })
    }
}

impl Logger {
    /// Opens the log database in `dir` (creating it), deletes entries past the limits and
    /// starts the writer. If the database cannot be opened, the logger stays silent: the app
    /// itself must not fail because of its log.
    pub fn start(dir: &Path) -> Logger {
        let mut conn = match open(dir) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("activity log off: {e}");
                return Logger { tx: None };
            }
        };
        let pruned = prune(&mut conn, now_ms()).unwrap_or_else(|e| {
            eprintln!("activity log prune: {e}");
            0
        });
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let sink = Sink::new(conn);
        if std::thread::Builder::new().name("activity-log".into()).spawn(move || writer(sink, rx)).is_err() {
            return Logger { tx: None };
        }
        let log = Logger { tx: Some(tx) };
        if pruned > 0 {
            log.write(
                Entry::new(Level::Debug, "server", "logs", format!("removed {pruned} old log entries"))
                    .data(serde_json::json!({ "removed": pruned, "keep_days": KEEP_DAYS, "keep_max": KEEP_MAX })),
            );
        }
        log
    }

    /// Queues an entry. Never blocks; drops it when the queue is full or the log is off.
    pub fn write(&self, mut e: Entry) {
        let Some(tx) = &self.tx else { return };
        e.message = cut(&e.message, MAX_MESSAGE);
        if let Some(d) = &e.data {
            let text = d.to_string();
            if text.len() > MAX_DATA {
                e.data = Some(serde_json::json!({ "truncated": cut(&text, MAX_DATA) }));
            }
        }
        let _ = tx.try_send(Cmd::Write(Box::new(e)));
    }

    pub fn info(&self, source: &str, category: &str, message: impl Into<String>) {
        self.write(Entry::new(Level::Info, source, category, message));
    }

    /// Waits (at most `timeout`) until everything queued so far is in the database.
    pub fn flush(&self, timeout: Duration) {
        let Some(tx) = &self.tx else { return };
        let (ack, done) = mpsc::channel();
        if tx.try_send(Cmd::Flush(ack)).is_ok() {
            let _ = done.recv_timeout(timeout);
        }
    }
}

/// Schema of `logs.db`, applied in order like the store's. Never edit an entry that has shipped.
const MIGRATIONS: &[&str] = &["
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
  device TEXT NOT NULL DEFAULT '',
  repeat INTEGER NOT NULL DEFAULT 1,
  last_ts INTEGER
);
CREATE INDEX logs_ts ON logs(ts, id);
"];

/// Opens (and if needed creates) `logs.db` in `dir` with its schema up to date.
fn open(dir: &Path) -> Res<Connection> {
    std::fs::create_dir_all(dir)?;
    let mut conn = Connection::open(dir.join(LOG_FILE))?;
    conn.busy_timeout(Duration::from_secs(2))?;
    // auto_vacuum only takes effect before the first table exists, so it comes first.
    conn.execute_batch(&format!(
        "PRAGMA auto_vacuum = INCREMENTAL; PRAGMA journal_mode = WAL;
         PRAGMA journal_size_limit = {WAL_BYTES}; PRAGMA wal_autocheckpoint = {};",
        WAL_BYTES / 4096 / 2
    ))?;
    loop {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let done = tx.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
        let Some(sql) = MIGRATIONS.get(done) else { break };
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", done + 1))?;
        tx.commit()?;
    }
    Ok(conn)
}

/// A connection for reading, clearing and exporting (the writer keeps its own).
pub fn open_read(dir: &Path) -> Res<Connection> {
    open(dir)
}

/// The writer thread: takes up to `BATCH` queued entries at a time, writes them in one
/// transaction, then answers the flushes that were waiting for them.
fn writer(mut sink: Sink, rx: Receiver<Cmd>) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![];
        let mut acks = vec![];
        let mut next = Some(first);
        while let Some(cmd) = next.take() {
            match cmd {
                Cmd::Write(e) => batch.push(*e),
                Cmd::Flush(ack) => acks.push(ack),
            }
            if batch.len() < BATCH {
                next = rx.try_recv().ok();
            }
        }
        if !batch.is_empty() {
            if let Err(e) = sink.write(&batch) {
                eprintln!("activity log write: {e}");
            }
        }
        for ack in acks {
            let _ = ack.send(());
        }
    }
}
