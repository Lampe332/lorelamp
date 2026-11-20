//! The fit pill per size tier and "Bestes Modell für deinen PC", the pure part: a PC profile (VRAM,
//! RAM, GPU vendor, suggested backend) and the catalog in, placements and picks out. The live PC
//! scan only fills the profile ([`Pc::from_scan`], [`Pc::read`]); the tests feed simulated ones.
//!
//! Memory is counted the way llama.cpp allocates it (`rp_core::vram::estimate` on the catalog's
//! attention shape): weights, the KV cache of the app's KV type at the context, buffers. A MoE model
//! that does not fit into the VRAM may keep the experts of its first layers in RAM
//! (`--n-cpu-moe`); its VRAM part and its RAM part are checked each against their own budget.

use std::collections::HashSet;

use rp_core::gguf::GgufInfo;
use rp_core::vram::{self, Load};
use serde::Serialize;
use serde_json::Value;

use super::catalog::{Model, Shape};

/// The KV cache type the app starts models with (`-ctk q8_0 -ctv q8_0`).
pub const KV: &str = "q8_0";
/// The context a model must hold to count as fitting (a chat of some length).
pub const BASE_CTX: u64 = 8192;
/// A tier that still holds this much context wins over a bigger one that holds less (a long chat
/// should not hit the limit soon).
pub const ROOMY_CTX: u64 = 32768;
/// VRAM kept free for Windows and other programs.
pub const VRAM_HEADROOM_GB: f64 = 1.5;
/// What the ROCm/HIP runtime takes on top of the estimate's buffer reserve (the real use is 0.5–1.1
/// GB above llama.cpp's own projection, `docs/benchmarks.md` M0.2).
pub const ROCM_EXTRA_GB: f64 = 0.5;
/// Share of the RAM that may hold what the GPU has no room for (or the whole model without a GPU).
pub const RAM_SHARE: f64 = 0.5;
/// RAM that Windows, a browser and the app itself hold: what is not available to the model server.
pub const OS_RAM_GB: f64 = 6.0;
/// Generation reads every active weight once per token; a byte from system RAM takes about this
/// many times as long as one from VRAM (dual-channel DDR4/DDR5 vs. GDDR6). Only for the order of
/// the Tempo column, never shown as a number.
pub const CPU_COST: f64 = 10.0;
/// Quality a model needs for the Tempo and Kontext columns (when any model that fits reaches it).
pub const MIN_QUALITY: f64 = 3.5;
/// When no model that fits reaches [`MIN_QUALITY`]: how far below the best one may be.
pub const QUALITY_SLACK: f64 = 0.5;
/// A model whose file is on disk counts this much better when the columns are ordered (the
/// owner's rule: installed and measured models first); the quality it shows stays the same.
pub const INSTALLED_BONUS: f64 = 0.5;
/// A benchmark that measured at least this many tokens/s puts a model ahead of every estimate in
/// the Tempo column; one that measured less puts it behind them.
pub const FAST_TG: f64 = 20.0;
/// Without a GPU the CPU reads the prompt too: a context step above this would take many minutes
/// to fill, so the picks offer none larger.
pub const CPU_CTX_MAX: u64 = 32768;

/// "GB" as Windows shows it (GiB): a 24 GB card has 24 × 2^30 bytes.
const GB: f64 = 1_073_741_824.0;
pub(super) const NAMES: [&str; 3] = ["small", "balanced", "large"];
/// Rollenspiel: a model of another base line may stand this far below the best one not shown yet
/// and still be taken first, so a column does not list three copies of one base model.
pub const VARIETY_SLACK: f64 = 1.0;

/// The PC as far as the picks need it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Pc {
    pub gpu: Option<String>,
    pub vram_bytes: u64,
    pub ram_bytes: u64,
    /// `scan`: from the last "PC prüfen"; `live`: read now; `profile`: a simulated PC.
    pub source: &'static str,
    /// `amd` / `nvidia` / `intel` / `other`; None without a GPU.
    pub vendor: Option<String>,
    /// The suggested llama.cpp build (`cuda` / `rocm` / `vulkan` / `cpu`, as `GET /system/scan`
    /// names it); None: not known (a scan from before it was stored).
    pub backend: Option<String>,
}

impl Pc {
    /// A simulated PC (what the tests check the rules with).
    #[cfg(test)]
    pub fn profile(vram_gb: u64, ram_gb: u64, vendor: Option<&str>, backend: &str) -> Pc {
        Pc { gpu: vendor.map(|v| format!("{v} GPU")), vram_bytes: vram_gb << 30, ram_bytes: ram_gb << 30, source: "profile", vendor: vendor.map(str::to_string), backend: Some(backend.to_string()) }
    }

    /// From the stored PC scan (`{gpus:[{name, vendor, vram_total}], ram_total, backend:{suggested}}`):
    /// the biggest GPU.
    pub fn from_scan(json: &str) -> Option<Pc> {
        let v: Value = serde_json::from_str(json).ok()?;
        let ram = v.get("ram_total")?.as_u64()?;
        let gpu = v.get("gpus")?.as_array()?.iter().filter_map(|g| Some((g.get("vram_total")?.as_u64()?, g))).max_by_key(|g| g.0);
        let text = |g: &Value, k: &str| g.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Pc {
            vram_bytes: gpu.map_or(0, |g| g.0),
            gpu: gpu.and_then(|g| text(g.1, "name")),
            ram_bytes: ram,
            source: "scan",
            vendor: gpu.and_then(|g| text(g.1, "vendor")),
            backend: v.get("backend").and_then(|b| text(b, "suggested")),
        })
    }

    /// The biggest GPU (WDDM dedicated memory), the RAM and the suggested build, read directly from
    /// the system.
    pub fn read() -> Option<Pc> {
        let ram = hw::system().ok().map(|s| s.ram_total_bytes)?;
        let gpus = hw::gpus().unwrap_or_default();
        let suggestion = hw::backend_suggestion(&gpus, hw::rocm_runtime().ok().flatten().is_some());
        let gpu = gpus.iter().filter(|g| g.dedicated_bytes >= hw::MIN_DEDICATED_BYTES).max_by_key(|g| g.dedicated_bytes);
        Some(Pc {
            vram_bytes: gpu.map_or(0, |g| g.dedicated_bytes),
            gpu: gpu.map(|g| g.name.clone()),
            ram_bytes: ram,
            source: "live",
            vendor: gpu.map(|g| crate::system::vendor(g.vendor).to_string()),
            backend: Some(crate::system::backend_name(suggestion.backend).0.to_string()),
        })
    }

    /// The model runs on a GPU (a card with its own memory and a GPU build suggested for it).
    pub fn gpu_on(&self) -> bool {
        self.vram_bytes >= hw::MIN_DEDICATED_BYTES && self.backend.as_deref() != Some("cpu")
    }

    /// VRAM the model server may fill: the card minus [`VRAM_HEADROOM_GB`] and, with ROCm,
    /// [`ROCM_EXTRA_GB`]; 0 without a GPU.
    pub fn vram_budget(&self) -> u64 {
        if !self.gpu_on() {
            return 0;
        }
        let extra = if self.backend.as_deref() == Some("rocm") { ROCM_EXTRA_GB } else { 0.0 };
        (self.vram_bytes as f64 - (VRAM_HEADROOM_GB + extra) * GB).max(0.0) as u64
    }

    /// RAM the model may fill: [`RAM_SHARE`] of it.
    pub fn ram_budget(&self) -> u64 {
        (self.ram_bytes as f64 * RAM_SHARE) as u64
    }

    /// `bytes` in RAM are fine: within [`Pc::ram_budget`], and the benchmark would start it too
    /// (with [`OS_RAM_GB`] taken, it leaves its floor of the available RAM, `rp_core::bench::ram_floor`).
    /// On a 16 GB PC the second rule decides, from 32 GB on the first.
    pub fn ram_fits(&self, bytes: u64) -> bool {
        let available = (self.ram_bytes as f64 - OS_RAM_GB * GB).max(0.0) as u64;
        bytes <= self.ram_budget() && bytes + rp_core::bench::ram_floor(bytes) <= available
    }
}

pub fn vram_budget_gb(pc: &Pc) -> f64 {
    pc.vram_budget() as f64 / GB
}

