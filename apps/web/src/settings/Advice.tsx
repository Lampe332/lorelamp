// "Beste Einstellung für deinen Zweck", above the benchmark of a profile: the purpose sentence with
// its chips, then what the app would set for it in two groups that say where each value comes from.
// "Gemessen": context, KV cache and MoE layers of the benchmark row that fits the needs (nothing
// before the first benchmark). "Aus deinem Zweck abgeleitet": sampler values with a reason each,
// and what the model's author recommends. "Setzen" writes start options and sampler into the profile.

import { useCallback, useEffect, useState } from "react";
import { ctxShort, errorText, num, when, type Profile, type Sampler } from "../api";
import { numberFormat, t, texts } from "../i18n";
import { PurposeField } from "../models/Purpose";
import { applyAdvice, getAdvice, usePurpose, type Advice, type Derived } from "../models/purposeApi";
import { Icon } from "../ui";
import { draftOf } from "./args";

/** The sampler values the advice names, read in the language on screen each time. */
const FIELD: Partial<Record<keyof Sampler, string>> = texts(
  {},
  {
    temperature: "models.advice.field.temperature",
    top_p: "models.advice.field.top_p",
    top_k: "models.advice.field.top_k",
    min_p: "models.advice.field.min_p",
    repeat_penalty: "models.advice.field.repeat_penalty",
    presence_penalty: "models.advice.field.presence_penalty",
    dry_multiplier: "models.advice.field.dry_multiplier",
    dry_base: "models.advice.field.dry_base",
    dry_allowed_length: "models.advice.field.dry_allowed_length",
    n_predict: "models.advice.field.n_predict",
  },
);

const PICK = {
  speed: texts({}, { name: "models.advice.pick.speed", why: "models.advice.pick.speed.why" }),
  balanced: texts({}, { name: "models.advice.pick.balanced", why: "models.advice.pick.balanced.why" }),
  context: texts({}, { name: "models.advice.pick.context", why: "models.advice.pick.context.why" }),
};

function valueText(d: Derived): string {
  if (d.field === "n_predict") return t("models.advice.tokens", { n: d.value });
  if (d.field === "dry_allowed_length") return t("models.advice.tokensInRow", { n: d.value });
  return numberFormat({ maximumFractionDigits: 3 }).format(d.value);
}

/** Why a derived value is proposed, in plain words. */
function whyText(d: Derived, figures: number | null): string {
  switch (d.why) {
    case "author":
      return t("models.advice.why.author");
    case "long_chats":
      return t("models.advice.why.long_chats");
    case "group":
      if (d.field === "n_predict") return figures ? t("models.advice.why.group.room", { n: figures }) : t("models.advice.why.group.roomSome");
      if (d.field === "dry_allowed_length") return t("models.advice.why.group.names");
      return t("models.advice.why.group");
    case "story":
      return t("models.advice.why.story");
    case "fast":
      return t("models.advice.why.fast");
    default:
      return t("models.advice.why.other");
  }
}

