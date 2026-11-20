// Start layout "Bisher": the one from before the four new ones.

import { when, type Recent } from "../api";
import { t } from "../i18n";
import { rich } from "../i18n/rich";
import { hueStyle, Portrait } from "../ui";
import { AddCard, Aura, FEW, Figures, GoOn, Poster, quote, SOON, who, type ViewProps } from "./blocks";
import { FirstSteps, Fresh, LayoutSwitch, ModelStrip, SoonLine } from "./extras";

const MORE_CHATS = 6; // rows under the lead
const CHARACTERS = 6; // posters in the characters column (two rows of three, three of two)

/** The layout from before the four new ones, in the same order: the last chat large, the further
 *  chats, the characters as posters. It shares the backdrop of A–D. On wide windows the characters
 *  stand beside the chats; with few of them the last chat spans the width above both columns. */
export function Classic({ data, notes, engine, layout, onImport }: ViewProps) {
  const { characters, recent, lead, byUse, fresh, timings } = data;
  const more = recent.slice(1, 1 + MORE_CHATS);
  const few = characters.length <= FEW;
  return (
    <div className="view">
      <div className="home">
        <Aura lead={lead} fallback={byUse[0]} />
        <div className={`page hub classic${few ? " few" : ""}`}>
          <div className="title-row">
            <h1>{t("shell.nav.start")}</h1>
            <div className="actions hx-actions">
              <ModelStrip engine={engine} timings={timings} />
              <LayoutSwitch layout={layout} />
            </div>
          </div>
          {notes}

          {!lead && <FirstSteps engine={engine} characters={byUse} />}

          <div className="classic-grid">
            {lead && (
              <a className="resume cl-lead" href={`#/chat/${lead.chat.id}`} style={hueStyle(lead.name)} data-log-label="Weiterschreiben">
                <span className="cl-pic">
                  <Portrait who={who(lead)} className="pic" />
                </span>
                <span className="resume-text">
                  <span className="who">
                    {rich("system.hub.lastWith", { name: <b>{lead.name}</b> })}
                    {lead.chat.title && <> · {lead.chat.title}</>}
                  </span>
                  <blockquote>{quote(lead, t("system.hub.noMessageInChat"))}</blockquote>
                  <span className="meta num">
                    {t("system.hub.messages", { n: lead.chat.messages })} · {when(lead.chat.updated_at)}
                  </span>
                </span>
                <GoOn />
              </a>
            )}

            {(more.length > 0 || fresh.length > 0) && (
              <div className="classic-main">
                {more.length > 0 && (
                  <section aria-labelledby="more-0">
                    <h2 className="sec" id="more-0">
                      {t("system.hub.moreChats")}
                      {recent.length > 1 + MORE_CHATS && <span className="sec-note num">{t("system.hub.recentN", { n: recent.length - 1 })}</span>}
                    </h2>
                    <ChatRows rows={more} />
                  </section>
                )}
                <Fresh list={fresh} id="fresh-0" />
              </div>
            )}

            {characters.length > 0 && (
              <section className="classic-chars" aria-labelledby="chars-0">
                <h2 className="sec" id="chars-0">
                  {t("system.hub.characters")}
                  <a className="sec-link" href="#/chars">
                    {characters.length > CHARACTERS ? t("system.hub.showAll", { n: characters.length }) : t("system.hub.manage")}
                  </a>
                </h2>
                {few ? (
                  <Figures list={byUse} onImport={onImport} />
                ) : (
                  <div className="grid-chars">
                    {byUse.slice(0, CHARACTERS).map((c) => (
                      <Poster key={c.id} c={c} />
                    ))}
                    {characters.length < CHARACTERS && <AddCard onImport={onImport} />}
                  </div>
                )}
              </section>
            )}
          </div>
          <SoonLine items={SOON} />
        </div>
      </div>
    </div>
  );
}

function ChatRows({ rows }: { rows: Recent[] }) {
  return (
    <div className="chat-list">
      {rows.map((r) => (
        <a className="row-card chat-row hub-row" key={r.chat.id} href={`#/chat/${r.chat.id}`} style={hueStyle(r.name)} data-log-label="Chat öffnen">
          <Portrait who={who(r)} className="av" />
          <span className="chat-open">
            <span className="l1">
              <b className="named">{r.name}</b>
              {r.chat.title && <span className="chat-title">{r.chat.title}</span>}
            </span>
            <span className="l2">{quote(r, t("system.hub.noMessage"))}</span>
          </span>
          <span className="r num">
            {t("system.hub.messages", { n: r.chat.messages })}
            <small>{when(r.chat.updated_at)}</small>
          </span>
        </a>
      ))}
    </div>
  );
}