/// Rounded to one decimal, in GB.
pub(super) fn gb1(bytes: u64) -> f64 {
    (bytes as f64 / GB * 10.0).round() / 10.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// All on the GPU.
    Fits,
    /// Part in RAM: the experts of the first `n_cpu_moe` layers (MoE), the first layers of a dense
    /// model (`-ngl`, pill only) or, without a GPU, the whole model. Named as in `POST /models/estimate`.
    #[serde(rename = "cpu_help")]
    Cpu,
    TooBig,
}

/// How a file runs on a PC at a context: the verdict, `--n-cpu-moe`, `-ngl`, and the VRAM and RAM
/// parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub fit: Fit,
    pub n_cpu_moe: u64,
    /// `-ngl` the placement needs: the last N layers on the GPU, the others with their KV cache in
    /// RAM. None: no such limit (all on the GPU, MoE experts in RAM, or no GPU at all).
    pub gpu_layers: Option<u64>,
    pub gpu_bytes: u64,
    pub ram_bytes: u64,
}

const TOO_BIG: Place = Place { fit: Fit::TooBig, n_cpu_moe: 0, gpu_layers: None, gpu_bytes: 0, ram_bytes: 0 };

impl Place {
    /// A setting the picks may suggest: all on the GPU, the experts of a MoE model in RAM, or a
    /// PC without a GPU. Dense layers on the CPU next to a GPU are too slow to recommend.
    fn usable(&self, moe: bool, gpu_on: bool) -> bool {
        match self.fit {
            Fit::Fits => true,
            Fit::Cpu => moe || !gpu_on,
            Fit::TooBig => false,
        }
    }
}

/// The load arguments the app would use: `ctx`, [`KV`], all layers on the GPU (`gpu`) or none.
fn load(ctx: u64, gpu: bool, n_cpu_moe: u64) -> Load {
    load_layers(ctx, if gpu { None } else { Some(0) }, n_cpu_moe)
}

fn load_layers(ctx: u64, gpu_layers: Option<u64>, n_cpu_moe: u64) -> Load {
    Load { ctx, cache_k: KV.into(), cache_v: KV.into(), gpu_layers, n_cpu_moe }
}

/// Where a file (`info`) runs on `pc` at `ctx`:
/// - with a GPU: all on it when the estimate fits [`Pc::vram_budget`]; else a MoE model with the
///   smallest `--n-cpu-moe` whose VRAM part fits, when its RAM part fits ([`Pc::ram_fits`]); a
///   dense model with as many layers on the GPU as its budget holds ([`by_layers`], `-ngl`);
/// - without a GPU: weights, KV cache and buffers in RAM ([`Pc::ram_fits`]).
pub fn place(info: &GgufInfo, pc: &Pc, ctx: u64) -> Place {
    let vram = pc.vram_budget();
    if !pc.gpu_on() {
        let Some(e) = vram::estimate(info, &load(ctx, false, 0)) else { return TOO_BIG };
        let need = e.cpu_bytes + vram::RESERVE_BYTES;
        return Place { fit: if pc.ram_fits(need) { Fit::Cpu } else { Fit::TooBig }, n_cpu_moe: 0, gpu_layers: None, gpu_bytes: 0, ram_bytes: need };
    }
    if !info.is_moe() {
        return by_layers(info, pc, ctx);
    }
    let Some(all) = vram::estimate(info, &load(ctx, true, 0)) else { return TOO_BIG };
    if all.gpu_bytes <= vram {
        return Place { fit: Fit::Fits, n_cpu_moe: 0, gpu_layers: None, gpu_bytes: all.gpu_bytes, ram_bytes: 0 };
    }
    let layers = info.block_count.unwrap_or(0);
    for n in 1..=layers {
        let Some(e) = vram::estimate(info, &load(ctx, true, n)) else { break };
        if e.gpu_bytes <= vram {
            // More layers on the CPU only need more RAM: the smallest N decides.
            let fit = if pc.ram_fits(e.cpu_bytes) { Fit::Cpu } else { Fit::TooBig };
            return Place { fit, n_cpu_moe: n, gpu_layers: None, gpu_bytes: e.gpu_bytes, ram_bytes: e.cpu_bytes };
        }
    }
    TOO_BIG
}

/// With a GPU, placed by whole layers (`-ngl`): all on the GPU when its budget holds them, else
/// the most layers whose VRAM part fits, the first ones with their KV cache in RAM. What a dense
/// model needs, and a MoE model on a build without `--n-cpu-moe`.
pub fn by_layers(info: &GgufInfo, pc: &Pc, ctx: u64) -> Place {
    let vram = pc.vram_budget();
    let layers = info.block_count.unwrap_or(0);
    for g in (0..=layers).rev() {
        let Some(e) = vram::estimate(info, &load_layers(ctx, Some(g), 0)) else { break };
        if e.gpu_bytes > vram {
            continue;
        }
        if g == layers {
            return Place { fit: Fit::Fits, n_cpu_moe: 0, gpu_layers: None, gpu_bytes: e.gpu_bytes, ram_bytes: 0 };
        }
        // Fewer layers on the GPU only need more RAM: the largest N decides.
        let fit = if pc.ram_fits(e.cpu_bytes) { Fit::Cpu } else { Fit::TooBig };
        return Place { fit, n_cpu_moe: 0, gpu_layers: Some(g), gpu_bytes: e.gpu_bytes, ram_bytes: e.cpu_bytes };
    }
    TOO_BIG
}

/// What a benchmark measured for a model file (`bench_result_<profile id>`): the fastest
/// generation and the largest context among the settings that fit.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Measured {
    /// Generation, tokens/s.
    pub tg: f64,
    pub ctx: u64,
}

impl Measured {
    /// From a stored benchmark result (`{rows:[{ctx, tg, outcome, …}]}`); None when nothing fit.
    pub fn from_result(result: &Value) -> Option<Measured> {
        let rows: Vec<rp_core::bench::Row> = serde_json::from_value(result.get("rows")?.clone()).ok()?;
        let ok: Vec<&rp_core::bench::Row> = rows.iter().filter(|r| r.fits()).collect();
        let tg = ok.iter().filter_map(|r| r.tg).max_by(f64::total_cmp)?;
        let ctx = ok.iter().map(|r| r.ctx).max()?;
        Some(Measured { tg: (tg * 10.0).round() / 10.0, ctx })
    }

    /// The better of two measurements of the same file (several profiles can use it).
    pub fn max(a: Option<Measured>, b: Measured) -> Measured {
        match a {
            Some(a) if a.tg >= b.tg => Measured { ctx: a.ctx.max(b.ctx), ..a },
            Some(a) => Measured { ctx: a.ctx.max(b.ctx), ..b },
            None => b,
        }
    }
}

/// A catalog model as the picks see it: its attention shape, per tier (small, balanced, large) the
/// size that can be downloaded (None: no such tier, gone from the repo or gated), whether the file
/// is on disk and what a benchmark measured with it.
#[derive(Debug, Clone)]
pub struct Sized {
    pub model: Model,
    pub shape: Shape,
    pub sizes: [Option<u64>; 3],
    pub installed: [bool; 3],
    pub measured: [Option<Measured>; 3],
}

impl Sized {
    /// What the memory estimate needs of tier `i`.
    pub fn info(&self, i: usize) -> Option<GgufInfo> {
        Some(self.shape.info(self.sizes[i]?, self.model.expert_share.unwrap_or(0.0), self.model.context_max))
    }

    /// The pill of tier `i` at [`BASE_CTX`].
    #[cfg(test)]
    pub fn place(&self, i: usize, pc: &Pc) -> Option<Place> {
        Some(place(&self.info(i)?, pc, BASE_CTX))
    }
}

/// Why a model is in a column; the UI turns it into a short German reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Roleplay: the highest quality that fits.
    BestQuality,
    /// Roleplay: rated well and already on disk ([`INSTALLED_BONUS`] put it ahead).
    InstalledQuality,
    /// Tempo: a MoE model (few active parameters), all on the GPU.
    FastMoe,
    /// Tempo: a dense model, all on the GPU.
    SmallDense,
    /// Tempo: a benchmark measured at least [`FAST_TG`] tokens/s with it.
    MeasuredFast,
    /// Tempo: a benchmark measured less than [`FAST_TG`] tokens/s (it stands behind the estimates).
    MeasuredSlow,
    /// Tempo: a MoE model with experts in RAM next to the GPU (slower than all on the GPU).
    PartlyInRam,
    /// Tempo, PC without a GPU: a MoE model on the CPU (few active parameters per token).
    CpuMoe,
    /// Tempo, PC without a GPU: a dense model on the CPU (slow).
    CpuDense,
    /// Kontext: the largest context that fits.
    RoomForContext,
}

