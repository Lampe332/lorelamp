//! Benchmark / auto-tune of a model profile, the pure part: which start options to try (context
//! steps × KV cache type × MoE layers on the CPU), when a run does not fit or must stop, and which
//! result is best for each purpose. The server starts the candidates and measures them
//! (`rp_server::bench`); see `docs/benchmarks.md` (M0) for where the rules come from.

use serde::{Deserialize, Serialize};

use crate::gguf::GgufInfo;
use crate::llama_args::{is_flag, without_flags};
use crate::vram::{self, Load};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// More overflow into shared memory than this and a candidate does not fit.
pub const OVERFLOW_LIMIT_MIB: i64 = 300;
/// A stage stops when available system RAM (free + standby, which Windows hands out at once) falls
/// below the candidate's floor: this much for a candidate whose part on the CPU is large ...
pub const RAM_FLOOR_BYTES: u64 = 3 * GIB;
/// ... and this much for one that is (almost) all on the GPU: what it reads while loading is file
/// cache Windows can drop, not memory it keeps ([`ram_floor`]).
pub const RAM_FLOOR_MIN_BYTES: u64 = GIB;
/// ... or less than this is left of the commit limit (RAM + page file): there allocations fail
/// system-wide and the PC freezes, whatever the candidate needs ...
pub const COMMIT_FLOOR_BYTES: u64 = 2 * GIB;
/// ... or the model server's shared GPU memory (pinned host buffers + overflow) passes the
/// candidate's own part on the CPU (its pinned buffers, booked as shared) by this much. A small GPU
/// with the experts of a MoE model in RAM pins far more than 6 GB on purpose; what counts is what
/// comes on top.
pub const SHARED_LIMIT_BYTES: u64 = 6 * GIB;
/// Measured runs per candidate (after one warmup) ...
pub const MIN_RUNS: usize = 3;
/// ... and at most this many while the spread stays above [`SPREAD_LIMIT`].
pub const MAX_RUNS: usize = 6;
/// Spread `(max − min) / median` of the generation speed above which a result counts as uncertain.
pub const SPREAD_LIMIT: f64 = 0.10;
/// Two results were measured under different conditions when the available RAM at the start
/// differs by more than this ...
pub const SAME_RAM_MIB: u64 = 3072;
/// ... or the free VRAM by more than this (a browser or a game holds about that much).
pub const SAME_VRAM_MIB: u64 = 1024;
/// Results kept per profile, one per kind of conditions, newest first.
pub const KEEP_RESULTS: usize = 4;
/// "Tempo" looks at contexts from this size on (if any was measured).
pub const SPEED_MIN_CTX: u64 = 8192;
/// "Tempo": rows this close to the fastest count as equally fast (measuring noise), at least; the
/// spread of the fastest row widens it up to [`SPREAD_LIMIT`].
pub const SPEED_NOISE: f64 = 0.05;
/// "Ausgewogen": the largest context whose tg is at least this share of the best.
pub const BALANCED_SHARE: f64 = 0.85;
/// "Kontext": the largest context that still writes at least this many tokens/s.
pub const CONTEXT_MIN_TG: f64 = 15.0;
/// Starts per context step and KV type at most (MoE: first guess plus corrections).
pub const MAX_TRIES: usize = 3;
/// The KV cache types tried, in this order (q8_0 needs half the memory of f16).
pub const KV_TYPES: [&str; 2] = ["q8_0", "f16"];
/// Context steps of the search; the trained context caps them.
pub const STAGES: [u64; 5] = [8192, 16384, 32768, 65536, 131072];
/// Where the search starts when the first step does not fit entirely into the device memory (a
/// small GPU, or a small RAM without a GPU) ([`Plan::fit_to`]).
pub const SMALL_STAGE: u64 = 4096;
/// A step whose estimate is more than this over the free VRAM, even with every expert layer on the
/// CPU, surely overflows (the estimate is ~0.5 GB low with HIP, `docs/benchmarks.md` M0.2): it is
/// left out. Closer than this, the run decides.
pub const SURE_MARGIN_BYTES: u64 = 512 * MIB;

/// Without a GPU the search measures at most this many context steps ([`Plan::spread_out`]).
pub const CPU_STAGES: usize = 3;
/// A slow PC gets shorter runs, so that at least this many candidates fit the budget ([`run_sizes`]) ...
pub const MIN_CANDIDATES: usize = 3;
/// ... but never shorter than this prompt ...
pub const MIN_PROMPT_TOKENS: usize = 128;
/// ... and this many new tokens.
pub const MIN_GEN_TOKENS: u32 = 8;
/// Stopping a server and waiting until its memory is back, between two candidates (≤ 5 s).
pub const SETTLE_SECS: f64 = 5.0;

/// Start options the benchmark sets; every other load argument of the profile stays as it is.
pub const TUNED_FLAGS: [&str; 8] = ["-c", "--ctx-size", "-ctk", "--cache-type-k", "-ctv", "--cache-type-v", "--n-cpu-moe", "-ncmoe"];

/// One combination of start options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub ctx: u64,
    /// `-ctk` and `-ctv`.
    pub kv: String,
    /// `--n-cpu-moe` (0: all experts on the GPU, or not a MoE model).
    pub n_cpu_moe: u64,
}

/// How a candidate ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Measured, no overflow worth speaking of.
    Ok,
    /// Loaded, but more than [`OVERFLOW_LIMIT_MIB`] went to shared memory: not measured further.
    Overflow,
    /// Stopped: available RAM fell below the candidate's floor or the commit limit was nearly
    /// reached ([`Row::limit`] says which, with the numbers).
    Ram,
    /// Stopped: the server's shared GPU memory passed [`SHARED_LIMIT_BYTES`].
    Shared,
    /// Not started: by the estimate the part on the CPU does not fit into the available RAM or the
    /// commit left ([`Row::limit`]).
    Skipped,
    /// Did not come up (crash, start error, load timeout) or the measurement failed.
    Failed,
    /// The time budget ran out while it ran.
    Time,
}

impl Outcome {
    /// Larger contexts of the same KV type are not tried after this.
    fn stops_stage(self) -> bool {
        matches!(self, Outcome::Ram | Outcome::Shared | Outcome::Skipped | Outcome::Failed | Outcome::Time)
    }

    /// Ended for want of memory (a larger KV type would need more, not less).
    fn memory(self) -> bool {
        matches!(self, Outcome::Overflow | Outcome::Ram | Outcome::Shared | Outcome::Skipped)
    }
}

/// One line of the result table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub ctx: u64,
    pub kv: String,
    pub n_cpu_moe: u64,
    /// Prompt reading, tokens/s (median of the measured runs).
    pub pp: Option<f64>,
    /// Generation, tokens/s (median).
    pub tg: Option<f64>,
    /// Process shared GPU memory minus pinned host buffers, MiB; None: not known (no GPU counters).
    pub overflow_mib: Option<i64>,
    pub outcome: Outcome,
    /// Seconds until the server answered as ready.
    pub load_s: Option<f64>,
    /// Every measured run (the warmup is not among them), tokens/s. Empty in results stored before.
    #[serde(default)]
    pub pp_runs: Vec<f64>,
    #[serde(default)]
    pub tg_runs: Vec<f64>,
    /// `(max − min) / median` of `tg_runs` (0.04 = 4 %).
    #[serde(default)]
    pub spread: Option<f64>,
    /// The spread stayed above [`SPREAD_LIMIT`] (no time for more runs): the speed is not certain.
    #[serde(default)]
    pub uncertain: bool,
    /// Why it was stopped or left out (`ram`, `skipped`, `shared`), with the numbers.
    #[serde(default)]
    pub limit: Option<Limit>,
}

