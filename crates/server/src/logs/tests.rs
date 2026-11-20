//! Tests of the activity log: queue, merging, limits, reading and export.

use serde_json::json;

use super::export::tz_label;
use super::limits::insert;
use super::*;
use crate::store;

fn temp_dir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rp-app-logs-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn entry(ts: i64, level: Level, source: &str, category: &str, message: &str) -> Entry {
    let mut e = Entry::new(level, source, category, message);
    e.ts = ts;
    e
}

#[test]
fn writes_through_the_queue_and_reads_back() {
    let dir = temp_dir();
    let log = Logger::start(&dir);
    log.write(Entry::new(Level::Info, "ui", "click", "Klick: Start").device("PC ab12").data(json!({ "route": "#/" })));
    log.write(Entry::new(Level::Error, "chat", "generate", "generation failed").chat(3).character(7));
    log.flush(Duration::from_secs(5));

    let conn = open_read(&dir).unwrap();
    let page = query(&conn, &Query::default()).unwrap();
    assert_eq!(page.total, 2);
    let err = page.entries.iter().find(|e| e.level == "error").unwrap();
    assert_eq!((err.chat_id, err.character_id), (Some(3), Some(7)));
    let click = page.entries.iter().find(|e| e.category == "click").unwrap();
    assert_eq!(click.device, "PC ab12");
    assert_eq!(click.data, Some(json!({ "route": "#/" })));
    assert!(page.max_id >= 2);
    drop(conn);
    drop(log);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn filters_search_and_paging() {
    let dir = temp_dir();
    let mut conn = open(&dir).unwrap();
    insert(
        &mut conn,
        &[
            entry(1000, Level::Debug, "server", "http", "GET /api/library -> 200"),
            entry(2000, Level::Info, "ui", "click", "Klick: Charaktere"),
            entry(3000, Level::Warn, "ui", "api", "PUT /api/settings -> 400"),
            entry(4000, Level::Info, "chat", "generate", "generation finished"),
            entry(5000, Level::Error, "engine", "crash", "model server exited"),
            entry(6000, Level::Info, "ui", "chat", "Senden"),
        ],
    )
    .unwrap();
    fn messages(conn: &Connection, q: Query) -> Vec<String> {
        query(conn, &q).unwrap().entries.into_iter().map(|e| e.message).collect()
    }
    let msgs = |q: Query| messages(&conn, q);
    let f = |filter: &str| Query { filter: filter.into(), ..Query::default() };

    assert_eq!(msgs(f("problems")), ["model server exited", "PUT /api/settings -> 400"]);
    assert_eq!(msgs(f("clicks")), ["Klick: Charaktere"]);
    assert_eq!(msgs(f("chat")), ["Senden", "generation finished"]);
    assert_eq!(msgs(f("model")), ["model server exited", "generation finished"]);
    assert_eq!(msgs(f("server")), ["GET /api/library -> 200"]);
    assert_eq!(msgs(f("all")).len(), 6);
    assert_eq!(msgs(Query { q: "SETTINGS".into(), ..Query::default() }), ["PUT /api/settings -> 400"]);

    // Newest first, a page at a time; the next page starts below the last entry shown.
    let first = query(&conn, &Query { limit: Some(4), ..Query::default() }).unwrap();
    assert!(first.has_more);
    assert_eq!(first.matching, 6);
    let last = first.entries.last().unwrap();
    let rest = msgs(Query { limit: Some(4), before_ts: Some(last.ts), before_id: Some(last.id), ..Query::default() });
    assert_eq!(rest, ["Klick: Charaktere", "GET /api/library -> 200"]);
    // Live updates: what was written after the newest id seen.
    insert(&mut conn, &[entry(500, Level::Info, "ui", "nav", "late UI event")]).unwrap();
    assert_eq!(messages(&conn, Query { after_id: Some(first.max_id), ..Query::default() }), ["late UI event"]);

    assert_eq!(clear(&conn).unwrap(), 7);
    assert_eq!(query(&conn, &Query::default()).unwrap().total, 0);
    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
}

/// The limits: a week, 20 000 entries, 8 MB of database (14 MB on disk with batch and WAL),
/// repeats merged within a minute.
#[test]
fn limits_are_a_week_twenty_thousand_and_eight_mb() {
    assert_eq!((KEEP_DAYS, KEEP_MAX), (7, 20_000));
    assert_eq!((MAX_BYTES, MAX_DISK_BYTES), (8 * 1024 * 1024, 14 * 1024 * 1024));
    assert_eq!(MERGE_MS, 60_000);
}

/// Entries older than seven days go, six days old stay.
#[test]
fn retention_drops_what_is_older_than_a_week() {
    let dir = temp_dir();
    let mut conn = open(&dir).unwrap();
    let day = 86_400_000;
    let now = 100 * day;
    insert(
        &mut conn,
        &[
            entry(now - 8 * day, Level::Info, "ui", "click", "eight days"),
            entry(now - 7 * day - 1, Level::Info, "ui", "click", "a week and a moment"),
            entry(now - 6 * day, Level::Info, "ui", "click", "six days"),
            entry(now - 1_000, Level::Info, "ui", "click", "now"),
        ],
    )
    .unwrap();
    assert_eq!(prune(&mut conn, now).unwrap(), 2);
    let left: Vec<String> = query(&conn, &Query::default()).unwrap().entries.into_iter().map(|e| e.message).collect();
    assert_eq!(left, ["now", "six days"]);
    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn retention_keeps_seven_days_and_twenty_thousand() {
    let dir = temp_dir();
    let mut conn = open(&dir).unwrap();
    let now = 100 * 86_400_000;
    let old = now - (KEEP_DAYS * 86_400_000) - 1;
    let mut batch = vec![entry(old, Level::Info, "ui", "click", "too old")];
    batch.extend((0..KEEP_MAX + 5).map(|i| entry(now - 1000 + i, Level::Info, "ui", "click", "recent")));
    insert(&mut conn, &batch).unwrap();
    assert_eq!(prune(&mut conn, now).unwrap(), 6);
    let page = query(&conn, &Query { limit: Some(1), ..Query::default() }).unwrap();
    assert_eq!(page.total, KEEP_MAX);
    let oldest: i64 = conn.query_row("SELECT MIN(ts) FROM logs", [], |r| r.get(0)).unwrap();
    assert_eq!(oldest, now - 1000 + 5, "the oldest of the recent ones went first");
    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn timestamps_in_local_time() {
    let leap_day = 951_782_400_000; // 2000-02-29 00:00:00 UTC
    assert_eq!(fmt_ts(0, 0), "1970-01-01 00:00:00.000");
    assert_eq!(fmt_ts(leap_day + 3_723_004, 0), "2000-02-29 01:02:03.004");
    assert_eq!(fmt_ts(leap_day + 3_723_004, 60), "2000-02-29 02:02:03.004");
    assert_eq!(fmt_ts(leap_day - 1, 0), "2000-02-28 23:59:59.999");
    assert_eq!(fmt_ts(leap_day, -90), "2000-02-28 22:30:00.000");
    assert_eq!(file_stamp(leap_day + 3_723_004, 60), "2000-02-29-0202");
    assert_eq!(tz_label(60), "UTC+01:00");
    assert_eq!(tz_label(-330), "UTC-05:30");
}

#[test]
fn paths_are_shortened_in_every_spelling() {
    let home = r"C:\Users\Tester";
    let text = r#"C:\Users\Tester\AppData\Roaming\rp-app {"p":"C:\\Users\\Tester\\models\\m.gguf"} c:/users/tester/x d:\other"#;
    assert_eq!(
        shorten_paths(text, home),
        r#"~\AppData\Roaming\rp-app {"p":"~\\models\\m.gguf"} ~/x d:\other"#
    );
    assert_eq!(shorten_paths("nothing here", ""), "nothing here");
}

/// The export: setup head, every entry, the model server's log; no token, no home folder.
#[test]
fn export_hides_the_token_and_the_home_folder() {
    let dir = temp_dir();
    let mut conn = open(&dir).unwrap();
    let token = "Tok3nTok3nTok3nTok3n";
    let mut e = entry(1_700_000_000_000, Level::Error, "engine", "crash", "model server exited");
    e.data = Some(json!({ "log": r"C:\Users\Tester\AppData\Roaming\rp-app\llama-server.log", "url": format!("/?token={token}") }));
    insert(&mut conn, &[e]).unwrap();
    let head = vec![("Modell".to_string(), "orion.gguf".to_string()), ("Datenordner".into(), r"C:\Users\Tester\AppData".into())];
    let llama = vec![("llama-server.log".to_string(), Some(r"loading C:\Users\Tester\m.gguf".to_string()))];
    let text = export(&conn, &head, &llama, 60, Some(r"C:\Users\Tester"), token).unwrap();

    assert!(text.starts_with("Lorelamp Aktivitäts-Log"));
    assert!(text.contains("Modell                 orion.gguf"));
    assert!(text.contains("2023-11-14 23:13:20.000  ERROR  engine/crash"));
    assert!(text.contains(r"~\\AppData\\Roaming\\rp-app\\llama-server.log"));
    assert!(text.contains(r"loading ~\m.gguf"));
    assert!(!text.contains(token));
    assert!(!text.to_lowercase().contains(r"users\tester"));
    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tail_keeps_the_last_lines() {
    let dir = temp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.log");
    std::fs::write(&path, (1..=300).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")).unwrap();
    let t = tail(&path, 200).unwrap();
    assert_eq!(t.lines().count(), 200);
    assert!(t.starts_with("line 101") && t.ends_with("line 300"));
    assert!(tail(&dir.join("missing.log"), 5).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Repeats within `MERGE_MS` become one row with a counter; anything different starts a new one.
#[test]
fn repeats_are_merged_into_a_counter() {
    let dir = temp_dir();
    let mut sink = Sink::new(open(&dir).unwrap());
    let click = |ts: i64| entry(ts, Level::Info, "ui", "click", "Klick: „Senden“").device("PC ab12");
    let http = |ts: i64, ms: u64| {
        entry(ts, Level::Warn, "server", "http", "GET /api/x -> 404").data(json!({ "path": "/api/x", "status": 404, "ms": ms }))
    };
    let mut from_ui = click(4_000);
    from_ui.repeat = 3; // the browser merged three already
    from_ui.last_ts = Some(4_500);
    sink.write(&[click(1_000), click(1_200), click(1_400), http(1_500, 3)]).unwrap();
    // Across batches: the next click follows an http entry, so it starts a new row.
    // 58.5 s after the last one still counts as a repeat, 61 s after it no longer.
    sink.write(&[http(1_600, 9), http(2_000, 1), click(3_000), from_ui, click(63_000), click(124_000)]).unwrap();

    let conn = open_read(&dir).unwrap();
    let rows: Vec<(String, i64, Option<i64>)> = query(&conn, &Query::default())
        .unwrap()
        .entries
        .into_iter()
        .rev()
        .map(|e| (e.message, e.repeat, e.last_ts))
        .collect();
    assert_eq!(
        rows,
        [
            ("Klick: „Senden“ (×3)".to_string(), 3, Some(1_400)),
            ("GET /api/x -> 404 (×3)".to_string(), 3, Some(2_000)),
            ("Klick: „Senden“ (×5)".to_string(), 5, Some(63_000)),
            ("Klick: „Senden“".to_string(), 1, None),
        ]
    );
    // The export shows the count and the time of the last one.
    let text = export(&conn, &[], &[], 0, None, "").unwrap();
    assert!(text.contains("Klick: „Senden“ (×5)  bis 00:01:03.000"), "{text}");
    drop(conn);
    // A cleared log takes the next repeat as a new entry.
    clear(&open_read(&dir).unwrap()).unwrap();
    sink.write(&[click(124_100)]).unwrap();
    assert_eq!(query(&open_read(&dir).unwrap(), &Query::default()).unwrap().total, 1);
    drop(sink);
    let _ = std::fs::remove_dir_all(dir);
}

/// Days of use without a restart: count and file size stay within the limits, and the space of
/// deleted entries goes back to the disk.
#[test]
fn stays_within_its_limits_while_running() {
    let dir = temp_dir();
    let log = Logger::start(&dir);
    assert_eq!(open_read(&dir).unwrap().query_row("PRAGMA auto_vacuum", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    // Big entries: about 45 MB in all, without a single restart.
    let filler = "x".repeat(7_000);
    for i in 0..6_500 {
        log.write(Entry::new(Level::Error, "ui", "error", format!("Skriptfehler {i}")).data(json!({ "stack": filler })));
        if i % 1_000 == 999 {
            log.flush(Duration::from_secs(30));
            assert!(disk_bytes(&dir) <= MAX_DISK_BYTES, "{} bytes after {i} entries", disk_bytes(&dir));
        }
    }
    log.flush(Duration::from_secs(30));
    let bytes = disk_bytes(&dir);
    assert!(bytes <= MAX_DISK_BYTES, "{bytes} bytes on disk");
    let conn = open_read(&dir).unwrap();
    let page = query(&conn, &Query { limit: Some(1), ..Query::default() }).unwrap();
    assert_eq!(page.entries[0].message, "Skriptfehler 6499", "the newest stay");
    assert!(page.total < 6_500 && page.total > 500, "{} kept", page.total);
    drop(conn);

    // Many small entries: never more than KEEP_MAX.
    let mut sink = Sink::new(open(&dir).unwrap());
    let now = now_ms();
    for chunk in 0..((KEEP_MAX as usize + 2_000) / BATCH) {
        let batch: Vec<Entry> =
            (0..BATCH).map(|i| entry(now + (chunk * BATCH + i) as i64, Level::Info, "ui", "nav", &format!("Seite {chunk}/{i}"))).collect();
        sink.write(&batch).unwrap();
    }
    let total: i64 = sink.conn.query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0)).unwrap();
    assert!(total <= KEEP_MAX, "{total} entries");
    drop(sink);
    drop(log);
    let _ = std::fs::remove_dir_all(dir);
}

/// The log has its own file: writing it leaves the real data untouched.
#[test]
fn the_real_database_does_not_grow_from_logging() {
    let dir = temp_dir();
    let store = store::Store::open(&dir).unwrap();
    let data_bytes = || -> u64 {
        ["rp-app.db", "rp-app.db-wal"].iter().filter_map(|f| std::fs::metadata(dir.join(f)).ok()).map(|m| m.len()).sum()
    };
    let before = data_bytes();
    let log = Logger::start(&dir);
    for i in 0..3_000 {
        log.write(Entry::new(Level::Info, "ui", "click", format!("Klick {i}")).data(json!({ "route": "#/" })));
    }
    log.flush(Duration::from_secs(30));
    assert_eq!(data_bytes(), before);
    assert!(disk_bytes(&dir) > 0);
    let conn = Connection::open(dir.join(store::DB_FILE)).unwrap();
    let tables: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'logs'", [], |r| r.get(0)).unwrap();
    assert_eq!(tables, 0, "no log table next to chats and cards");
    drop((conn, log, store));
    let _ = std::fs::remove_dir_all(dir);
}