/// One pick of "Bestes Modell für deinen PC".
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Pick {
    pub id: String,
    pub tier: &'static str,
    /// The memory it needs at [`BASE_CTX`], GB: the VRAM part (without a GPU: the RAM part).
    pub est_gb: f64,
    /// Quality (0.6 × rp + 0.4 × de), one decimal.
    pub q: f64,
    /// The file of this tier is on disk.
    pub installed: bool,
    /// Benchmark of this file (tokens/s, largest context that fit); None: an estimate only.
    pub measured: Option<Measured>,
    pub reason_code: Reason,
    /// `fits` (all on the GPU) or `cpu_help` (MoE experts in RAM, or a PC without a GPU).
    pub fit: Fit,
    /// `--n-cpu-moe` at [`BASE_CTX`] (0: none).
    pub n_cpu_moe: u64,
    /// VRAM and RAM part at [`BASE_CTX`], GB.
    pub gpu_gb: f64,
    pub ram_gb: f64,
    /// The largest context step (8k … 128k, at most the trained one) that still fits the same way.
    pub ctx_max: u64,
}

/// Why a column is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Empty {
    /// No model of the catalog fits this PC.
    NothingFits,
    /// Every model that fits already stands in another column.
    AllShown,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct EmptyReasons {
    pub roleplay: Option<Empty>,
    pub tempo: Option<Empty>,
    pub context: Option<Empty>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Best {
    pub roleplay: Vec<Pick>,
    pub tempo: Vec<Pick>,
    pub context: Vec<Pick>,
    /// Per column: why it is empty (None: it has picks).
    pub empty: EmptyReasons,
}

/// A model in the tier it would run in on a PC ([`candidate`]).
pub(super) struct Cand<'a> {
    pub(super) m: &'a Model,
    pub(super) tier: usize,
    pub(super) place: Place,
    pub(super) ctx_max: u64,
    pub(super) q: f64,
    pub(super) installed: bool,
    pub(super) measured: Option<Measured>,
    /// Bytes read per generated token, RAM bytes weighted with [`CPU_COST`]: lower is faster.
    pub(super) cost: f64,
}

impl Cand<'_> {
    fn pick(&self, reason_code: Reason) -> Pick {
        let p = &self.place;
        Pick {
            id: self.m.id.clone(),
            tier: NAMES[self.tier],
            est_gb: gb1(if p.gpu_bytes > 0 { p.gpu_bytes } else { p.ram_bytes }),
            q: (self.q * 10.0).round() / 10.0,
            installed: self.installed,
            measured: self.measured,
            reason_code,
            fit: p.fit,
            n_cpu_moe: p.n_cpu_moe,
            gpu_gb: gb1(p.gpu_bytes),
            ram_gb: gb1(p.ram_bytes),
            ctx_max: self.ctx_max,
        }
    }

    /// Tie order: installed before not installed, measured before estimated, then the id.
    fn tie(&self, o: &Cand) -> std::cmp::Ordering {
        o.installed.cmp(&self.installed).then(o.measured.is_some().cmp(&self.measured.is_some())).then_with(|| self.m.id.cmp(&o.m.id))
    }

    /// The quality the columns order by: [`INSTALLED_BONUS`] more when the file is on disk.
    fn rank_q(&self) -> f64 {
        self.q + if self.installed { INSTALLED_BONUS } else { 0.0 }
    }

    /// The context the Kontext column orders by: what a benchmark fitted, else the estimate;
    /// without a GPU at most [`CPU_CTX_MAX`].
    pub(super) fn ctx_rank(&self, gpu_on: bool) -> u64 {
        let ctx = self.measured.map_or(self.ctx_max, |m| m.ctx);
        if gpu_on {
            ctx
        } else {
            ctx.min(CPU_CTX_MAX)
        }
    }
}

/// Bytes a generated token reads: the dense part and the used share of the experts; what sits in
/// RAM counts [`CPU_COST`] times.
fn cost(s: &Sized, size: u64, p: &Place, gpu_on: bool) -> f64 {
    let size = size as f64;
    let share = s.model.expert_share.unwrap_or(0.0);
    let dense = size * (1.0 - share);
    let experts = s.shape.experts.map_or(0.0, |e| size * share * e.used as f64 / e.count.max(1) as f64);
    if !gpu_on {
        return (dense + experts) * CPU_COST;
    }
    let on_cpu = p.n_cpu_moe as f64 / s.shape.layers.max(1) as f64;
    dense + experts * (1.0 - on_cpu) + experts * on_cpu * CPU_COST
}

/// The candidate of a model: the tier it would run in on this PC, None when no tier is usable.
/// Order: an installed tier first; then all on the GPU before RAM help; then a tier that still
/// holds [`ROOMY_CTX`] (or the trained context) before one that holds less; among tiers all on the
/// GPU the biggest, among tiers with RAM help balanced, small, large (the RAM part costs speed).
pub(super) fn candidate<'a>(s: &'a Sized, pc: &Pc) -> Option<Cand<'a>> {
    let gpu_on = pc.gpu_on();
    let top = s.model.context_max.unwrap_or(BASE_CTX);
    let (tier, p, ctx_max) = (0..3)
        .filter_map(|i| {
            let info = s.info(i)?;
            let p = place(&info, pc, BASE_CTX);
            p.usable(s.model.moe, gpu_on).then(|| (i, p, ctx_max(&info, pc, &p, s.model.moe, top)))
        })
        .max_by_key(|(i, p, ctx)| (s.installed[*i], p.fit == Fit::Fits, *ctx >= ROOMY_CTX.min(top), if p.fit == Fit::Fits { *i } else { [1, 2, 0][*i] }))?;
    Some(Cand { m: &s.model, tier, place: p, ctx_max, q: s.model.quality(), installed: s.installed[tier], measured: s.measured[tier], cost: cost(s, s.sizes[tier]?, &p, gpu_on) })
}

/// The largest context step (up to `top`, without a GPU up to [`CPU_CTX_MAX`]) at which the file
/// still runs the way `p` says (all on the GPU stays all on the GPU; RAM help may take more layers).
fn ctx_max(info: &GgufInfo, pc: &Pc, p: &Place, moe: bool, top: u64) -> u64 {
    let top = if pc.gpu_on() { top } else { top.min(CPU_CTX_MAX) };
    rp_core::bench::STAGES
        .iter()
        .copied()
        .filter(|&c| c <= top)
        .filter(|&c| {
            let q = place(info, pc, c);
            q.fit == p.fit && q.usable(moe, pc.gpu_on())
        })
        .max()
        .unwrap_or(BASE_CTX)
}

