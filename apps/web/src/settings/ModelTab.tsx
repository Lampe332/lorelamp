// Settings tab "Modell", the simple one: the card "Dein Modell" (which model runs, whether it is
// loaded, how fast it wrote, its context) with the few things a normal user does: switch to another
// model ("Meine Modelle"), find the best start options (the benchmark), unload it or let it unload
// itself. Below only the downloads that run and the ways on: new models (tab "Vorschläge") and every
// setting exactly (tab "Experte", settings/Expert.tsx).

import { useEffect, useId, useRef, useState } from "react";
import { ctxShort, modelName, num, paramsShort } from "../api";
import { t } from "../i18n";
import { DownloadsPanel } from "../models/Downloads";
import { openDownloads, useDownloads } from "../models/store";
import { respot, useSpotAsked } from "../spot";
import { Icon } from "../ui";
import { problemLabel, problemOf } from "../app/engineProblem";
import { PcOnly } from "../phone/Phone";
import { Benchmark } from "./Benchmark";
import { EngineNote } from "./EngineNote";
import { LocalModelList } from "./LocalModels";
import { SwitchProgress } from "./switch";
import { MemoryLine } from "./Unload";
import { useModels, type Models } from "./useModels";

type Panel = "switch" | "bench";
type Shown = Record<Panel, boolean>;

export function ModelTab() {
  const m = useModels();
  const base = useId();
  // Each panel opens and closes on its own, so both can be open at once.
  const [switchOpen, setSwitchOpen] = useState(false);
  /** null: not toggled yet, so it follows a running benchmark (which opens it); false: closed on purpose. */
  const [benchOpen, setBenchOpen] = useState<boolean | null>(null);
  const benching = m.engine?.bench_profile_id != null;
  const shown: Shown = { switch: switchOpen, bench: benchOpen ?? benching };
  // Ctrl+K asks for a place inside a closed panel: open it, then the place is shown.
  const [askedList, doneList] = useSpotAsked("local-models");
  const [askedBench, doneBench] = useSpotAsked("benchmark");
  const done = useRef({ doneList, doneBench });
  useEffect(() => {
    done.current = { doneList, doneBench };
  });
  useEffect(() => {
    if (!askedList && !askedBench) return;
    if (askedList) setSwitchOpen(true);
    else setBenchOpen(true);
    respot();
    // The request is used up once the place was shown (two frames later).
    const timer = window.setTimeout(() => (askedList ? done.current.doneList() : done.current.doneBench()), 800);
    return () => window.clearTimeout(timer);
  }, [askedList, askedBench]);

  const toggle = (which: Panel) => {
    const next = !shown[which];
    if (which === "switch") setSwitchOpen(next);
    else setBenchOpen(next);
    if (!next) return;
    const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
    requestAnimationFrame(() => document.getElementById(`${base}-${which}`)?.scrollIntoView({ block: "nearest", behavior: calm ? "auto" : "smooth" }));
  };

  const { list } = useDownloads();
  const running = openDownloads(list).length > 0;

  return (
    <div className="view">
      <div className="page set mx-simple">
        {m.error && <p className="bad">{m.error}</p>}
        <Hero m={m} base={base} shown={shown} onToggle={toggle} />

        {shown.switch && (
          <section className="mx-panel" id={`${base}-switch`} aria-labelledby={`${base}-switch-h`} data-spot="local-models">
            <div className="mx-panel-head">
              <h2 id={`${base}-switch-h`}>{t("models.tab.mine")}</h2>
              <p>{t("models.tab.mine.note")}</p>
            </div>
            <LocalModelList data={m.local} error={m.localError} progress={m.progress} usePath={m.usePath} engine={m.engine} switching={m.switching} onUse={(x) => void m.use(x)} onChanged={m.loadAll} />
          </section>
        )}
        {shown.bench && m.active && (
          <section className="mx-panel" id={`${base}-bench`} aria-labelledby={`${base}-bench-h`}>
            <div className="mx-panel-head">
              <h2 id={`${base}-bench-h`}>{t("models.tab.bench")}</h2>
              <p>{t("models.tab.bench.note", { name: m.active.name })}</p>
            </div>
            {/* Another model file or engine in the same profile: nothing of the old measurement stays (the server forgets it too). */}
            <PcOnly what={t("models.bench.what")}>
              <Benchmark key={`${m.active.id}:${m.active.engine_id}:${m.active.model_file}`} p={m.active} moe={m.engine?.model.moe ?? false} onChanged={m.loadAll} />
            </PcOnly>
          </section>
        )}

        {running && (
          <section className="mx-panel" aria-labelledby={`${base}-dl-h`} data-spot="model-downloads">
            <div className="mx-panel-head">
              <h2 id={`${base}-dl-h`}>{t("models.dl.title")}</h2>
            </div>
            <DownloadsPanel empty={null} />
          </section>
        )}

        <nav className="mx-more" aria-label={t("models.tab.more")}>
          <a className="mx-link" href="#/settings/suggestions">
            <span>
              <b>{t("models.tab.find")}</b>
              <span>{t("models.tab.find.note")}</span>
            </span>
            <Icon name="next" />
          </a>
          <a className="mx-link" href="#/settings/expert">
            <span>
              <b>{t("models.tab.expert")}</b>
              <span>{t("models.tab.expert.note")}</span>
            </span>
            <Icon name="next" />
          </a>
        </nav>
      </div>
    </div>
  );
}

