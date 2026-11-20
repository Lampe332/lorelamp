//! Import of many files at once: every file is read on the blocking pool (several at a time),
//! zips with many cards are unpacked entry by entry, and what was found is stored in as few
//! transactions as the memory allows (one for every ordinary import).
//! Also the persona formats: SillyTavern backup / `settings.json` (+ `User Avatars`), RisuAI
//! persona PNG and the app's own persona zip.
//!
//! Memory: uploads go to a temporary file first; a file or zip entry is only read into memory
//! once the program-wide budget ([`WORK_BUDGET`], shared by every import, link and zip) has room
//! for it; at most [`JOBS_AT_ONCE`] bulk imports run at once; pictures are decoded at most
//! `imaging::DECODERS` at a time. So many uploads at once, zip bombs or a huge collection take
//! time or end in `too_large`, never all the RAM.
//!
//! Nothing here logs file names, card or persona text.

use std::collections::HashMap;
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::distr::{Alphanumeric, SampleString};
use rp_core::formats::{self, Content, ImportError, ImageKind, MAX_INPUT, MAX_ZIP_ENTRIES};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::task::{JoinHandle, JoinSet};

use crate::chat::Hub;
use crate::imaging;
use crate::store::{self, content_hash, Added, NewCard, NewItem, NewPersona, PersonaEdit};

/// Largest upload of one import request (all files together). It is written to a temporary file
/// while it arrives, so this bounds the disk, not the RAM.
pub const MAX_UPLOAD: u64 = 1 << 30;
/// Most bytes the entries of all zips of one import may unpack to together.
pub const MAX_UNZIPPED: u64 = 4 << 30;
/// Most bytes a zip entry may unpack to per compressed byte (deflate reaches ~1000; card PNGs
/// ~1, card JSON ~3–10), checked for entries above 1 MiB.
pub const MAX_RATIO: u64 = 100;
const RATIO_FROM: u64 = 1 << 20;
/// Persona uploads are read in memory: a smaller limit (for the upload and again for what its
/// zips unpack to).
pub const MAX_PERSONA_UPLOAD: u64 = 256 << 20;

/// Longest persona name, description and the other persona fields (as the persona routes allow).
pub const PERSONA_NAME_MAX: usize = 40;
pub const PERSONA_TEXT_MAX: usize = 20_000;
pub const PERSONA_PRONOUNS_MAX: usize = 40;
pub const PERSONA_FIELD_MAX: usize = 5_000;

/// Key under which the app keeps its own persona fields in exchange files (RisuAI JSON and
/// SillyTavern `persona_descriptions`); other apps ignore it.
pub const RP_PERSONA_KEY: &str = "rp_app";

// ---------------------------------------------------------------------------------------------
// Memory shared by every import

/// Bytes of files being read and parsed at the same time in the whole program (a file of `n`
/// bytes takes [`work_cost`]`(n)`). Pictures have their own limit in `imaging`.
pub const WORK_BUDGET: u32 = 512 << 20;
static WORK: Semaphore = Semaphore::const_new(WORK_BUDGET as usize);
/// Bulk imports (uploads, SillyTavern) that run at the same time; more wait for their turn.
pub const JOBS_AT_ONCE: usize = 2;
static JOBS: Semaphore = Semaphore::const_new(JOBS_AT_ONCE);
/// The cards a job found are written to the database whenever they hold about this many bytes,
/// so a large collection is never all in memory at once.
const STORE_EVERY: u64 = 128 << 20;

/// Room a file takes while it is read and parsed: its bytes, the copies of parsing (picture,
/// card text) and a little more; never more than the whole budget.
fn work_cost(len: u64) -> u32 {
    len.saturating_mul(3).saturating_add(4 << 20).min(u64::from(WORK_BUDGET)) as u32
}

/// Waits until the budget has room for reading and parsing `len` bytes.
pub async fn reserve(len: u64) -> SemaphorePermit<'static> {
    let permit = WORK.acquire_many(work_cost(len)).await.expect("the budget is never closed");
    #[cfg(test)]
    WORK_PEAK.fetch_max(WORK_BUDGET as usize - WORK.available_permits(), Ordering::SeqCst);
    permit
}

