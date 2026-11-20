//! The curated model catalog (`crates/models/catalog/models.json`, built into the program) with a
//! live check of its repos against the Hugging Face API (size, SHA-256, gated, license), the fit
//! pill per size tier (Passt / Passt mit CPU-Hilfe / Zu groß) for this PC, which models are
//! already on disk, and the best picks per purpose (Rollenspiel / Tempo / Kontext) by quality
//! (researched roleplay and German ratings in the catalog) and fit (the rules: `pick.rs`).
//!
//! Live data is cached in `<data>/catalog-cache.json`; each repo is asked at most once per 24 h
//! (a failed attempt counts too), with one request each (`?blobs=true`). Without network the
//! built-in data is used and the answer says from when it is.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;
use rp_core::gguf::{GgufInfo, PerLayer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::pick::{self, best, vram_budget_gb, Measured, Pc, Sized};
use super::{now, quant, suggest, write_atomic, Fetch};
use crate::logs::{Entry as LogEntry, Level};

pub const CATALOG_JSON: &str = include_str!("../../../models/catalog/models.json");
const CACHE_FILE: &str = "catalog-cache.json";
pub const MAX_AGE_SECS: i64 = 24 * 3600;
const PARALLEL: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Quant {
    pub file: String,
    pub quant: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Quants {
    pub small: Option<Quant>,
    pub balanced: Option<Quant>,
    pub large: Option<Quant>,
}

impl Quants {
    pub fn tiers(&self) -> [(&'static str, Option<&Quant>); 3] {
        [("small", self.small.as_ref()), ("balanced", self.balanced.as_ref()), ("large", self.large.as_ref())]
    }
}

/// A public page behind a model's ratings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// German, as the app's source language.
    pub label: String,
    /// The same in English, shown in every other language (mostly names and figures).
    pub label_en: String,
    pub url: String,
}

/// A catalog text in the app's other languages (German is the field it belongs to).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Translations {
    pub en: String,
    pub es: String,
    pub ru: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub id: String,
    /// Key into [`Catalog::shapes`].
    pub shape: String,
    /// MoE only: share of the tensor bytes in expert tensors (what `--n-cpu-moe` moves to RAM).
    #[serde(default)]
    pub expert_share: Option<f64>,
    pub name: String,
    pub family: String,
    /// Base line for filters: `gemma` / `qwen` / `mistral` / `nemo` / `llama`.
    pub base: String,
    /// What the model is made for, from its card: `general` / `roleplay` / `story` / `adventure`.
    pub focus: Vec<String>,
    pub repo: String,
    pub source_repo: Option<String>,
    pub params_total_b: Option<f64>,
    pub params_active_b: Option<f64>,
    pub moe: bool,
    pub arch: Option<String>,
    pub context_max: Option<u64>,
    pub quants: Quants,
    pub vram_class: String,
    pub vram_est_gb: f64,
    /// `good` / `medium` / `unknown`.
    pub german: String,
    pub german_source: Option<String>,
    /// `standard` / `light` / `uncensored`.
    pub censorship: String,
    /// Roleplay quality 1–5 (researched estimate).
    pub rp: u8,
    /// German quality 1–5 (researched estimate).
    pub de: u8,
    /// 0 refuses adult content, 1 mostly allows it, 2 refusal-removed.
    pub uncensored: u8,
    /// Short German note on the writing style.
    pub writing: String,
    /// The same note in the other languages.
    pub writing_tr: Translations,
    /// `low` / `mid` / `high`.
    pub confidence: String,
    /// Public pages only.
    pub sources: Vec<Source>,
    pub reasoning: bool,
    pub template: Option<String>,
    pub author_sampler: Option<Value>,
    pub license: Option<String>,
    pub gated: bool,
    pub last_checked: String,
    pub notes: Option<String>,
}

impl Model {
    /// Quality for this (German) app: 0.6 × roleplay + 0.4 × German.
    pub fn quality(&self) -> f64 {
        0.6 * f64::from(self.rp) + 0.4 * f64::from(self.de)
    }
}

/// The attention layout of a model line, read from a GGUF header: what the KV cache needs per token.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub layers: u64,
    /// KV heads and key/value length of the full-attention layers.
    pub kv_heads: u64,
    pub key_length: u64,
    pub value_length: u64,
    /// Every `full_every`th layer has full attention, the others a sliding window.
    #[serde(default)]
    pub swa: Option<Swa>,
    /// Every `full_every`th layer has attention, the others a recurrent state.
    #[serde(default)]
    pub recurrent: Option<Recurrent>,
    /// The last N layers reuse the caches of earlier ones.
    #[serde(default)]
    pub shared_kv_layers: u64,
    /// KV heads of every layer, where they differ and no sliding window explains it (0: a layer
    /// without attention, as in the pruned Nemotron models).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_heads_per_layer: Option<Vec<u64>>,
    #[serde(default)]
    pub experts: Option<Experts>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Swa {
    pub full_every: u64,
    pub window: u64,
    pub kv_heads: u64,
    pub key_length: u64,
    pub value_length: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Recurrent {
    pub full_every: u64,
    /// f32 state of one recurrent layer, bytes.
    pub state_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Experts {
    pub count: u64,
    pub used: u64,
}

impl Shape {
    /// What `rp_core::vram::estimate` needs of a file with this shape: `size_bytes` of weights,
    /// `expert_share` of them in expert tensors, the trained context.
    pub fn info(&self, size_bytes: u64, expert_share: f64, context: Option<u64>) -> GgufInfo {
        let layers = self.layers;
        let full = |every: u64, il: u64| (il + 1).is_multiple_of(every.max(1));
        let kv_heads = match (&self.kv_heads_per_layer, self.swa) {
            (Some(per_layer), _) => PerLayer::PerLayer(per_layer.clone()),
            (None, Some(s)) => PerLayer::PerLayer((0..layers).map(|il| if full(s.full_every, il) { self.kv_heads } else { s.kv_heads }).collect()),
            (None, None) => PerLayer::One(self.kv_heads),
        };
        GgufInfo {
            version: 3,
            name: None,
            architecture: None,
            file_type: None,
            file_type_name: String::new(),
            size_label: None,
            context_length: context,
            block_count: Some(layers),
            embedding_length: None,
            head_count: Some(PerLayer::One(self.kv_heads)),
            head_count_kv: Some(kv_heads),
            key_length: Some(self.key_length),
            value_length: Some(self.value_length),
            key_length_swa: self.swa.map(|s| s.key_length),
            value_length_swa: self.swa.map(|s| s.value_length),
            sliding_window: self.swa.map(|s| s.window),
            sliding_window_pattern: self.swa.map(|s| (0..layers).map(|il| !full(s.full_every, il)).collect()),
            shared_kv_layers: (self.shared_kv_layers > 0).then_some(self.shared_kv_layers),
            full_attention_interval: self.recurrent.map(|r| r.full_every),
            recurrent_state_bytes: self.recurrent.map(|r| r.state_bytes),
            expert_count: self.experts.map(|e| e.count),
            expert_used_count: self.experts.map(|e| e.used),
            has_chat_template: true,
            chat_template_family: None,
            tensor_count: 0,
            parameter_count: 0,
            weight_bytes: size_bytes,
            expert_weight_bytes: (size_bytes as f64 * expert_share.clamp(0.0, 1.0)) as u64,
            expert_parameter_count: 0,
            file_bytes: size_bytes,
            split: None,
            is_projector: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u32,
    pub generated: String,
    /// Notes for whoever edits the file; not used by the app.
    #[serde(default)]
    #[allow(dead_code)]
    pub field_notes: Value,
    pub shapes: BTreeMap<String, Shape>,
    pub models: Vec<Model>,
}

pub fn embedded() -> Catalog {
    serde_json::from_str(CATALOG_JSON).expect("catalog JSON is checked by the tests")
}

/// What the live check found for one repo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Live {
    pub checked_at: i64,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub gated: bool,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    /// GGUF files: size and SHA-256.
    #[serde(default)]
    pub files: BTreeMap<String, (u64, Option<String>)>,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    repos: BTreeMap<String, Live>,
}

pub(crate) struct CatState {
    path: PathBuf,
    cache: Cache,
    syncing: bool,
}

impl CatState {
    pub fn load(data_dir: &Path) -> CatState {
        let path = data_dir.join(CACHE_FILE);
        let cache = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        CatState { path, cache, syncing: false }
    }
}

/// File names (lower case) of the GGUF files in `dir` and below (a few levels, bounded).
fn gguf_names(dir: &Path, depth: u32, out: &mut HashSet<String>, budget: &mut u32) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() && depth > 0 => gguf_names(&p, depth - 1, out, budget),
            Ok(t) if t.is_file() => {
                if let Some(n) = p.file_name().and_then(|n| n.to_str()).filter(|n| quant::is_gguf(n)) {
                    out.insert(n.to_ascii_lowercase());
                }
            }
            _ => {}
        }
    }
}

/// What the picks know of the catalog file named `name` (a file name, any case): its memory
/// shape at the size the picks use (`live` size when the repo was checked, else the built-in one).
pub fn info_of(catalog: &Catalog, live: &BTreeMap<String, Live>, name: &str) -> Option<GgufInfo> {
    catalog.models.iter().find_map(|m| {
        let q = m.quants.tiers().into_iter().filter_map(|(_, q)| q).find(|q| q.file.rsplit('/').next().unwrap_or(&q.file).eq_ignore_ascii_case(name))?;
        let size = live.get(&m.repo).filter(|l| l.ok).and_then(|l| l.files.get(&q.file)).map_or(q.size_bytes, |f| f.0);
        Some(catalog.shapes.get(&m.shape)?.info(size, m.expert_share.unwrap_or(0.0), m.context_max))
    })
}

/// The catalog model one of whose tier files is named `name` (a file name, any case).
pub fn model_of_file<'a>(catalog: &'a Catalog, name: &str) -> Option<&'a Model> {
    catalog.models.iter().find(|m| m.quants.tiers().into_iter().filter_map(|(_, q)| q).any(|q| q.file.rsplit('/').next().unwrap_or(&q.file).eq_ignore_ascii_case(name)))
}

