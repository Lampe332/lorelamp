// The filters and the sort order of the catalog in the tab "Vorschläge": what each one keeps, the
// order of the cards, and the choice kept in this browser (localStorage, read and written with
// try/catch, so a blocked storage only means a fresh start).

import { t, texts } from "../i18n";
import type { CatalogModel, CatalogQuant, Tier } from "./types";
import { TIERS } from "./words";

export type Kind = "all" | "moe" | "dense";
export type Censor = "all" | "open" | "free";
export type Have = "all" | "yes" | "no";
export type FitLevel = "gpu" | "ram";

export type Filters = {
  /** Only tiers that fit: as the server's pill says (`fit` of the tier: all on the GPU, or with RAM
   *  help, or in the RAM of a PC without a GPU); without a PC reading, tiers whose need all on the
   *  GPU is at most `gb` (the slider, set by hand). */
  fit: boolean;
  /** With a graphics card, how a tier must fit: `gpu` all on the card (the pill "Passt"), `ram` also
   *  with a part in the RAM (the pill "Passt mit CPU-Hilfe", slower). A PC without a usable card has
   *  one level only: in its RAM. */
  fitLevel: FitLevel;
  gb: number | null;
  kind: Kind;
  /** Families (`base`); empty: all. */
  bases: string[];
  /** Index into PARAM_STEPS: total parameters from … to. */
  pMin: number;
  pMax: number;
  /** Context at least (tokens); 0: any. */
  ctx: number;
  rp: number;
  de: number;
  censor: Censor;
  /** Licences ("" = none given); empty: all. */
  licenses: string[];
  have: Have;
  /** Size tiers; empty: all. */
  tiers: Tier[];
};

export const PARAM_STEPS = [0, 4, 8, 12, 16, 24, 32, Infinity] as const;
export const CTX_STEPS = [0, 32_768, 131_072, 262_144] as const;
export const GB_MIN = 4;
export const GB_MAX = 48;

export const NO_FILTERS: Filters = {
  fit: false,
  fitLevel: "gpu",
  gb: null,
  kind: "all",
  bases: [],
  pMin: 0,
  pMax: PARAM_STEPS.length - 1,
  ctx: 0,
  rp: 0,
  de: 0,
  censor: "all",
  licenses: [],
  have: "all",
  tiers: [],
};

export type Sort = "rec" | "rp" | "de" | "tempo" | "small" | "big" | "name";
/** The sort orders; `label` is read in the language on screen each time (never spread a row). */
export const SORTS: { id: Sort; readonly label: string }[] = [
  texts({ id: "rec" as Sort }, { label: "models.sort.rec" }),
  texts({ id: "rp" as Sort }, { label: "models.sort.rp" }),
  texts({ id: "de" as Sort }, { label: "models.sort.de" }),
  texts({ id: "tempo" as Sort }, { label: "models.sort.tempo" }),
  texts({ id: "small" as Sort }, { label: "models.sort.small" }),
  texts({ id: "big" as Sort }, { label: "models.sort.big" }),
  texts({ id: "name" as Sort }, { label: "models.sort.name" }),
];

export const BASE_NAME: Record<string, string> = { gemma: "Gemma", qwen: "Qwen", mistral: "Mistral", nemo: "Mistral Nemo", llama: "Llama" };
export const baseName = (b: string) => BASE_NAME[b] ?? b.charAt(0).toUpperCase() + b.slice(1);
export const licenseName = (l: string) => (l === "" ? t("models.filter.noLicense") : l === "apache-2.0" ? "Apache 2.0" : l === "mit" ? "MIT" : l);

/** The slider's start without a PC reading: a value the user sees and moves, never used with a PC. */
const GB_START = 8;
/** The GB the filter "passt" compares with when the PC is unknown: the slider. */
export const gbOf = (f: Filters) => f.gb ?? GB_START;

/** The server read the PC: every tier carries its pill (`fit`). */
export const pcKnown = (models: CatalogModel[]) => models.some((m) => TIERS.some((t) => m.quants[t]?.fit));

/** Parameters used per token (billions): the active ones of a MoE model, else all. */
export const activeB = (m: CatalogModel) => (m.moe ? (m.params_active_b ?? m.params_total_b) : m.params_total_b) ?? Infinity;
const mainSize = (m: CatalogModel) => (m.quants.balanced ?? m.quants.small ?? m.quants.large)?.size_bytes ?? 0;

/** The tiers a filter looks at, and whether one of them passes the size filters. */
const tiersOf = (f: Filters) => (f.tiers.length ? f.tiers : TIERS);
/** Whether a tier fits: the pill's verdict when the PC is known (all on the card; with the level
 *  `ram` also with a part in the RAM; on a PC without a usable card whatever runs in its RAM), else
 *  the slider. */
function fits(q: CatalogQuant, f: Filters): boolean {
  if (!q.fit) return q.est_gb <= gbOf(f);
  if (q.fit === "cpu_help") return f.fitLevel === "ram" || !q.gpu_gb;
  return q.fit === "fits";
}
function tierOk(q: CatalogQuant | null, f: Filters): boolean {
  return !!q && !q.missing && (!f.fit || fits(q, f));
}

/** Order of the passing tiers, as the picks choose one: all on the GPU before RAM help; among those
 *  all on the GPU the biggest, among those with RAM help balanced, small, large. */
function rank(q: CatalogQuant, t: Tier): number {
  return q.fit === "cpu_help" ? 10 + ["balanced", "small", "large"].indexOf(t) : 2 - TIERS.indexOf(t);
}

