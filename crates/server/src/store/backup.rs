//! Backups through the store's own connection: the safety copy right before a restore, and the
//! restore itself (the backup file is copied into the running database through SQLite's backup
//! API, then brought to this build's schema and cleaned up like at a start). Naming, listing and
//! pruning live in `crate::backup`.

use std::path::Path;

use super::{migrate, Res, Store};
#[cfg(test)]
use crate::backup::Reason;
use crate::backup::{self, Info};

impl Store {
    /// A backup of the current state through the store's connection (the app itself backs up
    /// through connections of their own, see `backup::create`, and before a restore through
    /// `backup_before_restore`).
    #[cfg(test)]
    pub fn backup(&self, reason: Reason) -> Res<(Info, usize)> {
        backup::create_from(&self.conn, &self.dir, reason)
    }

    /// The `before-restore` backup of a restore of `source` that holds the backups folder; the
    /// pruning after it spares `source`.
    pub fn backup_before_restore(&self, held: &backup::Held, source: &Path) -> Res<(Info, usize)> {
        backup::create_before_restore(held, &self.conn, &self.dir, source)
    }

    /// Replaces the whole database with the backup at `src` (checked with `backup::check`
    /// before). Answers the schema version of the backup and the version after its migrations.
    pub fn restore_from(&mut self, src: &Path) -> Res<(usize, usize)> {
        let from = backup::open_source(src)?;
        backup::copy(&from, &mut self.conn)?;
        drop(from);
        let _: String = self.conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let versions = migrate(&mut self.conn)?;
        self.cleanup()?;
        Ok(versions)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use rusqlite::{Connection, OpenFlags};

    use super::super::tests::{sample, temp_store};
    use super::super::{schema_target, DB_FILE, MIGRATIONS};
    use super::*;
    use crate::backup::{
        apply_limit, before_migration, check, create, create_within, daily_due, list, locate, overview, remove, remove_many, remove_older, Kind,
        LimitReached, DAY_MS, DIR, KEEP_DAILY, KEEP_MIGRATION, KEEP_RESTORE,
    };

    static N: AtomicU32 = AtomicU32::new(0);

    fn counts(conn: &Connection) -> (i64, i64, i64, i64) {
        let n = |t: &str| conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap();
        (n("characters"), n("chats"), n("messages"), n("personas"))
    }

    fn read(path: &Path) -> Connection {
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
    }

    /// A store with a character, a chat and its greeting.
    fn filled() -> (Store, std::path::PathBuf) {
        let (mut s, dir) = temp_store();
        let id = s.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        s.create_chat(id, Some("Hi.")).unwrap();
        (s, dir)
    }

    /// A backup is a separate, valid database with the same rows, written while the store's
    /// connection stays open (WAL mode, uncheckpointed writes included).
    #[test]
    fn backup_is_a_valid_copy_with_the_same_counts() {
        let (s, dir) = filled();
        let (b, pruned) = create(&dir, Reason::Manual).unwrap();
        assert_eq!((b.reason, pruned), (Reason::Manual, 0));
        assert!(b.name.starts_with("rp-app-") && b.name.ends_with("-manual.db"), "{}", b.name);
        let path = dir.join(DIR).join(&b.name);
        assert_eq!(b.size, std::fs::metadata(&path).unwrap().len());
        let copy = read(&path);
        let ok: String = copy.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        let mode: String = copy.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!((ok.as_str(), mode.as_str()), ("ok", "delete"));
        assert_eq!(counts(&copy), counts(&s.conn));
        assert_eq!(counts(&copy), (1, 1, 1, 1));
        let v: i64 = copy.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, schema_target());
        drop(copy);
        assert!(!dir.join(DIR).join(format!("{}-wal", b.name)).exists());
        assert!(check(&path, schema_target()).is_ok());
        assert_eq!(check(&path, schema_target() - 1), Err("backup_newer"));
        let listed = list(&dir).unwrap();
        assert_eq!(listed.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), [b.name.as_str()]);
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An app-made backup file from the past (`stamp` = `2026-01-05-030000`), `size` bytes.
    fn fake(dir: &Path, stamp: &str, reason: &str, size: usize) -> String {
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        let name = format!("rp-app-{stamp}-{reason}.db");
        std::fs::write(dir.join(DIR).join(&name), vec![b'x'; size]).unwrap();
        name
    }

    fn day(d: &str) -> String {
        format!("{d}-030000")
    }

