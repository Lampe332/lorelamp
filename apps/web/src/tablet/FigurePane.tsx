// The characters screen on a tablet: the cards on the left, the chosen figure on the right — its
// picture large, what the card says about it, the way back into its last chat and its other chats.
// On a wide tablet the pane is always there (a tap on a card only changes whom it shows); on a
// narrow one it comes in over the cards when a card is tapped. Which of the two is CSS alone.

import { useEffect, useRef } from "react";
import { chatName, when, type CharInfo } from "../api";
import { t } from "../i18n";
import { Icon, Portrait, hueStyle } from "../ui";
import { holdFigures, useCharChats } from "./data";
import { useSwipeAway } from "./swipe";
import "./tablet.css";

type Props = {
  character: CharInfo;
  /** A card was tapped for this figure (a narrow tablet shows the pane only then). */
  open: boolean;
  /** Changes whenever chats may have changed. */
  rev: unknown;
  /** Continue the latest chat (or start the first one); `fresh` always starts a new one. */
  onChat: (character: CharInfo, fresh: boolean) => void;
  onClose: () => void;
};

/** How many of the figure's chats the pane lists before "Alle Chats". */
const SHOWN = 4;

export function FigurePane({ character: c, open, rev, onChat, onClose }: Props) {
  const { chats, error } = useCharChats(c.id, rev);
  const pane = useRef<HTMLElement>(null);
  useSwipeAway(pane, onClose);
  // A chat opened from here shows its figure at once.
  useEffect(() => {
    if (chats) holdFigures(c, chats);
  }, [c, chats]);
  const about = c.tagline || c.blurb;
  return (
    <aside ref={pane} className="tb-pane" data-open={open} style={hueStyle(c.name)} aria-label={t("cards.pane.label", { name: c.name })}>
      <div className="tb-pane-pic">
        <Portrait who={c} className="tb-pane-img" full />
        <button type="button" className="icon-btn tb-pane-close" onClick={onClose} aria-label={t("cards.pane.close")} title={t("cards.close")}>
          <Icon name="close" small />
        </button>
      </div>
      <div className="tb-pane-body">
        <div className="tb-pane-title">
          <h2>{c.name}</h2>
        </div>
        {about && <p className="tb-pane-about">{about}</p>}
        {c.tags.length > 0 && (
          <ul className="tb-pane-tags" aria-label={t("cards.cardTags")}>
            {c.tags.slice(0, 6).map((tag) => (
              <li key={tag} className="tag">
                {tag}
              </li>
            ))}
          </ul>
        )}
        <div className="tb-pane-acts">
          <button type="button" className="btn primary" onClick={() => onChat(c, false)}>
            {c.chats > 0 ? t("cards.pane.continue") : t("cards.startChat")}
            <Icon name="next" small />
          </button>
          {c.chats > 0 && (
            <button type="button" className="btn solid" onClick={() => onChat(c, true)}>
              <Icon name="plus" small />
              {t("cards.newChat")}
            </button>
          )}
        </div>
        {c.chats > 0 && (
          <section className="tb-pane-chats" aria-label={t("cards.pane.chatsWith", { name: c.name })}>
            <div className="tb-pane-head">
              <h3>{t("cards.chats", { n: c.chats })}</h3>
              {c.chats > SHOWN && (
                <a className="tb-pane-all" href={`#/char/${c.id}/chats`}>
                  {t("cards.pane.showAll")}
                </a>
              )}
            </div>
            {chats === null && !error ? (
              <div aria-busy="true">
                <span className="bar-ph" />
              </div>
            ) : error ? (
              <p className="tb-figure-note" role="alert">
                {t("cards.pane.loadFailed", { error })}
              </p>
            ) : (
              <nav className="tb-pane-list" aria-label={t("cards.pane.recent")}>
                {(chats ?? []).slice(0, SHOWN).map((chat) => (
                  <a key={chat.id} className="tb-chat-link" href={`#/chat/${chat.id}`} data-log-label="Chat öffnen">
                    <b>{chatName(chat)}</b>
                    <span className="num">
                      {t("cards.messages", { n: chat.messages })} · {when(chat.updated_at)}
                    </span>
                  </a>
                ))}
              </nav>
            )}
          </section>
        )}
        <nav className="tb-pane-more" aria-label={t("cards.pane.more")}>
          <a className="btn ghost" href={`#/char/${c.id}/chats`}>
            <Icon name="chat" small />
            {t("cards.pane.allChats")}
          </a>
          <a className="btn ghost" href={`#/char/${c.id}/settings`}>
            <Icon name="card" small />
            {t("cards.editCard")}
          </a>
        </nav>
      </div>
    </aside>
  );
}