/// Top 3 per purpose for this PC, each model in at most one column:
/// - Rollenspiel: the highest quality ([`Model::quality`], an installed file [`INSTALLED_BONUS`]
///   more); all on the GPU before RAM help.
/// - Tempo: among the usable ones (up to one point below "good", quality ≥ [`MIN_QUALITY`] − 1)
///   first those a benchmark measured at [`FAST_TG`] tokens/s or more (fastest first), then the
///   estimates by the fewest bytes read per token ([`cost`]: where the model runs is in it, in
///   steps of 0.5 GB), then those measured slower; among equals the good ones, then quality.
/// - Kontext: among the usable ones the largest context that fits (measured before estimated;
///   without a GPU at most [`CPU_CTX_MAX`]); among equals the good ones, then quality.
///
/// "Good" falls back to "within [`QUALITY_SLACK`] of the best that fits" on small PCs where no
/// model reaches [`MIN_QUALITY`]; an installed file counts [`INSTALLED_BONUS`] more. When every
/// usable one is already shown, Tempo and Kontext go on with the rest, in the same order. Ties:
/// installed, then measured, first. Each column gets its share of the models that fit (as if the
/// columns took turns: with three models one each, from nine on three each), Rollenspiel chooses
/// first, and no model stands twice; a column without one is empty with its reason ([`Empty`]).
///
/// Variety (N14): fine-tunes of one base model (the same `shape`) write and run alike, so each
/// base line stands in the columns once, as long as a model of another line is about as good
/// (Rollenspiel: within [`VARIETY_SLACK`] of the best not shown; Tempo and Kontext: usable) and
/// not less at hand (installed, measured); else the best model not shown yet is taken, whatever
/// its line. Within a column the picks stand in the column's own order. The Tempo reason
/// says where the model runs ([`Reason`]): "fast" codes only for all on the GPU or a measurement.
/// `uncensored_only` keeps models with `uncensored ≥ 1`.
pub fn best(models: &[Sized], pc: &Pc, uncensored_only: bool) -> Best {
    let gpu_on = pc.gpu_on();
    let cands: Vec<Cand> = models.iter().filter(|s| !uncensored_only || s.model.uncensored >= 1).filter_map(|s| candidate(s, pc)).collect();
    let top_q = cands.iter().map(|c| c.q).fold(0.0, f64::max);
    let min_q = MIN_QUALITY.min(top_q - QUALITY_SLACK) - 1e-9;

    let mut roleplay: Vec<&Cand> = cands.iter().collect();
    roleplay.sort_by(|a, b| b.rank_q().total_cmp(&a.rank_q()).then((b.place.fit == Fit::Fits).cmp(&(a.place.fit == Fit::Fits))).then_with(|| a.tie(b)));

    // Good ones first, then those up to one point below, then the rest: the others only fill a
    // column the good ones (shown elsewhere) leave empty.
    let band = |c: &Cand| if c.rank_q() >= min_q { 0 } else if c.rank_q() >= min_q - 1.0 { 1 } else { 2 };
    // Measured fast (fastest first), estimated (cheapest first), measured slow (fastest first).
    let speed = |c: &Cand| match c.measured {
        Some(m) if m.tg >= FAST_TG => (0, -m.tg),
        None => (1, (c.cost / GB * 2.0).round()),
        Some(m) => (2, -m.tg),
    };
    let by_speed = |a: &Cand, b: &Cand| {
        let (x, y) = (speed(a), speed(b));
        x.0.cmp(&y.0).then(x.1.total_cmp(&y.1))
    };
    // Tempo and Kontext order by their own measure among everything up to one point below "good"
    // (the best models stand in Rollenspiel; here the fastest and the roomiest of the others
    // count), the good ones first among equals; the rest only fills what they leave empty.
    let usable = |c: &Cand| i32::from(band(c) == 2);
    let mut tempo: Vec<&Cand> = cands.iter().collect();
    tempo.sort_by(|a, b| usable(a).cmp(&usable(b)).then_with(|| by_speed(a, b)).then(band(a).cmp(&band(b))).then(b.q.total_cmp(&a.q)).then_with(|| a.tie(b)));

    let mut context: Vec<&Cand> = cands.iter().collect();
    context.sort_by(|a, b| usable(a).cmp(&usable(b)).then(b.ctx_rank(gpu_on).cmp(&a.ctx_rank(gpu_on))).then(band(a).cmp(&band(b))).then(b.q.total_cmp(&a.q)).then_with(|| a.tie(b)));

    let roleplay_reason = |c: &Cand| if c.installed { Reason::InstalledQuality } else { Reason::BestQuality };
    let tempo_reason = |c: &Cand| match c.measured {
        Some(m) if m.tg >= FAST_TG => Reason::MeasuredFast,
        Some(_) => Reason::MeasuredSlow,
        None if !gpu_on && c.m.moe => Reason::CpuMoe,
        None if !gpu_on => Reason::CpuDense,
        None if c.place.fit != Fit::Fits => Reason::PartlyInRam,
        None if c.m.moe => Reason::FastMoe,
        None => Reason::SmallDense,
    };
    let mut cols: [Vec<Pick>; 3] = Default::default();
    let mut shown: HashSet<&str> = HashSet::new();
    let mut lines: HashSet<&str> = HashSet::new();
    // Per column the places in its list, so a pick that came later but ranks higher stands first.
    let mut taken: [Vec<usize>; 3] = Default::default();
    let lists = [&roleplay, &tempo, &context];
    // Each column gets its share of the usable models (as if they took turns), Rollenspiel first:
    // the main recommendation has the first choice of the base lines.
    let share = |k: usize| (cands.len() / 3 + usize::from(k < cands.len() % 3)).min(3);
    for (k, list) in lists.into_iter().enumerate() {
        for _ in 0..share(k) {
            let Some(plain) = list.iter().find(|c| !shown.contains(c.m.id.as_str())) else { break };
            // About as good as the plain choice for this column, and not less at hand.
            let near = |c: &Cand| {
                let good = if k == 0 { c.rank_q() >= plain.rank_q() - VARIETY_SLACK - 1e-9 } else { usable(c) <= usable(plain) };
                good && (c.installed || !plain.installed) && (c.measured.is_some() || plain.measured.is_none())
            };
            let at = list.iter().position(|c| !shown.contains(c.m.id.as_str()) && !lines.contains(c.m.shape.as_str()) && near(c)).or_else(|| list.iter().position(|c| std::ptr::eq(*c, *plain)));
            let Some(at) = at else { break };
            lines.insert(list[at].m.shape.as_str());
            shown.insert(list[at].m.id.as_str());
            taken[k].push(at);
        }
    }
    for (k, places) in taken.iter_mut().enumerate() {
        places.sort_unstable();
        for at in places.iter() {
            let c = lists[k][*at];
            cols[k].push(c.pick(match k {
                0 => roleplay_reason(c),
                1 => tempo_reason(c),
                _ => Reason::RoomForContext,
            }));
        }
    }
    let why = |col: &Vec<Pick>| col.is_empty().then_some(if cands.is_empty() { Empty::NothingFits } else { Empty::AllShown });
    let [roleplay, tempo, context] = cols;
    let empty = EmptyReasons { roleplay: why(&roleplay), tempo: why(&tempo), context: why(&context) };
    Best { roleplay, tempo, context, empty }
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{embedded, Catalog};
    use super::*;
    use rp_core::bench::{self, Candidate, Outcome, Plan, Room, Row};
    use serde_json::json;

    fn pc(vram_gb: u64, ram_gb: u64) -> Pc {
        Pc::profile(vram_gb, ram_gb, Some("nvidia"), "cuda")
    }

    /// Built-in sizes, nothing installed or measured.
    fn sized_all(c: &Catalog) -> Vec<Sized> {
        c.models
            .iter()
            .map(|m| Sized {
                model: m.clone(),
                shape: c.shapes[&m.shape].clone(),
                sizes: m.quants.tiers().map(|(_, q)| q.map(|q| q.size_bytes)),
                installed: [false; 3],
                measured: [None; 3],
            })
            .collect()
    }

    fn sized<'a>(all: &'a [Sized], id: &str) -> &'a Sized {
        all.iter().find(|s| s.model.id == id).unwrap()
    }

    fn picks(b: &Best) -> impl Iterator<Item = &Pick> {
        b.roleplay.iter().chain(&b.tempo).chain(&b.context)
    }

    fn row(b: &Best) -> String {
        let col = |l: &[Pick], e: Option<Empty>| {
            if l.is_empty() {
                return format!("({e:?})");
            }
            l.iter().map(|p| format!("{} {}{}{}", p.id, &p.tier[..1], if p.n_cpu_moe > 0 { format!(" N{}", p.n_cpu_moe) } else { String::new() }, format_args!(" {}k", p.ctx_max / 1024))).collect::<Vec<_>>().join(", ")
        };
        format!("RP: {} | Tempo: {} | Kontext: {}", col(&b.roleplay, b.empty.roleplay), col(&b.tempo, b.empty.tempo), col(&b.context, b.empty.context))
    }

    /// The simulated PCs: 6–24 GB VRAM with 16 and 32 GB RAM on each vendor, and PCs without a GPU.
    fn profiles() -> Vec<Pc> {
        let mut out = Vec::new();
        for (vendor, backend) in [("nvidia", "cuda"), ("amd", "rocm"), ("intel", "vulkan")] {
            for vram in [6, 8, 12, 16, 24] {
                for ram in [16, 32] {
                    out.push(Pc::profile(vram, ram, Some(vendor), backend));
                }
            }
        }
        out.extend([16, 32, 64].map(|ram| Pc::profile(0, ram, None, "cpu")));
        out
    }

    #[test]
    fn pill() {
        let c = embedded();
        let all = sized_all(&c);
        let fit = |id: &str, tier: usize, p: &Pc| sized(&all, id).place(tier, p).unwrap();
        // Orion Q5_K_M (18.6 GiB) at 8k: all on a 24 GB card, on a 16 GB card with experts in RAM.
        assert_eq!(fit("orion-26b-a4b-v1-1", 2, &pc(24, 32)).fit, Fit::Fits);
        let p = fit("orion-26b-a4b-v1-1", 2, &pc(16, 32));
        assert_eq!(p.fit, Fit::Cpu);
        assert!(p.n_cpu_moe > 0 && p.gpu_bytes <= pc(16, 32).vram_budget() && p.ram_bytes <= pc(16, 32).ram_budget(), "{p:?}");
        // A dense 31B Q5 on 12 GB: VRAM + half of 32 GB hold it (pill "CPU-Hilfe") with the last
        // N layers on the GPU (`-ngl N`, the setting behind the pill), not 16 GB RAM.
        let dense = fit("gemma-4-31b-it", 2, &pc(12, 32));
        let layers = sized(&all, "gemma-4-31b-it").shape.layers;
        assert_eq!(dense.fit, Fit::Cpu);
        assert!(dense.gpu_layers.is_some_and(|n| n > 0 && n < layers) && dense.n_cpu_moe == 0, "{dense:?}");
        assert!(dense.gpu_bytes <= pc(12, 32).vram_budget() && pc(12, 32).ram_fits(dense.ram_bytes), "{dense:?}");
        assert_eq!(fit("gemma-4-31b-it", 2, &pc(12, 16)).fit, Fit::TooBig);
        // All on the GPU or MoE experts in RAM: no -ngl limit.
        assert_eq!(fit("gemma-4-12b-it", 1, &pc(24, 32)).gpu_layers, None);
        assert_eq!(p.gpu_layers, None);
        // Without a GPU everything is in RAM.
        let none = Pc::profile(0, 16, None, "cpu");
        assert_eq!(fit("gemma-4-e4b-it", 1, &none).fit, Fit::Cpu);
        assert_eq!(fit("gemma-4-31b-it", 1, &none).fit, Fit::TooBig);
        // ROCm keeps 0.5 GB more free than CUDA.
        assert_eq!(Pc::profile(24, 32, Some("nvidia"), "cuda").vram_budget() - Pc::profile(24, 32, Some("amd"), "rocm").vram_budget(), 1 << 29);
    }

    /// The KV cache comes from the shape, not from a flat addition: Orion at 128k q8_0 needs the
    /// 1,519 MiB the server log showed (docs/benchmarks.md M0.1), Mistral Small 160 KiB per token in f16.
    #[test]
    fn kv_cache_from_the_shape() {
        let c = embedded();
        let all = sized_all(&c);
        let info = sized(&all, "orion-26b-a4b-v1-1").info(1).unwrap();
        let kv = |ctx: u64, t: &str| vram::estimate(&info, &Load { ctx, cache_k: t.into(), cache_v: t.into(), ..Load::default() }).unwrap().kv_gpu;
        let mib = kv(131_072, "q8_0") as f64 / 1_048_576.0;
        assert!((mib - 1519.0).abs() < 15.0, "{mib} MiB");
        assert!(kv(131_072, "f16") > kv(131_072, "q8_0") * 18 / 10, "f16 needs about twice the bytes");
        let ms = sized(&all, "cydonia-24b-v4-3").info(1).unwrap();
        let e = |ctx| vram::estimate(&ms, &Load { ctx, ..Load::default() }).unwrap().kv_gpu;
        assert_eq!(e(16_384) - e(8192), 8192 * 160 * 1024);
        // A hybrid Qwen3.5 holds a KV cache in a quarter of its layers only.
        let qw = sized(&all, "qwen3-5-9b-heretic").info(1).unwrap();
        let q = |ctx| vram::estimate(&qw, &Load { ctx, ..Load::default() }).unwrap().kv_gpu;
        assert_eq!(q(16_384) - q(8192), 8192 * 8 * 4 * 512 * 2);
    }

    /// The fix for "the three columns show the same models": on every simulated PC the columns
    /// never share a model, each pick fits by its VRAM and RAM part, a column is only empty with a
    /// reason, and as long as the catalog has three usable models every column has one.
    #[test]
    fn picks_on_every_pc_profile() {
        let c = embedded();
        let all = sized_all(&c);
        for p in profiles() {
            let b = best(&all, &p, false);
            let label = format!("{} GB VRAM {} GB RAM {}", p.vram_bytes >> 30, p.ram_bytes >> 30, p.backend.as_deref().unwrap_or("-"));
            eprintln!("{label}: {}", row(&b));
            let ids: Vec<&str> = picks(&b).map(|x| x.id.as_str()).collect();
            let unique: HashSet<&str> = ids.iter().copied().collect();
            assert_eq!(unique.len(), ids.len(), "{label}: a model in two columns: {ids:?}");
            let usable = all.iter().filter(|s| candidate(s, &p).is_some()).count();
            for (col, e) in [(&b.roleplay, b.empty.roleplay), (&b.tempo, b.empty.tempo), (&b.context, b.empty.context)] {
                assert_eq!(col.is_empty(), e.is_some(), "{label}: an empty column has its reason, a full one none");
                if usable >= 3 {
                    assert!(!col.is_empty(), "{label}: {usable} usable models, still an empty column");
                }
                if usable == 0 {
                    assert_eq!(e, Some(Empty::NothingFits));
                }
            }
            if usable >= 9 {
                assert!([&b.roleplay, &b.tempo, &b.context].iter().all(|l| l.len() == 3), "{label}");
            }
            assert!(b.roleplay.windows(2).all(|w| w[0].q >= w[1].q), "{label}");
            for x in picks(&b) {
                // Re-estimate from the catalog: the VRAM part fits the VRAM budget, the RAM part the RAM budget.
                let s = sized(&all, &x.id);
                let tier = NAMES.iter().position(|t| *t == x.tier).unwrap();
                let info = s.info(tier).unwrap();
                let e = vram::estimate(&info, &load(BASE_CTX, p.gpu_on(), x.n_cpu_moe)).unwrap();
                if p.gpu_on() {
                    assert!(e.gpu_bytes <= p.vram_budget(), "{label}: {} VRAM part {} > {}", x.id, e.gpu_bytes, p.vram_budget());
                    assert!(e.cpu_bytes <= p.ram_budget() && p.ram_fits(e.cpu_bytes), "{label}: {} RAM part", x.id);
                    assert!(x.fit == Fit::Fits || s.model.moe, "{label}: {} dense with layers on the CPU", x.id);
                    assert_eq!(x.n_cpu_moe > 0, x.fit == Fit::Cpu, "{label}: {}", x.id);
                } else {
                    assert!(p.ram_fits(e.cpu_bytes + vram::RESERVE_BYTES), "{label}: {} in RAM", x.id);
                    assert_eq!((x.fit, x.gpu_gb), (Fit::Cpu, 0.0));
                }
                // The largest context fits as well.
                let at_max = place(&info, &p, x.ctx_max);
                assert_eq!(at_max.fit, x.fit, "{label}: {} at {}", x.id, x.ctx_max);
                assert!(x.ctx_max >= BASE_CTX && x.ctx_max <= s.model.context_max.unwrap_or(u64::MAX));
            }
        }
        // Nothing fits a 2 GB card with 4 GB RAM: three empty columns, each with the reason.
        let tiny = best(&all, &Pc::profile(2, 4, Some("nvidia"), "cuda"), false);
        assert!(picks(&tiny).next().is_none());
        assert_eq!(tiny.empty, EmptyReasons { roleplay: Some(Empty::NothingFits), tempo: Some(Empty::NothingFits), context: Some(Empty::NothingFits) });
    }

    /// 12 GB, the reported case: before, all three columns showed Gemma 4 12B heretic, Gemma 4 12B
    /// and Famino. Now a MoE model with its experts in RAM leads Rollenspiel and the columns differ.
    #[test]
    fn twelve_gb_columns_differ() {
        let c = embedded();
        let all = sized_all(&c);
        for ram in [16, 32] {
            let b = best(&all, &pc(12, ram), false);
            let firsts = [&b.roleplay[0].id, &b.tempo[0].id, &b.context[0].id];
            assert!(firsts[0] != firsts[1] && firsts[1] != firsts[2] && firsts[0] != firsts[2], "{firsts:?}");
            assert_eq!(b.roleplay[0].id, "orion-26b-a4b-v1-1", "the best roleplay model with its experts in RAM");
            assert_eq!(b.roleplay[0].fit, Fit::Cpu);
        }
    }

    /// 8 GB with 32 GB RAM: a 26B-A4B MoE runs with its experts in RAM; with 16 GB RAM it does not fit.
    #[test]
    fn eight_gb_card_gets_a_moe_with_experts_in_ram() {
        let c = embedded();
        let all = sized_all(&c);
        let b = best(&all, &pc(8, 32), false);
        let moe: Vec<&Pick> = picks(&b).filter(|p| sized(&all, &p.id).model.moe && p.fit == Fit::Cpu && p.n_cpu_moe > 0).collect();
        assert!(!moe.is_empty(), "{}", row(&b));
        for p in &moe {
            assert!(p.ram_gb > 0.0 && p.gpu_gb <= vram_budget_gb(&pc(8, 32)) + 0.05, "{p:?}");
        }
        assert_eq!(b.roleplay[0].id, "orion-26b-a4b-v1-1");
        let small_ram = best(&all, &pc(8, 16), false);
        assert!(picks(&small_ram).all(|p| p.ram_gb <= 8.0), "{}", row(&small_ram));
    }

    /// 24 GB: the best roleplay model runs all on the GPU in its biggest tier.
    #[test]
    fn twenty_four_gb() {
        let c = embedded();
        let all = sized_all(&c);
        let b = best(&all, &Pc::profile(24, 32, Some("amd"), "rocm"), false);
        assert_eq!((b.roleplay[0].id.as_str(), b.roleplay[0].tier, b.roleplay[0].fit), ("orion-26b-a4b-v1-1", "large", Fit::Fits));
        assert!(picks(&b).all(|p| p.fit == Fit::Fits), "{}", row(&b));
        assert!([&b.roleplay, &b.tempo, &b.context].iter().all(|l| l.len() == 3));
        assert!(b.tempo.iter().all(|p| p.q >= MIN_QUALITY - 1.0), "Tempo takes usable models only: {}", row(&b));
        assert!(b.context.windows(2).all(|w| w[0].ctx_max >= w[1].ctx_max), "{:?}", b.context);
        // The best line leads Rollenspiel in its three kinds of size: 31B dense and 24B dense follow.
        assert_eq!(b.roleplay.iter().map(|p| sized(&all, &p.id).model.shape.as_str()).collect::<Vec<_>>(), ["gemma4-26b-a4b", "gemma4-31b", "mistral-small-24b"]);
        // Tempo leads with a MoE model of another line, all on the GPU.
        assert!(sized(&all, &b.tempo[0].id).model.moe && b.tempo[0].reason_code == Reason::FastMoe, "{}", row(&b));
    }

    /// N14: the columns showed fine-tunes of one base model (on 26 of 33 PCs the three first
    /// places were Gemma-4-26B-A4B descendants). Now a base line stands twice on the page only
    /// when the usable lines (up to one point below "good") are fewer than the places; with nine
    /// usable lines the page shows nine, and the three first places are of three lines.
    #[test]
    fn columns_show_different_base_models() {
        let c = embedded();
        let all = sized_all(&c);
        let mut nine = 0;
        for p in profiles() {
            let b = best(&all, &p, false);
            let label = format!("{} GB / {} GB {}: {}", p.vram_bytes >> 30, p.ram_bytes >> 30, p.backend.as_deref().unwrap_or("-"), row(&b));
            let shape = |x: &Pick| sized(&all, &x.id).model.shape.clone();
            let cands: Vec<Cand> = all.iter().filter_map(|s| candidate(s, &p)).collect();
            let top_q = cands.iter().map(|c| c.q).fold(0.0, f64::max);
            let min_q = MIN_QUALITY.min(top_q - QUALITY_SLACK) - 1e-9;
            let usable: HashSet<&str> = cands.iter().filter(|c| c.q >= min_q - 1.0).map(|c| c.m.shape.as_str()).collect();
            let shown: Vec<String> = picks(&b).map(shape).collect();
            let unique: HashSet<&String> = shown.iter().collect();
            assert!(unique.len() >= shown.len().min(usable.len()), "{label}: {} lines among {} picks, {} usable lines", unique.len(), shown.len(), usable.len());
            if usable.len() >= 9 {
                nine += 1;
                let firsts: HashSet<String> = [&b.roleplay, &b.tempo, &b.context].iter().filter_map(|l| l.first()).map(shape).collect();
                assert_eq!((unique.len(), firsts.len()), (9, 3), "{label}");
            }
        }
        assert!(nine >= 12, "nine usable lines on the PCs from 12 GB up: {nine}");
        // The reported case, 6 GB / 16 GB: one model on the whole page before, now six of four lines.
        let b = best(&all, &pc(6, 16), false);
        assert_eq!(picks(&b).count(), 6, "{}", row(&b));
        assert_eq!(b.empty, EmptyReasons::default());
    }

    /// Ties: installed before not installed, measured before estimated; a measured pick carries
    /// its tokens/s. An installed tier wins over a bigger one that is not on disk.
    #[test]
    fn installed_and_measured_win_ties() {
        let c = embedded();
        let mut all = sized_all(&c);
        let idx = |all: &[Sized], id: &str| all.iter().position(|s| s.model.id == id).unwrap();
        let p = pc(24, 32);
        let base = best(&all, &p, false);
        // Gemma 4 26B-A4B heretic and SOMPOA heresy have the same files and ratings.
        let sompoa = idx(&all, "gemma-4-26b-a4b-sompoa-heresy");
        let pos = |b: &Best, id: &str| picks(b).position(|x| x.id == id);
        // Their line is Orion's: without a file on disk neither stands in a column.
        assert_eq!((pos(&base, "gemma-4-26b-a4b-heretic"), pos(&base, "gemma-4-26b-a4b-sompoa-heresy")), (None, None), "{}", row(&base));
        all[sompoa].installed[1] = true;
        all[sompoa].measured[1] = Some(Measured { tg: 87.5, ctx: 65_536 });
        let b = best(&all, &p, false);
        // Installed and measured, it is shown although its line is there already; its twin is not.
        assert!(pos(&b, "gemma-4-26b-a4b-sompoa-heresy").is_some() && pos(&b, "gemma-4-26b-a4b-heretic").is_none(), "{}", row(&b));
        let s = picks(&b).find(|x| x.id == "gemma-4-26b-a4b-sompoa-heresy").unwrap();
        assert_eq!((s.tier, s.installed, s.measured), ("balanced", true, Some(Measured { tg: 87.5, ctx: 65_536 })), "the installed tier, not the bigger one");
        if b.tempo.iter().any(|x| x.id == s.id) {
            assert_eq!(s.reason_code, Reason::MeasuredFast);
        }
    }

    fn index(all: &[Sized], id: &str) -> usize {
        all.iter().position(|s| s.model.id == id).unwrap()
    }

    /// M2 (a): a model measured at 2 tokens/s is not Tempo's first, it stands behind the
    /// estimates and its reason says it was measured slow.
    #[test]
    fn measured_slow_is_not_tempo_first() {
        let c = embedded();
        let mut all = sized_all(&c);
        let p = Pc::profile(24, 32, Some("amd"), "rocm");
        let first = best(&all, &p, false).tempo[0].clone();
        let i = index(&all, &first.id);
        let tier = NAMES.iter().position(|t| *t == first.tier).unwrap();
        all[i].installed[tier] = true;
        all[i].measured[tier] = Some(Measured { tg: 2.0, ctx: 65_536 });
        let b = best(&all, &p, false);
        assert_ne!(b.tempo[0].id, first.id, "{}", row(&b));
        assert!(b.tempo.iter().all(|x| x.measured.is_none_or(|m| m.tg >= FAST_TG) || x.reason_code == Reason::MeasuredSlow), "{:?}", b.tempo);
        assert!(b.tempo.iter().all(|x| x.reason_code != Reason::MeasuredFast || x.measured.is_some_and(|m| m.tg >= FAST_TG)));
    }

    /// M2 (b): an installed model measured at 150 tokens/s with 128k is Tempo's first (it was in
    /// no column before), with the reason "measured".
    #[test]
    fn measured_fast_installed_leads_tempo() {
        let c = embedded();
        let mut all = sized_all(&c);
        let p = Pc::profile(24, 32, Some("amd"), "rocm");
        let id = "gemma-4-12b-it";
        assert!(picks(&best(&all, &p, false)).all(|x| x.id != id), "in no column without the measurement");
        let i = index(&all, id);
        all[i].installed[1] = true;
        all[i].measured[1] = Some(Measured { tg: 150.0, ctx: 131_072 });
        let b = best(&all, &p, false);
        assert_eq!((b.tempo[0].id.as_str(), b.tempo[0].reason_code), (id, Reason::MeasuredFast), "{}", row(&b));
    }

    /// M2 (c): an installed and measured model moves ahead of a better rated one that is not on
    /// disk (within [`INSTALLED_BONUS`]), with the reason "installed".
    #[test]
    fn installed_moves_ahead_in_roleplay() {
        let c = embedded();
        let mut all = sized_all(&c);
        let p = Pc::profile(24, 32, Some("amd"), "rocm");
        let id = "cydonia-24b-v4-3";
        let before = best(&all, &p, false);
        let place_of = |b: &Best| b.roleplay.iter().position(|x| x.id == id);
        assert_eq!(place_of(&before), Some(2), "{}", row(&before));
        let i = index(&all, id);
        all[i].installed[1] = true;
        all[i].measured[1] = Some(Measured { tg: 15.0, ctx: 32_768 });
        let b = best(&all, &p, false);
        assert_eq!(place_of(&b), Some(1), "{}", row(&b));
        assert_eq!(b.roleplay[1].reason_code, Reason::InstalledQuality);
        assert_eq!(b.roleplay[0].reason_code, Reason::BestQuality);
    }

    /// M3: the Tempo reason says where the model runs. A MoE model with experts in RAM is not
    /// "fast" but "partly in RAM"; Tempo's first has the smallest cost of the good ones; without a
    /// GPU a dense model is never "fast" and no context step is above [`CPU_CTX_MAX`].
    #[test]
    fn tempo_reason_follows_the_placement() {
        let c = embedded();
        let all = sized_all(&c);
        for p in profiles() {
            let b = best(&all, &p, false);
            let label = format!("{} GB / {} GB {}: {}", p.vram_bytes >> 30, p.ram_bytes >> 30, p.backend.as_deref().unwrap_or("-"), row(&b));
            let cands: Vec<Cand> = all.iter().filter_map(|s| candidate(s, &p)).collect();
            let top_q = cands.iter().map(|c| c.q).fold(0.0, f64::max);
            let min_q = MIN_QUALITY.min(top_q - QUALITY_SLACK) - 1e-9;
            // Tempo's first is the cheapest usable model (up to one point below "good") of the
            // base lines Rollenspiel left, and the column stands in the order of its cost.
            let class = |id: &str| (cands.iter().find(|c| c.m.id == id).unwrap().cost / GB * 2.0).round();
            let usable = |id: &str| cands.iter().find(|c| c.m.id == id).unwrap().q >= min_q - 1.0;
            let shown_lines: HashSet<&str> = picks(&b).map(|x| sized(&all, &x.id).model.shape.as_str()).collect();
            // No usable model of a base line that is not on the page would have been faster than
            // Tempo's slowest pick.
            if let Some(last) = b.tempo.iter().rfind(|x| usable(&x.id)) {
                let faster = cands.iter().find(|c| c.q >= min_q - 1.0 && !shown_lines.contains(c.m.shape.as_str()) && (c.cost / GB * 2.0).round() < class(&last.id));
                assert!(faster.is_none(), "{label}: {} is faster", faster.unwrap().m.id);
            }
            assert!(b.tempo.windows(2).all(|w| !(usable(&w[0].id) && usable(&w[1].id)) || class(&w[0].id) <= class(&w[1].id)), "{label}");
            for x in &b.tempo {
                let moe = sized(&all, &x.id).model.moe;
                let want = match (p.gpu_on(), x.fit, moe) {
                    (false, _, true) => Reason::CpuMoe,
                    (false, _, false) => Reason::CpuDense,
                    (true, Fit::Cpu, _) => Reason::PartlyInRam,
                    (true, _, true) => Reason::FastMoe,
                    (true, _, false) => Reason::SmallDense,
                };
                assert_eq!(x.reason_code, want, "{label}: {}", x.id);
            }
            if !p.gpu_on() {
                assert!(picks(&b).all(|x| x.ctx_max <= CPU_CTX_MAX), "{label}");
                assert!(b.tempo.iter().all(|x| !matches!(x.reason_code, Reason::SmallDense | Reason::FastMoe)), "{label}");
            }
        }
        // The reported cases: 6 GB / 32 GB has a MoE with experts in RAM in Tempo, not as "fast".
        let b = best(&all, &pc(6, 32), false);
        assert!(b.tempo.iter().any(|x| x.reason_code == Reason::PartlyInRam), "{}", row(&b));
        // A CPU PC with 64 GB: a large dense model is "on the CPU", never "fast".
        let b = best(&all, &Pc::profile(0, 64, None, "cpu"), false);
        assert!(b.tempo.iter().filter(|x| !sized(&all, &x.id).model.moe).all(|x| x.reason_code == Reason::CpuDense), "{}", row(&b));
    }

    #[test]
    fn uncensored_filter() {
        let c = embedded();
        let all = sized_all(&c);
        for gb in [24, 12, 8] {
            let b = best(&all, &pc(gb, 32), true);
            for p in picks(&b) {
                assert!(sized(&all, &p.id).model.uncensored >= 1, "{}", p.id);
            }
            assert!(!b.roleplay.is_empty());
        }
        // Without the filter the models that refuse adult content are among the picks.
        let open = best(&all, &pc(12, 32), false);
        assert!(picks(&open).any(|p| sized(&all, &p.id).model.uncensored == 0), "{}", row(&open));
    }

    /// The build suggested per GPU vendor, and what it means for the budget.
    #[test]
    fn backend_per_vendor() {
        let gpu = |vendor: hw::Vendor, device_id: u32, driver: &str| hw::Gpu {
            name: "test".into(),
            vendor,
            vendor_id: 0,
            device_id,
            luid: hw::Luid { high: 0, low: 1 },
            dedicated_bytes: 12 << 30,
            shared_bytes: 16 << 30,
            driver: hw::DriverVersion::parse(driver),
        };
        let name = |g: &[hw::Gpu], rocm: bool| crate::system::backend_name(hw::backend_suggestion(g, rocm).backend).0;
        assert_eq!(name(&[gpu(hw::Vendor::Nvidia, 0x2684, "32.0.15.8097")], false), "cuda");
        assert_eq!(name(&[gpu(hw::Vendor::Nvidia, 0x1B80, "32.0.15.8097")], false), "cuda", "a GTX 1080 gets CUDA 12");
        assert_eq!(name(&[gpu(hw::Vendor::Nvidia, 0x2684, "")], false), "cuda", "unknown driver: CUDA 12, not Vulkan");
        assert_eq!(name(&[gpu(hw::Vendor::Amd, 0x744c, "")], true), "rocm");
        assert_eq!(name(&[gpu(hw::Vendor::Amd, 0x744c, "")], false), "vulkan");
        assert_eq!(name(&[gpu(hw::Vendor::Intel, 0x56a0, "")], false), "vulkan");
        assert_eq!(name(&[], false), "cpu");
        // The budget follows the backend: a CPU build uses no VRAM, whatever the card has.
        assert_eq!(Pc::profile(8, 32, Some("other"), "cpu").vram_budget(), 0);
        assert!(Pc::profile(8, 32, Some("intel"), "vulkan").vram_budget() > 0);
    }

    #[test]
    fn measured_from_bench_result() {
        let row = |ctx: u64, tg: Option<f64>, outcome: &str| json!({ "ctx": ctx, "kv": "q8_0", "n_cpu_moe": 0, "pp": 900.0, "tg": tg, "overflow_mib": 0, "outcome": outcome, "load_s": 3.0 });
        let r = json!({ "rows": [row(8192, Some(91.26), "ok"), row(32768, Some(80.0), "ok"), row(131072, Some(20.0), "overflow"), row(65536, None, "ram")] });
        assert_eq!(Measured::from_result(&r), Some(Measured { tg: 91.3, ctx: 32_768 }));
        assert_eq!(Measured::from_result(&json!({ "rows": [row(8192, None, "skipped")] })), None);
        assert_eq!(Measured::from_result(&json!({})), None);
        let a = Measured { tg: 50.0, ctx: 65_536 };
        assert_eq!(Measured::max(Some(a), Measured { tg: 60.0, ctx: 8192 }), Measured { tg: 60.0, ctx: 65_536 });
    }

    #[test]
    fn pc_from_scan() {
        let scan = r#"{"gpus":[{"name":"iGPU","vendor":"intel","vram_total":536870912},{"name":"Big GPU","vendor":"amd","vram_total":25753026560,"driver":"x"}],"ram_total":34359738368,"cpu":{},"backend":{"suggested":"rocm"}}"#;
        assert_eq!(
            Pc::from_scan(scan),
            Some(Pc { gpu: Some("Big GPU".into()), vram_bytes: 25_753_026_560, ram_bytes: 34_359_738_368, source: "scan", vendor: Some("amd".into()), backend: Some("rocm".into()) })
        );
        assert_eq!(Pc::from_scan(r#"{"gpus":[],"ram_total":8}"#).map(|p| (p.vram_bytes, p.gpu_on())), Some((0, false)));
        assert_eq!(Pc::from_scan("{}"), None);
        assert_eq!(serde_json::to_value(Fit::Cpu).unwrap(), "cpu_help");
    }

    /// The benchmark's candidates for a catalog model on a simulated PC: the fitted plan, run with
    /// the estimate standing in for the measurement (a candidate fits when its VRAM part is within
    /// the free VRAM and its RAM part plus floor within the available RAM).
    fn bench_candidates(s: &Sized, tier: usize, p: &Pc) -> Vec<Candidate> {
        let info = s.info(tier).unwrap();
        let gpu = p.gpu_on();
        // Free VRAM with the model server stopped: the card minus what other programs hold;
        // available RAM: what Windows, a browser and the app leave.
        let free = gpu.then(|| p.vram_bytes - (VRAM_HEADROOM_GB * GB) as u64);
        let ram = p.ram_bytes - (OS_RAM_GB * GB) as u64;
        let base = load(0, gpu, 0);
        let room = Room { gpu, free_vram: free, ram_available: Some(ram) };
        let plan = Plan::for_model(&info, gpu, bench::stages(info.context_length)).fit_to(&info, &base, &room);
        let est = |c: &Candidate| vram::estimate(&info, &Load { ctx: c.ctx, cache_k: c.kv.clone(), cache_v: c.kv.clone(), n_cpu_moe: c.n_cpu_moe, ..base.clone() }).unwrap();
        let fits = |c: &Candidate| {
            let e = est(c);
            let need = if gpu { e.cpu_bytes } else { e.cpu_bytes + vram::RESERVE_BYTES };
            free.is_none_or(|f| e.gpu_bytes <= f) && need + bench::ram_floor(need) <= ram
        };
        let mut rows: Vec<Row> = Vec::new();
        let mut out = Vec::new();
        while let Some(c) = bench::next(&plan, &rows, |ctx, kv| room.start(&info, &base, plan.moe_layers, ctx, kv)) {
            let mut r = Row::new(&c, if fits(&c) { Outcome::Ok } else { Outcome::Overflow });
            if r.outcome == Outcome::Ok {
                r.tg = Some(50.0);
            } else {
                r.overflow_mib = Some((est(&c).gpu_bytes.saturating_sub(free.unwrap_or(0)) >> 20) as i64 + 1);
            }
            rows.push(r);
            out.push(c);
            assert!(out.len() < 60, "the search ends");
        }
        // No candidate surely fails: the VRAM part is within the free VRAM plus the margin the run
        // decides, the RAM part fits, except the calibration one layer below a fitting one.
        let possible = |c: &Candidate| {
            let e = est(c);
            let need = if gpu { e.cpu_bytes } else { e.cpu_bytes + vram::RESERVE_BYTES };
            free.is_none_or(|f| e.gpu_bytes <= f + bench::SURE_MARGIN_BYTES) && need + bench::ram_floor(need) <= ram
        };
        for c in &out {
            let above = Candidate { n_cpu_moe: c.n_cpu_moe + 1, ..c.clone() };
            assert!(possible(c) || (s.model.moe && fits(&above)), "{} on {} GB: {c:?} surely does not fit", s.model.id, p.vram_bytes >> 30);
        }
        out
    }

    #[test]
    fn bench_candidates_follow_the_pc() {
        let c = embedded();
        let all = sized_all(&c);
        let orion = sized(&all, "orion-26b-a4b-v1-1");
        for p in profiles() {
            for (id, tier) in [("orion-26b-a4b-v1-1", 1), ("gemma-4-12b-it", 1), ("gemma-4-e4b-it", 1), ("cydonia-24b-v4-3", 0), ("qwen3-6-35b-a3b-heretic", 0)] {
                let list = bench_candidates(sized(&all, id), tier, &p);
                if p.ram_bytes == 32 << 30 && p.backend.as_deref() == Some("cuda") {
                    eprintln!("{} GB / {} GB {id}: {:?}", p.vram_bytes >> 30, p.ram_bytes >> 30, list.iter().map(|c| format!("{}k {} N{}", c.ctx / 1024, c.kv, c.n_cpu_moe)).collect::<Vec<_>>());
                }
            }
        }
        // 24 GB: Orion Q4_K_M starts at 8k with every expert on the GPU.
        let big = bench_candidates(orion, 1, &pc(24, 32));
        assert_eq!(big[0], Candidate { ctx: 8192, kv: "q8_0".into(), n_cpu_moe: 0 });
        // 8 GB: it starts at 4k with experts in RAM, and the lists differ.
        let small = bench_candidates(orion, 1, &pc(8, 32));
        assert_eq!((small[0].ctx, small[0].kv.as_str()), (4096, "q8_0"));
        assert!(small[0].n_cpu_moe > 0, "{small:?}");
        assert_ne!(small, big);
        // 8 GB with 16 GB RAM: the experts do not fit into the RAM either, nothing is started.
        assert!(bench_candidates(orion, 1, &pc(8, 16)).is_empty());
        // A dense 24B on 8 GB surely does not fit: no candidate; on 16 GB it starts at 8k.
        let cydonia = sized(&all, "cydonia-24b-v4-3");
        assert!(bench_candidates(cydonia, 0, &pc(8, 32)).is_empty());
        assert_eq!(bench_candidates(cydonia, 0, &pc(16, 32))[0].ctx, 8192);
        // Without a GPU the steps follow the RAM: a 12B runs with 16 GB available, not with 8 GB
        // (weights, buffers and the 3 GB floor).
        let gemma12 = sized(&all, "gemma-4-12b-it");
        let cpu = bench_candidates(gemma12, 1, &Pc::profile(0, 32, None, "cpu"));
        assert!(!cpu.is_empty() && cpu.iter().all(|c| c.n_cpu_moe == 0), "{cpu:?}");
        assert!(bench_candidates(gemma12, 1, &Pc::profile(0, 16, None, "cpu")).is_empty());
    }

    /// What "Bestes Modell" suggests, the benchmark can measure: every pick on every simulated PC
    /// has at least one candidate, with experts in RAM exactly when the pick has them.
    #[test]
    fn every_pick_can_be_benchmarked() {
        let c = embedded();
        let all = sized_all(&c);
        for p in profiles() {
            let b = best(&all, &p, false);
            for x in picks(&b) {
                let tier = NAMES.iter().position(|t| *t == x.tier).unwrap();
                let list = bench_candidates(sized(&all, &x.id), tier, &p);
                let label = format!("{} GB / {} GB {}: {} {}", p.vram_bytes >> 30, p.ram_bytes >> 30, p.backend.as_deref().unwrap_or("-"), x.id, x.tier);
                assert!(!list.is_empty(), "{label}: no benchmark candidate");
                assert_eq!(list[0].n_cpu_moe > 0, x.n_cpu_moe > 0, "{label}: {list:?}");
            }
        }
    }
}
