//! Backups of the database (`rp-app.db`; the activity log `logs.db` is not backed up) through
//! SQLite's online backup API, so a copy is consistent while the app runs and writes.
//!
//! They live in `<data>/backups/` as `rp-app-<YYYY-MM-DD-HHMMSS>-<reason>.db` (time in UTC).
//! Kept after every new backup: the newest 7 daily ones, of the older daily ones the oldest of each
//! calendar week (UTC, Monday to Sunday) for the newest 4 weeks ("weekly"), the newest 3 before a
//! migration and the newest 3 before a restore. `manual` ones are never deleted on their own, and
//! neither are copies made by hand next to the database (`rp-app.db.backup-…`), which are listed
//! and can be restored like the others. The backups folder stays within its limit (setting
//! `backup_limit_bytes`, one of `LIMIT_CHOICES`, default `DEFAULT_LIMIT_BYTES`): above it the
//! oldest automatic backups go first, and a manual backup that would not fit next to the other
//! manual ones is refused (`LimitReached`).
//!
//! Restoring goes through `Hub::restore_backup` (it must hold the hub's lock and refuse while the
//! model writes) and `Store::restore_from`. It holds the backups folder (`hold`) from its check to
//! its end, so nothing deletes the backup being restored meanwhile, and the pruning after its
//! `before-restore` backup spares it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::json;

use crate::logs::{self, Entry, Level, Logger};
use crate::store::{Res, DB_FILE};

/// The folder inside the data folder.
pub const DIR: &str = "backups";
/// Daily backups kept as such (the newest).
pub const KEEP_DAILY: usize = 7;
/// Calendar weeks of which the oldest daily backup is kept beyond the daily ones.
pub const KEEP_WEEKLY: usize = 4;
/// Backups before a migration kept (the newest).
pub const KEEP_MIGRATION: usize = 3;
/// Backups before a restore kept (the newest).
pub const KEEP_RESTORE: usize = 3;
const MB: u64 = 1024 * 1024;
/// The limits the owner can choose for the backups folder (all backups together): 500 MB, 1, 2, 5 GB.
pub const LIMIT_CHOICES: [u64; 4] = [500 * MB, 1024 * MB, 2048 * MB, 5120 * MB];
/// The limit until the owner chooses another.
pub const DEFAULT_LIMIT_BYTES: u64 = 1024 * MB;
/// The setting (table `settings`) that holds the chosen limit in bytes.
pub const LIMIT_KEY: &str = "backup_limit_bytes";
/// From this share of the limit on (in percent) the folder counts as nearly full.
const NEAR_FULL_PERCENT: u64 = 90;
/// A new daily backup is due when the newest one is this old.
pub const DAY_MS: i64 = 24 * 3600 * 1000;
/// How often the running server looks whether a daily backup is due.
const CHECK_EVERY: Duration = Duration::from_secs(10 * 60);
/// Copies the owner made by hand next to the database.
const HAND_PREFIX: &str = "rp-app.db.backup-";
const AUTO_PREFIX: &str = "rp-app-";
/// `YYYY-MM-DD-HHMMSS`
const STAMP_LEN: usize = 17;

/// Only one backup is written at a time (names are picked by looking which one is free), and
/// nothing is pruned or deleted while a backup is written or restored.
static WRITING: Mutex<()> = Mutex::new(());

/// The backups folder held for one caller (see `hold`).
pub struct Held {
    _guard: MutexGuard<'static, ()>,
}

/// Holds the backups folder: until the answer is dropped no backup is written, pruned or deleted
/// by anyone else. A restore holds it from its check to its end, so its source cannot go meanwhile.
pub fn hold() -> Held {
    Held { _guard: WRITING.lock().unwrap_or_else(|e| e.into_inner()) }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    Daily,
    Migration,
    Manual,
    BeforeRestore,
    /// A copy made by hand next to the database.
    Hand,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Daily => "daily",
            Reason::Migration => "migration",
            Reason::Manual => "manual",
            Reason::BeforeRestore => "before-restore",
            Reason::Hand => "hand",
        }
    }

    fn parse(s: &str) -> Option<Reason> {
        [Reason::Daily, Reason::Migration, Reason::Manual, Reason::BeforeRestore].into_iter().find(|r| r.as_str() == s)
    }

    /// Made by the app on its own, and therefore pruned.
    pub fn automatic(self) -> bool {
        matches!(self, Reason::Daily | Reason::Migration | Reason::BeforeRestore)
    }

    /// The kind the list shows (a daily one kept for its week becomes `Weekly` there).
    fn kind(self) -> Kind {
        match self {
            Reason::Daily => Kind::Daily,
            Reason::Migration => Kind::Migration,
            Reason::Manual | Reason::Hand => Kind::Manual,
            Reason::BeforeRestore => Kind::Restore,
        }
    }
}

