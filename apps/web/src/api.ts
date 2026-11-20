// Types and helpers for the one HTTP + WebSocket API (same for the desktop window and phones).

import { dateFormat, keysUnder, num, numberFormat, t } from "./i18n";
import { bare, clientId, logEvent } from "./log";
import type { DownloadEvent } from "./models/types";


export type CharInfo = {
  id: number;
  name: string;
  blurb: string;
  /** The user's own short text under the card; null = the automatic `blurb`. */
  tagline: string | null;
  tags: string[];
  has_avatar: boolean;
  custom_avatar: boolean;
  avatar_rev: number;
  /** Which part of the picture shows where it is cropped: x and y in percent. */
  avatar_pos: [number, number];
  chats: number;
  /** New chats with this character are bound to this persona; null = they follow the active one. */
  default_persona_id: number | null;
  /** What applies when the card describes the user differently; null = as the chat's persona says. */
  user_priority: Priority | null;
  /** How the card's picture of the user fits the persona of new chats (default persona, else the active one). */
  user_check_status: Fit;
  /** Marked with a star in the library. */
  favorite: boolean;
  /** The user's own tags for ordering the library (not the card's `tags`). */
  own_tags: string[];
  /** Import time, Unix seconds. */
  created_at: number;
  /** When one of its chats last changed; null = no chat yet. */
  last_chat_at: number | null;
};

/** `GET /tags`: every own tag in use and how many characters carry it. */
export type TagCount = { name: string; count: number };

/** `GET /search/messages`: one message that matches. In `snippet` the matched words sit between
 *  `MARK_START` and `MARK_END`. */
export type SearchHit = {
  message_id: number;
  chat_id: number;
  character_id: number;
  character: string;
  has_avatar: boolean;
  avatar_rev: number;
  avatar_pos: [number, number];
  chat_title: string;
  chat_created_at: number;
  role: "user" | "assistant";
  created_at: number;
  snippet: string;
};
export const MARK_START = "\u0002";
export const MARK_END = "\u0003";

/** Who wins when a card describes the user differently than the persona. */
export type Priority = "persona" | "card";
/** ok = nothing differs; overlap = both describe the same thing; conflict = they contradict each other. */
export type Fit = "ok" | "overlap" | "conflict";
export type CheckKind = "role" | "appearance" | "personality" | "gender" | "name";
/** One place of the card about the user. `value`: "male"/"female" for gender, the name for name. */
export type CheckItem = {
  kind: CheckKind;
  field: string;
  excerpt: string;
  verdict: Fit;
  value?: string;
  persona_text?: string;
};
/** `GET /characters/{id}/user-check`: the card compared with one persona. */
export type UserCheck = { status: Fit; persona_id: number; items: CheckItem[] };

/** The editable fields of a character card. */
export type Card = {
  name: string;
  description: string;
  personality: string;
  scenario: string;
  first_mes: string;
  mes_example: string;
  system_prompt: string;
  post_history_instructions: string;
  creator_notes: string;
  alternate_greetings: string[];
  tags: string[];
};
export type Character = { info: CharInfo; card: Card };

/** One chat as the overviews show it. Times are Unix seconds. */
export type ChatInfo = {
  id: number;
  character_id: number;
  created_at: number;
  updated_at: number;
  messages: number;
  /** Start of the newest message. */
  last: string;
  /** This chat plays with its own version of the card. */
  custom_card: boolean;
  /** The name the user gave it; empty = unnamed. */
  title: string;
  /** The persona this chat is bound to; null = it follows the active persona. */
  persona_id: number | null;
  /** Who the user plays as here, as of this snapshot. A later switch of the active persona only sends
   *  "library", so for a free chat use `persona_id ?? active_persona_id` (see `chatPersonaId`). */
  effective_persona_id: number;
};

/** Who the user plays as in a chat right now. */
export const chatPersonaId = (c: ChatInfo, activeId: number | null) => c.persona_id ?? activeId ?? c.effective_persona_id;

/** What a chat is called in lists: its name, or its date while it has none. */
export const chatName = (c: ChatInfo) => {
  if (c.title) return c.title;
  const d = new Date(c.created_at * 1000);
  const time = dateFormat({ hour: "2-digit", minute: "2-digit" }).format(d);
  const days = daysAgo(d);
  if (days === 0) return t("shell.chatName.today", { time });
  if (days === 1) return t("shell.chatName.yesterday", { time });
  return t("shell.chatName.date", { date: when(c.created_at) });
};

