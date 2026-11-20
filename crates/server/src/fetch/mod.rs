//! Models from Hugging Face: the download manager (`downloads`), the curated catalog with its
//! live check (`catalog`), the fit pill and the picks for a PC profile (`pick`), and the pure
//! helpers for links, quants and tiers (`quant`). Routes: `routes/fetch.rs`.
//!
//! Network rules: one request per user action, the catalog's repos at most once per 24 h each,
//! user agent `Lorelamp/<version>`, no token, no account. Tests point the base URL at a local
//! stand-in (`Running::set_hf_base`, or `RP_HF_BASE` for a debug test server; release builds ignore it).

pub mod catalog;
pub mod downloads;
pub mod pick;
pub mod quant;
pub mod suggest;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rp_models::HfClient;

use crate::chat::Hub;

pub(crate) struct Fetch {
    hub: Arc<Hub>,
    data_dir: PathBuf,
    hf: RwLock<HfClient>,
    dl: Mutex<downloads::DlState>,
    cat: Mutex<catalog::CatState>,
}

impl Fetch {
    /// Loads the download list (a download that ran when the app stopped goes on) and the
    /// catalog cache. Needs a Tokio runtime.
    pub fn start(hub: Arc<Hub>, data_dir: &Path) -> Arc<Fetch> {
        let hf = test_hf_base().and_then(|b| HfClient::with_base(&b).ok()).unwrap_or_default();
        let fetch = Arc::new(Fetch {
            hub,
            data_dir: data_dir.to_path_buf(),
            hf: RwLock::new(hf),
            dl: Mutex::new(downloads::DlState::load(data_dir)),
            cat: Mutex::new(catalog::CatState::load(data_dir)),
        });
        fetch.pump();
        fetch
    }

    pub fn hf(&self) -> HfClient {
        self.hf.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Another Hugging Face base URL (tests). Downloads already in the list keep their URLs.
    pub fn set_hf_base(&self, base: &str) -> Result<(), String> {
        let c = HfClient::with_base(base).map_err(|e| e.to_string())?;
        *self.hf.write().unwrap_or_else(|p| p.into_inner()) = c;
        Ok(())
    }

    /// Where downloads go: the folder chosen in the tab "Modell", else `<data>/models`.
    pub fn models_dir(&self) -> PathBuf {
        match self.hub.store(|s| s.download_folder()) {
            Ok(Some(dir)) => PathBuf::from(dir),
            _ => self.data_dir.join("models"),
        }
    }
}

/// `RP_HF_BASE`: a stand-in for Hugging Face, read by debug builds only (the e2e test server). Model
/// files and their SHA-256 come from this address, so a release build always asks huggingface.co.
fn test_hf_base() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("RP_HF_BASE").ok().filter(|b| b.starts_with("http://") || b.starts_with("https://"))
}

pub(crate) fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Writes `bytes` next to `path` first, then renames, so a crash never leaves half a file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