/// Which limit a candidate hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitKind {
    /// Available RAM (free + standby).
    Ram,
    /// What is left of the commit limit (RAM + page file).
    Commit,
    /// The model server's shared GPU memory (its own pinned part + [`SHARED_LIMIT_BYTES`]).
    Shared,
}

/// A limit with the numbers that hit it: `needed_mib` is more than `available_mib`. For `shared`,
/// `available_mib` is the limit and `needed_mib` what the server holds (or would pin).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limit {
    pub what: LimitKind,
    pub available_mib: u64,
    pub needed_mib: u64,
}

impl Limit {
    fn new(what: LimitKind, available: u64, needed: u64) -> Limit {
        Limit { what, available_mib: available / MIB, needed_mib: needed.div_ceil(MIB) }
    }
}

/// One reading of the memory a candidate depends on (None: not known, e.g. not Windows).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    /// Available RAM: free + standby (`ullAvailPhys`, `PhysicalAvailable`).
    pub available: Option<u64>,
    /// Commit limit minus commit in use.
    pub commit_free: Option<u64>,
    /// The model server's shared GPU memory.
    pub shared: Option<u64>,
}

impl Row {
    pub fn new(c: &Candidate, outcome: Outcome) -> Row {
        Row {
            ctx: c.ctx,
            kv: c.kv.clone(),
            n_cpu_moe: c.n_cpu_moe,
            pp: None,
            tg: None,
            overflow_mib: None,
            outcome,
            load_s: None,
            pp_runs: Vec::new(),
            tg_runs: Vec::new(),
            spread: None,
            uncertain: false,
            limit: None,
        }
    }

    /// Takes the measured runs `(pp, tg)`: medians, spread, uncertain.
    pub fn set_runs(&mut self, runs: &[(f64, f64)]) {
        let round = |v: f64| (v * 10.0).round() / 10.0;
        let pp: Vec<f64> = runs.iter().map(|r| r.0).collect();
        let tg: Vec<f64> = runs.iter().map(|r| r.1).collect();
        self.pp = median(&pp).map(round);
        self.tg = median(&tg).map(round);
        let s = spread(&tg);
        self.spread = s.map(|s| (s * 1000.0).round() / 1000.0);
        self.uncertain = s.is_some_and(|s| s > SPREAD_LIMIT);
        self.pp_runs = pp.into_iter().map(round).collect();
        self.tg_runs = tg.into_iter().map(round).collect();
    }

    /// Measured and without overflow: a usable setting.
    pub fn fits(&self) -> bool {
        self.outcome == Outcome::Ok && self.tg.is_some_and(|t| t > 0.0)
    }

    pub fn candidate(&self) -> Candidate {
        Candidate { ctx: self.ctx, kv: self.kv.clone(), n_cpu_moe: self.n_cpu_moe }
    }
}

/// What the search runs over.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub stages: Vec<u64>,
    pub kvs: Vec<String>,
    /// Number of layers when `--n-cpu-moe` is searched (a MoE model with layers on the GPU).
    pub moe_layers: Option<u64>,
    /// Expert weights per layer, MiB (how much one more CPU layer takes off the GPU).
    pub layer_mib: u64,
}

/// Context steps up to the trained context (which is the last one when it is smaller than 128k).
pub fn stages(ctx_train: Option<u64>) -> Vec<u64> {
    let top = ctx_train.filter(|&c| c >= 512).unwrap_or(131072).min(*STAGES.last().unwrap_or(&131072));
    let mut out: Vec<u64> = STAGES.iter().copied().filter(|&s| s < top).collect();
    out.push(top);
    out
}

impl Plan {
    /// The search for a model: every context step, both KV types, `--n-cpu-moe` for a MoE model
    /// whose layers are on the GPU.
    pub fn for_model(info: &GgufInfo, gpu: bool, stages: Vec<u64>) -> Plan {
        let layers = info.block_count.unwrap_or(0);
        let moe = gpu && info.is_moe() && layers > 0;
        Plan {
            stages,
            kvs: KV_TYPES.iter().map(|s| s.to_string()).collect(),
            moe_layers: moe.then_some(layers),
            layer_mib: if moe { (info.expert_weight_bytes / layers / MIB).max(1) } else { 1 },
        }
    }

    /// The search narrowed to this PC:
    /// - when the first step does not fit entirely into the device ([`Room::on_device`]), the
    ///   search starts at [`SMALL_STAGE`];
    /// - a step that surely does not fit with the first KV type ([`Room::start`] is None) is left out.
    ///
    /// No step left: nothing fits this PC, the benchmark has nothing to start.
    pub fn fit_to(mut self, info: &GgufInfo, base: &Load, room: &Room) -> Plan {
        let kv = self.kvs.first().cloned().unwrap_or_else(|| KV_TYPES[0].into());
        let layers = self.moe_layers;
        if let Some(&first) = self.stages.first() {
            if first > SMALL_STAGE && !room.on_device(info, base, first, &kv) {
                self.stages.insert(0, SMALL_STAGE);
            }
        }
        self.stages.retain(|&ctx| room.start(info, base, layers, ctx, &kv).is_some());
        self
    }

    /// Context steps × KV types: an upper bound for the progress display.
    pub fn pairs(&self) -> usize {
        self.stages.len() * self.kvs.len()
    }

    /// The search of a run without a GPU: the smallest, the middle and the largest of the steps
    /// that fit ([`CPU_STAGES`]). On the CPU the speed hardly changes with the context (what a step
    /// shows is whether its RAM part fits), and a slow CPU has time for about three candidates, so
    /// these three cover the whole range instead of the three smallest steps.
    pub fn spread_out(mut self) -> Plan {
        let n = self.stages.len();
        if n > CPU_STAGES {
            self.stages = vec![self.stages[0], self.stages[n / 2], self.stages[n - 1]];
        }
        self
    }
}

/// The run sizes `(prompt tokens, new tokens)` for the rest of a benchmark, decided once after the
/// first warmup: `warm_secs` for one run of `prompt`/`gen`, `load_secs` to load that candidate,
/// `left_secs` of the budget left after the warmup. Unchanged when its [`MIN_RUNS`] runs and
/// [`MIN_CANDIDATES`] − 1 more candidates (load, warmup, runs, [`SETTLE_SECS`]) fit into 90 % of
/// what is left; else both shortened in proportion (a run's time grows with both), down to
/// [`MIN_PROMPT_TOKENS`] / [`MIN_GEN_TOKENS`].
pub fn run_sizes(prompt: usize, gen: u32, warm_secs: f64, load_secs: f64, left_secs: f64) -> (usize, u32) {
    let more = (MIN_CANDIDATES - 1) as f64;
    // Runs still to come at the length of one: this candidate's, and warmup + runs of the others.
    let runs = MIN_RUNS as f64 + more * (1 + MIN_RUNS) as f64;
    let per_run = (0.9 * left_secs - more * (load_secs + SETTLE_SECS)) / runs;
    if warm_secs <= 0.0 || per_run >= warm_secs {
        return (prompt, gen);
    }
    let f = (per_run / warm_secs).max(0.0);
    let prompt = ((prompt as f64 * f) as usize).clamp(MIN_PROMPT_TOKENS.min(prompt), prompt);
    let gen = ((gen as f64 * f) as u32).clamp(MIN_GEN_TOKENS.min(gen), gen);
    (prompt, gen)
}

/// Time for one more candidate: what ran so far plus the mean cost of a candidate so far (the
/// stop and settle before it not counted) stays within the budget. Seconds.
pub fn time_for_another(elapsed: f64, costs: &[f64], budget: f64) -> bool {
    let typical = if costs.is_empty() { 0.0 } else { costs.iter().sum::<f64>() / costs.len() as f64 };
    elapsed + typical <= budget
}

/// The memory a benchmark may use, read with the active model stopped (None: not known, then
/// nothing is ruled out by it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Room {
    /// Layers go to a GPU (else everything runs on the CPU).
    pub gpu: bool,
    pub free_vram: Option<u64>,
    pub ram_available: Option<u64>,
}

