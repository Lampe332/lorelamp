// "Was willst du mit dem Modell machen?": one sentence (German or English), the needs the app read
// from it as chips the user can switch, and the catalog's answer for this PC: the best five with a
// reason each. `PurposeField` is the sentence with its chips alone ("Beste Einstellung finden" shows
// it too); the stored purpose is one for the whole app.

import { useId } from "react";
import { t } from "../i18n";
import { Icon } from "../ui";
import { Dots, GetButton } from "./pieces";
import type { usePurpose } from "./purposeApi";
import { size, TIER_NAME } from "./store";
import type { Catalog, Need } from "./types";
import { caveatLine, moeTitle, NEED_NAME, placeText, reasonLine } from "./words";
import { ctxShort } from "../api";
import "./purpose.css";

type Field = ReturnType<typeof usePurpose>;

/** The sentence and its chips. `label` names the field for this place. */
export function PurposeField({ field, label, placeholder }: { field: Field; label: string; placeholder: string }) {
  const id = useId();
  const { p, text, error, type, toggle, clear } = field;
  const hit = (n: Need) => p?.hits.find((h) => h.need === n)?.phrase;
  // What the sentence said and the user switched off, or did not say and the user switched on.
  const corrected = p ? p.all.filter((n) => p.needs.includes(n) !== p.parsed.includes(n)).length : 0;
  return (
    <div className="pu-field">
      <label className="pu-label" htmlFor={id}>
        {label}
      </label>
      <div className="pu-input">
        <textarea id={id} className="input pu-text" rows={2} maxLength={500} value={text} placeholder={placeholder} onChange={(e) => type(e.target.value)} spellCheck={false} />
        {text !== "" && (
          <button type="button" className="icon-btn pu-clear" aria-label={t("models.purpose.clear")} onClick={clear}>
            <Icon name="close" small />
          </button>
        )}
      </div>
      {error && (
        <p className="bad" role="alert">
          {error}
        </p>
      )}
      <fieldset className="pu-needs" disabled={!p}>
        <legend>{p && p.needs.length > 0 ? t("models.purpose.matters") : t("models.purpose.orPick")}</legend>
        <div className="pu-chips">
          {(p?.all ?? []).map((n) => {
            const on = p!.needs.includes(n);
            const why = hit(n);
            return (
              <button key={n} type="button" className="fchip pu-chip" aria-pressed={on} title={on && why ? t("models.purpose.found", { phrase: why }) : undefined} onClick={() => toggle(n)}>
                {on && <Icon name="check" small />}
                {NEED_NAME[n]}
              </button>
            );
          })}
        </div>
        <p className="help pu-hint" role="status">
          {!p
            ? ""
            : p.needs.length === 0
              ? text.trim() === ""
                ? t("models.purpose.hint.empty")
                : t("models.purpose.hint.nothing")
              : corrected > 0
                ? t("models.purpose.hint.corrected")
                : t("models.purpose.hint.read")}
        </p>
      </fieldset>
    </div>
  );
}

/** The row in the tab "Vorschläge": the field and the answer for this PC. */
export function PurposeRow({ cat, field, busy }: { cat: Catalog | null; field: Field; busy: boolean }) {
  const needs = field.p?.needs ?? [];
  const answer = cat?.for_purpose ?? null;
  // The answer on screen belongs to the needs on screen once the catalog carries the same ones.
  const fresh = cat?.purpose !== undefined && cat.purpose.needs.join() === needs.join();
  const top = answer ? answer.top.map((id) => answer.ranked.find((s) => s.id === id)).filter((s) => s !== undefined) : [];
  return (
    <div className="set-row">
      <div className="set-label">
        <h2>{t("models.purpose.title")}</h2>
        <p>{t("models.purpose.note")}</p>
      </div>
      <div className="set-panel" data-spot="model-purpose">
        <PurposeField field={field} label={t("models.purpose.label")} placeholder={t("models.purpose.placeholder")} />
        {needs.length > 0 &&
          (cat === null ? (
            <div aria-busy="true">
              <span className="bar-ph" />
              <span className="bar-ph short" />
            </div>
          ) : !answer ? (
            <p className="help">{t("models.purpose.noPc")}</p>
          ) : top.length === 0 ? (
            <p className="sg-empty">{t("models.purpose.none")}</p>
          ) : (
            <section className={`pu-answer${busy || !fresh ? " sug-stale" : ""}`} aria-label={t("models.purpose.answer")} aria-busy={busy || !fresh || undefined}>
              <h3 className="pu-title">{t("models.purpose.answer")}</h3>
              <ol className="pu-list">
                {top.map((s, i) => {
                  const m = cat.models.find((x) => x.id === s.id);
                  const q = m?.quants[s.tier];
                  if (!m || !q) return null;
                  const where = placeText(s.fit, s.gpu_gb, s.ram_gb);
                  const caveat = caveatLine(s);
                  return (
                    <li key={s.id} className="pu-pick" aria-label={m.name}>
                      <span className="pu-rank num">{i + 1}.</span>
                      <div className="pu-body">
                        <div className="pu-head">
                          <b>{m.name}</b>
                          <span className="best-meta num">
                            {TIER_NAME[s.tier]} · {size(q.size_bytes)}
                          </span>
                        </div>
                        <span className="sug-rates">
                          <Dots label={t("models.rate.rp")} value={m.rp} />
                          <Dots label={t("models.rate.de")} value={m.de} />
                        </span>
                        <p className="pu-why">{reasonLine(s)}</p>
                        {caveat && <p className="pu-but">{caveat}</p>}
                        <span className="x8-place num" title={moeTitle(s.n_cpu_moe)}>
                          {[where, t("models.ctxUpTo", { ctx: ctxShort(s.ctx_max) })].filter(Boolean).join(" · ")}
                        </span>
                      </div>
                      <div className="pu-act">
                        <GetButton m={m} tier={s.tier} q={q} small />
                      </div>
                    </li>
                  );
                })}
              </ol>
              <p className="help">{t("models.purpose.order")}</p>
            </section>
          ))}
      </div>
    </div>
  );
}
