//! Rough memory estimate of a model at a context size, from its GGUF metadata only: weights,
//! KV cache (per layer: heads × key/value length × tokens × bytes of the cache type, sliding-window
//! layers capped at their window, layers that share another layer's cache left out) and a fixed
//! reserve for compute buffers. Only a preview before the start ("ca." in the UI); the truth is
//! what llama-server allocates.

use crate::gguf::{GgufInfo, PerLayer};

const MIB: u64 = 1024 * 1024;
/// Compute buffers, CUDA/HIP context and the like (~1.2 GB).
pub const RESERVE_BYTES: u64 = 1200 * MIB;
/// Free system RAM that must stay free when part of the model runs on the CPU.
pub const RAM_KEEP_FREE_BYTES: u64 = 3 * 1024 * MIB;
/// llama.cpp's default micro-batch, which a sliding-window cache holds on top of its window.
const UBATCH: u64 = 512;

/// Bytes per element of a KV cache type (block quants with their scales); None: unknown type.
pub fn kv_type_bytes(t: &str) -> Option<f64> {
    Some(match t.to_ascii_lowercase().as_str() {
        "f32" => 4.0,
        "f16" | "bf16" => 2.0,
        "q8_0" => 34.0 / 32.0,
        "q4_0" | "iq4_nl" => 18.0 / 32.0,
        "q4_1" => 20.0 / 32.0,
        "q5_0" => 22.0 / 32.0,
        "q5_1" => 24.0 / 32.0,
        _ => return None,
    })
}

/// How the model is loaded (the load arguments that change the memory).
#[derive(Debug, Clone, PartialEq)]
pub struct Load {
    /// Context in tokens; 0: the trained one.
    pub ctx: u64,
    /// `-ctk` / `-ctv` (llama.cpp default f16).
    pub cache_k: String,
    pub cache_v: String,
    /// `-ngl`; None: all layers on the GPU.
    pub gpu_layers: Option<u64>,
    /// `--n-cpu-moe`: the experts of the first N layers stay in system RAM.
    pub n_cpu_moe: u64,
}