/// What a backup is kept as.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Daily,
    Weekly,
    Migration,
    Manual,
    Restore,
}

/// One backup file.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub name: String,
    pub reason: Reason,
    pub kind: Kind,
    /// When the file was last written, Unix milliseconds.
    pub time: i64,
    /// When the backup was made (from its name; the file time for a copy made by hand), Unix ms.
    pub created_at: i64,
    /// Bytes.
    pub size: u64,
    /// Schema version of the database inside (`None`: not readable).
    pub schema: Option<i64>,
}

/// A manual backup that would not fit into the limit (bytes) next to the other manual ones.
#[derive(Debug)]
pub struct LimitReached(pub u64);

impl std::fmt::Display for LimitReached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the backups would exceed {} MB", self.0 / MB)
    }
}

/// The limit of the backups folder chosen in the database behind `conn` (setting `LIMIT_KEY`);
/// `DEFAULT_LIMIT_BYTES` when none is chosen, the value is not one of `LIMIT_CHOICES`, or the
/// database has no settings yet.
pub fn limit_in(conn: &Connection) -> u64 {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [LIMIT_KEY], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| LIMIT_CHOICES.contains(v))
        .unwrap_or(DEFAULT_LIMIT_BYTES)
}

impl std::error::Error for LimitReached {}

/// The list of backups with what they and the database take, and what is left on the disk.
#[derive(Debug, Serialize)]
pub struct Overview {
    /// Newest first.
    pub backups: Vec<Info>,
    /// Bytes of all backups in the backups folder (what `limit_bytes` applies to).
    pub total_bytes: u64,
    pub limit_bytes: u64,
    /// `total_bytes` is at least 90 % of `limit_bytes`.
    pub near_full: bool,
    /// The database with its write-ahead file.
    pub db_bytes: u64,
    /// Free space on the disk of the data folder (`None`: unknown).
    pub disk_free: Option<u64>,
}

/// The reason of an app-made backup file name (`rp-app-2026-11-15-130501-daily.db`).
fn parse_auto(name: &str) -> Option<Reason> {
    let rest = name.strip_prefix(AUTO_PREFIX)?.strip_suffix(".db")?;
    let (stamp, reason) = (rest.get(..STAMP_LEN)?, rest.get(STAMP_LEN..)?.strip_prefix('-')?);
    let shape = stamp.bytes().enumerate().all(|(i, b)| if [4, 7, 10].contains(&i) { b == b'-' } else { b.is_ascii_digit() });
    if !shape {
        return None;
    }
    Reason::parse(reason)
}

/// A hand-made copy next to the database: `rp-app.db.backup-<anything plain>`, never one of the
/// side files SQLite puts next to a database it opened.
fn hand_name_ok(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(HAND_PREFIX) else { return false };
    !rest.is_empty()
        && name.len() <= 120
        && rest.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !rest.starts_with('.')
        && ![".part", "-wal", "-shm", "-journal"].iter().any(|s| rest.ends_with(s))
}

/// The file of a backup by its name, as the list gives it. Err: `bad_name` (not a backup name —
/// also every attempt to reach another folder) or `backup_missing`.
pub fn locate(data_dir: &Path, name: &str) -> Result<(PathBuf, Reason), &'static str> {
    let (path, reason) = if let Some(reason) = parse_auto(name) {
        (data_dir.join(DIR).join(name), reason)
    } else if hand_name_ok(name) {
        (data_dir.join(name), Reason::Hand)
    } else {
        return Err("bad_name");
    };
    if !path.is_file() {
        return Err("backup_missing");
    }
    Ok((path, reason))
}

fn info(path: &Path, name: String, reason: Reason) -> Res<Info> {
    let meta = std::fs::metadata(path)?;
    let time = meta.modified()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let created_at = stamp_secs(&name).map_or(time, |s| s * 1000);
    let schema = open_source(path).and_then(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0))).ok();
    Ok(Info { name, reason, kind: reason.kind(), time, created_at, size: meta.len(), schema })
}

