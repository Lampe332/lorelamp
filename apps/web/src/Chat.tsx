import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { chatName, chatPersonaId, copyText, errorText, num, setSwipe, type CharInfo, type ChatInfo, type EngineStatus, type Msg, type Persona } from "./api";
import { flushSync } from "react-dom";
import { numberFormat, t } from "./i18n";
import { rich } from "./i18n/rich";
import { useFormFactor } from "./app/formfactor";
import { Backdrop, Stage } from "./chat/Stage";
import { LookPanel, LookPanelLoading } from "./chat/LookPanel";
import { lookProps, messageTime, resolve, shownBackdrop, useChatLook, useReducedMotion, useStage, useStageSide } from "./chat/look";
import { MessageActions, type Actions, type Write } from "./chat/MessageActions";
import { PRESETS, presetThumbUrl } from "./chat/presets";
import { codeText } from "./app/useChatSocket";
import { Prose } from "./chat/Prose";
import { continueReply, regenerate, writeError } from "./chat/write";
import "./chat/chat.css";
import { sendsMessage, useGeneral } from "./general/store";
import { CHAT_FIND_EVENT, onJump, takeJump } from "./jump";
import { ChatPersona } from "./persona/ChatPersona";
import { fold } from "./search";
import { ChatFigure } from "./tablet/ChatFigure";
import { heldFigure, useFigureShown } from "./tablet/data";
import { useVariantSwipe } from "./tablet/swipe";
import { hueStyle, Icon, Portrait } from "./ui";
import { useWindow } from "./virtual";

export type ChatState = {
  chatId: number;
  chat: ChatInfo;
  character: CharInfo;
  messages: Msg[];
  generating: number | null;
  /** A reply being written again: the text it had, shown faded until the first new word arrives. */
  prior?: { id: number; text: string };
};

type Props = {
  chat: ChatState | null;
  /** All personas and the active one: who wrote your messages, and who you play as here. */
  personas: Persona[];
  activePersonaId: number | null;
  /** Binds this chat to a persona; null = it follows the active one again. */
  onPersona: (id: number | null) => void;
  engine: EngineStatus;
  /** Why the model is not running, if the app knows. */
  engineNote: string;
  /** The model is unloaded: sending loads it again, so sending is not blocked. */
  unloaded?: boolean;
  online: boolean;
  /** The model is writing (in this chat or another one). */
  busy: boolean;
  notice: string;
  onSend: (text: string) => void;
  onStop: () => void;
  /** Replaces the text of a message; resolves to whether it was saved. */
  onEdit: (id: number, text: string) => Promise<boolean>;
  /** Removes a message and everything after it; resolves to whether that happened. */
  onRewind: (id: number) => Promise<boolean>;
  /** Opens a new chat that holds this one up to and including the message. */
  onFork: (id: number) => void;
  /** One level up: the chat overview of this character (the library while that is unknown). */
  onBack: () => void;
  /** The card editor of this character holds unsaved changes. */
  unsaved: boolean;
  /** Context of the running model in tokens (`/engine` `model.ctx`); null = unknown. */
  ctx: number | null;
};

/** The folded words of a search in the chat (see search.ts `fold`); empty = none. */
function findWords(query: string): string[] {
  return fold(query.trim()).text.split(/\s+/).filter(Boolean);
}

/** A message as an editable text, in the face it is read in. Ctrl + Enter saves, Escape leaves.
 *  `again`: your message is followed by the chat's last reply only, which can be written anew
 *  right after saving ("wait": not now, the model is busy or sending is blocked). */
