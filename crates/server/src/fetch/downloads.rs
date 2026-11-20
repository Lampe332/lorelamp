//! The download manager: a list of downloads (one per request; a multi-part model or a model
//! with its `mmproj` is one download with several files), at most [`MAX_RUNNING`] at a time, the
//! rest waits. The bytes go through `rp_models::download_file` (`.part` file, `Range` +
//! `If-Range`, 206 checked, SHA-256 against `lfs.oid`, retries with backoff on 429/5xx).
//!
//! The list lives in `<data>/downloads.json` and is written on every change of state (not on
//! progress: the `.part` files are the progress). A download that ran when the app stopped goes
//! on at the next start. Every device gets `{"type":"download", id, done, total, state, speed, …}`
//! at most every 250 ms per download, on every change of state, and `state: "removed"` when one
//! leaves the list; a finished one also sends `{"type":"models"}` (the model folder changed).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use rand::distr::{Alphanumeric, SampleString};
use rp_models::{download_file, free_space, CancellationToken, Error as DlError, Expected, Progress, RepoFile};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

use super::{now, quant, write_atomic, Fetch};
use crate::error::ApiError;
use crate::logs::{Entry, Level};

const FILE: &str = "downloads.json";
pub const MAX_RUNNING: usize = 2;
/// Free space that must stay on the drive after a download.
pub const SPACE_MARGIN: u64 = 1 << 30;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const MAX_FILES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Paused,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobFile {
    /// Path inside the repo (and below the download's folder).
    pub path: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
    #[serde(default)]
    pub finished: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub repo: String,
    pub files: Vec<JobFile>,
    /// `<models folder>/<org>/<name>`.
    pub dir: PathBuf,
    pub state: JobState,
    #[serde(default)]
    pub error: Option<String>,
    pub created: i64,
    #[serde(skip)]
    done: u64,
    #[serde(skip)]
    speed: f64,
    /// When devices last heard of it (progress is held back for 250 ms after that).
    #[serde(skip)]
    sent: Option<Instant>,
}

impl Job {
    fn total(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// The model's name: the first file that is not a projector, without part suffix.
    fn name(&self) -> String {
        let main = self.files.iter().find(|f| !quant::is_mmproj(&f.path)).or(self.files.first());
        let path = main.map(|f| f.path.as_str()).unwrap_or("");
        let base = quant::split_part(path).map(|(b, _, _)| b).unwrap_or(path);
        let base = base.rsplit('/').next().unwrap_or(base);
        base.strip_suffix(".gguf").or_else(|| base.strip_suffix(".GGUF")).unwrap_or(base).to_string()
    }

    /// Bytes on disk: finished files plus the `.part` files.
    fn recount(&mut self) {
        self.done = self
            .files
            .iter()
            .map(|f| if f.finished { f.size } else { part_len(&dest_of(&self.dir, &f.path)) })
            .sum();
    }

    pub fn view(&self) -> Value {
        json!({
            "id": self.id, "repo": self.repo, "name": self.name(), "state": self.state,
            "done": self.done, "total": self.total(), "speed": self.speed.round(), "error": self.error,
            "created": self.created, "folder": self.dir.display().to_string(),
            "files": self.files.iter().map(|f| json!({ "path": f.path, "size": f.size, "finished": f.finished })).collect::<Vec<_>>(),
        })
    }

    fn event(&self) -> Value {
        let mut v = self.view();
        v["type"] = json!("download");
        v
    }
}

/// `dir` + a repo path (`/`-separated) as a path of this system.
pub fn dest_of(dir: &Path, path: &str) -> PathBuf {
    let mut p = dir.to_path_buf();
    p.extend(path.split('/'));
    p
}

fn with_suffix(p: &Path, s: &str) -> PathBuf {
    let mut o = p.as_os_str().to_owned();
    o.push(s);
    PathBuf::from(o)
}

fn part_len(dest: &Path) -> u64 {
    std::fs::metadata(with_suffix(dest, ".part")).map(|m| m.len()).unwrap_or(0)
}

fn remove_parts(job: &Job) {
    for f in job.files.iter().filter(|f| !f.finished) {
        let dest = dest_of(&job.dir, &f.path);
        let _ = std::fs::remove_file(with_suffix(&dest, ".part"));
        let _ = std::fs::remove_file(with_suffix(&dest, ".part.etag"));
    }
}

pub(crate) struct DlState {
    path: PathBuf,
    jobs: Vec<Job>,
    /// One per live task (also while a paused or removed one is still winding down).
    tokens: HashMap<String, CancellationToken>,
    /// Removed while its task ran: the task deletes the parts when it ends.
    remove: HashSet<String>,
    shutting_down: bool,
    tasks: Vec<JoinHandle<()>>,
}

impl DlState {
    pub fn load(data_dir: &Path) -> DlState {
        let path = data_dir.join(FILE);
        let mut jobs: Vec<Job> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        for j in &mut jobs {
            if j.state == JobState::Running {
                j.state = JobState::Queued;
            }
            j.recount();
        }
        DlState { path, jobs, tokens: HashMap::new(), remove: HashSet::new(), shutting_down: false, tasks: Vec::new() }
    }