impl Fetch {
    fn cat(&self) -> std::sync::MutexGuard<'_, CatState> {
        self.cat.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The PC the picks are made for: the last "PC prüfen", else read now.
    pub async fn pc(&self) -> Option<Pc> {
        let scan = self.hub.store(|s| s.last_pc_scan_json()).ok().flatten();
        match scan.as_deref().and_then(Pc::from_scan) {
            Some(pc) => Some(pc),
            None => tokio::task::spawn_blocking(Pc::read).await.ok().flatten(),
        }
    }

    /// [`info_of`] with the live data of the last catalog check.
    pub fn catalog_info(&self, name: &str) -> Option<GgufInfo> {
        info_of(&embedded(), &self.cat().cache.repos, name)
    }

    /// Model files already on disk: the app's model folder, LM Studio's (`RP_LMSTUDIO_DIR` as for
    /// "Meine Modelle") and those of the profiles.
    fn installed_names(&self) -> HashSet<String> {
        let mut names = HashSet::new();
        let mut budget = 20_000;
        gguf_names(&self.models_dir(), 4, &mut names, &mut budget);
        if let Some(lm) = crate::engine::local::lmstudio_dir() {
            gguf_names(&lm, 4, &mut names, &mut budget);
        }
        if let Ok(files) = self.hub.store(|s| s.profile_model_files()) {
            for f in files {
                if let Some(n) = Path::new(&f).file_name().and_then(|n| n.to_str()) {
                    names.insert(n.to_ascii_lowercase());
                }
            }
        }
        names
    }

