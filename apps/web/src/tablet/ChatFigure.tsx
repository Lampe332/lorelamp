// The figure of a chat on a tablet. Held sideways it is a column of its own beside the story: the
// picture stays put while the story scrolls, under it the figure's chats to switch between. Held
// upright the same piece lies above the story as a head: picture, name and the chats as a row to
// flick through. Which of the two it is, is CSS alone, so turning the tablet keeps everything.

import { useEffect, useRef, useState } from "react";
import { chatName, when, type CharInfo, type ChatInfo } from "../api";
import { t } from "../i18n";
import { Icon, Portrait } from "../ui";
import { holdFigures, openNewChat, useCharChats } from "./data";
import "./tablet.css";

type Props = {
  character: CharInfo;
  /** The chat on show; its entry in the list is marked. */
  chat: ChatInfo;
  /** How many messages it has right now (the list's own count is from the last fetch). */
  messages: number;
  onClose: () => void;
};

export function ChatFigure({ character, chat, messages, onClose }: Props) {
  // A finished exchange changes counts and order; a token of a running answer does not.
  const { chats, error } = useCharChats(character.id, `${chat.id}:${messages}:${chat.title}`);
  const [starting, setStarting] = useState(false);
  const [problem, setProblem] = useState("");
  const list = useRef<HTMLElement>(null);
  // A tap on one of these chats finds the figure already in its place.
  useEffect(() => {
    if (chats) holdFigures(character, chats);
  }, [character, chats]);

  // The open chat stays in view in the row (upright) or the list (sideways).
  useEffect(() => {
    const row = list.current;
    const here = row?.querySelector<HTMLElement>('[aria-current="page"]');
    if (!row || !here) return;
    const [r, h] = [row.getBoundingClientRect(), here.getBoundingClientRect()];
    if (h.left < r.left || h.right > r.right) row.scrollLeft += h.left - r.left - 12;
    if (h.top < r.top || h.bottom > r.bottom) row.scrollTop += h.top - r.top - 12;
  }, [chat.id, chats]);

  const start = async () => {
    setStarting(true);
    setProblem(await openNewChat(character.id));
    setStarting(false);
  };
  const about = character.tagline || character.blurb;
  // Until the list is there, the open chat stands for it.
  const shown = chats ?? [chat];
  return (
    <aside className="tb-figure" aria-label={t("chat.figure.label", { name: character.name })}>
      <div className="tb-figure-pic">
        <Portrait who={character} className="tb-figure-img" full />
      </div>
      <button type="button" className="icon-btn tb-figure-close" onClick={onClose} aria-label={t("chat.figure.hide")} title={t("chat.figure.hide")}>
        <Icon name="close" small />
      </button>
      <div className="tb-figure-text">
        <p className="tb-figure-name">{character.name}</p>
        {about && <p className="tb-figure-about">{about}</p>}
      </div>
      <div className="tb-figure-chats">
        <div className="tb-figure-head">
          <h2>{chats ? t("chat.chats", { n: chats.length }) : t("chat.figure.chats")}</h2>
          <button type="button" className="btn sm solid tb-figure-new" onClick={() => void start()} disabled={starting} aria-label={t("chat.figure.newWith", { name: character.name })}>
            <Icon name="plus" small />
            <span>{t("chat.figure.new")}</span>
          </button>
        </div>
        <nav className="tb-chat-list" aria-label={t("chat.figure.list", { name: character.name })} ref={list}>
          {shown.map((c) => {
            const open = c.id === chat.id;
            const n = open ? messages : c.messages;
            return (
              <a key={c.id} className="tb-chat-link" href={`#/chat/${c.id}`} aria-current={open ? "page" : undefined} data-log-label="Chat wechseln">
                <b>{chatName(open ? chat : c)}</b>
                <span className="num">
                  {t("chat.messages", { n })} · {when(c.updated_at)}
                </span>
              </a>
            );
          })}
        </nav>
        {(problem || error) && (
          <p className="tb-figure-note" role="alert">
            {problem || t("chat.figure.failed", { error })}
          </p>
        )}
      </div>
    </aside>
  );
}
