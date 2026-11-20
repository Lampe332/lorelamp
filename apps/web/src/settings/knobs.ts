// The controls of the settings tab "Experte": every sampler value of a profile (limits as in
// rp_core::sampler, defaults as llama-server uses them when a profile leaves them out) in groups, and
// the start options with a slider.

import { num, type Sampler } from "../api";
import { t, texts, type Key } from "../i18n";

/** `label` and `note` are read in the language on screen each time (`texts`; never spread a knob). */
export type Knob = {
  key: keyof Sampler;
  readonly label: string;
  /** One line on what it does. */
  readonly note: string;
  /** The value used when the profile leaves it out. */
  dflt: number;
  /** What the server accepts. */
  min: number;
  max: number;
  step: number;
  /** The slider's part of that range (the field takes the whole range). */
  range: [number, number];
  /** The slider's step, when it differs from `step`. */
  slide?: number;
  /** Ctrl+K points here. */
  spot?: string;
};

/** What the app sends when a profile leaves them out (rp_core::sampler). */
export const APP_TEMPERATURE = 0.8;
export const APP_N_PREDICT = 800;
const WHOLE = 1 << 20;

/** A knob; its words are `models.knob.<key>` (the label) and `models.knob.<key>.note`. */
const k = (key: keyof Sampler, dflt: number, [min, max, step]: [number, number, number], range: [number, number], spot?: string, slide?: number): Knob =>
  texts({ key, dflt, min, max, step, range, spot, slide }, { label: `models.knob.${key}` as Key, note: `models.knob.${key}.note` as Key });

/** `title` and `note` are `models.group.<id>` and `models.group.<id>.note`, read each time. */
export type KnobGroup = { id: string; readonly title: string; readonly note: string; knobs: Knob[] };
const group = (id: string, knobs: Knob[]): KnobGroup => texts({ id, knobs }, { title: `models.group.${id}` as Key, note: `models.group.${id}.note` as Key });

/** Every sampler value, grouped the way people think about them. */
export const GROUPS: KnobGroup[] = [
  group("random", [k("temperature", APP_TEMPERATURE, [0, 5, 0.05], [0, 2], "temperature")]),
  group("pick", [
    k("min_p", 0.05, [0, 1, 0.01], [0, 0.5], "min-p"),
    k("top_p", 0.95, [0, 1, 0.01], [0, 1], "top-p"),
    k("top_k", 40, [0, 1000, 1], [0, 200], "top-k"),
    k("typical_p", 1, [0, 1, 0.01], [0, 1]),
  ]),
  group("repeat", [
    k("repeat_penalty", 1, [0, 5, 0.01], [0.8, 2], "repeat-penalty"),
    k("repeat_last_n", 64, [-1, WHOLE, 1], [-1, 4096]),
    k("presence_penalty", 0, [-2, 2, 0.05], [-2, 2]),
    k("frequency_penalty", 0, [-2, 2, 0.05], [-2, 2]),
  ]),
  group("dry", [
    k("dry_multiplier", 0, [0, 10, 0.05], [0, 2], "dry"),
    k("dry_base", 1.75, [1, 10, 0.05], [1, 4]),
    k("dry_allowed_length", 2, [0, 1000, 1], [0, 20]),
    k("dry_penalty_last_n", -1, [-1, WHOLE, 1], [-1, 4096]),
  ]),
  group("xtc", [k("xtc_probability", 0, [0, 1, 0.01], [0, 1], "xtc"), k("xtc_threshold", 0.1, [0, 1, 0.01], [0, 0.5])]),
  group("length", [k("n_predict", APP_N_PREDICT, [1, 32768, 1], [32, 8192], "n-predict", 32)]),
  group("seed", [k("seed", -1, [-1, 4294967295, 1], [-1, 9999], "seed")]),
];

export const KNOBS: Knob[] = GROUPS.flatMap((g) => g.knobs);

/** Decimals of a step (0.05 → 2), for clean slider values. */
export const decimals = (step: number) => (String(step).split(".")[1] ?? "").length;

/** The problem with a typed value, or null. "" means "not set" and is always fine. */
export function badValue(k: Knob, text: string): string | null {
  if (text === "") return null;
  const v = Number(text);
  if (!Number.isFinite(v)) return t("models.knob.bad.nan", { label: k.label });
  if (v < k.min || v > k.max) return t("models.knob.bad.range", { label: k.label, min: num(k.min), max: num(k.max) });
  if (k.step === 1 && !Number.isInteger(v)) return t("models.knob.bad.whole", { label: k.label });
  return null;
}
