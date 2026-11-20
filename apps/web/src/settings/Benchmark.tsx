// The benchmark of one profile, inside its model settings: start it (up to five minutes, the model
// restarts several times), watch it, cancel it, then the table of every candidate and three
// recommendations (Tempo / Ausgewogen / Kontext), each with "Setzen" that writes it into the profile.
// Each row shows how far its runs spread, a stop names the limit with its numbers, and every result
// says what memory was free when it started; results measured under different conditions (apps open,
// apps closed) can be switched between, and "Setzen" takes the setting from the result shown.
// Recommendations that chose the same row share one card; a card's text comes from its row. A CPU
// run names only the RAM, steps the time did not reach are named, and a cancelled or failed run
// shows its rows apart from the stored result.

import { useCallback, useEffect, useState } from "react";
import {
  api,
  ApiError,
  ctxShort,
  errorText,
  gb,
  num,
  postJson,
  when,
  type BenchConditions,
  type BenchEvent,
  type BenchLimit,
  type BenchPurpose,
  type BenchResult,
  type BenchRow,
  type Profile,
} from "../api";
import { useBenchEvents } from "../app/benchEvents";
import { locale, t, texts } from "../i18n";
import { rich } from "../i18n/rich";
import { percent, withUnit } from "../models/words";
import { Icon } from "../ui";
import { PurposeAdvice } from "./Advice";
import { draftOf, FLAGS, withArg } from "./args";

/** The three recommendations' words, read in the language on screen each time (`texts`). */
const PURPOSE: Record<BenchPurpose, { readonly label: string; readonly note: string; readonly is: string }> = {
  speed: texts({}, { label: "models.bench.speed", note: "models.bench.speed.note", is: "models.bench.speed.is" }),
  balanced: texts({}, { label: "models.bench.balanced", note: "models.bench.balanced.note", is: "models.bench.balanced.is" }),
  context: texts({}, { label: "models.bench.context", note: "models.bench.context.note", is: "models.bench.context.is" }),
};
const ORDER: BenchPurpose[] = ["speed", "balanced", "context"];
/** Tempo looks at 8k and more first, and only without such a row at the smaller ones (`rp_core::bench::SPEED_MIN_CTX`). */
const SPEED_MIN_CTX = 8192;

/** "a", "a und b", "a, b und c" in the app's language. */
const and = (items: string[]) => new Intl.ListFormat(locale(), { style: "long", type: "conjunction" }).format(items);

/** One recommendation card: the purposes that chose this row (several when they chose the same). */
type Card = { purposes: BenchPurpose[]; row: BenchRow };

function cardsOf(res: BenchResult): Card[] {
  const out: (Card & { at: number })[] = [];
  for (const purpose of ORDER) {
    const pick = res.picks.find((x) => x.purpose === purpose);
    const row = pick ? res.rows[pick.row] : undefined;
    if (!pick || !row) continue;
    const same = out.find((c) => c.at === pick.row);
    if (same) same.purposes.push(purpose);
    else out.push({ purposes: [purpose], row, at: pick.row });
  }
  return out;
}

/** What a card says, from the row it shows: Tempo names 8k only when its row has that much, and
 *  purposes that chose the same row say so once. */
function cardNote(card: Card, res: BenchResult): string {
  const fitting = res.rows.filter((r) => r.outcome === "ok").length;
  if (card.purposes.length > 1) {
    if (fitting === 1) return t("models.bench.card.only");
    return t("models.bench.card.same", { list: and(card.purposes.map((p) => PURPOSE[p].is)) });
  }
  const purpose = card.purposes[0];
  if (purpose !== "speed" || card.row.ctx >= SPEED_MIN_CTX) return PURPOSE[purpose].note;
  if (res.rows.some((r) => r.ctx >= SPEED_MIN_CTX)) return t("models.bench.card.no8k");
  const largest = Math.max(...res.rows.map((r) => r.ctx));
  return t("models.bench.card.upTo", { ctx: ctxShort(largest) });
}

