//! The model engine (llama.cpp `llama-server`): what to start (`LaunchSpec`, from the active
//! model profile or, as emergency fallback, `engine.json`), the supervisor that keeps it running
//! and switches it when the profile changes (`supervisor`), what a build understands (`probe`),
//! and what the loaded model is (`model`).
//!
//! `engine.json` in the data folder is the settings file from before model profiles. The first
//! start with profiles copies it into the database (engine "Eigener Build" + active profile) and
//! leaves the file as it is. Form:
//! {
//!   "llama_server": "D:\\llm\\llama-server.exe",
//!   "extra_path": ["C:\\path\\to\\rocm\\bin"],
//!   "model": "C:\\models\\model.gguf",
//!   "port": 8080,
//!   "args": ["-c", "131072", "-ngl", "999", "-fa", "on"]
//! }
//! (An old `"stop_on_exit"` is ignored: a model server the app started always ends with it.)

pub mod idle;
pub mod install;
pub mod job;
pub mod local;
pub mod model;
pub mod probe;
mod supervisor;

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub use supervisor::{port_in_use, DesiredFn, EngineHandle, EventFn, Problem};

/// The port llama-server gets when nothing else is set. Always passed explicitly: newer
/// llama.cpp builds default to 9931.
pub const DEFAULT_ENGINE_PORT: u16 = 8080;

/// The settings file from before model profiles (see the module docs).
#[derive(Debug, Clone, Deserialize)]
pub struct EngineConfig {
    pub llama_server: PathBuf,
    #[serde(default)]
    pub extra_path: Vec<PathBuf>,
    pub model: PathBuf,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub args: Vec<String>,
}

fn default_port() -> u16 {
    DEFAULT_ENGINE_PORT
}

pub const ENGINE_JSON: &str = "engine.json";

/// `engine.json` in the data folder. Ok(None): there is none.
pub fn load_engine_json(data_dir: &Path) -> Result<Option<EngineConfig>, String> {
    let path = data_dir.join(ENGINE_JSON);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display()))
}

/// Everything needed to start one llama-server.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub exe: PathBuf,
    pub extra_path: Vec<PathBuf>,
    pub model: PathBuf,
    pub port: u16,
    /// Load arguments of the profile (without `-m`, `--host`, `--port`; the supervisor adds those).
    pub args: Vec<String>,
    /// The profile it comes from; None: `engine.json`.
    pub profile_id: Option<i64>,
}

impl LaunchSpec {
    pub fn from_engine_json(cfg: &EngineConfig) -> LaunchSpec {
        LaunchSpec {
            exe: cfg.llama_server.clone(),
            extra_path: cfg.extra_path.clone(),
            model: cfg.model.clone(),
            port: cfg.port,
            args: cfg.args.clone(),
            profile_id: None,
        }
    }

    /// Would start the same server (the profile it came from does not matter: two profiles that
    /// differ only in their sampler share one running server).
    pub fn same_launch(&self, other: &LaunchSpec) -> bool {
        self.exe == other.exe
            && self.extra_path == other.extra_path
            && self.model == other.model
            && self.port == other.port
            && self.args == other.args
    }

    pub fn model_file_name(&self) -> Option<String> {
        self.model.file_name().map(|f| f.to_string_lossy().to_string())
    }
}

/// What the supervisor should run right now (asked on every check).
#[derive(Debug, Clone, PartialEq)]
pub struct Desired {
    /// Ok(None): nothing configured. Err: the settings cannot be read.
    pub spec: Result<Option<LaunchSpec>, String>,
    /// The model is writing a reply: a switch (or an unload) waits until it is done.
    pub busy: bool,
    /// Unloaded to free the graphics memory: run no model server (see `idle`).
    pub unloaded: bool,
}

/// The compute backend of a build, from the ggml libraries next to the program
/// (`ggml-hip.dll` → rocm, …). "unknown" when none is there (a static build).
pub fn detect_backend(exe: &Path) -> &'static str {
    let Some(dir) = exe.parent() else { return "unknown" };
    let Ok(entries) = std::fs::read_dir(dir) else { return "unknown" };
    let names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).collect();
    let has = |part: &str| names.iter().any(|n| n.contains(&format!("ggml-{part}")));
    for (part, backend) in [("hip", "rocm"), ("cuda", "cuda"), ("vulkan", "vulkan"), ("sycl", "sycl"), ("metal", "metal"), ("cpu", "cpu")] {
        if has(part) {
            return backend;
        }
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_json_defaults_apply() {
        let cfg: EngineConfig = serde_json::from_str(r#"{ "llama_server": "a.exe", "model": "m.gguf" }"#).unwrap();
        assert_eq!(cfg.port, 8080);
        assert!(cfg.args.is_empty() && cfg.extra_path.is_empty());
    }

    #[test]
    fn same_launch_ignores_the_profile() {
        let a = LaunchSpec {
            exe: "a.exe".into(),
            extra_path: vec![],
            model: "m.gguf".into(),
            port: 8080,
            args: vec!["-c".into(), "2048".into()],
            profile_id: Some(1),
        };
        let b = LaunchSpec { profile_id: Some(2), ..a.clone() };
        assert!(a.same_launch(&b));
        let c = LaunchSpec { args: vec!["-c".into(), "4096".into()], ..a.clone() };
        assert!(!a.same_launch(&c));
    }

    #[test]
    fn backend_from_the_libraries_next_to_the_program() {
        let dir = std::env::temp_dir().join(format!("rp-app-backend-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("llama-server.exe");
        assert_eq!(detect_backend(&exe), "unknown");
        std::fs::write(dir.join("ggml-cpu-haswell.dll"), b"").unwrap();
        assert_eq!(detect_backend(&exe), "cpu");
        std::fs::write(dir.join("ggml-hip.dll"), b"").unwrap();
        assert_eq!(detect_backend(&exe), "rocm", "a GPU backend wins over the CPU libraries next to it");
        let _ = std::fs::remove_dir_all(dir);
    }
}