impl Room {
    fn estimate(&self, info: &GgufInfo, base: &Load, ctx: u64, kv: &str, n: u64) -> Option<vram::Estimate> {
        let gpu_layers = if self.gpu { base.gpu_layers } else { Some(0) };
        vram::estimate(info, &Load { ctx, cache_k: kv.into(), cache_v: kv.into(), gpu_layers, n_cpu_moe: n })
    }

    /// Its part in RAM leaves the candidate's floor of the available RAM ([`skip_before_start`]);
    /// without a GPU the buffers are in RAM too.
    fn ram_ok(&self, e: &vram::Estimate) -> bool {
        let need = if self.gpu { e.cpu_bytes } else { e.cpu_bytes + vram::RESERVE_BYTES };
        self.ram_available.is_none_or(|r| need + ram_floor(need) <= r)
    }

    /// Fits entirely into the device at `ctx`: the GPU without experts on the CPU, or without a GPU
    /// the RAM.
    pub fn on_device(&self, info: &GgufInfo, base: &Load, ctx: u64, kv: &str) -> bool {
        self.estimate(info, base, ctx, kv, 0).is_some_and(|e| if self.gpu { e.cpu_bytes == 0 && self.free_vram.is_none_or(|f| e.gpu_bytes <= f) } else { self.ram_ok(&e) })
    }

    /// The `--n-cpu-moe` a candidate at `ctx`/`kv` starts with: the smallest (up to `moe_layers`;
    /// 0 for a dense model or without a GPU) whose VRAM part fits the free VRAM, else the smallest
    /// within [`SURE_MARGIN_BYTES`] over it. None: it surely does not fit, because no N gets within
    /// the margin or that N's part in RAM is too much for the available RAM.
    pub fn start(&self, info: &GgufInfo, base: &Load, moe_layers: Option<u64>, ctx: u64, kv: &str) -> Option<u64> {
        let layers = if self.gpu { moe_layers.unwrap_or(0) } else { 0 };
        let fewest = |margin: u64| {
            (0..=layers).find_map(|n| self.estimate(info, base, ctx, kv, n).filter(|e| !self.gpu || self.free_vram.is_none_or(|f| e.gpu_bytes <= f.saturating_add(margin))).map(|e| (n, e)))
        };
        let (n, e) = fewest(0).or_else(|| fewest(SURE_MARGIN_BYTES))?;
        self.ram_ok(&e).then_some(n)
    }
}

/// The next candidate to run, from what was measured so far; None: done. `start(ctx, kv)` is the
/// `--n-cpu-moe` a step starts with by the memory estimate against the memory free now, None when
/// it surely does not fit ([`Room::start`]): that step and the larger ones of the KV type are left out.
///
/// Per KV type the context steps go up. A MoE step starts at the estimate (corrected by what the
/// first measured step showed, never below the step before) and moves up by as many layers as the
/// overflow is large; on the first step that fits it also tries one layer less until that
/// overflows, which finds the real minimum. A step that cannot be made to fit, or that had to stop
/// (RAM, shared memory, crash, time), ends that KV type: the last good step stands. A later KV type
/// is only tried up to the largest step the first one reached (it needs more memory, not less);
/// when the first one reached none for want of memory, it is not tried at all, but when the first
/// one only failed to start (a crash, say q8_0 without flash attention), it is tried on its own.
pub fn next(plan: &Plan, rows: &[Row], start: impl Fn(u64, &str) -> Option<u64>) -> Option<Candidate> {
    let mut first_top: Option<u64> = None;
    let mut first_memory = false;
    for (ki, kv) in plan.kvs.iter().enumerate() {
        if ki > 0 && first_top.is_none() && first_memory {
            return None;
        }
        let mut floor_n = 0u64;
        let mut offset: Option<i64> = None;
        let mut top: Option<u64> = None;
        for &ctx in &plan.stages {
            if ki > 0 && first_top.is_some_and(|t| ctx > t) {
                break;
            }
            let here: Vec<&Row> = rows.iter().filter(|r| r.ctx == ctx && r.kv == *kv).collect();
            let tried = |n: u64| here.iter().any(|r| r.n_cpu_moe == n);
            if let Some(good) = here.iter().filter(|r| r.fits()).min_by_key(|r| r.n_cpu_moe) {
                let n = good.n_cpu_moe;
                // Calibration on the first good step: one layer less, until that does not fit.
                if plan.moe_layers.is_some() && offset.is_none() && n > floor_n && !tried(n - 1) && here.len() < MAX_TRIES + 2 {
                    return Some(Candidate { ctx, kv: kv.clone(), n_cpu_moe: n - 1 });
                }
                if offset.is_none() {
                    offset = Some(n as i64 - start(ctx, kv).unwrap_or(n) as i64);
                }
                floor_n = n;
                top = Some(ctx);
                continue;
            }
            if here.iter().any(|r| r.outcome.stops_stage()) {
                break;
            }
            let Some(layers) = plan.moe_layers else {
                if here.is_empty() && start(ctx, kv).is_some() {
                    return Some(Candidate { ctx, kv: kv.clone(), n_cpu_moe: 0 });
                }
                break; // a dense model that overflows (or surely would): a larger context does not fit either
            };
            if here.len() >= MAX_TRIES {
                break;
            }
            let n = match here.iter().filter(|r| r.outcome == Outcome::Overflow).max_by_key(|r| r.n_cpu_moe) {
                None => {
                    let Some(s) = start(ctx, kv) else { break };
                    (s as i64 + offset.unwrap_or(0)).max(floor_n as i64) as u64
                }
                Some(o) => {
                    let over = o.overflow_mib.unwrap_or(0).max(0) as u64;
                    o.n_cpu_moe + over.div_ceil(plan.layer_mib).max(1)
                }
            }
            .min(layers);
            if tried(n) {
                break; // all experts on the CPU and still too big
            }
            return Some(Candidate { ctx, kv: kv.clone(), n_cpu_moe: n });
        }
        if ki == 0 {
            first_top = top;
            first_memory = rows.iter().any(|r| r.kv == *kv && r.outcome.memory());
        }
    }
    None
}

/// How much available RAM a candidate must leave: half its part on the CPU (the estimate's
/// `cpu_bytes`), at least [`RAM_FLOOR_MIN_BYTES`], at most [`RAM_FLOOR_BYTES`]. A candidate all on
/// the GPU keeps little in RAM; what loading it costs is file cache that Windows gives back.
pub fn ram_floor(cpu_bytes: u64) -> u64 {
    (cpu_bytes / 2).clamp(RAM_FLOOR_MIN_BYTES, RAM_FLOOR_BYTES)
}

/// Why a running candidate (`cpu_bytes` on the CPU by the estimate) must stop now: the commit
/// limit is nearly reached, available RAM fell below its floor, or the server's shared GPU memory
/// passed its own pinned part plus [`SHARED_LIMIT_BYTES`]. None: go on.
pub fn guard(mem: &Memory, cpu_bytes: u64) -> Option<(Outcome, Limit)> {
    let floor = ram_floor(cpu_bytes);
    let shared_limit = cpu_bytes + SHARED_LIMIT_BYTES;
    if let Some(c) = mem.commit_free.filter(|&c| c < COMMIT_FLOOR_BYTES) {
        Some((Outcome::Ram, Limit::new(LimitKind::Commit, c, COMMIT_FLOOR_BYTES)))
    } else if let Some(r) = mem.available.filter(|&r| r < floor) {
        Some((Outcome::Ram, Limit::new(LimitKind::Ram, r, floor)))
    } else {
        mem.shared.filter(|&s| s > shared_limit).map(|s| (Outcome::Shared, Limit::new(LimitKind::Shared, shared_limit, s)))
    }
}

