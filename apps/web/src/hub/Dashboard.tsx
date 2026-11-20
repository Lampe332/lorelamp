// Start layout A, "Dashboard".

import { num, when, type Recent } from "../api";
import { numberFormat, t } from "../i18n";
import { Portrait } from "../ui";
import { Aura, cachePct, CountFacts, EngineFigure, Figures, LeadCard, QuickBar, quote, SearchWidget, SOON, speed, who, type ViewProps } from "./blocks";
import { FirstSteps, Fresh, LayoutSwitch, ModelAct, modelState, SoonLine } from "./extras";

/** The last chat as a quiet compact row: picture, name, where it stands. */
function MiniChat({ r }: { r: Recent }) {
  return (
    <a className="mini-chat" href={`#/chat/${r.chat.id}`} data-log-label="Chat öffnen">
      <Portrait who={who(r)} className="av" />
      <span className="mc-text">
        <span className="mc-name">
          <b>{r.name}</b>
          {r.chat.title && <span>{r.chat.title}</span>}
        </span>
        <span className="mc-sub">{quote(r, t("system.hub.noMessage"))}</span>
      </span>
      <span className="mc-when num">{when(r.chat.updated_at)}</span>
    </a>
  );
}

export function Dashboard({ data, notes, engine, layout, onPalette, onImport }: ViewProps) {
  const { characters, recent, lead, byUse, fresh, timings } = data;
  const pct = timings && cachePct(timings);
  const model = modelState(engine);
  return (
    <div className="view">
      <div className="home">
        <Aura lead={lead} fallback={byUse[0]} />
        <div className="page hub dash">
          <div className="title-row">
            <h1>{t("shell.nav.start")}</h1>
            <div className="actions hx-actions">
              <SearchWidget onPalette={onPalette} />
              <LayoutSwitch layout={layout} />
            </div>
          </div>
          {notes}

          <dl className="kpis" aria-label={t("system.hub.kpis")}>
            <CountFacts data={data} />
            <div>
              <dt>{t("system.hub.speedLast")}</dt>
              <dd className="num">
                {speed(timings)}
                {timings && <small>{t("system.hub.tokS")}</small>}
              </dd>
            </div>
            <div>
              <dt>{t("system.hub.model")}</dt>
              <dd className="kpi-engine">
                <EngineFigure engine={engine} />
                {model.kind === "ready" && pct !== null && pct !== undefined && <small className="num">{t("system.hub.cacheLast", { pct: t("system.pct", { n: pct }) })}</small>}
                {model.kind === "idle" && <ModelAct state={model} />}
              </dd>
            </div>
          </dl>

          <QuickBar data={data} onPalette={onPalette} onImport={onImport} />

          {!lead && <FirstSteps engine={engine} characters={byUse} />}
          {characters.length > 0 && (
            <div className="dash-grid">
              <div className="dash-main">
                {lead && <LeadCard lead={lead} className="lead-card" />}

                <section className="dash-figs" aria-labelledby="figs-a">
                  <h2 className="side-title" id="figs-a">
                    {t("system.hub.yourFigures")}
                    <a className="sec-link" href="#/chars">
                      {characters.length > 8 ? t("system.hub.showAll", { n: characters.length }) : t("system.hub.manageShort")}
                    </a>
                  </h2>
                  <Figures list={byUse.slice(0, 8)} onImport={onImport} few={5} />
                </section>
              </div>

              <div className="dash-col">
                {recent.length > 0 && (
                  <aside className="dash-side" aria-label={t("system.hub.recent")}>
                    <h2 className="side-title">
                      {t("system.hub.recent")}
                      <span className="num">{num(Math.min(recent.length, 6))}</span>
                    </h2>
                    <div className="mini-list">
                      {recent.slice(0, 6).map((r) => (
                        <MiniChat key={r.chat.id} r={r} />
                      ))}
                    </div>
                  </aside>
                )}
                <Fresh list={fresh} id="fresh-a" className="dash-fresh" />
                <section className="dash-measure" aria-labelledby="measure-a">
                  <h2 className="side-title" id="measure-a">
                    {t("system.hub.lastMeasure")}
                  </h2>
                  {timings ? (
                    <dl className="facts num">
                      <div>
                        <dt>{t("system.hub.speed")}</dt>
                        <dd>{t("system.hub.tokSLastValue", { speed: numberFormat({ minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(timings.tok_s) })}</dd>
                      </div>
                      <div>
                        <dt>{t("system.hub.written")}</dt>
                        <dd>{t("system.hub.tokensN", { n: timings.predicted })}</dd>
                      </div>
                      <div>
                        <dt>{t("system.hub.read")}</dt>
                        <dd>{t("system.hub.tokensN", { n: timings.processed })}</dd>
                      </div>
                      <div>
                        <dt>{t("system.hub.fromCacheLabel")}</dt>
                        <dd>
                          {t("system.hub.tokensN", { n: timings.cached })}
                          {pct !== null && ` (${t("system.pct", { n: pct })})`}
                        </dd>
                      </div>
                    </dl>
                  ) : (
                    <p className="help">{t("system.hub.measureLater")}</p>
                  )}
                </section>
              </div>
            </div>
          )}

          <SoonLine items={SOON} />
        </div>
      </div>
    </div>
  );
}
