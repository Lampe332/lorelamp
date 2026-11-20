//! What a llama-server build understands: its `--help` (the flags) and `--version`, read once
//! per program file (path + modification time) and kept for the rest of the run.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use rp_core::llama_args;

/// `--help` of a ROCm build loads the GPU libraries first; that can take a few seconds.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Fewer flags than this and the output was not a help text (a crash message, a wrapper).
const MIN_FLAGS: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub flags: HashSet<String>,
    /// `b12387`; None when `--version` says nothing readable.
    pub version: Option<String>,
}

type Cache = Mutex<HashMap<(PathBuf, Option<SystemTime>), Arc<Result<Probe, String>>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The probe of this build (run once per program file). Err: the help could not be read.
pub fn probe(exe: &Path, extra_path: &[PathBuf]) -> Arc<Result<Probe, String>> {
    let key = (exe.to_path_buf(), std::fs::metadata(exe).and_then(|m| m.modified()).ok());
    if let Some(hit) = cache().lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return hit.clone();
    }
    let result = Arc::new(run_probe(exe, extra_path));
    cache().lock().unwrap_or_else(|p| p.into_inner()).insert(key, result.clone());
    result
}

fn run_probe(exe: &Path, extra_path: &[PathBuf]) -> Result<Probe, String> {
    let help = run(exe, extra_path, "--help")?;
    let flags = llama_args::parse_help(&help);
    if flags.len() < MIN_FLAGS {
        return Err(format!("--help lists only {} flags", flags.len()));
    }
    let version = run(exe, extra_path, "--version").ok().and_then(|t| llama_args::parse_version(&t));
    Ok(Probe { flags, version })
}

/// The PATH a build runs with: its extra folders (GPU runtime) first.
pub fn search_path(extra_path: &[PathBuf]) -> Result<std::ffi::OsString, String> {
    let mut path = extra_path.to_vec();
    if let Some(old) = std::env::var_os("PATH") {
        path.extend(std::env::split_paths(&old));
    }
    std::env::join_paths(path).map_err(|e| e.to_string())
}

/// Runs `exe arg` hidden and answers stdout + stderr; killed after `PROBE_TIMEOUT`.
pub(crate) fn run(exe: &Path, extra_path: &[PathBuf], arg: &str) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.arg(arg).env("PATH", search_path(extra_path)?).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{arg}: {e}"))?;
    let _ = super::job::adopt(&child); // ends with the app; it is killed after the timeout anyway
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{arg}: no answer within {} s", PROBE_TIMEOUT.as_secs()));
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut text = String::from_utf8_lossy(&out.join().unwrap_or_default()).to_string();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&err.join().unwrap_or_default()));
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_is_an_error_and_is_cached() {
        let exe = Path::new("Z:\\does-not-exist\\llama-server.exe");
        let first = probe(exe, &[]);
        assert!(first.is_err());
        assert!(Arc::ptr_eq(&first, &probe(exe, &[])), "probed once per program file");
    }
}
