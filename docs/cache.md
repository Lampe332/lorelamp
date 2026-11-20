# KV-cache strategy (Phase 0, package 0.3) — 2026-11-12

Tool: `bench/cache_sim.py` · Server: Orion-26B-A4B (gemma4, **iSWA**), `-c 131072 -fa on -ctk/-ctv q8_0 -np 1`, default SWA settings (no `--swa-full`).
Simulation: 30 roleplay turns, 120 generated tokens per turn, chat history budget 12,000 tokens (history starts above budget, so trimming happens), 5 keyword lorebook entries, author's note every turn. Turn 1 excluded from the averages.

## Prompt layouts compared
- **A – SillyTavern-like:** triggered lorebook entries in the head (before the card), author's note 4 messages deep, history trimmed one message at a time.
- **B – Head / history / tail:** static head (system, card, examples), triggered lore + author's note appended only to the final user message, history trimmed in sticky blocks (~25 % of budget, first message pinned).
- **C – Append-only:** like B, but the tail notes stay in the history permanently.

## Results
| Layout | Cache hit rate | Recomputed tokens / turn (median) | Prefill time, 29 turns | Full re-reads in 30 turns |
|---|---:|---:|---:|---:|
| A | 13.4 % | 12,778 | 99.2 s | 28 |
| **B** | **95.1 %** | **251** | **8.4 s** | **1** |
| C | 88.4 % | 356 | 14.4 s | 3 |

Generation speed was the same for all (~95–98 tok/s). A 12k history is small; with 100k+ chats layout A means ~100 s waiting **per turn**, layout B ~0.2 s.

**Decision: layout B is the default.** Rules for the prompt builder:
1. Head never changes during a chat (no lorebook entries, no random/time macros resolved per turn).
2. Everything that changes per turn goes into the final user message (tail). Rolling back ~250 tokens works fine even with iSWA (no checkpoints needed in this range).
3. Trim the history in sticky blocks (cut ~25 % once, keep the cut position until the budget is exceeded again). Recomputing the per-turn-trimmed history is what kills layout A.
4. Gemma-style templates do not allow system messages mid-chat → dynamic notes are appended to the user message text.
5. Side requests (suggestions, impersonate, quick-fix) must share the same head + history and only differ in the tail (expected cost ≈ tail size; to be confirmed when built).

## Slot save / restore to disk (`--slot-save-path`)
| Step | Result |
|---|---|
| Fill slot | 63,441 tokens in 35.8 s |
| Save | 2.4 s · file **978 MB** (~15.4 MB per 1k tokens) |
| Erase + restore | 0.8 s |
| Next request | only **8 tokens** recomputed, 63,436 from cache |

- Works with iSWA **without** `--swa-full` → no extra VRAM needed.
- A full 128k chat ≈ 2 GB on disk. Plan default limit 20 GB ≈ 10 long chats (LRU eviction).
- Reopening a long chat: ~1–2 s instead of 36–100 s → plan target "100k chat < 5 s" is reachable.

## Not yet covered
- A model without SWA (e.g. dense Mistral/Llama) — expected to behave at least as well; check in Phase 1.
- Cache behaviour of suggestion/impersonate requests (Phase 2).
