//! SillyTavern installations on this PC (`GET /import/sillytavern/find`) and what one holds:
//! cards, personas and chats (`GET /import/sillytavern/contents`, `POST /import/sillytavern`).
//! ST is only read, never written.
//!
//! The search looks at the user folder (and its Desktop, Documents, Downloads) and every local
//! fixed drive, at most [`MAX_DEPTH`] folders deep, for folders whose name contains
//! "sillytavern". One counts when it has `data/<user>/characters` (ST 1.12 and newer) or
//! `public/characters` (older ones). System, program and hidden folders are skipped, and the
//! search stops after [`BUDGET`]. Env `RP_ST_SEARCH_ROOTS` (paths split by `;`) replaces the
//! roots, so tests never search the real drives.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

/// Deepest folder level looked at below a root.
pub const MAX_DEPTH: usize = 4;
/// Time the search may take.
pub const BUDGET: Duration = Duration::from_secs(3);
/// Folder of the user pictures (personas) inside an ST data folder.
pub const AVATARS: &str = "User Avatars";

/// One user's data of an installation.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Install {
    /// The installation folder.
    pub path: String,
    /// The ST user (`data/<user>`); null for the old layout (`public/`).
    pub user: Option<String>,
    /// Card files (`.png`, `.json`) in `characters`.
    pub characters: usize,
    /// Personas in `settings.json`.
    pub personas: usize,
    /// Pictures in `User Avatars`.
    pub avatars: usize,
    /// The data folder (holds `characters`, `settings.json`, `User Avatars`).
    #[serde(skip)]
    pub data: PathBuf,
}

/// What a search found.
#[derive(Debug, Clone, Serialize)]
pub struct Found {
    pub installs: Vec<Install>,
    pub searched_ms: u128,
    /// false: the time ran out before every folder was looked at.
    pub complete: bool,
}

fn ext_is(path: &Path, exts: &[&str]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Card files directly in a `characters` folder.
pub fn card_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| ext_is(p, &["png", "json"]))
        .collect();
    files.sort();
    files
}

fn count_images(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && ext_is(&e.path(), &["png", "jpg", "jpeg", "webp", "gif"]))
        .count()
}

/// `settings.json` of a data folder, if it is there and JSON.
pub fn settings(data: &Path) -> Option<Value> {
    let bytes = std::fs::read(data.join("settings.json")).ok()?;
    serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).ok()
}

/// The persona keys (avatar file names) of `settings.json`.
pub fn persona_keys(settings: &Value) -> Vec<String> {
    settings["power_user"]["personas"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default()
}

fn install(path: &Path, user: Option<String>, data: PathBuf) -> Install {
    let personas = settings(&data).map_or(0, |s| persona_keys(&s).len());
    Install {
        path: path.display().to_string(),
        user,
        characters: card_files(&data.join("characters")).len(),
        personas,
        avatars: count_images(&data.join(AVATARS)),
        data,
    }
}

/// The user data of an ST installation folder (empty: it is none).
pub fn installs_at(path: &Path) -> Vec<Install> {
    let mut out = vec![];
    let data = path.join("data");
    let mut users: Vec<(String, PathBuf)> = std::fs::read_dir(&data)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(_, dir)| dir.join("characters").is_dir())
        .collect();
    users.sort();
    for (user, dir) in users {
        out.push(install(path, Some(user), dir));
    }
    let public = path.join("public");
    if public.join("characters").is_dir() {
        out.push(install(path, None, public));
    }
    out
}

/// Folder of the chats inside an ST data folder (`chats/<character>/<name>.jsonl`).
pub const CHATS: &str = "chats";