    fn save(&self) {
        match serde_json::to_vec_pretty(&self.jobs) {
            Ok(bytes) => {
                if let Err(e) = write_atomic(&self.path, &bytes) {
                    eprintln!("downloads.json not written: {e}");
                }
            }
            Err(e) => eprintln!("downloads.json not written: {e}"),
        }
    }

    fn job(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }
}

/// `POST /models/downloads` body.
#[derive(Debug, Deserialize)]
pub struct AddRequest {
    pub repo: String,
    pub file: Option<String>,
    pub files: Option<Vec<String>>,
    /// Only with exactly one file: with `size` the server needs no request to Hugging Face.
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

fn error_code(e: &DlError) -> &'static str {
    match e {
        DlError::Gated(_) => "gated",
        DlError::NotFound(_) => "file_missing",
        DlError::HashMismatch { .. } => "hash_mismatch",
        DlError::SizeMismatch { .. } => "size_mismatch",
        DlError::NoSpace { .. } => "no_space",
        DlError::Io(e) if e.kind() == std::io::ErrorKind::StorageFull => "no_space",
        DlError::Io(_) => "file_error",
        DlError::Status { status: 429, .. } => "rate_limited",
        DlError::Cancelled => "cancelled",
        DlError::Http(_) | DlError::Stalled { .. } | DlError::Status { .. } | DlError::TooManyRedirects | DlError::Json(_) => "network",
    }
}

/// A Hugging Face API error as the answer to a user action.
pub fn hf_error(e: DlError) -> ApiError {
    match e {
        DlError::Gated(_) => ApiError::new(StatusCode::FORBIDDEN, "gated"),
        DlError::NotFound(_) => ApiError::not_found("repo_missing"),
        DlError::Status { status: 429, .. } => ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        _ => ApiError::new(StatusCode::BAD_GATEWAY, "hf_unreachable"),
    }
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

impl Fetch {
    fn dl(&self) -> std::sync::MutexGuard<'_, DlState> {
        self.dl.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn emit(&self, id: &str) {
        let ev = self.dl().job(id).map(|j| {
            j.sent = Some(Instant::now());
            j.event()
        });
        if let Some(ev) = ev {
            self.hub.notify(&ev);
        }
    }

    fn log(&self, level: Level, msg: &str, data: Value) {
        self.hub.log().write(Entry::new(level, "server", "download", msg).data(data));
    }

    pub fn downloads(&self) -> Vec<Value> {
        self.dl().jobs.iter().map(Job::view).collect()
    }

    /// Starts waiting downloads while fewer than [`MAX_RUNNING`] run.
    pub fn pump(self: &Arc<Self>) {
        let mut started = Vec::new();
        {
            let mut st = self.dl();
            if st.shutting_down {
                return;
            }
            st.tasks.retain(|h| !h.is_finished());
            let mut live = st.tokens.len();
            let DlState { jobs, tokens, tasks, .. } = &mut *st;
            for j in jobs.iter_mut() {
                if live >= MAX_RUNNING {
                    break;
                }
                // A task that is still winding down for this one (paused and resumed at once)
                // must end first: two writers on one part file.
                if j.state != JobState::Queued || tokens.contains_key(&j.id) {
                    continue;
                }
                j.state = JobState::Running;
                j.error = None;
                live += 1;
                let token = CancellationToken::new();
                tokens.insert(j.id.clone(), token.clone());
                tasks.push(tokio::spawn(self.clone().run(j.id.clone(), token)));
                started.push(j.id.clone());
            }
            if !started.is_empty() {
                st.save();
            }
        }
        for id in started {
            self.emit(&id);
        }
    }

    async fn run(self: Arc<Self>, id: String, cancel: CancellationToken) {
        let hf = self.hf();
        let (http, retry) = (hf.http().clone(), hf.retry);
        let outcome: Result<(), DlError> = async {
            loop {
                let next = {
                    let mut st = self.dl();
                    let Some(job) = st.job(&id) else { return Err(DlError::Cancelled) };
                    let base: u64 = job.files.iter().filter(|f| f.finished).map(|f| f.size).sum();
                    let remaining: u64 = job
                        .files
                        .iter()
                        .filter(|f| !f.finished)
                        .map(|f| f.size.saturating_sub(part_len(&dest_of(&job.dir, &f.path))))
                        .sum();
                    job.files.iter().position(|f| !f.finished).map(|i| (i, job.files[i].clone(), job.dir.clone(), base, remaining))
                };
                let Some((idx, file, dir, base, remaining)) = next else { return Ok(()) };
                let dest = dest_of(&dir, &file.path);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Ok(available) = free_space(&dir) {
                    if available < remaining + SPACE_MARGIN {
                        return Err(DlError::NoSpace { needed: remaining + SPACE_MARGIN, available });
                    }
                }
                let (me, jid) = (self.clone(), id.clone());
                let expected = Expected { size: Some(file.size), sha256: file.sha256.clone() };
                download_file(&http, &file.url, &dest, &expected, retry, &cancel, move |p: Progress| {
                    me.progress(&jid, base + p.downloaded, p.bytes_per_sec);
                })
                .await?;
                let mut st = self.dl();
                if let Some(job) = st.job(&id) {
                    job.files[idx].finished = true;
                    job.recount();
                }
                st.save();
            }
        }
        .await;
        self.finish(&id, outcome);
    }

    fn progress(&self, id: &str, done: u64, speed: f64) {
        let ev = {
            let mut st = self.dl();
            let Some(job) = st.job(id) else { return };
            job.done = done;
            job.speed = speed;
            let due = job.sent.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
            (due && job.state == JobState::Running).then(|| {
                job.sent = Some(Instant::now());
                job.event()
            })
        };
        if let Some(ev) = ev {
            self.hub.notify(&ev);
        }
    }

    fn finish(self: &Arc<Self>, id: &str, outcome: Result<(), DlError>) {
        let mut models_changed = false;
        let mut removed = false;
        {
            let mut st = self.dl();
            st.tokens.remove(id);
            if st.shutting_down {
                return; // the list keeps "running": the next start goes on
            }
            if st.remove.remove(id) {
                if let Some(i) = st.jobs.iter().position(|j| j.id == id) {
                    let job = st.jobs.remove(i);
                    remove_parts(&job);
                }
                st.save();
                removed = true;
            } else if let Some(job) = st.job(id) {
                job.speed = 0.0;
                match &outcome {
                    Ok(()) => {
                        job.state = JobState::Done;
                        job.recount();
                        models_changed = true;
                    }
                    // Paused (or about to be removed): the state is already what the user chose.
                    Err(DlError::Cancelled) => job.recount(),
                    Err(e) => {
                        job.state = JobState::Failed;
                        job.error = Some(error_code(e).to_string());
                        job.recount();
                    }
                }
                let data = json!({ "id": job.id, "repo": job.repo, "bytes": job.done, "files": job.files.len() });
                let failed = job.error.clone().filter(|_| job.state == JobState::Failed);
                st.save();
                drop(st);
                match (&outcome, failed) {
                    (Ok(()), _) => self.log(Level::Info, "download finished", data),
                    (Err(e), Some(code)) => {
                        let mut data = data;
                        data["error"] = json!(code);
                        data["detail"] = json!(e.to_string());
                        self.log(Level::Warn, "download failed", data);
                    }
                    _ => {}
                }
            }
        }
        if removed {
            self.hub.notify(&json!({ "type": "download", "id": id, "state": "removed" }));
        } else {
            self.emit(id);
        }
        if models_changed {
            self.hub.models_changed();
        }
        self.pump();
    }

    /// A new download from a request. One request to Hugging Face unless the request names
    /// exactly one file with size and SHA-256 (the catalog knows both).
    pub async fn add(self: &Arc<Self>, req: AddRequest) -> Result<Value, ApiError> {
        let repo = req.repo.trim().to_string();
        if !quant::valid_repo(&repo) {
            return Err(ApiError::bad_request("bad_repo"));
        }
        let mut paths: Vec<String> = req.file.into_iter().chain(req.files.unwrap_or_default()).map(|p| p.trim().to_string()).collect();
        paths.dedup();
        if paths.is_empty() || paths.len() > MAX_FILES || !paths.iter().all(|p| quant::valid_file(p)) {
            return Err(ApiError::bad_request("bad_file"));
        }
        if req.sha256.as_deref().is_some_and(|s| !is_sha256(s)) {
            return Err(ApiError::bad_request("bad_sha256"));
        }
        let hf = self.hf();
        let files: Vec<RepoFile> = match (&paths[..], req.size, req.sha256) {
            ([one], Some(size), Some(sha)) if size > 0 => vec![RepoFile { path: one.clone(), size, sha256: Some(sha.to_ascii_lowercase()) }],
            _ => {
                let (info, listing) = hf.repo_files(&repo).await.map_err(hf_error)?;
                if info.gated.is_some() {
                    return Err(ApiError::new(StatusCode::FORBIDDEN, "gated"));
                }
                let mut out: Vec<RepoFile> = Vec::new();
                for p in &paths {
                    let mut wanted = vec![p.clone()];
                    if let Some((base, _, count)) = quant::split_part(p) {
                        wanted = (1..=count).map(|i| format!("{base}-{i:05}-of-{count:05}.gguf")).collect();
                    }
                    for w in wanted {
                        let f = listing.iter().find(|f| f.path == w).ok_or_else(|| ApiError::not_found("file_missing"))?;
                        if !out.iter().any(|o| o.path == f.path) {
                            out.push(f.clone());
                        }
                    }
                }
                out
            }
        };
        let (org, name) = repo.split_once('/').unwrap_or((&repo, ""));
        let dir = self.models_dir().join(org).join(name);
        std::fs::create_dir_all(&dir).map_err(ApiError::storage)?;
        let mut job = Job {
            id: Alphanumeric.sample_string(&mut rand::rng(), 12),
            repo: repo.clone(),
            files: files
                .iter()
                .map(|f| JobFile {
                    url: hf.file_url(&repo, &f.path).to_string(),
                    path: f.path.clone(),
                    size: f.size,
                    sha256: f.sha256.clone(),
                    finished: std::fs::metadata(dest_of(&dir, &f.path)).is_ok_and(|m| m.is_file() && m.len() == f.size),
                })
                .collect(),
            dir: dir.clone(),
            state: JobState::Queued,
            error: None,
            created: now(),
            done: 0,
            speed: 0.0,
            sent: None,
        };
        if job.files.iter().all(|f| f.finished) {
            return Err(ApiError::conflict("exists"));
        }
        job.recount();
        let remaining = job.total() - job.done;
        if let Ok(available) = free_space(&dir) {
            if available < remaining + SPACE_MARGIN {
                return Err(ApiError::new(StatusCode::INSUFFICIENT_STORAGE, "no_space"));
            }
        }
        let view = {
            let mut st = self.dl();
            let busy = st.jobs.iter().any(|j| {
                j.repo == job.repo && j.state != JobState::Done && j.files.iter().any(|f| job.files.iter().any(|n| n.path == f.path))
            });
            if busy {
                return Err(ApiError::conflict("already_downloading"));
            }
            let view = job.view();
            st.jobs.push(job);
            st.save();
            view
        };
        let id = view["id"].as_str().unwrap_or_default().to_string();
        self.log(Level::Info, "download added", json!({ "id": id, "repo": repo, "files": files.len(), "bytes": view["total"] }));
        self.emit(&id);
        self.pump();
        Ok(view)
    }

    pub fn pause(self: &Arc<Self>, id: &str) -> Result<Value, ApiError> {
        let view = {
            let mut st = self.dl();
            let token = st.tokens.get(id).cloned();
            let Some(job) = st.job(id) else { return Err(ApiError::not_found("download_missing")) };
            match job.state {
                JobState::Running => {
                    job.state = JobState::Paused;
                    job.speed = 0.0;
                    if let Some(t) = token {
                        t.cancel();
                    }
                }
                JobState::Queued => job.state = JobState::Paused,
                _ => return Err(ApiError::conflict("bad_state")),
            }
            let view = job.view();
            st.save();
            view
        };
        self.emit(id);
        self.pump();
        Ok(view)
    }

    pub fn resume(self: &Arc<Self>, id: &str) -> Result<Value, ApiError> {
        let view = {
            let mut st = self.dl();
            let Some(job) = st.job(id) else { return Err(ApiError::not_found("download_missing")) };
            match job.state {
                JobState::Paused | JobState::Failed => {
                    job.state = JobState::Queued;
                    job.error = None;
                }
                _ => return Err(ApiError::conflict("bad_state")),
            }
            let view = job.view();
            st.save();
            view
        };
        self.emit(id);
        self.pump();
        Ok(view)
    }

    /// Stops and forgets a download; its `.part` files are deleted. A finished one only leaves
    /// the list (the model stays).
    pub fn delete(self: &Arc<Self>, id: &str) -> Result<(), ApiError> {
        {
            let mut st = self.dl();
            let Some(i) = st.jobs.iter().position(|j| j.id == id) else { return Err(ApiError::not_found("download_missing")) };
            if let Some(t) = st.tokens.get(id).cloned() {
                st.remove.insert(id.to_string());
                t.cancel();
                return Ok(()); // the task deletes the parts once it let go of them
            }
            let job = st.jobs.remove(i);
            if job.state != JobState::Done {
                remove_parts(&job);
            }
            st.save();
        }
        self.log(Level::Info, "download removed", json!({ "id": id }));
        self.hub.notify(&json!({ "type": "download", "id": id, "state": "removed" }));
        Ok(())
    }

    /// Stops every running download without changing the list (they go on at the next start)
    /// and waits until their files are closed.
    pub async fn shutdown(&self) {
        let tasks = {
            let mut st = self.dl();
            st.shutting_down = true;
            for t in st.tokens.values() {
                t.cancel();
            }
            std::mem::take(&mut st.tasks)
        };
        for t in tasks {
            let _ = tokio::time::timeout(Duration::from_secs(10), t).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disk that fills up while the file is written is "no_space", not "file_error".
    #[test]
    fn full_disk_while_writing_is_no_space() {
        assert_eq!(error_code(&DlError::Io(std::io::Error::from(std::io::ErrorKind::StorageFull))), "no_space");
        #[cfg(windows)]
        for code in [39, 112] {
            // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL as the write returns them.
            assert_eq!(error_code(&DlError::Io(std::io::Error::from_raw_os_error(code))), "no_space", "{code}");
        }
        assert_eq!(error_code(&DlError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied))), "file_error");
    }
}
