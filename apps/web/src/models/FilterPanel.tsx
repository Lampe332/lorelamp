// The filters of the catalog in the tab "Vorschläge": a column beside the cards on wide panels, above
// them on narrow ones, a sheet from the bottom on a phone. Every choice is a button with aria-pressed
// (several may be on for Familie, Lizenz and Stufe), the sizes are sliders.

import { useId } from "react";
import { ctxShort, num } from "../api";
import { t, texts } from "../i18n";
import { Icon } from "../ui";
import { TIER_NAME, TIERS, withUnit } from "./words";
import { baseName, CTX_STEPS, GB_MAX, GB_MIN, gbOf, licenseName, PARAM_STEPS, type Censor, type Filters, type FitLevel, type Have, type Kind } from "./filters";
import "./purpose.css";

/** `pc`: the server read the PC (with or without a usable GPU): "passt" follows the pills; null: the
 *  slider decides. */
type Props = { f: Filters; set: (f: Filters) => void; pc: "gpu" | "cpu" | null; bases: string[]; licenses: string[] };

/** The line under the switch "Passt auf meinen PC": what it counts, per level. */
const FIT_HOW = texts({}, { gpu: "models.filter.how.gpu", ram: "models.filter.how.ram", cpu: "models.filter.how.cpu" });
const LEVELS = (): [FitLevel, string][] => [
  ["gpu", t("models.filter.level.gpu")],
  ["ram", t("models.filter.level.ram")],
];

function Choice<T extends string | number>({ name, options, value, pick }: { name: string; options: [T, string][]; value: T; pick: (v: T) => void }) {
  return (
    <fieldset className="sug-f">
      <legend>{name}</legend>
      <div className="sug-chips">
        {options.map(([v, label]) => (
          <button key={String(v)} type="button" className="fchip" aria-pressed={value === v} onClick={() => pick(v)}>
            {label}
          </button>
        ))}
      </div>
    </fieldset>
  );
}

function Many<T extends string>({ name, options, value, pick }: { name: string; options: [T, string][]; value: T[]; pick: (v: T[]) => void }) {
  return (
    <fieldset className="sug-f">
      <legend>{name}</legend>
      <div className="sug-chips">
        {options.map(([v, label]) => {
          const on = value.includes(v);
          return (
            <button key={v} type="button" className="fchip" aria-pressed={on} onClick={() => pick(on ? value.filter((x) => x !== v) : [...value, v])}>
              {on && <Icon name="check" small />}
              {label}
            </button>
          );
        })}
      </div>
    </fieldset>
  );
}

/** Whole GB with the unit: "8 GB". */
const gbText = (n: number) => withUnit(n, "gigabyte", 0);
const paramText =(i: number) => (PARAM_STEPS[i] === Infinity ? t("models.filter.any") : t("models.filter.billions", { b: num(PARAM_STEPS[i]) }));
const atLeast = (): [number, string][] => [
  [0, t("models.filter.all")],
  [2, t("models.filter.from", { v: num(2) })],
  [3, t("models.filter.from", { v: num(3) })],
  [4, t("models.filter.from", { v: num(4) })],
  [5, num(5)],
];