const OUTCOME: Record<BenchRow["outcome"], string> = texts(
  {},
  {
    ok: "models.bench.outcome.ok",
    overflow: "models.bench.outcome.overflow",
    ram: "models.bench.outcome.ram",
    shared: "models.bench.outcome.shared",
    skipped: "models.bench.outcome.skipped",
    failed: "models.bench.outcome.failed",
    time: "models.bench.outcome.time",
  },
);

const clock = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
const speed = (v: number | null) => (v === null ? "–" : num(Math.round(v * 10) / 10));
const kvOf = (args: string[]) => draftOf(args).ctk ?? "f16";
const mib = (n: number) => gb(n * 1024 * 1024);
const mb = (n: number) => withUnit(n, "megabyte", 0);

/** The outcome as the table says it; a stop for memory names which limit it hit. */
function outcomeText(r: BenchRow): string {
  if (r.outcome === "ram" && r.limit?.what === "commit") return t("models.bench.outcome.commit");
  if (r.outcome === "ram" && r.limit) return t("models.bench.outcome.ramLimit");
  if (r.outcome === "shared" && r.limit) return t("models.bench.outcome.sharedLimit");
  return OUTCOME[r.outcome];
}

/** The limit's numbers: "RAM: 2,1 GB verfügbar, 3,4 GB nötig" (in MB when both round to the same GB). */
function limitText(l: BenchLimit): string {
  const same = mib(l.available_mib) === mib(l.needed_mib);
  const [have, need] = same ? [mb(l.available_mib), mb(l.needed_mib)] : [mib(l.available_mib), mib(l.needed_mib)];
  if (l.what === "ram") return t("models.bench.limit.ram", { have, need });
  if (l.what === "commit") return t("models.bench.limit.commit", { have, need });
  return t("models.bench.limit.shared", { have, need });
}

/** The profile's start options with this row's context, KV cache and MoE layers (the others stay),
 *  the same way the server's apply writes them (`rp_core::bench::args_with`). */
function argsFor(args: string[], r: BenchRow): string[] {
  let out = withArg(args, [...FLAGS.ctx], String(r.ctx));
  out = withArg(out, [...FLAGS.ctk], r.kv);
  out = withArg(out, [...FLAGS.ctv], r.kv);
  return withArg(out, [...FLAGS.ncmoe], r.n_cpu_moe > 0 ? String(r.n_cpu_moe) : null);
}

const pickRow = (res: BenchResult, purpose: BenchPurpose) => {
  const pick = res.picks.find((x) => x.purpose === purpose);
  return pick ? res.rows[pick.row] : undefined;
};

type State = { running: BenchEvent | null; result: BenchResult | null; results?: BenchResult[] };