    /// Benchmark results per model file name (lower case), from every profile that has one.
    fn measured_files(&self) -> HashMap<String, Measured> {
        let results = self.hub.store(|s| -> crate::store::Res<Vec<(String, Value)>> {
            let mut out = Vec::new();
            for p in s.profiles()? {
                if let Some(r) = s.bench_result(p.id)? {
                    out.push((p.model_file, r));
                }
            }
            Ok(out)
        });
        let mut map: HashMap<String, Measured> = HashMap::new();
        for (file, r) in results.unwrap_or_default() {
            let (Some(name), Some(m)) = (Path::new(&file).file_name().and_then(|n| n.to_str()), Measured::from_result(&r)) else { continue };
            let key = name.to_ascii_lowercase();
            let best = Measured::max(map.get(&key).copied(), m);
            map.insert(key, best);
        }
        map
    }

    /// Starts the live check of every repo whose data is older than 24 h (in the background;
    /// every device gets `{"type":"catalog"}` when it is done). False when nothing is due.
    pub fn sync_due(self: &Arc<Self>, catalog: &Catalog) -> bool {
        let due: Vec<String> = {
            let mut st = self.cat();
            if st.syncing {
                return true;
            }
            let t = now();
            let mut repos: Vec<String> = catalog.models.iter().map(|m| m.repo.clone()).collect();
            repos.sort();
            repos.dedup();
            let due: Vec<String> = repos.into_iter().filter(|r| st.cache.repos.get(r).is_none_or(|l| t - l.checked_at >= MAX_AGE_SECS)).collect();
            if due.is_empty() {
                return false;
            }
            st.syncing = true;
            due
        };
        let me = self.clone();
        tokio::spawn(async move {
            let hf = me.hf();
            let results: Vec<(String, Live)> = futures_util::stream::iter(due)
                .map(|repo| {
                    let hf = hf.clone();
                    async move {
                        let live = match hf.repo_files(&repo).await {
                            Ok((info, files)) => Live {
                                checked_at: now(),
                                ok: true,
                                error: None,
                                gated: info.gated.is_some(),
                                license: info.license,
                                last_modified: info.last_modified,
                                files: files.into_iter().filter(|f| quant::is_gguf(&f.path)).map(|f| (f.path, (f.size, f.sha256))).collect(),
                            },
                            Err(e) => Live { checked_at: now(), ok: false, error: Some(e.to_string()), ..Live::default() },
                        };
                        (repo, live)
                    }
                })
                .buffer_unordered(PARALLEL)
                .collect()
                .await;
            let ok = results.iter().filter(|r| r.1.ok).count();
            let failed = results.len() - ok;
            {
                let mut st = me.cat();
                for (repo, live) in results {
                    // A failed check keeps the last good data, only the time moves on.
                    let entry = st.cache.repos.entry(repo).or_default();
                    if live.ok || !entry.ok {
                        *entry = live;
                    } else {
                        entry.checked_at = live.checked_at;
                        entry.error = live.error;
                    }
                }
                st.syncing = false;
                if let Ok(bytes) = serde_json::to_vec(&st.cache) {
                    if let Err(e) = write_atomic(&st.path, &bytes) {
                        eprintln!("catalog cache not written: {e}");
                    }
                }
            }
            me.hub.log().write(LogEntry::new(Level::Info, "server", "catalog", "catalog checked").data(json!({ "ok": ok, "failed": failed })));
            me.hub.notify(&json!({ "type": "catalog" }));
        });
        true
    }