impl Default for Load {
    fn default() -> Self {
        Load { ctx: 0, cache_k: "f16".into(), cache_v: "f16".into(), gpu_layers: None, n_cpu_moe: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Estimate {
    pub ctx: u64,
    pub weights_gpu: u64,
    pub weights_cpu: u64,
    pub kv_gpu: u64,
    pub kv_cpu: u64,
    /// In VRAM: weights + KV on the GPU + [`RESERVE_BYTES`].
    pub gpu_bytes: u64,
    /// In system RAM: weights and KV of the layers on the CPU.
    pub cpu_bytes: u64,
}

/// The estimate; None when the file does not say how many layers it has.
pub fn estimate(info: &GgufInfo, load: &Load) -> Option<Estimate> {
    let layers = info.block_count.filter(|&n| n > 0)?;
    let ctx = if load.ctx == 0 { info.context_length.unwrap_or(4096) } else { load.ctx };
    let bk = kv_type_bytes(&load.cache_k).unwrap_or(2.0);
    let bv = kv_type_bytes(&load.cache_v).unwrap_or(2.0);
    // llama.cpp offloads the last `-ngl` layers.
    let on_gpu = |il: u64| il >= layers.saturating_sub(load.gpu_layers.unwrap_or(u64::MAX));
    let gpu_layers = (0..layers).filter(|&il| on_gpu(il)).count() as u64;

    let dense = info.weight_bytes.saturating_sub(info.expert_weight_bytes);
    let dense_gpu = (dense as u128 * gpu_layers as u128 / layers as u128) as u64;
    // Computed from the share of layers (not a rounded per-layer size), so that with every layer on
    // the GPU nothing is left over for the CPU.
    let expert_layers_gpu = (0..layers).filter(|&il| on_gpu(il) && il >= load.n_cpu_moe).count() as u64;
    let experts_gpu = (info.expert_weight_bytes as u128 * expert_layers_gpu as u128 / layers as u128) as u64;
    let weights_gpu = dense_gpu + experts_gpu;
    let weights_cpu = info.weight_bytes.saturating_sub(weights_gpu);

    let heads = |p: &Option<PerLayer>, il: u64| match p {
        Some(PerLayer::One(n)) => Some(*n),
        Some(PerLayer::PerLayer(v)) => v.get(il as usize).copied(),
        None => None,
    };
    let head_count = |il| heads(&info.head_count, il).unwrap_or(0);
    let default_len = |il| info.embedding_length.zip(Some(head_count(il)).filter(|&h| h > 0)).map(|(e, h)| e / h).unwrap_or(0);
    // The last `shared_kv_layers` layers reuse the cache of earlier ones.
    let own_cache = layers.saturating_sub(info.shared_kv_layers.unwrap_or(0));
    let window = info.sliding_window.unwrap_or(0);
    let (mut kv_gpu, mut kv_cpu) = (0f64, 0f64);
    for il in 0..own_cache {
        // A recurrent layer of a hybrid model keeps a fixed state instead of a KV cache.
        if info.full_attention_interval.is_some_and(|n| (il + 1) % n != 0) {
            let bytes = info.recurrent_state_bytes.unwrap_or(0) as f64;
            if on_gpu(il) {
                kv_gpu += bytes;
            } else {
                kv_cpu += bytes;
            }
            continue;
        }
        let swa = window > 0 && info.sliding_window_pattern.as_ref().and_then(|p| p.get(il as usize)).copied().unwrap_or(false);
        let kv_heads = heads(&info.head_count_kv, il).unwrap_or_else(|| head_count(il));
        let (k, v) = if swa {
            (info.key_length_swa.or(info.key_length), info.value_length_swa.or(info.value_length))
        } else {
            (info.key_length, info.value_length)
        };
        let (k, v) = (k.unwrap_or_else(|| default_len(il)), v.unwrap_or_else(|| default_len(il)));
        let tokens = if swa { ctx.min((window + UBATCH).div_ceil(256) * 256) } else { ctx };
        let bytes = kv_heads as f64 * tokens as f64 * (k as f64 * bk + v as f64 * bv);
        if on_gpu(il) {
            kv_gpu += bytes;
        } else {
            kv_cpu += bytes;
        }
    }
    let (kv_gpu, kv_cpu) = (kv_gpu.ceil() as u64, kv_cpu.ceil() as u64);
    Some(Estimate {
        ctx,
        weights_gpu,
        weights_cpu,
        kv_gpu,
        kv_cpu,
        gpu_bytes: weights_gpu + kv_gpu + RESERVE_BYTES,
        cpu_bytes: weights_cpu + kv_cpu,
    })
}

/// Does it fit: `fits` (all in free VRAM), `cpu_help` (part in system RAM: slower), `too_big`,
/// `unknown` (no VRAM figure).
pub fn fit(e: &Estimate, free_vram: Option<u64>, free_ram: Option<u64>) -> &'static str {
    let Some(free) = free_vram else { return "unknown" };
    let ram = free_ram.unwrap_or(0).saturating_sub(RAM_KEEP_FREE_BYTES);
    if e.gpu_bytes <= free {
        if e.cpu_bytes == 0 {
            "fits"
        } else if e.cpu_bytes <= ram {
            "cpu_help"
        } else {
            "too_big"
        }
    } else if e.gpu_bytes + e.cpu_bytes <= free + ram {
        "cpu_help"
    } else {
        "too_big"
    }
}

/// Context steps for a slider: 2k up to the trained context (which is always the last step).
pub fn ctx_steps(ctx_train: Option<u64>) -> Vec<u64> {
    const STEPS: [u64; 14] = [2048, 4096, 8192, 12288, 16384, 24576, 32768, 49152, 65536, 98304, 131072, 196608, 262144, 524288];
    let top = ctx_train.filter(|&c| c >= 2048).unwrap_or(131072);
    let mut out: Vec<u64> = STEPS.iter().copied().filter(|&s| s < top).collect();
    out.push(top);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * MIB;

    fn dense() -> GgufInfo {
        GgufInfo {
            version: 3,
            name: Some("Dense".into()),
            architecture: Some("llama".into()),
            file_type: Some(15),
            file_type_name: "Q4_K_M".into(),
            size_label: None,
            context_length: Some(32768),
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
            parameter_count: 8_000_000_000,
            weight_bytes: 4 * GIB,
            expert_weight_bytes: 0,
            expert_parameter_count: 0,
            file_bytes: 4 * GIB,
            split: None,
            is_projector: false,
        }
    }

    #[test]
    fn dense_kv_matches_the_llama_formula() {
        // Llama-3-8B shape: 32 layers × 8 KV heads × 128 × 2 (K+V) × 2 bytes = 128 KiB per token.
        let e = estimate(&dense(), &Load { ctx: 8192, ..Load::default() }).unwrap();
        assert_eq!(e.kv_gpu, 8192 * 128 * 1024);
        assert_eq!(e.kv_gpu, GIB);
        assert_eq!((e.weights_gpu, e.weights_cpu, e.kv_cpu, e.cpu_bytes), (4 * GIB, 0, 0, 0));
        assert_eq!(e.gpu_bytes, 5 * GIB + RESERVE_BYTES);
        // q8_0 holds 34 bytes per 32 values.
        let q8 = estimate(&dense(), &Load { ctx: 8192, cache_k: "q8_0".into(), cache_v: "q8_0".into(), ..Load::default() }).unwrap();
        assert_eq!(q8.kv_gpu, GIB * 34 / 64);
        // ctx 0 is the trained context.
        assert_eq!(estimate(&dense(), &Load::default()).unwrap().ctx, 32768);
    }

    #[test]
    fn gpu_layers_and_cpu_experts_move_memory_to_ram() {
        let half = estimate(&dense(), &Load { ctx: 8192, gpu_layers: Some(16), ..Load::default() }).unwrap();
        assert_eq!((half.weights_gpu, half.weights_cpu), (2 * GIB, 2 * GIB));
        assert_eq!((half.kv_gpu, half.kv_cpu), (GIB / 2, GIB / 2));
        let none = estimate(&dense(), &Load { ctx: 8192, gpu_layers: Some(0), ..Load::default() }).unwrap();
        assert_eq!((none.weights_gpu, none.kv_gpu, none.gpu_bytes), (0, 0, RESERVE_BYTES));

        let moe = GgufInfo { expert_count: Some(8), expert_used_count: Some(2), weight_bytes: 20 * GIB, expert_weight_bytes: 16 * GIB, ..dense() };
        let all = estimate(&moe, &Load { ctx: 8192, ..Load::default() }).unwrap();
        assert_eq!((all.weights_gpu, all.weights_cpu), (20 * GIB, 0));
        let ncmoe = estimate(&moe, &Load { ctx: 8192, n_cpu_moe: 8, ..Load::default() }).unwrap();
        assert_eq!((ncmoe.weights_gpu, ncmoe.weights_cpu), (16 * GIB, 4 * GIB), "a quarter of the expert layers stays on the CPU");
        assert_eq!(ncmoe.kv_cpu, 0, "the KV cache of those layers stays on the GPU");
    }

    #[test]
    fn sliding_window_and_shared_layers() {
        // Gemma-like: 4 layers, three of them SWA (window 512, smaller heads), the last one shares a cache.
        let g = GgufInfo {
            block_count: Some(4),
            head_count: Some(PerLayer::One(8)),
            head_count_kv: Some(PerLayer::PerLayer(vec![1, 1, 1, 2])),
            key_length: Some(512),
            value_length: Some(512),
            key_length_swa: Some(256),
            value_length_swa: Some(256),
            sliding_window: Some(512),
            sliding_window_pattern: Some(vec![true, true, false, true]),
            shared_kv_layers: Some(1),
            weight_bytes: GIB,
            ..dense()
        };
        let e = estimate(&g, &Load { ctx: 32768, ..Load::default() }).unwrap();
        let swa_tokens = 1024; // window + micro-batch, padded to 256
        let swa = 2 * swa_tokens * 256 * 2 * 2; // layers 0 and 1: 1 head × (K+V) × 2 bytes
        let full = 32768 * 512 * 2 * 2; // layer 2: 1 head; layer 3 has no cache of its own
        assert_eq!(e.kv_gpu, swa + full);
        // Below the window the SWA layers hold the context only.
        let small = estimate(&g, &Load { ctx: 512, ..Load::default() }).unwrap();
        assert_eq!(small.kv_gpu, 2 * 512 * (256 * 2 * 2) + 512 * (512 * 2 * 2));
        assert!(estimate(&GgufInfo { block_count: None, ..dense() }, &Load::default()).is_none());
    }

    /// Qwen3.5-like hybrid: only every 4th layer holds a KV cache, the others a fixed state.
    #[test]
    fn hybrid_layers_keep_a_fixed_state() {
        let state = 2 * MIB;
        let h = GgufInfo { full_attention_interval: Some(4), recurrent_state_bytes: Some(state), ..dense() };
        let e = estimate(&h, &Load { ctx: 8192, ..Load::default() }).unwrap();
        // 8 of 32 layers × 8 KV heads × 128 × 2 (K+V) × 2 bytes per token, plus 24 states.
        assert_eq!(e.kv_gpu, 8 * 8 * 128 * 2 * 2 * 8192 + 24 * state);
        let long = estimate(&h, &Load { ctx: 65536, ..Load::default() }).unwrap();
        assert_eq!(long.kv_gpu - e.kv_gpu, 8 * 8 * 128 * 2 * 2 * (65536 - 8192), "the states do not grow with the context");
    }

    #[test]
    fn fit_verdicts() {
        let e = Estimate { gpu_bytes: 10 * GIB, cpu_bytes: 0, ..Estimate::default() };
        assert_eq!(fit(&e, Some(12 * GIB), Some(16 * GIB)), "fits");
        assert_eq!(fit(&e, Some(8 * GIB), Some(16 * GIB)), "cpu_help");
        assert_eq!(fit(&e, Some(4 * GIB), Some(8 * GIB)), "too_big", "4 + (8 - 3) GB free is less than 10");
        assert_eq!(fit(&e, None, Some(16 * GIB)), "unknown");
        let split = Estimate { gpu_bytes: 10 * GIB, cpu_bytes: 4 * GIB, ..Estimate::default() };
        assert_eq!(fit(&split, Some(12 * GIB), Some(16 * GIB)), "cpu_help");
        assert_eq!(fit(&split, Some(12 * GIB), Some(5 * GIB)), "too_big");
    }

    #[test]
    fn steps_end_at_the_trained_context() {
        assert_eq!(ctx_steps(Some(32768)), vec![2048, 4096, 8192, 12288, 16384, 24576, 32768]);
        assert_eq!(*ctx_steps(Some(40960)).last().unwrap(), 40960);
        assert_eq!(*ctx_steps(None).last().unwrap(), 131072);
        assert_eq!(ctx_steps(Some(1024)), ctx_steps(None), "a nonsense value falls back to 128k");
    }
}