/// Days since 1970-01-01 of a calendar date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix seconds of the time in an app-made backup's name (UTC).
fn stamp_secs(name: &str) -> Option<i64> {
    parse_auto(name)?;
    let st = &name[AUTO_PREFIX.len()..AUTO_PREFIX.len() + STAMP_LEN]; // `2026-11-15-130501`
    let n = |r: std::ops::Range<usize>| st[r].parse::<i64>().ok();
    let days = days_from_civil(n(0..4)?, n(5..7)?, n(8..10)?);
    Some(days * 86_400 + n(11..13)? * 3600 + n(13..15)? * 60 + n(15..17)?)
}

/// The calendar week (Monday to Sunday, UTC) of a time, counted from the week of 1970-01-01.
fn week_of(secs: i64) -> i64 {
    // 1970-01-01 was a Thursday: three days after the Monday that starts its week.
    (secs.div_euclid(86_400) + 3).div_euclid(7)
}

/// An app-made file in the backups folder.
struct Made {
    name: String,
    reason: Reason,
    secs: i64,
    size: u64,
}

/// Every app-made backup in the folder, the newest first.
fn made_in(dir: &Path) -> Res<Vec<Made>> {
    let mut out = Vec::new();
    for name in files_in(dir)? {
        let (Some(reason), Some(secs)) = (parse_auto(&name), stamp_secs(&name)) else { continue };
        let Ok(meta) = std::fs::metadata(dir.join(&name)) else { continue };
        if meta.is_file() {
            out.push(Made { name, reason, secs, size: meta.len() });
        }
    }
    out.sort_by(|a, b| b.secs.cmp(&a.secs).then_with(|| b.name.cmp(&a.name)));
    Ok(out)
}

/// What the retention rule keeps and drops among `files` (newest first): answers the daily ones
/// kept for their week, and the names to delete — by the rule, then the oldest automatic ones
/// while all that stays is above `limit`. `keep` (the backup being restored) is never dropped.
fn plan(files: &[Made], limit: u64, keep: Option<&str>) -> (HashSet<String>, Vec<String>) {
    let mut gone: Vec<&str> = Vec::new();
    let newest = |reason: Reason| files.iter().filter(move |f| f.reason == reason);
    // Older daily ones: the oldest of each week (the last seen, newest first), the newest weeks.
    let mut weeks: BTreeMap<i64, &str> = BTreeMap::new();
    for f in newest(Reason::Daily).skip(KEEP_DAILY) {
        if let Some(prev) = weeks.insert(week_of(f.secs), &f.name) {
            gone.push(prev);
        }
    }
    let weekly: HashSet<String> = weeks.values().rev().take(KEEP_WEEKLY).map(|n| n.to_string()).collect();
    gone.extend(weeks.values().rev().skip(KEEP_WEEKLY).copied());
    gone.extend(newest(Reason::Migration).skip(KEEP_MIGRATION).map(|f| f.name.as_str()));
    gone.extend(newest(Reason::BeforeRestore).skip(KEEP_RESTORE).map(|f| f.name.as_str()));
    gone.retain(|n| Some(*n) != keep);

    let mut total: u64 = files.iter().filter(|f| !gone.contains(&f.name.as_str())).map(|f| f.size).sum();
    // The newest (the one just written) always stays.
    for f in files.iter().skip(1).rev() {
        if total <= limit {
            break;
        }
        if f.reason.automatic() && Some(f.name.as_str()) != keep && !gone.contains(&f.name.as_str()) {
            gone.push(&f.name);
            total -= f.size;
        }
    }
    (weekly, gone.into_iter().map(str::to_string).collect())
}

fn files_in(dir: &Path) -> Res<Vec<String>> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Ok(entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Every backup, the newest first.
pub fn list(data_dir: &Path) -> Res<Vec<Info>> {
    let mut out = Vec::new();
    let dir = data_dir.join(DIR);
    for name in files_in(&dir)? {
        if let Some(reason) = parse_auto(&name) {
            out.push(info(&dir.join(&name), name, reason)?);
        }
    }
    for name in files_in(data_dir)? {
        let path = data_dir.join(&name);
        if hand_name_ok(&name) && path.is_file() {
            out.push(info(&path, name, Reason::Hand)?);
        }
    }
    let (weekly, _) = plan(&made_in(&dir)?, u64::MAX, None);
    for b in &mut out {
        if weekly.contains(&b.name) {
            b.kind = Kind::Weekly;
        }
    }
    out.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| b.name.cmp(&a.name)));
    Ok(out)
}

