# Benchmarks (Phase 0, package 0.2) — 2026-11-12

## Setup
- GPU: AMD Radeon RX 7900 XTX 24 GB · Windows 11 · 32 GB RAM
- Model: TheDrummer Orion-26B-A4B v1.1 Q4_K_M (gemma4 MoE, 16.72 GiB, iSWA)
- Server flags (= the developer's daily setup): `-c 131072 -ngl 999 -fa on -ctk q8_0 -ctv q8_0 -np 1`
- ROCm runtime: TheRock ROCm 7.2 DLLs (`rocm_sdk_libraries_custom` 7.2.0.dev0) on `PATH`

## 1. Real-world speed via llama-server (what the app will see)
Fresh prompt each time (`cache_prompt: false`), 128 generated tokens. Tool: `bench/cache_sim.py --depth`.

| Context | Prefill speed | Prefill time | Generation |
|---:|---:|---:|---:|
| 1,767 tok | 2,075 tok/s | 0.9 s | **99.5 tok/s** |
| 13,821 tok | 3,226 tok/s | 4.3 s | **95.7 tok/s** |
| 58,055 tok | 1,836 tok/s | 31.6 s | **72.5 tok/s** |
| 117,108 tok | 1,166 tok/s | 100.4 s | **53.0 tok/s** |

**Key fact:** re-reading a ~117k chat from scratch costs **~100 s**. Losing the KV cache is the single biggest UX cost → see [cache.md](cache.md).

## 2. llama-bench (pp2048 / tg128, KV q8_0, 2 repetitions)
| Build | pp2048 | tg128 | pp2048 @ d8192 | tg128 @ d8192 |
|---|---:|---:|---:|---:|
| TheRock build b12283 (own) | 892 | **88.2** | 766 | 26.5 |
| Official `win-rocm-10.0` b12319 + TheRock 7.2 DLLs | 909 | 60.8 | 765 | 26.0 |

- `tg @ d8192` from llama-bench (~26 tok/s) does **not** match the server (95.7 tok/s at 13.8k) for this iSWA model → **Auto-Tune must measure through llama-server, not llama-bench.**
- The official build is ~31 % slower at generation here, most likely because it is compiled for ROCm 10 but ran on the ROCm 7.2 runtime (version mismatch).

## 3. Findings that change the plan
1. **Official Windows ROCm zip is not self-contained**: without a ROCm runtime on `PATH` it finds no GPU (`--list-devices` → none) and silently falls back to CPU (16.6 tok/s). The app must ship/download a **matching pair** of llama.cpp HIP build + ROCm runtime and must detect a CPU fallback and warn.
2. **Vulkan is a stability risk on this machine**: during the Vulkan llama-bench run the screen showed artifacts and the PC needed a hard reset (Kernel-Power 41, 19:35). The run was not repeated. Vulkan stays a fallback for GPUs without ROCm, with a warning, and must be tested on other machines in the beta.
3. **Windows GPU memory counters do not see HIP allocations** (total stayed at ~7.1 GB with Orion loaded). The VRAM guard must use llama.cpp's own device report (`--list-devices`: `24560 MiB, 24411 MiB free`).
4. Measured baseline confirms the plan targets: ≥ 95 tok/s up to ~14k context, 53 tok/s at ~117k.

Raw output: `bench/raw/` (not committed).

## Modell-Paket M0 (14.11.2026)

### M0.1 Baseline (current app settings)
Own ROCm build b12283, Orion-26B-A4B-v1.1 Q4_K_M, flags `-c 131072 -ngl 999 -fa on -ctk q8_0 -ctv q8_0 -np 1`. Speed from llama-server `timings`; tg = 128 tokens, warmup + 3 runs per depth (prompt cached, `cache_prompt: true`); pp = first read of the prompt. Filler = generated neutral narrative text.

**Measurement finding first:** HIP's free-memory query does **not** change on this machine when memory is allocated. `--list-devices` reported `24411 MiB free` before, during and after a run with ~18 GB loaded, and the server's own exit breakdown shows `free 24399` with `unaccounted -17989`. "free before − free during" is therefore always 0 and **cannot** be used. The Windows WDDM counters (`GPU Adapter Memory` / `GPU Process Memory`, Dedicated + Shared Usage) **do** see the allocations (this corrects finding 3 above), so they are used as the real usage below. Other apps (desktop, browser, chat, media apps) already held ~6.6 GB dedicated before each start, so the real free VRAM was ~17.9 GB, not 24.4 GB.

| Depth (prompt tokens) | pp (first read) | pp wall time | tg mean (min–max), 3 runs | cache_n on repeat |
|---:|---:|---:|---:|---:|
| 0 | – | – | **59.9** (59.7–60.1) tok/s | – |
| 8,991 | 739 tok/s | 12.2 s | **24.8** (24.75–24.85) tok/s | 9,000 |
| 31,999 | 650 tok/s | 49.4 s | **11.9** (11.75–11.94) tok/s | 32,009 |
| 119,995 | 395 tok/s | 304.1 s | **4.07** (4.03–4.09) tok/s | 120,006 |

| Memory | Value |
|---|---|
| Load time until `/health` 200 | 39.4 s |
| Log: ROCm0 model / KV / compute buffer | 17,119 / 1,519 (+12 rotation) / 705 MiB (host: 578 model, 152 compute) |
| Server exit breakdown "self" (ROCm0) | 19,355 MiB = 17,118 model + 1,531 context + 705 compute |
| `--list-devices` free before / during | 24,411 / 24,411 MiB (unusable, see above) |
| WDDM adapter dedicated before → during | 6,855 → 23,490 MiB (+16,635) |
| WDDM process (llama-server) dedicated + shared | 16,690 + 3,244 = 19,934 MiB (after 32k: 16,873 + 3,262) |

**Follow-up turn** (9k prompt + 301 new tokens): `cache_n` 9,001, `prompt_n` 301 → cache share **96.8 %** of the 9,302-token prompt; the new 301 tokens were read at 535 tok/s, tg 24.3 tok/s.

**Why everything is 2–4× slower than section 1:** ~3.2 GB of the server's allocation sits in **shared** (system) memory, because the GPU only had ~17.7 GB dedicated left while the config needs ~19.4 GB. WDDM silently pages the overflow to system RAM instead of failing, so generation drops from ~100 to ~60 tok/s at depth 0 and from ~96 to ~25 tok/s at 9k. After the server stopped, adapter dedicated was back at 6,784 MiB.

**Conclusion**
- With the other apps of a normal desktop session open, the current settings do **not** fit into dedicated VRAM; the result is not a crash but a 2–13× slowdown (4 tok/s and 5 min prefill at 120k), which a user would read as "the app is slow".
- The prefix cache works for this iSWA model: identical re-sends reuse all tokens, a follow-up turn reuses 96.8 %.
- The app's VRAM guard and the model package must read the WDDM counters (dedicated usage of the adapter, and shared usage of the llama-server process as an "overflow" alarm), not `--list-devices`.

### M0.2 llama-fit-params vs. real VRAM
Own ROCm build b12283, Orion-26B-A4B-v1.1 Q4_K_M, `-fa on -np 1 --fit on -fitt 1024`, no `-ngl`; ~6.6 GB dedicated VRAM held by other desktop apps (adapter dedicated before each run 6.5–6.8 GB). "fit-params" = `llama-fit-params -fitp on` (model + context + compute); "server" = the server's own `--fit` projection in its log. Real = WDDM `GPU Process Memory` dedicated + shared of the server (peak). tg = llama-server `timings`, 128 tokens at depth 0 (clean reference in section 1: ~100 tok/s). From c16k q8_0 on, runs used `-lm none` (no mmap) because mmap loading pushed free system RAM below the 3 GB safety floor; VRAM buffers are identical.

| Config | fit suggests | Predicted GPU (fit-params / server) MiB | Real process dedicated + shared MiB | Real − server prediction | Shared overflow | tg d0 |
|---|---|---:|---:|---:|---|---:|
| c16k f16 ub512 | no change, 31/31 layers | 18,289 / 17,899 | 16,931 + 1,435 = 18,366 | +467 | yes, 1.4 GB | 60.6 |
| c16k q8_0 ub512 | no change, 31/31 | 18,010 / 17,620 | 17,068 + 1,599 = 18,667 | +1,047 | yes, 1.6 GB | 67.9 |
| c64k f16 ub512 | no change, 31/31 | 19,297 / 18,907 | 17,054 + 2,949 = 20,003 | +1,096 | yes, 2.9 GB | 59.4 |
| c64k q8_0 ub512 | no change, 31/31 | 18,568 / 18,355 | 17,047 + 2,434 = 19,481 | +1,126 | yes, 2.4 GB | 57.9 |
| c128k f16 ub512 | no change (`-ngl -1`) | 20,641 / – | not measured | – | – | – |
| c128k q8_0 ub512 | no change (`-ngl -1`) | 19,354 / – | not measured | – | – | – |
| c64k q8_0 ub1024 | no change (`-ngl -1`) | 19,222 / – | not measured | – | – | – |
| c64k q8_0 ub2048 | no change (`-ngl -1`) | 20,532 / – | not measured | – | – | – |
| c128k q8_0, `-fitt 7680` | moves the MoE expert tensors of blocks 24–30 to CPU (`-ot …=CPU`), keeps `-c 131072` | (target ≤ 24,399 − 7,680 = 16,719) | not measured | – | – | – |

Not measured: the last five server runs were skipped because another `llama-server` (a different session, CPU-only) was running and the one-server rule applies; the predictions above come from `llama-fit-params` alone.

In every run the server's `--fit` reported "24399 MiB of free device memory … no changes needed" — the same HIP free value that never changes on this machine (M0.1) — while the adapter actually sat at 23.4–24.0 of 24.0 GB dedicated and 1.4–2.9 GB of the server spilled into shared memory.

**Conclusion**
- `llama-fit-params` is a good estimate of the server's **own** need: the real total (dedicated + shared) is 0.5–1.1 GB above the server projection (≈ +1.1 GB with q8_0 KV; HIP runtime overhead not counted by fit). It is usable for "how much does this config need" if the app adds ~1.2 GB.
- It is **not** a source for "does it fit": `--fit` takes its free memory from HIP, which reports 24,399 MiB free no matter what other apps hold, so `--fit on -fitt 1024` never changes anything here and every config already overflows into shared memory (tg 58–68 instead of ~100 tok/s).
- `-fitt` does change the result (with 7,680 MiB reserve fit offloads 7 blocks of experts to the CPU), so the app can steer fit by passing `-fitt = (WDDM adapter dedicated used by others) + ~1.2 GB margin`; whether that removes the shared overflow and what it costs in tg is still open (run not done).
- The "passt" display must therefore be: fit-params need + 1.2 GB ≤ 24,560 − WDDM adapter dedicated (measured before start), and after start the process shared counter must stay near 0.

#### M0.2 (rest) — the five skipped server runs
Same flags as above (`-lm none`). Conditions were worse than in the first half: other apps held **7.8–8.3 GB** adapter dedicated before each start (not 6.6), and other agents ran Rust/clang/Gradle builds (heavy CPU load, free RAM swinging between 2.5 and 11 GB; the scripts now wait for ≥ 8 GB free RAM before a start, because a load at 5 GB free hit the 3 GB safety floor once). The CPU-only test server on port 7871 ran the whole time. tg values are therefore lower than in the first half and only comparable within this table.

| Config | Server `--fit` | Log ROCm0 model / KV / compute MiB | Real process dedicated + shared MiB | Adapter dedicated before → peak | tg d0 | tg d9k |
|---|---|---|---:|---:|---:|---:|
| c128k q8_0 ub512 | no change | 17,119 / 1,519 / 705 | 15,267 + 5,249 = 20,516 | 8,272 → 23,552 | 35.3 | – |
| c128k f16 ub512 | no change | 17,119 / 2,860 / 273 | 15,385 + 6,029 = 21,414 | 8,185 → 23,552 | 33.8 | – |
| c64k q8_0 ub1024 | no change | 17,119 / 893 / 515 | 15,522 + 4,201 = 19,723 | 8,081 → 23,830 | 39.7 | – |
| c64k q8_0 ub2048 | no change | 17,119 / 999 / 840 | 15,735 + 4,601 = 20,336 | 7,960 → 23,628 | 42.7 | – |
| c128k q8_0, `--fit on -fitt 7680` | experts of blocks 24–30 → CPU (`-ot …=CPU`, `-ngl 31`) | 14,294 / 1,519 / 705 | 14,778 + 5,735 = 20,513 (peak 14,961 + 5,753) | 7,848 → 22,861 | 49.6 | 26.2 (pp 625 tok/s) |

Observations:
- The adapter dedicated counter tops out at **~23.5–23.8 GB** of 24 GB in every run; whatever does not fit there goes to shared. With 8 GB held by others the server only gets ~15.3 GB dedicated, so even c64k overflows by 4+ GB.
- f16 KV at 128k is the only config that came close to the 6 GB shared safety limit (6,029 MiB).
- With `-fitt 7680`, fit did move 2.8 GB of expert weights to the CPU (log: host 3,553 MiB = 3,402 model + 151 compute), but the process **shared** counter did not drop (5.7 GB). The CPU-side weights live in pinned host memory (`ROCm_Host` buffers), which WDDM also books as the GPU process's shared memory. Real overflow ≈ 5,735 − 3,553 ≈ 2.2 GB. So the overflow alarm must be `process shared − host buffers from the log`, not `process shared` alone.

**Conclusion**
- With ~8 GB used by other apps no config of this model fits fully in dedicated VRAM; the overflow (not the KV type or `-ub`) is what sets tg (34–43 tok/s at d0).
- `-fitt 7680` was too small here: fit targets 24,399 − 7,680 = 16.7 GB, but only ~15.5 GB were really left (7.8 GB others + ~1.1 GB HIP overhead fit does not count). It still gave the best tg (49.6 d0) of this table.
- The reserve has to be computed from the WDDM value at start time, which is what M0.2-R tests next.

#### M0.2-R Reserve rule `-fitt = WDDM adapter dedicated before start + 1024`
c=131072, KV q8_0, `-fa on -np 1 -lm none --fit on`. Three attempts; none reached a tg measurement.

| Attempt | Adapter dedicated before → `-fitt` | What fit chose | Result |
|---|---|---|---|
| 1 | 7,729 → 8,753 | 10 blocks of experts → CPU (device 15,403 MiB, host 4,680 MiB) | killed by the safety guard while the KV cache was created: free system RAM 1.5 GB (other agents' builds were running) |
| 2 | – | – | not started: free RAM stayed below the 11 GB pre-start limit for 5 min |
| 3 | 7,884 → 8,908 | experts of blocks 21–30 → CPU (ROCm0 model 13,167 MiB, `-ngl 31`) | server up after 17.7 s, then killed by the guard: process shared **6,862 MiB** (> 6 GB limit) with process dedicated only 13,651 MiB and adapter peak 21.7 GB, free RAM 2.6 GB |

**Conclusion**
- The rule does make fit move enough weights off the GPU (device need 15.4 GB vs. ~16 GB really free), and dedicated VRAM no longer fills up (adapter peak 21.7 of 24 GB).
- But every MiB fit moves to the CPU lands in pinned host memory that WDDM books as the process's *shared* GPU memory, and it costs the same amount of system RAM. With ~4.7 GB of experts on the host the shared counter passes 6 GB and free RAM drops to 1.5–2.6 GB while other programs build in parallel. On this 32 GB machine with this model, "fit with CPU experts at 128k" and "safe RAM headroom" do not go together.
- Consequences for the app: (1) the overflow alarm must use `process shared − host buffers from the log`; (2) before choosing a config with CPU experts the app must also check free system RAM (host part + ~3 GB); (3) the weights alone need 17.1 GB (+ ~1.1 GB HIP overhead), so with ~8 GB held by other apps even c16k cannot fit fully in dedicated VRAM; a small expert offload (M0.3: `--n-cpu-moe 4` at c16k) does fit and doubles tg, as long as enough RAM is free. M0.3 was therefore run on a c16k base instead of this config.

### M0.3 Tuning knobs (`-ub`, `-np`, `--n-cpu-moe`, `-t`)
Planned on top of the M0.2-R reserve-rule config, which could not run safely (see above). Run instead on the c16k config of M0.6: own build b12283, `-c 16384 -ctk q8_0 -ctv q8_0 -fa on -np 1 -lm none`, depth 8,991 tokens, warmup + 2 runs. Other apps 7.2–7.7 GB dedicated, parallel builds of other agents running. "Host" = pinned `ROCm_Host` buffers from the log; "overflow" = process shared − host.

| Variant | ROCm0 model / compute MiB | Host MiB | Process dedicated + shared MiB | ≈ Overflow | pp 9k tok/s | tg 9k (min–max) |
|---|---|---:|---:|---:|---:|---:|
| base `-ub 512`, `-t` default (= 24 = physical cores of 32 threads) | 17,119 / 161 | 617 | 15,859 + 2,821 | ~2.2 GB | 690 | 20.8 (20.8–20.9) |
| `-ub 1024` | 17,119 / 322 | 657 | 16,213 + 2,896 | ~2.2 GB | 677 | 20.8 (20.7–20.8) |
| `-t 24` (explicit) | 17,119 / 161 | 617 | 16,265 + 2,599 | ~2.0 GB | 672 | 21.6 (21.5–21.7) |
| `--n-cpu-moe 4` | **14,567** / 443 | 3,169 | 15,847 + 3,299 | **~0.1 GB** | **2,043** | **42.1** (37.6–46.6) |

**Conclusion**
- The only knob that counts is the one that removes the overflow: moving the experts of 4 layers (2.55 GB) to the CPU brings the device part under what is really free, and pp at 9k triples (690 → 2,043 tok/s) while tg doubles (20.8 → 42.1). The CPU-side experts cost far less than overflowed VRAM does.
- `-ub 1024` (+161 MiB compute) and `-t` (default is already 24 = physical cores) change nothing measurable; `-np 1` stays.
- So the app should not tune `-ub`/`-t`, but pick the smallest `--n-cpu-moe` (or let fit do it with a correct reserve) so that device need + ~1.1 GB ≤ 24,560 − WDDM dedicated used by others, and verify after start that `process shared − host` is ~0.

### M0-Rest – everyday setting without load (2026-11-16)
Own build b12283, `-ngl 999 -fa on -ctk q8_0 -ctv q8_0 -np 1 -lm none -c <ctx> --n-cpu-moe N`, warmup + 2 runs per depth. "Adapter before" = WDDM adapter dedicated right before start (other apps); "Host" = pinned `ROCm_Host` buffers (model + compute) from the log; "Overflow" = process shared − host. Process values are taken after the last finished depth.

| Context | N | Adapter before MiB | Process dedicated + shared MiB | Host MiB | ≈ Overflow MiB | pp 9k tok/s | tg d0 | tg d9k | tg deep |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 16k | 2 | 6,734 | 16,323 + 2,823 | 1,925 | ~900 | 631 | 60.5 (60.5–60.5) | 25.0 (25.0–25.0) | – |
| 16k | 3 | 6,950 | 15,998 + 3,148 | 2,580 | ~570 | 597 | 60.2 (60.0–60.5) | 29.6 (29.1–30.0) | – |
| 16k | **4** | 6,883 | 15,851 + 3,295 | 3,169 | **~126** | **2,117** | 57.6 (56.5–58.7) | **54.5** (53.7–55.3) | – |
| 32k | **4** | 6,708 | 16,021 + 3,311 | 3,185 | **~126** | 2,059 | 57.3 (56.3–58.4) | **55.4** (55.4–55.5) | d30k not run (left out of this run by mistake) |
| 64k | 4 | 6,698 | 15,920 + 3,971 | 3,217 | ~750 | 546 | 55.8 (55.4–56.1) | 29.0 (28.9–29.1) | d60k: server process ended without an error line during the prefill at 43,008 tokens (495 tok/s, 87 s in); guard not triggered (min free RAM 8.0 GB) – no value |
| 64k | **5** | **1,775** (other apps had freed ~5 GB just before) | 16,046 + 3,845 | 3,747 | **~98** | 2,023 | 59.0 (58.7–59.2) | **55.6** (55.4–55.8) | see next row |
| 64k | **5** (2nd run, deep) | 1,649 | 16,046 + 3,845 | 3,747 | ~98 | 2,055 | – | 57.5 (57.4–57.5) | **d60k (60,001 tok): pp 1,358 tok/s, tg 44.6 (44.6–44.7)**; follow-up 9k + 301 tok: `cache_n` 9,001, first token after **0.34 s** (948 tok/s) |
| 64k | 6 | 1,792 | 15,517 + 4,374 | 4,276 | ~98 | 1,964 | 55.6 (55.4–55.7) | 52.6 (51.4–53.8) | not run (time) |

Conditions: no builds of other agents during the measured runs (a first 16k N=2 attempt was killed by the RAM guard at 1.35 GB free RAM when an Android Gradle/clang build of another agent started; it was repeated after the build ended). Other apps held 6.7–7.0 GB dedicated for the 16k/32k/64k-N4 rows; before the 64k-N5 runs the owner's chat/media apps were closed and only ~1.7 GB was held. The process dedicated counter stayed at ~15.5–16.0 GB in every run, also with 22 GB free, so the overflow depends mainly on the device part of the config (ROCm0 model + KV + compute): ≤ ~15.5 GB → overflow ~0.1 GB, 15.8 GB → 0.6 GB, 16.0 GB → 0.75–0.9 GB.

**Conclusion**
- Minimum `--n-cpu-moe` for overflow ~0: **16k → 4, 32k → 4, 64k → 5** (64k N=5 measured with only 1.7 GB held by other apps). Without overflow tg at 9k is 54–57 tok/s and pp ~2,000 tok/s at every context size; with 0.6–0.9 GB overflow it falls to 25–30 tok/s and pp to ~600 tok/s. Each extra CPU layer costs only ~5 % tg (64k N=5 → N=6: 55.6 → 52.6) and ~530 MiB of pinned system RAM.
- 64k is the largest context that meets the target (≥ 35 tok/s at 9k, overflow ~0) and it still gives 44.6 tok/s at 60k with 1,358 tok/s prefill; the follow-up turn starts after 0.34 s.
- Recommended everyday line (engine.json `args`) — 64k with **N=6**, one layer of margin, because N=5 was only verified with the chat/media apps closed and 64k-N=5's device part (15.5 GB) is ~170 MiB above 16k-N=4, which already showed ~126 MiB overflow with ~6.9 GB held by others; the margin costs ~5 % tg (52.6 tok/s at 9k) and 4.3 GB pinned RAM:

  `-c 65536 -ngl 999 -fa on -ctk q8_0 -ctv q8_0 -np 1 -lm none --n-cpu-moe 6 --slot-save-path <slot folder>`

- Open: 64k N=5/N=6 with the usual ~6.7 GB of other apps open (overflow there not measured); d30k at 32k; why the 64k-N=4 server ended silently at 43k tokens of prefill (not reproduced with N=5, which ran 60k fine). `-lm none` is kept from the measurements (mmap loading pushed free RAM below the guard in M0.2).

### M0.4 Prompt cache on a follow-up turn

### M0.4 Prompt cache on a follow-up turn
Own build b12283, `-c 20480 -ctk q8_0 -ctv q8_0 -fa on -np 1 -lm none -ub 512`, all layers on GPU (overflows ~2.6–2.8 GB into shared as in M0.2). History = 15,993 tokens, read once, then 1 × 128 tokens tg on it, then the same prompt + 301 new tokens with 1 predicted token (latency = wall time until that token, i.e. the time a user waits for the reply to start). Parallel builds of other agents were running.

| Variant | KV buffer MiB (ROCm0) | Process dedicated + shared MiB | pp 16k (first read) | tg at 16k | Follow-up `cache_n` / `prompt_n` | Follow-up latency |
|---|---:|---:|---:|---:|---:|---:|
| standard (iSWA, SWA cache 1,536 cells) | 372 | 15,904 + 2,810 | 723 tok/s (22.2 s) | 17.0 | 16,004 / 301 (98.2 % reused) | **0.68 s** (301 tok at 467 tok/s) |
| `--swa-full` | 2,338 | 15,908 + 4,869 | 502 tok/s | – | – | – (killed by the RAM guard: free RAM 2.95 GB during tg) |
| `--cache-reuse 256` | 372 | 16,164 + 2,550 | 634 tok/s (25.3 s) | 16.6 | 16,004 / 301 (98.2 %) | 0.89 s (356 tok/s) |

**Conclusion**
- The default iSWA cache already reuses the whole history on an appended turn (same as M0.1); the follow-up reply starts after 0.7 s at 16k.
- `--swa-full` costs +2 GB of KV for a 20k context (6× the default), makes the overflow larger (pp −30 %) and brings no benefit for append-only prompts — do not use it.
- `--cache-reuse 256` gives the same `cache_n` here (it only helps when text in the middle changes, which layout B avoids); the slower numbers are within the noise of the parallel builds. Not needed as a default.

### M0.6 Download b12387
Both official llama.cpp release assets of build **b12387** were fetched from the GitHub release API (asset field `digest` = SHA-256) and unpacked into separate folders next to the existing engine folders (own engine folder per build, nothing overwritten). No executable was started; no GPU was used.

| Asset | Size (bytes) | SHA-256 vs. `digest` | Check files present |
|---|---:|---|---|
| `llama-b12387-bin-win-rocm-10.0-x64.zip` | 280,412,960 | ok | `llama-server.exe`, `llama-fit-params.exe`, `ggml-hip.dll` |
| `llama-b12387-bin-win-vulkan-x64.zip` | 38,119,681 | ok | `llama-server.exe`, `llama-fit-params.exe`, `ggml-vulkan.dll` |

Download took 5.2 s (rocm) and 0.6 s (vulkan). The zips were deleted after unpacking.

**DLL finding, rocm zip** (flat folder, no sub-folders): the ROCm-related files are `amdhip64_7.dll` (15.8 MB), `amd_comgr.dll` (116 MB), `rocm_kpack.dll` and `ggml-hip.dll` (1.06 GB). There is **no** `rocblas`, `hipblas` or `hipblaslt` DLL and no `rocblas/library` folder in the zip. Besides those: `ggml-base/ggml/ggml-rpc`, `libomp.dll`, 14 CPU-variant DLLs (`ggml-cpu-*`) and the `llama*`/`mtmd` DLLs. The size of `ggml-hip.dll` suggests the BLAS/kernel code is linked in or loaded through `rocm_kpack`; whether this build finds the GPU without a ROCm runtime on `PATH` (the b12319 zip did not, see section 3) was **not tested** here because the GPU was busy.

#### M0.6 GPU part — does b12387-rocm find the GPU?
`llama-server --list-devices` of the official b12387 rocm zip:

| PATH | Result |
|---|---|
| unchanged (no ROCm runtime) | `Available devices: (none)` — no error message, it silently falls back to CPU |
| ROCm runtime folder (the Python `_rocm_sdk_libraries_custom\bin`) first on PATH | `ROCm0: AMD Radeon RX 7900 XTX (24560 MiB, 24411 MiB free)` |

Short run, both builds with the ROCm runtime on PATH, same config (the reserve-rule config from M0.2-R could not run, see above): `-c 16384 -ctk q8_0 -ctv q8_0 -fa on -np 1 -lm none -ub 512`, all 31 layers on GPU, warmup + 2 runs per depth. Other apps 7.6–8.0 GB dedicated; parallel builds of other agents and the CPU test server were running (noisy, the two builds ran back to back).

| Build | Load s | ROCm0 model / KV / compute MiB | Process dedicated + shared MiB | tg d0 (min–max) | pp 9k tok/s | tg d9k (min–max) |
|---|---:|---|---:|---:|---:|---:|
| b12387 official | 12.0 | 17,119 / 329 / 161 | 15,859 + 2,806 | 41.5 (40.9–42.0) | 736 | 19.8 (18.5–21.1) |
| b12283 own | 10.7 | 17,119 / 329 / 161 | 15,859 + 2,803 | 39.2 (37.5–40.9) | 690 | 20.8 (20.8–20.9) |

**Conclusion**
- The official rocm zip works on this GPU **only** with a ROCm runtime on PATH; without it there is no error, just "(none)" and a CPU run. The app must check `--list-devices` for `ROCm0` after start and must ship or locate the runtime.
- With the runtime, b12387 allocates exactly the same buffers and is as fast as the own build within noise (tg d0 +6 %, d9k −5 %, pp +7 %). Nothing speaks against switching to the official build.

### M0.7 Download-Fortsetzung (Range)
Test file: `unsloth/gemma-4-E2B-it-GGUF` / `gemma-4-E2B-it-Q4_K_M.gguf` (3,106,738,272 bytes, public, not gated; smallest Gemma 4 E2B/E4B Instruct Q4_K_M from the allowed repos, the bartowski and lmstudio-community files are larger/equal class). Probe: Python `urllib`, redirects followed manually so that the headers are re-sent on every hop.

| Step | Result |
|---|---|
| Part 1: `GET .../resolve/main/<file>` | `302` from the Hugging Face host, then `200` from the CDN host (`Accept-Ranges: bytes`, ETag present). Connection closed on purpose after 932,184,064 bytes (30 %): 8.5 s, ~110 MB/s |
| Part 2: `Range: bytes=932184064-` + `If-Range: <ETag from part 1>` | `302`, then **`206 Partial Content`** from the CDN, `Content-Range: bytes 932184064-3106738271/3106738272`, same ETag. The redirect to the CDN **does respect** the Range header (it also works when the header is re-sent on the second hop). Remaining 2,174,554,208 bytes in 19.5 s, ~111 MB/s |
| Final size | 3,106,738,272 = expected `size` (ok) |
| SHA-256 | equals `lfs.oid` from the HF tree API (ok); `.part` renamed to `.gguf` |

Findings for the app downloader:
- Resume via `Range` + `If-Range` works against the HF CDN; a `206` must still be checked explicitly (on a `200` the file has to be restarted from zero instead of appended).
- The ETag the CDN returns is **not** the file's SHA-256, so it is only usable for `If-Range`, not for verification. Verify with `lfs.oid` from `/api/models/<repo>/tree/main?recursive=true`.
- The HF host answers the first request with a redirect, so a client must keep the `Range` header across redirects (and must not forward `Authorization` to the CDN host for private repos).
- Xet storage (15.11.2026, `llmfan46/Mistral-Small-3.2-24B-Instruct-2506-ultra-uncensored-heretic-GGUF`, Q5_K_M, 16,763,997,344 bytes): the HF host answers `302` with its own `Content-Length: 1300` plus `X-Linked-Size: 16763997344` and `X-Linked-ETag: "<sha256 = lfs.oid>"`; the CDN (`us.aws.cdn.hf.co/xet-bridge-us/…`) answers `206`, `Content-Range: bytes 0-1023/16763997344`, ETag = Xet hash (not the SHA-256). A `Range` request at an uncached offset took **221 s** (`bytes=7375-`) and 78 s (`bytes=8000000000-`) until the CDN sent its headers; the old 60 s read timeout broke such downloads after the first ~1 KB and the exhausted resumes were reported as a size mismatch. The download client now waits 300 s and reports a stall as a network error.
- Hashing 3.1 GB took ~3 s from disk cache; the check is cheap enough to run after every download.

## P6 benchmark on the dev PC (2026-11-15)
The app's own benchmark (`POST /api/models/profiles/{id}/benchmark`, `crates/server/src/bench.rs` + `rp_core::bench`) on the owner's model, once, with a test server (`rp-server --manage-engine`, own data folder) and the own ROCm build b12283 (HIP runtime first on PATH). Profile before the run = the M0-Rest everyday line: `-c 65536 -ngl 999 -fa on -ctk q8_0 -ctv q8_0 -np 1 -lm none --n-cpu-moe 6`. Default run parameters: context steps 8k/16k/32k/64k/128k (capped at 128k, the model trains 256k) × KV q8_0/f16 × `--n-cpu-moe` (searched from the estimate, here never needed), warmup + 2 runs of a 1,984-token prompt with 64 new tokens, `/completion` `timings`, budget 5 min. Conditions: night, no other GPU work; the WDDM adapter counters read 0–0.2 GB dedicated held by other programs, free RAM ≥ 19 GB during the run.

| Context | KV | MoE on CPU | pp tok/s | tg tok/s | Overflow MiB | Load s | Fits |
|---:|---|---:|---:|---:|---:|---:|---|
| 8k | q8_0 | 0 | 3,600 | 100.8 | 97 | 5.2 | yes |
| 16k | q8_0 | 0 | 3,620 | 100.5 | 97 | 4.9 | yes |
| 32k | q8_0 | 0 | 3,607 | 101.0 | 97 | 4.9 | yes |
| 64k | q8_0 | 0 | 3,578 | 100.3 | 97 | 4.9 | yes |
| 128k | q8_0 | 0 | 3,574 | 101.3 | 97 | 5.2 | yes |
| 8k | f16 | 0 | 3,301 | 102.3 | 96 | 5.2 | yes |
| 16k | f16 | 0 | 3,284 | 102.4 | 96 | 5.2 | yes |
| 32k | f16 | 0 | 3,298 | 101.8 | 96 | 4.9 | yes |
| 64k | f16 | 0 | 3,306 | 102.6 | 96 | 4.9 | yes |
| 128k | f16 | 0 | 3,307 | 102.6 | 96 | 5.2 | yes |

- **Duration 95 s** (10 candidates, ~10 s each: the model file was in the file cache, so a load took ~5 s); the previously active profile was ready again 102 s after the start.
- **Recommendations:** Tempo, Ausgewogen and Kontext all point to **128k f16, `--n-cpu-moe` 0** (102.6 tok/s; every row is within 2 % of the best, so the largest context wins all three).
- Check of the applied Tempo setting at depth (prompt 10,855 tokens, warmup + 2 runs): pp 3,511–3,531 tok/s, **tg 95.7–98.5 tok/s** — above the M0-Rest values (64k N=5/6: ~53–56 tok/s at 9k), because nothing else held VRAM this time (M0-Rest: 1.7–7 GB held by other programs). The result is only valid for the memory situation it was measured in; with the usual desktop apps open, 128k f16 (device part ~20.3 GB) will overflow again, and the benchmark has to be repeated then (it then raises `--n-cpu-moe` or stops at a smaller step).
- Overflow ≈ 96–97 MiB in every row = process shared minus the pinned host buffers named in the server log (`ROCm_Host` model 577.5 + output 1 + compute 151.5 MiB with `-lm none`). **Finding of the first attempt:** without `-lv 4` the server log names no buffers, so the host part could not be subtracted and every candidate looked like overflow (it ended after 3 tries at 8k, 32 s, nothing stored, profile unchanged). The benchmark now adds `-lv 4` to its own runs (never to the profile) and falls back to the estimate's CPU part when the log has no buffer lines.
- Odd counter state: during this run the WDDM **dedicated** counters (adapter and process) read ~0 while **shared** worked (`Get-Counter` at 06:57: adapter 0 MiB dedicated / 4,440 MiB shared, llama-server 11 / 4,355 MiB; nobody had used the PC since 02:24, the cause inside Windows or the driver is unknown). Not a bug in `crates/hw`: `Get-Counter` and the kernel's own statistics (`D3DKMTQueryStatistics`: adapter dedicated = resident bytes, process dedicated = committed bytes of the local segments, `cargo run -p hw --example vram_probe`) are the same source and read the same; by day all three agree (07:25: 15,879 / 15,879 / 15,860 MiB). Since then `hw` treats a dedicated value below 512 MiB that is smaller than the shared one as unknown (`VramUsage::adapter_dedicated`), so the estimate says "unknown" instead of "the whole card is free". tg ~100 tok/s (the clean reference of section 1) confirms that nothing spilled.

## X5 Honest benchmark results (2026-11-16)
Owner's report (24 GB AMD, 32 GB RAM, browser/Discord/Steam open): "128k q8_0" and "8k f16" ended with "Abbruch: wenig RAM", the best was 64k at 44 tok/s, while other measurements on the same PC reached 128k at 60–100 tok/s; and 8k = 42 tok/s < 32k = 64 tok/s (noise).

What the code did and what changed:
- **RAM value.** `crates/hw` already reads *available* RAM (`GetPerformanceInfo` `PhysicalAvailable` = free + zero + standby, the same as `ullAvailPhys`), not *free*. Check on the dev PC with other agents' builds running: free + zero 0.00 GiB, standby 5.99 GiB, available 5.99 GiB, `hw` 5.2 GiB (read a moment later). So the guard did not count standby cache as used.
- **Guard.** It stopped every candidate at < 3 GB available, however little the candidate itself keeps in RAM. A candidate all on the GPU loses available RAM while it loads (file cache of the model file that Windows can drop), so it could be stopped as "wenig RAM" for memory it never needed. Now (`rp_core::bench::guard` / `skip_before_start`): the floor is half the candidate's estimated CPU part, 1–3 GB; in addition the commit limit must keep 2 GB (commit exhaustion is what froze the PC, mapped file pages do not count against it); a start is skipped when CPU part + floor > available or CPU part + 2 GB > commit left. Every stop or skip carries `limit {what: ram|commit|shared, available_mib, needed_mib}`.
- **Runs.** Before: warmup with 8 new tokens, then 2 runs, mean. Now: warmup of the same size, then ≥ 3 runs, median; while `(max − min) / median` of tg > 10 % up to 6 runs, as long as the candidate stays inside its share of the budget (budget / planned candidates); otherwise the row is marked `uncertain`. The row keeps `pp_runs`, `tg_runs`, `spread`.
- **After a stop.** The next candidate is read after the available RAM stopped rising (≤ 5 s), not right after the process ended. A first KV type that only crashed (no step reached, not for memory) no longer keeps the second one from running; one that ran out of memory at its first step still does (the second needs more).
- **Conditions.** Each result has `conditions {ram_available_mib, vram_free_mib, commit_free_mib}` read after the active model was stopped. `bench_results_<id>` keeps up to 4 results, one per kind of conditions (different: > 3 GB available RAM or > 1 GB free VRAM apart); a new result replaces only the one measured under similar conditions. `bench_result_<id>` stays the newest one (what "Setzen" uses). `GET …/benchmark` adds `results`.

CPU check (gemma-4-E2B Q4_K_M, `-ngl 0`, test server, `RP_BENCH_CTX=2048,4096`, 256-token prompt, 16 new tokens, budget 120 s; other agents' builds ran on the PC): 34.9 s, conditions 16,187 MiB available RAM / 20,057 MiB free VRAM / 65,196 MiB commit left.

| Context | KV | pp (median) | tg (median) | tg runs | Spread | Uncertain |
|---:|---|---:|---:|---|---:|---|
| 2k | q8_0 | 878.7 | 26.6 | 27.5, 26.6, 25.3 | 8.1 % | no |
| 4k | q8_0 | 920.7 | 28.5 | 20.0, 30.9, 33.0, 25.0, 29.5, 27.5 | 45.7 % | yes |
| 2k | f16 | 900.6 | 30.5 | 31.5, 29.0, 30.5 | 8.3 % | no |
| 4k | f16 | 881.3 | 28.4 | 25.4, 27.2, 28.2, 30.6, 31.4, 28.5 | 21.4 % | yes |

The two noisy rows got the extra runs and are marked uncertain instead of passing as exact numbers. Open: whether 128k now runs with the apps open, and the spread on the GPU, can only be seen in a GPU run on the owner's PC.

## X8 Picks and benchmark candidates for every PC (2026-11-16)
The model picks, the fit pill and the benchmark's start options were tuned to the dev PC (24 GB AMD, 32 GB RAM). Reported bug: with 12 GB the three columns of "Bestes Modell für deinen PC" showed the same three models. Cause: all three columns took their top 3 from the same small pool (only models entirely in VRAM, by "file + 1.5 GB"), and on 12 GB quality, active parameters and VRAM left sort that pool the same way; nothing kept a model out of a second column.

What the logic does now (`crates/server/src/fetch/pick.rs`, pure: PC profile in, picks out; the live scan only fills the profile):
- **Profile**: VRAM, RAM, GPU vendor, suggested backend (`pc.vendor`, `pc.backend` in `GET /models/catalog`). VRAM budget = card − 1.5 GB (Windows, other programs) − 0.5 GB more with ROCm (M0.2: real use 0.5–1.1 GB above llama.cpp's projection); 0 with the CPU build. A RAM part fits when it is at most half the RAM and the benchmark would start it too: with 6 GB taken by Windows, a browser and the app, it leaves its floor (1–3 GB, `bench::ram_floor`) of the rest (`Pc::ram_fits`; decides on 16 GB PCs, from 32 GB on the half does).
- **Memory** per tier at 8k with KV q8_0 by `rp_core::vram::estimate` on the catalog's attention shape (`shapes` in `models.json`, read from the GGUF headers of the balanced files: layers, KV heads, key/value length, sliding window and pattern, shared KV layers, hybrid layers with their recurrent state, experts used/total; `expert_share` per MoE model). The KV cache per token follows the KV type and the context instead of a flat 1.5 GB. Check against the dev PC: Orion-26B-A4B at 128k q8_0 → 1,519 MiB, the server log said 1,519 (M0.1). Qwen3.5/3.6 hold a KV cache in every 4th layer only (`full_attention_interval`, now also read from installed GGUF files); before, `vram::estimate` counted all layers.
- **Placement**: all on the GPU (`fits`); else a MoE model with the smallest `--n-cpu-moe` whose VRAM part fits, if its RAM part fits (`cpu_help`, `n_cpu_moe`, `gpu_gb`, `ram_gb`); a dense model with layers on the CPU only counts for the pill, never as a pick; without a GPU everything in RAM. `ctx_max` = largest step up to 128k (and the trained context) that still runs the same way.
- **Columns** take turns (Rollenspiel, Tempo, Kontext, …), each its best model not shown yet: no model is in two columns; a column without one is empty with `empty.<column>` = `nothing_fits` / `all_shown`. Rollenspiel = quality (0.6 rp + 0.4 de, ratings from `w7_ratings`), all-on-GPU before RAM help; Tempo = fewest bytes read per token (RAM bytes × 10), Kontext = largest `ctx_max`; both good ones (≥ 3.5, or within 0.5 of the best) first, then up to one point below, then the rest. Ties: installed, then measured. Tier: an installed one, then all on the GPU, then one that still holds 32k, then the biggest (with RAM help: balanced, small, large).

Picks on simulated PCs (NVIDIA/CUDA; first pick per column, `N` = `--n-cpu-moe`, test `picks_on_every_pc_profile` prints all 33 profiles):

| VRAM / RAM | Rollenspiel | Tempo | Kontext |
|---|---|---|---|
| 6 / 16 | Huihui Qwen3.5-4B | (all_shown) | (all_shown) |
| 6 / 32 | Orion 26B-A4B Q4_K_M, N29 | Gemma 4 26B-A4B heretic, N27 | SOMPOA heresy, N27 |
| 8 / 16 | Gemma 4 E4B heretic | Qwen3.5-9B heretic (small) | Gemma 4 E4B-it |
| 8 / 32 | Orion Q4_K_M, N25 | Gemma 4 26B-A4B heretic, N23 | SOMPOA heresy, N23 |
| 12 / 16 | Orion small, N11 | Gemma 4 26B-A4B heretic small, N9 | SOMPOA heresy small, N10 |
| 12 / 32 | Orion Q4_K_M, N16 | Gemma 4 26B-A4B heretic, N14 | SOMPOA heresy, N14 |
| 16 / 32 | Orion Q4_K_M, N8 | Gemma 4 26B-A4B heretic Q3_K_M, all on the GPU | Goetia, N10 |
| 24 / 32 | Orion Q5_K_M, all on GPU, 128k | Gemma 4 26B-A4B heretic Q5 | SOMPOA heresy Q5 |
| CPU / 16 | Gemma 4 E4B heretic | Gemma 4 E4B-it | Qwen3.5-9B heretic |
| CPU / 32 | Orion small | Gemma 4 26B-A4B heretic small | SOMPOA heresy small |

Test `every_pick_can_be_benchmarked`: on every simulated PC each pick has at least one benchmark candidate, with experts in RAM exactly when the pick has them.

**Benchmark candidates** (`rp_core::bench`): `Plan::fit_to` reads the memory with the active model stopped (`Room`: GPU or not, free VRAM, available RAM). When the first step does not fit entirely into the device, the search starts at 4k; a step is left out when it surely does not fit (`Room::start` None: even the best `--n-cpu-moe` leaves the estimate > 512 MiB over the free VRAM, or its RAM part plus the candidate's floor is over the available RAM). `next` asks the same rule for every step and KV type, so f16 never starts where it surely cannot fit. Simulated with the estimate standing in for the measurement: Orion Q4_K_M starts at 8k q8_0 N0 on 24 GB, at 4k q8_0 N24 on 8 GB/32 GB RAM, and has no candidate on 8 GB/16 GB RAM; a dense 24B has none on 8 GB. The shared-memory guard now allows the candidate's own pinned part plus 6 GB (a small GPU pins far more than 6 GB of experts on purpose); the RAM and commit guards stay as in X5. The result keeps the fitted `stages`.

**Backend suggestion** (`hw::backend_suggestion`, only a suggestion, nothing switches): NVIDIA → CUDA 13 (driver ≥ 580), CUDA 12 for a card before Turing (PCI id < 0x1E00) or a driver 525–579 or an unknown driver (was Vulkan); older drivers Vulkan. AMD → ROCm with a runtime, else Vulkan. Intel → Vulkan (reason names SYCL as the alternative). No GPU / other vendor → CPU.

Assumptions: 1.5 GB VRAM for other programs (the dev PC had 1.7–8 GB, the benchmark measures the real value); 1.2 GB buffers (`vram::RESERVE_BYTES`); ROCm +0.5 GB; RAM budget half the RAM, 6 GB RAM held by Windows and other programs; RAM ten times slower than VRAM per byte (order of Tempo only). Not measured on other PCs.

## R2 Benchmark on PCs without a GPU, results per model (2026-11-16)
Review findings H2, M4, M8 (model review of `a812b6a`).
- **Who computes on the GPU** (`system::computing_gpu`, used by the benchmark): a GPU with at least 2 GB dedicated memory (`hw::MIN_DEDICATED_BYTES`, as for the backend suggestion; `main_gpu` now skips integrated graphics and no longer keeps a failed DXGI read), an engine whose backend is not `cpu`, and `-ngl` not 0. Before, any adapter counted: with only integrated graphics the benchmark either found "Keine Einstellung passte" (free VRAM ≈ 0) or planned 8k–128k without a RAM check, and a candidate's RAM part was 0 (floor 1 GB). Now such a PC is a CPU run: every step is checked against the available RAM, a candidate's RAM part is weights + KV + compute buffers (`bench::cpu_bytes`), so the check before the start and the guard use its real floor (3 GB from 6 GB in RAM), `conditions.vram_free_mib` is null and `gpu` is null.
- **CPU runs** (`plan_for`, `Params::defaults`, `rp_core::bench::run_sizes`): three steps spread over the steps that fit (smallest, middle, largest: on the CPU the speed hardly changes with the context, what counts is how far the RAM reaches), runs of 512 + 32 tokens instead of 2,048 + 64. After the first warmup of a benchmark a slow PC gets shorter runs (down to 128 + 8) so that this candidate and two more fit into 90 % of the time left; a fast PC and every GPU keep their sizes. `RP_BENCH_*` win (sizes from the environment are never shortened). Simulated (test `a_slow_cpu_measures_three_candidates_in_the_budget`): 4–6 core CPU with a 12B model (pp 25, tg 3 tokens/s, 30 s load) 1 candidate before → 3 now (runs 209 + 13 tokens, ~276 s); dev PC on the GPU (P6) unchanged, 10 candidates at 2,048 + 64.
- **Result fields** (additional, older results still read): `measured_with {model_file, engine_id, profile_created_at}`, `prompt_tokens`, `gen_tokens`, `unmeasured_stages` (steps the time did not reach; empty unless time ran out).
- **Results per model** (`store/models.rs`): changing the model file or the engine of a profile deletes `bench_result_<id>` / `bench_results_<id>` and takes back "gemessen"; reading delivers only results whose `measured_with` matches the profile now (a benchmark of the old file that ends after the change, or a profile number used again after a delete). Results without `measured_with` (stored before) are still delivered.

Live check, CPU (gemma-4-E2B Q4_K_M, CPU build b12387, `-ngl 0`, test server port 8861 / model 8871, **no** `RP_BENCH_*`; other agents' builds and test browsers ran on the PC, 9.6–11.6 GB RAM available):

| Run | Duration | Candidates | Steps | Sizes | Spread of tg per row | Uncertain |
|---|---:|---:|---|---|---|---:|
| 1 | 141.1 s | 6 (all ok) | 8k / 32k / 128k × q8_0, f16 | 512 + 32 | 5.9 – 151 % | 5 of 6 |
| 2 | 108.9 s | 6 (all ok) | 8k / 32k / 128k × q8_0, f16 | 512 + 32 | 8.5 – 102 % | 4 of 6 |

Medians tg 19.3–31.0 tokens/s, pp 710–1,007; single runs dropped to 2–12 tokens/s while other processes took the CPU, which the extra runs and the `uncertain` mark caught. At most one llama-server at a time; `gpu` null, `vram_free_mib` null. Open: a PC with only integrated graphics, an NVIDIA PC, and a slow CPU were not available; their numbers are simulated, not measured.

## Y1 Settings for a purpose, Tempo without noise, 55 models (2026-11-16)

Nothing in this section was measured on a GPU; the rules are pure and covered by tests, the UI was run against the CPU model (`-ngl 0`).

**Tempo ignores differences within the noise (N2).** A live run had 16k f16 at 31.5 tokens/s and 128k q8_0 at 30.6 (3 % less, single runs spread up to 7 %); "Tempo setzen" cost 112k of context for nothing. `rp_core::bench::recommend` now takes, among the rows within the noise of the fastest (`SPEED_NOISE` 5 %, widened by the fastest row's own spread up to `SPREAD_LIMIT` 10 %), the largest context. With the live rows Tempo is 128k (`tempo_ignores_differences_within_the_noise`); a 10 % difference stays one.

**Settings for a purpose.** `rp_core::purpose` reads needs from one sentence (German or English, no model involved; 22 example sentences in its tests) and maps them:

| What | From | Rule |
|---|---|---|
| Context, KV cache, `--n-cpu-moe` | the last benchmark | long chats, groups, much context → the "Kontext" row (largest context at ≥ 15 tokens/s); fast or weak PC → "Tempo"; both, or nothing said → "Ausgewogen"; a run without the wanted row falls back to Ausgewogen, then Tempo |
| DRY (0.8 / 1.75 / 2) | derived | long chats and groups; 3 free tokens with three or more characters (names repeat); values a profile or the model's author already set stay |
| Reply length (`n_predict`) | derived | short replies 300; long replies or stories 1200; groups 800 + 150 per character above two (at most six); "fast" keeps 60 % of that, or 500 alone |
| Temperature, top-p/k, min-p, repetition penalty | the model's author | the catalog's `author_sampler`, when the profile's file is a catalog file |

`GET /models/profiles/{id}/advice` shows it split into measured and derived, `POST …/advice/apply` writes start options and sampler into the profile. The DRY numbers: multiplier 0.8 as the sampler's author suggests and model cards use (e.g. NemoMix Unleashed: "0.8/1.75/2/0 DRY"), base 1.75 and allowed length 2 are llama.cpp's own defaults. No reply-length or DRY value was tuned by measurement here; they are stated rules.

**Catalog.** 24 → 55 models, 2B to 70B (`crates/models/catalog/models.json` v3). Sizes and SHA-256 from the Hugging Face API, attention shapes from the first 400 kB of each balanced file's GGUF header; no model file was downloaded. Picks on the simulated PCs (`picks_on_every_pc_profile`): 6 GB / 16 GB shows six models of four base lines (one model before), from 12 GB up nine base lines on the page (`columns_show_different_base_models`, N14).
