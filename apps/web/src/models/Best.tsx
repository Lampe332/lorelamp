// "Bestes Modell für deinen PC": the server's top 3 per purpose (Rollenspiel, Tempo, Kontext; a base
// model line stands once on the page while lines suffice, so the columns show different families), each
// with its size tier, the ratings, a short reason and where it runs (all in the VRAM, part in the RAM,
// or on a PC without a GPU all in the RAM) with the context it still holds; an empty column says why.
// "Nur unzensierte" asks for the picks among the models with `uncensored ≥ 1`. On a phone one purpose
// at a time.

import { useState } from "react";
import { t, texts } from "../i18n";
import { Dots, GetButton } from "./pieces";
import { gb, memGb, size, TIER_NAME } from "./store";
import type { Catalog, EmptyReason, Pick, Purpose } from "./types";
import { moeTitle, placeLine, reasonText, withUnit } from "./words";

/** The three columns; `name` and `what` are read in the language on screen each time. */
const PURPOSES: { id: Purpose; readonly name: string; readonly what: string }[] = [
  texts({ id: "roleplay" as Purpose }, { name: "models.best.roleplay", what: "models.best.roleplay.what" }),
  texts({ id: "tempo" as Purpose }, { name: "models.best.tempo", what: "models.best.tempo.what" }),
  texts({ id: "context" as Purpose }, { name: "models.best.context", what: "models.best.context.what" }),
];

/** Why a column has no pick. */
export function emptyText(why: EmptyReason | null | undefined, uncensored: boolean): string {
  if (why === "nothing_fits") return uncensored ? t("models.best.empty.uncensored") : t("models.best.empty.nothing_fits");
  if (why === "all_shown") return t("models.best.empty.all_shown");
  return t("models.best.empty");
}

/** Whole GB of a memory size, with its unit: "24 GB". */
const wholeGb = (bytes: number) => withUnit(memGb(bytes), "gigabyte", 0);

/** How the fit is reckoned: on the GPU with the part of the VRAM the model may fill, else in the RAM. */
export function howText(picks: Pick[] | null, budget: number | null, hasGpu: boolean): string {
  const tail = t("models.best.how.tail");
  if (budget && budget > 0) {
    const split = (picks ?? []).some((p) => p.fit === "cpu_help" && p.gpu_gb);
    return [t("models.best.how.gpu", { budget: gb(budget) }), split ? t("models.best.how.moe") : null, tail].filter(Boolean).join(" ");
  }
  return `${hasGpu ? t("models.best.how.cpuWithGpu") : t("models.best.how.cpu")} ${tail}`;
}

function pcLine(cat: Catalog): string | null {
  if (!cat.pc) return null;
  const vram = cat.pc.vram_bytes > 0 ? t("models.best.vram", { size: wholeGb(cat.pc.vram_bytes) }) : t("models.best.noGpu");
  const facts = [cat.pc.gpu, vram, t("models.best.ram", { size: wholeGb(cat.pc.ram_bytes) })].filter(Boolean).join(" · ");
  return cat.pc.source === "scan" ? t("models.best.lastScan", { facts }) : facts;
}

export function Best({ cat, busy, uncensored, setUncensored }: { cat: Catalog | null; busy: boolean; uncensored: boolean; setUncensored: (on: boolean) => void }) {
  const line = cat && pcLine(cat);
  const [shown, setShown] = useState<Purpose>("roleplay");
  const best = cat?.best ?? null;
  const all = best ? [...best.roleplay, ...best.tempo, ...best.context] : null;
  // No model fits at all: one sentence instead of three empty columns.
  const none = best !== null && all!.length === 0 && PURPOSES.every((pu) => best.empty?.[pu.id] === "nothing_fits");
  return (
    <div className="set-row">
      <div className="set-label">
        <h2>{t("models.best.title")}</h2>
        <p>
          {cat === null
            ? t("models.best.reading")
            : line
              ? `${line}. ${howText(all, cat.vram_budget_gb, cat.pc!.vram_bytes > 0)}`
              : t("models.best.unread")}
        </p>
      </div>
      <div className="set-panel" data-spot="best-models" aria-busy={busy || undefined}>
        <label className="switch-row sug-uncensored" data-spot="best-uncensored">
          <span className="sw-text">
            <b>{t("models.best.uncensored")}</b>
            <span>{t("models.best.uncensored.note")}</span>
          </span>
          <input type="checkbox" role="switch" className="switch" checked={uncensored} onChange={(e) => setUncensored(e.target.checked)} />
        </label>
        {cat === null ? (
          <div aria-busy="true">
            <span className="bar-ph" />
            <span className="bar-ph short" />
          </div>
        ) : !best ? (
          <p className="help">{t("models.best.noPc")}</p>
        ) : none ? (
          <p className={`sg-empty x8-empty${busy ? " sug-stale" : ""}`}>{emptyText("nothing_fits", uncensored)}</p>
        ) : (
          <div className={`best${busy ? " sug-stale" : ""}`}>
            <div className="seg best-tabs" role="group" aria-label={t("models.best.purpose")}>
              {PURPOSES.map((pu) => (
                <button key={pu.id} type="button" aria-pressed={shown === pu.id} onClick={() => setShown(pu.id)}>
                  {pu.name}
                </button>
              ))}
            </div>
            {PURPOSES.map((pu) => {
              const picks = best[pu.id];
              return (
                <section key={pu.id} className={`best-col${shown === pu.id ? " on" : ""}`} aria-label={pu.name}>
                  <h3>{pu.name}</h3>
                  <p className="best-what">{pu.what}</p>
                  {picks.length === 0 ? (
                    <p className="sg-empty x8-empty">{emptyText(best.empty?.[pu.id], uncensored)}</p>
                  ) : (
                    <ol className="best-list">
                      {picks.map((p, i) => {
                        const m = cat.models.find((x) => x.id === p.id);
                        const q = m?.quants[p.tier];
                        if (!m || !q) return null;
                        const place = placeLine(p);
                        return (
                          <li key={p.id} className="best-pick" aria-label={m.name}>
                            <span className="best-rank num">{i + 1}.</span>
                            <div className="best-body">
                              <b>{m.name}</b>
                              <span className="best-meta num">
                                {TIER_NAME[p.tier]} · {size(q.size_bytes)}
                              </span>
                              <span className="sug-rates">
                                <Dots label={t("models.rate.rp")} value={m.rp} />
                                <Dots label={t("models.rate.de")} value={m.de} />
                              </span>
                              <span className="best-why">{reasonText(p, m, cat.vram_budget_gb ?? 0)}</span>
                              {place && (
                                <span className="x8-place num" title={moeTitle(p.n_cpu_moe)}>
                                  {place}
                                </span>
                              )}
                              <GetButton m={m} tier={p.tier} q={q} small />
                            </div>
                          </li>
                        );
                      })}
                    </ol>
                  )}
                </section>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
