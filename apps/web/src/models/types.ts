// What the server says about model downloads, pasted Hugging Face links, the catalog and the live
// search (`/api/models/downloads|resolve|catalog|search`, socket events "download" and "catalog").

export type DownloadState = "queued" | "running" | "paused" | "done" | "failed";

export type Download = {
  id: string;
  repo: string;
  /** The model file without ".gguf" and part suffix. */
  name: string;
  state: DownloadState;
  done: number;
  total: number;
  /** Bytes per second while it runs. */
  speed: number;
  error: string | null;
  created: number;
  folder: string;
  files: { path: string; size: number; finished: boolean }[];
};

/** `{"type":"download", …}`: a download's new state; `removed` when it left the list. */
export type DownloadEvent = { type: "download"; id: string; state: DownloadState | "removed" } & Partial<Omit<Download, "id" | "state">>;

export type Tier = "small" | "balanced" | "large";
export type Fit = "fits" | "cpu_help" | "too_big";

export type CatalogQuant = {
  file: string;
  quant: string;
  size_bytes: number;
  sha256: string;
  /** The file is gone from the repo (live check). */
  missing: boolean;
  est_gb: number;
  /** null without a PC reading. */
  fit: Fit | null;
  /** How this PC would run it at 8k: `--n-cpu-moe` (0: none), the VRAM and the RAM part (GB). Null without a PC reading. */
  n_cpu_moe?: number | null;
  /** `-ngl` the placement needs (a dense model with layers in RAM); null: no such limit. */
  n_gpu_layers?: number | null;
  gpu_gb?: number | null;
  ram_gb?: number | null;
  installed: boolean;
  /** A benchmark of this file on this PC, null: an estimate only. */
  measured: Measured | null;
};

/** What a benchmark measured: the fastest generation (tokens/s) and the largest context that fit. */
export type Measured = { tg: number; ctx: number };

/** `label` German, `label_en` shown in every other language. */
export type Source = { label: string; label_en: string; url: string };

export type CatalogModel = {
  id: string;
  name: string;
  family: string;
  /** Base line for the filter "Familie". */
  base: string;
  /** What the model is made for: `general` / `roleplay` / `story` / `adventure`. */
  focus?: string[];
  repo: string;
  source_repo: string | null;
  params_total_b: number | null;
  params_active_b: number | null;
  moe: boolean;
  arch: string | null;
  context_max: number | null;
  /** Key of the attention shape the memory estimate uses. */
  shape?: string;
  /** MoE only: share of the tensor bytes in expert tensors (what can move to RAM). */
  expert_share?: number | null;
  quants: Record<Tier, CatalogQuant | null>;
  vram_class: string;
  german: "good" | "medium" | "unknown";
  censorship: "standard" | "light" | "uncensored";
  /** Roleplay and German quality 1–5 (researched estimates). */
  rp: number;
  de: number;
  /** 0 refuses adult content, 1 mostly allows it, 2 refusal-removed. */
  uncensored: 0 | 1 | 2;
  /** Short German note on the writing style. */
  writing: string;
  /** The same note in the other languages. */
  writing_tr: { en: string; es: string; ru: string };
  confidence: "low" | "mid" | "high";
  /** Public pages the ratings come from. */
  sources: Source[];
  /** Quality for this app: 0.6 × rp + 0.4 × de, one decimal. */
  q: number;
  reasoning: boolean;
  template: string | null;
  license: string | null;
  gated: boolean;
  notes: string | null;
  /** Date of the last check (live, else the catalog's own). */
  checked: string;
  live: boolean;
};

/** Why a model is in a column of "Bestes Modell für deinen PC" (`Reason` in `fetch/pick.rs`):
 *  `fast_moe` / `small_dense` run all on the GPU, `measured_fast` is at least 20 tokens/s measured. */
export type Reason =
  | "best_quality"
  | "installed_quality"
  | "fast_moe"
  | "small_dense"
  | "measured_fast"
  | "measured_slow"
  | "partly_in_ram"
  | "cpu_moe"
  | "cpu_dense"
  | "room_for_context";

/** How "Benutzen" set up a new model on this PC (`placement` of `POST /models/local/use`). */
export type Placement = {
  fit: Fit;
  /** `--n-cpu-moe` written (0: none). */
  n_cpu_moe: number;
  /** `-ngl` written (null: the template's stays). */
  n_gpu_layers: number | null;
  ctx: number;
  gpu_gb: number;
  ram_gb: number;
  /** `catalog`: sized like the picks; `file`: from the file's own header. */
  source: string;
  /** `n_cpu_moe_unsupported`: the engine lacks `--n-cpu-moe`, whole layers stay in RAM instead. */
  fallback: string | null;
};

