//! Benchmark / auto-tune of a model profile: runs the candidates `rp_core::bench` chooses one after
//! the other through the supervisor (never a second model server), measures each through
//! llama-server's `timings` and the WDDM counters, and keeps the table with three recommendations
//! (Tempo / Ausgewogen / Kontext) in the settings (`bench_result_<profile id>`). The profile itself
//! only changes when the user applies a recommendation. When it ends, the active profile runs again.
//!
//! Whether the model computes on a GPU (`system::computing_gpu`: a GPU with real graphics memory, a
//! build that is not the CPU build, `-ngl` not 0) decides the rest; without one it is a CPU run,
//! whose steps and candidates are checked against the RAM. Before the first candidate the context
//! steps are fitted to the PC (`plan_for`: a small GPU starts at 4k, steps that surely do not fit
//! even with every expert on the CPU are left out; a CPU run measures three steps spread over the
//! range). A CPU run starts with runs of 512 + 32 tokens instead of 2,048 + 64, and after the first
//! warmup a slow PC gets shorter runs so that at least three candidates fit the budget
//! (`rp_core::bench::run_sizes`; `RP_BENCH_*` sizes are kept as they are).
//! Per candidate: stop the running server and wait until the memory it held is back, read the
//! free VRAM, available RAM (free + standby) and commit, skip it (with the numbers) if its CPU part
//! does not fit, start it, watch the memory while it loads and runs (`rp_core::bench::guard`: commit
//! < 2 GB left, available RAM below the candidate's floor of 1–3 GB, shared memory > 6 GB above
//! its own pinned part), read the
//! overflow once it is up (process shared − pinned host buffers from the server log; > 300 MiB =
//! does not fit, not measured further), then a warmup of the same size (discarded) and at least
//! three runs of a ~2k-token prompt with 64 new tokens, more (up to six) while the generation
//! speeds spread more than 10 % and the candidate's share of the budget allows; the row keeps the
//! medians, every run and the spread. The result carries the memory conditions at the start
//! (`conditions`); the profile keeps one result per kind of conditions (`rp_core::bench::keep_results`).
//! A result names what it was measured with (`measured_with`: model file, engine, profile) and the
//! run sizes (`prompt_tokens`, `gen_tokens`); `unmeasured_stages` lists the steps the time did not
//! reach. The store delivers a result only while the profile still starts that model file with
//! that engine.
//! Every device gets `{"type":"bench", "profile_id", "stage", "done", "total", "elapsed_s",
//! "budget_s", "candidate", "rows"}`; `stage` is `loading`, `measuring`, `done`, `cancelled` or
//! `failed` (and `result` on `done`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rp_core::bench::{self, Candidate, Conditions, Memory, Outcome, Plan, Room, Row};
use rp_core::gguf::GgufInfo;
use rp_core::vram::{self, Load};
use serde_json::{json, Value};

use crate::chat::Hub;
use crate::engine::{EngineHandle, LaunchSpec};
use crate::logs::{Entry, Level};
use crate::system::{main_gpu, memory_now};

/// How long and how large the runs are. `RP_BENCH_*` in the environment shorten them for tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    /// The whole benchmark: when it is over, the best result so far counts.
    pub budget: Duration,
    /// Prompt length of a measured run (at most half the context).
    pub prompt_tokens: usize,
    /// New tokens of a measured run.
    pub gen_tokens: u32,
    /// Context steps; None: from the model's trained context.
    pub stages: Option<Vec<u64>>,
    /// Longest wait for one candidate to load.
    pub load_timeout: Duration,
    /// A slow PC may get shorter runs after the first warmup (`rp_core::bench::run_sizes`); not
    /// when the environment sets the sizes.
    pub fit_sizes: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params::defaults(true)
    }
}