/** The tiers of `m` that pass, the best first (for the card's first choice). */
export function passingTiers(m: CatalogModel, f: Filters): Tier[] {
  return tiersOf(f)
    .filter((t) => tierOk(m.quants[t], f))
    .sort((a, b) => rank(m.quants[a]!, a) - rank(m.quants[b]!, b));
}

export function keep(m: CatalogModel, f: Filters): boolean {
  const p = m.params_total_b;
  const installed = TIERS.some((t) => m.quants[t]?.installed);
  return (
    (f.kind === "all" || (f.kind === "moe") === m.moe) &&
    (f.bases.length === 0 || f.bases.includes(m.base)) &&
    (f.pMin === 0 || (p !== null && p >= PARAM_STEPS[f.pMin])) &&
    (f.pMax === PARAM_STEPS.length - 1 || (p !== null && p <= PARAM_STEPS[f.pMax])) &&
    (f.ctx === 0 || (m.context_max ?? 0) >= f.ctx) &&
    m.rp >= f.rp &&
    m.de >= f.de &&
    (f.censor === "all" || (f.censor === "open" ? m.uncensored >= 1 : m.uncensored === 2)) &&
    (f.licenses.length === 0 || f.licenses.includes(m.license ?? "")) &&
    (f.have === "all" || (f.have === "yes") === installed) &&
    ((!f.fit && f.tiers.length === 0) || passingTiers(m, f).length > 0)
  );
}

const byName = (a: CatalogModel, b: CatalogModel) => a.name.localeCompare(b.name, "de");
export const ORDER: Record<Sort, (a: CatalogModel, b: CatalogModel) => number> = {
  rec: (a, b) => b.q - a.q || b.rp - a.rp || byName(a, b),
  rp: (a, b) => b.rp - a.rp || b.q - a.q || byName(a, b),
  de: (a, b) => b.de - a.de || b.q - a.q || byName(a, b),
  tempo: (a, b) => activeB(a) - activeB(b) || b.q - a.q || byName(a, b),
  small: (a, b) => mainSize(a) - mainSize(b) || byName(a, b),
  big: (a, b) => mainSize(b) - mainSize(a) || byName(a, b),
  name: byName,
};

/** How many filters differ from the start (for the button and "zurücksetzen"). */
export function activeCount(f: Filters): number {
  return (
    Number(f.fit) +
    Number(f.kind !== "all") +
    Number(f.bases.length > 0) +
    Number(f.pMin !== 0 || f.pMax !== PARAM_STEPS.length - 1) +
    Number(f.ctx !== 0) +
    Number(f.rp !== 0) +
    Number(f.de !== 0) +
    Number(f.censor !== "all") +
    Number(f.licenses.length > 0) +
    Number(f.have !== "all") +
    Number(f.tiers.length > 0)
  );
}

// ---------------------------------------------------------------- kept in this browser

const KEY = "rp-sug-view";
type Kept = { filters: Filters; sort: Sort; open: boolean | null; uncensored: boolean };

const isStrings = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");
const oneOf = <T extends string>(v: unknown, all: readonly T[], dflt: T): T => (all.includes(v as T) ? (v as T) : dflt);
const int = (v: unknown, lo: number, hi: number, dflt: number) => (Number.isInteger(v) && (v as number) >= lo && (v as number) <= hi ? (v as number) : dflt);

/** The stored choice, each field checked; anything odd falls back to the start value. */
export function loadKept(): Kept {
  let raw: Record<string, unknown> = {};
  try {
    raw = JSON.parse(localStorage.getItem(KEY) ?? "{}") ?? {};
  } catch {
    // blocked or broken storage: start fresh
  }
  const f = (raw.filters ?? {}) as Record<string, unknown>;
  const top = PARAM_STEPS.length - 1;
  const filters: Filters = {
    fit: f.fit === true,
    fitLevel: oneOf(f.fitLevel, ["gpu", "ram"] as const, "gpu"),
    gb: f.gb === null || f.gb === undefined ? null : int(f.gb, GB_MIN, GB_MAX, 8),
    kind: oneOf(f.kind, ["all", "moe", "dense"] as const, "all"),
    bases: isStrings(f.bases) ? f.bases : [],
    pMin: int(f.pMin, 0, top, 0),
    pMax: int(f.pMax, 0, top, top),
    ctx: (CTX_STEPS as readonly number[]).includes(f.ctx as number) ? (f.ctx as number) : 0,
    rp: int(f.rp, 0, 5, 0),
    de: int(f.de, 0, 5, 0),
    censor: oneOf(f.censor, ["all", "open", "free"] as const, "all"),
    licenses: isStrings(f.licenses) ? f.licenses : [],
    have: oneOf(f.have, ["all", "yes", "no"] as const, "all"),
    tiers: isStrings(f.tiers) ? (f.tiers.filter((t) => (TIERS as string[]).includes(t)) as Tier[]) : [],
  };
  if (filters.pMin > filters.pMax) [filters.pMin, filters.pMax] = [0, top];
  return {
    filters,
    sort: oneOf(raw.sort, SORTS.map((s) => s.id), "rec"),
    open: typeof raw.open === "boolean" ? raw.open : null,
    uncensored: raw.uncensored === true,
  };
}

export function saveKept(k: Kept) {
  try {
    localStorage.setItem(KEY, JSON.stringify(k));
  } catch {
    // storage full or blocked: the choice lasts until the page closes
  }
}
