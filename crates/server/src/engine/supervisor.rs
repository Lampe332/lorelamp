//! The supervisor keeps a model server running for as long as the app lives: if nothing answers
//! on the port it starts llama-server with what `Desired` asks for (the active model profile),
//! restarts it if it dies, switches it when the profile asks for other start arguments (never
//! while a reply is being written), and stops it again when the app exits (only if it started
//! it). A lock file makes sure only one app instance manages it.

use std::fs::File;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rp_core::llama_args;
use serde_json::json;

use super::probe::{probe, search_path};
use super::{Desired, LaunchSpec};
use crate::logs::{Entry, Level, Logger};
use crate::{EngineNote, Note};

/// After this many crashes within the crash window the supervisor gives up (no restart loop that
/// keeps filling the GPU memory) and stays so until the start arguments change, the user asks for
/// a restart (`retry`) or the app restarts. Time alone never starts it again.
const MAX_CRASHES: usize = 3;
/// After stopping the old server for a switch: how long to wait for its port to be free.
const PORT_FREE_WAIT: Duration = Duration::from_secs(15);

/// The supervisor's clock; shorter only in tests.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How often it looks at the model server (sooner when woken).
    pub check_every: Duration,
    /// Pause before restarting a crashed server, so the GPU memory is free again.
    pub restart_pause: Duration,
    /// Crashes older than this do not count towards giving up.
    pub crash_window: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { check_every: Duration::from_secs(3), restart_pause: Duration::from_secs(10), crash_window: Duration::from_secs(300) }
    }
}

/// Who holds the model server's port while none of ours runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PortState {
    Free,
    /// A llama-server started elsewhere answers there: use it, start none.
    Llama,
    /// Another program holds it: a model server could not listen there.
    Other,
}

/// What the supervisor should do on one check. Separated from the process handling for tests.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// A server is there (one started elsewhere), or we are waiting out a pause.
    Wait,
    Start,
    /// Too many crashes: no further start until the start arguments change or the user asks.
    GiveUp,
    /// Another program holds the port: no start until it is free or the port is changed.
    PortBusy,
}

pub fn next_step(port: PortState, gave_up: bool, crashes_in_window: usize, since_last_crash: Option<Duration>, pause: Duration) -> Step {
    match port {
        PortState::Llama => Step::Wait,
        PortState::Other => Step::PortBusy,
        PortState::Free if gave_up || crashes_in_window >= MAX_CRASHES => Step::GiveUp,
        PortState::Free if since_last_crash.is_some_and(|d| d < pause) => Step::Wait,
        PortState::Free => Step::Start,
    }
}

/// Why no model server runs although one should (`GET /engine` → `problem`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Problem {
    /// Another program (not a llama-server) holds `port`; change the setting `engine_port`.
    PortBusy { port: u16 },
    /// The model server crashed `crashes` times in a row; no further start until asked.
    GaveUp { crashes: usize },
}