/// The list with the sizes for the Backups tab; `limit_bytes` is the folder's limit.
pub fn overview(data_dir: &Path, limit_bytes: u64) -> Res<Overview> {
    let backups = list(data_dir)?;
    let total_bytes: u64 = backups.iter().filter(|b| b.reason != Reason::Hand).map(|b| b.size).sum();
    let near_full = total_bytes.saturating_mul(100) >= limit_bytes.saturating_mul(NEAR_FULL_PERCENT);
    let db_bytes = [DB_FILE.to_string(), format!("{DB_FILE}-wal")]
        .iter()
        .filter_map(|f| std::fs::metadata(data_dir.join(f)).ok())
        .map(|m| m.len())
        .sum();
    let disk_free = rp_models::free_space(data_dir).ok();
    Ok(Overview { backups, total_bytes, limit_bytes, near_full, db_bytes, disk_free })
}

/// After the limit changed: applies the retention rule and `limit` to the backups folder right
/// away (oldest automatic ones first, manual ones never). Answers how many went.
pub fn apply_limit(data_dir: &Path, limit: u64) -> Res<usize> {
    let _one = hold();
    prune(&data_dir.join(DIR), limit, None)
}

/// Deletes several backups of the backups folder by their names from the list, all or none:
/// every name is checked first, then every file is renamed to a name the list ignores (when one
/// cannot be — e.g. it is open elsewhere — the renamed ones get their names back and none is
/// deleted), and only then are they deleted. Err: `bad_name` (not a backup name, or a copy made by
/// hand outside the folder), `backup_missing`, `storage`.
pub fn remove_many(data_dir: &Path, names: &[String]) -> Result<Vec<Info>, &'static str> {
    let _one = hold();
    let mut found: Vec<(PathBuf, Info)> = Vec::new();
    for name in names {
        if found.iter().any(|(_, i)| &i.name == name) {
            continue;
        }
        let (path, reason) = locate(data_dir, name)?;
        if reason == Reason::Hand {
            return Err("bad_name");
        }
        let gone = info(&path, name.clone(), reason).map_err(|_| "storage")?;
        found.push((path, gone));
    }
    let aside = |p: &Path| {
        let mut name = p.as_os_str().to_owned();
        name.push(".deleting");
        PathBuf::from(name)
    };
    for (n, (path, _)) in found.iter().enumerate() {
        if std::fs::rename(path, aside(path)).is_err() {
            for (back, _) in &found[..n] {
                let _ = std::fs::rename(aside(back), back);
            }
            return Err("storage");
        }
    }
    let mut out = Vec::with_capacity(found.len());
    for (path, gone) in found {
        // Renamed, so nothing holds it open: removing it does not fail for another reason than
        // the disk itself, and a file left over keeps a name the list ignores.
        std::fs::remove_file(aside(&path)).map_err(|_| "storage")?;
        out.push(gone);
    }
    Ok(out)
}

/// Deletes every automatic backup (daily, migration, before-restore) made more than `days` days
/// before `now_ms`; manual ones and copies made by hand stay. Answers what went.
pub fn remove_older(data_dir: &Path, days: u32, now_ms: i64) -> Res<Vec<Info>> {
    let _one = hold();
    let dir = data_dir.join(DIR);
    let before = now_ms - i64::from(days) * DAY_MS;
    let mut out = Vec::new();
    for f in made_in(&dir)? {
        if f.reason.automatic() && f.secs * 1000 < before {
            let path = dir.join(&f.name);
            let gone = info(&path, f.name, f.reason)?;
            std::fs::remove_file(&path)?;
            out.push(gone);
        }
    }
    Ok(out)
}

/// Deletes one backup of the backups folder by its name from the list. Err: `bad_name` (not a
/// backup name, or a copy made by hand outside the folder), `backup_missing`, `storage`.
pub fn remove(data_dir: &Path, name: &str) -> Result<Info, &'static str> {
    let _one = hold();
    let (path, reason) = locate(data_dir, name)?;
    if reason == Reason::Hand {
        return Err("bad_name");
    }
    let gone = info(&path, name.to_string(), reason).map_err(|_| "storage")?;
    std::fs::remove_file(&path).map_err(|_| "storage")?;
    Ok(gone)
}