/** A recently used chat plus what is needed to show its character. */
export type Recent = { chat: ChatInfo; name: string; has_avatar: boolean; avatar_rev: number; avatar_pos: [number, number] };
export type Timings = { tok_s: number; processed: number; cached: number; predicted: number };
/** Figures across the library: every message in a live chat, and the newest measured answer. */
export type Stats = { messages: number; last_timings: Timings | null };
/** `theme` and `home_layout`: ids from theme.ts, the same on every device; `language`: the app's
 *  language (`de`, `en`, `es`, `ru`), also the one the figures answer in. */
/** Who the user plays as: the active one's name is `{{user}}`, its description goes into the prompt. */
export type Persona = {
  id: number;
  name: string;
  /** Free text, read after the fields below. */
  description: string;
  /** Up to 40 characters, e.g. "sie/ihr"; empty = none. */
  pronouns: string;
  appearance: string;
  personality: string;
  background: string;
  /** false: in the fixed head of the prompt (fast). true: attached to the last own message on every turn. */
  every_turn: boolean;
  /** When a card describes the user differently: "persona" = the persona wins (default), "card" = the card does. */
  card_priority: Priority;
  has_avatar: boolean;
  avatar_rev: number;
  /** Which part of the picture shows where it is cropped: x and y in percent. */
  avatar_pos: [number, number];
  created_at: number;
  updated_at: number;
};
/** `user_name` is the active persona's name. */
export type Library = {
  characters: CharInfo[];
  recent: Recent[];
  user_name: string;
  theme: string;
  home_layout: string;
  language: string;
  /** Layout of the characters tab: "poster", "compact" or "list" (library/layout.ts). */
  library_layout: string;
  stats: Stats;
  /** All personas, sorted by name. */
  personas: Persona[];
  active_persona_id: number;
};
/** `persona_id`: who wrote one of your messages; null for the character's and for old messages.
 *  `swipes`: all variants of a reply (only with two or more, e.g. from SillyTavern), `swipe_id` the shown one. */
export type Msg = { id: number; role: "user" | "assistant"; content: string; timings?: Timings; error?: string; persona_id?: number | null; swipes?: string[]; swipe_id?: number; /** When it was written (Unix seconds). */ at?: number };

export type ServerEvent =
  | { type: "history"; chat_id: number; chat: ChatInfo; character: CharInfo; messages: Msg[]; busy: boolean; generating: number | null }
  | { type: "message"; chat_id: number; message: Msg }
  | { type: "assistant_start"; chat_id: number; id: number }
  | { type: "token"; chat_id: number; id: number; text: string }
  | {
      type: "done";
      chat_id: number;
      id: number;
      content: string;
      removed: boolean;
      stopped: boolean;
      timings: Timings | null;
      error: string | null;
      /** What was written: a new reply, another variant of the last one, or more text at its end.
       *  After the last two the chat's `history` follows. */
      mode?: "reply" | "variant" | "continue";
      /** The chat holds new text of the model. */
      kept?: boolean;
    }
  | { type: "library" }
  | { type: "notice"; code: string }
  /** Model profiles or engines changed (also which profile is active). */
  | { type: "models" }
  | { type: "engine"; event: EngineEvent; profile_id: number | null; port?: number | null }
  | { type: "import"; job: string; done: number; total: number; finished: boolean }
  | import("./app/engineDownload").EngineDownloadEvent
  /** Progress and end of a benchmark of a model profile. */
  | BenchEvent
  /** A model download changed (progress at most every 250 ms) or left the list. */
  | DownloadEvent
  /** The live check of the model catalog is done. */
  | { type: "catalog" };

/** What the app's model supervisor just did: stop for a switch, start, ready, crash, give up; a
 *  llama-server started elsewhere answers on the port (`foreign`), another program holds it (`port_busy`). */
export type EngineEvent = "starting" | "switching" | "ready" | "crashed" | "failed" | "waking" | "unloaded" | "foreign" | "port_busy";

/** Why no model server runs although one should: another program holds the port, or the
 *  supervisor gave up after `crashes` crashes (no further start until `POST /engine/restart`). */
export type EngineProblem = { code: "port_busy"; port: number } | { code: "gave_up"; crashes: number };

export type EngineStatus = "ready" | "loading" | "error" | "offline";

