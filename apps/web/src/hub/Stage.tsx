// Start layout C, "Bühne".

import { when } from "../api";
import { t } from "../i18n";
import { hueStyle, Portrait } from "../ui";
import { Figures, LeadActions, quote, Shelf, SOON, who, type ViewProps } from "./blocks";
import { FigureLines, FirstSteps, Fresh, LayoutSwitch, ModelStrip, SoonLine } from "./extras";

const SIDE_FIGS = 3; // "Schnell starten" beside the stage's panel on wide windows

export function Stage({ data, notes, engine, layout, onImport, onNewChat }: ViewProps) {
  const { characters, recent, lead, byUse, fresh, timings } = data;
  const tools = (
    <>
      <ModelStrip engine={engine} timings={timings} />
      <LayoutSwitch layout={layout} />
    </>
  );
  return (
    <div className="view">
      <div className="stage-page">
        {lead && (
          // A phone has no room for the tools under the stage: the page's title and the layout switch
          // stand above it, as in the other layouts (the stage's name stays the page's heading).
          <div className="page hub hx-stage-phone">
            <div className="title-row">
              <span className="hx-stage-title" aria-hidden="true">
                {t("shell.nav.start")}
              </span>
              <div className="actions hx-actions">
                <LayoutSwitch layout={layout} />
              </div>
            </div>
          </div>
        )}
        {lead ? (
          <section className="stage" style={hueStyle(lead.name)} aria-label={t("system.hub.recent")}>
            <span className="stage-bg" aria-hidden="true">
              <Portrait who={who(lead)} className="stage-bg-img" />
            </span>
            <div className="stage-inner">
              <a href={`#/chat/${lead.chat.id}`} className="stage-pic" data-log-label="Weiterschreiben" tabIndex={-1} aria-hidden="true">
                <Portrait who={who(lead)} className="pic" />
              </a>
              <div className="stage-panel">
                <span className="who">
                  {t("system.hub.lastPlayed")}
                  {lead.chat.title && <> · {lead.chat.title}</>}
                </span>
                <h1 className="stage-name">{lead.name}</h1>
                <blockquote>{quote(lead, t("system.hub.noMessageInChat"))}</blockquote>
                <span className="meta num">
                  {t("system.hub.messages", { n: lead.chat.messages })} · {when(lead.chat.updated_at)}
                </span>
                <LeadActions lead={lead} onNewChat={onNewChat} />
              </div>
              <div className="hx-stage-side">
                <h2 className="hx-head">{t("system.hub.quickStart")}</h2>
                <FigureLines list={byUse.slice(0, SIDE_FIGS)} label={t("system.hub.quickStart")} />
              </div>
            </div>
          </section>
        ) : (
          <div className="page hub hx-stage-top">
            <div className="title-row">
              <h1>{t("shell.nav.start")}</h1>
              <div className="actions hx-actions">{tools}</div>
            </div>
          </div>
        )}
        <div className="page stage-rows">
          {lead && <div className="hx-toolbar">{tools}</div>}
          {notes}
          {!lead && <FirstSteps engine={engine} characters={byUse} />}

          {recent.length > 1 && (
            <section aria-labelledby="row-read">
              <h2 className="sec" id="row-read">
                {t("system.hub.readOn")}
              </h2>
              <Shelf label={t("system.hub.readOn")}>
                {recent.slice(1, 9).map((r) => (
                  <a key={r.chat.id} className="read-card" href={`#/chat/${r.chat.id}`} style={hueStyle(r.name)} data-log-label="Chat öffnen">
                    <span className="rc-top">
                      <Portrait who={who(r)} className="av" />
                      <span className="rc-who">
                        <b>{r.name}</b>
                        <span>{r.chat.title || when(r.chat.updated_at)}</span>
                      </span>
                    </span>
                    <span className="rc-quote">{quote(r, t("system.hub.noMessage"))}</span>
                    <span className="meta num">{t("system.hub.messages", { n: r.chat.messages })}</span>
                  </a>
                ))}
              </Shelf>
            </section>
          )}

          {characters.length > 0 && (
            <section aria-labelledby="row-chars">
              <h2 className="sec" id="row-chars">
                {t("system.hub.yourFigures")}
                <a className="sec-link" href="#/chars">
                  {t("system.hub.showAllShort")}
                </a>
              </h2>
              <Figures list={byUse.slice(0, 12)} onImport={onImport} shelf />
            </section>
          )}

          <Fresh list={fresh} id="fresh-c" className="hx-fresh-row" />
          <SoonLine items={SOON} />
        </div>
      </div>
    </div>
  );
}