/// `2026-11-15-130501` (UTC) for Unix seconds.
fn stamp(secs: i64) -> String {
    let t = logs::fmt_ts(secs * 1000, 0); // `2026-11-15 13:05:01.000`
    format!("{}-{}{}{}", &t[..10], &t[11..13], &t[14..16], &t[17..19])
}

/// Copies one whole database into another in a single pass (one consistent read of the source).
/// Waits up to 10 s while another connection holds a lock it needs.
pub(crate) fn copy(from: &Connection, to: &mut Connection) -> Res<()> {
    let backup = Backup::new(from, to)?;
    let started = Instant::now();
    loop {
        match backup.step(-1)? {
            StepResult::Done => return Ok(()),
            StepResult::More => {}
            StepResult::Busy | StepResult::Locked if started.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => return Err("the database stayed locked".into()),
        }
    }
}

/// Writes a backup of `src` (a connection to the app's database) into the backups folder, then
/// applies the retention rule and the limit chosen in that database (`limit_in`). Answers the new
/// backup and how many old ones went. A manual one that would not fit: `LimitReached`.
pub fn create_from(src: &Connection, data_dir: &Path, reason: Reason) -> Res<(Info, usize)> {
    create_within(src, data_dir, reason, limit_in(src))
}

/// `create_from` with the folder's limit given.
pub(crate) fn create_within(src: &Connection, data_dir: &Path, reason: Reason, limit: u64) -> Res<(Info, usize)> {
    let held = hold();
    write(&held, src, data_dir, reason, limit, None)
}

/// The `before-restore` backup of a restore that holds the folder (`hold`) while it restores
/// `source`: the pruning after it never deletes `source`, whatever the rule or the limit say
/// (the next backup prunes it as usual).
pub(crate) fn create_before_restore(held: &Held, src: &Connection, data_dir: &Path, source: &Path) -> Res<(Info, usize)> {
    let keep = source.file_name().and_then(|n| n.to_str());
    write(held, src, data_dir, Reason::BeforeRestore, limit_in(src), keep)
}

/// Writes the backup and prunes (sparing `keep`), with the folder held.
fn write(_held: &Held, src: &Connection, data_dir: &Path, reason: Reason, limit: u64, keep: Option<&str>) -> Res<(Info, usize)> {
    let dir = data_dir.join(DIR);
    std::fs::create_dir_all(&dir)?;
    if reason == Reason::Manual {
        // The copy has exactly the pages the source has; automatic ones make room, manual ones not.
        let pages: i64 = src.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let page: i64 = src.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        let manual: u64 = made_in(&dir)?.iter().filter(|f| f.reason == Reason::Manual).map(|f| f.size).sum();
        if manual + (pages * page) as u64 > limit {
            return Err(Box::new(LimitReached(limit)));
        }
    }
    // Each backup gets a second of its own (whatever its reason), later than every name there,
    // so the names keep the order they were written in (pruning relies on it): several in the same
    // second take the following seconds. Only after an hour of steps (a clock set back) does a free
    // second count, whatever its place.
    let taken: Vec<String> = files_in(&dir)?.into_iter().filter(|n| parse_auto(n).is_some()).collect();
    let newest = taken.iter().map(|n| &n[AUTO_PREFIX.len()..AUTO_PREFIX.len() + STAMP_LEN]).max().map(str::to_string);
    let mut secs = logs::now_ms() / 1000;
    let mut steps = 0;
    let name = loop {
        let st = stamp(secs);
        let at = format!("{AUTO_PREFIX}{st}-");
        let in_order = steps >= 3600 || newest.as_deref().is_none_or(|m| st.as_str() > m);
        if in_order && !taken.iter().any(|n| n.starts_with(&at)) {
            break format!("{at}{}.db", reason.as_str());
        }
        secs += 1;
        steps += 1;
    };
    let path = dir.join(&name);
    // Written under a name the list ignores, so a cut-off copy never shows up as a backup.
    let part = dir.join(format!("{name}.part"));
    let written = (|| -> Res<()> {
        let mut dst = Connection::open(&part)?;
        copy(src, &mut dst)?;
        // One self-contained file (the copy may carry the WAL flag of the source).
        let _: String = dst.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))?;
        dst.close().map_err(|(_, e)| e)?;
        std::fs::rename(&part, &path)?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    let made = info(&path, name, reason)?;
    let pruned = prune(&dir, limit, keep)?;
    Ok((made, pruned))
}