/// Not worth a start, and why: its part on the CPU (estimate) would leave less than its floor of
/// the available RAM, or less than [`COMMIT_FLOOR_BYTES`] of the commit limit.
pub fn skip_before_start(cpu_bytes: u64, mem: &Memory) -> Option<Limit> {
    let ram_need = cpu_bytes + ram_floor(cpu_bytes);
    let commit_need = cpu_bytes + COMMIT_FLOOR_BYTES;
    if let Some(r) = mem.available.filter(|&r| r < ram_need) {
        Some(Limit::new(LimitKind::Ram, r, ram_need))
    } else {
        mem.commit_free.filter(|&c| c < commit_need).map(|c| Limit::new(LimitKind::Commit, c, commit_need))
    }
}

/// The median (None: no values).
pub fn median(v: &[f64]) -> Option<f64> {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(s[n / 2]),
        _ => Some((s[n / 2 - 1] + s[n / 2]) / 2.0),
    }
}

/// `(max − min) / median` (None: no values or a median of 0).
pub fn spread(v: &[f64]) -> Option<f64> {
    let m = median(v).filter(|&m| m > 0.0)?;
    let max = v.iter().copied().fold(f64::MIN, f64::max);
    let min = v.iter().copied().fold(f64::MAX, f64::min);
    Some((max - min) / m)
}

/// Another measured run is due: fewer than [`MIN_RUNS`], or the spread of the generation speeds
/// is above [`SPREAD_LIMIT`] and there were fewer than [`MAX_RUNS`] (the caller checks the time).
pub fn more_runs(tg: &[f64]) -> bool {
    tg.len() < MIN_RUNS || (tg.len() < MAX_RUNS && spread(tg).is_some_and(|s| s > SPREAD_LIMIT))
}

/// What the memory looked like when a benchmark started (after the active model was stopped): the
/// same settings measure differently with a browser and a game open than with everything closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conditions {
    pub ram_available_mib: Option<u64>,
    pub vram_free_mib: Option<u64>,
    pub commit_free_mib: Option<u64>,
}

impl Conditions {
    /// Close enough that a new result may replace the older one (a value known on one side only
    /// counts as different).
    pub fn similar(&self, other: &Conditions) -> bool {
        let close = |a: Option<u64>, b: Option<u64>, by: u64| match (a, b) {
            (Some(a), Some(b)) => a.abs_diff(b) <= by,
            (None, None) => true,
            _ => false,
        };
        close(self.ram_available_mib, other.ram_available_mib, SAME_RAM_MIB) && close(self.vram_free_mib, other.vram_free_mib, SAME_VRAM_MIB)
    }
}

/// The results kept for a profile after a new one: the new one first, then the older ones whose
/// conditions (`"conditions"`) differ from it, at most [`KEEP_RESULTS`]. A result without
/// conditions (stored before they were recorded) never counts as similar to one with them.
pub fn keep_results(older: Vec<serde_json::Value>, new: serde_json::Value) -> Vec<serde_json::Value> {
    let cond = |v: &serde_json::Value| serde_json::from_value::<Conditions>(v.get("conditions")?.clone()).ok();
    let mut taken: Vec<Option<Conditions>> = vec![cond(&new)];
    let mut out = vec![new];
    for old in older {
        if out.len() >= KEEP_RESULTS {
            break;
        }
        let c = cond(&old);
        let same = |t: &Option<Conditions>| match (t, &c) {
            (Some(a), Some(b)) => a.similar(b),
            (None, None) => true,
            _ => false,
        };
        if !taken.iter().any(same) {
            taken.push(c);
            out.push(old);
        }
    }
    out
}

/// The real overflow: the server's shared GPU memory minus its pinned host buffers (CPU experts,
/// host compute buffer), which WDDM books as shared too.
pub fn overflow_mib(process_shared_bytes: u64, host_mib: f64) -> i64 {
    (process_shared_bytes as f64 / MIB as f64 - host_mib).round() as i64
}

/// Sum of the pinned host buffers in a llama-server log (`ROCm_Host model buffer size = 3129.85
/// MiB`, `… compute buffer …`, `… output buffer …`; CUDA/Vulkan builds name them the same way).
pub fn host_buffer_mib(log: &str) -> f64 {
    log.lines()
        .filter_map(|line| {
            let (head, tail) = line.split_once("buffer size =")?;
            head.split_whitespace().any(|w| w.ends_with("_Host")).then_some(())?;
            tail.split_whitespace().next()?.parse::<f64>().ok()
        })
        .sum()
}

/// The purposes a result is offered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Speed,
    Balanced,
    Context,
}

impl Purpose {
    pub fn parse(s: &str) -> Option<Purpose> {
        match s {
            "speed" => Some(Purpose::Speed),
            "balanced" => Some(Purpose::Balanced),
            "context" => Some(Purpose::Context),
            _ => None,
        }
    }
}

/// A recommendation: the row of the table it points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pick {
    pub purpose: Purpose,
    pub row: usize,
}

/// The three recommendations from the measured rows (fewer when nothing fits):
/// - Tempo: the highest tg at ≥ 8k (any context when no 8k step was measured); among the rows as
///   fast as that one within the measuring noise ([`SPEED_NOISE`], or the fastest row's own spread
///   up to [`SPREAD_LIMIT`]) the largest context, so "Tempo" does not give context away for a
///   difference the next run would not show,
/// - Ausgewogen: the largest context with tg ≥ 85 % of the best,
/// - Kontext: the largest context without overflow with tg ≥ 15 tokens/s.
///
/// Ties go to the higher tg.
pub fn recommend(rows: &[Row]) -> Vec<Pick> {
    let good: Vec<(usize, &Row)> = rows.iter().enumerate().filter(|(_, r)| r.fits()).collect();
    let tg = |r: &Row| r.tg.unwrap_or(0.0);
    let largest = |set: Vec<&(usize, &Row)>| set.into_iter().max_by(|a, b| a.1.ctx.cmp(&b.1.ctx).then(tg(a.1).total_cmp(&tg(b.1)))).map(|(i, _)| *i);
    let mut out = Vec::new();
    let wide: Vec<(usize, &Row)> = good.iter().filter(|(_, r)| r.ctx >= SPEED_MIN_CTX).copied().collect();
    let speed_set = if wide.is_empty() { &good } else { &wide };
    if let Some((_, fastest)) = speed_set.iter().max_by(|a, b| tg(a.1).total_cmp(&tg(b.1))) {
        let noise = fastest.spread.unwrap_or(0.0).clamp(SPEED_NOISE, SPREAD_LIMIT);
        let floor = tg(fastest) * (1.0 - noise) - 1e-9;
        if let Some(i) = largest(speed_set.iter().filter(|(_, r)| tg(r) >= floor).collect()) {
            out.push(Pick { purpose: Purpose::Speed, row: i });
        }
    }
    let best = good.iter().map(|(_, r)| tg(r)).fold(0.0, f64::max);
    if let Some(i) = largest(good.iter().filter(|(_, r)| tg(r) >= best * BALANCED_SHARE).collect()) {
        out.push(Pick { purpose: Purpose::Balanced, row: i });
    }
    if let Some(i) = largest(good.iter().filter(|(_, r)| tg(r) >= CONTEXT_MIN_TG).collect()) {
        out.push(Pick { purpose: Purpose::Context, row: i });
    }
    out
}

/// The profile's load arguments with the candidate's options (the old ones of those flags go).
pub fn args_with(base: &[String], c: &Candidate) -> Vec<String> {
    let mut out = without_flags(base, &TUNED_FLAGS);
    out.extend(["-c".to_string(), c.ctx.to_string(), "-ctk".into(), c.kv.clone(), "-ctv".into(), c.kv.clone()]);
    if c.n_cpu_moe > 0 {
        out.extend(["--n-cpu-moe".to_string(), c.n_cpu_moe.to_string()]);
    }
    out
}