/// Asked on every check: what should run right now.
pub type DesiredFn = Arc<dyn Fn() -> Desired + Send + Sync>;
/// Told about every change of the model server: (event, profile id, port). Events: `starting`,
/// `switching`, `ready`, `crashed`, `failed`, `unloaded`, `foreign` (a llama-server started
/// elsewhere answers on the port), `port_busy` (another program holds the port).
pub type EventFn = Arc<dyn Fn(&'static str, Option<i64>, u16) + Send + Sync>;

/// What the rest of the server may see of the supervisor.
#[derive(Debug, Default, Clone)]
struct View {
    /// This app instance holds `engine.lock` and manages the model server.
    managing: bool,
    /// A llama-server we did not start answers on the port.
    foreign: bool,
    /// Another program holds this port.
    port_busy: Option<u16>,
    /// Gave up after this many crashes (0: not given up).
    gave_up: usize,
    /// What our running server was started with.
    running: Option<LaunchSpec>,
    /// Flags of the profile the build does not know (left out at the last start).
    dropped_args: Vec<String>,
    /// Build of the running server, from its `--version`.
    version: Option<String>,
}

struct Shared {
    data_dir: PathBuf,
    child: Mutex<Option<Child>>,
    stopping: AtomicBool,
    /// The user asked for a new series of start attempts (`retry`).
    retry: AtomicBool,
    view: Mutex<View>,
    wake: (Mutex<bool>, Condvar),
    log: Logger,
    timing: Timing,
}

/// Handle on the running supervisor; cheap to clone.
#[derive(Clone)]
pub struct EngineHandle(Arc<Shared>);

/// What the supervisor remembers between two checks, so the log gets transitions, not a line
/// every three seconds.
#[derive(Default)]
struct Watch {
    crashes: Vec<Instant>,
    /// Our server was started then and has not answered `/health` yet.
    loading_since: Option<Instant>,
    /// The last problem logged for the settings (logged again only when it changes).
    config_error: Option<String>,
    nothing_configured: bool,
    /// Gave up after too many crashes; stays until the start arguments change or `retry`.
    gave_up: bool,
    foreign: bool,
    port_busy: bool,
    lock_conflict: bool,
    /// A switch waits for the running reply (logged once).
    switch_waiting: bool,
    /// The next start is the second half of a switch.
    switching: bool,
    /// What the last start attempt used; another spec resets the crash count.
    last_spec: Option<LaunchSpec>,
    /// Programs whose `--help` could not be read (logged once each).
    probe_failed: Vec<PathBuf>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl EngineHandle {
    /// Starts the supervisor thread. `note` gets what the UI shows when something is wrong.
    pub fn start(data_dir: PathBuf, note: EngineNote, log: Logger, desired: DesiredFn, events: EventFn) -> EngineHandle {
        Self::start_with(data_dir, note, log, desired, events, Timing::default())
    }

    /// `start` with another clock (tests).
    pub fn start_with(data_dir: PathBuf, note: EngineNote, log: Logger, desired: DesiredFn, events: EventFn, timing: Timing) -> EngineHandle {
        let handle = EngineHandle(Arc::new(Shared {
            data_dir,
            child: Mutex::new(None),
            stopping: AtomicBool::new(false),
            retry: AtomicBool::new(false),
            view: Mutex::new(View::default()),
            wake: (Mutex::new(false), Condvar::new()),
            log,
            timing,
        }));
        let sh = handle.0.clone();
        std::thread::spawn(move || {
            let mut watch = Watch::default();
            // Held for as long as this instance manages the model server.
            let mut held: Option<File> = None;
            while !sh.stopping.load(Ordering::Relaxed) {
                if held.is_none() {
                    held = lock_file(&sh.data_dir.join("engine.lock"));
                    if held.is_some() && watch.lock_conflict {
                        watch.lock_conflict = false;
                        sh.log.info("engine", "lock", "engine lock free again: this window manages the model server now");
                    }
                }
                lock(&sh.view).managing = held.is_some();
                if held.is_some() {
                    check(&sh, &note, &mut watch, &desired, &events);
                } else {
                    *lock(&note) = None; // another open window of the app looks after the model
                    if !watch.lock_conflict {
                        watch.lock_conflict = true;
                        sh.log.write(Entry::new(
                            Level::Warn,
                            "engine",
                            "lock",
                            "engine.lock is held by another app window: that one manages the model server",
                        ));
                    }
                }
                let (flag, cv) = &sh.wake;
                let mut woken = lock(flag);
                if !*woken {
                    woken = cv.wait_timeout(woken, sh.timing.check_every).map(|(g, _)| g).unwrap_or_else(|p| p.into_inner().0);
                }
                *woken = false;
            }
        });
        handle
    }

    /// Look again now (after the active profile changed) instead of at the next check.
    pub fn wake(&self) {
        let (flag, cv) = &self.0.wake;
        *lock(flag) = true;
        cv.notify_all();
    }

    pub fn managing(&self) -> bool {
        lock(&self.0.view).managing
    }

    pub fn foreign(&self) -> bool {
        lock(&self.0.view).foreign
    }

    /// Why no model server runs although one should (None: nothing wrong that we know of).
    pub fn problem(&self) -> Option<Problem> {
        let v = lock(&self.0.view);
        match (v.port_busy, v.gave_up) {
            (Some(port), _) => Some(Problem::PortBusy { port }),
            (None, 0) => None,
            (None, crashes) => Some(Problem::GaveUp { crashes }),
        }
    }

    /// The user asks for a new series of start attempts after the supervisor gave up. Answers
    /// whether it had given up (only then does a start follow).
    pub fn retry(&self) -> bool {
        let gave_up = lock(&self.0.view).gave_up > 0;
        self.0.retry.store(true, Ordering::Relaxed);
        self.wake();
        gave_up
    }

    /// What our running server was started with (None: none of ours runs).
    pub fn running(&self) -> Option<LaunchSpec> {
        lock(&self.0.view).running.clone()
    }

    pub fn dropped_args(&self) -> Vec<String> {
        lock(&self.0.view).dropped_args.clone()
    }

    /// Process id of the model server this app started, while it runs (for its share of the VRAM).
    pub fn pid(&self) -> Option<u32> {
        lock(&self.0.child).as_ref().map(|c| c.id())
    }

    pub fn version(&self) -> Option<String> {
        lock(&self.0.view).version.clone()
    }

    /// Call when the app exits: stops the server if this app started it, also while it still loads.
    /// (Should the app end without this, the job object ends it: `job.rs`.)
    pub fn shutdown(&self) {
        self.0.stopping.store(true, Ordering::Relaxed);
        self.wake();
        // Taking the slot waits for a check that is starting a server right now; after it no
        // check starts another (`stopping`).
        if let Some(mut child) = lock(&self.0.child).take() {
            let _ = child.kill();
            let _ = child.wait();
            self.0.log.info("engine", "stop", "model server stopped (app closed)");
        }
        lock(&self.0.view).running = None;
    }
}

/// Opens `path` so that nobody else can open it until the handle is dropped. Only the app
/// instance that holds it manages the model server: two instances would each start one before
/// either answers on the port, and two loaded models overfill the GPU memory.
pub fn lock_file(path: &Path) -> Option<File> {
    std::fs::create_dir_all(path.parent()?).ok()?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
        options.open(path).ok()
    }
    // Elsewhere files cannot be opened exclusively; an advisory lock (flock on Unix) on the
    // open file does the same between app instances and ends with the handle or the process.
    #[cfg(not(windows))]
    {
        let file = options.open(path).ok()?;
        file.try_lock().ok()?;
        Some(file)
    }
}

/// Something already listens on the port (e.g. a llama-server started elsewhere).
pub fn port_in_use(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// llama-server answers `/health` with 200 once the model is loaded (503 while it loads).
pub fn health_ok(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    if s.write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    let n = s.read(&mut head).unwrap_or(0);
    head[..n].starts_with(b"HTTP/1.1 200") || head[..n].starts_with(b"HTTP/1.0 200")
}

/// Who holds the port: nobody, a llama-server (its `/health`: 200 with a `status`, or 503 while
/// it loads the model), or another program (anything else, also no answer at all).
pub fn port_state(port: u16) -> PortState {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return PortState::Free };
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    if s.write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return PortState::Other;
    }
    let mut answer = Vec::new();
    let _ = s.take(4096).read_to_end(&mut answer);
    if llama_health(&String::from_utf8_lossy(&answer)) {
        PortState::Llama
    } else {
        PortState::Other
    }
}