impl Params {
    /// Budget 300 s; runs of 2,048 prompt tokens and 64 new ones with a GPU, 512 and 32 on the CPU
    /// (which reads a prompt 10–100 times slower).
    pub fn defaults(gpu_on: bool) -> Params {
        let (prompt_tokens, gen_tokens) = if gpu_on { (2048, 64) } else { (512, 32) };
        Params { budget: Duration::from_secs(300), prompt_tokens, gen_tokens, stages: None, load_timeout: Duration::from_secs(150), fit_sizes: true }
    }

    /// The defaults, or `RP_BENCH_BUDGET_SECS`, `RP_BENCH_PROMPT_TOKENS`, `RP_BENCH_GEN_TOKENS`,
    /// `RP_BENCH_CTX` (comma-separated context steps).
    pub fn from_env(gpu_on: bool) -> Params {
        Params::from_vars(gpu_on, |k| std::env::var(k).ok())
    }

    fn from_vars(gpu_on: bool, var: impl Fn(&str) -> Option<String>) -> Params {
        let num = |k: &str| var(k).and_then(|v| v.trim().parse::<u64>().ok()).filter(|&n| n > 0);
        let d = Params::defaults(gpu_on);
        let (prompt, gen) = (num("RP_BENCH_PROMPT_TOKENS"), num("RP_BENCH_GEN_TOKENS"));
        Params {
            budget: num("RP_BENCH_BUDGET_SECS").map_or(d.budget, Duration::from_secs),
            prompt_tokens: prompt.map_or(d.prompt_tokens, |n| n as usize),
            gen_tokens: gen.map_or(d.gen_tokens, |n| n as u32),
            stages: var("RP_BENCH_CTX").map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).filter(|&c: &u64| c >= 512).collect::<Vec<u64>>()).filter(|v| !v.is_empty()),
            load_timeout: d.load_timeout,
            fit_sizes: prompt.is_none() && gen.is_none(),
        }
    }
}

/// How a candidate is loaded, for the estimate: without a GPU every layer on the CPU, whatever
/// `-ngl` says (a CPU build ignores it).
pub fn load(args: &[String], gpu_on: bool, ctx: u64, kv: &str, n_cpu_moe: u64) -> Load {
    let gpu_layers = if gpu_on { bench::gpu_layers(args) } else { Some(0) };
    Load { ctx, cache_k: kv.into(), cache_v: kv.into(), gpu_layers, n_cpu_moe }
}

/// A candidate's part in RAM by the estimate; without a GPU with the compute buffers, which are in
/// RAM then too (the floor and the checks before and while it runs follow from it).
pub fn cpu_bytes(info: &GgufInfo, args: &[String], gpu_on: bool, c: &Candidate) -> u64 {
    let buffers = if gpu_on { 0 } else { vram::RESERVE_BYTES };
    vram::estimate(info, &load(args, gpu_on, c.ctx, &c.kv, c.n_cpu_moe)).map_or(0, |e| e.cpu_bytes + buffers)
}

/// The search for this PC: steps from the environment as they are; else the model's steps fitted to
/// the memory read with the active model stopped (`Plan::fit_to`) and, without a GPU, spread over
/// three steps (`Plan::spread_out`). Pure: the PC comes in as `gpu_on` and the two readings.
pub fn plan_for(info: &GgufInfo, args: &[String], gpu_on: bool, stages: Option<Vec<u64>>, free_vram: Option<u64>, ram_available: Option<u64>) -> Plan {
    if let Some(stages) = stages {
        return Plan::for_model(info, gpu_on, stages);
    }
    let room = Room { gpu: gpu_on, free_vram, ram_available };
    let plan = Plan::for_model(info, gpu_on, bench::stages(info.context_length)).fit_to(info, &load(args, gpu_on, 0, bench::KV_TYPES[0], 0), &room);
    if gpu_on {
        plan
    } else {
        plan.spread_out()
    }
}

/// The sizes of the measured runs: from the params, and once (after the first warmup) shortened
/// for a slow PC.
struct Sizes {
    prompt: usize,
    gen: u32,
    /// Set by the environment or already decided.
    fixed: bool,
}