function EditBox({ text, onChange, onSave, onSaveAgain, again, onCancel }: { text: string; onChange: (t: string) => void; onSave: () => void; onSaveAgain: () => void; again: Write; onCancel: () => void }) {
  const box = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight + 2}px`;
  }, [text]);
  useEffect(() => {
    const el = box.current;
    el?.focus({ preventScroll: true });
    el?.setSelectionRange(el.value.length, el.value.length);
  }, []);
  return (
    <form
      className="msg-edit"
      onSubmit={(e) => {
        e.preventDefault();
        onSave();
      }}
    >
      <textarea
        ref={box}
        className="textarea"
        value={text}
        aria-label={t("chat.edit.label")}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            onSave();
          }
        }}
      />
      <div className="msg-edit-bar">
        <span className="hint-inline">{t("chat.edit.keys")}</span>
        <button type="button" className="btn sm ghost" onClick={onCancel}>
          {t("chat.edit.cancel")}
        </button>
        {again !== "no" && (
          <button type="button" className="btn sm cv-save-again" disabled={!text.trim() || again === "wait"} title={t("chat.edit.saveAgain.title")} onClick={onSaveAgain}>
            {t("chat.edit.saveAgain")}
          </button>
        )}
        <button type="submit" className="btn sm primary" disabled={!text.trim()}>
          {t("chat.edit.save")}
        </button>
      </div>
    </form>
  );
}

/** A reply with variants (e.g. from SillyTavern): which one is shown, and steps to its neighbours by
 *  click, tap or the arrow keys. The shown one always comes from the chat's history on the socket.
 *  On the chat's last reply a step past its last variant writes a new one (`onNew`, as in other chat
 *  apps); `newWait`: that is possible here, just not now (the model is busy or sending is blocked). */
function Variants({ id, at, total, locked, onNew, newWait = false }: { id: number; at: number; total: number; locked: boolean; onNew?: () => void; newWait?: boolean }) {
  const [note, setNote] = useState("");
  const pending = useRef(false);
  useEffect(() => {
    if (!note) return;
    const timer = window.setTimeout(() => setNote(""), 5000);
    return () => window.clearTimeout(timer);
  }, [note]);
  const atLast = at >= total - 1;
  const writesNew = atLast && (onNew !== undefined || newWait);
  const go = (d: 1 | -1) => {
    const to = at + d;
    if (d === 1 && atLast && onNew && !locked && !pending.current) {
      setNote("");
      onNew();
      return;
    }
    if (locked || pending.current || to < 0 || to >= total) return;
    pending.current = true;
    setNote("");
    setSwipe(id, to)
      .catch((e) => setNote(t("chat.variants.failed", { error: errorText(e) })))
      .finally(() => (pending.current = false));
  };
  // aria-disabled instead of disabled: the button keeps the focus when it reaches an end.
  const prevOff = locked || at <= 0;
  const nextOff = locked || (atLast && !onNew);
  return (
    <>
      <div
        className="x7-swipe"
        role="group"
        aria-label={t("chat.variants.label")}
        onKeyDown={(e) => {
          if ((e.key !== "ArrowLeft" && e.key !== "ArrowRight") || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
          e.preventDefault();
          // A held key steps through the variants, but never writes one after another.
          if (e.repeat && e.key === "ArrowRight" && atLast) return;
          go(e.key === "ArrowLeft" ? -1 : 1);
        }}
      >
        <button type="button" className="icon-btn x7-prev" aria-label={t("chat.variants.prev")} title={t("chat.variants.prev.title")} aria-disabled={prevOff} onClick={() => go(-1)}>
          <Icon name="chevron" small />
        </button>
        <span className="x7-count num" aria-live="polite">
          <span className="sr">{t("chat.variants.at", { at: at + 1, total })}</span>
          <span aria-hidden="true">
            {at + 1} / {total}
          </span>
        </span>
        <button
          type="button"
          className={`icon-btn x7-next${writesNew ? " cv-new-variant" : ""}`}
          aria-label={writesNew ? t("chat.variants.new") : t("chat.variants.next")}
          title={writesNew ? t("chat.variants.new.title") : t("chat.variants.next.title")}
          aria-disabled={nextOff}
          onClick={() => go(1)}
        >
          <Icon name="chevron" small />
        </button>
      </div>
      {note && (
        <p className="x7-note" role="status">
          {note}
        </p>
      )}
    </>
  );
}

type MessageProps = {
  m: Msg;
  who: CharInfo;
  /** Your message: the persona it was written as (old ones: who you play as here now). */
  me: Persona | undefined;
  streaming: boolean;
  /** The text being edited, when this message is open for editing. */
  draft: string | null;
  /** The model writes in this chat: the story must not change under it. */
  locked: boolean;
  /** The greeting stays: a chat without it would not be this card's chat. */
  canRewind: boolean;
  last: boolean;
  copied: boolean;
  act: Actions;
  /** Words of the search in this chat when this message matches it (set apart in the text). */
  words: string[] | null;
  /** The search hit shown now. */
  hit: boolean;
  /** Just jumped to: highlighted for a moment. */
  flash: boolean;
  /** New text of an answer fades in while it is written. */
  animate: boolean;
  /** The chat's last reply can be written again or on ("no" for every other message). */
  write: Write;
  /** Why writing it again or on did not start, if it did not. */
  writeNote: string;
  /** The time it was written stands beside the name. */
  times: boolean;
  /** Arrived while the chat was open (comes on screen the way the look says). */
  fresh: boolean;
  /** Being written again: the text it had, until the first new word arrives. */
  prior: string;
  /** Your message right before the chat's last reply: "Speichern und neu antworten" (see EditBox). */
  again: Write;
  onSaveAgain: () => void;
};

const Message = memo(function Message({ m, who, me, streaming, draft, locked, canRewind, last, copied, act, words, hit, flash, animate, write, writeNote, times, fresh, prior, again, onSaveAgain }: MessageProps) {
  const mine = m.role === "user";
  const tm = m.timings;
  const time = times && m.at ? <time dateTime={new Date(m.at * 1000).toISOString()} className="msg-time num">{messageTime(m.at)}</time> : null;
  // Until its first new word a reply written again keeps its old text, faded, the caret at its start.
  const rewriting = streaming && prior !== "" && m.content.trim() === "";
  const timings = tm && { speed: tm.tok_s.toFixed(0), predicted: tm.predicted, processed: tm.processed, cached: tm.cached };
  return (
    <article
      className={`msg ${mine ? "user" : "ai"}${last ? " last" : ""}${hit ? " hit" : ""}${flash ? " flash" : ""}${fresh ? " cv-new" : ""}${rewriting ? " cv-rewrite" : ""}`}
      data-msg={m.id}
      aria-label={mine ? t("chat.msg.mine") : t("chat.msg.theirs")}
    >
      {mine ? (
        <header className="msg-head">
          <b>
            <span className="msg-name">{me?.name ?? t("chat.msg.you")}</span>
            {time}
          </b>
          {me && <Portrait who={me} kind="persona" className="av" />}
        </header>
      ) : (
        <header className="msg-head">
          <Portrait who={who} className="av" />
          <b>
            <span className="msg-name">{who.name}</span>
            {time}
          </b>
        </header>
      )}
      {draft !== null ? (
        <EditBox text={draft} onChange={(text) => act.editText(m.id, text)} onSave={act.saveEdit} onSaveAgain={onSaveAgain} again={again} onCancel={act.cancelEdit} />
      ) : rewriting ? (
        <>
          <Prose text={prior} streaming={false} words={null} animate={false} />
          <span className="sr" role="status">
            {t("chat.typing")}
          </span>
        </>
      ) : (
        <Prose text={m.content} streaming={streaming} words={words} animate={animate} who={mine ? undefined : who.name} />
      )}
      {m.error && <p className="msg-error">{codeText(m.error) ?? t("chat.msg.aborted", { error: m.error })}</p>}
      {!streaming && draft === null && (
        <footer className="msg-foot">
          {m.swipes && m.swipes.length > 1 && (
            <Variants id={m.id} at={m.swipe_id ?? 0} total={m.swipes.length} locked={locked} onNew={write === "ready" ? () => act.regen(m) : undefined} newWait={write === "wait"} />
          )}
          {/* The short line in view, all of it on hover; where the row is narrow, all of it on a line of its own (chat.css). */}
          {timings && (
            <p className="msg-note num" title={t("chat.msg.timings", timings)}>
              <span className="cv-tm-short">{t("chat.msg.timings.short", timings)}</span>
              <span className="cv-tm-full">{t("chat.msg.timings", timings)}</span>
            </p>
          )}
          {/* Every action of a message lives in chat/MessageActions.tsx; a further one goes there. */}
          <MessageActions m={m} locked={locked} canRewind={canRewind} copied={copied} write={write} act={act} />
          {writeNote && (
            <p className="cv-write-note" role="alert">
              {writeNote}
            </p>
          )}
        </footer>
      )}
    </article>
  );
});

/** The unsent text of each chat stays on this device until it is sent, across reloads and chat changes. */
const draftKey = (chatId: number) => `rp-draft:${chatId}`;

function loadDraft(chatId: number): string {
  try {
    return localStorage.getItem(draftKey(chatId)) ?? "";
  } catch {
    return "";
  }
}

function saveDraft(chatId: number, text: string) {
  try {
    if (text) localStorage.setItem(draftKey(chatId), text);
    else localStorage.removeItem(draftKey(chatId));
  } catch {
    // Private windows and blocked storage: the draft lives as long as the page.
  }
}

/** The ready-made backdrops of "Aussehen" are fetched once the pointer or the focus reaches its
 *  button, so the panel opens with its pictures in place (over Wi-Fi they take a moment). */
let lookWarm = false;
function warmLook() {
  if (lookWarm) return;
  lookWarm = true;
  for (const p of PRESETS) {
    const img = new Image();
    img.decoding = "async";
    img.src = presetThumbUrl(p.id);
  }
}
const warmProps = { onPointerEnter: warmLook, onFocus: warmLook, onTouchStart: warmLook };

/** Something else on screen uses Escape first: a menu, the quick actions, a message being edited. */
const escapeTaken = () => document.querySelector('[role="menu"], [role="dialog"], [role="listbox"], .msg-edit') !== null;

const coarsePointer = window.matchMedia("(pointer: coarse)").matches;
const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
/** Distance from the end (px) under which the reader counts as "at the end" and new text is followed. */
const NEAR_END = 80;

/** The slim bar over a chat: who, which story, and the two ways into the card. */
type BarProps = {
  chat: ChatState | null;
  unsaved: boolean;
  onBack: () => void;
  personas: Persona[];
  activePersonaId: number | null;
  onPersona: (id: number | null) => void;
  ctx?: number | null;
  onFind?: () => void;
  /** The character's picture is shown large; the picture in the bar switches it. */
  stage?: boolean;
  onStage?: (on: boolean) => void;
  /** "Aussehen" of this chat is open; the button opens and closes it. */
  lookOpen?: boolean;
  onLook?: () => void;
};

/** From this share of the context on, the meter shows the warning colour. */
const CTX_WARN = 0.85;

/** Tokens for the meter: "12,4k" (k = 1024, as llama.cpp counts the context), small numbers whole. */
const tokensShort = (n: number) => (n >= 1000 ? `${numberFormat({ maximumFractionDigits: 1 }).format(n / 1024)}k` : num(n));

/** How full the model's context is after the newest measured answer of this chat: what that request
 *  read (new and from the cache) plus what it wrote. */
function contextUse(messages: Msg[], ctx: number | null | undefined) {
  if (!ctx) return null;
  let t: Msg["timings"];
  for (let i = messages.length - 1; i >= 0 && !t; i--) t = messages[i].timings;
  if (!t) return null;
  const used = t.processed + t.cached + t.predicted;
  const share = Math.min(1, used / ctx);
  return { used, ctx, share, warn: share >= CTX_WARN, pct: Math.round(share * 100) };
}

/** Phones: the meter is a line along the header's foot, its share is said in the subtitle. */
function ContextMeter({ messages, ctx: total }: { messages: Msg[]; ctx: number | null | undefined }) {
  const u = contextUse(messages, total);
  if (!u) return null;
  const { used, ctx, share, warn, pct } = u;
  return (
    <div
      className={`ctx-meter${warn ? " warn" : ""}`}
      role="meter"
      aria-label={t("chat.ctx.label")}
      aria-valuemin={0}
      aria-valuemax={ctx}
      aria-valuenow={Math.min(used, ctx)}
      aria-valuetext={t("chat.ctx.value", { used, ctx, pct })}
      title={t("chat.ctx.title", { used, ctx, pct })}
      data-used={used}
      data-ctx={ctx}
    >
      <span className="num">
        {tokensShort(used)} / {tokensShort(ctx)}
      </span>
      <i aria-hidden="true">
        <b style={{ width: `${Math.max(2, share * 100)}%` }} />
      </i>
    </div>
  );
}

function ChatBar({ chat, unsaved, onBack, personas, activePersonaId, onPersona, ctx, onFind, stage = false, onStage, lookOpen = false, onLook }: BarProps) {
  const c = chat?.character;
  const ctxUse = chat ? contextUse(chat.messages, ctx) : null;
  // A new reply counts once its first word is there (one written again was counted already).
  const count = chat
    ? chat.messages.filter((m) => m.id !== chat.generating || m.content.trim() !== "" || chat.prior?.id === m.id || m.timings != null || (m.swipes?.length ?? 0) > 0).length
    : 0;
  return (
    <header className="chat-bar">
      <button className="icon-btn back" onClick={onBack} aria-label={t("chat.bar.back")} title={t("chat.bar.back")}>
        <Icon name="back" />
      </button>
      {chat && c ? (
        <>
          {onStage ? (
            <button
              type="button"
              className="cv-av"
              aria-pressed={stage}
              aria-label={t("chat.bar.stage", { name: c.name })}
              title={stage ? t("chat.bar.stageHide") : t("chat.bar.stageShow")}
              onClick={() => onStage(!stage)}
            >
              <Portrait who={c} className="av" />
            </button>
          ) : (
            <Portrait who={c} className="av" />
          )}
          <div className="t">
            <h1>{c.name}</h1>
            <span className="num">
              {ctxUse && <span className={`ctx-inline${ctxUse.warn ? " warn" : ""}`}>{t("chat.ctx.inline", { pct: ctxUse.pct })} · </span>}
              <span className="chat-name">{chatName(chat.chat)}</span> · {t("chat.messages", { n: count })}
            </span>
          </div>
          <ContextMeter messages={chat.messages} ctx={ctx} />
          <div className="chat-tools">
            {onFind && (
              <button className="icon-btn" onClick={onFind} aria-label={t("chat.find")} title={t("chat.find.title")}>
                <Icon name="search" small />
              </button>
            )}
            {onLook && (
              <button className="icon-btn cv-look-btn" onClick={onLook} {...warmProps} aria-expanded={lookOpen} aria-label={t("chat.look.open")} title={t("chat.look.open.title")}>
                <Icon name="palette" small />
              </button>
            )}
            <ChatPersona personas={personas} activeId={activePersonaId} bound={chat.chat.persona_id} onBind={onPersona} />
            {/* Two different cards: this chat's own copy, and the character's card that every chat starts from. */}
            <a
              className={`btn sm${chat.chat.custom_card ? " on" : ""}`}
              href={`#/chat/${chat.chatId}/card`}
              aria-label={chat.chat.custom_card ? t("chat.bar.ownCard") : t("chat.bar.adaptCard")}
              title={t("chat.bar.cardTitle")}
            >
              <Icon name="sliders" small />
              <span className="hide-sm">{chat.chat.custom_card ? t("chat.bar.ownCardShort") : t("chat.bar.cardForChat")}</span>
            </a>
            <a
              className="btn sm ghost has-dot"
              href={`#/char/${c.id}/settings`}
              aria-label={t("chat.bar.editFigure")}
              title={t("chat.bar.editFigure.title")}
            >
              <Icon name="card" small />
              <span className="hide-sm">{t("chat.bar.editFigureShort")}</span>
              {unsaved && <i className="unsaved" title={t("chat.bar.unsaved")} />}
            </a>
          </div>
        </>
      ) : (
        <div className="t" aria-hidden="true">
          <span className="bar-ph short" />
        </div>
      )}
    </header>
  );
}