/// One card file of `characters`, read for a look (name, content hash, picture).
#[derive(Debug, Clone)]
pub struct CardFile {
    /// The file name (the stable id for the API).
    pub id: String,
    /// None when the file is no readable card (`error`).
    pub name: Option<String>,
    pub hash: Option<String>,
    pub has_image: bool,
    pub error: Option<&'static str>,
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Reads a card file: (name, content hash, picture bytes). Err: the import error code.
pub fn read_card(path: &Path) -> Result<(String, String, Option<Vec<u8>>), &'static str> {
    if std::fs::metadata(path).map_err(|_| "broken")?.len() > rp_core::formats::MAX_INPUT as u64 {
        return Err("too_large");
    }
    let bytes = std::fs::read(path).map_err(|_| "broken")?;
    let file = rp_core::formats::detect_and_parse(&bytes).map_err(|e| e.code())?;
    let rp_core::formats::Content::Cards(cards) = file.content else { return Err("no_card") };
    let c = cards.into_iter().next().ok_or("no_card")?;
    let hash = crate::store::content_hash(&c.raw).ok_or("broken")?;
    Ok((c.card.name, hash, c.image.map(|i| i.bytes)))
}

/// Every card file of a data folder, read.
pub fn cards(data: &Path) -> Vec<CardFile> {
    card_files(&data.join("characters"))
        .into_iter()
        .map(|path| {
            let id = file_name(&path);
            match read_card(&path) {
                Ok((name, hash, image)) => CardFile { id, name: Some(name), hash: Some(hash), has_image: image.is_some(), error: None },
                Err(code) => CardFile { id, name: None, hash: None, has_image: false, error: Some(code) },
            }
        })
        .collect()
}

/// One chat file: id `<folder>/<file>.jsonl` (the stable id for the API).
#[derive(Debug, Clone)]
pub struct ChatFile {
    pub id: String,
    /// The character folder (SillyTavern names it after the card file without extension).
    pub folder: String,
    pub path: PathBuf,
}

/// Every chat file of a data folder (`chats/*/*.jsonl`), sorted. Links are not followed.
pub fn chat_files(data: &Path) -> Vec<ChatFile> {
    let mut out = vec![];
    for dir in std::fs::read_dir(data.join(CHATS)).into_iter().flatten().flatten() {
        if !dir.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(folder) = dir.file_name().into_string() else { continue };
        for f in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let path = f.path();
            if f.file_type().is_ok_and(|t| t.is_file()) && ext_is(&path, &["jsonl"]) {
                if let Ok(name) = f.file_name().into_string() {
                    out.push(ChatFile { id: format!("{folder}/{name}"), folder: folder.clone(), path });
                }
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// A read chat file and the SHA-256 of its messages (the head is left out: SillyTavern rewrites
/// it whenever it saves metadata).
pub fn read_chat(path: &Path, tz: i32) -> Result<(rp_core::st_chat::StChat, String), &'static str> {
    use sha2::{Digest, Sha256};
    if std::fs::metadata(path).map_err(|_| "broken")?.len() > rp_core::formats::MAX_INPUT as u64 {
        return Err("too_large");
    }
    let bytes = std::fs::read(path).map_err(|_| "broken")?;
    let chat = rp_core::st_chat::parse(&bytes, tz)?;
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).map_err(|_| "broken")?;
    let mut h = Sha256::new();
    for line in rp_core::st_chat::message_lines(text) {
        h.update(line.as_bytes());
        h.update(b"\n");
    }
    Ok((chat, h.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}

/// The card a chat belongs to: the card file named like the chat folder, else a card whose
/// name is the folder name, else one named like the chat head's `character_name`. None also when
/// a name fits several different cards (no guess; the same card twice counts as one).
pub fn match_card(folder: &str, character_name: &str, cards: &[CardFile]) -> Option<usize> {
    let stem = |c: &CardFile| Path::new(&c.id).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // Ok(None): no card of that name; Err: several different ones.
    let named = |n: &str| -> Result<Option<usize>, ()> {
        let n = n.trim();
        let mut hits = cards.iter().enumerate().filter(|(_, c)| !n.is_empty() && c.name.as_deref().is_some_and(|x| x.trim().eq_ignore_ascii_case(n)));
        let Some((first, card)) = hits.next() else { return Ok(None) };
        if hits.any(|(_, c)| c.hash != card.hash) {
            return Err(());
        }
        Ok(Some(first))
    };
    if let Some(i) = cards.iter().position(|c| stem(c) == folder).or_else(|| cards.iter().position(|c| stem(c).eq_ignore_ascii_case(folder))) {
        return Some(i);
    }
    match named(folder) {
        Ok(Some(i)) => Some(i),
        Ok(None) => named(character_name).ok().flatten(),
        Err(()) => None,
    }
}

/// Folder names never searched (compared without case; `Program Files*` by prefix).
fn skipped(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with('.')
        || n.starts_with('$')
        || n.starts_with("program files")
        || matches!(n.as_str(), "windows" | "programdata" | "appdata" | "node_modules" | "system volume information" | "recovery")
}

#[cfg(windows)]
fn hidden_or_system(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn hidden_or_system(_: &std::fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
}

/// Local fixed drives (`C:\`, `D:\`, …); no network, removable or optical drives.
#[cfg(windows)]
fn fixed_drives() -> Vec<PathBuf> {
    const DRIVE_FIXED: u32 = 3;
    // SAFETY: GetLogicalDrives takes no arguments and only returns a bit mask.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let root = format!("{}:\\", char::from(b'A' + i));
            let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: `wide` is a NUL-terminated UTF-16 string that lives for the whole call.
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            (kind == DRIVE_FIXED).then(|| PathBuf::from(root))
        })
        .collect()
}

#[cfg(not(windows))]
fn fixed_drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

/// Where the search starts: `RP_ST_SEARCH_ROOTS` if set, else the user folders, then the drives.
pub fn roots() -> Vec<PathBuf> {
    if let Ok(list) = std::env::var("RP_ST_SEARCH_ROOTS") {
        return list.split(';').map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    }
    let mut out = vec![];
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from) {
        for sub in ["Desktop", "Documents", "Downloads"] {
            out.push(home.join(sub));
        }
        out.push(home);
    }
    out.extend(fixed_drives());
    out
}

fn is_candidate(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_ascii_lowercase().contains("sillytavern"))
}