/// What the routes see of the running benchmark.
#[derive(Default)]
pub struct Status {
    cancel: AtomicBool,
    /// The newest progress event of the running (or last) benchmark.
    last: Mutex<Option<Value>>,
}

impl Status {
    pub fn last(&self) -> Option<Value> {
        self.last.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// A new benchmark starts: not cancelled, no progress yet.
    pub fn begin(&self) {
        self.cancel.store(false, Ordering::Relaxed);
        *self.last.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// One benchmark of one profile. The caller made `hub.bench_begin` succeed.
pub struct Job {
    pub hub: Arc<Hub>,
    pub engine: EngineHandle,
    pub status: Arc<Status>,
    pub data_dir: PathBuf,
    pub profile_id: i64,
    /// What the profile starts (its own arguments are the base of every candidate).
    pub spec: LaunchSpec,
    pub info: Arc<GgufInfo>,
    pub params: Params,
    /// The GPU the model server computes on (`system::computing_gpu`); None: a CPU run.
    pub gpu: Option<&'static hw::Gpu>,
    /// Model file and engine of the profile (`Profile::bench_stamp`): the result is only valid
    /// for them.
    pub measured_with: Value,
}

/// Why a candidate stopped early (besides an outcome for its row, and the limit it hit).
enum Stop {
    Row(Outcome, Option<bench::Limit>),
    Cancelled,
}

const MIB: u64 = 1024 * 1024;

impl Job {
    fn event(&self, stage: &str, rows: &[Row], plan: &Plan, started: Instant, candidate: Option<&Candidate>, extra: Option<(&str, Value)>) {
        let mut ev = json!({
            "type": "bench",
            "profile_id": self.profile_id,
            "stage": stage,
            "done": rows.len(),
            "total": plan.pairs().max(rows.len() + usize::from(candidate.is_some())),
            "elapsed_s": started.elapsed().as_secs_f64().round(),
            "budget_s": self.params.budget.as_secs(),
            "candidate": candidate,
            "rows": rows,
        });
        if let Some((k, v)) = extra {
            ev[k] = v;
        }
        *self.status.last.lock().unwrap_or_else(|p| p.into_inner()) = Some(ev.clone());
        self.hub.notify(&ev);
    }

    /// The model server is stopped (or never came up): wait until its process and port are gone.
    async fn stop_server(&self) {
        self.hub.bench_set(None);
        let deadline = Instant::now() + Duration::from_secs(20);
        while (self.engine.pid().is_some() || crate::engine::port_in_use(self.spec.port)) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Available RAM (free + standby), commit left and, with `pid`, that model server's shared GPU
    /// memory now (None: not known).
    async fn memory(pid: Option<u32>) -> Memory {
        tokio::task::spawn_blocking(move || {
            let sys = hw::system().ok();
            let shared = pid.and_then(|pid| main_gpu().and_then(|g| hw::vram_usage().ok()?.process(pid, g.luid).map(|p| p.shared_bytes)));
            Memory { available: sys.as_ref().map(|s| s.ram_available_bytes), commit_free: sys.as_ref().map(hw::SystemInfo::commit_free_bytes), shared }
        })
        .await
        .unwrap_or_default()
    }

    /// After a server stopped: the memory once the available RAM no longer rises (at most ~5 s),
    /// so the next candidate is not judged by what the last one still held.
    async fn settled_memory() -> Memory {
        let mut last = Self::memory(None).await;
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let now = Self::memory(None).await;
            let rising = matches!((last.available, now.available), (Some(a), Some(b)) if b > a + 128 * MIB);
            last = now;
            if !rising {
                break;
            }
        }
        last
    }

    /// Overflow of the running server, MiB: its shared GPU memory minus the pinned host buffers its
    /// log names (`cpu_bytes`, the estimate of the part on the CPU, when the log names none). None:
    /// no GPU counters for it (CPU build, not Windows).
    async fn overflow(&self, pid: u32, cpu_bytes: u64) -> Option<i64> {
        let log = tokio::fs::read_to_string(self.data_dir.join("llama-server.log")).await.unwrap_or_default();
        let logged = bench::host_buffer_mib(&log);
        let host = if logged > 0.0 { logged } else { cpu_bytes as f64 / (1024.0 * 1024.0) };
        Self::memory(Some(pid)).await.shared.map(|shared| bench::overflow_mib(shared, host))
    }

    /// Returns when the candidate (`cpu_bytes` on the CPU) must stop: a guard tripped, its server
    /// died, the time is up or the user cancelled. Polled every half second next to whatever the
    /// candidate waits for.
    async fn watch(&self, pid: u32, cpu_bytes: u64, deadline: Instant) -> Stop {
        loop {
            if self.status.cancelled() {
                return Stop::Cancelled;
            }
            if Instant::now() >= deadline {
                return Stop::Row(Outcome::Time, None);
            }
            if self.engine.pid() != Some(pid) {
                return Stop::Row(Outcome::Failed, None);
            }
            if let Some((o, limit)) = bench::guard(&Self::memory(Some(pid)).await, cpu_bytes) {
                return Stop::Row(o, Some(limit));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// One `/completion` with the filler prompt: (prompt tokens/s, generated tokens/s).
    async fn completion(&self, prompt: &str, n_predict: u32) -> Result<(f64, f64), String> {
        let body = json!({
            "prompt": prompt, "n_predict": n_predict, "cache_prompt": false, "temperature": 0.0,
            "ignore_eos": true, "stream": false,
        });
        let res = reqwest::Client::new()
            .post(format!("{}/completion", self.hub.llama_url()))
            .json(&body)
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = res.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
        let t = &v["timings"];
        match (t["prompt_per_second"].as_f64(), t["predicted_per_second"].as_f64()) {
            (Some(pp), Some(tg)) => Ok((pp, tg)),
            _ => Err("no timings in the answer".into()),
        }
    }

    /// Runs one candidate with the memory read before (`mem`); extra runs for a high spread only
    /// until `share` (its part of the budget) is used up. After the first warmup of the benchmark
    /// the run sizes are decided (`sizes`). None: cancelled.
    async fn run_candidate(&self, c: &Candidate, mem: &Memory, deadline: Instant, share: Duration, sizes: &mut Sizes) -> Option<Row> {
        let mut row = Row::new(c, Outcome::Failed);
        let cpu = cpu_bytes(&self.info, &self.spec.args, self.gpu.is_some(), c);
        if let Some(limit) = bench::skip_before_start(cpu, mem) {
            row.outcome = Outcome::Skipped;
            row.limit = Some(limit);
            return Some(row);
        }
        let spec = LaunchSpec { args: bench::run_args(&self.spec.args, c), ..self.spec.clone() };
        self.hub.bench_set(Some(spec));
        // The supervisor starts it at its next check (woken now).
        let t0 = Instant::now();
        let pid = loop {
            if let Some(pid) = self.engine.pid() {
                break pid;
            }
            if self.status.cancelled() {
                return None;
            }
            if t0.elapsed() > Duration::from_secs(15) {
                return Some(row); // the start failed (the activity log says why)
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let load_deadline = deadline.min(t0 + self.params.load_timeout);
        let up = async {
            while self.hub.engine_status().await != "ready" {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        };
        let stopped = tokio::select! {
            _ = up => None,
            s = self.watch(pid, cpu, load_deadline) => Some(s),
        };
        match stopped {
            Some(Stop::Cancelled) => return None,
            Some(Stop::Row(o, limit)) => {
                // Out of time while loading counts as "Zeit"; a load that never ended as failed.
                row.outcome = if o == Outcome::Time && Instant::now() < deadline { Outcome::Failed } else { o };
                row.limit = limit;
                self.hub.bench_set(None);
                return Some(row);
            }
            None => {}
        }
        row.load_s = Some((t0.elapsed().as_secs_f64() * 10.0).round() / 10.0);

        // Overflow: the server's shared GPU memory minus its pinned host buffers.
        tokio::time::sleep(Duration::from_millis(500)).await;
        row.overflow_mib = self.overflow(pid, cpu).await;
        if row.overflow_mib.is_some_and(|o| o > bench::OVERFLOW_LIMIT_MIB) {
            row.outcome = Outcome::Overflow;
            return Some(row);
        }

        // A warmup of the same size (discarded: the first run after a load is slower), then at least
        // MIN_RUNS measured runs, more while they spread too much and the candidate's share of the
        // budget holds another one.
        let filler = |tokens: usize| bench::filler(tokens.min(c.ctx as usize / 2));
        let mut prompt = filler(sizes.prompt);
        let share_end = deadline.min(t0 + share);
        let mut runs: Vec<(f64, f64)> = Vec::new();
        let mut warm = false;
        let mut last = Duration::ZERO;
        loop {
            let tg: Vec<f64> = runs.iter().map(|r| r.1).collect();
            if warm && (!bench::more_runs(&tg) || (tg.len() >= bench::MIN_RUNS && Instant::now() + last > share_end)) {
                break;
            }
            let t = Instant::now();
            let measured = tokio::select! {
                r = self.completion(&prompt, sizes.gen) => Ok(r),
                s = self.watch(pid, cpu, deadline) => Err(s),
            };
            match measured {
                Ok(Ok(r)) if warm => runs.push(r),
                Ok(Ok(_)) => {
                    warm = true;
                    if !sizes.fixed {
                        // The first warmup shows how fast this PC is: a slow one gets shorter runs.
                        let left = deadline.saturating_duration_since(Instant::now()).as_secs_f64();
                        let load_s = row.load_s.unwrap_or(0.0);
                        (sizes.prompt, sizes.gen) = bench::run_sizes(sizes.prompt, sizes.gen, t.elapsed().as_secs_f64(), load_s, left);
                        sizes.fixed = true;
                        prompt = filler(sizes.prompt);
                        last = Duration::ZERO;
                        continue;
                    }
                }
                Ok(Err(e)) => {
                    self.hub.log().write(Entry::new(Level::Warn, "engine", "bench", format!("benchmark run failed: {e}")).data(json!({ "profile_id": self.profile_id })));
                    return Some(row);
                }
                Err(Stop::Cancelled) => return None,
                Err(Stop::Row(o, limit)) => {
                    row.outcome = o;
                    row.limit = limit;
                    self.hub.bench_set(None);
                    return Some(row);
                }
            }
            last = t.elapsed();
        }
        row.set_runs(&runs);
        // The KV cache is in use now: read the overflow again, the larger value counts.
        let after = self.overflow(pid, cpu).await;
        row.overflow_mib = row.overflow_mib.max(after);
        row.outcome = if row.overflow_mib.is_some_and(|o| o > bench::OVERFLOW_LIMIT_MIB) { Outcome::Overflow } else { Outcome::Ok };
        Some(row)
    }
}

/// The whole benchmark. Ends with the active profile running again (as before) and, if anything
/// fit, the result stored for the profile.
pub async fn run(job: Job) {
    let started = Instant::now();
    let deadline = started + job.params.budget;
    let gpu_on = job.gpu.is_some();
    // The memory a GPU run may use; a CPU run does not depend on the VRAM (nor its conditions).
    let read_vram = || async move {
        if gpu_on {
            tokio::task::spawn_blocking(|| memory_now(None).1).await.unwrap_or(None)
        } else {
            None
        }
    };
    let plan = match job.params.stages.clone() {
        Some(stages) => plan_for(&job.info, &job.spec.args, gpu_on, Some(stages), None, None),
        None => {
            // Which steps can fit at all is read with the active model stopped: a small GPU starts
            // lower, and what surely does not fit is not started.
            job.stop_server().await;
            let mem = Job::settled_memory().await;
            plan_for(&job.info, &job.spec.args, gpu_on, None, read_vram().await, mem.available)
        }
    };
    job.hub.log().write(Entry::new(Level::Info, "engine", "bench", "benchmark started").data(json!({
        "profile_id": job.profile_id, "stages": plan.stages, "moe_layers": plan.moe_layers, "budget_s": job.params.budget.as_secs(), "gpu": gpu_on,
    })));
    let mut rows: Vec<Row> = Vec::new();
    let mut costs: Vec<f64> = Vec::new();
    let mut conditions: Option<Conditions> = None;
    let mut sizes = Sizes { prompt: job.params.prompt_tokens, gen: job.params.gen_tokens, fixed: !job.params.fit_sizes };
    // Extra runs of a candidate whose speeds spread stay inside its part of the budget.
    let share = job.params.budget / plan.pairs().max(1) as u32;
    let mut end = "done";
    let mut time_up = false;
    loop {
        if job.status.cancelled() {
            end = "cancelled";
            break;
        }
        job.stop_server().await;
        let pid_gone = job.engine.pid().is_none();
        let mem = Job::settled_memory().await;
        let free_vram = read_vram().await;
        // The conditions of the whole run: the memory once the active model is stopped.
        let mib = |b: Option<u64>| b.map(|b| b / MIB);
        conditions.get_or_insert(Conditions { ram_available_mib: mib(mem.available), vram_free_mib: mib(free_vram), commit_free_mib: mib(mem.commit_free) });
        let room = Room { gpu: gpu_on, free_vram, ram_available: mem.available };
        let base = load(&job.spec.args, gpu_on, 0, bench::KV_TYPES[0], 0);
        let start = |ctx: u64, kv: &str| room.start(&job.info, &base, plan.moe_layers, ctx, kv);
        let Some(c) = bench::next(&plan, &rows, start) else { break };
        if !pid_gone {
            break;
        }
        // Not enough time left for one more (by the mean of the ones so far): stop with what is there.
        if !bench::time_for_another(started.elapsed().as_secs_f64(), &costs, job.params.budget.as_secs_f64()) {
            time_up = true;
            break;
        }
        job.event("loading", &rows, &plan, started, Some(&c), None);
        let t = Instant::now();
        let Some(row) = job.run_candidate(&c, &mem, deadline, share, &mut sizes).await else {
            end = "cancelled";
            break;
        };
        costs.push(t.elapsed().as_secs_f64());
        job.hub.log().write(Entry::new(Level::Info, "engine", "bench", "benchmark candidate measured").data(json!({ "profile_id": job.profile_id, "row": row })));
        let timed_out = row.outcome == Outcome::Time;
        rows.push(row);
        job.event("measuring", &rows, &plan, started, None, None);
        if timed_out {
            time_up = true;
            break;
        }
    }
    // A cancel that came while a step ended the loop another way (no candidate left, the server
    // still up) is still a cancel, not a failed run.
    if job.status.cancelled() {
        end = "cancelled";
    }
    job.stop_server().await;
    job.hub.bench_end();
    let duration = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    let picks = bench::recommend(&rows);
    // Steps of the plan the time did not reach (none tried with any KV type).
    let unmeasured: Vec<u64> = if time_up { plan.stages.iter().copied().filter(|&s| !rows.iter().any(|r| r.ctx == s)).collect() } else { Vec::new() };
    let result = json!({
        "profile_id": job.profile_id,
        "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        "duration_s": duration,
        "rows": rows,
        "picks": picks,
        "moe": plan.moe_layers.is_some(),
        "stages": plan.stages,
        "unmeasured_stages": unmeasured,
        "prompt_tokens": sizes.prompt,
        "gen_tokens": sizes.gen,
        "gpu": job.gpu.map(|g| g.name.clone()),
        "conditions": conditions,
        "measured_with": job.measured_with,
    });
    if end == "done" && !picks.is_empty() {
        if let Err(e) = job.hub.store(|s| s.set_bench_result(job.profile_id, &result)) {
            job.hub.log().write(Entry::new(Level::Error, "engine", "bench", format!("benchmark result not stored: {e}")));
        }
    }
    let stage = if end == "done" && picks.is_empty() { "failed" } else { end };
    job.hub.log().write(Entry::new(Level::Info, "engine", "bench", format!("benchmark {stage}")).data(json!({
        "profile_id": job.profile_id, "duration_s": duration, "candidates": rows.len(), "picks": picks,
    })));
    job.event(stage, &rows, &plan, started, None, Some(("result", result)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{computing_gpu, largest_gpu};
    use rp_core::gguf::PerLayer;

    const GIB: u64 = 1024 * MIB;

    /// A dense model of 7 GiB (32 layers, 8 KV heads × 128), trained for 128k.
    fn dense7() -> GgufInfo {
        GgufInfo {
            version: 3,
            name: Some("Dense".into()),
            architecture: Some("llama".into()),
            file_type: Some(15),
            file_type_name: "Q4_K_M".into(),
            size_label: None,
            context_length: Some(131072),
            block_count: Some(32),
            embedding_length: Some(4096),
            head_count: Some(PerLayer::One(32)),
            head_count_kv: Some(PerLayer::One(8)),
            key_length: Some(128),
            value_length: Some(128),
            key_length_swa: None,
            value_length_swa: None,
            sliding_window: None,
            sliding_window_pattern: None,
            shared_kv_layers: None,
            full_attention_interval: None,
            recurrent_state_bytes: None,
            expert_count: None,
            expert_used_count: None,
            has_chat_template: true,
            chat_template_family: None,
            tensor_count: 0,
            parameter_count: 12_000_000_000,
            weight_bytes: 7 * GIB,
            expert_weight_bytes: 0,
            expert_parameter_count: 0,
            file_bytes: 7 * GIB,
            split: None,
            is_projector: false,
        }
    }

    fn gpu(name: &str, vendor: hw::Vendor, dedicated: u64) -> hw::Gpu {
        hw::Gpu { name: name.into(), vendor, vendor_id: 0, device_id: 0, luid: hw::Luid { high: 0, low: 1 }, dedicated_bytes: dedicated, shared_bytes: 16 * GIB, driver: None }
    }

    fn mem(available: u64) -> Memory {
        Memory { available: Some(available), commit_free: Some(60 * GIB), shared: None }
    }

    /// H2: a PC with only integrated graphics (128 MiB "dedicated") or with the CPU build of the
    /// engine is planned as a CPU run: every step's RAM part is checked against the available RAM,
    /// and a candidate's part in RAM is the whole model (not 0), so its floor is 3 GiB.
    #[test]
    fn a_pc_without_a_computing_gpu_is_planned_as_a_cpu_run() {
        let info = dense7();
        let none: Vec<String> = Vec::new();
        let igpu = largest_gpu(vec![gpu("Intel(R) UHD Graphics 770", hw::Vendor::Intel, 128 * MIB)]);
        assert!(computing_gpu(igpu.as_ref(), "vulkan", &none).is_none(), "integrated graphics");
        let dgpu = largest_gpu(vec![gpu("AMD Radeon RX 7900 XTX", hw::Vendor::Amd, 24 * GIB)]);
        assert!(computing_gpu(dgpu.as_ref(), "cpu", &none).is_none(), "CPU build without -ngl 0");
        let gpu_on = false;

        // 4 GiB available: a 7 GiB model fits nowhere (the old view, a GPU without counters, planned 8k–128k).
        assert_eq!(plan_for(&info, &none, gpu_on, None, None, Some(4 * GIB)).stages, Vec::<u64>::new());
        // 16 GiB available: 128k q8_0 (7 + 8.5 GiB KV + buffers + floor) is left out; three steps up to 64k.
        let p = plan_for(&info, &none, gpu_on, None, None, Some(16 * GIB));
        assert_eq!((p.stages.clone(), p.moe_layers), (vec![8192, 32768, 65536], None));
        assert_eq!(plan_for(&info, &none, gpu_on, None, None, Some(32 * GIB)).stages, vec![8192, 32768, 131072]);

        // The candidate's part in RAM: weights + KV + compute buffers (was 0 before).
        let c = Candidate { ctx: 8192, kv: "q8_0".into(), n_cpu_moe: 0 };
        let cpu = cpu_bytes(&info, &none, gpu_on, &c);
        assert!(cpu > 8 * GIB, "{} MiB", cpu / MIB);
        assert_eq!(bench::ram_floor(cpu), 3 * GIB);
        let stop = bench::guard(&mem(2560 * MIB), cpu);
        assert_eq!(stop.map(|s| (s.0, s.1.needed_mib)), Some((Outcome::Ram, 3072)), "stopped below 3 GiB available");
        assert_eq!(bench::skip_before_start(cpu, &mem(10 * GIB)).map(|l| l.what), Some(bench::LimitKind::Ram), "not started: 8.7 + 3 GiB > 10");
        assert_eq!(bench::skip_before_start(cpu, &mem(16 * GIB)), None);
        // The same with a GPU build and -ngl 0.
        assert_eq!(cpu_bytes(&info, &["-ngl".to_string(), "0".to_string()], false, &c), cpu);
    }

    /// A dedicated GPU with a GPU build plans as before: the steps fitted to the free VRAM with every
    /// layer on the GPU, all of them tried, nothing of the model in RAM.
    #[test]
    fn a_dedicated_gpu_is_planned_as_before() {
        let info = dense7();
        let args: Vec<String> = ["-ngl", "999", "-fa", "on"].iter().map(|s| s.to_string()).collect();
        let room = Room { gpu: true, free_vram: Some(20 * GIB), ram_available: Some(16 * GIB) };
        let old = Plan::for_model(&info, true, bench::stages(info.context_length)).fit_to(&info, &Load { ctx: 0, cache_k: "q8_0".into(), cache_v: "q8_0".into(), gpu_layers: Some(999), n_cpu_moe: 0 }, &room);
        let new = plan_for(&info, &args, true, None, Some(20 * GIB), Some(16 * GIB));
        assert_eq!(new, old);
        assert_eq!(new.stages, vec![8192, 16384, 32768, 65536, 131072]);
        assert_eq!(cpu_bytes(&info, &args, true, &Candidate { ctx: 8192, kv: "q8_0".into(), n_cpu_moe: 0 }), 0);
        // Steps from the environment stay as they are.
        assert_eq!(plan_for(&info, &args, false, Some(vec![2048, 4096]), None, Some(GIB)).stages, vec![2048, 4096]);
    }

    /// M8: a CPU run measures shorter runs; `RP_BENCH_*` win over every default.
    #[test]
    fn run_sizes_follow_the_pc_and_the_environment() {
        let none = |_: &str| None;
        let cpu = Params::from_vars(false, none);
        assert_eq!((cpu.prompt_tokens, cpu.gen_tokens, cpu.budget.as_secs(), cpu.fit_sizes), (512, 32, 300, true));
        let gpu = Params::from_vars(true, none);
        assert_eq!((gpu.prompt_tokens, gpu.gen_tokens, gpu.budget.as_secs(), gpu.fit_sizes), (2048, 64, 300, true));
        assert_eq!(gpu, Params::default());
        let env = |k: &str| match k {
            "RP_BENCH_CTX" => Some("2048,4096".to_string()),
            "RP_BENCH_PROMPT_TOKENS" => Some("256".to_string()),
            "RP_BENCH_GEN_TOKENS" => Some("16".to_string()),
            "RP_BENCH_BUDGET_SECS" => Some("120".to_string()),
            _ => None,
        };
        let e2e = Params::from_vars(false, env);
        assert_eq!((e2e.prompt_tokens, e2e.gen_tokens, e2e.budget.as_secs(), e2e.stages, e2e.fit_sizes), (256, 16, 120, Some(vec![2048, 4096]), false));
    }
}
