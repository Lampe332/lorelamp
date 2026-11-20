//! What the loaded model is: name, architecture, quant, parameters, MoE/SWA and the context,
//! from the GGUF file (read once per file: path + modification time) and llama-server's
//! `/props`. Every field is optional: what cannot be read is "unknown" (null), never an error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use rp_core::gguf::{self, GgufInfo};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ModelInfo {
    /// File name of the model (no folder).
    pub file: Option<String>,
    pub name: Option<String>,
    pub arch: Option<String>,
    pub quant: Option<String>,
    pub params_total: Option<u64>,
    /// Parameters per token (MoE: shared part plus the experts in use).
    pub params_active: Option<u64>,
    pub moe: Option<bool>,
    pub swa: Option<bool>,
    /// Context the model was trained for.
    pub ctx_train: Option<u64>,
    /// Context the server actually uses (per slot).
    pub ctx: Option<u64>,
    /// llama.cpp build of the server.
    pub build: Option<String>,
}

type Cache = Mutex<HashMap<PathBuf, (Option<SystemTime>, Option<Arc<GgufInfo>>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The GGUF metadata of `path`, read once per file version. None: not readable (also cached,
/// so a broken file is not read again on every request).
pub fn gguf_cached(path: &Path) -> Option<Arc<GgufInfo>> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some((seen, info)) = cache().lock().unwrap_or_else(|p| p.into_inner()).get(path) {
        if *seen == mtime {
            return info.clone();
        }
    }
    let info = gguf::read_info(path).ok().map(Arc::new);
    cache().lock().unwrap_or_else(|p| p.into_inner()).insert(path.to_path_buf(), (mtime, info.clone()));
    info
}

/// What `/props` says: (model file, context per slot, build).
pub fn from_props(props: &Value) -> (Option<PathBuf>, Option<u64>, Option<String>) {
    let model = props["model_path"].as_str().filter(|s| !s.is_empty()).map(PathBuf::from);
    let ctx = props["default_generation_settings"]["n_ctx"].as_u64().or_else(|| props["n_ctx"].as_u64()).filter(|&n| n > 0);
    let build = props["build_info"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    (model, ctx, build)
}

/// The fields that come from the file itself.
pub fn summarize(path: &Path, info: Option<&GgufInfo>) -> ModelInfo {
    let file = path.file_name().map(|f| f.to_string_lossy().to_string());
    let stem = path.file_stem().map(|f| f.to_string_lossy().to_string());
    let Some(i) = info else { return ModelInfo { file, name: stem, ..ModelInfo::default() } };
    ModelInfo {
        file,
        name: i.name.clone().filter(|n| !n.trim().is_empty()).or(stem),
        arch: i.architecture.clone(),
        quant: i.file_type.map(|_| i.file_type_name.clone()).filter(|q| q != "unbekannt"),
        params_total: (i.parameter_count > 0).then_some(i.parameter_count),
        params_active: (i.parameter_count > 0).then(|| i.active_parameters()),
        moe: Some(i.is_moe()),
        swa: Some(i.has_swa()),
        ctx_train: i.context_length,
        ctx: None,
        build: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn props_of_old_and_new_builds() {
        let new = json!({ "model_path": "C:\\m\\a.gguf", "build_info": "b12387-1a2b3c4", "default_generation_settings": { "n_ctx": 4096 } });
        assert_eq!(from_props(&new), (Some(PathBuf::from("C:\\m\\a.gguf")), Some(4096), Some("b12387-1a2b3c4".into())));
        assert_eq!(from_props(&json!({ "n_ctx": 2048 })), (None, Some(2048), None));
        assert_eq!(from_props(&json!(null)), (None, None, None));
    }

    #[test]
    fn unreadable_file_is_unknown_not_an_error() {
        let path = std::env::temp_dir().join(format!("rp-app-broken-{}.gguf", std::process::id()));
        std::fs::write(&path, b"not a gguf").unwrap();
        assert!(gguf_cached(&path).is_none());
        let m = summarize(&path, gguf_cached(&path).as_deref());
        assert_eq!((m.file.as_deref(), m.name.as_deref(), m.arch, m.params_total), (path.file_name().and_then(|f| f.to_str()), path.file_stem().and_then(|f| f.to_str()), None, None));
        let _ = std::fs::remove_file(path);
        assert!(gguf_cached(Path::new("Z:\\missing.gguf")).is_none());
    }

    /// Real files: `RP_TEST_GGUF` = one or more paths, separated by `;`.
    #[test]
    #[ignore]
    fn real_models() {
        let list = std::env::var("RP_TEST_GGUF").expect("RP_TEST_GGUF not set");
        for path in list.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let t = std::time::Instant::now();
            let info = gguf_cached(Path::new(path));
            let first = t.elapsed();
            let t = std::time::Instant::now();
            assert!(gguf_cached(Path::new(path)).is_some(), "second read from the cache");
            let cached = t.elapsed();
            let m = summarize(Path::new(path), info.as_deref());
            println!(
                "{}: name={:?} arch={:?} quant={:?} params_total={:?} params_active={:?} moe={:?} swa={:?} ctx_train={:?} (read {} ms, cached {} us)",
                m.file.as_deref().unwrap_or("?"), m.name, m.arch, m.quant, m.params_total, m.params_active, m.moe, m.swa, m.ctx_train,
                first.as_millis(), cached.as_micros()
            );
            assert!(m.arch.is_some() && m.quant.is_some() && m.params_total.is_some_and(|p| p > 1_000_000));
            assert!(m.params_active <= m.params_total);
        }
    }
}