export function FilterPanel({ f, set, pc, bases, licenses }: Props) {
  const id = useId();
  const top = PARAM_STEPS.length - 1;
  const gbNow = gbOf(f);
  const put = <K extends keyof Filters>(k: K, v: Filters[K]) => set({ ...f, [k]: v });
  return (
    <div className="sug-filter">
      <fieldset className="sug-f">
        <legend>{t("models.filter.memory")}</legend>
        <label className="switch-row sug-fit">
          <span className="sw-text">
            <b>{pc ? t("models.filter.fitPc") : t("models.filter.fitVram")}</b>
            <span>{pc === "gpu" ? FIT_HOW[f.fitLevel] : pc ? FIT_HOW.cpu : t("models.filter.noScan")}</span>
          </span>
          <input type="checkbox" role="switch" className="switch" checked={f.fit} onChange={(e) => put("fit", e.target.checked)} />
        </label>
        {pc === "gpu" && (
          <div className={`pu-levels${f.fit ? "" : " off"}`} role="group" aria-label={t("models.filter.level")}>
            {LEVELS().map(([v, label]) => (
              <button key={v} type="button" className="fchip" aria-pressed={f.fitLevel === v} disabled={!f.fit} onClick={() => put("fitLevel", v)}>
                {label}
              </button>
            ))}
          </div>
        )}
        {!pc && (
          <div className={`sug-range${f.fit ? "" : " off"}`}>
            <label htmlFor={`${id}-gb`}>{t("models.filter.atMost")}</label>
            <output htmlFor={`${id}-gb`} className="num">
              {gbText(gbNow)}
            </output>
            <input
              id={`${id}-gb`}
              type="range"
              className="sug-slider"
              min={GB_MIN}
              max={GB_MAX}
              step={1}
              value={gbNow}
              disabled={!f.fit}
              aria-label={t("models.filter.vramAtMost")}
              aria-valuetext={gbText(gbNow)}
              onChange={(e) => put("gb", Number(e.target.value))}
            />
          </div>
        )}
      </fieldset>
      <Choice<Kind>
        name={t("models.filter.kind")}
        options={[
          ["all", t("models.filter.all")],
          ["moe", "MoE"],
          ["dense", t("models.filter.dense")],
        ]}
        value={f.kind}
        pick={(v) => put("kind", v)}
      />
      <Many name={t("models.filter.family")} options={bases.map((b) => [b, baseName(b)] as [string, string])} value={f.bases} pick={(v) => put("bases", v)} />
      <fieldset className="sug-f">
        <legend>{t("models.filter.params")}</legend>
        <div className="sug-range">
          <label htmlFor={`${id}-pmin`}>{t("models.filter.fromLabel")}</label>
          <output htmlFor={`${id}-pmin`} className="num">
            {paramText(f.pMin)}
          </output>
          <input
            id={`${id}-pmin`}
            type="range"
            className="sug-slider"
            min={0}
            max={top}
            step={1}
            value={f.pMin}
            aria-label={t("models.filter.paramsFrom")}
            aria-valuetext={paramText(f.pMin)}
            onChange={(e) => {
              const v = Number(e.target.value);
              set({ ...f, pMin: v, pMax: Math.max(f.pMax, v) });
            }}
          />
        </div>
        <div className="sug-range">
          <label htmlFor={`${id}-pmax`}>{t("models.filter.toLabel")}</label>
          <output htmlFor={`${id}-pmax`} className="num">
            {paramText(f.pMax)}
          </output>
          <input
            id={`${id}-pmax`}
            type="range"
            className="sug-slider"
            min={0}
            max={top}
            step={1}
            value={f.pMax}
            aria-label={t("models.filter.paramsTo")}
            aria-valuetext={paramText(f.pMax)}
            onChange={(e) => {
              const v = Number(e.target.value);
              set({ ...f, pMax: v, pMin: Math.min(f.pMin, v) });
            }}
          />
        </div>
      </fieldset>
      <Choice<number>
        name={t("models.filter.ctx")}
        options={CTX_STEPS.map((c) => [c, c === 0 ? t("models.filter.all") : ctxShort(c)] as [number, string])}
        value={f.ctx}
        pick={(v) => put("ctx", v)}
      />
      <Choice<number> name={t("models.filter.rp")} options={atLeast()} value={f.rp} pick={(v) => put("rp", v)} />
      <Choice<number> name={t("models.filter.de")} options={atLeast()} value={f.de} pick={(v) => put("de", v)} />
      <Choice<Censor>
        name={t("models.filter.censor")}
        options={[
          ["all", t("models.filter.all")],
          ["open", t("models.filter.censor.open")],
          ["free", t("models.filter.censor.free")],
        ]}
        value={f.censor}
        pick={(v) => put("censor", v)}
      />
      <Many name={t("models.filter.license")} options={licenses.map((l) => [l, licenseName(l)] as [string, string])} value={f.licenses} pick={(v) => put("licenses", v)} />
      <Choice<Have>
        name={t("models.installed")}
        options={[
          ["all", t("models.filter.all")],
          ["yes", t("models.installed")],
          ["no", t("models.filter.notInstalled")],
        ]}
        value={f.have}
        pick={(v) => put("have", v)}
      />
      <Many name={t("models.filter.tier")} options={TIERS.map((t) => [t, TIER_NAME[t]] as [(typeof TIERS)[number], string])} value={f.tiers} pick={(v) => put("tiers", v)} />
    </div>
  );
}