    /// One new daily backup applies the rule to what is there: the newest 7 daily ones, the oldest
    /// of the older ones in their week (as `weekly`), the newest 3 before a migration and before a
    /// restore; every manual one and every copy made by hand stays.
    #[test]
    fn keeps_daily_weekly_migration_restore_and_every_manual() {
        let (s, dir) = filled();
        std::fs::write(dir.join("rp-app.db.backup-2026-11-10"), std::fs::read(dir.join(DB_FILE)).unwrap()).unwrap();
        // 2026-01-05 is a Monday: ten days up to Wednesday of the next week.
        let daily: Vec<String> = (5..=14).map(|d| fake(&dir, &day(&format!("2026-01-{d:02}")), "daily", 10)).collect();
        let migration: Vec<String> = (1..=5).map(|d| fake(&dir, &day(&format!("2026-02-0{d}")), "migration", 10)).collect();
        let restore: Vec<String> = (1..=5).map(|d| fake(&dir, &day(&format!("2026-03-0{d}")), "before-restore", 10)).collect();
        let manual: Vec<String> = (1..=5).map(|d| fake(&dir, &day(&format!("2026-04-0{d}")), "manual", 10)).collect();

        let (now, pruned) = s.backup(Reason::Daily).unwrap();
        assert_eq!(now.kind, Kind::Daily);
        // Gone: Jan 6–8 (the week keeps Jan 5), two migration and two restore ones.
        assert_eq!(pruned, 3 + 2 + 2);
        let listed = list(&dir).unwrap();
        let kind = |n: &str| listed.iter().find(|b| b.name == n).map(|b| b.kind);
        assert_eq!(kind(&now.name), Some(Kind::Daily));
        for n in &daily[9 - (KEEP_DAILY - 2)..] {
            assert_eq!(kind(n), Some(Kind::Daily), "{n}");
        }
        assert_eq!(kind(&daily[0]), Some(Kind::Weekly), "the Monday stays for its week");
        for n in &daily[1..4] {
            assert_eq!(kind(n), None, "{n} went");
        }
        assert_eq!(migration.iter().filter(|n| kind(n) == Some(Kind::Migration)).count(), KEEP_MIGRATION);
        assert_eq!(kind(&migration[4]), Some(Kind::Migration), "the newest stay");
        assert_eq!(restore.iter().filter(|n| kind(n) == Some(Kind::Restore)).count(), KEEP_RESTORE);
        assert_eq!(kind(&restore[0]), None, "the oldest went");
        assert!(manual.iter().all(|n| kind(n) == Some(Kind::Manual)));
        assert_eq!(kind("rp-app.db.backup-2026-11-10"), Some(Kind::Manual));
        assert_eq!(listed.len(), 1 + 7 + 1 + 3 + 3 + 5);
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The backups folder stays within its limit: the oldest automatic ones go first, manual ones
    /// never; a manual backup that does not fit next to the other manual ones is refused.
    #[test]
    fn the_folder_stays_within_its_limit() {
        let (s, dir) = filled();
        assert_eq!(s.backup_limit(), 1024 * 1024 * 1024, "1 GB until another is chosen");
        let db: u64 = s.conn.query_row("SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()", [], |r| r.get::<_, i64>(0)).unwrap() as u64;
        let old: Vec<String> = (5..=7).map(|d| fake(&dir, &day(&format!("2026-01-0{d}")), "daily", 1000)).collect();
        let migration = fake(&dir, &day("2026-02-01"), "migration", 1000);
        let manual = fake(&dir, &day("2026-04-01"), "manual", 1000);

        // Room for the new one, the manual one and 1500 bytes more: the three oldest daily ones go.
        let (daily, pruned) = create_within(&s.conn, &dir, Reason::Daily, db + 2500).unwrap();
        assert_eq!(pruned, 3);
        let names = || list(&dir).unwrap().into_iter().map(|b| b.name).collect::<Vec<_>>();
        assert!(old.iter().all(|n| !names().contains(n)));
        assert!([&migration, &manual, &daily.name].iter().all(|n| names().contains(n)));
        let ov = overview(&dir, db + 2500).unwrap();
        assert!(ov.total_bytes <= db + 2500 && ov.total_bytes == db + 2000, "{}", ov.total_bytes);

        // A manual one that would not fit next to the other manual one: refused, nothing written.
        let before = std::fs::read_dir(dir.join(DIR)).unwrap().count();
        let err = create_within(&s.conn, &dir, Reason::Manual, db + 999).unwrap_err();
        assert!(err.downcast_ref::<LimitReached>().is_some(), "{err}");
        assert_eq!(std::fs::read_dir(dir.join(DIR)).unwrap().count(), before);

        // Just fits: the automatic ones make room, the manual ones stay.
        let (second, pruned) = create_within(&s.conn, &dir, Reason::Manual, db + 1000).unwrap();
        assert_eq!(pruned, 2, "the migration and the daily one went");
        let mut left = names();
        left.sort();
        let mut want = vec![manual.clone(), second.name.clone()];
        want.sort();
        assert_eq!(left, want);
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A backup file of `size` bytes whose space is reserved but not written (fast also for
    /// hundreds of MB).
    fn big(dir: &Path, stamp: &str, reason: &str, size: u64) -> String {
        let name = fake(dir, stamp, reason, 0);
        std::fs::OpenOptions::new().write(true).open(dir.join(DIR).join(&name)).unwrap().set_len(size).unwrap();
        name
    }

    /// The limit is a setting: only the four choices are taken; a smaller one prunes at once
    /// (oldest automatic first, manual never), the list says when the folder is nearly full, and
    /// a manual backup over the chosen limit is refused.
    #[test]
    fn the_limit_is_a_setting_and_a_smaller_one_prunes() {
        const MB: u64 = 1024 * 1024;
        let (mut s, dir) = filled();
        assert_eq!(s.backup_limit(), 1024 * MB);
        for bad in [0, 1, 499 * MB, 1000 * 1000 * 1000, 3072 * MB, u64::MAX] {
            assert!(s.set_backup_limit(bad).is_err(), "{bad}");
        }
        assert_eq!(s.backup_limit(), 1024 * MB, "nothing saved");
        // A value that is no choice (written by hand) counts as the default.
        s.set_setting(crate::backup::LIMIT_KEY, "12345").unwrap();
        assert_eq!(s.backup_limit(), 1024 * MB);

        let manual = big(&dir, &day("2026-04-01"), "manual", 480 * MB);
        let oldest = big(&dir, &day("2026-01-05"), "daily", 30 * MB);
        let younger = [big(&dir, &day("2026-01-06"), "daily", 1024), big(&dir, &day("2026-01-07"), "migration", 1024)];
        let names = || list(&dir).unwrap().into_iter().map(|b| b.name).collect::<Vec<_>>();
        let ov = overview(&dir, s.backup_limit()).unwrap();
        assert_eq!((ov.limit_bytes, ov.near_full, names().len()), (1024 * MB, false, 4));

        for limit in crate::backup::LIMIT_CHOICES {
            s.set_backup_limit(limit).unwrap();
            assert_eq!(s.backup_limit(), limit);
        }
        s.set_backup_limit(500 * MB).unwrap();
        assert_eq!(apply_limit(&dir, s.backup_limit()).unwrap(), 1, "510 MB > 500 MB: the oldest automatic one goes");
        let left = names();
        assert!(!left.contains(&oldest));
        assert!(left.contains(&manual) && younger.iter().all(|n| left.contains(n)), "{left:?}");
        let ov = overview(&dir, s.backup_limit()).unwrap();
        assert_eq!((ov.limit_bytes, ov.near_full), (500 * MB, true), "{} of 500 MB", ov.total_bytes / MB);
        assert!(ov.total_bytes <= 500 * MB);

        // Manual ones over the chosen limit: a new manual backup is refused, nothing written.
        let more = big(&dir, &day("2026-04-02"), "manual", 25 * MB);
        let before = names();
        let err = s.backup(Reason::Manual).unwrap_err();
        assert_eq!(err.downcast_ref::<LimitReached>().map(|l| l.0), Some(500 * MB), "{err}");
        assert_eq!(names(), before);
        // At 1 GB it fits again.
        s.set_backup_limit(1024 * MB).unwrap();
        let (made, _) = s.backup(Reason::Manual).unwrap();
        assert!(names().contains(&made.name) && names().contains(&more));
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Deleting several by name: exactly the named ones; one refused name (another folder, the
    /// database, a copy made by hand, a missing one) and none is deleted.
    #[test]
    fn delete_many_by_name_all_or_none() {
        let (s, dir) = filled();
        std::fs::write(dir.join("rp-app.db.backup-by-hand"), b"x").unwrap();
        let all: Vec<String> = (1..=4).map(|d| fake(&dir, &day(&format!("2026-01-0{d}")), if d == 1 { "manual" } else { "daily" }, 10)).collect();
        for bad in ["../rp-app.db", "..\\rp-app.db", "rp-app.db", "logs.db", "rp-app.db.backup-by-hand", "C:\\Windows\\win.ini", "backups/../rp-app.db"] {
            let names = vec![all[0].clone(), bad.to_string(), all[1].clone()];
            assert_eq!(remove_many(&dir, &names).err(), Some("bad_name"), "{bad:?}");
        }
        let missing = vec![all[0].clone(), "rp-app-2020-01-01-000000-manual.db".to_string()];
        assert_eq!(remove_many(&dir, &missing).err(), Some("backup_missing"));
        assert_eq!(list(&dir).unwrap().len(), 5, "nothing went");
        assert!(dir.join(DB_FILE).is_file() && dir.join("rp-app.db.backup-by-hand").is_file());

        let gone = remove_many(&dir, &[all[0].clone(), all[2].clone(), all[0].clone()]).unwrap();
        assert_eq!(gone.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), [all[0].as_str(), all[2].as_str()]);
        let mut left: Vec<String> = list(&dir).unwrap().into_iter().map(|b| b.name).collect();
        left.sort();
        assert_eq!(left, [all[1].clone(), all[3].clone(), "rp-app.db.backup-by-hand".to_string()]);
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// "Automatic ones older than N days": daily, migration and restore ones made more than N days
    /// ago go; manual ones, copies made by hand and younger ones stay.
    #[test]
    fn delete_automatic_older_than_days() {
        let (s, dir) = filled();
        std::fs::write(dir.join("rp-app.db.backup-by-hand"), b"x").unwrap();
        let old = [fake(&dir, &day("2026-01-05"), "daily", 10), fake(&dir, &day("2026-01-01"), "migration", 10), fake(&dir, &day("2026-01-10"), "before-restore", 10)];
        let stay = [
            fake(&dir, &day("2025-12-01"), "manual", 10),
            fake(&dir, &day("2026-01-13"), "daily", 10), // exactly 7 days: not older
            fake(&dir, &day("2026-01-15"), "daily", 10),
        ];
        // 2026-01-20 03:00:00 UTC.
        let now_ms = 1_768_878_000_000;
        let mut gone: Vec<String> = remove_older(&dir, 7, now_ms).unwrap().into_iter().map(|b| b.name).collect();
        gone.sort();
        let mut want = old.to_vec();
        want.sort();
        assert_eq!(gone, want);
        let left: Vec<String> = list(&dir).unwrap().into_iter().map(|b| b.name).collect();
        assert!(stay.iter().all(|n| left.contains(n)) && left.contains(&"rp-app.db.backup-by-hand".to_string()), "{left:?}");
        assert_eq!(left.len(), 4);
        assert!(remove_older(&dir, 7, now_ms).unwrap().is_empty());
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Deleting one backup: only files of the backups folder by a name the list gives, never a
    /// copy made by hand, the database or anything reached through `..`.
    #[test]
    fn delete_only_reaches_backups() {
        let (s, dir) = filled();
        let (b, _) = s.backup(Reason::Manual).unwrap();
        std::fs::write(dir.join("rp-app.db.backup-by-hand"), b"x").unwrap();
        for bad in [
            "",
            "..",
            "../rp-app.db",
            "..\\rp-app.db",
            "rp-app.db",
            "logs.db",
            "backups/../rp-app.db",
            &format!("{}/../../rp-app.db", b.name),
            &format!("..\\{}", b.name),
            "rp-app.db.backup-by-hand",
            "C:\\Windows\\win.ini",
        ] {
            assert_eq!(remove(&dir, bad).err(), Some("bad_name"), "{bad:?}");
        }
        assert!(dir.join(DB_FILE).is_file() && dir.join("rp-app.db.backup-by-hand").is_file());
        assert_eq!(remove(&dir, "rp-app-2020-01-01-000000-manual.db").err(), Some("backup_missing"));
        assert_eq!(remove(&dir, &b.name).unwrap().name, b.name);
        assert!(!dir.join(DIR).join(&b.name).exists());
        assert_eq!(remove(&dir, &b.name).err(), Some("backup_missing"));
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A restore brings the backed-up state back, and the state it replaced is backed up first
    /// (`before-restore`) — through the hub, which refuses while the model writes.
    #[tokio::test]
    async fn restore_brings_the_old_state_back() {
        let (mut s, dir) = filled();
        let (b, _) = s.backup(Reason::Manual).unwrap();
        let mara = s.conn.query_row("SELECT id FROM characters", [], |r| r.get::<_, i64>(0)).unwrap();
        // After the backup: Mara is deleted for good, another character comes.
        s.set_deleted(mara, true).unwrap();
        s.cleanup().unwrap();
        s.add_character(&sample(r#"{"name":"Ilse"}"#), None).unwrap();
        assert_eq!(counts(&s.conn), (1, 0, 0, 1));

        let hub = std::sync::Arc::new(crate::chat::Hub::new(s, "http://127.0.0.1:1".into(), crate::logs::Logger::start(&dir)));
        let (path, reason) = locate(&dir, &b.name).unwrap();
        assert_eq!(reason, Reason::Manual);
        let safety = hub.restore_backup(&path).unwrap();
        assert_eq!(safety.reason, Reason::BeforeRestore);
        let names: String = hub.store(|s| s.conn.query_row("SELECT group_concat(name) FROM characters", [], |r| r.get(0))).unwrap();
        assert_eq!(names, "Mara");
        assert_eq!(hub.store(|s| counts(&s.conn)), (1, 1, 1, 1));
        // The store keeps working on the restored database.
        hub.store(|s| s.add_character(&sample(r#"{"name":"Moos"}"#), None)).unwrap();
        assert_eq!(hub.store(|s| counts(&s.conn)).0, 2);
        // The safety copy holds the replaced state (Ilse only).
        let before = read(&dir.join(DIR).join(&safety.name));
        let n: String = before.query_row("SELECT group_concat(name) FROM characters", [], |r| r.get(0)).unwrap();
        assert_eq!(n, "Ilse");
        drop(before);

        // While the model writes, nothing is restored and no safety copy is made.
        let chat = hub.open_chat(hub.store(|s| s.conn.query_row("SELECT id FROM characters WHERE name = 'Mara'", [], |r| r.get::<_, i64>(0))).unwrap(), false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        let backups = list(&dir).unwrap().len();
        hub.send(chat, "Hello".into()).unwrap();
        assert_eq!(hub.restore_backup(&path).err(), Some("busy"));
        assert_eq!(list(&dir).unwrap().len(), backups);
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A file that is no database of this app is refused before anything changes.
    #[test]
    fn broken_backup_is_refused() {
        let (s, dir) = filled();
        let hub = crate::chat::Hub::new(s, "http://127.0.0.1:1".into(), crate::logs::Logger::start(&dir));
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        let bad = dir.join(DIR).join("rp-app-2026-01-01-000000-manual.db");
        std::fs::write(&bad, b"not a database at all, just text that is long enough").unwrap();
        assert_eq!(hub.restore_backup(&bad).err(), Some("bad_backup"));
        let empty = dir.join(DIR).join("rp-app-2026-01-01-000001-manual.db");
        Connection::open(&empty).unwrap().execute_batch("CREATE TABLE x (a); PRAGMA user_version = 3;").unwrap();
        assert_eq!(hub.restore_backup(&empty).err(), Some("bad_backup"));
        assert_eq!(list(&dir).unwrap().iter().filter(|b| b.reason == Reason::BeforeRestore).count(), 0);
        assert_eq!(hub.store(|s| counts(&s.conn)), (1, 1, 1, 1));
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Restoring the oldest of three `before-restore` backups: the safety copy made right before
    /// would push it out by the rule "newest 3", but it is the one being restored — it stays and
    /// is restored; the next backup prunes it as usual.
    #[test]
    fn restore_keeps_its_source_against_the_count_rule() {
        let (mut s, dir) = filled();
        let restores: Vec<Info> = (0..KEEP_RESTORE).map(|_| s.backup(Reason::BeforeRestore).unwrap().0).collect();
        s.add_character(&sample(r#"{"name":"Ilse"}"#), None).unwrap();
        let hub = crate::chat::Hub::new(s, "http://127.0.0.1:1".into(), crate::logs::Logger::start(&dir));
        let (source, _) = locate(&dir, &restores[0].name).unwrap();
        let safety = hub.restore_backup(&source).unwrap();
        assert!(source.is_file(), "the chosen backup is still there");
        assert_eq!(hub.store(|s| counts(&s.conn)), (1, 1, 1, 1), "and was restored");
        let kept = |dir: &Path| list(dir).unwrap().iter().filter(|b| b.reason == Reason::BeforeRestore).count();
        assert_eq!(kept(&dir), KEEP_RESTORE + 1);
        assert!(list(&dir).unwrap().iter().any(|b| b.name == safety.name));
        // Afterwards it is an ordinary old one again.
        let (_, pruned) = hub.store(|s| s.backup(Reason::BeforeRestore)).unwrap();
        assert_eq!((pruned, kept(&dir)), (2, KEEP_RESTORE));
        assert!(!source.exists());
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Restoring the oldest automatic backup when the safety copy pushes the folder over its
    /// limit: the oldest automatic one would go first, but it is the one being restored.
    #[test]
    fn restore_keeps_its_source_against_the_limit() {
        const MB: u64 = 1024 * 1024;
        let (mut s, dir) = filled();
        let (source, _) = s.backup(Reason::Daily).unwrap();
        s.add_character(&sample(r#"{"name":"Ilse"}"#), None).unwrap();
        s.set_backup_limit(500 * MB).unwrap();
        // A manual one that leaves room for the source but not for the safety copy as well.
        big(&dir, &day("2026-01-01"), "manual", 500 * MB - source.size);
        let hub = crate::chat::Hub::new(s, "http://127.0.0.1:1".into(), crate::logs::Logger::start(&dir));
        let (path, _) = locate(&dir, &source.name).unwrap();
        hub.restore_backup(&path).unwrap();
        assert!(path.is_file(), "the chosen backup is still there");
        assert_eq!(hub.store(|s| counts(&s.conn)), (1, 1, 1, 1), "and was restored");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Deleting several, one of them held open elsewhere (as SQLite opens a file: others may read
    /// and write, not delete or rename): none goes, not even those before it in the list.
    #[cfg(windows)]
    #[test]
    fn delete_many_keeps_all_when_one_is_held_open() {
        use std::os::windows::fs::OpenOptionsExt;
        const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
        let (s, dir) = filled();
        let all: Vec<String> = (1..=3).map(|d| fake(&dir, &day(&format!("2026-01-0{d}")), "daily", 10)).collect();
        let held = std::fs::OpenOptions::new().read(true).share_mode(SHARE_READ_WRITE).open(dir.join(DIR).join(&all[1])).unwrap();
        assert_eq!(remove_many(&dir, &all).err(), Some("storage"));
        let names = || list(&dir).unwrap().into_iter().map(|b| b.name).collect::<Vec<_>>();
        assert!(all.iter().all(|n| names().contains(n)), "none went: {:?}", names());
        assert_eq!(std::fs::read_dir(dir.join(DIR)).unwrap().count(), 3, "no file left aside");
        drop(held);
        assert_eq!(remove_many(&dir, &all).unwrap().len(), 3);
        assert_eq!(std::fs::read_dir(dir.join(DIR)).unwrap().count(), 0);
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// While a restore holds the backups folder, deleting waits until it is done.
    #[test]
    fn delete_waits_while_a_restore_holds_the_folder() {
        let (s, dir) = filled();
        let name = fake(&dir, &day("2026-01-01"), "daily", 10);
        let held = crate::backup::hold();
        let (d, n) = (dir.clone(), name.clone());
        let deleting = std::thread::spawn(move || remove_many(&d, &[n]).map(|g| g.len()));
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!deleting.is_finished() && dir.join(DIR).join(&name).is_file(), "still waiting");
        drop(held);
        assert_eq!(deleting.join().unwrap(), Ok(1));
        assert!(!dir.join(DIR).join(&name).exists());
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Only names the list can give lead to a file; nothing reaches outside the two folders.
    #[test]
    fn names_are_checked_strictly() {
        let (s, dir) = filled();
        let (b, _) = s.backup(Reason::Daily).unwrap();
        std::fs::write(dir.join("rp-app.db.backup-2026-11-10_vor-update"), b"x").unwrap();
        assert_eq!(locate(&dir, &b.name).unwrap().1, Reason::Daily);
        assert_eq!(locate(&dir, "rp-app.db.backup-2026-11-10_vor-update").unwrap().1, Reason::Hand);
        assert_eq!(locate(&dir, "rp-app-2026-01-01-000000-daily.db").err(), Some("backup_missing"));
        for bad in [
            "",
            "rp-app.db",
            "logs.db",
            "../rp-app.db",
            "..\\rp-app.db",
            "backups/x.db",
            "rp-app-2026-01-01-000000-evil.db",
            "rp-app-2026-01-01-00000x-daily.db",
            "rp-app-2026-01-01-000000-daily.db/../../rp-app.db",
            "rp-app-2026-01-01-000000-daily.db\\..\\x",
            "rp-app-2026-01-01-000000-daily.db.part",
            "rp-app.db.backup-",
            "rp-app.db.backup-../../x",
            "rp-app.db.backup-a\\b",
            "rp-app.db.backup-a:b",
            "rp-app.db.backup-x-wal",
            "rp-app.db.backup-..",
            "C:\\Windows\\win.ini",
            "/etc/passwd",
        ] {
            assert_eq!(locate(&dir, bad).err(), Some("bad_name"), "{bad:?}");
        }
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Before a migration: a backup of the old version with all its data, only when there is an
    /// old version with data; a fresh or current database gets none.
    #[test]
    fn migration_backup_only_for_an_older_database() {
        let dir = std::env::temp_dir().join(format!("rp-app-bk-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(before_migration(&dir, schema_target()).unwrap().is_none(), "no database yet");
        let old = schema_target() - 1;
        {
            let conn = Connection::open(dir.join(DB_FILE)).unwrap();
            conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
            for sql in &MIGRATIONS[..old] {
                conn.execute_batch(sql).unwrap();
            }
            conn.execute_batch(&format!(
                "PRAGMA user_version = {old};
                 INSERT INTO characters (name, raw_json, has_png, created_at) VALUES ('Mara', '{{\"name\":\"Mara\"}}', 0, 1);
                 INSERT INTO chats (character_id, created_at, updated_at) VALUES (1, 1, 1);"
            ))
            .unwrap();
        }
        let (b, _) = before_migration(&dir, schema_target()).unwrap().expect("a backup before the migration");
        assert_eq!(b.reason, Reason::Migration);
        let s = Store::open(&dir).unwrap();
        assert_eq!(s.migrated(), (old, schema_target()));
        let copy = read(&dir.join(DIR).join(&b.name));
        let v: i64 = copy.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, old, "the backup holds the version before the migration");
        assert_eq!(counts(&copy), (1, 1, 0, 1));
        drop(copy);
        assert!(before_migration(&dir, schema_target()).unwrap().is_none(), "current version: none");
        assert_eq!(list(&dir).unwrap().len(), 1);
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The daily backup is due without one, and again once the newest is a day old.
    #[test]
    fn daily_is_due_after_a_day() {
        let (s, dir) = filled();
        assert!(daily_due(&dir, crate::logs::now_ms()).unwrap());
        s.backup(Reason::Manual).unwrap();
        assert!(daily_due(&dir, crate::logs::now_ms()).unwrap(), "only a daily one counts");
        let (b, _) = s.backup(Reason::Daily).unwrap();
        assert!(!daily_due(&dir, crate::logs::now_ms()).unwrap());
        assert!(daily_due(&dir, b.time + DAY_MS).unwrap());
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The chosen rhythm moves the automatic backup: weekly waits seven days, off never makes one.
    #[test]
    fn rhythm_setting_moves_the_automatic_backup() {
        use crate::backup::{next_automatic, RHYTHM_KEY};
        let (mut s, dir) = filled();
        let (b, _) = s.backup(Reason::Daily).unwrap();
        s.set_general(RHYTHM_KEY, "weekly").unwrap();
        assert!(!daily_due(&dir, b.time + DAY_MS).unwrap(), "a day is not enough for weekly");
        assert!(daily_due(&dir, b.time + 7 * DAY_MS).unwrap());
        assert_eq!(next_automatic(&dir, b.time).unwrap(), Some(b.time + 7 * DAY_MS));
        s.set_general(RHYTHM_KEY, "off").unwrap();
        assert!(!daily_due(&dir, b.time + 400 * DAY_MS).unwrap());
        assert_eq!(next_automatic(&dir, b.time).unwrap(), None);
        s.set_general(RHYTHM_KEY, "daily").unwrap();
        assert!(daily_due(&dir, b.time + DAY_MS).unwrap());
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }
}