/// [`reserve`] from a thread of the blocking pool (zip entries are read there).
fn reserve_blocking(len: u64) -> SemaphorePermit<'static> {
    tokio::runtime::Handle::current().block_on(reserve(len))
}

/// Waits for the turn of a bulk import (at most [`JOBS_AT_ONCE`] at once).
pub async fn turn() -> SemaphorePermit<'static> {
    JOBS.acquire().await.expect("the job queue is never closed")
}

/// Most of the budget ever in use at once (tests).
#[cfg(test)]
pub static WORK_PEAK: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------------------------
// Files

/// What one file (or one zip entry) turned out to be.
pub enum Prepared {
    /// Cards and/or personas, ready to store (BYAF and Perchance files can hold several cards).
    Items(Vec<NewItem>),
    Error(&'static str),
}

/// The result of one item for the API.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FileResult {
    pub file: String,
    /// "imported" | "duplicate" | "error"
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<i64>,
    /// A chat taken over from SillyTavern.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<i64>,
    /// A chat from SillyTavern: how many of its lines were left out as `is_system` (hidden there,
    /// no part of the story).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_skipped: Option<usize>,
}

impl FileResult {
    pub fn error(file: &str, code: &'static str) -> FileResult {
        FileResult { file: file.to_string(), status: "error", code: Some(code), character_id: None, persona_id: None, chat_id: None, system_skipped: None }
    }

    pub fn added(file: &str, added: Added) -> FileResult {
        let (status, character_id, persona_id) = match added {
            Added::Character(id) => ("imported", Some(id), None),
            Added::DuplicateCharacter(id) => ("duplicate", Some(id), None),
            Added::Persona(id) => ("imported", None, Some(id)),
            Added::DuplicatePersona(id) => ("duplicate", None, Some(id)),
        };
        FileResult { file: file.to_string(), status, code: None, character_id, persona_id, chat_id: None, system_skipped: None }
    }

    /// A chat that was stored (`new`) or was already there, with its character and the number of
    /// system lines left out.
    pub fn chat(file: &str, new: bool, character_id: i64, chat_id: i64, system_skipped: usize) -> FileResult {
        let status = if new { "imported" } else { "duplicate" };
        FileResult {
            file: file.to_string(),
            status,
            code: None,
            character_id: Some(character_id),
            persona_id: None,
            chat_id: Some(chat_id),
            system_skipped: Some(system_skipped),
        }
    }
}

fn cut(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect::<String>().trim().to_string()
}

/// The app's own persona fields from an exchange file (missing ones stay empty).
fn rp_fields(edit: &mut PersonaEdit, ext: &Value) {
    let s = |key: &str, max: usize| ext[key].as_str().map(|t| cut(t, max));
    edit.pronouns = s("pronouns", PERSONA_PRONOUNS_MAX);
    edit.appearance = s("appearance", PERSONA_FIELD_MAX);
    edit.personality = s("personality", PERSONA_FIELD_MAX);
    edit.background = s("background", PERSONA_FIELD_MAX);
    edit.every_turn = ext["every_turn"].as_bool();
    edit.card_priority = ext["card_priority"]
        .as_str()
        .filter(|p| rp_core::prompt::Priority::parse(p).is_some())
        .map(str::to_string);
}

/// A persona from name and description (cut to the limits). None: no name.
pub fn persona_edit(name: &str, description: &str, ext: &Value) -> Option<PersonaEdit> {
    let name = cut(name, PERSONA_NAME_MAX);
    if name.is_empty() {
        return None;
    }
    let mut edit = PersonaEdit { name: Some(name), description: Some(cut(description, PERSONA_TEXT_MAX)), ..PersonaEdit::default() };
    rp_fields(&mut edit, ext);
    Some(edit)
}

