// The words of the tab "Vorschläge" that depend on the server's numbers: the reason of a pick,
// where a file runs (pill, pick line, "Benutzen"), and the size tiers, in the app's language
// (dictionary `models`). Pure functions that import nothing of the app but `i18n` (no React
// components, no `api.ts`), so the e2e tests can check every text directly (in German there).

import { numberFormat, t, texts } from "../i18n";
import type { CatalogModel, CatalogQuant, Fit, Need, Pick, Placement, Tier, Why } from "./types";

const num = (n: number) => numberFormat().format(n);
/** 8192 → "8k", 131072 → "128k" (as `ctxShort` in `api.ts`). */
const ctxShort = (n: number) => (n >= 1024 ? `${num(Math.round(n / 1024))}k` : num(n));

/** `Intl` puts a no-break space between number and unit; the app writes an ordinary one there. */
const plainSpace = (s: string) => s.replace(/[  ]/g, " ");

/** A number with its unit as the app's language writes it: `withUnit(17.4, "gigabyte", 1)` →
 *  "17,4 GB" (de), "17.4 GB" (en), "17,4 ГБ" (ru). `min`: the fewest decimals (default `digits`). */
export const withUnit = (n: number, unit: string, digits: number, min = digits) =>
  plainSpace(numberFormat({ style: "unit", unit, maximumFractionDigits: digits, minimumFractionDigits: min }).format(n));

/** A whole percent (0–100) as the language writes it: "42 %" (de), "42%" (en). */
export const percent = (n: number) => plainSpace(numberFormat({ style: "percent", maximumFractionDigits: 0 }).format(n / 100));

/** 20.06 → "20,1 GB" (de), "20.1 GB" (en), "20,1 ГБ" (ru). */
export const gb = (n: number) => withUnit(n, "gigabyte", 1);
/** One decimal: a rating like 4.2. */
const oneDecimal = (n: number) => numberFormat({ maximumFractionDigits: 1, minimumFractionDigits: 1 }).format(n);

/** Read in the language on screen each time (`texts`). */
export const TIER_NAME: Record<Tier, string> = texts({}, { small: "models.tier.small", balanced: "models.tier.balanced", large: "models.tier.large" });
export const TIERS: Tier[] = ["small", "balanced", "large"];
export const FIT_NAME: Record<Fit, string> = texts({}, { fits: "models.fit.fits", cpu_help: "models.fit.cpu_help", too_big: "models.fit.too_big" });

/** The pill's word for a tier: on a PC without a GPU (no VRAM part) "RAM help" would be odd, the
 *  processor does all of it. */
export function fitName(q: Pick | CatalogQuant): string | null {
  if (!q.fit) return null;
  return q.fit === "cpu_help" && !q.gpu_gb ? t("models.fit.cpuOnly") : FIT_NAME[q.fit];
}

/** Where a file runs at 8k, from the server's VRAM and RAM part: "ca. 20,1 GB im Grafikspeicher + 9,8 GB im
 *  Arbeitsspeicher"; null when the parts are not known. */
export function placeText(fit: Fit | null | undefined, gpuGb: number | null | undefined, ramGb: number | null | undefined): string | null {
  if (fit === "fits" && gpuGb) return t("models.place.gpu", { gpu: gb(gpuGb) });
  if (fit !== "cpu_help" || ramGb == null) return null;
  return gpuGb ? t("models.place.split", { gpu: gb(gpuGb), ram: gb(ramGb) }) : t("models.place.ram", { ram: gb(ramGb) });
}

/** The plain-words tooltip for `--n-cpu-moe n` (no flag names on the simple view). */
export const moeTitle = (n: number | null | undefined) => (n ? t("models.moeTitle", { n }) : undefined);

/** The plain-words tooltip for `-ngl n`: whole layers split between the card and the processor. */
export const layersTitle = (n: number | null | undefined) => (n != null ? t("models.layersTitle", { n }) : undefined);

/** The tooltip of a pill or a placement: the expert split, else the layer split. */
export const splitTitle = (x: { n_cpu_moe?: number | null; n_gpu_layers?: number | null }) => moeTitle(x.n_cpu_moe) ?? layersTitle(x.n_gpu_layers);

const activeB = (m: CatalogModel) => num((m.moe ? m.params_active_b : null) ?? m.params_total_b ?? 0);

/** The short reason for a pick, from the server's `reason_code` (see `Reason` in
 *  `crates/server/src/fetch/pick.rs`). "Schnell" only where the model runs all on the GPU or a
 *  benchmark measured it; an unknown code gets a neutral sentence. */
export function reasonText(p: Pick, m: CatalogModel, budget: number): string {
  const tg = p.measured ? Math.round(p.measured.tg) : null;
  switch (p.reason_code) {
    case "best_quality":
      if (p.fit === "cpu_help") return p.gpu_gb ? t("models.reason.best_quality.split") : t("models.reason.best_quality.ram");
      return t("models.reason.best_quality");
    case "installed_quality":
      return t("models.reason.installed_quality");
    case "fast_moe":
      return t("models.reason.fast_moe", { b: activeB(m) });
    case "small_dense":
      return t("models.reason.small_dense", { b: activeB(m) });
    case "measured_fast":
      return tg ? t("models.reason.measured_fast", { tg }) : t("models.reason.measured_fast.plain");
    case "measured_slow":
      return tg ? t("models.reason.measured_slow", { tg }) : t("models.reason.measured_slow.plain");
    case "partly_in_ram":
      return m.moe ? t("models.reason.partly_in_ram.moe", { b: activeB(m) }) : t("models.reason.partly_in_ram");
    case "cpu_moe":
      return t("models.reason.cpu_moe", { b: activeB(m) });
    case "cpu_dense":
      return t("models.reason.cpu_dense", { b: activeB(m) });
    case "room_for_context":
      return p.ctx_max ? t("models.reason.room_for_context", { ctx: ctxShort(p.ctx_max) }) : t("models.reason.room_for_context.free", { free: gb(Math.max(0, budget - p.est_gb)) });
    default:
      return t("models.reason.other");
  }
}

