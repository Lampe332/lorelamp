//! Minimal GGUF metadata reader: header, key/value pairs and tensor infos.
//!
//! Only the head of the file is read through a buffer; large arrays (the tokenizer
//! vocabulary) are skipped, never loaded. Tensor byte sizes come from the differences
//! between neighbouring tensor offsets, so no table of quant block sizes is needed and
//! new tensor types never break the reader. Unknown keys are ignored; a broken or cut
//! file gives an error, never a panic.

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"GGUF";
/// Strings longer than this are skipped instead of loaded (chat templates are far smaller).
const MAX_STRING: u64 = 4 << 20;
/// Numeric/bool arrays up to this length are loaded (per-layer values); longer ones are skipped.
const MAX_ARRAY: u64 = 4096;
const MAX_KEY: u64 = 64 << 10;
const MAX_DIMS: u32 = 8;

#[derive(Debug)]
pub enum GgufError {
    Io(io::Error),
    NotGguf,
    BigEndian,
    UnsupportedVersion(u32),
    Truncated,
    Invalid(String),
}

impl fmt::Display for GgufError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GgufError::Io(e) => write!(f, "read error: {e}"),
            GgufError::NotGguf => write!(f, "not a GGUF file"),
            GgufError::BigEndian => write!(f, "big-endian GGUF is not supported"),
            GgufError::UnsupportedVersion(v) => write!(f, "unsupported GGUF version {v}"),
            GgufError::Truncated => write!(f, "file is truncated"),
            GgufError::Invalid(m) => write!(f, "invalid GGUF: {m}"),
        }
    }
}

impl std::error::Error for GgufError {}

impl From<io::Error> for GgufError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            GgufError::Truncated
        } else {
            GgufError::Io(e)
        }
    }
}

/// A value that is either one number for all layers or one number per layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum PerLayer {
    One(u64),
    PerLayer(Vec<u64>),
}