/// The picture as PNG with its content type, for a persona.
fn persona_picture(bytes: Vec<u8>) -> Option<(Vec<u8>, &'static str)> {
    let mime = store::image_mime(&bytes)?;
    Some((bytes, mime))
}

/// A card or persona file (a plain zip of files is `no_card` here; [`Job`] unpacks those). Runs
/// on the blocking pool: it decodes pictures and makes thumbnails.
pub fn prepare(bytes: &[u8]) -> Prepared {
    match formats::detect_and_parse(bytes) {
        Ok(file) => match file.content {
            Content::Cards(cards) => {
                let mut items = Vec::with_capacity(cards.len());
                for c in cards {
                    let Some(hash) = content_hash(&c.raw) else { return Prepared::Error(ImportError::Broken.code()) };
                    let (png, thumb) = match &c.image {
                        Some(img) => (imaging::to_png(&img.bytes), imaging::thumb_webp(&img.bytes)),
                        None => (None, None),
                    };
                    items.push(NewItem::Card(NewCard { name: c.card.name.clone(), raw: c.raw, png, thumb, hash, source_url: None }));
                }
                Prepared::Items(items)
            }
            Content::Persona { persona, image } => {
                let Some(edit) = persona_edit(&persona.name, &persona.prompt, &persona.raw[RP_PERSONA_KEY]) else {
                    return Prepared::Error(ImportError::Broken.code());
                };
                let avatar = image.and_then(|i| persona_picture(i.bytes));
                Prepared::Items(vec![NewItem::Persona(NewPersona { edit, avatar })])
            }
        },
        Err(e) => Prepared::Error(e.code()),
    }
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

// ---------------------------------------------------------------------------------------------
// Zips

/// One file of a zip: its name, and its bytes with their room in the budget (give the room back
/// by dropping it once the bytes are no longer needed), or why it was not read.
pub struct Entry {
    pub name: String,
    pub read: Result<(Vec<u8>, SemaphorePermit<'static>), &'static str>,
}

/// Reads the files of a zip one at a time (folders and macOS/hidden leftovers skipped) and hands
/// each to `each` (false: stop). Must run on the blocking pool. Per entry: it waits for room in
/// the budget before it is inflated; more than `limit` bytes, or more than [`MAX_RATIO`] bytes per
/// compressed byte, is `too_large` for that entry. `unzipped` counts the bytes of all zips of one
/// import against `total`: past it the zip ends with `Err(too_large)`. At most
/// [`MAX_ZIP_ENTRIES`] entries. Sizes come from the headers and are checked again while
/// inflating (a lying header gives `broken`, never more bytes).
pub fn each_entry<R: Read + Seek>(reader: R, limit: u64, unzipped: &AtomicU64, total: u64, mut each: impl FnMut(Entry) -> bool) -> Result<(), &'static str> {
    let (too_large, broken) = (ImportError::TooLarge.code(), ImportError::Broken.code());
    let mut zip = zip::ZipArchive::new(reader).map_err(|_| broken)?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(too_large);
    }
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|_| broken)?;
        let name = file.name().replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or("");
        if file.is_dir() || base.is_empty() || base.starts_with('.') || name.starts_with("__MACOSX/") {
            continue;
        }
        let size = file.size();
        let read = if size > limit || (size > RATIO_FROM && size / file.compressed_size().max(1) > MAX_RATIO) {
            Err(too_large)
        } else if unzipped.fetch_add(size, Ordering::SeqCst) + size > total {
            return Err(too_large);
        } else {
            let permit = reserve_blocking(size);
            let mut buf = Vec::with_capacity(size as usize);
            match (&mut file).take(size + 1).read_to_end(&mut buf) {
                Ok(_) if buf.len() as u64 == size => Ok((buf, permit)),
                _ => Err(broken),
            }
        };
        drop(file);
        if !each(Entry { name, read }) {
            break;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Jobs

/// Order of a result: (file index, entry index inside a zip + 1, or 0 for the file itself).
type Key = (usize, usize);

/// What one file or zip entry gave, and about how many bytes its items keep in memory.
struct Part {
    key: Key,
    file: String,
    prepared: Prepared,
    weight: u64,
}

/// Where a file of a job is (always on disk: uploads are written there while they arrive).
enum Source {
    /// A file of another app: only read.
    Disk(PathBuf),
    /// An upload in the job's temporary folder: deleted once read.
    Spooled(PathBuf),
}

/// The parts of a job that its tasks share.
struct Shared {
    hub: Arc<Hub>,
    id: String,
    cpus: Semaphore,
    done: AtomicUsize,
    total: AtomicUsize,
    last: Mutex<Instant>,
    /// Bytes the job's zips unpacked so far (against [`MAX_UNZIPPED`]).
    unzipped: AtomicU64,
}

/// Fewest milliseconds between two progress events of a job (the last one always goes out).
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

impl Shared {
    fn progress(&self, force: bool) {
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        if force || last.elapsed() >= PROGRESS_EVERY {
            *last = Instant::now();
            let (done, total) = (self.done.load(Ordering::SeqCst), self.total.load(Ordering::SeqCst));
            self.hub.import_progress(&self.id, done, total, force);
        }
    }

    fn count_done(&self) {
        self.done.fetch_add(1, Ordering::SeqCst);
        self.progress(false);
    }

    /// Parses bytes on the blocking pool once a CPU is free; `permit` (their room in the budget)
    /// goes back when parsing is done. true: the bytes are a plain zip of files.
    async fn prepare(self: Arc<Self>, bytes: Vec<u8>, permit: SemaphorePermit<'static>) -> (Prepared, bool) {
        let cpu = self.cpus.acquire().await.expect("the semaphore is never closed");
        let out = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let prepared = prepare(&bytes);
            let plain_zip = is_zip(&bytes) && matches!(prepared, Prepared::Error(code) if code == ImportError::NoCard.code());
            (prepared, plain_zip)
        })
        .await
        .unwrap_or((Prepared::Error("broken"), false));
        drop(cpu);
        out
    }
}