/** What the model file and the running server say; every field can be unknown. */
export type ModelInfo = {
  /** File name without the folder. */
  file: string | null;
  name: string | null;
  arch: string | null;
  quant: string | null;
  params_total: number | null;
  /** Parameters per token (MoE: the shared part plus the experts in use). */
  params_active: number | null;
  moe: boolean | null;
  swa: boolean | null;
  /** Context the model was trained for. */
  ctx_train: number | null;
  /** Context the server really uses (per slot). */
  ctx: number | null;
  /** llama.cpp build of the server. */
  build: string | null;
};

/** `GET /engine`. */
/** Why no model answers: a code (`shell.engineNote.<code>`), the system's own detail (an exit status,
 *  an error, a path) and the port; `gave_up`: no further start attempt. See `engineNoteText`. */
export type EngineNote = { code: string; detail?: string; port?: number; gave_up?: boolean };

export type EngineInfo = {
  status: EngineStatus;
  note: EngineNote | null;
  model: ModelInfo;
  /** The newest measured answer (the same as the library's). */
  last_timings: Timings | null;
  profile: { id: number; name: string; measured: boolean } | null;
  /** A benchmark of this profile holds the model server (it restarts with test settings). */
  bench_profile_id?: number | null;
  /** This server starts and supervises the model itself. */
  managed: boolean;
  /** Start options the build does not know; they were left out. */
  dropped_args: string[];
  /** Unloaded to free the graphics memory; the next message loads it again. */
  unloaded: boolean;
  /** Unload after this many quiet minutes; 0 = never. */
  idle_unload_min: number;
  /** Process id of the model server this app started, while it runs. */
  pid: number | null;
  /** The port the managed model server uses (setting `engine_port`). */
  port?: number;
  /** Why no model server runs (null while it is ready or nothing is wrong). */
  problem?: EngineProblem | null;
};

/** Sampler settings of a profile; a field left out uses the model server's default. */
export type Sampler = Partial<
  Record<
    | "temperature"
    | "top_p"
    | "top_k"
    | "min_p"
    | "typical_p"
    | "repeat_penalty"
    | "repeat_last_n"
    | "presence_penalty"
    | "frequency_penalty"
    | "dry_multiplier"
    | "dry_base"
    | "dry_allowed_length"
    | "dry_penalty_last_n"
    | "xtc_probability"
    | "xtc_threshold"
    | "n_predict"
    | "seed",
    number
  >
>;

/** A model profile: model file, start options, sampler and reasoning. Exactly one is active. */
export type Profile = {
  id: number;
  name: string;
  engine_id: number;
  model_file: string;
  /** llama-server arguments without model, host and port. */
  load_args: string[];
  sampler: Sampler;
  reasoning: "off" | "on";
  /** The start options come from a measurement, not from a guess. */
  measured: boolean;
  active: boolean;
  created_at: number;
  updated_at: number;
};

/** One line of a benchmark's table (`rp_core::bench::Row`). */
export type BenchRow = {
  ctx: number;
  kv: string;
  n_cpu_moe: number;
  /** Prompt reading and generation, tokens/s. */
  pp: number | null;
  tg: number | null;
  /** Shared GPU memory minus pinned host buffers, MiB; null: not known (no GPU counters). */
  overflow_mib: number | null;
  outcome: "ok" | "overflow" | "ram" | "shared" | "skipped" | "failed" | "time";
  load_s: number | null;
  /** Every measured run, tokens/s (missing in results stored before they were kept). */
  pp_runs?: number[];
  tg_runs?: number[];
  /** `(max − min) / median` of `tg_runs` (0.08 = 8 %). */
  spread?: number | null;
  /** The runs spread too far apart for the speed to be certain. */
  uncertain?: boolean;
  /** Why it was stopped or left out, with the numbers (`ram` outcome: available RAM or the commit limit). */
  limit?: BenchLimit | null;
};
/** A memory limit a candidate hit: `needed_mib` is more than `available_mib`; for `shared`,
 *  `available_mib` is the limit and `needed_mib` what the model server holds. */