/// The arguments of a benchmark run: [`args_with`] plus `-lv 4` (unless the profile sets a log
/// level), so the server log names its pinned host buffers (`ROCm_Host … buffer size`), which the
/// overflow is measured against. Never written into the profile.
pub fn run_args(base: &[String], c: &Candidate) -> Vec<String> {
    let mut out = args_with(base, c);
    if arg_value(base, &["-lv", "--verbosity", "--log-verbosity"]).is_none() && !base.iter().any(|a| a == "-v" || a == "--verbose" || a == "--log-verbose") {
        out.extend(["-lv".to_string(), "4".to_string()]);
    }
    out
}

/// The value of the last of `names` in `args` (`-ngl 0`, `--gpu-layers=0`).
pub fn arg_value<'a>(args: &'a [String], names: &[&str]) -> Option<&'a str> {
    let mut found = None;
    for (i, t) in args.iter().enumerate() {
        if !is_flag(t) {
            continue;
        }
        match t.split_once('=') {
            Some((flag, v)) if names.contains(&flag) => found = Some(v),
            None if names.contains(&t.as_str()) => found = args.get(i + 1).map(String::as_str).filter(|v| !is_flag(v)),
            _ => {}
        }
    }
    found
}

/// `-ngl` of the arguments: None when not set (llama.cpp puts everything it can on the GPU).
pub fn gpu_layers(args: &[String]) -> Option<u64> {
    arg_value(args, &["-ngl", "--gpu-layers", "--n-gpu-layers"]).and_then(|v| v.parse().ok())
}