/// Opens the app's database next to the running store (never creates it).
fn open_live(data_dir: &Path) -> Res<Connection> {
    let mut flags = OpenFlags::default();
    flags.remove(OpenFlags::SQLITE_OPEN_CREATE);
    let conn = Connection::open_with_flags(data_dir.join(DB_FILE), flags)?;
    conn.busy_timeout(Duration::from_secs(10))?;
    Ok(conn)
}

/// A backup of the app's database through a connection of its own (the running store keeps
/// working meanwhile).
pub fn create(data_dir: &Path, reason: Reason) -> Res<(Info, usize)> {
    create_from(&open_live(data_dir)?, data_dir, reason)
}

/// Deletes what the retention rule and `limit` drop, never `keep` (see `plan`). Answers how many
/// went.
fn prune(dir: &Path, limit: u64, keep: Option<&str>) -> Res<usize> {
    let (_, gone) = plan(&made_in(dir)?, limit, keep);
    for name in &gone {
        std::fs::remove_file(dir.join(name))?;
    }
    Ok(gone.len())
}

/// Before the migrations of this start: a backup when the database already exists with data
/// (a schema version above 0) and is older than `target`.
pub fn before_migration(data_dir: &Path, target: usize) -> Res<Option<(Info, usize)>> {
    if !data_dir.join(DB_FILE).is_file() {
        return Ok(None);
    }
    let conn = open_live(data_dir)?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version <= 0 || version as usize >= target {
        return Ok(None);
    }
    create_from(&conn, data_dir, Reason::Migration).map(Some)
}

/// Setting: how often the app backs up by itself (`daily`, `weekly`, `off`).
pub const RHYTHM_KEY: &str = "backup_rhythm";

/// How often the app makes its own backup. Backups before a migration or a restore are made
/// regardless of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Rhythm {
    #[default]
    Daily,
    Weekly,
    Off,
}

impl Rhythm {
    pub const ALL: [Rhythm; 3] = [Rhythm::Daily, Rhythm::Weekly, Rhythm::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            Rhythm::Daily => "daily",
            Rhythm::Weekly => "weekly",
            Rhythm::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Rhythm> {
        Rhythm::ALL.into_iter().find(|r| r.as_str() == s)
    }

    /// When the next automatic backup is due after the newest one (`None`: none yet); `None` =
    /// never. A time in the past means "now".
    pub fn next_due(self, newest: Option<i64>, now_ms: i64) -> Option<i64> {
        let every = match self {
            Rhythm::Daily => DAY_MS,
            Rhythm::Weekly => 7 * DAY_MS,
            Rhythm::Off => return None,
        };
        Some(newest.map_or(now_ms, |t| t + every))
    }
}

/// The rhythm chosen in the database behind `conn`; daily when none is chosen.
pub fn rhythm_in(conn: &Connection) -> Rhythm {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [RHYTHM_KEY], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| Rhythm::parse(&v))
        .unwrap_or_default()
}

/// When the app makes its next backup by itself (ms since 1970; in the past: at the next
/// check); `None` when the rhythm is off.
pub fn next_automatic(data_dir: &Path, now_ms: i64) -> Res<Option<i64>> {
    // A data folder without a database yet has no choice stored: daily.
    let rhythm = open_live(data_dir).map(|c| rhythm_in(&c)).unwrap_or_default();
    let newest = list(data_dir)?.into_iter().filter(|b| b.reason == Reason::Daily).map(|b| b.time).max();
    Ok(rhythm.next_due(newest, now_ms))
}

/// An automatic backup is due: none yet, or the newest is as old as the chosen rhythm says
/// (a day, a week; never when it is off).
pub fn daily_due(data_dir: &Path, now_ms: i64) -> Res<bool> {
    Ok(next_automatic(data_dir, now_ms)?.is_some_and(|t| t <= now_ms))
}