export type BenchLimit = { what: "ram" | "commit" | "shared"; available_mib: number; needed_mib: number };
/** The memory when a benchmark started (null: not known). */
export type BenchConditions = { ram_available_mib: number | null; vram_free_mib: number | null; commit_free_mib: number | null };
export type BenchPurpose = "speed" | "balanced" | "context";
/** A stored benchmark result of a profile. */
export type BenchResult = {
  profile_id: number;
  at: number;
  duration_s: number;
  rows: BenchRow[];
  picks: { purpose: BenchPurpose; row: number }[];
  moe: boolean;
  /** The GPU the model computed on; null: a CPU run. */
  gpu: string | null;
  /** Missing in results stored before they were recorded. */
  conditions?: BenchConditions | null;
  /** Size of each run: tokens read and written (missing in older results). */
  prompt_tokens?: number;
  gen_tokens?: number;
  /** Context steps the time did not reach (empty unless time ran out; missing in older results). */
  unmeasured_stages?: number[];
};
/** `{"type":"bench",…}` over the socket: progress of a running benchmark, and its end. */
export type BenchEvent = {
  type: "bench";
  profile_id: number;
  stage: "loading" | "measuring" | "done" | "cancelled" | "failed";
  done: number;
  total: number;
  elapsed_s: number;
  budget_s: number;
  candidate: { ctx: number; kv: string; n_cpu_moe: number } | null;
  rows: BenchRow[];
  result?: BenchResult;
};

/** A llama-server build. `origin`: "own" (the user's) or "official" (a release the app installed). */
export type Engine = {
  id: number;
  name: string;
  exe: string;
  extra_path: string[];
  backend: string;
  version: string | null;
  origin: string;
  created_at: number;
  exists: boolean;
};

/** Where a found model lies: the app's model folder, LM Studio's (read only), a folder the user added. */
export type ModelOrigin = "app" | "lmstudio" | "extra";

/** A model file in the model folders (`GET /models/local`); a split model is one entry. */
export type LocalModel = {
  /** The file to start (a split model's first part). */
  path: string;
  file: string;
  dir: string;
  origin: ModelOrigin;
  root: string;
  size_bytes: number;
  parts: number;
  missing_parts: number[];
  /** Multimodal projectors next to it (file names). */
  mmproj: string[];
  name: string | null;
  arch: string | null;
  quant: string | null;
  params_total: number | null;
  params_active: number | null;
  moe: boolean | null;
  ctx_train: number | null;
  readable: boolean;
  profiles: { id: number; name: string; active: boolean }[];
  /** In the app's own folder: the app may delete it. */
  deletable: boolean;
};

export type ModelFolder = { path: string; exists: boolean };
export type ModelFolders = { app: ModelFolder & { default: boolean }; lmstudio: ModelFolder | null; extra: ModelFolder[] };
export type LocalModels = { folders: ModelFolders; models: LocalModel[] };

/** Does a model fit at one context: all in VRAM, partly in system RAM, not at all, unknown. */
export type MemoryFit = "fits" | "cpu_help" | "too_big" | "unknown";

/** `POST /models/estimate`: the memory a model needs ("ca.") at each context step. */
export type Estimate = {
  layers: number | null;
  moe: boolean;
  ctx_train: number | null;
  gpu: string | null;
  vram_bytes: number | null;
  /** Free for the model server now (WDDM counters, our own model server counted as free). */
  vram_free_bytes: number | null;
  ram_free_bytes: number | null;
  steps: { ctx: number; gpu_bytes: number; cpu_bytes: number; fit: MemoryFit }[];
};

/** What activating a profile did to the model server. */
export type ActivateEngine = "restarting" | "unchanged" | "unmanaged" | "other_window" | "foreign";
/** `ActivateEngine` plus `port_busy` (another program holds the model server's port: nothing starts
 *  until it is free or changed). Kept apart so places that list every answer by name still compile
 *  until they name this one too. */
export type ActivateAnswer = ActivateEngine | "port_busy";

/** Where the API lives and the token for it.
 *
 *  Default (the server delivers this page): same origin, token from the address (`?token=`), kept for
 *  the tab in sessionStorage. A UI that is not delivered by the server (later: the phone app) is
 *  pointed at one with `connectTo`, which keeps address and token together in localStorage. A token in
 *  the address always means "the server of this page", so a stored pair is never used with it. */
const SERVER_KEY = "rp-server";

type Server = { base: string; token: string };

/** Takes `token` out of the address bar (and so out of the history and bookmarks) once it is kept
 *  in sessionStorage: a reload takes it from there. Every other parameter (`shell`) and the hash
 *  route stay as they are. */
function dropTokenFromAddress() {
  const url = new URL(location.href);
  url.searchParams.delete("token");
  history.replaceState(history.state, "", url.pathname + url.search + url.hash);
}

