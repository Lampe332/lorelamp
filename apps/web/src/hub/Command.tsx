// Start layout D, "Kommandozentrale".

import { chatName, when, type CharInfo } from "../api";
import { t } from "../i18n";
import { CommandField } from "../Palette";
import { hueStyle, Icon, Portrait } from "../ui";
import {
  AddCard,
  Aura,
  ChatLine,
  chatsOf,
  CountFacts,
  EngineStats,
  FEW,
  FigTags,
  LeadActions,
  quote,
  SOON,
  who,
  type ViewProps,
} from "./blocks";
import { FirstSteps, Fresh, LayoutSwitch, SoonLine, Star } from "./extras";

const CMD_ROWS = 6; // chats under the lead
const CMD_FIGS = 6; // figure rows

/** A figure as a row with room for its description, and a new chat one click away. */
function FigureRow({ c, onNewChat, lines }: { c: CharInfo; onNewChat: (id: number) => void; lines: number }) {
  return (
    <li className="cmd-fig" style={hueStyle(c.name)}>
      <a className="cmd-fig-open" href={`#/char/${c.id}/chats`} data-log-label="Figur öffnen">
        <Portrait who={c} className="art" />
        <span className="cmd-fig-text">
          <b>{c.name}</b>
          <span className="meta num">{chatsOf(c)}</span>
          {c.blurb && (
            <span className="cmd-fig-blurb" style={{ WebkitLineClamp: lines }}>
              {c.blurb}
            </span>
          )}
          <FigTags c={c} />
        </span>
      </a>
      <div className="hx-fig-acts">
        <Star c={c} />
        <button type="button" className="btn sm cmd-fig-new" onClick={() => onNewChat(c.id)} aria-label={t("system.hub.newChatWith", { name: c.name })} data-log-label="Neuer Chat mit der Figur">
          <Icon name="plus" small />
          <span className="hide-sm">{t("system.hub.newChat")}</span>
        </button>
      </div>
    </li>
  );
}

export function CommandCenter({ data, notes, engine, layout, commands, onPalette, onImport, onNewChat }: ViewProps) {
  const { characters, recent, lead, byUse, fresh, timings } = data;
  const rows = recent.slice(1, 1 + CMD_ROWS);
  const figs = byUse.slice(0, CMD_FIGS);
  const leadChats = lead ? (characters.find((c) => c.id === lead.chat.character_id)?.chats ?? 0) : 0;
  return (
    <div className="view">
      <div className="home">
      <Aura lead={lead} fallback={byUse[0]} />
      <div className="page hub cmd-page">
        <div className="cmd-head">
          <div className="cmd-ask">
            <h1 className="cmd-title">{t("system.hub.ask")}</h1>
            <CommandField commands={commands} />
            <nav className="chips" aria-label={t("system.hub.quick")}>
              {lead && (
                <a className="chip chip-main joy" href={`#/chat/${lead.chat.id}`} data-log-label="Weiterschreiben">
                  <Icon name="next" small />
                  {t("system.hub.goOn")}
                </a>
              )}
              <button type="button" className="chip" onClick={() => onPalette(t("cmd.newWith", { name: "" }))} disabled={characters.length === 0}>
                <Icon name="plus" small />
                {t("system.hub.newChat")}
              </button>
              <button type="button" className="chip" onClick={onImport}>
                <Icon name="upload" small />
                {t("system.hub.import")}
              </button>
              <button type="button" className="chip" onClick={() => onPalette(t("shell.general.look.themeNamed", { name: "" }))}>
                <Icon name="palette" small />
                {t("system.hub.themeShort")}
              </button>
              <a className="chip" href="#/personas">
                <Icon name="persona" small />
                {t("system.hub.personas")}
              </a>
              <LayoutSwitch layout={layout} className="hx-chip" />
            </nav>
          </div>
          <section className="cmd-engine" aria-label={t("system.hub.model")}>
            <EngineStats engine={engine} timings={timings} heading />
          </section>
        </div>
        {notes}
        {!lead && <FirstSteps engine={engine} characters={byUse} />}

        {characters.length > 0 && (
          <dl className="cmd-kpis" aria-label={t("system.hub.kpis")}>
            <CountFacts data={data} />
            <div>
              <dt>{t("system.hub.lastPlayed")}</dt>
              <dd className="num cmd-kpi-when">{lead ? when(lead.chat.updated_at) : "–"}</dd>
            </div>
          </dl>
        )}

        {characters.length > 0 && (
          <div className="cmd-grid">
            {lead && (
              <section className="cmd-recent" aria-labelledby="cmd-recent">
                <h2 className="cmd-sec" id="cmd-recent">
                  {t("system.hub.recent")}
                  <span className="cmd-sec-note num">{t("system.hub.chats", { n: recent.length })}</span>
                </h2>
                <div className="cmd-panel">
                  <div className="cmd-lead" style={hueStyle(lead.name)}>
                    <a className="cmd-lead-pic" href={`#/chat/${lead.chat.id}`} tabIndex={-1} aria-hidden="true" data-log-label="Weiterschreiben">
                      <Portrait who={who(lead)} className="pic" />
                    </a>
                    <div className="cmd-lead-text">
                      <span className="cl-head">
                        <b>{lead.name}</b>
                        <span>{chatName(lead.chat)}</span>
                      </span>
                      <blockquote className="cmd-lead-quote">{quote(lead, t("system.hub.noMessageInChat"))}</blockquote>
                      <span className="meta num">
                        {t("system.hub.messages", { n: lead.chat.messages })} · {when(lead.chat.updated_at)}
                        {leadChats > 1 && <> · {t("system.hub.oneOf", { n: leadChats })}</>}
                      </span>
                      <LeadActions lead={lead} onNewChat={onNewChat} />
                    </div>
                  </div>
                  {rows.length > 0 && (
                    <div className="chat-lines">
                      {rows.map((r) => (
                        <ChatLine key={r.chat.id} r={r} />
                      ))}
                    </div>
                  )}
                </div>
                <Fresh list={fresh} id="fresh-d" className="cmd-fresh" />
              </section>
            )}

            <section className="cmd-chars" aria-labelledby="cmd-chars">
              <h2 className="cmd-sec" id="cmd-chars">
                {t("system.hub.figures")}
                <a className="sec-link" href="#/chars">
                  {characters.length > CMD_FIGS ? t("system.hub.showAll", { n: characters.length }) : t("system.hub.manageShort")}
                </a>
              </h2>
              <ul className="cmd-figs">
                {figs.map((c) => (
                  <FigureRow key={c.id} c={c} onNewChat={onNewChat} lines={figs.length <= FEW ? 4 : 2} />
                ))}
              </ul>
              {byUse.length <= FEW + 1 && <AddCard onImport={onImport} />}
              {!lead && <Fresh list={fresh} id="fresh-d" className="cmd-fresh" />}
            </section>
          </div>
        )}
        <SoonLine items={SOON} />
      </div>
      </div>
    </div>
  );
}