impl PerLayer {
    /// Largest value over all layers.
    pub fn max(&self) -> u64 {
        match self {
            PerLayer::One(v) => *v,
            PerLayer::PerLayer(v) => v.iter().copied().max().unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SplitInfo {
    /// Number of parts according to the file name (`-00001-of-0000N`).
    pub count: u32,
    /// Part numbers (1-based) that exist next to the given file.
    pub found: Vec<u32>,
    /// Part numbers that are missing.
    pub missing: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GgufInfo {
    pub version: u32,
    pub name: Option<String>,
    pub architecture: Option<String>,
    pub file_type: Option<u32>,
    /// Readable quant name of `general.file_type` ("unbekannt" when unknown or missing).
    pub file_type_name: String,
    pub size_label: Option<String>,
    pub context_length: Option<u64>,
    pub block_count: Option<u64>,
    pub embedding_length: Option<u64>,
    pub head_count: Option<PerLayer>,
    pub head_count_kv: Option<PerLayer>,
    pub key_length: Option<u64>,
    pub value_length: Option<u64>,
    pub key_length_swa: Option<u64>,
    pub value_length_swa: Option<u64>,
    pub sliding_window: Option<u64>,
    pub sliding_window_pattern: Option<Vec<bool>>,
    pub shared_kv_layers: Option<u64>,
    /// Hybrid models (Qwen3.5/3.6): only every Nth layer has attention (and a KV cache); the
    /// others are recurrent and keep a fixed state ([`GgufInfo::recurrent_state_bytes`]).
    pub full_attention_interval: Option<u64>,
    /// State of one recurrent layer for one sequence, bytes (f32 conv + SSM state, from `ssm.*`).
    pub recurrent_state_bytes: Option<u64>,
    pub expert_count: Option<u64>,
    pub expert_used_count: Option<u64>,
    pub has_chat_template: bool,
    /// Template family by simple detection (gemma, chatml, llama3, mistral, …), "unbekannt" if none matches.
    pub chat_template_family: Option<String>,
    /// Tensors over all parts that were found.
    pub tensor_count: u64,
    /// Sum of tensor elements (estimated parameter count).
    pub parameter_count: u64,
    /// Bytes of all tensor data.
    pub weight_bytes: u64,
    /// Bytes of expert tensors (`*_exps*`), a subset of `weight_bytes`.
    pub expert_weight_bytes: u64,
    /// Elements of the expert tensors, a subset of `parameter_count`.
    pub expert_parameter_count: u64,
    /// Size of all parts on disk.
    pub file_bytes: u64,
    pub split: Option<SplitInfo>,
    /// Multimodal projector (`mmproj-*.gguf` or architecture `clip`), not a main model.
    pub is_projector: bool,
}

impl GgufInfo {
    /// Mixture of experts: more than one expert per layer.
    pub fn is_moe(&self) -> bool {
        self.expert_count.is_some_and(|n| n > 1)
    }

    /// Sliding-window attention in some or all layers.
    pub fn has_swa(&self) -> bool {
        self.sliding_window.is_some_and(|w| w > 0)
            || self.sliding_window_pattern.as_ref().is_some_and(|p| p.iter().any(|&s| s))
    }

    /// Parameters used per token: all of a dense model; for a MoE model the share of the expert
    /// tensors that the `expert_used_count` experts make up, plus everything else.
    pub fn active_parameters(&self) -> u64 {
        match (self.expert_count, self.expert_used_count) {
            (Some(n), Some(used)) if n > 1 && used <= n && self.expert_parameter_count > 0 => {
                let shared = self.parameter_count.saturating_sub(self.expert_parameter_count);
                let per_expert = self.expert_parameter_count as u128 * used as u128 / n as u128;
                shared.saturating_add(per_expert as u64)
            }
            _ => self.parameter_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Val {
    U(u64),
    I(i64),
    B(bool),
    S(String),
    A(Vec<Val>),
    Skipped,
}

impl Val {
    fn as_u64(&self) -> Option<u64> {
        match *self {
            Val::U(v) => Some(v),
            Val::I(v) if v >= 0 => Some(v as u64),
            Val::B(b) => Some(b as u64),
            _ => None,
        }
    }
}

struct Tensor {
    name: String,
    elements: u64,
    offset: u64,
}

struct Parsed {
    version: u32,
    kv: HashMap<String, Val>,
    tensors: Vec<Tensor>,
    /// Absolute start of the tensor data section.
    data_start: u64,
    file_len: u64,
}

struct Cur {
    r: BufReader<File>,
    pos: u64,
    len: u64,
}

impl Cur {
    fn need(&self, n: u64) -> Result<(), GgufError> {
        match self.pos.checked_add(n) {
            Some(end) if end <= self.len => Ok(()),
            _ => Err(GgufError::Truncated),
        }
    }

    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], GgufError> {
        self.need(N as u64)?;
        let mut b = [0u8; N];
        self.r.read_exact(&mut b)?;
        self.pos += N as u64;
        Ok(b)
    }

    fn u32(&mut self) -> Result<u32, GgufError> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }

    fn u64(&mut self) -> Result<u64, GgufError> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }

    fn skip(&mut self, n: u64) -> Result<(), GgufError> {
        self.need(n)?;
        let off = i64::try_from(n).map_err(|_| GgufError::Truncated)?;
        self.r.seek_relative(off)?;
        self.pos += n;
        Ok(())
    }

    /// Reads a string; strings over `limit` are skipped and give `None`.
    fn string(&mut self, limit: u64) -> Result<Option<String>, GgufError> {
        let n = self.u64()?;
        self.need(n)?;
        if n > limit {
            self.skip(n)?;
            return Ok(None);
        }
        let mut buf = vec![0u8; n as usize];
        self.r.read_exact(&mut buf)?;
        self.pos += n;
        Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
    }

    fn scalar(&mut self, ty: u32) -> Result<Val, GgufError> {
        Ok(match ty {
            0 => Val::U(u8::from_le_bytes(self.bytes()?) as u64),
            1 => Val::I(i8::from_le_bytes(self.bytes()?) as i64),
            2 => Val::U(u16::from_le_bytes(self.bytes()?) as u64),
            3 => Val::I(i16::from_le_bytes(self.bytes()?) as i64),
            4 => Val::U(self.u32()? as u64),
            5 => Val::I(i32::from_le_bytes(self.bytes()?) as i64),
            6 => {
                self.skip(4)?;
                Val::Skipped
            }
            7 => Val::B(self.bytes::<1>()?[0] != 0),
            8 => self.string(MAX_STRING)?.map_or(Val::Skipped, Val::S),
            10 => Val::U(self.u64()?),
            11 => Val::I(i64::from_le_bytes(self.bytes()?)),
            12 => {
                self.skip(8)?;
                Val::Skipped
            }
            t => return Err(GgufError::Invalid(format!("unknown value type {t}"))),
        })
    }

    fn value(&mut self, ty: u32, load_arrays: bool) -> Result<Val, GgufError> {
        if ty != 9 {
            return self.scalar(ty);
        }
        let elem = self.u32()?;
        let n = self.u64()?;
        let fixed = fixed_size(elem);
        if elem == 9 {
            return Err(GgufError::Invalid("nested arrays are not supported".into()));
        }
        let loadable = load_arrays && n <= MAX_ARRAY && elem != 8 && fixed.is_some();
        if loadable {
            let mut v = Vec::with_capacity(n as usize);
            for _ in 0..n {
                v.push(self.scalar(elem)?);
            }
            return Ok(Val::A(v));
        }
        match fixed {
            Some(size) => self.skip(n.checked_mul(size).ok_or(GgufError::Truncated)?)?,
            None if elem == 8 => {
                for _ in 0..n {
                    let len = self.u64()?;
                    self.skip(len)?;
                }
            }
            None => return Err(GgufError::Invalid(format!("unknown array type {elem}"))),
        }
        Ok(Val::Skipped)
    }
}

fn fixed_size(ty: u32) -> Option<u64> {
    match ty {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    }
}

fn parse_file(path: &Path) -> Result<Parsed, GgufError> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut c = Cur { r: BufReader::with_capacity(256 << 10, file), pos: 0, len };

    if &c.bytes::<4>()? != MAGIC {
        return Err(GgufError::NotGguf);
    }
    let version = c.u32()?;
    if version > 0xFFFF && version.swap_bytes() <= 0xFFFF {
        return Err(GgufError::BigEndian);
    }
    if !(2..=3).contains(&version) {
        return Err(GgufError::UnsupportedVersion(version));
    }
    let tensor_count = c.u64()?;
    let kv_count = c.u64()?;

    let mut kv = HashMap::new();
    for _ in 0..kv_count {
        let key = c
            .string(MAX_KEY)?
            .ok_or_else(|| GgufError::Invalid("key too long".into()))?;
        let ty = c.u32()?;
        let val = c.value(ty, !key.starts_with("tokenizer."))?;
        kv.insert(key, val);
    }

    // Every tensor info takes at least 24 bytes; this bounds the allocation below.
    c.need(tensor_count.checked_mul(24).ok_or(GgufError::Truncated)?)?;
    let mut tensors = Vec::with_capacity(tensor_count as usize);
    for _ in 0..tensor_count {
        let name = c
            .string(MAX_KEY)?
            .ok_or_else(|| GgufError::Invalid("tensor name too long".into()))?;
        let dims = c.u32()?;
        if dims > MAX_DIMS {
            return Err(GgufError::Invalid(format!("tensor {name} has {dims} dimensions")));
        }
        let mut elements: u64 = 1;
        for _ in 0..dims {
            elements = elements.saturating_mul(c.u64()?);
        }
        let _ty = c.u32()?;
        let offset = c.u64()?;
        tensors.push(Tensor { name, elements, offset });
    }

    let align = kv.get("general.alignment").and_then(Val::as_u64).unwrap_or(32).max(1);
    let data_start = c.pos.div_ceil(align).saturating_mul(align);
    if data_start > len {
        return Err(GgufError::Truncated);
    }
    Ok(Parsed { version, kv, tensors, data_start, file_len: len })
}

/// Tensor byte sizes from offset differences; the last tensor ends at the end of the file.
fn tensor_bytes(p: &Parsed) -> Result<Vec<(usize, u64)>, GgufError> {
    let mut order: Vec<usize> = (0..p.tensors.len()).collect();
    order.sort_by_key(|&i| p.tensors[i].offset);
    let data_len = p.file_len - p.data_start;
    let mut out = Vec::with_capacity(order.len());
    for (k, &i) in order.iter().enumerate() {
        let start = p.tensors[i].offset;
        let end = order.get(k + 1).map_or(data_len, |&j| p.tensors[j].offset);
        if start > data_len || end > data_len {
            return Err(GgufError::Truncated);
        }
        out.push((i, end - start));
    }
    Ok(out)
}

/// `name-00001-of-00003.gguf` → (stem `name`, part 1, count 3); None for a file in one piece.
pub fn split_parts(file_name: &str) -> Option<(&str, u32, u32)> {
    let base = file_name.strip_suffix(".gguf")?;
    let (rest, count) = base.rsplit_once("-of-")?;
    let (stem, part) = rest.rsplit_once('-')?;
    let ok = |s: &str| s.len() == 5 && s.bytes().all(|b| b.is_ascii_digit());
    if !ok(count) || !ok(part) {
        return None;
    }
    let (part, count) = (part.parse().ok()?, count.parse().ok()?);
    (part >= 1 && part <= count).then_some((stem, part, count))
}

/// Readable name of `general.file_type` (llama.cpp `llama_ftype`).
pub fn file_type_name(ft: u32) -> &'static str {
    match ft {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        38 => "MXFP4_MOE",
        _ => "unbekannt",
    }
}

/// Template family by marker tokens.
pub fn chat_template_family(t: &str) -> &'static str {
    if t.contains("<start_of_turn>") || t.contains("<|turn>") {
        "gemma"
    } else if t.contains("<|start|>") && t.contains("<|channel|>") {
        "harmony"
    } else if t.contains("<|im_start|>") {
        "chatml"
    } else if t.contains("<|start_header_id|>") {
        "llama3"
    } else if t.contains("<｜User｜>") || t.contains("<｜Assistant｜>") {
        "deepseek"
    } else if t.contains("[INST]") {
        "mistral"
    } else if t.contains("<|user|>") && t.contains("<|assistant|>") {
        "phi"
    } else {
        "unbekannt"
    }
}

fn per_layer(v: Option<&Val>) -> Option<PerLayer> {
    match v? {
        Val::A(a) => Some(PerLayer::PerLayer(a.iter().map(|x| x.as_u64().unwrap_or(0)).collect())),
        other => other.as_u64().map(PerLayer::One),
    }
}

/// Reads the metadata of a GGUF file. For a split model pass any part; all parts next to
/// it are read (only their headers) and their sizes summed.
pub fn read_info(path: &Path) -> Result<GgufInfo, GgufError> {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let split = split_parts(file_name);

    // Part 1 carries the model metadata; for a split model start there when it exists.
    let mut parts: Vec<(u32, PathBuf)> = Vec::new();
    let mut split_info = None;
    if let Some((stem, _, count)) = split {
        let dir = path.parent().unwrap_or(Path::new(""));
        let (mut found, mut missing) = (Vec::new(), Vec::new());
        for n in 1..=count {
            let p = dir.join(format!("{stem}-{n:05}-of-{count:05}.gguf"));
            if p.is_file() {
                found.push(n);
                parts.push((n, p));
            } else {
                missing.push(n);
            }
        }
        split_info = Some(SplitInfo { count, found, missing });
    }
    if parts.is_empty() {
        parts.push((1, path.to_path_buf()));
    }

    let main = parse_file(&parts[0].1)?;
    let mut info = summarize(&main, file_name)?;
    for (_, p) in parts.iter().skip(1) {
        let extra = parse_file(p)?;
        add_tensors(&mut info, &extra)?;
    }
    info.split = split_info;
    Ok(info)
}

fn add_tensors(info: &mut GgufInfo, p: &Parsed) -> Result<(), GgufError> {
    for (i, bytes) in tensor_bytes(p)? {
        let t = &p.tensors[i];
        info.tensor_count += 1;
        info.parameter_count = info.parameter_count.saturating_add(t.elements);
        info.weight_bytes += bytes;
        if t.name.contains("_exps") {
            info.expert_weight_bytes += bytes;
            info.expert_parameter_count = info.expert_parameter_count.saturating_add(t.elements);
        }
    }
    info.file_bytes += p.file_len;
    Ok(())
}

/// One recurrent layer's state for one sequence as llama.cpp keeps it (f32): the convolution state
/// `(kernel − 1) × (inner + 2 × groups × state)` plus the SSM state `state × inner`. None without
/// the `ssm.*` keys.
pub fn recurrent_state_bytes(conv_kernel: Option<u64>, inner: Option<u64>, groups: Option<u64>, state: Option<u64>) -> Option<u64> {
    let (k, inner, state) = (conv_kernel?, inner?, state?);
    let conv = k.saturating_sub(1) * (inner + 2 * groups.unwrap_or(1) * state);
    Some((conv + state * inner) * 4)
}

fn summarize(p: &Parsed, file_name: &str) -> Result<GgufInfo, GgufError> {
    let kv = &p.kv;
    let s = |k: &str| match kv.get(k) {
        Some(Val::S(s)) => Some(s.clone()),
        _ => None,
    };
    let architecture = s("general.architecture");
    let arch = architecture.clone().unwrap_or_default();
    let a = |k: &str| kv.get(&format!("{arch}.{k}"));
    let au = |k: &str| a(k).and_then(Val::as_u64);

    let file_type = kv.get("general.file_type").and_then(Val::as_u64).map(|v| v as u32);
    let template = kv.get("tokenizer.chat_template");
    let sliding_window_pattern = match a("attention.sliding_window_pattern") {
        Some(Val::A(v)) => Some(v.iter().map(|x| x.as_u64().unwrap_or(0) != 0).collect()),
        _ => None,
    };

    let mut info = GgufInfo {
        version: p.version,
        name: s("general.name"),
        is_projector: file_name.to_ascii_lowercase().starts_with("mmproj") || arch == "clip",
        architecture,
        file_type,
        file_type_name: file_type.map_or("unbekannt", file_type_name).to_string(),
        size_label: s("general.size_label"),
        context_length: au("context_length"),
        block_count: au("block_count"),
        embedding_length: au("embedding_length"),
        head_count: per_layer(a("attention.head_count")),
        head_count_kv: per_layer(a("attention.head_count_kv")),
        key_length: au("attention.key_length"),
        value_length: au("attention.value_length"),
        key_length_swa: au("attention.key_length_swa"),
        value_length_swa: au("attention.value_length_swa"),
        sliding_window: au("attention.sliding_window"),
        sliding_window_pattern,
        shared_kv_layers: au("attention.shared_kv_layers"),
        full_attention_interval: au("full_attention_interval").filter(|&n| n > 1),
        recurrent_state_bytes: recurrent_state_bytes(au("ssm.conv_kernel"), au("ssm.inner_size"), au("ssm.group_count"), au("ssm.state_size")),
        expert_count: au("expert_count"),
        expert_used_count: au("expert_used_count"),
        has_chat_template: template.is_some(),
        chat_template_family: match template {
            Some(Val::S(t)) => Some(chat_template_family(t).to_string()),
            Some(_) => Some("unbekannt".to_string()),
            None => None,
        },
        tensor_count: 0,
        parameter_count: 0,
        weight_bytes: 0,
        expert_weight_bytes: 0,
        expert_parameter_count: 0,
        file_bytes: 0,
        split: None,
    };
    add_tensors(&mut info, p)?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Builder for synthetic GGUF files.
    #[derive(Default)]
    struct Gguf {
        kv: Vec<u8>,
        kv_count: u64,
        tensors: Vec<(String, Vec<u64>, u64)>,
        version: u32,
    }

    fn str_bytes(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u64).to_le_bytes());
        out.extend(s.as_bytes());
    }

    impl Gguf {
        fn new() -> Self {
            Gguf { version: 3, ..Default::default() }
        }
        fn key(&mut self, k: &str, ty: u32) -> &mut Vec<u8> {
            self.kv_count += 1;
            str_bytes(&mut self.kv, k);
            self.kv.extend(ty.to_le_bytes());
            &mut self.kv
        }
        fn s(mut self, k: &str, v: &str) -> Self {
            let b = self.key(k, 8);
            str_bytes(b, v);
            self
        }
        fn u32(mut self, k: &str, v: u32) -> Self {
            self.key(k, 4).extend(v.to_le_bytes());
            self
        }
        fn i32(mut self, k: &str, v: i32) -> Self {
            self.key(k, 5).extend(v.to_le_bytes());
            self
        }
        fn u64(mut self, k: &str, v: u64) -> Self {
            self.key(k, 10).extend(v.to_le_bytes());
            self
        }
        fn f32(mut self, k: &str, v: f32) -> Self {
            self.key(k, 6).extend(v.to_le_bytes());
            self
        }
        fn arr_i32(mut self, k: &str, v: &[i32]) -> Self {
            let b = self.key(k, 9);
            b.extend(5u32.to_le_bytes());
            b.extend((v.len() as u64).to_le_bytes());
            for x in v {
                b.extend(x.to_le_bytes());
            }
            self
        }
        fn arr_bool(mut self, k: &str, v: &[bool]) -> Self {
            let b = self.key(k, 9);
            b.extend(7u32.to_le_bytes());
            b.extend((v.len() as u64).to_le_bytes());
            b.extend(v.iter().map(|&x| x as u8));
            self
        }
        fn arr_str(mut self, k: &str, v: &[&str]) -> Self {
            let b = self.key(k, 9);
            b.extend(8u32.to_le_bytes());
            b.extend((v.len() as u64).to_le_bytes());
            for x in v {
                str_bytes(b, x);
            }
            self
        }
        fn tensor(mut self, name: &str, dims: &[u64], bytes: u64) -> Self {
            self.tensors.push((name.into(), dims.to_vec(), bytes));
            self
        }
        fn build(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend(b"GGUF");
            out.extend(self.version.to_le_bytes());
            out.extend((self.tensors.len() as u64).to_le_bytes());
            out.extend(self.kv_count.to_le_bytes());
            out.extend(&self.kv);
            let mut offset = 0u64;
            for (name, dims, bytes) in &self.tensors {
                str_bytes(&mut out, name);
                out.extend((dims.len() as u32).to_le_bytes());
                for d in dims {
                    out.extend(d.to_le_bytes());
                }
                out.extend(12u32.to_le_bytes());
                out.extend(offset.to_le_bytes());
                offset += bytes.div_ceil(32) * 32;
            }
            while out.len() % 32 != 0 {
                out.push(0);
            }
            let total: u64 = self.tensors.iter().map(|t| t.2.div_ceil(32) * 32).sum();
            out.resize(out.len() + total as usize, 0xAB);
            out
        }
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "rp-gguf-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn write(&self, name: &str, data: &[u8]) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, data).unwrap();
            p
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn gemma_like() -> Gguf {
        Gguf::new()
            .s("general.architecture", "gemma4")
            .s("general.name", "Mini Gemma")
            .u32("general.file_type", 15)
            .s("general.size_label", "2B")
            .u32("gemma4.context_length", 131072)
            .u32("gemma4.block_count", 4)
            .u32("gemma4.embedding_length", 256)
            .u32("gemma4.attention.head_count", 8)
            .arr_i32("gemma4.attention.head_count_kv", &[1, 1, 1, 2])
            .u32("gemma4.attention.key_length", 512)
            .u32("gemma4.attention.value_length", 512)
            .u32("gemma4.attention.key_length_swa", 256)
            .u32("gemma4.attention.value_length_swa", 256)
            .u32("gemma4.attention.sliding_window", 512)
            .arr_bool("gemma4.attention.sliding_window_pattern", &[true, true, true, false])
            .u32("gemma4.attention.shared_kv_layers", 2)
            .f32("gemma4.rope.freq_base", 10000.0)
            .arr_str("tokenizer.ggml.tokens", &["<pad>", "<bos>", "hello", "world"])
            .s("tokenizer.chat_template", "{{ '<start_of_turn>user\n' }}")
            .s("some.unknown.key", "ignored")
            .tensor("token_embd.weight", &[256, 1000], 128_000)
            .tensor("blk.0.attn_q.weight", &[256, 256], 40_010)
    }

    #[test]
    fn reads_dense_model() {
        let d = TempDir::new();
        let p = d.write("mini.gguf", &gemma_like().build());
        let i = read_info(&p).unwrap();
        assert_eq!(i.version, 3);
        assert_eq!(i.name.as_deref(), Some("Mini Gemma"));
        assert_eq!(i.architecture.as_deref(), Some("gemma4"));
        assert_eq!(i.file_type, Some(15));
        assert_eq!(i.file_type_name, "Q4_K_M");
        assert_eq!(i.size_label.as_deref(), Some("2B"));
        assert_eq!(i.context_length, Some(131072));
        assert_eq!(i.block_count, Some(4));
        assert_eq!(i.embedding_length, Some(256));
        assert_eq!(i.head_count, Some(PerLayer::One(8)));
        assert_eq!(i.head_count_kv, Some(PerLayer::PerLayer(vec![1, 1, 1, 2])));
        assert_eq!(i.head_count_kv.as_ref().unwrap().max(), 2);
        assert_eq!((i.key_length, i.value_length), (Some(512), Some(512)));
        assert_eq!((i.key_length_swa, i.value_length_swa), (Some(256), Some(256)));
        assert_eq!(i.sliding_window, Some(512));
        assert_eq!(i.sliding_window_pattern, Some(vec![true, true, true, false]));
        assert_eq!(i.shared_kv_layers, Some(2));
        assert!(i.has_chat_template);
        assert_eq!(i.chat_template_family.as_deref(), Some("gemma"));
        assert_eq!(i.tensor_count, 2);
        assert_eq!(i.parameter_count, 256 * 1000 + 256 * 256);
        // Offsets are 32-aligned: the last tensor (40010 bytes) runs to the padded file end.
        assert_eq!(i.weight_bytes, 128_000 + 40_032);
        assert_eq!(i.expert_weight_bytes, 0);
        assert_eq!(i.expert_count, None);
        assert!(!i.is_projector);
        assert!(i.split.is_none());
        assert_eq!(i.file_bytes, std::fs::metadata(&p).unwrap().len());
    }

    #[test]
    fn reads_moe_model_and_version_2() {
        let mut g = Gguf::new()
            .s("general.architecture", "qwen3moe")
            .i32("general.file_type", 7)
            .u64("qwen3moe.context_length", 40960)
            .u32("qwen3moe.attention.head_count_kv", 4)
            .u32("qwen3moe.expert_count", 128)
            .u32("qwen3moe.expert_used_count", 8)
            .s("tokenizer.chat_template", "<|im_start|>user")
            .tensor("blk.0.ffn_up_exps.weight", &[64, 32, 128], 64_000)
            .tensor("blk.0.ffn_gate_inp.weight", &[64, 128], 8_192);
        g.version = 2;
        let d = TempDir::new();
        let i = read_info(&d.write("moe.gguf", &g.build())).unwrap();
        assert_eq!(i.version, 2);
        assert_eq!(i.file_type_name, "Q8_0");
        assert_eq!(i.context_length, Some(40960));
        assert_eq!(i.head_count_kv, Some(PerLayer::One(4)));
        assert_eq!((i.expert_count, i.expert_used_count), (Some(128), Some(8)));
        assert_eq!(i.chat_template_family.as_deref(), Some("chatml"));
        assert_eq!(i.expert_weight_bytes, 64_000);
        assert_eq!(i.weight_bytes, 64_000 + 8_192);
        assert_eq!(i.parameter_count, 64 * 32 * 128 + 64 * 128);
        assert_eq!(i.expert_parameter_count, 64 * 32 * 128);
        assert_eq!(i.active_parameters(), 64 * 128 + 64 * 32 * 128 * 8 / 128);
        assert_eq!((i.full_attention_interval, i.recurrent_state_bytes), (None, None));
    }

    /// Qwen3.5-9B's keys (read from its GGUF header): every 4th layer has attention, the others keep
    /// a recurrent state of 3 × (4096 + 2 × 16 × 128) + 128 × 4096 floats.
    #[test]
    fn reads_hybrid_recurrent_keys() {
        let g = Gguf::new()
            .s("general.architecture", "qwen35")
            .u32("qwen35.block_count", 32)
            .u32("qwen35.full_attention_interval", 4)
            .u32("qwen35.ssm.conv_kernel", 4)
            .u32("qwen35.ssm.state_size", 128)
            .u32("qwen35.ssm.group_count", 16)
            .u32("qwen35.ssm.inner_size", 4096);
        let d = TempDir::new();
        let i = read_info(&d.write("hybrid.gguf", &g.build())).unwrap();
        assert_eq!(i.full_attention_interval, Some(4));
        assert_eq!(i.recurrent_state_bytes, Some((3 * (4096 + 2 * 16 * 128) + 128 * 4096) * 4));
        assert_eq!(recurrent_state_bytes(None, Some(1), None, Some(1)), None);
    }

    #[test]
    fn unknown_file_type_and_missing_template() {
        let g = Gguf::new().s("general.architecture", "llama").u32("general.file_type", 999);
        let d = TempDir::new();
        let i = read_info(&d.write("x.gguf", &g.build())).unwrap();
        assert_eq!(i.file_type_name, "unbekannt");
        assert!(!i.has_chat_template);
        assert_eq!(i.chat_template_family, None);
        assert_eq!(i.tensor_count, 0);
        let g = Gguf::new().s("general.architecture", "llama");
        let i = read_info(&d.write("y.gguf", &g.build())).unwrap();
        assert_eq!(i.file_type_name, "unbekannt");
    }

    #[test]
    fn detects_projector() {
        let g = Gguf::new().s("general.architecture", "clip").tensor("v.patch_embd.weight", &[16, 16], 512);
        let d = TempDir::new();
        let i = read_info(&d.write("mmproj-model-f16.gguf", &g.build())).unwrap();
        assert!(i.is_projector);
        let g = Gguf::new().s("general.architecture", "llama");
        assert!(read_info(&d.write("mmproj-x.gguf", &g.build())).unwrap().is_projector);
    }

    #[test]
    fn split_model_sums_parts() {
        let d = TempDir::new();
        let p1 = gemma_like().u32("split.count", 3).build();
        let p2 = Gguf::new().u32("split.no", 1).tensor("blk.1.ffn_down_exps.weight", &[10, 10], 3_200).build();
        let p3 = Gguf::new().u32("split.no", 2).tensor("output.weight", &[256, 10], 640).build();
        let a = d.write("big-00001-of-00003.gguf", &p1);
        d.write("big-00002-of-00003.gguf", &p2);
        d.write("big-00003-of-00003.gguf", &p3);
        let total = (p1.len() + p2.len() + p3.len()) as u64;
        // Any part works as entry point.
        for entry in [a.clone(), d.0.join("big-00003-of-00003.gguf")] {
            let i = read_info(&entry).unwrap();
            assert_eq!(i.name.as_deref(), Some("Mini Gemma"));
            assert_eq!(i.tensor_count, 4);
            assert_eq!(i.file_bytes, total);
            assert_eq!(i.expert_weight_bytes, 3_200);
            assert_eq!(i.expert_parameter_count, 100);
            assert_eq!(i.parameter_count, 256 * 1000 + 256 * 256 + 100 + 2560);
            assert_eq!(
                i.split,
                Some(SplitInfo { count: 3, found: vec![1, 2, 3], missing: vec![] })
            );
        }
        std::fs::remove_file(d.0.join("big-00002-of-00003.gguf")).unwrap();
        let i = read_info(&a).unwrap();
        assert_eq!(i.split.unwrap().missing, vec![2]);
        assert_eq!(i.tensor_count, 3);
    }

    #[test]
    fn split_name_parsing() {
        assert_eq!(split_parts("a-b-00002-of-00004.gguf"), Some(("a-b", 2, 4)));
        assert_eq!(split_parts("a-00005-of-00004.gguf"), None);
        assert_eq!(split_parts("a-2-of-4.gguf"), None);
        assert_eq!(split_parts("model.gguf"), None);
    }

    #[test]
    fn broken_files_give_errors() {
        let d = TempDir::new();
        let good = gemma_like().build();
        assert!(matches!(read_info(&d.write("a.gguf", b"GGUX\x03\0\0\0")), Err(GgufError::NotGguf)));
        assert!(matches!(read_info(&d.write("b.gguf", b"GG")), Err(GgufError::Truncated)));
        assert!(matches!(read_info(&d.write("c.gguf", b"")), Err(GgufError::Truncated)));
        let mut v1 = good.clone();
        v1[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(read_info(&d.write("d.gguf", &v1)), Err(GgufError::UnsupportedVersion(1))));
        let mut be = good.clone();
        be[4..8].copy_from_slice(&3u32.to_be_bytes());
        assert!(matches!(read_info(&d.write("e.gguf", &be)), Err(GgufError::BigEndian)));
        // Cut at every position in the header: never a panic, always an error.
        let header_end = good.len() - 168_032;
        for cut in (0..header_end).step_by(7) {
            assert!(read_info(&d.write("f.gguf", &good[..cut])).is_err(), "cut at {cut}");
        }
        // Cut inside the tensor data: the last tensor would start past the file end.
        assert!(matches!(
            read_info(&d.write("g.gguf", &good[..good.len() - 50_000])),
            Err(GgufError::Truncated)
        ));
        // Absurd counts must not allocate.
        let mut huge = good.clone();
        huge[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(read_info(&d.write("h.gguf", &huge)).is_err());
        // Unknown value type.
        let mut bad = Gguf::new().u32("k", 1).build();
        let ty_pos = 24 + 8 + 1;
        bad[ty_pos..ty_pos + 4].copy_from_slice(&77u32.to_le_bytes());
        assert!(matches!(read_info(&d.write("i.gguf", &bad)), Err(GgufError::Invalid(_))));
    }

    #[test]
    fn template_families() {
        assert_eq!(chat_template_family("<|start_header_id|>user"), "llama3");
        assert_eq!(chat_template_family("[INST] hi [/INST]"), "mistral");
        assert_eq!(chat_template_family("<|start|>user<|channel|>"), "harmony");
        assert_eq!(chat_template_family("{{ message }}"), "unbekannt");
    }

    /// Real files: set `RP_GGUF_SAMPLES` to a `;`-separated list of paths.
    #[test]
    #[ignore]
    fn real_samples() {
        let list = std::env::var("RP_GGUF_SAMPLES").expect("RP_GGUF_SAMPLES not set");
        for path in list.split(';').filter(|s| !s.trim().is_empty()) {
            let t = std::time::Instant::now();
            let i = read_info(Path::new(path.trim())).unwrap();
            println!("{} ({} ms)", Path::new(path).file_name().unwrap().to_string_lossy(), t.elapsed().as_millis());
            println!(
                "  name={:?} arch={:?} ft={:?}/{} size={:?} ctx={:?} blocks={:?} emb={:?} heads={:?} kv={:?}",
                i.name, i.architecture, i.file_type, i.file_type_name, i.size_label, i.context_length,
                i.block_count, i.embedding_length, i.head_count, i.head_count_kv
            );
            println!(
                "  k/v={:?}/{:?} swa k/v={:?}/{:?} window={:?} pattern={:?} shared_kv={:?} experts={:?}/{:?} full_every={:?} recurrent={:?}",
                i.key_length, i.value_length, i.key_length_swa, i.value_length_swa, i.sliding_window,
                i.sliding_window_pattern.as_ref().map(|p| p.iter().map(|&b| if b { 'S' } else { 'G' }).collect::<String>()),
                i.shared_kv_layers, i.expert_count, i.expert_used_count, i.full_attention_interval, i.recurrent_state_bytes
            );
            println!(
                "  template={} family={:?} tensors={} params={} weights={} experts={} file={} weights<=file:{}",
                i.has_chat_template, i.chat_template_family, i.tensor_count, i.parameter_count,
                i.weight_bytes, i.expert_weight_bytes, i.file_bytes, i.weight_bytes <= i.file_bytes
            );
            assert!(i.tensor_count > 0 && i.weight_bytes <= i.file_bytes);
        }
    }
}