const server = ((): Server => {
  const fromUrl = new URLSearchParams(location.search).get("token");
  try {
    if (fromUrl) {
      sessionStorage.setItem("rp-token", fromUrl);
      dropTokenFromAddress();
      return { base: "", token: fromUrl };
    }
    const saved = JSON.parse(localStorage.getItem(SERVER_KEY) ?? "null") as Partial<Server> | null;
    if (typeof saved?.base === "string" && typeof saved.token === "string" && saved.base) {
      return { base: saved.base.replace(/\/+$/, ""), token: saved.token };
    }
    return { base: "", token: sessionStorage.getItem("rp-token") ?? "" };
  } catch {
    return { base: "", token: fromUrl ?? "" };
  }
})();

export const token = server.token;

/** Full address of an API path ("/library" → "/api/library", or on the configured server). */
export const apiUrl = (path: string) => `${server.base}/api${path}`;

/** The socket's address, on the configured server or this page's. */
export const wsUrl = (path: string) => {
  const origin = server.base || `${location.protocol}//${location.host}`;
  return `${origin.replace(/^http/, "ws")}/api${path}`;
};

/** Points this UI at a server (e.g. `http://192.168.1.13:7860`) and loads it again; `null` goes back to
 *  the server that delivered the page. Used by the pairing of the phone app (phase 3). */
export function connectTo(target: { base: string; token: string } | null) {
  try {
    if (target) localStorage.setItem(SERVER_KEY, JSON.stringify({ base: new URL(target.base).origin, token: target.token }));
    else localStorage.removeItem(SERVER_KEY);
  } catch {
    return; // no storage: nothing can be kept, the current server stays
  }
  location.replace(location.pathname + location.hash);
}

/** Thrown by `api`; `code` is the server's error code (see `ERROR_TEXT`). */
export class ApiError extends Error {
  /** `status`: the HTTP status, 0 without an answer. */
  constructor(
    public code: string,
    public status = 0,
  ) {
    super(code);
  }
}

/** Hands a request to the activity log (method, path, status, time); `logEvent` keeps it only when it
 *  failed or took 1 s or more, quick successes are dropped. The log's own traffic and the status poll
 *  are left out here, they would only log themselves. */
function logRequest(method: string, path: string, status: number, started: number, code?: string) {
  const p = bare(path);
  if (p.startsWith("/logs") || p === "/engine") return;
  const level = status === 0 || status >= 500 ? "error" : status >= 400 ? "warn" : "debug";
  logEvent(level, "api", `${method} /api${p} → ${status || "keine Verbindung"}`, {
    data: { method, path: `/api${p}`, status, ms: Math.round(performance.now() - started), ...(code ? { error: code } : {}) },
  });
}

/** Sends a request with the token in the header; answers the response, or throws `ApiError` with the
 *  server's code. Failed and slow requests are logged (see `logRequest`). */
async function request(path: string, init: RequestInit): Promise<Response> {
  const method = (init.method ?? "GET").toUpperCase();
  const started = performance.now();
  let res: Response;
  try {
    res = await fetch(apiUrl(path), { ...init, headers: { Authorization: `Bearer ${token}`, "X-RP-Client": clientId, ...init.headers } });
  } catch {
    logRequest(method, path, 0, started, "offline");
    throw new ApiError("offline");
  }
  if (!res.ok) {
    const body = (await res.json().catch(() => null)) as { error?: string } | null;
    const code = body?.error ?? (res.status === 413 ? "too_large" : "unknown");
    logRequest(method, path, res.status, started, code);
    throw new ApiError(code, res.status);
  }
  logRequest(method, path, res.status, started);
  return res;
}

export async function api<T = void>(path: string, init: RequestInit = {}): Promise<T> {
  const res = await request(path, init);
  return (res.status === 204 ? undefined : await res.json()) as T;
}

/** The file name a download announces (`filename*=UTF-8''…` first, then `filename="…"`). */
function announcedName(disposition: string | null): string | null {
  if (!disposition) return null;
  const exact = /filename\*=UTF-8''([^;]+)/i.exec(disposition);
  if (exact) {
    try {
      return decodeURIComponent(exact[1].trim());
    } catch {
      // a broken encoding: fall back to the plain name
    }
  }
  return /filename="([^"]+)"/i.exec(disposition)?.[1] ?? null;
}

type SaveType = { description: string; accept: Record<string, string[]> };
type SavePicker = (options: { suggestedName: string; types?: SaveType[] }) => Promise<FileSystemFileHandle>;