/// Searches the roots breadth first (each folder once), within the depth and the time budget.
pub fn search(roots: &[PathBuf], budget: Duration) -> Found {
    let started = Instant::now();
    let mut seen: HashSet<String> = HashSet::new();
    let mut installs: Vec<Install> = vec![];
    let mut complete = true;
    let key = |p: &Path| p.display().to_string().trim_end_matches(['\\', '/']).to_lowercase();
    'roots: for root in roots {
        let mut level = vec![root.clone()];
        for depth in 0..=MAX_DEPTH {
            let mut next = vec![];
            for dir in level {
                if started.elapsed() > budget {
                    complete = false;
                    break 'roots;
                }
                if !seen.insert(key(&dir)) {
                    continue;
                }
                if is_candidate(&dir) {
                    let found = installs_at(&dir);
                    if !found.is_empty() {
                        installs.extend(found);
                        continue;
                    }
                }
                if depth == MAX_DEPTH {
                    continue;
                }
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    // Symlinks and junctions are not followed (`is_dir` is false for them).
                    let Ok(kind) = entry.file_type() else { continue };
                    if !kind.is_dir() || entry.file_name().to_str().is_none_or(skipped) {
                        continue;
                    }
                    if entry.metadata().is_ok_and(|m| hidden_or_system(&m)) {
                        continue;
                    }
                    next.push(entry.path());
                }
            }
            if next.is_empty() {
                break;
            }
            level = next;
        }
    }
    installs.sort_by(|a, b| (a.path.to_lowercase(), &a.user).cmp(&(b.path.to_lowercase(), &b.user)));
    Found { installs, searched_ms: started.elapsed().as_millis(), complete }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fake ST installation: new layout with `default-user` (cards, settings with personas,
    /// one avatar) when `new`, else the old `public/` layout with cards only.
    pub(crate) fn fake_install(dir: &Path, new: bool) {
        let data = if new { dir.join("data").join("default-user") } else { dir.join("public") };
        let chars = data.join("characters");
        std::fs::create_dir_all(&chars).unwrap();
        std::fs::write(chars.join("Ilse.png"), include_bytes!("../../../tests/fixtures/ilse_brandt.png")).unwrap();
        std::fs::write(chars.join("Mara.json"), include_bytes!("../../../tests/fixtures/mara_voss.json")).unwrap();
        std::fs::write(chars.join("notes.txt"), b"not a card").unwrap();
        std::fs::create_dir_all(chars.join("Ilse")).unwrap(); // sprites folder: not a card
        if new {
            let backup: Value = serde_json::from_slice(include_bytes!("../../../tests/fixtures/formats/personas_20261115.json")).unwrap();
            let settings = serde_json::json!({ "username": "x", "power_user": backup });
            std::fs::write(data.join("settings.json"), serde_json::to_vec(&settings).unwrap()).unwrap();
            std::fs::create_dir_all(data.join(AVATARS)).unwrap();
            std::fs::write(data.join(AVATARS).join("user-default.png"), plain_png()).unwrap();
        }
    }

    /// A small picture without card data (what `User Avatars` holds).
    pub(crate) fn plain_png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(8, 8, image::Rgb([40, 90, 140])).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    pub(crate) fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rp-app-st-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_installs_within_depth() {
        let root = temp("search");
        fake_install(&root.join("Games").join("SillyTavern"), true);
        fake_install(&root.join("old").join("sillytavern-1.10"), false);
        // Too deep (depth 5), in a skipped folder, and a name without "sillytavern".
        fake_install(&root.join("a").join("b").join("c").join("d").join("SillyTavern"), true);
        fake_install(&root.join("node_modules").join("SillyTavern"), true);
        fake_install(&root.join("Tavern"), true);
        // A candidate that is no installation.
        std::fs::create_dir_all(root.join("SillyTavern-Launcher").join("docs")).unwrap();

        let found = search(&[root.clone(), root.join("Games")], BUDGET);
        assert!(found.complete);
        let got: Vec<(String, Option<String>, usize, usize, usize)> =
            found.installs.iter().map(|i| (i.path.replace(&root.display().to_string(), ""), i.user.clone(), i.characters, i.personas, i.avatars)).collect();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            got,
            vec![
                (format!("{sep}Games{sep}SillyTavern"), Some("default-user".into()), 2, 2, 1),
                (format!("{sep}old{sep}sillytavern-1.10"), None, 2, 0, 0),
            ]
        );
        assert!(installs_at(&root.join("Tavern")).len() == 1, "valid, but not found by name");
        assert!(installs_at(&root.join("SillyTavern-Launcher")).is_empty());

        let none = search(std::slice::from_ref(&root), Duration::ZERO);
        assert!(!none.complete);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Several cards with the chat's name and none named like its folder: no card is guessed
    /// (unless they are the same card twice).
    #[test]
    fn same_named_cards_are_not_guessed() {
        let card = |id: &str, name: &str, hash: &str| CardFile { id: id.into(), name: Some(name.into()), hash: Some(hash.into()), has_image: false, error: None };
        let cards = [card("Theo.json", "Theo Lindqvist", "a"), card("Theo-alt.png", "Theo Lindqvist", "b"), card("Greta.json", "Greta Holm", "c")];
        assert_eq!(match_card("Theo", "Theo Lindqvist", &cards), Some(0), "folder named like the card file");
        assert_eq!(match_card("Theo-alt", "Theo Lindqvist", &cards), Some(1));
        assert_eq!(match_card("Theo Lindqvist", "", &cards), None, "folder named like two cards");
        assert_eq!(match_card("Unbekannt", "Theo Lindqvist", &cards), None, "head names two cards");
        assert_eq!(match_card("Unbekannt", "Greta Holm", &cards), Some(2));
        let twice = [card("Theo.json", "Theo Lindqvist", "a"), card("Kopie.json", "Theo Lindqvist", "a")];
        assert_eq!(match_card("Unbekannt", "Theo Lindqvist", &twice), Some(0), "the same card twice");
    }

    #[test]
    fn skip_rules() {
        for name in ["Windows", "Program Files", "Program Files (x86)", "ProgramData", "$Recycle.Bin", "AppData", "node_modules", ".git", ".cache"] {
            assert!(skipped(name), "{name}");
        }
        for name in ["SillyTavern", "Games", "Documents"] {
            assert!(!skipped(name), "{name}");
        }
    }
}