/** Where a pick runs at 8k and, outside the Kontext column (its reason says it), the context it holds.
 *  No-break spaces keep "· Kontext bis 128k" in one piece, with the dot at the end of a wrapped line. */
export function placeLine(p: Pick): string | null {
  const where = placeText(p.fit, p.gpu_gb, p.ram_gb);
  const ctx = p.ctx_max && p.reason_code !== "room_for_context" ? t("models.ctxUpTo", { ctx: ctxShort(p.ctx_max) }) : null;
  return where && ctx ? `${where} · ${ctx}` : (where ?? ctx);
}

/** How "Benutzen" set up a new model on this PC (`placement` of `POST /models/local/use`). */
export function placementText(pl: Placement): string {
  const ctx = ctxShort(pl.ctx);
  // Too big as the engine runs it (e.g. a profile that keeps everything on the processor): no split is set.
  if (pl.fit === "too_big") return t("models.placement.too_big", { ctx });
  if (pl.fit === "fits") return t("models.placement.fits", { gpu: gb(pl.gpu_gb), ctx });
  if (!pl.gpu_gb) return t("models.placement.cpu", { ram: gb(pl.ram_gb), ctx });
  const split = t("models.placement.split", { gpu: gb(pl.gpu_gb), ram: gb(pl.ram_gb), ctx });
  return pl.n_gpu_layers != null ? `${split} ${t("models.placement.layers", { n: pl.n_gpu_layers })}` : split;
}

// ---------------------------------------------------------------- purpose ("Was willst du mit dem Modell machen?")

/** The chips, in the server's order (`Need::ALL`), read in the language on screen each time. */
export const NEED_NAME: Record<Need, string> = texts(
  {},
  {
    long_chats: "models.need.long_chats",
    group: "models.need.group",
    story: "models.need.story",
    german: "models.need.german",
    english: "models.need.english",
    fast: "models.need.fast",
    context: "models.need.context",
    uncensored: "models.need.uncensored",
    weak_pc: "models.need.weak_pc",
    quality: "models.need.quality",
    short_replies: "models.need.short_replies",
    long_replies: "models.need.long_replies",
  },
);

const params = (v: number | null) => num(Math.round((v ?? 0) * 10) / 10);

/** One reason or caveat of a suggestion in plain words (`Why.code` in `fetch/suggest.rs`); null for
 *  a code this version does not know. */
export function whyText(w: Why): string | null {
  const v = w.value;
  const of = num(v ?? 0);
  switch (w.code) {
    case "roleplay":
      return t("models.why.roleplay", { v: of });
    case "weak_roleplay":
      return t("models.why.weak_roleplay", { v: of });
    case "german":
      return (v ?? 0) >= 5 ? t("models.why.german.top") : t("models.why.german", { v: of });
    case "weak_german":
      return t("models.why.weak_german", { v: of });
    case "english":
      return t("models.why.english", { v: of });
    case "adventure":
      return t("models.why.adventure");
    case "story":
      return t("models.why.story");
    case "context":
      return t("models.why.context", { ctx: ctxShort(v ?? 0) });
    case "little_context":
      return t("models.why.little_context", { ctx: ctxShort(v ?? 0) });
    case "fast_moe":
      return t("models.why.fast_moe", { b: params(v) });
    case "fast_small":
      return t("models.why.fast_small", { b: params(v) });
    case "slow":
      return t("models.why.slow", { b: params(v) });
    case "measured_fast":
      return t("models.why.measured_fast", { tg: Math.round(v ?? 0) });
    case "measured_slow":
      return t("models.why.measured_slow", { tg: Math.round(v ?? 0) });
    case "light":
      return t("models.why.light", { size: gb(v ?? 0) });
    case "heavy":
      return t("models.why.heavy", { size: gb(v ?? 0) });
    case "uncensored":
      return t("models.why.uncensored");
    case "partly_in_ram":
      return t("models.why.partly_in_ram", { size: gb(v ?? 0) });
    case "installed":
      return t("models.why.installed");
    default:
      return null;
  }
}

const upper = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** The reason line of a suggestion: its reasons, the strongest first ("Hält 128k Kontext · stark im
 *  Rollenspiel (5 von 5)"); a model that only stands there by its ratings says so. */
export function reasonLine(s: { reasons: Why[]; q: number }): string {
  const parts = s.reasons.map(whyText).filter((x): x is string => x !== null);
  return parts.length ? upper(parts.join(" · ")) : t("models.why.overall", { q: oneDecimal(s.q) });
}

/** What speaks against it for this purpose; null when nothing does. */
export function caveatLine(s: { caveats: Why[] }): string | null {
  const parts = s.caveats.map(whyText).filter((x): x is string => x !== null);
  return parts.length ? t("models.why.but", { list: parts.join(" · ") }) : null;
}

/** Why the placement differs from the picks' plan; null when it does not (or the reason is unknown). */
export function fallbackText(fallback: string | null | undefined): string | null {
  if (fallback === "n_cpu_moe_unsupported") return t("models.fallback.n_cpu_moe_unsupported");
  return null;
}