export type Pick = {
  id: string;
  tier: Tier;
  est_gb: number;
  q: number;
  installed: boolean;
  measured: Measured | null;
  reason_code: Reason;
  /** `fits`: all on the GPU; `cpu_help`: MoE experts in RAM, or a PC without a GPU. */
  fit?: Fit;
  /** `--n-cpu-moe` at 8k (0: none). */
  n_cpu_moe?: number;
  /** VRAM and RAM part at 8k, GB. */
  gpu_gb?: number;
  ram_gb?: number;
  /** The largest context step that still runs the same way. */
  ctx_max?: number;
};

export type Purpose = "roleplay" | "tempo" | "context";

/** One thing the user wants from a model (`Need` in `crates/core/src/purpose.rs`), in chip order. */
export type Need =
  | "long_chats"
  | "group"
  | "story"
  | "german"
  | "english"
  | "fast"
  | "context"
  | "uncensored"
  | "weak_pc"
  | "quality"
  | "short_replies"
  | "long_replies";

/** `GET|PUT /models/purpose`: the sentence, the needs in effect, what the sentence alone says and
 *  the words behind each of those. */
export type UsePurpose = {
  text: string;
  needs: Need[];
  /** How many characters the sentence names for a group. */
  figures: number | null;
  parsed: Need[];
  hits: { need: Need; phrase: string }[];
  all: Need[];
};

/** A reason or caveat behind a suggestion (`Why` in `fetch/suggest.rs`). */
export type Why = { code: string; value: number | null; points: number };

/** A catalog model placed for the stored purpose on this PC. */
export type Suggestion = {
  id: string;
  tier: Tier;
  score: number;
  q: number;
  installed: boolean;
  measured: Measured | null;
  fit: Fit;
  n_cpu_moe: number;
  gpu_gb: number;
  ram_gb: number;
  est_gb: number;
  ctx_max: number;
  reasons: Why[];
  caveats: Why[];
};

/** Why a column is empty: no catalog model fits this PC, or every one that fits stands in another column. */
export type EmptyReason = "nothing_fits" | "all_shown";

export type Catalog = {
  version: number;
  generated: string;
  source: string;
  syncing: boolean;
  /** Entries with live data. */
  live: number;
  /** `scan`: from the last "PC prüfen"; `live`: read now; `profile`: a simulated PC. */
  pc: {
    gpu: string | null;
    vram_bytes: number;
    ram_bytes: number;
    source: "scan" | "live" | "profile";
    /** `amd` / `nvidia` / `intel` / `other`; null without a GPU. */
    vendor?: string | null;
    /** The suggested llama.cpp build: `cuda` / `rocm` / `vulkan` / `cpu`; null: not known. */
    backend?: string | null;
  } | null;
  vram_budget_gb: number | null;
  best: (Record<Purpose, Pick[]> & { empty?: Partial<Record<Purpose, EmptyReason | null>> }) | null;
  /** The picks are limited to models with `uncensored ≥ 1` (`?uncensored=1`). */
  uncensored_only: boolean;
  /** The stored purpose and the catalog ordered for it: `ranked` is every model that runs on this
   *  PC, the best for the needs first; `top` the ids shown as the answer. Null without a PC reading. */
  purpose?: { text: string; needs: Need[]; figures: number | null };
  for_purpose?: { top: string[]; ranked: Suggestion[] } | null;
  quanters: string[];
  models: CatalogModel[];
};

export type RepoFile = { path: string; size: number; sha256: string | null };
export type Choice = { key: string; quant: string | null; tier: Tier | null; files: RepoFile[]; size: number };
export type TierGroup = { recommended: string | null; choices: Choice[] };
export type Resolved = {
  repo: string;
  /** The file the link pointed at, if it did. */
  file: string | null;
  license: string | null;
  last_modified: string | null;
  groups: Record<Tier, TierGroup>;
  other: Choice[];
  mmproj: RepoFile[];
};

export type SearchHit = {
  id: string;
  downloads: number;
  likes: number;
  last_modified: string | null;
  params: number | null;
  arch: string | null;
  context: number | null;
  curated: boolean;
};