export function Benchmark({ p, moe, onChanged }: { p: Profile; moe: boolean; onChanged: () => Promise<void> }) {
  const [state, setState] = useState<State | null>(null);
  const [error, setError] = useState("");
  const [ended, setEnded] = useState<BenchEvent | null>(null); // the end of a run seen here
  const [starting, setStarting] = useState(false);
  const [applying, setApplying] = useState<BenchPurpose | null>(null);
  const [shownAt, setShownAt] = useState<number | null>(null); // the result shown (its `at`); null: the newest
  const [since, setSince] = useState(0); // local clock of the running benchmark's start
  const [, tick] = useState(0);

  const load = useCallback(async () => {
    try {
      setState(await api<State>(`/models/profiles/${p.id}/benchmark`));
    } catch (err) {
      setError(errorText(err));
    }
  }, [p.id]);
  useEffect(() => {
    void load();
  }, [load]);

  const onEvent = useCallback(
    (ev: BenchEvent) => {
      if (ev.profile_id !== p.id) return;
      setSince(Date.now() - ev.elapsed_s * 1000);
      if (ev.stage === "loading" || ev.stage === "measuring") {
        setState((s) => ({ ...s, result: s?.result ?? null, running: ev }));
      } else {
        setEnded(ev);
        setState((s) => ({ ...s, running: null, result: ev.stage === "done" && ev.result ? ev.result : (s?.result ?? null) }));
        // A new result replaces the one measured under similar conditions: read the kept list again.
        if (ev.stage === "done") {
          setShownAt(null);
          void load();
        }
      }
    },
    [p.id, load],
  );
  useBenchEvents(onEvent);

  const running = state?.running ?? null;
  useEffect(() => {
    if (!running) return;
    if (!since) setSince(Date.now() - running.elapsed_s * 1000);
    const timer = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(timer);
  }, [running, since]);

  const start = async () => {
    setStarting(true);
    setError("");
    setEnded(null);
    try {
      await postJson(`/models/profiles/${p.id}/benchmark`, {});
      setSince(Date.now());
      setState((s) => ({ ...s, result: s?.result ?? null, running: s?.running ?? { type: "bench", profile_id: p.id, stage: "loading", done: 0, total: 0, elapsed_s: 0, budget_s: 300, candidate: null, rows: [] } }));
    } catch (err) {
      setError(err instanceof ApiError && err.code === "unmanaged" ? t("models.bench.unmanaged") : errorText(err));
    } finally {
      setStarting(false);
    }
  };
  const cancel = async () => {
    try {
      await postJson(`/models/profiles/${p.id}/benchmark/cancel`, {});
    } catch (err) {
      setError(errorText(err));
    }
  };
  const latest = state?.result ?? null;
  const results = state?.results?.length ? state.results : latest ? [latest] : [];
  const shownIndex = Math.max(0, shownAt === null ? 0 : results.findIndex((r) => r.at === shownAt));
  const result = results[shownIndex] ?? latest;

  const apply = async (purpose: BenchPurpose) => {
    const row = result ? pickRow(result, purpose) : undefined;
    if (!row) return;
    setApplying(purpose);
    setError("");
    try {
      // The newest result is the server's own (it applies its pick); an older one shown here is
      // written into the profile the same way.
      if (shownIndex === 0) await postJson(`/models/profiles/${p.id}/benchmark/apply`, { purpose });
      else await postJson(`/models/profiles/${p.id}`, { load_args: argsFor(p.load_args, row), measured: true }, "PUT");
      await onChanged();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setApplying(null);
    }
  };

  // A run that was cancelled or found nothing keeps its own table, apart from the stored result's.
  const endedRows = !running && ended && ended.stage !== "done" ? ended.rows : [];
  // Is this row what the profile has now (set from a measurement)?
  const now = draftOf(p.load_args);
  const isSet = (r: BenchRow) => p.measured && Number(now.ctx) === r.ctx && kvOf(p.load_args) === r.kv && Number(now.ncmoe ?? 0) === r.n_cpu_moe;

  return (
    <section className="bm" data-spot="benchmark" aria-label={t("models.bench.what")}>
      <PurposeAdvice p={p} measuredAt={latest?.at ?? null} running={running !== null} onChanged={onChanged} />
      <div className="bm-head">
        <div className="bm-intro">
          <h3 className="ms-sub">{t("models.bench.title")}</h3>
          <p className="help">{moe ? t("models.bench.intro.moe") : t("models.bench.intro")}</p>
        </div>
        {running ? (
          <button type="button" className="btn sm" onClick={() => void cancel()}>
            {t("models.dl.cancel")}
          </button>
        ) : (
          <button type="button" className="btn sm solid" data-spot-focus disabled={starting || state === null} onClick={() => void start()}>
            <Icon name="chart" small />
            {result ? t("models.bench.again") : t("models.bench.start")}
          </button>
        )}
      </div>

      {running && <Progress ev={running} since={since} />}
      {error && (
        <p className="bad bm-note" role="alert">
          {error}
        </p>
      )}
      {!running && ended?.stage === "cancelled" && <p className="help bm-note">{t("models.bench.cancelled")}</p>}
      {!running && ended?.stage === "failed" && <p className="bad bm-note">{t("models.bench.failed")}</p>}
      {running && running.rows.length > 0 && <Table rows={running.rows} moe={moe} />}
      {endedRows.length > 0 && (
        <div className="r2-run" role="group" aria-label={ended?.stage === "cancelled" ? t("models.bench.run.cancelled") : t("models.bench.run.none")}>
          <p className="help bm-note">{ended?.stage === "cancelled" ? t("models.bench.run.cancelled.note") : t("models.bench.run.none.note")}</p>
          <Table rows={endedRows} moe={moe} legend={!result} />
        </div>
      )}

      {!running && result && (
        <>
          {results.length > 1 && <Results results={results} shown={shownIndex} onShow={(r) => setShownAt(r.at)} />}
          <div className="x5-about">
            <p className="help bm-note">
              {t(shownIndex === 0 ? "models.bench.about.latest" : "models.bench.about", { when: when(result.at), time: clock(result.duration_s), gpu: result.gpu ?? "CPU" })}
              {result.prompt_tokens != null && result.gen_tokens != null && (
                <span className="num">
                  {" "}
                  · {t("models.bench.perRun", { read: num(result.prompt_tokens), wrote: num(result.gen_tokens) })}
                </span>
              )}
            </p>
            {result.conditions && <Conditions c={result.conditions} cpu={result.gpu === null} />}
            {result.unmeasured_stages && result.unmeasured_stages.length > 0 && (
              <p className="help r2-unmeasured">{rich("models.bench.unmeasured", { list: <span className="num">{and(result.unmeasured_stages.map(ctxShort))}</span> })}</p>
            )}
          </div>
          <div className="bm-picks">
            {cardsOf(result).map((card) => {
              const r = card.row;
              const set = isSet(r);
              const label = card.purposes.map((x) => PURPOSE[x].label).join(" · ");
              const purpose = card.purposes[0];
              return (
                <div key={card.purposes.join("-")} className={`bm-pick${set ? " on" : ""}`} data-purpose={purpose} data-purposes={card.purposes.join(" ")}>
                  <span className="bm-pick-label">{label}</span>
                  <b className="bm-pick-tg num">{t("models.bench.tokS", { v: speed(r.tg) })}</b>
                  <span className="bm-pick-facts num">
                    {t("models.advice.ctx", { ctx: ctxShort(r.ctx) })} · {r.kv}
                    {r.n_cpu_moe > 0 ? ` · ${t("models.advice.moe", { n: r.n_cpu_moe })}` : ""}
                    {r.uncertain && <span className="x5-unsure"> · {t("models.bench.unsure")}</span>}
                  </span>
                  <span className="help">{cardNote(card, result)}</span>
                  {set ? (
                    <span className="sm-stamp bm-set">
                      <Icon name="check" small />
                      {t("models.advice.set")}
                    </span>
                  ) : (
                    <button type="button" className="btn sm" disabled={applying !== null} onClick={() => void apply(purpose)} aria-label={t("models.bench.apply.aria", { label })}>
                      {applying === purpose ? t("models.advice.applying") : t("models.advice.apply")}
                    </button>
                  )}
                </div>
              );
            })}
          </div>
          {result.rows.length > 0 && <Table rows={result.rows} moe={moe} />}
        </>
      )}
      {state === null && !error && <span className="bar-ph short" aria-busy="true" />}
    </section>
  );
}