export function Chat({ chat, personas, activePersonaId, onPersona, engine, engineNote, unloaded = false, online, busy, notice, onSend, onStop, onEdit, onRewind, onFork, onBack, unsaved, ctx }: Props) {
  const [text, setText] = useState("");
  const sendKey = useGeneral().value.send_key;
  // The search in this chat: the bar is open while `find` is a string; `findAt` is the hit shown.
  const [find, setFind] = useState<string | null>(null);
  const [findAt, setFindAt] = useState(0);
  const [flash, setFlash] = useState<number | null>(null);
  const findInput = useRef<HTMLInputElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const [edit, setEdit] = useState<{ id: number; text: string } | null>(null);
  const [copied, setCopied] = useState<number | null>(null);
  // Why "Neu generieren" / "Weiter schreiben" did not start, under the message it was asked for.
  const [writeNote, setWriteNote] = useState<{ id: number; text: string } | null>(null);
  const frame = useRef<HTMLDivElement>(null);
  const [atEnd, setAtEnd] = useState(true);
  const stream = useRef<HTMLDivElement>(null);
  const col = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const pinned = useRef(true); // follow new text only while the reader is at the end
  const lastTop = useRef(0); // scroll position at the last scroll event
  // How this chat looks: reading settings (the app's, and what this chat changes), backdrop, large picture.
  const { view: lookView, setLook, setFrame, reload: reloadLook } = useChatLook(chat?.chatId);
  const [stage, setStage] = useStage();
  const stageSide = useStageSide(frame, chat !== null);
  // A tablet has its own place for the figure (tablet/ChatFigure) in place of the large picture.
  const tablet = useFormFactor() === "tablet";
  const [figure, setFigure] = useFigureShown();
  useVariantSwipe(stream, tablet && !!chat);
  const [lookOpen, setLookOpen] = useState(false);
  const closeLook = useCallback(() => setLookOpen(false), []);
  const reduced = useReducedMotion();

  // Personas by id. An entry keeps its object while what a message shows of it stays the same, so a
  // reload of the library does not re-render every message of a long chat.
  const known = useRef(new Map<number, Persona>());
  const personaById = useMemo(() => {
    const next = new Map<number, Persona>();
    for (const p of personas) {
      const old = known.current.get(p.id);
      const same =
        old && old.name === p.name && old.has_avatar === p.has_avatar && old.avatar_rev === p.avatar_rev && old.avatar_pos[0] === p.avatar_pos[0] && old.avatar_pos[1] === p.avatar_pos[1];
      next.set(p.id, same ? old : p);
    }
    known.current = next;
    return next;
  }, [personas]);
  const me = chat ? personaById.get(chatPersonaId(chat.chat, activePersonaId)) : undefined;

  // The message actions read the latest props through this ref, so their object never changes.
  const latest = useRef({ chat, edit, text, onEdit, onRewind, onFork, onStop });
  latest.current = { chat, edit, text, onEdit, onRewind, onFork, onStop };

  /** Saves the message being edited; `again`: then the chat's last reply (the one right after it) is
   *  written anew, as "Neu generieren" does. Unchanged text only closes the box (and writes anew). */
  const saveEdit = useMemo(
    () => (again: boolean) => {
      const { chat: c, edit: e, onEdit: save } = latest.current;
      if (!e || !e.text.trim()) return;
      const reply = again ? c?.messages[c.messages.length - 1] : undefined;
      const next = () => {
        setEdit(null);
        if (reply && reply.id !== e.id) write(reply.id, regenerate);
      };
      const before = c?.messages.find((m) => m.id === e.id)?.content;
      if (e.text.trim() === before) return next();
      void save(e.id, e.text).then((ok) => ok && next());
    },
    [], // eslint-disable-line react-hooks/exhaustive-deps
  );
  const saveAgain = useCallback(() => saveEdit(true), [saveEdit]);

  const act = useMemo<Actions>(
    () => ({
      copy: (m) => void copyText(m.content).then((ok) => ok && setCopied(m.id)),
      edit: (m) => setEdit({ id: m.id, text: m.content }),
      editText: (id, t) => setEdit({ id, text: t }),
      cancelEdit: () => setEdit(null),
      saveEdit: () => saveEdit(false),
      // Rewinding to one of your own messages hands its text back, to send it again in other words.
      rewind: (m) =>
        void latest.current.onRewind(m.id).then((ok) => {
          const c = latest.current.chat;
          if (ok && c && m.role === "user" && !latest.current.text.trim()) {
            setText(m.content);
            saveDraft(c.chatId, m.content);
            input.current?.focus();
          }
        }),
      fork: (id) => latest.current.onFork(id),
      // The new text arrives over the socket; the reader follows it from the end of the story.
      regen: (m) => write(m.id, regenerate),
      more: (m) => write(m.id, continueReply),
    }),
    [], // eslint-disable-line react-hooks/exhaustive-deps
  );
  function write(id: number, start: (id: number) => Promise<unknown>) {
    setWriteNote(null);
    pinned.current = true;
    setAtEnd(true);
    start(id).catch((e) => setWriteNote({ id, text: writeError(e) }));
  }

  // The last message there was when this chat came on screen: later ones are new and come in the
  // way the look says (chat.css `.cv-new`); what was there already never moves.
  const [seen, setSeen] = useState<{ chatId: number; last: number } | null>(null);
  if (chat && seen?.chatId !== chat.chatId) setSeen({ chatId: chat.chatId, last: chat.messages[chat.messages.length - 1]?.id ?? 0 });
  const newAfter = chat && seen?.chatId === chat.chatId ? seen.last : Infinity;

  // Long chats draw only the part around the visible one (see virtual.ts).
  const ids = useMemo(() => chat?.messages.map((m) => m.id) ?? [], [chat?.messages]);
  const list = useRef<HTMLDivElement>(null);
  const win = useWindow(ids, chat?.chatId, stream, list, pinned);
  // The story's scrollbar takes room at the right; in "Messenger breit" the message field leaves the
  // same room, so both share their right edge (chat.css `--cv-sb`).
  useLayoutEffect(() => {
    const el = stream.current;
    const at = root.current;
    if (!el || !at) return;
    const read = () => at.style.setProperty("--cv-sb", `${el.offsetWidth - el.clientWidth}px`);
    read();
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [chat?.chatId]);

  const toEnd = (smooth: boolean) => {
    const el = stream.current;
    if (!el) return;
    pinned.current = true;
    setAtEnd(true);
    if (win.enabled) return win.toEnd(smooth && !reducedMotion());
    el.scrollTo({ top: el.scrollHeight, behavior: smooth && !reducedMotion() ? "smooth" : "auto" });
  };

  useLayoutEffect(() => {
    const el = stream.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [chat?.messages, me]); // the persona can arrive after the messages and adds a line to each of yours

  // Whatever changes the height of the story (a late name, a picture, the window) keeps the end in view.
  useEffect(() => {
    const el = stream.current;
    const inner = col.current;
    if (!el || !inner) return;
    const ro = new ResizeObserver(() => {
      if (pinned.current) el.scrollTop = el.scrollHeight;
    });
    ro.observe(inner);
    ro.observe(el);
    return () => ro.disconnect();
  }, [chat !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  const chatId = chat?.chatId;
  useEffect(() => {
    pinned.current = true;
    setAtEnd(true);
    setEdit(null);
    setWriteNote(null);
    setText(chatId === undefined ? "" : loadDraft(chatId));
    if (!coarsePointer) input.current?.focus();
  }, [chatId]);

  // Escape stops the answer being written, unless a menu, a window or an edit needs it first.
  const writing = chat?.generating != null;
  useEffect(() => {
    if (!writing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || e.isComposing || e.repeat) return;
      // An edit box that closed on this very key is already gone from the page, so look at the target too.
      if (latest.current.edit || escapeTaken() || (e.target as Element | null)?.closest?.(".msg-edit")) return;
      e.preventDefault();
      latest.current.onStop();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [writing]);

  // A message to show: scrolled into view (drawn first in a long chat) and no longer followed to the end.
  const toIndex = win.toIndex;
  const showMessage = (index: number) => {
    pinned.current = false;
    setAtEnd(false);
    toIndex(index);
  };

  // A search hit from Ctrl+K: once this chat holds the message, jump there and highlight it.
  useEffect(() => {
    if (chatId === undefined) return;
    const take = () => {
      const c = latest.current.chat;
      if (!c || c.chatId !== chatId) return;
      const id = takeJump(chatId, (mid) => c.messages.some((m) => m.id === mid));
      if (id === null) return;
      showMessage(c.messages.findIndex((m) => m.id === id));
      setFlash(id);
    };
    take();
    return onJump(take);
  }, [chatId, ids]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (flash === null) return;
    const t = window.setTimeout(() => setFlash(null), 2400);
    return () => window.clearTimeout(t);
  }, [flash]);

  // Search in this chat: every message that holds all the words (case, accents and umlauts do not
  // matter), in story order. Folded texts are kept per message while they stay the same.
  const words = useMemo(() => (find === null ? [] : findWords(find)), [find]);
  const folded = useRef(new Map<number, { content: string; text: string }>());
  const hits = useMemo(() => {
    const out: number[] = [];
    if (!chat || words.length === 0) return out;
    chat.messages.forEach((m, i) => {
      let f = folded.current.get(m.id);
      if (!f || f.content !== m.content) {
        f = { content: m.content, text: fold(m.content).text };
        folded.current.set(m.id, f);
      }
      const t = f.text;
      if (words.every((w) => t.includes(w))) out.push(i);
    });
    return out;
  }, [chat?.messages, words]); // eslint-disable-line react-hooks/exhaustive-deps
  const shownAt = Math.min(findAt, hits.length - 1);
  const hitId = hits.length > 0 && chat ? (chat.messages[hits[shownAt]]?.id ?? null) : null;
  const hitIds = useMemo(() => new Set(hits.map((i) => chat?.messages[i]?.id)), [hits]); // eslint-disable-line react-hooks/exhaustive-deps
  const goHit = (k: number) => {
    if (hits[k] === undefined) return;
    setFindAt(k);
    showMessage(hits[k]);
  };
  const step = (d: 1 | -1) => hits.length > 0 && goHit((shownAt + d + hits.length) % hits.length);
  // New words: the first hit from the part on screen on, else the last one.
  const wordsKey = words.join(" ");
  useEffect(() => {
    if (hits.length === 0) return;
    const k = hits.findIndex((i) => i >= win.start);
    goHit(k === -1 ? hits.length - 1 : k);
  }, [wordsKey]); // eslint-disable-line react-hooks/exhaustive-deps

  const closeFind = () => {
    setFind(null);
    if (!coarsePointer) input.current?.focus();
  };
  // Ctrl+F (Cmd+F) opens the search of this chat instead of the browser's, which finds only what is drawn.
  useEffect(() => {
    // Drawn at once, so the keys typed right after Ctrl+F land in the field.
    const open = () => {
      flushSync(() => setFind((f) => f ?? ""));
      findInput.current?.focus();
      findInput.current?.select();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== "f" || !(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
      const a = document.activeElement;
      if ((a && a !== document.body && !root.current?.contains(a)) || document.querySelector('[role="dialog"]')) return;
      e.preventDefault();
      open();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener(CHAT_FIND_EVENT, open);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener(CHAT_FIND_EVENT, open);
    };
  }, []);
  useEffect(() => setFind(null), [chatId]);
  useEffect(() => setLookOpen(false), [chatId]);

  useEffect(() => {
    if (copied === null) return;
    const t = window.setTimeout(() => setCopied(null), 1500);
    return () => window.clearTimeout(t);
  }, [copied]);

  useLayoutEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
    el.style.overflowY = el.scrollHeight > 200 ? "auto" : "hidden";
  }, [text, chat?.chatId]);

  if (!chat) {
    // Tablet: the figure keeps its place while the next chat loads (it is known from the lists).
    const held = tablet && figure ? heldFigure() : null;
    return (
      <div className="chat-main chat" aria-busy="true">
        <ChatBar chat={null} unsaved={false} onBack={onBack} personas={personas} activePersonaId={activePersonaId} onPersona={onPersona} />
        {held ? (
          <ChatFigure character={held.character} chat={held.chat} messages={held.chat.messages} onClose={() => setFigure(false)} />
        ) : (
          tablet && figure && <div className="tb-figure" aria-hidden="true" />
        )}
        <div className="stream">
          <div className="col skeleton" aria-label={t("chat.loading")}>
            {[0, 1, 2].map((i) => (
              <div className="msg-ph" key={i}>
                <span className="bar-ph short" />
                <span className="bar-ph" />
                <span className="bar-ph" />
              </div>
            ))}
          </div>
        </div>
      </div>
    );
  }

  const { character, messages, generating } = chat;
  const writingHere = generating !== null;
  // Why sending has to wait, if it has to; said above the field instead of hidden in its placeholder.
  const blocked = !online
    ? t("chat.blocked.offline")
    : unloaded // sending loads the model again
      ? busy && !writingHere
        ? t("chat.blocked.busy")
        : ""
      : engine === "error"
      ? engineNote
        ? t("chat.blocked.errorWhy", { note: engineNote })
        : t("chat.blocked.error")
      : engine === "offline" && engineNote
        ? t("chat.blocked.notRunning", { note: engineNote })
        : engine !== "ready"
          ? t("chat.blocked.loading")
          : busy && !writingHere
            ? t("chat.blocked.busy")
            : "";
  const statusDot = !online || engine === "error" || (engine === "offline" && engineNote) ? "bad" : "wait";
  const canSend = !blocked && !busy && text.trim().length > 0;
  const send = () => {
    if (!canSend) return;
    onSend(text.trim());
    setText("");
    saveDraft(chat.chatId, "");
    toEnd(false);
  };
  const lastIndex = messages.length - 1;
  // Only the chat's last reply to a message of yours can be written again or on (the server's rule
  // too); it waits while the model writes anywhere or sending is blocked.
  const canWrite = lastIndex > 0 && messages[lastIndex].role === "assistant" && messages[lastIndex - 1].role === "user";
  const lastWrite: Write = !canWrite ? "no" : blocked || busy ? "wait" : "ready";
  const prior = chat.prior && chat.prior.id === generating ? chat.prior : null;
  const look = resolve(lookView?.global ?? {}, lookView?.look);
  const lookAttrs = lookProps(look);
  const animate = look.animate !== false && !reduced;
  const backdrop = shownBackdrop(chat.chatId, lookView);
  /** Wraps the selection of the message field in `mark` (or puts a pair there, the caret between). */
  const wrap = (mark: string) => {
    const el = input.current;
    if (!el) return;
    const [a, b] = [el.selectionStart, el.selectionEnd];
    // Changed in the field itself, caret included, and only then mirrored into the state: React
    // then finds the value it renders already there and leaves the caret alone. (Setting the caret
    // a frame later lost the race against keys typed right after Ctrl+I.)
    el.focus();
    el.setRangeText(`${mark}${el.value.slice(a, b)}${mark}`, a, b);
    el.setSelectionRange(a + mark.length, b + mark.length);
    setText(el.value);
    saveDraft(chat.chatId, el.value);
  };

  return (
    <div className={`chat-main chat${backdrop ? " cv-has-bg" : ""}`} style={{ ...hueStyle(character.name), ...lookAttrs.style }} {...lookAttrs.data} ref={root}>
      <ChatBar
        chat={chat}
        unsaved={unsaved}
        onBack={onBack}
        personas={personas}
        activePersonaId={activePersonaId}
        onPersona={onPersona}
        ctx={ctx}
        onFind={() => window.dispatchEvent(new Event(CHAT_FIND_EVENT))}
        stage={tablet ? figure : stage}
        onStage={tablet ? setFigure : setStage}
        lookOpen={lookOpen}
        onLook={() => setLookOpen((o) => !o)}
      />
      {tablet && figure && <ChatFigure character={character} chat={chat.chat} messages={messages.length} onClose={() => setFigure(false)} />}
      {find !== null && (
        <div className="find-bar" role="search" aria-label={t("chat.find")}>
          <Icon name="search" small />
          <input
            ref={findInput}
            className="find-input"
            value={find}
            placeholder={t("chat.find")}
            aria-label={t("chat.find")}
            spellCheck={false}
            autoComplete="off"
            enterKeyHint="search"
            onChange={(e) => setFind(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                closeFind();
              } else if (e.key === "Enter" || e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                step(e.key === "ArrowUp" || (e.key === "Enter" && e.shiftKey) ? -1 : 1);
              }
            }}
          />
          <span className="find-count num" role="status" aria-live="polite">
            {words.length === 0 ? "" : hits.length === 0 ? t("chat.find.none") : t("chat.find.at", { at: shownAt + 1, total: hits.length })}
          </span>
          <button className="icon-btn" onClick={() => step(-1)} disabled={hits.length === 0} aria-label={t("chat.find.prev")} title={t("chat.find.prev.title")}>
            <span className="flip">
              <Icon name="chevron" small />
            </span>
          </button>
          <button className="icon-btn" onClick={() => step(1)} disabled={hits.length === 0} aria-label={t("chat.find.next")} title={t("chat.find.next.title")}>
            <Icon name="chevron" small />
          </button>
          <button className="icon-btn" onClick={closeFind} aria-label={t("chat.find.close")} title={t("chat.closeEsc")}>
            <Icon name="close" small />
          </button>
        </div>
      )}

      <div className={`cv-frame${stage && !tablet ? " cv-staged" : ""}`} ref={frame} data-cv-stage={stageSide}>
      {stage && !tablet && <Stage who={character} onClose={() => setStage(false)} />}
      <div className="cv-main">
      {backdrop && <Backdrop url={backdrop.url} frame={backdrop} />}
      <div className="chat-body">
        <div
          className="stream"
          ref={stream}
          onScroll={(e) => {
            const el = e.currentTarget;
            // A jump to a message is on its way: this scroll still comes from the old place (often the
            // end), so it must not pin the reader there again.
            if (win.revealing()) {
              lastTop.current = el.scrollTop;
              return;
            }
            let near = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_END;
            // In a long chat the page can grow under a reader at the end (messages measured late):
            // there only scrolling up leaves the end.
            if (win.enabled && pinned.current && el.scrollTop >= lastTop.current - 1) near = true;
            lastTop.current = el.scrollTop;
            pinned.current = near;
            if (near !== atEnd) setAtEnd(near);
            win.update();
          }}
        >
          <div className="col" ref={col}>
            {messages.length === 0 && (
              <div className="chat-empty">
                <Portrait who={character} className="pic" />
                <p>{t("chat.empty.noGreeting")}</p>
                <p className="muted">{t("chat.empty.first", { name: character.name })}</p>
              </div>
            )}
            <div ref={list}>
              {win.before > 0 && <div aria-hidden="true" style={{ height: win.before }} />}
              {messages.slice(win.start, win.end + 1).map((m, k) => {
                const i = win.start + k;
                return (
                  <Message
                    key={m.id}
                    m={m}
                    who={character}
                    me={m.role === "user" ? ((m.persona_id != null ? personaById.get(m.persona_id) : undefined) ?? me) : undefined}
                    streaming={m.id === generating}
                    draft={edit?.id === m.id ? edit.text : null}
                    locked={writingHere}
                    canRewind={i > 0 || m.role === "user"}
                    last={i === lastIndex}
                    copied={copied === m.id}
                    act={act}
                    words={hitIds.has(m.id) ? words : null}
                    hit={hitId === m.id}
                    flash={flash === m.id}
                    animate={animate}
                    write={i === lastIndex ? lastWrite : "no"}
                    writeNote={writeNote?.id === m.id ? writeNote.text : ""}
                    times={look.times === true}
                    fresh={m.id > newAfter}
                    prior={prior?.id === m.id ? prior.text : ""}
                    again={i === lastIndex - 1 ? lastWrite : "no"}
                    onSaveAgain={saveAgain}
                  />
                );
              })}
              {win.after > 0 && <div aria-hidden="true" style={{ height: win.after }} />}
            </div>
            {notice && (
              <p className="msg-error" role="alert">
                {notice}
              </p>
            )}
          </div>
        </div>
        {!atEnd && (
          <button type="button" className="to-end" onClick={() => toEnd(true)}>
            <Icon name="down" small />
            {writingHere ? t("chat.toNew") : t("chat.toEnd")}
          </button>
        )}
      </div>

      <form
        className="composer"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        <div className="col">
          {blocked && (
            <p className="chat-status" role="status">
              <i className={`dot ${statusDot}`} />
              {blocked}
            </p>
          )}
          <div className="box">
            {/* On a phone the chat's bar has no room for it: "Aussehen" sits here until you type. */}
            <button type="button" className="icon-btn cv-look-box" onClick={() => setLookOpen((o) => !o)} {...warmProps} aria-expanded={lookOpen} aria-label={t("chat.look.open")} title={t("chat.look.open.title")}>
              <Icon name="palette" small />
            </button>
            <textarea
              ref={input}
              rows={1}
              value={text}
              onChange={(e) => {
                setText(e.target.value);
                saveDraft(chat.chatId, e.target.value);
              }}
              onKeyDown={(e) => {
                // Arrow up in the empty field opens your last message for editing, as in other chat apps.
                if (e.key === "ArrowUp" && text === "" && !writingHere && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey && !e.nativeEvent.isComposing) {
                  const lastMine = [...messages].reverse().find((m) => m.role === "user");
                  if (lastMine) {
                    e.preventDefault();
                    if (!atEnd) toEnd(false);
                    act.edit(lastMine);
                  }
                  return;
                }
                // Ctrl + I sets the selection as an *action*, as in a text editor.
                if (e.key.toLowerCase() === "i" && (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey) {
                  e.preventDefault();
                  wrap("*");
                  return;
                }
                // On touch keyboards Enter is a line break; sending is the button.
                // Which key sends is a setting (Enter, or Ctrl + Enter for long texts).
                if (!coarsePointer && !e.nativeEvent.isComposing && sendsMessage(e, sendKey)) {
                  e.preventDefault();
                  send();
                }
              }}
              placeholder={t("chat.input", { name: character.name })}
              aria-label={t("chat.input", { name: character.name })}
              enterKeyHint={coarsePointer ? "enter" : "send"}
            />
            <button type="button" className="icon-btn cv-star" onMouseDown={(e) => e.preventDefault() /* the field keeps focus and selection */} onClick={() => wrap("*")} aria-label={t("chat.star")} title={t("chat.star.title")}>
              <span aria-hidden="true">*</span>
            </button>
            {writingHere ? (
              <button type="button" className="send stop" onClick={onStop} aria-label={t("chat.stop")} title={t("chat.stop.title")}>
                <Icon name="stop" />
              </button>
            ) : (
              <button type="submit" className="send" disabled={!canSend} aria-label={t("chat.send")} title={t("chat.send")}>
                <Icon name="send" />
              </button>
            )}
          </div>
          <p className="hint">
            {sendKey === "ctrl_enter" ? (
              <>
                <span>{rich("chat.hint.ctrlSend", { ctrl: <kbd>{t("shell.key.ctrl")}</kbd>, enter: <kbd>Enter</kbd> })}</span>
                <span>{rich("chat.hint.enterLine", { enter: <kbd>Enter</kbd> })}</span>
              </>
            ) : (
              <>
                <span>{rich("chat.hint.enterSend", { enter: <kbd>Enter</kbd> })}</span>
                <span>{rich("chat.hint.shiftLine", { shift: <kbd>{t("chat.key.shift")}</kbd>, enter: <kbd>Enter</kbd> })}</span>
              </>
            )}
            {/* The key that works right now: Escape while the model writes, arrow up otherwise. */}
            {!coarsePointer &&
              (writingHere ? (
                <span>{rich("chat.hint.escStop", { esc: <kbd>Esc</kbd> })}</span>
              ) : (
                <span>
                  {rich("chat.hint.upEdit", {
                    up: (
                      <kbd>
                        <span aria-hidden="true">↑</span>
                        <span className="sr">{t("chat.key.up")}</span>
                      </kbd>
                    ),
                  })}
                </span>
              ))}
            <span>{t("chat.hint.stars")}</span>
          </p>
        </div>
      </form>
      </div>
      {lookOpen &&
        (lookView ? (
          <LookPanel chatId={chat.chatId} character={character} view={lookView} setLook={setLook} setFrame={setFrame} reload={reloadLook} stage={tablet ? figure : stage} onStage={tablet ? setFigure : setStage} onClose={closeLook} />
        ) : (
          <LookPanelLoading onClose={closeLook} />
        ))}
      </div>
    </div>
  );
}