/// Neutral narrative text of about `tokens` tokens (≈ 1.3 tokens per word), varied so that no
/// two sentences in a row are the same.
pub fn filler(tokens: usize) -> String {
    const SENTENCES: [&str; 12] = [
        "The morning train left the valley station a few minutes late, and the conductor walked slowly through the carriages.",
        "Along the river the old mills had been turned into workshops where carpenters built boats for the summer season.",
        "In the market square a baker sold round loaves while children chased pigeons around the stone fountain.",
        "The lighthouse keeper wrote the weather into his log every evening, noting the wind, the clouds and the height of the waves.",
        "Beyond the last farm the road climbed into the forest, where the air smelled of resin and damp moss.",
        "A small library on the corner kept maps of every hiking trail in the region, some of them drawn by hand.",
        "When the rain stopped, the streets filled again with bicycles, delivery carts and people carrying umbrellas.",
        "The museum on the hill showed tools from the early mines, lamps, ropes and wooden carts with iron wheels.",
        "At the harbour the fishermen mended their nets in the afternoon sun and talked about the price of fuel.",
        "The village choir met on Thursdays in the school hall to practise songs for the autumn festival.",
        "On clear nights the observatory opened its dome, and visitors took turns looking at the rings of distant planets.",
        "The ferry crossed the lake twice an hour, carrying tourists, farmers and a postman with heavy canvas bags.",
    ];
    let words_wanted = (tokens as f64 / 1.3) as usize;
    let mut out = String::new();
    let mut words = 0;
    let mut i = 0usize;
    while words < words_wanted {
        let s = SENTENCES[(i * 7) % SENTENCES.len()];
        out.push_str(&format!("Part {}. {s} ", i + 1));
        words += s.split_whitespace().count() + 2;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real measurements on the dev PC (docs/benchmarks.md, M0-Rest and M0.2; Orion-26B-A4B
    /// Q4_K_M, 30 layers, ~640 MiB of experts per layer). Overflow = process shared − host.
    fn m0(ctx: u64, kv: &str, n: u64, overflow: i64, tg: f64) -> Row {
        let outcome = if overflow > OVERFLOW_LIMIT_MIB { Outcome::Overflow } else { Outcome::Ok };
        let mut r = Row::new(&cand(ctx, kv, n), outcome);
        r.pp = (outcome == Outcome::Ok).then_some(2000.0);
        r.tg = (outcome == Outcome::Ok).then_some(tg);
        r.overflow_mib = Some(overflow);
        r.load_s = Some(15.0);
        r
    }

    fn orion_plan() -> Plan {
        Plan { stages: vec![8192, 16384, 32768, 65536, 131072], kvs: vec!["q8_0".into(), "f16".into()], moe_layers: Some(30), layer_mib: 640 }
    }

    fn cand(ctx: u64, kv: &str, n: u64) -> Candidate {
        Candidate { ctx, kv: kv.into(), n_cpu_moe: n }
    }

    #[test]
    fn stages_follow_the_trained_context() {
        assert_eq!(stages(Some(262144)), vec![8192, 16384, 32768, 65536, 131072]);
        assert_eq!(stages(Some(40960)), vec![8192, 16384, 32768, 40960]);
        assert_eq!(stages(Some(4096)), vec![4096]);
        assert_eq!(stages(None), STAGES.to_vec());
    }

    #[test]
    fn moe_search_follows_the_overflow_and_finds_the_minimum() {
        let plan = orion_plan();
        // The estimate says 2 layers at 16k (it does not know the HIP overhead).
        let est = |ctx: u64, _: &str| Some(if ctx <= 16384 { 2 } else { 3 });
        let mut rows = vec![];
        assert_eq!(next(&plan, &rows, est), Some(cand(8192, "q8_0", 2)));
        // 8k N=2: ~0.9 GB overflow → 2 layers more (900 / 640 rounded up).
        rows.push(m0(8192, "q8_0", 2, 900, 25.0));
        assert_eq!(next(&plan, &rows, est), Some(cand(8192, "q8_0", 4)));
        rows.push(m0(8192, "q8_0", 4, 126, 54.5));
        // Calibration: one layer less (N=3: 570 MiB overflow, M0-Rest) → the minimum is 4.
        assert_eq!(next(&plan, &rows, est), Some(cand(8192, "q8_0", 3)));
        rows.push(m0(8192, "q8_0", 3, 570, 29.6));
        // 16k: estimate 2 + learned offset 2 = 4.
        assert_eq!(next(&plan, &rows, est), Some(cand(16384, "q8_0", 4)));
        rows.push(m0(16384, "q8_0", 4, 126, 54.5));
        assert_eq!(next(&plan, &rows, est), Some(cand(32768, "q8_0", 5)), "estimate 3 + offset 2");
        rows.push(m0(32768, "q8_0", 5, 98, 55.4));
        // 64k never below the step before.
        assert_eq!(next(&plan, &rows, est), Some(cand(65536, "q8_0", 5)));
        rows.push(m0(65536, "q8_0", 5, 98, 55.6));
        assert_eq!(next(&plan, &rows, est), Some(cand(131072, "q8_0", 5)));
        // 128k: the RAM guard stops it → q8_0 ends at 64k; f16 goes up to 64k only.
        rows.push(Row::new(&cand(131072, "q8_0", 5), Outcome::Ram));
        assert_eq!(next(&plan, &rows, est), Some(cand(8192, "f16", 2)));
        rows.push(m0(8192, "f16", 2, 1435, 30.0));
        assert_eq!(next(&plan, &rows, est), Some(cand(8192, "f16", 5)), "1435 MiB → 3 layers more");
        rows.push(m0(8192, "f16", 5, 80, 57.0));
        assert_eq!(next(&plan, &rows, est), Some(cand(8192, "f16", 4)), "calibration of the second KV type");
        rows.push(m0(8192, "f16", 4, 400, 30.0));
        assert_eq!(next(&plan, &rows, est), Some(cand(16384, "f16", 5)));
        rows.push(m0(16384, "f16", 5, 700, 30.0));
        rows.push(m0(16384, "f16", 7, 600, 30.0));
        rows.push(m0(16384, "f16", 8, 500, 30.0));
        assert_eq!(next(&plan, &rows, est), None, "three tries on one step, then the KV type ends; nothing above 64k");
    }

    #[test]
    fn dense_and_cpu_models_try_each_step_once() {
        let plan = Plan { stages: vec![2048, 4096], kvs: vec!["q8_0".into(), "f16".into()], moe_layers: None, layer_mib: 1 };
        let est = |_: u64, _: &str| Some(0);
        let mut rows = vec![];
        for (ctx, kv) in [(2048, "q8_0"), (4096, "q8_0"), (2048, "f16"), (4096, "f16")] {
            assert_eq!(next(&plan, &rows, est), Some(cand(ctx, kv, 0)));
            let mut r = Row::new(&cand(ctx, kv, 0), Outcome::Ok);
            r.tg = Some(20.0);
            rows.push(r);
        }
        assert_eq!(next(&plan, &rows, est), None);
        // A dense model that overflows at a step: the larger ones are left out.
        let rows = vec![m0(2048, "q8_0", 0, 900, 0.0)];
        assert_eq!(next(&plan, &rows, est), None, "q8_0 reached no step, so f16 is not tried either");
    }

    /// A step the estimate rules out (too big for the VRAM even with the experts on the CPU, or for
    /// the RAM) is never started: the KV type ends at the step before.
    #[test]
    fn a_step_that_surely_does_not_fit_is_not_started() {
        let start = |ctx: u64, _: &str| (ctx <= 16384).then_some(4);
        let rows = vec![m0(8192, "q8_0", 4, 100, 55.0), m0(8192, "q8_0", 3, 570, 30.0), m0(16384, "q8_0", 4, 100, 55.0)];
        assert_eq!(next(&orion_plan(), &rows, start), Some(cand(8192, "f16", 4)), "32k is left out: q8_0 ends at 16k");
        let dense = Plan { moe_layers: None, layer_mib: 1, ..orion_plan() };
        let rows = vec![m0(8192, "q8_0", 0, 0, 60.0), m0(16384, "q8_0", 0, 0, 58.0)];
        assert_eq!(next(&dense, &rows, start), Some(cand(8192, "f16", 0)));
        assert_eq!(next(&dense, &[], |_: u64, _: &str| None), None, "nothing fits: nothing starts");
    }

    #[test]
    fn a_failed_start_falls_back_to_the_last_good_step() {
        let plan = orion_plan();
        let est = |_: u64, _: &str| Some(4);
        let rows = vec![m0(8192, "q8_0", 4, 120, 55.0), m0(8192, "q8_0", 3, 570, 0.0), Row::new(&cand(16384, "q8_0", 4), Outcome::Failed)];
        assert_eq!(next(&plan, &rows, est), Some(cand(8192, "f16", 4)), "q8_0 stops at 8k, f16 only up to 8k");
        let picks = recommend(&rows);
        assert!(picks.iter().all(|p| p.row == 0), "every purpose falls back to the one good step");
    }

    fn mem(available: u64, commit_free: u64, shared: u64) -> Memory {
        Memory { available: Some(available), commit_free: Some(commit_free), shared: Some(shared) }
    }

    #[test]
    fn guards_and_overflow() {
        // A candidate with 4 GiB on the CPU keeps a 2 GiB floor; one with 8 GiB the full 3 GiB.
        assert_eq!(ram_floor(4 * GIB), 2 * GIB);
        assert_eq!(ram_floor(8 * GIB), RAM_FLOOR_BYTES);
        assert_eq!(ram_floor(0), RAM_FLOOR_MIN_BYTES);
        assert_eq!(guard(&mem(8 * GIB, 40 * GIB, 6 * GIB + 1), 0).map(|g| g.0), Some(Outcome::Shared));
        assert_eq!(guard(&mem(8 * GIB, 40 * GIB, 4 * GIB), 4 * GIB), None);
        assert_eq!(guard(&Memory::default(), 4 * GIB), None, "no counters (not Windows): nothing to stop on");
        // M0-Rest 64k N=5: shared 3,845 MiB, host 3,747 → ~98 MiB.
        assert_eq!(overflow_mib(3845 * MIB, 3747.0), 98);
        // A small GPU with 7 GiB of experts in RAM pins them on purpose: only what comes on top counts.
        assert_eq!(skip_before_start(7 * GIB, &mem(20 * GIB, 40 * GIB, 0)), None);
        assert_eq!(guard(&mem(16 * GIB, 40 * GIB, 8 * GIB), 7 * GIB), None, "7 GiB pinned + 1 GiB overflow");
        assert_eq!(
            guard(&mem(16 * GIB, 40 * GIB, 13 * GIB + 1), 7 * GIB),
            Some((Outcome::Shared, Limit { what: LimitKind::Shared, available_mib: 13 * 1024, needed_mib: 13 * 1024 + 1 }))
        );
        assert!(skip_before_start(4 * GIB, &mem(9 * GIB, 40 * GIB, 0)).is_none());
    }

    /// The owner's case: browser, Discord and Steam open, Windows keeps most of the RAM as standby
    /// cache, so almost nothing is "free" but plenty is "available". A candidate all on the GPU
    /// (~0.6 GiB on the CPU: output + compute host buffers) loads, and while it reads the model file
    /// the available RAM dips to 2.2 GiB: the old flat 3 GiB floor stopped it as "wenig RAM".
    #[test]
    fn a_gpu_candidate_passes_with_little_free_but_enough_available_ram() {
        let cpu = 600 * MIB;
        let free_zero = 300 * MIB; // what "free" would have said: below every floor
        assert!(free_zero < RAM_FLOOR_MIN_BYTES);
        let during_load = mem(2200 * MIB, 30 * GIB, 700 * MIB);
        assert_eq!(guard(&during_load, cpu), None, "available 2.2 GiB > the 1 GiB floor of a GPU candidate");
        assert_eq!(skip_before_start(cpu, &mem(9 * GIB, 30 * GIB, 0)), None);
        // The same reading stops a candidate that keeps 8 GiB of experts in RAM.
        assert_eq!(guard(&during_load, 8 * GIB), Some((Outcome::Ram, Limit { what: LimitKind::Ram, available_mib: 2200, needed_mib: 3072 })));
    }

    #[test]
    fn the_guard_stops_with_the_numbers_when_memory_is_really_short() {
        // Available RAM below even the GPU candidate's floor.
        assert_eq!(guard(&mem(700 * MIB, 30 * GIB, 0), 0), Some((Outcome::Ram, Limit { what: LimitKind::Ram, available_mib: 700, needed_mib: 1024 })));
        // Commit nearly used up: stop first, whatever RAM says (this is what freezes Windows).
        assert_eq!(guard(&mem(12 * GIB, 1536 * MIB, 0), 0), Some((Outcome::Ram, Limit { what: LimitKind::Commit, available_mib: 1536, needed_mib: 2048 })));
        // Before the start: 4 GiB on the CPU + its 2 GiB floor > 5 GiB available.
        assert_eq!(skip_before_start(4 * GIB, &mem(5 * GIB, 40 * GIB, 0)), Some(Limit { what: LimitKind::Ram, available_mib: 5120, needed_mib: 6144 }));
        // Enough RAM, but the commit left would not hold it.
        assert_eq!(skip_before_start(4 * GIB, &mem(16 * GIB, 5 * GIB, 0)), Some(Limit { what: LimitKind::Commit, available_mib: 5120, needed_mib: 6144 }));
        let json = serde_json::to_value(Limit { what: LimitKind::Commit, available_mib: 1, needed_mib: 2 }).unwrap();
        assert_eq!(json, serde_json::json!({ "what": "commit", "available_mib": 1, "needed_mib": 2 }));
    }

    #[test]
    fn median_spread_and_more_runs() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[42.0, 64.0, 60.0]), Some(60.0));
        assert_eq!(median(&[10.0, 40.0, 20.0, 30.0]), Some(25.0));
        assert!((spread(&[57.0, 60.0, 63.0]).unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(spread(&[0.0, 0.0]), None);
        assert!(more_runs(&[60.0, 61.0]), "fewer than three");
        assert!(!more_runs(&[60.0, 61.0, 59.0]), "3.3 % spread: done");
        assert!(more_runs(&[42.0, 64.0, 60.0]), "37 % spread: one more");
        assert!(!more_runs(&[42.0, 64.0, 60.0, 61.0, 62.0, 63.0]), "six runs at most");
        // The row: median instead of mean, the runs kept, uncertain when the spread stays high.
        let mut r = Row::new(&cand(8192, "q8_0", 0), Outcome::Ok);
        r.set_runs(&[(1000.0, 42.04), (1100.0, 64.0), (1050.0, 60.0)]);
        assert_eq!((r.pp, r.tg, r.spread, r.uncertain), (Some(1050.0), Some(60.0), Some(0.366), true), "(64 − 42.04) / 60");
        assert_eq!(r.tg_runs, vec![42.0, 64.0, 60.0]);
        r.set_runs(&[(1000.0, 60.0), (1000.0, 61.0), (1000.0, 62.0)]);
        assert!(!r.uncertain);
        // A row stored before these fields existed still reads.
        let old: Row = serde_json::from_value(serde_json::json!({ "ctx": 8192, "kv": "q8_0", "n_cpu_moe": 0, "pp": 1.0, "tg": 2.0, "overflow_mib": null, "outcome": "ok", "load_s": 3.0 })).unwrap();
        assert_eq!((old.tg_runs.len(), old.spread, old.uncertain, old.limit), (0, None, false, None));
    }

    /// The owner's run: 128k q8_0 was stopped, then the first f16 step must still run; and a KV type
    /// that only crashed does not keep the other one from being tried.
    #[test]
    fn a_stopped_candidate_does_not_block_the_next() {
        let plan = Plan { stages: vec![8192, 65536, 131072], kvs: vec!["q8_0".into(), "f16".into()], moe_layers: None, layer_mib: 1 };
        let est = |_: u64, _: &str| Some(0);
        let mut rows = vec![m0(8192, "q8_0", 0, 0, 60.0), m0(65536, "q8_0", 0, 0, 58.0)];
        rows.push(Row::new(&cand(131072, "q8_0", 0), Outcome::Ram));
        assert_eq!(next(&plan, &rows, est), Some(cand(8192, "f16", 0)), "candidate 2 runs after candidate 1 was stopped");
        rows.push(m0(8192, "f16", 0, 0, 61.0));
        assert_eq!(next(&plan, &rows, est), Some(cand(65536, "f16", 0)));
        // q8_0 crashed at its first step (not for memory): f16 gets its own chance, all steps.
        let crashed = vec![Row::new(&cand(8192, "q8_0", 0), Outcome::Failed)];
        assert_eq!(next(&plan, &crashed, est), Some(cand(8192, "f16", 0)));
        let crashed = vec![crashed[0].clone(), m0(8192, "f16", 0, 0, 60.0), m0(65536, "f16", 0, 0, 59.0)];
        assert_eq!(next(&plan, &crashed, est), Some(cand(131072, "f16", 0)));
        // q8_0 already ran out of RAM at its first step: f16 (twice the KV) is left out.
        let short = vec![Row::new(&cand(8192, "q8_0", 0), Outcome::Ram)];
        assert_eq!(next(&plan, &short, est), None);
    }

    #[test]
    fn results_are_kept_per_conditions() {
        let res = |n: u64, ram: Option<u64>, vram: Option<u64>| {
            serde_json::json!({ "n": n, "conditions": { "ram_available_mib": ram, "vram_free_mib": vram, "commit_free_mib": null } })
        };
        let n = |v: &[serde_json::Value]| v.iter().map(|r| r["n"].as_u64().unwrap_or(0)).collect::<Vec<_>>();
        // Apps open (9 GB RAM, 16 GB VRAM free), then everything closed (18 / 23): both kept.
        let kept = keep_results(vec![], res(1, Some(9000), Some(16000)));
        let kept = keep_results(kept, res(2, Some(18000), Some(23000)));
        assert_eq!(n(&kept), vec![2, 1]);
        // Apps open again, a little different: replaces 1, keeps 2.
        let kept = keep_results(kept, res(3, Some(10500), Some(15500)));
        assert_eq!(n(&kept), vec![3, 2]);
        // Same RAM but a game holds 6 GB of VRAM: different.
        let kept = keep_results(kept, res(4, Some(10000), Some(10000)));
        assert_eq!(n(&kept), vec![4, 3, 2]);
        // A result from before conditions were recorded is kept next to the new ones.
        let kept = keep_results(vec![serde_json::json!({ "n": 0 })], res(5, Some(9000), Some(16000)));
        assert_eq!(n(&kept), vec![5, 0]);
        // At most KEEP_RESULTS.
        let mut kept = vec![];
        for i in 0..6 {
            kept = keep_results(kept, res(i, Some(i * 4000), Some(20000)));
        }
        assert_eq!(n(&kept), vec![5, 4, 3, 2]);
    }

    /// A whole benchmark on a PC of known speed, with the budget rule of the server: before each
    /// candidate the stop and settle, then `time_for_another`; a candidate costs load + warmup +
    /// [`MIN_RUNS`] runs, and the run sizes are decided after the first warmup. Returns the
    /// candidates measured, the sizes used and the seconds the run took.
    fn simulate(candidates: usize, sizes: (usize, u32), pp: f64, tg: f64, load: f64, budget: f64) -> (usize, (usize, u32), f64) {
        let (mut prompt, mut gen) = sizes;
        let secs = |p: usize, g: u32| p as f64 / pp + g as f64 / tg;
        let (mut elapsed, mut costs, mut decided) = (0.0, Vec::<f64>::new(), false);
        for _ in 0..candidates {
            elapsed += SETTLE_SECS;
            if !time_for_another(elapsed, &costs, budget) {
                break;
            }
            let warm = secs(prompt, gen);
            if !decided {
                (prompt, gen) = run_sizes(prompt, gen, warm, load, budget - (elapsed + load + warm));
                decided = true;
            }
            let cost = load + warm + MIN_RUNS as f64 * secs(prompt, gen);
            if elapsed + cost > budget {
                elapsed = budget; // "Zeit um": the candidate is not measured
                break;
            }
            elapsed += cost;
            costs.push(cost);
        }
        (costs.len(), (prompt, gen), elapsed)
    }

    /// M8: an older 4–6 core CPU with a 12B model (pp ≈ 25, tg ≈ 3 tokens/s, ~30 s to load) used to
    /// finish one candidate in the 300 s budget; with the CPU sizes (512 / 32) and runs shortened
    /// after the first warmup it measures at least three. A GPU (and a fast CPU) keep their sizes and
    /// measure every candidate as before.
    #[test]
    fn a_slow_cpu_measures_three_candidates_in_the_budget() {
        let (n, sizes, secs) = simulate(CPU_STAGES * KV_TYPES.len(), (512, 32), 25.0, 3.0, 30.0, 300.0);
        assert!(n >= MIN_CANDIDATES, "slow CPU: {n} candidates, sizes {sizes:?}, {secs:.0} s");
        assert!(sizes.0 >= MIN_PROMPT_TOKENS && sizes.1 >= MIN_GEN_TOKENS && sizes.0 < 512);
        // The dev PC on the CPU (gemma-4-E2B: pp ≈ 900, tg ≈ 30, load ≈ 3 s): sizes unchanged, all six.
        assert_eq!(simulate(6, (512, 32), 900.0, 30.0, 3.0, 300.0).0, 6);
        assert_eq!(simulate(6, (512, 32), 900.0, 30.0, 3.0, 300.0).1, (512, 32));
        // The dev PC on the GPU (P6: pp ≈ 3600, tg ≈ 100, load ≈ 5 s): ten candidates, 2048 / 64 as before.
        let (n, sizes, secs) = simulate(10, (2048, 64), 3600.0, 100.0, 5.0, 300.0);
        assert_eq!((n, sizes), (10, (2048, 64)));
        assert!(secs < 160.0, "{secs} s");
        assert_eq!(run_sizes(2048, 64, 1.2, 5.0, 290.0), (2048, 64), "a fast run is never shortened");
        // Nothing to shorten below the minimum.
        assert_eq!(run_sizes(512, 32, 500.0, 30.0, 100.0), (MIN_PROMPT_TOKENS, MIN_GEN_TOKENS));
    }

    #[test]
    fn a_cpu_plan_spreads_three_steps_over_the_range() {
        let plan = |stages: Vec<u64>| Plan { stages, kvs: vec!["q8_0".into(), "f16".into()], moe_layers: None, layer_mib: 1 }.spread_out().stages;
        assert_eq!(plan(STAGES.to_vec()), vec![8192, 32768, 131072]);
        assert_eq!(plan(vec![4096, 8192, 16384, 32768, 65536, 131072]), vec![4096, 32768, 131072]);
        assert_eq!(plan(vec![8192, 16384, 32768, 40960]), vec![8192, 32768, 40960]);
        assert_eq!(plan(vec![8192, 16384]), vec![8192, 16384], "fewer steps stay as they are");
    }

    #[test]
    fn host_buffers_from_a_real_log() {
        let log = "0.02.264.155 I load_tensors:        ROCm0 model buffer size = 14566.52 MiB
0.02.264.157 I load_tensors:    ROCm_Host model buffer size =  3129.85 MiB
0.09.767.104 I llama_context:  ROCm_Host  output buffer size =     1.00 MiB
0.09.773.444 I llama_kv_cache:      ROCm0 KV buffer size =   170.00 MiB
0.09.871.078 I sched_reserve:      ROCm0 compute buffer size =   442.55 MiB
0.09.871.085 I sched_reserve:  ROCm_Host compute buffer size =    39.53 MiB
0.03.670.176 I load_tensors:   CPU_Mapped model buffer size =   577.50 MiB";
        assert!((host_buffer_mib(log) - 3170.38).abs() < 0.01);
        assert_eq!(host_buffer_mib("no buffers here"), 0.0);
    }

    #[test]
    fn recommendations_from_real_rows() {
        let rows = vec![
            m0(16384, "q8_0", 2, 900, 25.0),
            m0(16384, "q8_0", 4, 126, 54.5),
            m0(32768, "q8_0", 4, 126, 55.4),
            m0(65536, "q8_0", 4, 750, 29.0),
            m0(65536, "q8_0", 5, 98, 55.6),
            m0(65536, "q8_0", 6, 98, 52.6),
            m0(8192, "f16", 4, 100, 58.0),
        ];
        let picks = recommend(&rows);
        let row = |p: Purpose| picks.iter().find(|x| x.purpose == p).map(|x| x.row);
        assert_eq!(row(Purpose::Speed), Some(4), "f16 at 8k is the fastest (58.0), 64k N=5 is within 5 % of it (55.6): the larger context");
        assert_eq!(row(Purpose::Balanced), Some(4), "64k N=5 keeps 55.6 ≥ 85 % of 58");
        assert_eq!(row(Purpose::Context), Some(4), "the largest context without overflow, the faster of the two 64k rows");

        // Slow large contexts: Ausgewogen stays smaller, Kontext takes the largest above 15 tok/s.
        let slow = vec![m0(8192, "q8_0", 0, 0, 60.0), m0(32768, "q8_0", 0, 0, 40.0), m0(65536, "q8_0", 2, 0, 16.0), m0(131072, "q8_0", 4, 0, 9.0)];
        let picks = recommend(&slow);
        let row = |p: Purpose| picks.iter().find(|x| x.purpose == p).map(|x| x.row);
        assert_eq!((row(Purpose::Speed), row(Purpose::Balanced), row(Purpose::Context)), (Some(0), Some(0), Some(2)));

        // Below 8k only (a small CPU test): Tempo still gets one.
        let small = vec![m0(2048, "q8_0", 0, 0, 20.0), m0(4096, "q8_0", 0, 0, 19.0)];
        let picks = recommend(&small);
        assert_eq!(picks.iter().map(|p| p.row).collect::<Vec<_>>(), vec![1, 1, 1], "19 is within 5 % of 20: Tempo takes 4k too");
        assert!(recommend(&[m0(8192, "q8_0", 0, 900, 0.0)]).is_empty(), "nothing fits: no recommendation");
    }

    /// N2, the rows of a live run: 16k f16 wrote 31.5 tokens/s, 128k q8_0 30.6 (3 % less, single
    /// runs spread up to 7 %). "Tempo" must not cost 112k of context for that.
    #[test]
    fn tempo_ignores_differences_within_the_noise() {
        let speed = |rows: &[Row]| recommend(rows).iter().find(|p| p.purpose == Purpose::Speed).map(|p| p.row);
        let live = vec![m0(8192, "q8_0", 0, 0, 30.9), m0(16384, "f16", 0, 0, 31.5), m0(32768, "q8_0", 0, 0, 30.2), m0(65536, "q8_0", 0, 0, 30.4), m0(131072, "q8_0", 0, 0, 30.6)];
        assert_eq!(speed(&live), Some(4), "128k is as fast as 16k within the noise");
        // A real difference stays one: 10 % slower is not "as fast".
        let real = vec![m0(8192, "q8_0", 0, 0, 60.0), m0(32768, "q8_0", 0, 0, 54.0)];
        assert_eq!(speed(&real), Some(0));
        // The fastest row's own spread widens the band, up to the limit of an "uncertain" row.
        let mut wide = real.clone();
        wide[0].spread = Some(0.12);
        assert_eq!(speed(&wide), Some(1), "54 is within 10 % of 60");
        wide[1].tg = Some(53.0);
        assert_eq!(speed(&wide), Some(0), "53 is not");
        // Below 8k the rule does not reach: a 4k row never wins over an 8k one for being close.
        let small = vec![m0(4096, "q8_0", 0, 0, 40.0), m0(8192, "q8_0", 0, 0, 20.0)];
        assert_eq!(speed(&small), Some(1));
    }

    #[test]
    fn args_and_values() {
        let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let base = s(&["-ngl", "999", "-fa", "on", "-c", "131072", "-ctk", "q8_0", "--cache-type-v=q8_0", "-np", "1", "--n-cpu-moe", "6"]);
        assert_eq!(args_with(&base, &cand(65536, "f16", 5)), s(&["-ngl", "999", "-fa", "on", "-np", "1", "-c", "65536", "-ctk", "f16", "-ctv", "f16", "--n-cpu-moe", "5"]));
        assert_eq!(args_with(&base, &cand(8192, "q8_0", 0)).iter().filter(|a| a.contains("moe")).count(), 0);
        assert_eq!(run_args(&s(&["-ngl", "0"]), &cand(8192, "q8_0", 0))[8..], s(&["-lv", "4"]), "the log must name the host buffers");
        assert!(!run_args(&s(&["-lv", "3"]), &cand(8192, "q8_0", 0)).contains(&"4".to_string()), "the profile's own log level stays");
        assert_eq!(gpu_layers(&s(&["-ngl", "0"])), Some(0));
        assert_eq!(gpu_layers(&s(&["--gpu-layers=12", "-c", "4096"])), Some(12));
        assert_eq!(gpu_layers(&s(&["-c", "4096"])), None);
        let text = filler(2048);
        let words = text.split_whitespace().count();
        assert!((1400..1700).contains(&words), "≈ 2k tokens: {words} words");
    }
}