function Progress({ ev, since }: { ev: BenchEvent; since: number }) {
  const elapsed = since ? Math.max(0, (Date.now() - since) / 1000) : ev.elapsed_s;
  const share = Math.min(1, elapsed / Math.max(1, ev.budget_s));
  const c = ev.candidate;
  return (
    <div className="bm-progress" role="status" aria-label={t("models.bench.running")}>
      <div className="bm-progress-line">
        <span>
          <i className="dot loading" />
          {c
            ? rich("models.bench.run", {
                n: num(ev.done + 1),
                what: (
                  <span className="num">
                    {ctxShort(c.ctx)} · {c.kv}
                    {c.n_cpu_moe > 0 ? ` · ${t("models.advice.moe", { n: c.n_cpu_moe })}` : ""}
                  </span>
                ),
              })
            : t("models.bench.preparing")}
        </span>
        <span className="num faint">{t("models.bench.clock", { elapsed: clock(elapsed), budget: clock(ev.budget_s) })}</span>
      </div>
      <i className="bm-bar" aria-hidden="true">
        <b style={{ width: `${Math.round(share * 100)}%` }} />
      </i>
    </div>
  );
}

/** What memory was free when the result's benchmark started; a CPU run names the RAM only. */
function Conditions({ c, cpu }: { c: BenchConditions; cpu: boolean }) {
  const parts = [
    c.ram_available_mib !== null && t("models.bench.cond.ram", { size: mib(c.ram_available_mib) }),
    !cpu && c.vram_free_mib !== null && t("models.bench.cond.vram", { size: mib(c.vram_free_mib) }),
  ].filter(Boolean);
  if (parts.length === 0) return null;
  return <p className="help x5-cond">{rich("models.bench.cond", { parts: <span className="num">{parts.join(" · ")}</span> })}</p>;
}