/// llama-server's answer to `/health`: `200 {"status":"ok"}`, or `503 … "Loading model"`.
fn llama_health(answer: &str) -> bool {
    let Some((head, body)) = answer.split_once("\r\n\r\n") else { return false };
    let status = head.split_whitespace().nth(1).unwrap_or("");
    head.starts_with("HTTP/") && ((status == "200" && body.contains("\"status\"")) || (status == "503" && body.to_ascii_lowercase().contains("loading")))
}

/// Start llama-server hidden in the background with these (already filtered) arguments;
/// output goes to `llama-server.log` in the data folder.
/// A failure is the note the UI shows (`no_engine_file`, `no_model_file`, `start_failed`, `start_error`).
pub fn spawn(spec: &LaunchSpec, args: &[String], data_dir: &Path) -> Result<Child, Note> {
    if !spec.exe.is_file() {
        return Err(Note::with("no_engine_file", spec.exe.display()));
    }
    if !spec.model.is_file() {
        return Err(Note::with("no_model_file", spec.model.display()));
    }
    let error = |e: std::io::Error| Note::with("start_error", e);
    std::fs::create_dir_all(data_dir).map_err(error)?;
    // Keep the previous run's log: after a crash it is the only place that says why.
    let _ = std::fs::rename(data_dir.join("llama-server.log"), data_dir.join("llama-server.prev.log"));
    let log = File::create(data_dir.join("llama-server.log")).map_err(error)?;
    let log_err = log.try_clone().map_err(error)?;

    let mut cmd = Command::new(&spec.exe);
    cmd.arg("-m")
        .arg(&spec.model)
        .args(["--host", "127.0.0.1", "--port", &spec.port.to_string()])
        .args(args)
        .env("PATH", search_path(&spec.extra_path).map_err(|e| Note::with("start_error", e))?)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|e| Note::with("start_failed", e))
}

