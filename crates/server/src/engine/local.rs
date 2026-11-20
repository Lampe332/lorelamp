//! "Meine Modelle": the GGUF files in the model folders. Three kinds of folders are searched
//! (recursively, `.gguf` only): the app's own model folder (default `<data>\models`, the only one
//! the app deletes in), LM Studio's model folder (read only) and further folders the user added.
//! A split model (`-00001-of-0000N`) is one entry; a multimodal projector (`mmproj-*`) is not a
//! model of its own but noted on the models next to it. The GGUF metadata is read once per file
//! version (path + modification time, `model::gguf_cached`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rp_core::gguf::split_parts;
use serde::Serialize;

use super::model::{gguf_cached, summarize};

/// Folders deeper than this below a model folder are not searched.
const MAX_DEPTH: usize = 8;
/// Stop after this many directory entries (someone added `C:\`).
const MAX_ENTRIES: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// The app's model folder.
    App,
    /// LM Studio's model folder.
    Lmstudio,
    /// A folder the user added.
    Extra,
}

/// The folders to search, in this order (a file found twice counts once, for the first one).
#[derive(Debug, Clone, PartialEq)]
pub struct Folders {
    pub app: PathBuf,
    /// `app` is the default `<data>\models` (not chosen by the user).
    pub app_default: bool,
    pub lmstudio: Option<PathBuf>,
    pub extra: Vec<PathBuf>,
}

impl Folders {
    pub fn roots(&self) -> Vec<(Origin, PathBuf)> {
        let mut out = vec![(Origin::App, self.app.clone())];
        out.extend(self.lmstudio.iter().map(|p| (Origin::Lmstudio, p.clone())));
        out.extend(self.extra.iter().map(|p| (Origin::Extra, p.clone())));
        out
    }
}

/// LM Studio's model folder: `RP_LMSTUDIO_DIR` when set (empty: none, so tests never see the real
/// one), else `%USERPROFILE%\.lmstudio\models`.
pub fn lmstudio_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("RP_LMSTUDIO_DIR") {
        return (!dir.trim().is_empty()).then(|| PathBuf::from(dir));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".lmstudio").join("models"))
}

/// The app's model folder when the user chose none.
pub fn default_app_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LocalModel {
    /// The file to start: the model itself, for a split model its first part.
    pub path: String,
    pub file: String,
    /// The folder it lies in.
    pub dir: String,
    pub origin: Origin,
    /// The model folder it was found in.
    pub root: String,
    /// All parts together.
    pub size_bytes: u64,
    /// Number of parts (1: one file).
    pub parts: u32,
    /// Parts that are missing next to the others (the model cannot start then).
    pub missing_parts: Vec<u32>,
    /// Multimodal projectors in the same folder (file names).
    pub mmproj: Vec<String>,
    pub name: Option<String>,
    pub arch: Option<String>,
    pub quant: Option<String>,
    pub params_total: Option<u64>,
    pub params_active: Option<u64>,
    pub moe: Option<bool>,
    pub ctx_train: Option<u64>,
    /// The GGUF header could be read.
    pub readable: bool,
}

/// Every `.gguf` below `dir` (not following links), at most `MAX_DEPTH` deep.
fn walk(dir: &Path, depth: usize, budget: &mut usize, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(ty) = e.file_type() else { continue };
        let name = e.file_name();
        let name = name.to_string_lossy();
        if ty.is_dir() {
            if depth < MAX_DEPTH && !name.starts_with('.') {
                walk(&e.path(), depth + 1, budget, out);
            }
        } else if ty.is_file() && name.to_ascii_lowercase().ends_with(".gguf") {
            out.push((e.path(), e.metadata().map(|m| m.len()).unwrap_or(0)));
        }
    }
}