export function PurposeAdvice({ p, measuredAt, running, onChanged }: { p: Profile; measuredAt: number | null; running: boolean; onChanged: () => Promise<void> }) {
  const [advice, setAdvice] = useState<Advice | null>(null);
  const [error, setError] = useState("");
  const [applying, setApplying] = useState(false);
  const [done, setDone] = useState<"restarting" | "saved" | null>(null);

  const load = useCallback(async () => {
    try {
      setAdvice(await getAdvice(p.id));
      setError("");
    } catch (err) {
      setError(errorText(err));
    }
  }, [p.id]);
  // The advice follows the profile (after "Setzen" or an edit elsewhere) and a new measurement.
  const stamp = `${p.updated_at}:${measuredAt ?? 0}`;
  useEffect(() => {
    void load();
  }, [load, stamp]);
  const field = usePurpose(
    useCallback(() => {
      setDone(null);
      void load();
    }, [load]),
  );

  const apply = async () => {
    setApplying(true);
    setError("");
    try {
      const r = await applyAdvice(p.id);
      setAdvice({ ...r.advice, same: true });
      setDone(r.engine);
      await onChanged();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setApplying(false);
    }
  };

  const now = draftOf(p.load_args);
  const m = advice?.measured ?? null;
  const own = advice?.derived.filter((d) => d.why !== "author") ?? [];
  const author = advice?.derived.filter((d) => d.why === "author") ?? [];
  const isSet = (d: Derived) => p.sampler[d.field] === d.value;
  const fallback = m && advice && m.purpose !== advice.bench_purpose;

  return (
    <section className="pu-advice" data-spot="purpose-advice" aria-label={t("models.advice.title")}>
      <div className="pu-advice-head">
        <h3 className="ms-sub">{t("models.advice.title")}</h3>
        <p className="help">{t("models.advice.note")}</p>
      </div>
      <PurposeField field={field} label={t("models.advice.label")} placeholder={t("models.advice.placeholder")} />
      {advice && (
        <>
          <div className="pu-cols">
            <div className="pu-col" role="group" aria-label={t("models.advice.measured")}>
              <span className="pu-kind">{t("models.advice.measured")}</span>
              {m ? (
                <ul className="pu-facts">
                  <li className={Number(now.ctx) === m.ctx ? "set" : undefined}>
                    <b className="num">{t("models.advice.ctx", { ctx: ctxShort(m.ctx) })}</b>
                    <span>
                      {Number(now.ctx) === m.ctx && <i className="pu-is-set">{t("models.advice.isSet")} </i>}
                      {t("models.advice.pickLine", {
                        name: PICK[m.purpose].name,
                        why: PICK[m.purpose].why,
                        speed: m.tg !== null ? t("models.advice.speed", { tg: num(Math.round(m.tg * 10) / 10) }) : "",
                        unsure: m.uncertain ? t("models.advice.unsure") : "",
                      })}
                    </span>
                  </li>
                  <li className={(now.ctk ?? "f16") === m.kv ? "set" : undefined}>
                    <b className="num">{t("models.advice.kv", { kv: m.kv })}</b>
                    <span>
                      {(now.ctk ?? "f16") === m.kv && <i className="pu-is-set">{t("models.advice.isSet")} </i>}
                      {m.kv === "q8_0" ? t("models.advice.kv.q8") : t("models.advice.kv.f16")}
                    </span>
                  </li>
                  {m.n_cpu_moe > 0 && (
                    <li className={Number(now.ncmoe ?? 0) === m.n_cpu_moe ? "set" : undefined}>
                      <b className="num">{t("models.advice.moe", { n: m.n_cpu_moe })}</b>
                      <span>{t("models.advice.moe.note")}</span>
                    </li>
                  )}
                  <li>
                    <span>
                      {m.at ? t("models.advice.from", { when: when(m.at) }) : t("models.advice.fromLast")}
                      {fallback ? ` ${t("models.advice.fallback", { wanted: PICK[advice.bench_purpose].name, took: PICK[m.purpose].name })}` : ""}
                    </span>
                  </li>
                </ul>
              ) : (
                <p className="help">{t("models.advice.notMeasured")}</p>
              )}
            </div>
            <div className="pu-col" role="group" aria-label={t("models.advice.derived.aria")}>
              <span className="pu-kind">{t("models.advice.derived")}</span>
              {own.length === 0 ? (
                <p className="help">{advice.purpose.needs.length === 0 ? t("models.advice.derived.noPurpose") : t("models.advice.derived.none")}</p>
              ) : (
                <ul className="pu-facts">
                  {own.map((d) => (
                    <li key={d.field} className={isSet(d) ? "set" : undefined} data-field={d.field}>
                      <b className="num">
                        {FIELD[d.field] ?? d.field} {valueText(d)}
                      </b>
                      <span>
                        {isSet(d) && <i className="pu-is-set">{t("models.advice.isSet")} </i>}
                        {whyText(d, advice.purpose.figures)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            {author.length > 0 && (
              <div className="pu-col" role="group" aria-label={t("models.advice.author")}>
                <span className="pu-kind">{t("models.advice.author")}</span>
                <ul className="pu-facts">
                  {author.map((d) => (
                    <li key={d.field} className={isSet(d) ? "set" : undefined} data-field={d.field}>
                      <b className="num">
                        {FIELD[d.field] ?? d.field} {valueText(d)}
                      </b>
                      {isSet(d) && (
                        <span>
                          <i className="pu-is-set">{t("models.advice.isSet")}</i>
                        </span>
                      )}
                    </li>
                  ))}
                  {advice.author_source && (
                    <li>
                      <span>{t("models.advice.source", { source: advice.author_source.replace(/ model card.*$/, "") })}</span>
                    </li>
                  )}
                </ul>
              </div>
            )}
          </div>
          <div className="pu-apply">
            {advice.same ? (
              <span className="sm-stamp" role="status">
                <Icon name="check" small />
                {done === "restarting" ? t("models.advice.setRestart") : t("models.advice.set")}
              </span>
            ) : (
              <button type="button" className="btn sm solid" disabled={applying || running} onClick={() => void apply()} aria-label={t("models.advice.apply.aria")}>
                {applying ? t("models.advice.applying") : t("models.advice.apply")}
              </button>
            )}
            {!advice.same && <p className="help">{m ? t("models.advice.apply.note") : t("models.advice.apply.noteSampler")}</p>}
          </div>
        </>
      )}
      {error && (
        <p className="bad" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