/** The type filter of the save dialog, by the extension of the suggested name. */
const SAVE_TYPES: Record<string, SaveType> = {
  png: { description: "PNG-Bild", accept: { "image/png": [".png"] } },
  json: { description: "JSON-Datei", accept: { "application/json": [".json"] } },
  zip: { description: "ZIP-Archiv", accept: { "application/zip": [".zip"] } },
  txt: { description: "Textdatei", accept: { "text/plain": [".txt"] } },
  md: { description: "Markdown", accept: { "text/markdown": [".md"] } },
  jsonl: { description: "SillyTavern-Chat", accept: { "application/jsonl": [".jsonl"] } },
};

/**
 * Fetches a file from the API and saves it. The token travels in the header, never in an address, so
 * the file goes through the page instead of a plain link.
 *
 * Where the page may ask for a save dialog (`showSaveFilePicker`: the desktop window and desktop
 * browsers) the user picks the place first — the dialog needs the fresh click, so it opens before the
 * file is fetched. In the desktop window a plain download is saved silently into the Downloads folder
 * (the window takes over WebView2's downloads), which looked like nothing happened. Elsewhere (a phone
 * over Wi-Fi is no secure context) it falls back to the browser's download.
 *
 * Returns the saved file's name, or `null` when the user cancelled the dialog.
 */
export async function download(path: string, init: RequestInit = {}, suggestedName = "export"): Promise<string | null> {
  const pick = (window as unknown as { showSaveFilePicker?: SavePicker }).showSaveFilePicker;
  if (pick) {
    let handle: FileSystemFileHandle | null = null;
    try {
      const type = SAVE_TYPES[suggestedName.split(".").pop()?.toLowerCase() ?? ""];
      handle = await pick.call(window, { suggestedName, ...(type ? { types: [type] } : {}) });
    } catch (e) {
      if (e instanceof DOMException && e.name === "AbortError") return null;
      // Not allowed here (no fresh click, e.g. from Ctrl+K): the plain download below.
    }
    if (handle) {
      const blob = await (await request(path, init)).blob();
      const out = await handle.createWritable();
      try {
        await out.write(blob);
      } finally {
        await out.close();
      }
      return handle.name;
    }
  }
  const res = await request(path, init);
  const blob = await res.blob();
  const name = announcedName(res.headers.get("Content-Disposition")) ?? suggestedName;
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.hidden = true;
  document.body.append(a);
  a.click();
  a.remove();
  // Some browsers read the Blob only after the click returns.
  window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
  return name;
}

/** Export formats of a chat: Markdown, plain text, SillyTavern's chat file. */
export type ExportFormat = "md" | "txt" | "jsonl";

/** Minutes east of UTC here, so dates in exported files are this device's time. */
export const tzOffset = () => -new Date().getTimezoneOffset();

/** One entry of the activity log. Times are Unix milliseconds. */
export type LogEntry = {
  id: number;
  ts: number;
  level: "debug" | "info" | "warn" | "error";
  source: string;
  category: string;
  message: string;
  data?: unknown;
  chat_id: number | null;
  character_id: number | null;
  device: string;
};
export type LogPage = { entries: LogEntry[]; has_more: boolean; max_id: number; total: number; matching: number };

/** One backup of the database. `time` (file last written) and `created_at` (backup made) are Unix
 *  milliseconds; `kind` is what it is kept as (a daily one kept for its week is `weekly`, a copy made
 *  by hand next to the database is `manual` with the reason `hand`); `schema` is null when unreadable. */
export type Backup = {
  name: string;
  reason: "daily" | "migration" | "manual" | "before-restore" | "hand";
  kind: "daily" | "weekly" | "migration" | "manual" | "restore";
  time: number;
  created_at: number;
  size: number;
  schema: number | null;
};

/** `GET /backups`: the backups (newest first) and the bytes they take against their limit
 *  (copies made by hand are not counted; `near_full` from 90 %), the database's own size and the free disk space. */
export type BackupOverview = { backups: Backup[]; total_bytes: number; limit_bytes: number; near_full: boolean; db_bytes: number; disk_free: number | null };
/** The limits `PUT /backups/limit` takes: 500 MB, 1, 2 and 5 GB. */
export const BACKUP_LIMITS = [500, 1024, 2048, 5120].map((mb) => mb * 1024 * 1024);
/** `PUT /backups/limit`: `pruned` automatic backups went at once to fit a smaller limit. */
export const setBackupLimit = (limit_bytes: number) => postJson<{ limit_bytes: number; pruned: number }>("/backups/limit", { limit_bytes }, "PUT");
/** `POST /backups/delete`: these backups (all or none), or every automatic one older than `days`. */
export const deleteBackups = (what: { names: string[] } | { older_than_days: number }) => postJson<{ deleted: string[] }>("/backups/delete", what);