/// Opens a backup file to read it: read-only and `immutable` (no `-wal`/`-shm` files appear
/// next to it, also for a hand-made copy of a database that was in WAL mode).
pub(crate) fn open_source(path: &Path) -> rusqlite::Result<Connection> {
    let p = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file:");
    if !p.starts_with('/') {
        uri.push('/'); // `file:/C:/…`
    }
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'_' | b'.' | b'~' => uri.push(b as char),
            _ => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri.push_str("?immutable=1");
    Connection::open_with_flags(uri, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX)
}

/// Whether a file can be restored: an intact database of this app, not newer than this build.
/// Err: `bad_backup` or `backup_newer`.
pub fn check(path: &Path, target: usize) -> Result<(), &'static str> {
    let conn = open_source(path).map_err(|_| "bad_backup")?;
    let ok: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(|_| "bad_backup")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|_| "bad_backup")?;
    let has_characters: bool = conn
        .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'characters')", [], |r| r.get(0))
        .map_err(|_| "bad_backup")?;
    if ok != "ok" || version <= 0 || !has_characters {
        return Err("bad_backup");
    }
    if version as usize > target {
        return Err("backup_newer");
    }
    Ok(())
}

/// The log entry of a written backup (name, reason, size; never anything from inside it).
pub fn log_made(log: &Logger, made: &(Info, usize), ms: u128) {
    let (b, pruned) = made;
    log.write(Entry::new(Level::Info, "server", "backup", format!("backup created ({})", b.reason.as_str())).data(json!({
        "name": b.name, "reason": b.reason, "bytes": b.size, "ms": ms as u64, "pruned": pruned,
    })));
}