/** Every kept result, one per kind of conditions: pick the one to show, with what each had free and its Tempo. */
function Results({ results, shown, onShow }: { results: BenchResult[]; shown: number; onShow: (r: BenchResult) => void }) {
  return (
    <div className="x5-results" role="group" aria-label={t("models.bench.results")}>
      {results.map((r, i) => {
        const c = r.conditions;
        const tempo = pickRow(r, "speed");
        return (
          <button key={r.at} type="button" className="x5-result" aria-pressed={i === shown} onClick={() => onShow(r)}>
            <span className="x5-result-when">{when(r.at)}</span>
            <span className="x5-result-mem num">
              {c?.ram_available_mib != null ? t("models.bench.results.ram", { size: mib(c.ram_available_mib) }) : t("models.bench.results.ramUnknown")}
              {r.gpu !== null && c?.vram_free_mib != null ? ` · ${t("models.bench.results.vram", { size: mib(c.vram_free_mib) })}` : ""}
            </span>
            <span className="x5-result-tg num">{tempo ? t("models.bench.results.tempo", { v: speed(tempo.tg) }) : t("models.bench.results.none")}</span>
          </button>
        );
      })}
    </div>
  );
}

function Table({ rows, moe: moeModel, legend = true }: { rows: BenchRow[]; moe: boolean; legend?: boolean }) {
  const moe = moeModel || rows.some((r) => r.n_cpu_moe > 0);
  const commit = rows.some((r) => r.limit?.what === "commit");
  // With any uncertain row, the others keep the same room after their spread, so the figures share one right edge.
  const unsure = rows.some((r) => r.uncertain);
  return (
    <>
      <div className="bm-table-wrap">
        <table className="bm-table num">
          <thead>
            <tr>
              <th scope="col">{t("models.sug.ctx")}</th>
              <th scope="col">KV</th>
              {moe && <th scope="col">{t("models.bench.col.moe")}</th>}
              <th scope="col">pp</th>
              <th scope="col">tg</th>
              <th scope="col">{t("models.bench.col.spread")}</th>
              <th scope="col">{t("models.bench.col.overflow")}</th>
              <th scope="col">{t("models.bench.col.fits")}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r, i) => (
              <tr key={i} className={[r.outcome === "ok" ? "" : "off", r.limit ? "x5-has-why" : ""].join(" ").trim() || undefined}>
                <td>{ctxShort(r.ctx)}</td>
                <td>{r.kv}</td>
                {moe && <td>{num(r.n_cpu_moe)}</td>}
                <td>{r.pp === null ? "–" : num(Math.round(r.pp))}</td>
                <td>{speed(r.tg)}</td>
                <td className="x5-spread" title={r.tg_runs?.length ? t("models.bench.runs.title", { list: r.tg_runs.map(speed).join(" · ") }) : undefined}>
                  {r.spread == null ? "–" : percent(Math.round(r.spread * 100))}
                  {r.uncertain ? <span className="x5-unsure">{t("models.bench.unsure")}</span> : unsure && <span className="x5-unsure-slot" aria-hidden="true" />}
                </td>
                <td>{r.overflow_mib === null ? "–" : t("models.bench.mib", { n: num(Math.max(0, r.overflow_mib)) })}</td>
                <td className={r.outcome === "ok" ? "ok" : undefined}>
                  {r.outcome === "ok" && <Icon name="check" small />}
                  {outcomeText(r)}
                  {r.limit && <span className="x5-why">{limitText(r.limit)}</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {legend && (
        <p className="help x5-legend">
          {t("models.bench.legend")}
          {commit ? ` ${t("models.bench.legend.commit")}` : ""}
        </p>
      )}
    </>
  );
}