/// One running import: files go in as they arrive, at most one per CPU is parsed at a time, and
/// every device hears `{"type":"import","job","done","total","finished"}`.
pub struct Job {
    shared: Arc<Shared>,
    set: JoinSet<Vec<Part>>,
    ready: Vec<Part>,
    /// The temporary folder of uploads, made on first use.
    spool: Option<PathBuf>,
    _turn: SemaphorePermit<'static>,
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Some(dir) = &self.spool {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Job {
    /// A new job once it is its turn (at most [`JOBS_AT_ONCE`] run at once).
    pub async fn start(hub: Arc<Hub>, id: String) -> Job {
        let turn = turn().await;
        let cpus = std::thread::available_parallelism().map_or(4, |n| n.get());
        let shared = Shared {
            hub,
            id,
            cpus: Semaphore::new(cpus),
            done: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            last: Mutex::new(Instant::now() - PROGRESS_EVERY),
            unzipped: AtomicU64::new(0),
        };
        Job { shared: Arc::new(shared), set: JoinSet::new(), ready: vec![], spool: None, _turn: turn }
    }

    /// A new temporary file for upload number `n` (deleted with the job).
    pub fn spool_file(&mut self, n: usize) -> std::io::Result<PathBuf> {
        let dir = match &self.spool {
            Some(dir) => dir.clone(),
            None => {
                let dir = std::env::temp_dir().join(format!("lorelamp-import-{}", Alphanumeric.sample_string(&mut rand::rng(), 12)));
                std::fs::create_dir_all(&dir)?;
                self.spool = Some(dir.clone());
                dir
            }
        };
        Ok(dir.join(format!("{n}.part")))
    }

    fn spawn(&mut self, n: usize, file: String, source: Source) {
        self.shared.total.fetch_add(1, Ordering::SeqCst);
        self.set.spawn(run_file(self.shared.clone(), n, file, source));
    }

    /// A file of another app on disk (only read, never changed).
    pub fn add_file(&mut self, key: Key, file: String, path: PathBuf) {
        self.spawn(key.0, file, Source::Disk(path));
    }

    /// An upload written by [`Job::spool_file`] (deleted once read).
    pub fn add_spooled(&mut self, key: Key, file: String, path: PathBuf) {
        self.spawn(key.0, file, Source::Spooled(path));
    }

    /// A file that was refused before reading (e.g. over the size limit).
    pub fn refuse(&mut self, key: Key, file: String, code: &'static str) {
        self.shared.total.fetch_add(1, Ordering::SeqCst);
        self.shared.done.fetch_add(1, Ordering::SeqCst);
        self.ready.push(Part { key, file, prepared: Prepared::Error(code), weight: 0 });
    }

    /// Waits for every file (also the entries of zips), stores what was found (in one
    /// transaction, or one per [`STORE_EVERY`] bytes of a large import) and answers one result
    /// per card or persona (per file for errors), in upload order.
    pub async fn finish(mut self) -> Result<Vec<FileResult>, store::StoreError> {
        let mut results: Vec<((Key, usize), FileResult)> = vec![];
        let mut waiting: Vec<((Key, usize), String, NewItem)> = vec![];
        let take = |parts: Vec<Part>, results: &mut Vec<((Key, usize), FileResult)>, waiting: &mut Vec<((Key, usize), String, NewItem)>| {
            let mut weight = 0;
            for part in parts {
                match part.prepared {
                    Prepared::Items(found) => {
                        weight += part.weight;
                        waiting.extend(found.into_iter().enumerate().map(|(i, item)| ((part.key, i), part.file.clone(), item)));
                    }
                    Prepared::Error(code) => results.push(((part.key, 0), FileResult::error(&part.file, code))),
                }
            }
            weight
        };
        take(std::mem::take(&mut self.ready), &mut results, &mut waiting);
        let mut held = 0u64;
        while let Some(joined) = self.set.join_next().await {
            let parts = joined.unwrap_or_else(|_| vec![Part { key: (usize::MAX, 0), file: String::new(), prepared: Prepared::Error("broken"), weight: 0 }]);
            held += take(parts, &mut results, &mut waiting);
            if held >= STORE_EVERY {
                self.store(&mut waiting, &mut results).await?;
                held = 0;
            }
        }
        self.store(&mut waiting, &mut results).await?;
        results.sort_by_key(|(key, _)| *key);
        self.shared.progress(true);
        Ok(results.into_iter().map(|(_, r)| r).collect())
    }

    /// Stores the waiting items (in upload order) in one transaction.
    async fn store(&self, waiting: &mut Vec<((Key, usize), String, NewItem)>, results: &mut Vec<((Key, usize), FileResult)>) -> Result<(), store::StoreError> {
        if waiting.is_empty() {
            return Ok(());
        }
        let mut batch = std::mem::take(waiting);
        batch.sort_by_key(|(key, _, _)| *key);
        let (owners, items): (Vec<_>, Vec<_>) = batch.into_iter().map(|(key, file, item)| ((key, file), item)).unzip();
        let hub = self.shared.hub.clone();
        let added = tokio::task::spawn_blocking(move || hub.store(|s| s.import_items(&items))).await??;
        results.extend(owners.into_iter().zip(added).map(|((key, file), added)| (key, FileResult::added(&file, added))));
        Ok(())
    }
}

/// Reads one file of a job: a card or persona file is parsed once the budget has room for it; a
/// plain zip (also one too large to be a card file) is unpacked entry by entry.
async fn run_file(shared: Arc<Shared>, n: usize, file: String, source: Source) -> Vec<Part> {
    let (path, spooled) = match source {
        Source::Disk(path) => (path, false),
        Source::Spooled(path) => (path, true),
    };
    let parts = read_file(&shared, n, &file, &path).await;
    if spooled {
        let _ = tokio::fs::remove_file(path).await;
    }
    parts
}

async fn read_file(shared: &Arc<Shared>, n: usize, file: &str, path: &std::path::Path) -> Vec<Part> {
    let one = |prepared: Prepared, weight: u64| {
        shared.count_done();
        vec![Part { key: (n, 0), file: file.to_string(), prepared, weight }]
    };
    let Ok(len) = tokio::fs::metadata(path).await.map(|m| m.len()) else { return one(Prepared::Error("broken"), 0) };
    if len <= MAX_INPUT as u64 {
        let permit = reserve(len).await;
        let Ok(bytes) = tokio::fs::read(path).await else { return one(Prepared::Error("broken"), 0) };
        let (prepared, plain_zip) = shared.clone().prepare(bytes, permit).await;
        if !plain_zip {
            return one(prepared, len);
        }
    } else if !starts_like_zip(path) {
        return one(Prepared::Error(ImportError::TooLarge.code()), 0);
    }
    // A plain zip: its entries are files of their own, read from the disk one at a time; the
    // zip itself counts as done.
    shared.count_done();
    let (sh, name, path, handle) = (shared.clone(), file.to_string(), path.to_path_buf(), tokio::runtime::Handle::current());
    let walked = tokio::task::spawn_blocking(move || match std::fs::File::open(path) {
        Ok(f) => walk(BufReader::new(f), &sh, &handle, n, &name),
        Err(_) => (vec![], Err(ImportError::Broken.code())),
    })
    .await
    .unwrap_or((vec![], Err("broken")));
    let (entries, end) = walked;
    let mut parts = Vec::with_capacity(entries.len() + 1);
    let found = entries.len();
    for (key, name, weight, read) in entries {
        let prepared = match read {
            Ok(task) => task.await.unwrap_or(Prepared::Error("broken")),
            Err(code) => Prepared::Error(code),
        };
        parts.push(Part { key, file: name, prepared, weight });
    }
    let end = match end {
        Ok(()) if found == 0 => Err(ImportError::NoCard.code()),
        other => other,
    };
    if let Err(code) = end {
        // About the zip as a whole (after the entries it gave until then).
        parts.push(Part { key: (n, found + 1), file: file.to_string(), prepared: Prepared::Error(code), weight: 0 });
    }
    parts
}

type Walked = (Vec<(Key, String, u64, Result<JoinHandle<Prepared>, &'static str>)>, Result<(), &'static str>);

/// Reads the entries of a job's zip and starts parsing each (several at a time, within the
/// budget). Runs on the blocking pool.
fn walk<R: Read + Seek>(reader: R, shared: &Arc<Shared>, handle: &tokio::runtime::Handle, n: usize, file: &str) -> Walked {
    let mut tasks = vec![];
    let end = each_entry(reader, MAX_INPUT as u64, &shared.unzipped, MAX_UNZIPPED, |entry| {
        shared.total.fetch_add(1, Ordering::SeqCst);
        let key = (n, tasks.len() + 1);
        let name = format!("{file}/{}", entry.name);
        match entry.read {
            Ok((bytes, permit)) => {
                let weight = bytes.len() as u64;
                let task = handle.spawn({
                    let shared = shared.clone();
                    async move {
                        let (prepared, _) = shared.clone().prepare(bytes, permit).await;
                        shared.count_done();
                        prepared
                    }
                });
                tasks.push((key, name, weight, Ok(task)));
            }
            Err(code) => {
                shared.count_done();
                tasks.push((key, name, 0, Err(code)));
            }
        }
        true
    });
    (tasks, end)
}

fn starts_like_zip(path: &std::path::Path) -> bool {
    let mut head = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && is_zip(&head)
}

/// Counts of a result list for the response and the log: (imported, duplicates, errors).
pub fn counts(results: &[FileResult]) -> (usize, usize, usize) {
    let n = |s: &str| results.iter().filter(|r| r.status == s).count();
    (n("imported"), n("duplicate"), n("error"))
}

// ---------------------------------------------------------------------------------------------
// Personas

/// File name without folders (browsers send `User Avatars/x.png` for folder uploads).
fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// SillyTavern persona lists: the backup file (`personas`, `persona_descriptions`) or
/// `settings.json` (the same under `power_user`). None: neither.
fn st_personas(raw: &Value) -> Option<(&serde_json::Map<String, Value>, &Value)> {
    let root = if raw["personas"].is_object() { raw } else { &raw["power_user"] };
    Some((root["personas"].as_object()?, &root["persona_descriptions"]))
}

/// What a persona import got: (file, item or error code) in upload order.
pub type PersonaParts = Vec<(String, Result<NewPersona, &'static str>)>;

/// Reads persona files: SillyTavern backup or `settings.json` (pictures from the other files of
/// the same upload, matched by file name), RisuAI persona PNGs, and zips holding any of these
/// (the app's own export; all zips of one call unpack to at most [`MAX_PERSONA_UPLOAD`]).
/// Pictures that belong to no persona give no result. Runs on the blocking pool.
pub fn read_personas(files: Vec<(String, Vec<u8>)>) -> PersonaParts {
    let unzipped = AtomicU64::new(0);
    let mut flat: Vec<(String, Result<Vec<u8>, &'static str>)> = vec![];
    for (name, bytes) in files {
        if is_zip(&bytes) && formats::detect_and_parse(&bytes).is_err() {
            let end = each_entry(Cursor::new(&bytes), MAX_INPUT as u64, &unzipped, MAX_PERSONA_UPLOAD, |entry| {
                flat.push((format!("{name}/{}", entry.name), entry.read.map(|(b, _)| b)));
                true
            });
            if let Err(code) = end {
                flat.push((name, Err(code)));
            }
        } else {
            flat.push((name, Ok(bytes)));
        }
    }
    let mut pictures: HashMap<String, Vec<u8>> = HashMap::new();
    let mut lists = vec![];
    let mut out: PersonaParts = vec![];
    for (name, bytes) in flat {
        let bytes = match bytes {
            Ok(b) => b,
            Err(code) => {
                out.push((name, Err(code)));
                continue;
            }
        };
        if let Ok(raw) = serde_json::from_slice::<Value>(strip_bom(&bytes)) {
            if st_personas(&raw).is_some() {
                lists.push((name, raw));
            } else {
                out.push((name, Err("no_persona")));
            }
            continue;
        }
        match formats::detect_and_parse(&bytes) {
            Ok(formats::ImportedFile { content: Content::Persona { persona, image }, .. }) => {
                let item = persona_edit(&persona.name, &persona.prompt, &persona.raw[RP_PERSONA_KEY])
                    .map(|edit| NewPersona { edit, avatar: image.and_then(|i| persona_picture(i.bytes)) })
                    .ok_or(ImportError::Broken.code());
                out.push((name, item));
            }
            Ok(_) => out.push((name, Err("no_persona"))),
            Err(_) if ImageKind::sniff(&bytes).is_some() => {
                pictures.insert(base_name(&name).to_string(), bytes);
            }
            Err(e) => out.push((name, Err(e.code()))),
        }
    }
    for (file, raw) in lists {
        let (names, descriptions) = st_personas(&raw).expect("checked above");
        for (avatar, name) in names {
            let d = &descriptions[avatar.as_str()];
            let description = d["description"].as_str().unwrap_or("");
            let item = name
                .as_str()
                .and_then(|n| persona_edit(n, description, &d[RP_PERSONA_KEY]))
                .map(|edit| NewPersona { edit, avatar: pictures.get(base_name(avatar)).cloned().and_then(persona_picture) })
                .ok_or("bad_name");
            out.push((file.clone(), item));
        }
    }
    out
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

/// The persona fields the app adds to exchange files.
pub fn rp_persona_ext(p: &store::Persona) -> Value {
    json!({
        "pronouns": p.pronouns,
        "appearance": p.appearance,
        "personality": p.personality,
        "background": p.background,
        "every_turn": p.every_turn,
        "card_priority": p.card_priority.as_str(),
    })
}