/// `spawn`, then into the job that ends it with the app (`job.rs`).
fn spawn_adopted(sh: &Shared, spec: &LaunchSpec, args: &[String]) -> Result<Child, Note> {
    let child = spawn(spec, args, &sh.data_dir)?;
    if let Err(e) = super::job::adopt(&child) {
        sh.log.write(Entry::new(Level::Warn, "engine", "start", format!("model server may outlive a hard end of the app: {e}")));
    }
    Ok(child)
}

/// What the log keeps of a start: the model's file name and every setting as it is (the export
/// shortens paths in the user's folder).
fn spec_data(spec: &LaunchSpec, args: &[String]) -> serde_json::Value {
    json!({
        "profile_id": spec.profile_id,
        "model_file": spec.model_file_name(),
        "model": spec.model.display().to_string(),
        "llama_server": spec.exe.display().to_string(),
        "extra_path": spec.extra_path.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "port": spec.port,
        "args": args,
    })
}

/// The profile's arguments without the flags the app does not allow (`llama_args::allowed_args`:
/// a profile saved before the allowlist keeps working without them) and without the flags this
/// build does not know (Err of the probe: the allowed ones, otherwise unchecked). Both kinds land in
/// `dropped`.
fn filtered_args(sh: &Shared, spec: &LaunchSpec, watch: &mut Watch) -> (Vec<String>, Vec<String>, Option<String>) {
    let allowed = llama_args::allowed_args(&spec.args);
    if !allowed.dropped.is_empty() {
        sh.log.write(
            Entry::new(Level::Warn, "engine", "args", "start flags the app does not allow were left out")
                .data(json!({ "dropped": allowed.dropped, "profile_id": spec.profile_id })),
        );
    }
    let mut dropped = allowed.dropped;
    match &*probe(&spec.exe, &spec.extra_path) {
        Ok(p) => {
            let f = llama_args::filter_args(&allowed.args, &p.flags);
            if !f.dropped.is_empty() {
                sh.log.write(
                    Entry::new(Level::Warn, "engine", "args", "start flags this llama-server does not know were left out")
                        .data(json!({ "dropped": f.dropped, "version": p.version, "profile_id": spec.profile_id })),
                );
            }
            dropped.extend(f.dropped);
            (f.args, dropped, p.version.clone())
        }
        Err(e) => {
            if !watch.probe_failed.contains(&spec.exe) {
                watch.probe_failed.push(spec.exe.clone());
                sh.log.write(
                    Entry::new(Level::Warn, "engine", "args", format!("llama-server --help not readable ({e}); start flags sent without the build's check"))
                        .data(json!({ "llama_server": spec.exe.display().to_string() })),
                );
            }
            (allowed.args, dropped, None)
        }
    }
}