    /// `GET /models/catalog`: the built-in catalog merged with the live data, the pill per tier,
    /// what is installed or measured, and the best picks for this PC (with `uncensored_only` only
    /// among models with `uncensored ≥ 1`).
    pub async fn catalog_view(self: &Arc<Self>, uncensored_only: bool) -> Value {
        let catalog = embedded();
        // The data as it is now; a check started here shows in the next answer.
        let cache: BTreeMap<String, Live> = self.cat().cache.repos.clone();
        let syncing = self.sync_due(&catalog);
        let pc = self.pc().await;
        let me = self.clone();
        let installed = tokio::task::spawn_blocking(move || me.installed_names()).await.unwrap_or_default();
        let measured = self.measured_files();

        let mut sized: Vec<Sized> = Vec::new();
        let mut out = Vec::new();
        let mut live_ok = 0;
        for m in &catalog.models {
            let live = cache.get(&m.repo).filter(|l| l.ok);
            live_ok += usize::from(live.is_some());
            let gated = live.map_or(m.gated, |l| l.gated);
            let mut quants = serde_json::Map::new();
            let shape = catalog.shapes.get(&m.shape).cloned();
            let mut s = Sized { model: m.clone(), shape: shape.clone().unwrap_or_default(), sizes: [None; 3], installed: [false; 3], measured: [None; 3] };
            for (i, (tier, q)) in m.quants.tiers().into_iter().enumerate() {
                let Some(q) = q else {
                    quants.insert(tier.into(), Value::Null);
                    continue;
                };
                let found = live.map(|l| l.files.get(&q.file));
                let missing = matches!(found, Some(None));
                let (size, sha) = match found.flatten() {
                    Some((s, h)) => (*s, h.clone().unwrap_or_else(|| q.sha256.clone())),
                    None => (q.size_bytes, q.sha256.clone()),
                };
                if !missing && !gated {
                    s.sizes[i] = Some(size);
                }
                let file = q.file.to_ascii_lowercase();
                s.installed[i] = installed.contains(&file);
                s.measured[i] = measured.get(&file).copied();
                // The need at 8k all on the GPU, and how this PC would run it.
                let info = shape.as_ref().map(|sh| sh.info(size, m.expert_share.unwrap_or(0.0), m.context_max));
                let need = info.as_ref().and_then(|i| rp_core::vram::estimate(i, &rp_core::vram::Load { ctx: pick::BASE_CTX, cache_k: pick::KV.into(), cache_v: pick::KV.into(), ..Default::default() }));
                let place = info.as_ref().zip(pc.as_ref()).map(|(i, pc)| pick::place(i, pc, pick::BASE_CTX));
                let gb = |b: u64| (b as f64 / 1_073_741_824.0 * 10.0).round() / 10.0;
                quants.insert(
                    tier.into(),
                    json!({
                        "file": q.file, "quant": q.quant, "size_bytes": size, "sha256": sha, "missing": missing,
                        "est_gb": need.map(|e| gb(e.gpu_bytes)),
                        "fit": place.map(|p| p.fit),
                        "n_cpu_moe": place.map(|p| p.n_cpu_moe),
                        "n_gpu_layers": place.and_then(|p| p.gpu_layers),
                        "gpu_gb": place.map(|p| gb(p.gpu_bytes)),
                        "ram_gb": place.map(|p| gb(p.ram_bytes)),
                        "installed": s.installed[i],
                        "measured": s.measured[i],
                    }),
                );
            }
            let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
            v["quants"] = Value::Object(quants);
            v["gated"] = json!(gated);
            v["license"] = json!(live.and_then(|l| l.license.clone()).or_else(|| m.license.clone()));
            v["checked"] = json!(live.map_or(m.last_checked.clone(), |l| date_of(l.checked_at)));
            v["live"] = json!(live.is_some());
            v["q"] = json!((m.quality() * 10.0).round() / 10.0);
            out.push(v);
            // Without a shape (the schema test rules it out) the memory is unknown: no pick.
            if shape.is_some() {
                sized.push(s);
            }
        }
        let mut owners: Vec<String> = quant::QUANTERS.iter().map(|s| s.to_string()).collect();
        for m in &catalog.models {
            if let Some((o, _)) = m.repo.split_once('/') {
                if !owners.iter().any(|x| x == o) {
                    owners.push(o.to_string());
                }
            }
        }
        // The stored purpose and the catalog ordered for it (without needs: by quality and fit).
        let purpose = self.hub.store(|s| s.use_purpose()).unwrap_or_default();
        let for_purpose = pc.as_ref().map(|p| {
            let ranked = suggest::suggest(&sized, p, &purpose.needs);
            let top: Vec<&str> = suggest::top(&ranked, &sized).into_iter().map(|s| s.id.as_str()).collect();
            json!({ "top": top, "ranked": ranked })
        });
        json!({
            "purpose": { "text": purpose.text, "needs": purpose.needs, "figures": purpose.figures },
            "for_purpose": for_purpose,
            "version": catalog.version,
            "generated": catalog.generated,
            "source": self.hf().base().as_str(),
            "syncing": syncing,
            "live": live_ok,
            "pc": pc,
            "vram_budget_gb": pc.as_ref().map(|p| (vram_budget_gb(p) * 10.0).round() / 10.0),
            "best": pc.as_ref().map(|p| best(&sized, p, uncensored_only)),
            "uncensored_only": uncensored_only,
            "quanters": owners,
            "models": out,
        })
    }
}