/// Runs for the life of the server: a daily backup at start when none is from the last 24 h,
/// then whenever the newest is a day old.
pub fn spawn_daily(data_dir: PathBuf, log: Logger) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CHECK_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let dir = data_dir.clone();
            let started = Instant::now();
            let made = tokio::task::spawn_blocking(move || -> Res<Option<(Info, usize)>> {
                if !daily_due(&dir, logs::now_ms())? {
                    return Ok(None);
                }
                create(&dir, Reason::Daily).map(Some)
            })
            .await;
            match made {
                Ok(Ok(Some(made))) => log_made(&log, &made, started.elapsed().as_millis()),
                Ok(Ok(None)) => {}
                Ok(Err(e)) => log.write(Entry::new(Level::Error, "server", "backup", format!("daily backup failed: {e}"))),
                Err(e) => log.write(Entry::new(Level::Error, "server", "backup", format!("daily backup failed: {e}"))),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rhythm_decides_when_the_next_backup_is_due() {
        let now = 100 * DAY_MS;
        // None yet: at once, unless backups are off.
        assert_eq!(Rhythm::Daily.next_due(None, now), Some(now));
        assert_eq!(Rhythm::Weekly.next_due(None, now), Some(now));
        assert_eq!(Rhythm::Off.next_due(None, now), None);
        let newest = now - 2 * DAY_MS;
        assert_eq!(Rhythm::Daily.next_due(Some(newest), now), Some(now - DAY_MS), "overdue");
        assert_eq!(Rhythm::Weekly.next_due(Some(newest), now), Some(now + 5 * DAY_MS));
        assert_eq!(Rhythm::Off.next_due(Some(newest), now), None);
        for r in Rhythm::ALL {
            assert_eq!(Rhythm::parse(r.as_str()), Some(r));
        }
        assert_eq!(Rhythm::parse("hourly"), None);
    }

    fn made(date: &str, reason: Reason, size: u64) -> Made {
        let name = format!("{AUTO_PREFIX}{date}-030000-{}.db", reason.as_str());
        let secs = stamp_secs(&name).unwrap();
        Made { name, reason, secs, size }
    }

    /// Newest first, as `made_in` answers.
    fn sorted(mut files: Vec<Made>) -> Vec<Made> {
        files.sort_by_key(|f| std::cmp::Reverse(f.secs));
        files
    }

    fn dates(names: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut out: Vec<String> = names.into_iter().map(|n| n[AUTO_PREFIX.len()..AUTO_PREFIX.len() + 10].to_string()).collect();
        out.sort();
        out
    }

    #[test]
    fn times_and_weeks_from_names() {
        assert_eq!(stamp_secs("rp-app-1970-01-01-000001-daily.db"), Some(1));
        assert_eq!(stamp_secs("rp-app-2026-11-15-130501-manual.db"), Some(1_794_747_901));
        assert_eq!(stamp_secs("rp-app.db.backup-x"), None);
        let week = |d: &str| week_of(stamp_secs(&format!("rp-app-{d}-030000-daily.db")).unwrap());
        // Monday to Sunday.
        assert_eq!(week("2026-11-09"), week("2026-11-15"));
        assert_ne!(week("2026-11-15"), week("2026-11-16"));
        assert_ne!(week("2026-11-08"), week("2026-11-09"));
    }

    /// A daily backup every day from 1 October to Sunday 15 November 2026: the last 7 days stay
    /// daily, then the Monday (the oldest) of each of the 4 weeks before; the day after, the week
    /// of the 9th joins the weekly ones and the oldest weekly one goes.
    #[test]
    fn seven_daily_and_four_weekly() {
        let mut days: Vec<String> = (1..=31).map(|d| format!("2026-10-{d:02}")).collect();
        days.extend((1..=15).map(|d| format!("2026-11-{d:02}")));
        let files = sorted(days.iter().map(|d| made(d, Reason::Daily, 1)).collect());
        let (weekly, gone) = plan(&files, u64::MAX, None);
        assert_eq!(dates(weekly.iter().cloned()), ["2026-10-12", "2026-10-19", "2026-10-26", "2026-11-02"]);
        assert_eq!(gone.len(), 46 - 7 - 4);
        let kept: Vec<Made> = files.into_iter().filter(|f| !gone.contains(&f.name)).collect();
        assert_eq!(dates(kept.iter().map(|f| f.name.clone()))[4..], ["2026-11-09", "2026-11-10", "2026-11-11", "2026-11-12", "2026-11-13", "2026-11-14", "2026-11-15"]);

        // One day later the rule keeps what it kept, moved on by a day.
        let mut next = kept;
        next.push(made("2026-11-16", Reason::Daily, 1));
        let (weekly, gone) = plan(&sorted(next), u64::MAX, None);
        assert_eq!(dates(weekly.iter().cloned()), ["2026-10-19", "2026-10-26", "2026-11-02", "2026-11-09"]);
        assert_eq!(dates(gone), ["2026-10-12"]);
    }

    /// Migration and restore ones: the newest 3 each; manual ones are never dropped, also not for
    /// the limit, where the oldest automatic ones go first and the newest file always stays.
    #[test]
    fn other_kinds_and_the_limit() {
        let mut files = Vec::new();
        for d in 1..=5 {
            files.push(made(&format!("2026-02-0{d}"), Reason::Migration, 100));
            files.push(made(&format!("2026-03-0{d}"), Reason::BeforeRestore, 100));
            files.push(made(&format!("2026-01-0{d}"), Reason::Manual, 100));
        }
        files.push(made("2026-04-01", Reason::Daily, 100));
        let files = sorted(files);
        let (_, gone) = plan(&files, u64::MAX, None);
        assert_eq!(dates(gone), ["2026-02-01", "2026-02-02", "2026-03-01", "2026-03-02"]);
        // 12 stay (1200 bytes); 1000 allowed: the two oldest automatic ones go as well.
        let (_, gone) = plan(&files, 1000, None);
        assert_eq!(dates(gone), ["2026-02-01", "2026-02-02", "2026-02-03", "2026-02-04", "2026-03-01", "2026-03-02"]);
        // Nothing allowed: every automatic one but the newest goes, the manual ones stay.
        let (_, gone) = plan(&files, 0, None);
        let kept: Vec<&Made> = files.iter().filter(|f| !gone.contains(&f.name)).collect();
        assert_eq!(kept.iter().filter(|f| f.reason == Reason::Manual).count(), 5);
        assert_eq!(kept.iter().filter(|f| f.reason != Reason::Manual).map(|f| f.name.as_str()).collect::<Vec<_>>(), [files[0].name.as_str()]);
        // The backup being restored stays against the rule and the limit; only it.
        let oldest_restore = files.iter().rev().find(|f| f.reason == Reason::BeforeRestore).unwrap().name.clone();
        let (_, gone) = plan(&files, 0, Some(&oldest_restore));
        assert!(!gone.contains(&oldest_restore));
        assert_eq!(gone.len(), files.iter().filter(|f| f.reason != Reason::Manual).count() - 2);
    }
}
