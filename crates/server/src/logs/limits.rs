//! Keeps the log within its limits (age, count, size) and gives freed space back to the disk.

use std::path::Path;

use rusqlite::{params, Connection};

use super::{Res, KEEP_DAYS, KEEP_MAX, LOG_FILE, MAX_BYTES};

/// What the log database and its write-ahead file take on disk, in bytes.
pub fn disk_bytes(dir: &Path) -> u64 {
    [LOG_FILE, "logs.db-wal", "logs.db-shm"]
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
        .map(|m| m.len())
        .sum()
}

/// Size of the database itself (pages in use and free), from SQLite.
pub(super) fn db_bytes(conn: &Connection) -> Res<u64> {
    let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    Ok((pages * size) as u64)
}

/// Hands freed pages back to the disk and empties the write-ahead file.
fn release(conn: &Connection) -> Res<()> {
    // Frees one page per step, so it has to be stepped to the end.
    let mut st = conn.prepare("PRAGMA incremental_vacuum")?;
    let mut rows = st.query([])?;
    while rows.next()?.is_some() {}
    drop(rows);
    drop(st);
    // Busy (a reader is in the middle of a page) is fine: the next round gets it.
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    Ok(())
}

/// Deletes entries older than `KEEP_DAYS`, all but the newest `KEEP_MAX`, and then the oldest
/// quarter at a time while the file is above `MAX_BYTES`; the space goes back to the disk.
/// Answers how many entries went.
pub fn prune(conn: &mut Connection, now: i64) -> Res<usize> {
    let tx = conn.transaction()?;
    let mut n = tx.execute("DELETE FROM logs WHERE ts < ?1", params![now - KEEP_DAYS * 86_400_000])?;
    n += tx.execute(
        "DELETE FROM logs WHERE id <= (SELECT id FROM logs ORDER BY id DESC LIMIT 1 OFFSET ?1)",
        params![KEEP_MAX],
    )?;
    tx.commit()?;
    release(conn)?;
    while db_bytes(conn)? > MAX_BYTES {
        let gone = conn.execute(
            "DELETE FROM logs WHERE id IN (SELECT id FROM logs ORDER BY id LIMIT max(1, (SELECT COUNT(*) FROM logs) / 4))",
            [],
        )?;
        release(conn)?;
        if gone == 0 {
            break;
        }
        n += gone;
    }
    Ok(n)
}

/// Deletes every entry and gives the space back.
pub fn clear(conn: &Connection) -> Res<usize> {
    let n = conn.execute("DELETE FROM logs", [])?;
    release(conn)?;
    Ok(n)
}

/// Writes entries as they are, one row each (no merging, no pruning).
#[cfg(test)]
pub(super) fn insert(conn: &mut Connection, batch: &[super::Entry]) -> Res<()> {
    use serde_json::Value;
    let tx = conn.transaction()?;
    {
        let mut st = tx.prepare_cached(
            "INSERT INTO logs (ts, level, source, category, message, data, chat_id, character_id, device)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for e in batch {
            st.execute(params![
                e.ts,
                e.level,
                e.source,
                e.category,
                e.message,
                e.data.as_ref().map(Value::to_string),
                e.chat_id,
                e.character_id,
                e.device
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}