export const postJson = <T = void>(path: string, body: unknown, method = "POST") =>
  api<T>(path, { method, headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });

/** `PUT /messages/{id}/swipe`: shows another variant of a reply; the chat arrives over the socket. */
/** The picture behind a link, fetched by the server (`netguard` checks the address; the page itself may
 *  only talk to this server). Throws `ApiError` (`bad_image`, `address_not_allowed`, `source_*`, …). */
export const pictureFromLink = (url: string) =>
  request("/pictures/from-link", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ url }) }).then((r) => r.blob());

export const setSwipe =(id: number, swipe_id: number) => postJson(`/messages/${id}/swipe`, { swipe_id }, "PUT");

/** `rev` changes with the picture, so a new one is fetched while old ones stay cached. */
export const avatarUrl = (characterId: number, rev: number) => apiUrl(`/characters/${characterId}/avatar?token=${encodeURIComponent(token)}&v=${rev}`);
export const personaAvatarUrl = (personaId: number, rev: number) => apiUrl(`/personas/${personaId}/avatar?token=${encodeURIComponent(token)}&rev=${rev}`);
/** The small WebP of a character's picture for grids and lists (the server answers the picture itself
 *  while the thumbnail is not made yet). */
export const thumbUrl = (characterId: number, rev: number) => apiUrl(`/characters/${characterId}/thumb?token=${encodeURIComponent(token)}&v=${rev}`);

/** `POST /import/link/preview`: a fetched card kept on the server (15 min) until it is confirmed. */
export type LinkPreview = {
  status: "preview";
  preview_id: string;
  source: string;
  page_url: string;
  /** Texts are cut to 600 characters (then end with "…"). */
  card: { name: string; creator: string; tags: string[]; description: string; first_mes: string; creator_notes: string };
  has_image: boolean;
  /** A character in the library with the same card. */
  duplicate_of: number | null;
};
/** The picture of a link preview (WebP); 404 once the preview has expired. */
export const linkPreviewImageUrl = (previewId: string) => apiUrl(`/import/link/preview/${encodeURIComponent(previewId)}/image?token=${encodeURIComponent(token)}`);

/** One SillyTavern installation the server found (`user` null: the old layout under `public/`). */
export type StInstall = { path: string; user: string | null; characters: number; personas: number; avatars: number };
/** `GET /import/sillytavern/find`: `complete` false when the search ran out of time. */
export type StFound = { installs: StInstall[]; searched_ms: number; complete: boolean };
export const findSillyTavern = (refresh = false) => api<StFound>(`/import/sillytavern/find${refresh ? "?refresh=1" : ""}`);

/** This device's offset from UTC in minutes east (what the server needs for dates SillyTavern wrote in local time). */
export const tzMinutes = () => -new Date().getTimezoneOffset();
/** One card, persona or chat of a SillyTavern installation; `duplicate_of`: the same one already in the
 *  app (character, persona or chat id); `error`: why it cannot be taken over. */
export type StCard = { id: string; name: string | null; has_image: boolean; duplicate_of: number | null; error: string | null; chats: number };
export type StPersona = { id: string; name: string | null; has_image: boolean; duplicate_of: number | null; error: string | null };
/** `id` is `<folder>/<file>.jsonl`; `card` the id of the card it belongs to; `last_date` Unix seconds. */
export type StChat = { id: string; folder: string; card: string | null; messages: number; last_date: number | null; duplicate_of: number | null; error: string | null };
/** `GET /import/sillytavern/contents`: what an installation holds; reading it imports nothing. */
export type StContents = { path: string; user: string | null; characters: StCard[]; personas: StPersona[]; chats: StChat[] };
const stWhere = (i: StInstall) => `path=${encodeURIComponent(i.path)}${i.user ? `&user=${encodeURIComponent(i.user)}` : ""}`;
export const stContents = (i: StInstall) => api<StContents>(`/import/sillytavern/contents?${stWhere(i)}&tz=${tzMinutes()}`);
/** The picture of one card or persona of an installation (WebP); 404 `no_avatar` when it has none. */
export const stImageUrl = (i: StInstall, what: { card: string } | { persona: string }) => {
  const [key, id] = "card" in what ? ["card", what.card] : ["persona", what.persona];
  return apiUrl(`/import/sillytavern/image?${stWhere(i)}&${key}=${encodeURIComponent(id)}&token=${encodeURIComponent(token)}`);
};