fn check(sh: &Shared, note: &EngineNote, watch: &mut Watch, desired: &DesiredFn, events: &EventFn) {
    let desired = desired();
    if desired.unloaded {
        unload(sh, note, watch, desired.busy, events);
        return;
    }
    if !matches!(desired.spec, Ok(Some(_))) {
        lock(&sh.view).port_busy = None;
        watch.port_busy = false;
    }
    let spec = match desired.spec {
        Ok(Some(spec)) => spec,
        Ok(None) => {
            if !watch.nothing_configured {
                watch.nothing_configured = true;
                sh.log.write(Entry::new(Level::Warn, "engine", "config", "no active model profile and no engine.json: nothing to start"));
            }
            *lock(note) = Some(Note::new("no_profile"));
            return;
        }
        Err(e) => {
            if watch.config_error.as_deref() != Some(e.as_str()) {
                sh.log.write(Entry::new(Level::Error, "engine", "config", format!("model settings not readable: {e}")));
                watch.config_error = Some(e.clone());
            }
            *lock(note) = Some(Note::with("config_unreadable", e));
            return;
        }
    };
    watch.config_error = None;
    watch.nothing_configured = false;
    if spec.exe.is_file() {
        // Read the build's `--help` now (once per program file), not while holding the process
        // slot that `shutdown` needs.
        let _ = probe(&spec.exe, &spec.extra_path);
    }
    let retry = sh.retry.swap(false, Ordering::Relaxed);
    if retry || watch.last_spec.as_ref().is_some_and(|last| !last.same_launch(&spec)) {
        // Another profile (or the user asks): its own chances, whatever the last one did.
        if retry && watch.gave_up {
            sh.log.write(Entry::new(Level::Info, "engine", "retry", "new start attempts asked for").data(json!({ "profile_id": spec.profile_id })));
            *lock(note) = None;
        }
        watch.crashes.clear();
        watch.gave_up = false;
        watch.last_spec = None;
        lock(&sh.view).gave_up = 0;
    }
    let events = |event: &'static str| events(event, spec.profile_id, spec.port);

    let mut slot = lock(&sh.child);
    if sh.stopping.load(Ordering::Relaxed) {
        return; // the app is closing: start nothing more
    }
    if let Some(running) = slot.as_mut() {
        match running.try_wait() {
            Ok(None) => {
                let current = lock(&sh.view).running.clone();
                let outdated = current.as_ref().is_some_and(|c| !c.same_launch(&spec));
                if !outdated {
                    watch.switch_waiting = false;
                    if let Some(since) = watch.loading_since {
                        if health_ok(spec.port) {
                            watch.loading_since = None;
                            sh.log.write(
                                Entry::new(Level::Info, "engine", "ready", "model server ready").data(json!({
                                    "load_seconds": since.elapsed().as_secs_f32().round(), "port": spec.port, "profile_id": spec.profile_id,
                                })),
                            );
                            events("ready");
                        }
                    }
                    return;
                }
                if desired.busy {
                    if !watch.switch_waiting {
                        watch.switch_waiting = true;
                        sh.log.info("engine", "switch", "model profile changed: the switch waits until the reply is written");
                    }
                    return;
                }
                watch.switch_waiting = false;
                let from = current.and_then(|c| c.profile_id);
                sh.log.write(
                    Entry::new(Level::Info, "engine", "switch", "switching the model server to another profile")
                        .data(json!({ "from_profile_id": from, "to_profile_id": spec.profile_id, "model_file": spec.model_file_name() })),
                );
                events("switching");
                let _ = running.kill();
                let _ = running.wait();
                *slot = None;
                lock(&sh.view).running = None;
                watch.loading_since = None;
                watch.switching = true;
                let deadline = Instant::now() + PORT_FREE_WAIT;
                while port_in_use(spec.port) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            Ok(Some(status)) => {
                watch.crashes.push(Instant::now());
                watch.loading_since = None;
                sh.log.write(
                    Entry::new(Level::Error, "engine", "crash", format!("model server exited ({status})"))
                        .data(json!({ "exit_code": status.code(), "crashes_in_5_min": watch.crashes.len(), "profile_id": spec.profile_id })),
                );
                *lock(note) = Some(Note::with("exited", status));
                events("crashed");
                *slot = None;
                lock(&sh.view).running = None;
            }
            Err(e) => {
                sh.log.write(Entry::new(Level::Error, "engine", "crash", format!("model server cannot be queried: {e}")));
                *lock(note) = Some(Note::with("unqueryable", e));
                *slot = None;
                lock(&sh.view).running = None;
            }
        }
    }

    let window = sh.timing.crash_window;
    watch.crashes.retain(|t| t.elapsed() < window);
    // Here none of ours runs (a running one returned above or was just stopped).
    let port = port_state(spec.port);
    if port != PortState::Llama {
        watch.foreign = false;
    }
    if port != PortState::Other {
        watch.port_busy = false;
    }
    {
        let mut view = lock(&sh.view);
        view.foreign = port == PortState::Llama;
        view.port_busy = (port == PortState::Other).then_some(spec.port);
    }
    match next_step(port, watch.gave_up, watch.crashes.len(), watch.crashes.last().map(Instant::elapsed), sh.timing.restart_pause) {
        Step::Wait => {
            if port == PortState::Llama {
                *lock(note) = None; // a server answers; nothing to report
                if !watch.foreign {
                    watch.foreign = true;
                    sh.log.write(
                        Entry::new(Level::Info, "engine", "start", format!("a llama-server already answers on port {}; not starting one", spec.port))
                            .data(json!({ "port": spec.port })),
                    );
                    events("foreign");
                }
            }
        }
        Step::PortBusy => {
            // Never stop the other program: it is not ours.
            *lock(note) = Some(Note { port: Some(spec.port), ..Note::new("port_busy") });
            if !watch.port_busy {
                watch.port_busy = true;
                sh.log.write(
                    Entry::new(Level::Warn, "engine", "port_busy", format!("port {} is held by another program; not starting the model server", spec.port))
                        .data(json!({ "port": spec.port, "profile_id": spec.profile_id })),
                );
                events("port_busy");
            }
        }
        Step::GiveUp => {
            if !watch.gave_up {
                watch.gave_up = true;
                lock(&sh.view).gave_up = watch.crashes.len().max(MAX_CRASHES);
                sh.log.write(
                    Entry::new(
                        Level::Error,
                        "engine",
                        "giveup",
                        format!("model server crashed {MAX_CRASHES} times within {} s; no further start until the profile changes or a restart is asked for", window.as_secs()),
                    )
                    .data(json!({ "crashes": watch.crashes.len(), "profile_id": spec.profile_id })),
                );
                events("failed");
            }
            let mut n = lock(note);
            if !n.as_ref().is_some_and(|x| x.gave_up) {
                let cause = n.take().unwrap_or_else(|| Note::new("not_starting"));
                *n = Some(Note { gave_up: true, ..cause });
            }
        }
        Step::Start => {
            let restart = !watch.crashes.is_empty();
            let (args, dropped, version) = filtered_args(sh, &spec, watch);
            watch.last_spec = Some(spec.clone());
            match spawn_adopted(sh, &spec, &args) {
                Ok(started) => {
                    let (category, message) = if std::mem::take(&mut watch.switching) {
                        ("switch", "model server started with the new profile".to_string())
                    } else if restart {
                        ("restart", format!("model server restarted (attempt {} of {MAX_CRASHES})", watch.crashes.len() + 1))
                    } else {
                        ("start", "model server started".to_string())
                    };
                    let mut data = spec_data(&spec, &args);
                    data["pid"] = json!(started.id());
                    data["version"] = json!(version);
                    sh.log.write(Entry::new(Level::Info, "engine", category, message).data(data));
                    watch.loading_since = Some(Instant::now());
                    *slot = Some(started);
                    *lock(&sh.view) = View { managing: true, running: Some(spec.clone()), dropped_args: dropped, version, ..View::default() };
                    *lock(note) = None;
                    events("starting");
                }
                Err(e) => {
                    watch.switching = false;
                    watch.crashes.push(Instant::now()); // a start that fails counts like a crash
                    sh.log.write(Entry::new(Level::Error, "engine", "start", format!("model server start failed: {} {}", e.code, e.detail.as_deref().unwrap_or(""))).data(spec_data(&spec, &args)));
                    *lock(note) = Some(e);
                    events("crashed");
                }
            }
        }
    }
}

/// Unloaded: stop our model server (never while a reply is written) and start none until the
/// model is wanted again. A server started elsewhere is left alone.
fn unload(sh: &Shared, note: &EngineNote, watch: &mut Watch, busy: bool, events: &EventFn) {
    *lock(note) = None;
    watch.switch_waiting = false;
    watch.switching = false;
    watch.loading_since = None;
    watch.port_busy = false;
    lock(&sh.view).port_busy = None;
    let mut slot = lock(&sh.child);
    let Some(running) = slot.as_mut() else { return };
    if busy {
        return;
    }
    let (profile_id, port) = lock(&sh.view).running.as_ref().map_or((None, 0), |r| (r.profile_id, r.port));
    let _ = running.kill();
    let _ = running.wait();
    *slot = None;
    lock(&sh.view).running = None;
    sh.log.write(
        Entry::new(Level::Info, "engine", "unload", "model server stopped: unloaded to free the graphics memory")
            .data(json!({ "profile_id": profile_id })),
    );
    events("unloaded", profile_id, port);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_holder_of_the_engine_lock() {
        let path = std::env::temp_dir().join(format!("rp-app-lock-test-{}", std::process::id())).join("engine.lock");
        let first = lock_file(&path).expect("first holder gets it");
        assert!(lock_file(&path).is_none(), "a second instance must not get it");
        drop(first);
        assert!(lock_file(&path).is_some(), "free again once the holder is gone");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn supervisor_decisions() {
        let s = Duration::from_secs;
        let pause = Timing::default().restart_pause;
        use PortState::*;
        assert_eq!(next_step(Free, false, 0, None, pause), Step::Start, "nothing runs: start");
        assert_eq!(next_step(Llama, false, 0, None, pause), Step::Wait, "a server is there: never start a second one");
        assert_eq!(next_step(Llama, false, 5, Some(s(1)), pause), Step::Wait);
        assert_eq!(next_step(Other, false, 0, None, pause), Step::PortBusy, "another program holds the port: no start");
        assert_eq!(next_step(Free, false, 1, Some(s(2)), pause), Step::Wait, "let the GPU memory settle after a crash");
        assert_eq!(next_step(Free, false, 1, Some(s(11)), pause), Step::Start);
        assert_eq!(next_step(Free, false, MAX_CRASHES, Some(s(60)), pause), Step::GiveUp, "no restart loop");
        assert_eq!(next_step(Free, true, 0, None, pause), Step::GiveUp, "the crashes left the window: still given up");
    }

    #[test]
    fn what_a_llama_server_answers_on_health() {
        assert!(llama_health("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"status\":\"ok\"}"));
        assert!(llama_health("HTTP/1.1 503 Service Unavailable\r\n\r\n{\"error\":{\"code\":503,\"message\":\"Loading model\",\"type\":\"unavailable_error\"}}"));
        assert!(!llama_health("HTTP/1.1 404 Not Found\r\n\r\nnot here"));
        assert!(!llama_health("HTTP/1.1 200 OK\r\n\r\n<html>router</html>"));
        assert!(!llama_health(""), "no answer at all");
    }

    /// A port nobody listens on (taken from the system, then given back).
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    type Seen = Arc<Mutex<Vec<(&'static str, u16)>>>;

    /// A real supervisor on a short clock. "A model server that does not start" is this test
    /// program itself: started with llama-server's arguments (`-m …`) it rejects them and exits
    /// at once, like a llama-server with a bad profile. Its `--help` is read as the probe.
    fn supervisor(name: &str, spec: Arc<Mutex<LaunchSpec>>) -> (EngineHandle, Seen, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rp-app-supervisor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let seen: Seen = Arc::default();
        let desired: DesiredFn = Arc::new(move || Desired { spec: Ok(Some(lock(&spec).clone())), busy: false, unloaded: false });
        let s = seen.clone();
        let events: EventFn = Arc::new(move |e, _, port| lock(&s).push((e, port)));
        let timing = Timing { check_every: Duration::from_millis(50), restart_pause: Duration::from_millis(100), crash_window: Duration::from_millis(2000) };
        let handle = EngineHandle::start_with(dir.clone(), Arc::new(Mutex::new(None)), Logger::start(&dir), desired, events, timing);
        (handle, seen, dir)
    }

    fn failing_spec(dir: &str, port: u16) -> LaunchSpec {
        let model = std::env::temp_dir().join(format!("rp-app-supervisor-{dir}-{}.gguf", std::process::id()));
        std::fs::write(&model, b"GGUF").unwrap();
        LaunchSpec { exe: std::env::current_exe().unwrap(), extra_path: vec![], model, port, args: vec![], profile_id: Some(1) }
    }

    fn count(seen: &Seen, event: &str) -> usize {
        lock(seen).iter().filter(|(e, _)| *e == event).count()
    }

    fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !ok() {
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn gives_up_for_good_until_the_profile_changes_or_a_restart_is_asked() {
        let spec = Arc::new(Mutex::new(failing_spec("giveup", free_port())));
        let (engine, seen, dir) = supervisor("giveup", spec.clone());
        wait_until("gave up", || count(&seen, "failed") == 1);
        assert_eq!(count(&seen, "starting"), MAX_CRASHES, "the allowed attempts, then no more");
        assert_eq!(engine.problem(), Some(Problem::GaveUp { crashes: MAX_CRASHES }));
        // Twice the crash window: every crash has left it, and still no start.
        std::thread::sleep(Duration::from_millis(4500));
        assert_eq!((count(&seen, "starting"), count(&seen, "failed")), (MAX_CRASHES, 1), "no start after the window: {:?}", lock(&seen));

        // The user asks for a restart: exactly one new series.
        assert!(engine.retry(), "it had given up");
        wait_until("gave up again", || count(&seen, "failed") == 2);
        std::thread::sleep(Duration::from_millis(4500));
        assert_eq!((count(&seen, "starting"), count(&seen, "failed")), (2 * MAX_CRASHES, 2), "{:?}", lock(&seen));

        // A changed profile (other start arguments): one more series.
        lock(&spec).args = vec!["-c".into(), "2048".into()];
        engine.wake();
        wait_until("gave up with the new profile", || count(&seen, "failed") == 3);
        std::thread::sleep(Duration::from_millis(4500));
        assert_eq!((count(&seen, "starting"), count(&seen, "failed")), (3 * MAX_CRASHES, 3), "{:?}", lock(&seen));
        assert_eq!(engine.problem(), Some(Problem::GaveUp { crashes: MAX_CRASHES }));
        engine.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_port_held_by_another_program_is_reported_and_left_alone() {
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); // listens, never answers
        let busy = other.local_addr().unwrap().port();
        let spec = Arc::new(Mutex::new(failing_spec("port", busy)));
        let (engine, seen, dir) = supervisor("port", spec.clone());
        wait_until("port reported", || count(&seen, "port_busy") == 1);
        assert_eq!(engine.problem(), Some(Problem::PortBusy { port: busy }));
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!((count(&seen, "starting"), engine.pid(), engine.foreign()), (0, None, false), "no start attempt");
        assert_eq!(count(&seen, "port_busy"), 1, "said once, not every check");
        assert!(port_in_use(busy), "the other program still holds its port");

        // The setting moves to a free port: the model server starts there.
        let free = free_port();
        lock(&spec).port = free;
        engine.wake();
        wait_until("started on the new port", || lock(&seen).contains(&("starting", free)));
        assert_ne!(engine.problem(), Some(Problem::PortBusy { port: busy }));
        assert!(port_in_use(busy), "never stopped");
        drop(other);
        engine.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// H1/H2: a profile saved before the allowlist starts without what it refuses; the left-out
    /// flags show in `dropped_args` (nothing crashes on them).
    #[test]
    fn a_profile_with_refused_flags_starts_without_them() {
        let mut s = failing_spec("refused", free_port());
        let target = std::env::temp_dir().join(format!("rp-app-supervisor-refused-{}.bat", std::process::id()));
        s.args = ["--log-file", &target.display().to_string(), "--chat-template", "{{ messages[1].content }}", "-hfr", "a/b", "-c", "2048"]
            .iter()
            .map(|a| a.to_string())
            .collect();
        let (engine, seen, dir) = supervisor("refused", Arc::new(Mutex::new(s)));
        wait_until("a start", || count(&seen, "starting") >= 1);
        wait_until("the start's view", || !engine.dropped_args().is_empty());
        let dropped = engine.dropped_args();
        for flag in ["--log-file", "--chat-template", "-hfr"] {
            assert!(dropped.iter().any(|d| d == flag), "{flag} not in {dropped:?}");
        }
        assert!(!dropped.iter().any(|d| d.contains("rp-app-supervisor-refused")), "values are not shown: {dropped:?}");
        engine.shutdown();
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn spawn_rejects_missing_files() {
        let spec = LaunchSpec {
            exe: "Z:\\does-not-exist\\llama-server.exe".into(),
            extra_path: vec![],
            model: "Z:\\nope.gguf".into(),
            port: 8080,
            args: vec![],
            profile_id: None,
        };
        assert_eq!(spawn(&spec, &[], &std::env::temp_dir()).unwrap_err().code, "no_engine_file");
    }
}