/// `YYYY-MM-DD` (UTC) of Unix seconds.
pub fn date_of(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalog file: every field as documented, files that match their tier.
    #[test]
    fn catalog_schema() {
        let c: Catalog = serde_json::from_str(CATALOG_JSON).expect("catalog parses with the strict schema");
        assert_eq!(c.version, 3);
        assert!(c.models.len() >= 50, "about 55 entries");
        let mut ids = HashSet::new();
        let date = |s: &str| s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && s.replace('-', "").bytes().all(|b| b.is_ascii_digit());
        assert!(date(&c.generated));
        for m in &c.models {
            assert!(ids.insert(m.id.clone()), "unique id {}", m.id);
            assert!(quant::valid_repo(&m.repo), "{}", m.repo);
            assert!(!m.name.trim().is_empty() && !m.family.trim().is_empty());
            assert!(["4-8", "8-12", "12-16", "16-24", "24+"].contains(&m.vram_class.as_str()), "{} {}", m.id, m.vram_class);
            assert!(["good", "medium", "unknown"].contains(&m.german.as_str()), "{} {}", m.id, m.german);
            assert!(["standard", "light", "uncensored"].contains(&m.censorship.as_str()), "{} {}", m.id, m.censorship);
            assert!(!m.gated, "no gated repos in the catalog: {}", m.id);
            assert!(["gemma", "qwen", "mistral", "nemo", "llama"].contains(&m.base.as_str()), "{} {}", m.id, m.base);
            assert!(!m.focus.is_empty() && m.focus.iter().all(|f| ["general", "roleplay", "story", "adventure"].contains(&f.as_str())), "{} focus {:?}", m.id, m.focus);
            assert!((1..=5).contains(&m.rp) && (1..=5).contains(&m.de) && m.uncensored <= 2, "{} ratings", m.id);
            assert!(["low", "mid", "high"].contains(&m.confidence.as_str()), "{} {}", m.id, m.confidence);
            assert!(!m.writing.trim().is_empty(), "{} writing", m.id);
            // The other languages hold no German letters (Spanish has its own ü).
            let german = |s: &str, letters: &[char]| s.trim().is_empty() || s.contains(letters);
            let t = &m.writing_tr;
            assert!(!german(&t.en, &['ä', 'ö', 'ü', 'ß']) && !german(&t.es, &['ä', 'ö', 'ß']) && !german(&t.ru, &['ä', 'ö', 'ü', 'ß']), "{} writing_tr", m.id);
            assert!(!m.sources.is_empty(), "{} sources", m.id);
            for src in &m.sources {
                assert!(src.url.starts_with("https://") && !src.url.contains(char::is_whitespace), "{} public URL: {}", m.id, src.url);
                assert!(!src.label.trim().is_empty(), "{} label", m.id);
                assert!(!german(&src.label_en, &['ä', 'ö', 'ü', 'ß']), "{} label_en {}", m.id, src.label_en);
            }
            assert_eq!(m.moe, m.params_active_b.is_some(), "active parameters exactly for MoE: {}", m.id);
            assert!(date(&m.last_checked), "{} {}", m.id, m.last_checked);
            assert!(m.vram_est_gb > 0.0 && m.vram_est_gb < 64.0);
            // A trained context, not a metadata artefact (Mistral Nemo merges claim 1,024,000).
            assert!(m.context_max.is_some_and(|c| (8192..=262_144).contains(&c)), "{} context_max {:?}", m.id, m.context_max);
            assert!(m.quants.balanced.is_some(), "{} needs a balanced tier", m.id);
            if m.moe {
                assert!(m.params_active_b.is_some() || m.params_total_b.is_some(), "{}", m.id);
            }
            // The memory estimate's inputs: a known shape, expert data exactly for MoE.
            let shape = c.shapes.get(&m.shape).unwrap_or_else(|| panic!("{}: unknown shape {}", m.id, m.shape));
            assert_eq!(m.moe, shape.experts.is_some(), "{}: experts in the shape exactly for MoE", m.id);
            assert_eq!(m.moe, m.expert_share.is_some_and(|s| s > 0.5 && s < 1.0), "{}: expert_share exactly for MoE", m.id);
            for (tier, q) in m.quants.tiers() {
                let Some(q) = q else { continue };
                assert!(quant::valid_file(&q.file), "{}", q.file);
                assert!(q.sha256.len() == 64 && q.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "{}", q.file);
                assert!(q.size_bytes > 100_000_000, "{}", q.file);
                assert_eq!(quant::quant_of(&q.file), Some(q.quant.as_str()), "quant in the file name of {}", q.file);
                let want = match tier {
                    "small" => quant::Tier::Small,
                    "balanced" => quant::Tier::Balanced,
                    _ => quant::Tier::Large,
                };
                assert_eq!(quant::tier_of(&q.quant), Some(want), "{} in tier {tier}", q.file);
            }
            let sizes: Vec<u64> = m.quants.tiers().iter().filter_map(|(_, q)| q.map(|q| q.size_bytes)).collect();
            assert!(sizes.windows(2).all(|w| w[0] < w[1]), "tiers grow: {}", m.id);
        }
        for (name, s) in &c.shapes {
            assert!(s.layers > 0 && s.kv_heads > 0 && s.key_length > 0 && s.value_length > 0, "{name}");
            assert!(s.swa.is_none_or(|w| w.full_every > 1 && w.window > 0) && s.recurrent.is_none_or(|r| r.full_every > 1 && r.state_bytes > 0), "{name}");
            assert!(s.shared_kv_layers < s.layers && s.experts.is_none_or(|e| e.used < e.count), "{name}");
            assert!(s.kv_heads_per_layer.as_ref().is_none_or(|v| v.len() as u64 == s.layers && s.swa.is_none() && v.iter().all(|h| *h == 0 || *h == s.kv_heads)), "{name}");
            assert!(c.models.iter().any(|m| m.shape == *name), "{name} is used");
        }
    }

    #[test]
    fn dates() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(1_794_787_200), "2026-11-16");
        assert_eq!(date_of(951_782_400), "2000-02-29");
    }
}
