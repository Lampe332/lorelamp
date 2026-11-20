// Settings tab "Vorschläge": the best models for this PC per purpose (Rollenspiel / Tempo / Kontext,
// `Best.tsx`), the curated catalog as cards with a foldable filter column (`FilterPanel.tsx`, a sheet
// on a phone), the live search "Mehr finden" and the field for a pasted Hugging Face link. Downloads
// start here and show in the panel at the top (and as a badge in the sidebar).

import { useCallback, useEffect, useId, useMemo, useRef, useState, type FormEvent } from "react";
import { api, ctxShort, num, postJson } from "../api";
import { useModelNews } from "../app/engineEvents";
import { lang, t } from "../i18n";
import { respot, useSpotAsked } from "../spot";
import { Icon } from "../ui";
import { Best } from "./Best";
import { DownloadsPanel, HfLink, TierPicker } from "./Downloads";
import { activeCount, keep, loadKept, NO_FILTERS, ORDER, passingTiers, pcKnown, saveKept, SORTS, type Filters, type Sort } from "./filters";
import { FilterPanel } from "./FilterPanel";
import { Dots, FitPill, GetButton, usePhone } from "./pieces";
import { PurposeRow } from "./Purpose";
import { usePurpose } from "./purposeApi";
import { dateShort, errorOf, size, TIER_NAME, TIERS, useCatalogNews, useDownloads } from "./store";
import type { Catalog, CatalogModel, Resolved, SearchHit, Suggestion, Tier } from "./types";
import { caveatLine, fitName, reasonLine } from "./words";

const params = (m: CatalogModel) =>
  m.params_total_b === null
    ? null
    : m.moe && m.params_active_b
      ? `${t("models.filter.billions", { b: num(m.params_total_b) })} · ${t("models.sug.active", { b: num(m.params_active_b) })}`
      : t("models.filter.billions", { b: num(m.params_total_b) });

/** The tier a card shows first: the biggest that fits entirely, else balanced. */
function firstTier(m: CatalogModel): Tier {
  const fit = [...TIERS].reverse().find((t) => m.quants[t]?.fit === "fits" && !m.quants[t]?.missing);
  return fit ?? (m.quants.balanced ? "balanced" : (TIERS.find((t) => m.quants[t]) ?? "balanced"));
}

export function Suggestions() {
  const [uncensored, setUncensored] = useState(() => loadKept().uncensored);
  const [cat, setCat] = useState<Catalog | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  // The newest request wins: a slow answer for the switch's old position never replaces a newer one.
  const asked = useRef(0);
  const load = useCallback(async () => {
    const n = ++asked.current;
    setBusy(true);
    try {
      const c = await api<Catalog>(uncensored ? "/models/catalog?uncensored=1" : "/models/catalog");
      if (n !== asked.current) return;
      setCat(c);
      setError("");
    } catch (e) {
      if (n === asked.current) setError(errorOf(e));
    } finally {
      if (n === asked.current) setBusy(false);
    }
  }, [uncensored]);
  useEffect(() => {
    void load();
  }, [load]);
  useCatalogNews(load);
  // A finished download changes what is installed.
  useModelNews(useCallback((n) => n.type === "models" && void load(), [load]));
  const arrived = cat !== null;
  useEffect(() => {
    if (arrived) respot();
  }, [arrived]);
  const { list } = useDownloads();
  // A stored purpose changes the catalog's order: read it again.
  const purpose = usePurpose(load);
  const pickUncensored = (on: boolean) => {
    setUncensored(on);
    saveKept({ ...loadKept(), uncensored: on });
  };

  return (
    <div className="view x2-view">
      <div className="page set sg">
        {list && list.length > 0 && (
          <div className="set-row">
            <div className="set-label">
              <h2>{t("models.dl.title")}</h2>
              <p>{t("models.dl.note")}</p>
            </div>
            <div className="set-panel" data-spot="downloads">
              <DownloadsPanel empty={null} />
            </div>
          </div>
        )}
        {error && !cat && (
          <p className="bad">
            {t("models.sug.loadFailed", { error })}{" "}
            <button type="button" className="btn sm ghost" onClick={() => void load()}>
              {t("models.sug.reload")}
            </button>
          </p>
        )}
        <PurposeRow cat={cat} field={purpose} busy={busy && cat !== null} />
        <Best cat={cat} busy={busy && cat !== null} uncensored={uncensored} setUncensored={pickUncensored} />
        <CatalogCards cat={cat} />
        <div className="set-row">
          <div className="set-label">
            <h2>{t("models.find.title")}</h2>
            <p>{t("models.find.note")}</p>
          </div>
          <div className="set-panel" data-spot="find-more">
            <FindMore />
          </div>
        </div>
        <div className="set-row">
          <div className="set-label">
            <h2>{t("models.hf.title")}</h2>
            <p>{t("models.hf.note")}</p>
          </div>
          <div className="set-panel">
            <HfLink />
          </div>
        </div>
      </div>
    </div>
  );
}