function Hero({ m, base, shown, onToggle }: { m: Models; base: string; shown: Shown; onToggle: (p: Panel) => void }) {
  const { engine, active, switching, progress } = m;
  if (!engine) {
    return (
      <section className="mx-hero" aria-busy="true" aria-label={t("models.hero.title")}>
        <span className="bar-ph" />
        <span className="bar-ph short" />
      </section>
    );
  }
  const info = engine.model;
  const name = modelName(info);
  const problem = switching ? null : problemOf(engine);
  const state = switching
    ? t("models.hero.switching")
    : engine.unloaded
      ? t("models.hero.unloaded")
      : engine.status === "ready"
        ? t("models.switch.runs")
        : problem
          ? problemLabel(problem)
          : engine.status === "error"
            ? t("models.hero.error")
            : engine.status === "loading"
              ? t("models.mem.loading")
              : t("models.hero.off");
  const dot = problem ? "offline" : switching || engine.status === "loading" ? "loading" : engine.unloaded ? "idle" : engine.status;
  const tokS = engine.last_timings ? Math.round(engine.last_timings.tok_s * 10) / 10 : null;
  const params = info.params_total === null ? null : paramsShort(info.params_total);
  const profileName = active?.name ?? engine.profile?.name ?? null;
  const measured = active?.measured ?? engine.profile?.measured ?? false;
  // A switch shows in the card, unless it runs from a row of "Meine Modelle", which shows it there.
  const here = m.usePath === null ? progress : null;

  return (
    <section className="mx-hero" data-spot="model" aria-labelledby={`${base}-hero`}>
      <div className="mx-hero-top">
        <h2 id={`${base}-hero`}>{t("models.hero.title")}</h2>
        <span className="mm-state">
          <i className={`dot ${dot}`} />
          {state}
        </span>
      </div>
      <div className="mx-hero-name">
        <b className="mx-name" title={info.file ?? undefined}>
          {name ?? t("models.hero.none")}
        </b>
        {info.file && <span className="mm-file">{info.file}</span>}
      </div>
      <dl className="mx-facts num">
        <div>
          <dt>{t("models.hero.speed")}</dt>
          <dd>
            {tokS === null ? (
              <small>{t("models.hero.noSpeed")}</small>
            ) : (
              <>
                {num(tokS)} <small>{t("models.unit.tokS")}</small>
              </>
            )}
          </dd>
        </div>
        <div>
          <dt>{t("models.sug.ctx")}</dt>
          <dd>
            {info.ctx ? ctxShort(info.ctx) : <span className="faint">–</span>} <small>{t("models.unit.tokens")}</small>
          </dd>
        </div>
        <div>
          <dt>{t("models.hero.size")}</dt>
          <dd>{[info.quant, params].filter(Boolean).join(" · ") || <span className="faint">–</span>}</dd>
        </div>
        <div>
          <dt>{t("models.hero.profile")}</dt>
          <dd className="mx-profile">
            <span>{profileName ?? <span className="faint">–</span>}</span>
            {profileName && (
              <span className={`tag${measured ? " tag-ok" : ""}`} title={measured ? t("models.measured.title") : t("models.guessed.title")}>
                {measured ? t("models.measured") : t("models.guessed")}
              </span>
            )}
          </dd>
        </div>
      </dl>
      {here && <SwitchProgress progress={here} engine={engine} />}
      <EngineNote engine={engine} switching={switching} onChanged={m.loadEngine} />
      <div className="mx-actions">
        <button
          type="button"
          className="btn primary"
          aria-expanded={shown.switch}
          aria-controls={shown.switch ? `${base}-switch` : undefined}
          onClick={() => onToggle("switch")}
        >
          <Icon name="model" small />
          {t("models.hero.switch")}
        </button>
        {active && (
          <button
            type="button"
            className="btn"
            aria-expanded={shown.bench}
            aria-controls={shown.bench ? `${base}-bench` : undefined}
            onClick={() => onToggle("bench")}
          >
            <Icon name="chart" small />
            {t("models.tab.bench")}
          </button>
        )}
      </div>
      <MemoryLine info={engine} onChanged={m.loadEngine} />
    </section>
  );
}
