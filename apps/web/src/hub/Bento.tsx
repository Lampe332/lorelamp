// Start layout B, "Bento".

import { num } from "../api";
import { t } from "../i18n";
import { Icon, Portrait } from "../ui";
import { Aura, ChatLine, CountFacts, EngineStats, LeadCard, SearchWidget, SOON, type ViewProps } from "./blocks";
import { FigureLines, FirstSteps, Fresh, LayoutSwitch, SoonLine } from "./extras";

const LIB_FACES = 5; // the library tile shows this many faces, then "+n"
const BENTO_MORE = 4; // rows in "Weitere Chats"
const BENTO_FIGS = 4; // rows in "Figuren"

export function Bento({ data, notes, engine, layout, onPalette, onImport }: ViewProps) {
  const { characters, recent, lead, byUse, fresh, timings } = data;
  const more = recent.slice(1, 1 + BENTO_MORE);
  return (
    <div className="view">
      <div className="home">
        <Aura lead={lead} fallback={byUse[0]} />
        <div className="page hub bento-page">
          <div className="title-row">
            <h1>{t("shell.nav.start")}</h1>
            <div className="actions hx-actions">
              <SearchWidget onPalette={onPalette} />
              <LayoutSwitch layout={layout} />
            </div>
          </div>
          {notes}
          <div className="bento">
            {lead ? <LeadCard lead={lead} className="tile t-lead" /> : <FirstSteps engine={engine} characters={byUse} className="tile t-lead hx-first-tile" />}

            <div className="tile t-engine">
              <EngineStats engine={engine} timings={timings} />
            </div>

            <div className="tile t-counts">
              <span className="tile-label">{t("system.hub.library")}</span>
              {byUse.length > 0 && (
                <a className="lib-faces" href="#/chars" data-log-label="Figuren öffnen">
                  <span className="lib-stack">
                    {byUse.slice(0, LIB_FACES).map((c) => (
                      <Portrait key={c.id} who={c} className="av" />
                    ))}
                    {byUse.length > LIB_FACES && <span className="av lib-more num">+{num(byUse.length - LIB_FACES)}</span>}
                  </span>
                  <span className="lib-names">{byUse.map((c) => c.name).join(", ")}</span>
                </a>
              )}
              <dl className="lib-counts" aria-label={t("system.hub.library")}>
                <CountFacts data={data} />
              </dl>
            </div>

            {more.length > 0 && (
              <div className="tile t-recent">
                <span className="tile-label">
                  {t("system.hub.moreChats")}
                  {recent.length > 1 + BENTO_MORE && <span className="tile-note num">{t("system.hub.recentN", { n: recent.length - 1 })}</span>}
                </span>
                <div className="chat-lines">
                  {more.map((r) => (
                    <ChatLine key={r.chat.id} r={r} />
                  ))}
                </div>
              </div>
            )}

            {/* Without a further chat the figures take that tile's room too. */}
            {byUse.length > 0 && (
              <div className={`tile t-chars hx-t-chars${more.length === 0 ? " t-wide" : ""}`}>
                <span className="tile-label">
                  {t("system.hub.figures")}
                  <a className="sec-link" href="#/chars">
                    {characters.length > BENTO_FIGS ? t("system.hub.allN", { n: characters.length }) : t("system.hub.all")}
                  </a>
                </span>
                <FigureLines list={byUse.slice(0, BENTO_FIGS)} label={t("system.hub.figures")} />
                <Fresh list={fresh} id="fresh-b" />
              </div>
            )}

            <div className={`tile t-actions${byUse.length === 0 ? " hx-t-rest" : ""}`}>
              <span className="tile-label">{t("system.hub.quickShort")}</span>
              <button type="button" className="tile-action" onClick={() => onPalette(t("cmd.newWith", { name: "" }))} disabled={characters.length === 0}>
                <Icon name="plus" small />
                {t("system.hub.newChat")}
              </button>
              <button type="button" className="tile-action" onClick={onImport}>
                <Icon name="upload" small />
                {t("system.hub.import")}
              </button>
              <a className="tile-action" href="#/personas">
                <Icon name="persona" small />
                {t("system.hub.personas")}
              </a>
              <button type="button" className="tile-action" onClick={() => onPalette(t("shell.general.look.themeNamed", { name: "" }))}>
                <Icon name="palette" small />
                {t("system.hub.theme")}
              </button>
              <button type="button" className="tile-action" onClick={() => onPalette()}>
                <Icon name="bolt" small />
                {t("system.hub.allActions")}
                <kbd>{t("shell.key.ctrlK")}</kbd>
              </button>
            </div>
          </div>
          <SoonLine items={SOON} />
        </div>
      </div>
    </div>
  );
}