/** The server's error codes in words, in the app's language (`shell.err.*` in the dictionaries):
 *  `ERROR_TEXT[code]` is read each time, `undefined` for a code the app has no words for. */
export const ERROR_TEXT: Record<string, string> = {};
for (const key of keysUnder("shell.err.")) {
  Object.defineProperty(ERROR_TEXT, key.slice("shell.err.".length), { enumerable: true, get: () => t(key) });
}

export const errorText = (e: unknown) => ERROR_TEXT[e instanceof ApiError ? e.code : "unknown"] ?? ERROR_TEXT.unknown;

// Numbers and dates in the app's language; the formatters are made once per language (i18n/format.ts).
export { num };

/** A context size as people say it: 4096 → "4k", 65536 → "64k", 131072 → "128k". */
export const ctxShort = (n: number) => (n >= 1024 ? `${num(Math.round(n / 1024))}k` : num(n));

/** A parameter count: 4 647 450 147 → "4,6 Mrd.", 350 000 000 → "350 Mio.". */
export function paramsShort(n: number): string {
  const v = n >= 1e9 ? n / 1e9 : n / 1e6;
  const b = numberFormat({ maximumFractionDigits: v < 10 ? 1 : 0 }).format(v);
  return n >= 1e9 ? t("models.filter.billions", { b }) : t("models.filter.millions", { b });
}

/** An engine's name. The one the server made from engine.json keeps its German name in the database
 *  ("Eigener Build") and is shown in the app's language. */
export const engineName = (e: { name: string; origin: string }) => (e.origin === "own" && e.name === "Eigener Build" ? t("models.engine.ownBuild") : e.name);

/** The name to show for a model: its own name, else the file without ".gguf". */
export const modelName = (m: ModelInfo | null | undefined) => m?.name || m?.file?.replace(/\.gguf$/i, "") || null;

/** "Q4_K_M · 64k": the model's quant and the context the server uses (empty when both are unknown). */
export const modelFacts = (m: ModelInfo | null | undefined) => [m?.quant, m?.ctx ? ctxShort(m.ctx) : null].filter(Boolean).join(" · ");

/** Bytes as Windows shows them: 3 106 738 272 → "2,9 GB", 891 289 600 → "850 MB" (1 GB = 1024³ bytes). */
export function gb(bytes: number): string {
  if (bytes < 1024 ** 3) return t("shell.size.mb", { n: num(Math.max(0, Math.round(bytes / 1024 ** 2))) });
  const v = bytes / 1024 ** 3;
  return t("shell.size.gb", { n: numberFormat({ maximumFractionDigits: v < 10 ? 1 : 0, minimumFractionDigits: v < 10 ? 1 : 0 }).format(v) });
}

export const count = (n: number, one: string, many: string) => `${num(n)} ${n === 1 ? one : many}`;

/** "heute, 20:31" / "gestern, 09:05" / "3. Nov., 14:20" from Unix seconds. */
/** Whole calendar days from `d` to today (0 = today). */
const daysAgo = (d: Date) => Math.round((new Date().setHours(0, 0, 0, 0) - new Date(d).setHours(0, 0, 0, 0)) / 86_400_000);

export function when(seconds: number): string {
  const d = new Date(seconds * 1000);
  const time = dateFormat({ hour: "2-digit", minute: "2-digit" }).format(d);
  const days = daysAgo(d);
  if (days === 0) return t("shell.time.today", { time });
  if (days === 1) return t("shell.time.yesterday", { time });
  return `${dateFormat({ day: "numeric", month: "short" }).format(d)}, ${time}`;
}

/** Puts text on the clipboard. Phones reach the app over plain http, where only the old way works. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const box = document.createElement("textarea");
    box.value = text;
    box.style.cssText = "position:fixed;opacity:0";
    document.body.append(box);
    box.select();
    const ok = document.execCommand("copy");
    box.remove();
    return ok;
  }
}

/** A message start as one plain line (no line breaks, no emphasis marks). */
export const snippet = (text: string) => text.replace(/\*+/g, "").replace(/\s+/g, " ").trim();