/// The same file on Windows whatever the case and slashes.
pub fn path_key(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

fn is_projector_name(file: &str) -> bool {
    file.to_ascii_lowercase().starts_with("mmproj")
}

/// All models in `roots`, sorted by name.
pub fn scan(roots: &[(Origin, PathBuf)]) -> Vec<LocalModel> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut budget = MAX_ENTRIES;
    for (origin, root) in roots {
        let mut files = Vec::new();
        walk(root, 0, &mut budget, &mut files);
        files.retain(|(p, _)| seen.insert(path_key(p)));
        // Per folder: projectors, and the models (split parts grouped by stem and count).
        let mut by_dir: BTreeMap<PathBuf, Vec<(PathBuf, u64)>> = BTreeMap::new();
        for f in files {
            by_dir.entry(f.0.parent().map(Path::to_path_buf).unwrap_or_default()).or_default().push(f);
        }
        for (dir, files) in by_dir {
            let mut projectors = Vec::new();
            // key → (part number, path, size) of every part found
            let mut groups: BTreeMap<String, Vec<(u32, PathBuf, u64)>> = BTreeMap::new();
            for (path, size) in files {
                let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                if is_projector_name(&file) {
                    projectors.push(file);
                    continue;
                }
                match split_parts(&file) {
                    Some((stem, part, count)) => groups.entry(format!("{stem}\0{count}")).or_default().push((part, path, size)),
                    None => groups.entry(file).or_default().push((1, path, size)),
                }
            }
            projectors.sort();
            let mut models = Vec::new();
            for (_, mut parts) in groups {
                parts.sort_by_key(|p| p.0);
                let (_, first, _) = &parts[0];
                let info = gguf_cached(first);
                // A projector with an ordinary name (architecture `clip`) is no model either.
                if info.as_ref().is_some_and(|i| i.is_projector) {
                    projectors.push(first.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
                    continue;
                }
                let file = first.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                let count = split_parts(&file).map_or(1, |(_, _, n)| n);
                let missing = (1..=count).filter(|n| !parts.iter().any(|p| p.0 == *n)).collect();
                let s = summarize(first, info.as_deref());
                models.push(LocalModel {
                    path: first.display().to_string(),
                    file,
                    dir: dir.display().to_string(),
                    origin: *origin,
                    root: root.display().to_string(),
                    size_bytes: parts.iter().map(|p| p.2).sum(),
                    parts: count,
                    missing_parts: missing,
                    mmproj: Vec::new(),
                    name: s.name,
                    arch: s.arch,
                    quant: s.quant,
                    params_total: s.params_total,
                    params_active: s.params_active,
                    moe: s.moe,
                    ctx_train: s.ctx_train,
                    readable: info.is_some(),
                });
            }
            for m in &mut models {
                m.mmproj = projectors.clone();
            }
            out.extend(models);
        }
    }
    out.sort_by(|a, b| {
        let key = |m: &LocalModel| m.name.clone().unwrap_or_else(|| m.file.clone()).to_lowercase();
        key(a).cmp(&key(b)).then_with(|| a.path.cmp(&b.path))
    });
    out
}

/// `path` lies inside `root` (both resolved, so `..` and other spellings do not get out).
pub fn inside(root: &Path, path: &Path) -> bool {
    match (std::fs::canonicalize(root), std::fs::canonicalize(path)) {
        (Ok(r), Ok(p)) => p != r && p.starts_with(&r),
        _ => false,
    }
}

/// The files that make up the model at `path` (every part of a split model that exists).
pub fn model_files(path: &Path) -> Vec<PathBuf> {
    let file = path.file_name().and_then(|f| f.to_str()).unwrap_or_default();
    let Some((stem, _, count)) = split_parts(file) else { return vec![path.to_path_buf()] };
    let dir = path.parent().unwrap_or(Path::new(""));
    (1..=count).map(|n| dir.join(format!("{stem}-{n:05}-of-{count:05}.gguf"))).filter(|p| p.is_file()).collect()
}

/// Deletes the model at `path` (all its parts). The caller checked that it may. Bytes freed.
pub fn delete_model(path: &Path) -> std::io::Result<u64> {
    let mut freed = 0;
    for f in model_files(path) {
        let len = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
        std::fs::remove_file(&f)?;
        freed += len;
    }
    Ok(freed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A tiny GGUF v3 file: a few metadata keys and one tensor of `data` bytes.
    pub fn gguf_bytes(name: &str, arch: &str, ctx: u32, data: usize) -> Vec<u8> {
        fn s(out: &mut Vec<u8>, v: &str) {
            out.extend((v.len() as u64).to_le_bytes());
            out.extend(v.as_bytes());
        }
        let mut kv = Vec::new();
        let mut n = 0u64;
        for (k, v) in [("general.architecture", arch), ("general.name", name)] {
            s(&mut kv, k);
            kv.extend(8u32.to_le_bytes());
            s(&mut kv, v);
            n += 1;
        }
        for (k, v) in [("general.file_type".to_string(), 15u32), (format!("{arch}.context_length"), ctx), (format!("{arch}.block_count"), 2)] {
            s(&mut kv, &k);
            kv.extend(4u32.to_le_bytes());
            kv.extend(v.to_le_bytes());
            n += 1;
        }
        let mut out = b"GGUF".to_vec();
        out.extend(3u32.to_le_bytes());
        out.extend(1u64.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend(kv);
        s(&mut out, "token_embd.weight");
        out.extend(1u32.to_le_bytes());
        out.extend((data as u64).to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(0u64.to_le_bytes());
        while !out.len().is_multiple_of(32) {
            out.push(0);
        }
        out.resize(out.len() + data, 7);
        out
    }

    pub struct Tmp(pub PathBuf);
    impl Tmp {
        pub fn new() -> Tmp {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!("rp-local-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        pub fn put(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_models_groups_parts_and_projectors() {
        let app = Tmp::new();
        let extra = Tmp::new();
        app.put("Mini-Q4_K_M.gguf", &gguf_bytes("Mini", "llama", 8192, 1000));
        app.put("vendor/repo/mmproj-Mini-F16.gguf", &gguf_bytes("Projector", "clip", 0, 10));
        app.put("vendor/repo/Vision-Q4_K_M.gguf", &gguf_bytes("Vision", "gemma3", 32768, 64));
        app.put("vendor/repo/notes.txt", b"not a model");
        app.put("big/Big-00001-of-00003.gguf", &gguf_bytes("Big", "qwen3moe", 40960, 100));
        app.put("big/Big-00002-of-00003.gguf", &gguf_bytes("", "qwen3moe", 0, 200));
        app.put("big/Big-00003-of-00003.gguf", &gguf_bytes("", "qwen3moe", 0, 300));
        app.put("half/Half-00001-of-00002.gguf", &gguf_bytes("Half", "llama", 4096, 10));
        app.put(".hidden/Secret.gguf", &gguf_bytes("Secret", "llama", 4096, 10));
        let broken = extra.put("Broken.GGUF", b"not a gguf at all");
        let roots = vec![(Origin::App, app.0.clone()), (Origin::Extra, extra.0.clone()), (Origin::Extra, app.0.join("big"))];
        let models = scan(&roots);
        let names: Vec<&str> = models.iter().map(|m| m.name.as_deref().unwrap_or("?")).collect();
        assert_eq!(names, vec!["Big", "Broken", "Half", "Mini", "Vision"], "projector and hidden folder left out, sorted");

        let big = &models[0];
        assert_eq!((big.parts, big.missing_parts.len(), big.origin), (3, 0, Origin::App), "found once, for the first folder");
        assert!(big.path.ends_with("Big-00001-of-00003.gguf"));
        assert_eq!(big.size_bytes, ["Big-00001-of-00003.gguf", "Big-00002-of-00003.gguf", "Big-00003-of-00003.gguf"]
            .iter()
            .map(|f| std::fs::metadata(app.0.join("big").join(f)).unwrap().len())
            .sum::<u64>());
        assert_eq!((big.arch.as_deref(), big.quant.as_deref(), big.ctx_train), (Some("qwen3moe"), Some("Q4_K_M"), Some(40960)));
        assert!(big.params_total.is_some_and(|p| p >= 600), "tensors of all parts counted");

        let broken_m = &models[1];
        assert_eq!((broken_m.readable, broken_m.origin, broken_m.path.as_str()), (false, Origin::Extra, broken.display().to_string().as_str()));
        assert_eq!(models[2].missing_parts, vec![2]);
        assert!(models[3].mmproj.is_empty());
        assert_eq!(models[4].mmproj, vec!["mmproj-Mini-F16.gguf".to_string()]);
    }

    #[test]
    fn metadata_is_cached_by_path_and_time() {
        let dir = Tmp::new();
        let p = dir.put("Cached.gguf", &gguf_bytes("First", "llama", 4096, 64));
        let when = std::fs::metadata(&p).unwrap().modified().unwrap();
        let roots = [(Origin::App, dir.0.clone())];
        assert_eq!(scan(&roots)[0].name.as_deref(), Some("First"));
        // Same file, other content, same time: the cached header is used.
        std::fs::write(&p, gguf_bytes("Other", "llama", 4096, 64)).unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(when).unwrap();
        assert_eq!(scan(&roots)[0].name.as_deref(), Some("First"));
        // A new time: read again.
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(when + std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(scan(&roots)[0].name.as_deref(), Some("Other"));
    }

    #[test]
    fn inside_and_delete_all_parts() {
        let app = Tmp::new();
        let other = Tmp::new();
        let a = app.put("x/A-00001-of-00002.gguf", &gguf_bytes("A", "llama", 4096, 32));
        app.put("x/A-00002-of-00002.gguf", &gguf_bytes("", "llama", 0, 32));
        let outside = other.put("B.gguf", &gguf_bytes("B", "llama", 4096, 32));
        assert!(inside(&app.0, &a));
        assert!(!inside(&app.0, &outside));
        assert!(!inside(&app.0, &app.0.join("x").join("..").join("..").join(other.0.file_name().unwrap()).join("B.gguf")), "no way out with ..");
        assert!(!inside(&app.0, &app.0), "the folder itself is not inside");
        assert!(!inside(&app.0, &app.0.join("missing.gguf")));
        assert_eq!(model_files(&a).len(), 2);
        let freed = delete_model(&a).unwrap();
        assert!(freed > 64);
        assert!(scan(&[(Origin::App, app.0.clone())]).is_empty());
        assert!(outside.is_file());
    }
}