function CatalogCards({ cat }: { cat: Catalog | null }) {
  const phone = usePhone();
  const [kept] = useState(loadKept);
  const [f, setF] = useState<Filters>(kept.filters);
  const [sort, setSort] = useState<Sort>(kept.sort);
  // Open by default on wide windows, closed on a phone (there it is a sheet over the page).
  const [open, setOpen] = useState(() => (phone ? false : (kept.open ?? true)));
  const [spotAsked, spotDone] = useSpotAsked("model-filter");
  const sheet = useRef<HTMLDialogElement>(null);
  const sortId = useId();
  const sideId = useId();

  const remember = (next: Partial<{ filters: Filters; sort: Sort; open: boolean }>) => saveKept({ ...loadKept(), ...next });
  const putF = (next: Filters) => {
    setF(next);
    remember({ filters: next });
  };
  const putOpen = (on: boolean) => {
    setOpen(on);
    if (!phone) remember({ open: on });
    // Closed by hand: a later Ctrl+K "Filter" opens them again.
    if (!on) spotDone();
  };
  // Ctrl+K "Filter": open them; the spot then points at them.
  useEffect(() => {
    if (!spotAsked) return;
    setOpen(true);
    requestAnimationFrame(() => respot());
  }, [spotAsked]);
  // The phone sheet follows `open`.
  useEffect(() => {
    const d = sheet.current;
    if (!d) return;
    if (phone && open && !d.open) d.showModal();
    else if ((!phone || !open) && d.open) d.close();
  }, [phone, open]);

  const budget = cat?.vram_budget_gb ?? null;
  const bases = useMemo(() => [...new Set((cat?.models ?? []).map((m) => m.base))].sort(), [cat]);
  const licenses = useMemo(() => {
    const all = [...new Set((cat?.models ?? []).map((m) => m.license ?? ""))];
    return all.sort((a, b) => (a === "" ? 1 : b === "" ? -1 : a.localeCompare(b)));
  }, [cat]);
  // With a purpose, "Empfehlung" is the server's order for it (models that do not run on this PC
  // come last, by quality), and every card says why it stands where it stands.
  const forPurpose = useMemo(() => {
    if (!cat?.purpose?.needs.length || !cat.for_purpose) return null;
    return new Map<string, { at: number; s: Suggestion }>(cat.for_purpose.ranked.map((s, at) => [s.id, { at, s }]));
  }, [cat]);
  const shown = useMemo(() => {
    if (!cat) return [];
    const list = cat.models.filter((m) => keep(m, f));
    if (sort !== "rec" || !forPurpose) return list.sort(ORDER[sort]);
    const at = (m: CatalogModel) => forPurpose.get(m.id)?.at ?? Infinity;
    return list.sort((a, b) => (at(a) === at(b) ? ORDER.rec(a, b) : at(a) - at(b)));
  }, [cat, f, sort, forPurpose]);
  // How the filter "passt" decides: the pills of a known PC (with or without a GPU), else the slider.
  const pc = cat && pcKnown(cat.models) ? (budget && budget > 0 ? "gpu" : "cpu") : null;
  const count = activeCount(f);
  const stand = !cat
    ? ""
    : cat.syncing
      ? t("models.sug.syncing")
      : cat.live > 0
        ? t("models.sug.live", { live: cat.live, all: cat.models.length })
        : t("models.sug.offline", { date: dateShort(cat.generated) });
  // The place Ctrl+K's "Filter" points at, once its fields are there (respot() follows the catalog).
  const spotName = cat ? "model-filter" : undefined;
  const panel = cat && <FilterPanel f={f} set={putF} pc={pc} bases={bases} licenses={licenses} />;

  return (
    <div className="set-row">
      <div className="set-label">
        <h2>{t("models.sug.title")}</h2>
        <p>{t("models.sug.note")}</p>
        {stand && <p className="sg-stand">{stand}</p>}
      </div>
      <div className="set-panel sug-panel" data-spot="suggestions">
        <div className="sg-bar sug-bar">
          <button type="button" className="btn sm sug-toggle" aria-expanded={open} aria-controls={phone ? undefined : sideId} onClick={() => putOpen(!open)} disabled={!cat}>
            <Icon name="sliders" small />
            {t("models.sug.filter")}
            {count > 0 && <span className="sug-badge num">{num(count)}</span>}
          </button>
          <span className="sg-count sug-count num" role="status">
            {cat ? t("models.sug.count", { shown: shown.length, all: cat.models.length }) : ""}
          </span>
          {count > 0 && (
            <button type="button" className="btn sm ghost" aria-label={t("models.sug.resetFilters")} onClick={() => putF(NO_FILTERS)}>
              {phone ? t("models.sug.reset") : t("models.sug.resetFilters")}
            </button>
          )}
          <label className="sg-sort" htmlFor={sortId}>
            {t("models.sug.sort")}
            <select
              id={sortId}
              className="input"
              value={sort}
              onChange={(e) => {
                const s = e.target.value as Sort;
                setSort(s);
                remember({ sort: s });
              }}
            >
              {SORTS.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className={`sug-body${open && !phone ? " with-side" : ""}`}>
          {open && !phone && (
            <aside id={sideId} className="sug-side" aria-label={t("models.sug.filter")} data-spot={spotName}>
              {panel}
            </aside>
          )}
          {cat === null ? (
            <div aria-busy="true">
              <span className="bar-ph" />
              <span className="bar-ph short" />
            </div>
          ) : shown.length === 0 ? (
            <p className="sg-empty">{t("models.sug.none")}</p>
          ) : (
            <ul className="sg-cards x2-list" aria-label={t("models.sug.title")}>
              {shown.map((m) => {
                const pass = f.fit || f.tiers.length ? passingTiers(m, f) : [];
                const mine = forPurpose?.get(m.id)?.s;
                const pref = pass[0] ?? mine?.tier ?? firstTier(m);
                return <Card key={`${m.id}:${pref}`} m={m} first={pref} why={mine} />;
              })}
            </ul>
          )}
        </div>
        {phone && (
          <dialog ref={sheet} className="sug-sheet" aria-label={t("models.sug.filter")} onClose={() => phone && putOpen(false)}>
            <div className="sug-sheet-head">
              <h2>{t("models.sug.filter")}</h2>
              <button type="button" className="icon-btn" aria-label={t("models.sug.closeFilters")} onClick={() => putOpen(false)}>
                <Icon name="close" />
              </button>
            </div>
            {/* Only while open: a closed sheet must not answer Ctrl+K's "Filter" (the spot would land on hidden fields). */}
            {open && (
              <div className="sug-sheet-body" data-spot={spotName}>
                {panel}
              </div>
            )}
            <div className="sug-sheet-foot">
              {count > 0 && (
                <button type="button" className="btn ghost" onClick={() => putF(NO_FILTERS)}>
                  {t("models.sug.reset")}
                </button>
              )}
              <button type="button" className="btn primary" onClick={() => putOpen(false)}>
                {cat ? t("models.sug.showHits", { n: shown.length }) : t("models.sug.done")}
              </button>
            </div>
          </dialog>
        )}
      </div>
    </div>
  );
}

function Card({ m, first, why }: { m: CatalogModel; first: Tier; why?: Suggestion }) {
  const but = why ? caveatLine(why) : null;
  const [tier, setTier] = useState<Tier>(first);
  const q = m.quants[tier];
  const p = params(m);
  const language = lang();
  const german = language === "de";
  const writing = german ? m.writing : m.writing_tr[language];
  return (
    <li className="sg-card" aria-label={m.name}>
      <div className="sg-head">
        <b className="sg-name">{m.name}</b>
        <span className="sg-sub">{[m.family, p].filter(Boolean).join(" · ")}</span>
      </div>
      <div className="sug-rates">
        <Dots label={t("models.rate.rp")} value={m.rp} />
        <Dots label={t("models.rate.de")} value={m.de} />
        {m.confidence === "low" && (
          <span className="sug-unsure" title={t("models.sug.unsure.title")}>
            {t("models.sug.unsure")}
          </span>
        )}
      </div>
      {/* The catalog carries its note in every language (crates/models/catalog/models.json). */}
      {writing && <p className="sug-writing">{writing}</p>}
      {why && (
        <p className="pu-card-why" data-purpose-why>
          <span>{t("models.sug.forPurpose")} </span>
          {reasonLine(why)}
          {but && <span> {but}</span>}
        </p>
      )}
      <dl className="sg-facts">
        <div>
          <dt>{t("models.sug.ctx")}</dt>
          <dd className="num">{m.context_max ? ctxShort(m.context_max) : "–"}</dd>
        </div>
        <div>
          <dt>{t("models.sug.build")}</dt>
          <dd>{m.moe ? "MoE" : t("models.sug.dense")}</dd>
        </div>
        <div>
          <dt>{t("models.sug.reasoning")}</dt>
          <dd>{m.reasoning ? t("models.sug.yes") : t("models.sug.no")}</dd>
        </div>
      </dl>
      <div className="tp-tiers sg-tiers" role="radiogroup" aria-label={t("models.sug.sizeOf", { name: m.name })}>
        {TIERS.map((t) => {
          const x = m.quants[t];
          return (
            <button key={t} type="button" role="radio" aria-checked={tier === t} className="tp-tier" disabled={!x} onClick={() => setTier(t)}>
              <b>{TIER_NAME[t]}</b>
              <span className="num">
                {x?.fit && <i className={`fit-dot fit-${x.fit}`} title={fitName(x) ?? undefined} />}
                {x ? size(x.size_bytes) : "–"}
              </span>
            </button>
          );
        })}
      </div>
      {q && (
        <div className="sg-act">
          <FitPill q={q} />
          <GetButton m={m} tier={tier} q={q} small />
        </div>
      )}
      {m.sources.length > 0 && (
        <div className="sug-sources">
          <span>{t("models.sug.sources")}</span>
          <ul>
            {m.sources.map((s) => (
              <li key={s.url + s.label}>
                <a href={s.url} target="_blank" rel="noreferrer" className="link" title={german ? s.label : s.label_en} lang={german ? undefined : "en"}>
                  {german ? s.label : s.label_en}
                </a>
              </li>
            ))}
          </ul>
        </div>
      )}
      <p className="sg-src">
        <a href={`https://huggingface.co/${m.repo}`} target="_blank" rel="noreferrer" className="link" title={m.repo}>
          {m.repo.split("/")[0]}
        </a>
        {q && <span className="num"> · {q.quant}</span>}
        {m.license && <span> · {m.license}</span>}
        <span className="sg-checked">{t("models.sug.checked", { date: dateShort(m.checked) })}</span>
      </p>
    </li>
  );
}

function FindMore() {
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [open, setOpen] = useState<{ id: string; r: Resolved | null; error: string } | null>(null);
  const id = useId();
  const search = async (e: FormEvent) => {
    e.preventDefault();
    if (q.trim().length < 2) return;
    setBusy(true);
    setError("");
    setOpen(null);
    try {
      setHits((await api<{ results: SearchHit[] }>(`/models/search?q=${encodeURIComponent(q.trim())}`)).results);
    } catch (err) {
      setError(errorOf(err));
    } finally {
      setBusy(false);
    }
  };
  const show = async (hit: SearchHit) => {
    if (open?.id === hit.id) return setOpen(null);
    setOpen({ id: hit.id, r: null, error: "" });
    try {
      const r = await postJson<Resolved>("/models/resolve", { url: hit.id });
      setOpen((o) => (o?.id === hit.id ? { ...o, r } : o));
    } catch (err) {
      setOpen((o) => (o?.id === hit.id ? { ...o, error: errorOf(err) } : o));
    }
  };
  return (
    <>
      <form className="mp-inline" role="search" onSubmit={(e) => void search(e)}>
        <label className="sr" htmlFor={id}>
          {t("models.find.label")}
        </label>
        <input id={id} className="input" value={q} onChange={(e) => setQ(e.target.value)} placeholder={t("models.find.placeholder")} autoComplete="off" />
        <button type="submit" className="btn" disabled={busy || q.trim().length < 2}>
          <Icon name="search" small />
          {busy ? t("models.find.busy") : t("models.find.go")}
        </button>
      </form>
      {error && <p className="bad">{error}</p>}
      {hits && hits.length === 0 && <p className="help">{t("models.find.none")}</p>}
      {hits && hits.length > 0 && (
        <ul className="fm-list" aria-label={t("models.find.hits")}>
          {hits.map((h) => (
            <li key={h.id} className="fm-row">
              <div className="fm-top">
                <div className="fm-text">
                  <span className="fm-title">
                    <b>{h.id.split("/")[1]}</b>
                    <span className={`tag${h.curated ? " tag-on" : ""}`}>{h.curated ? t("models.find.curated") : t("models.find.notCurated")}</span>
                  </span>
                  <span className="fm-meta num">
                    {[
                      h.id.split("/")[0],
                      h.params ? t("models.filter.billions", { b: num(Math.round(h.params / 1e8) / 10) }) : null,
                      h.context ? t("models.find.ctx", { ctx: ctxShort(h.context) }) : null,
                      t("models.find.downloads", { n: h.downloads }),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </div>
                <button type="button" className="btn sm" aria-expanded={open?.id === h.id} onClick={() => void show(h)}>
                  {open?.id === h.id ? t("models.close") : t("models.showFiles")}
                </button>
              </div>
              {open?.id === h.id && (
                <div className="fm-open">
                  {open.error ? <p className="bad">{open.error}</p> : open.r ? <TierPicker key={open.r.repo} r={open.r} /> : <span className="bar-ph" />}
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
