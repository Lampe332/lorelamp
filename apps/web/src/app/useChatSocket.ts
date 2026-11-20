// The one WebSocket of the app: it reconnects by itself, keeps the open chat up to date (history,
// streamed tokens, the finished answer) and tells when the library changed.

import { keysUnder, t } from "../i18n";
import { useCallback, useEffect, useRef, useState } from "react";
import { ERROR_TEXT, token, wsUrl, type Msg, type ServerEvent, type Timings } from "../api";
import type { ChatState } from "../Chat";
import { tellLookChanged } from "../chat/look";
import { tellGeneralChanged } from "../general/store";
import { forwardImportEvent } from "../import/job";
import { logEvent } from "../log";
import { applyDownloadEvent, tellCatalogNews } from "../models/store";
import { answerReady } from "../notify";
import { tokenBuffer } from "../tokens";
import { forwardBench } from "./benchEvents";
import { forwardEngineDownload } from "./engineDownload";
import { tellModelNews } from "./engineEvents";

/** Why the model wrote nothing, in the chat's own words (`chat.write.<code>`, e.g. `context_full`:
 *  the chat no longer fits the model's context, not even shortened); null for a code it has none for. */
const CHAT_CODES = keysUnder("chat.write.");
export function codeText(code: string): string | null {
  const own = CHAT_CODES.find((k) => k === `chat.write.${code}`);
  return own ? t(own) : null;
}

/** A reply written again starts empty in the chat's history: the text it had until then, to show
 *  until its first new word (Chat.tsx `prior`). */
function priorText(old: ChatState | null, chatId: number, generating: number | null, messages: Msg[]): ChatState["prior"] {
  if (generating === null) return undefined;
  const now = messages.find((m) => m.id === generating);
  if (!now || now.content.trim() !== "") return undefined;
  if (old?.chatId === chatId) {
    if (old.prior?.id === generating) return old.prior;
    const before = old.messages.find((m) => m.id === generating)?.content;
    if (before?.trim()) return { id: generating, text: before };
  }
  // Opened while it is written again: the variant on show until then.
  const shown = now.swipes?.[now.swipe_id ?? now.swipes.length - 1];
  return shown?.trim() ? { id: generating, text: shown } : undefined;
}

/** `chatId`: the chat on screen (null: none). `loadLibrary` (everything), `loadChats` (only what a
 *  finished reply changes) and `onToast` must keep their identity. */
export function useChatSocket(chatId: number | null, loadLibrary: () => Promise<void>, loadChats: () => Promise<void>, onToast: (text: string) => void) {
  const [online, setOnline] = useState(false);
  const [busy, setBusy] = useState(false);
  const [chat, setChat] = useState<ChatState | null>(null);
  const [notice, setNotice] = useState("");
  const [last, setLast] = useState<Timings | null>(null);
  const ws = useRef<WebSocket | null>(null);
  const current = useRef(chatId); // for the socket handlers, which outlive a render
  const name = useRef<string | null>(null); // the character of the chat on screen

  // Streamed text is drawn at most once per frame (see tokens.ts).
  const [tokens] = useState(() =>
    tokenBuffer((parts) =>
      setChat((c) => c && { ...c, messages: c.messages.map((m) => (parts.has(m.id) ? { ...m, content: m.content + parts.get(m.id) } : m)) }),
    ),
  );

  const onEvent = useCallback(
    (ev: ServerEvent) => {
      // How chats look changed (on this or another device): the open chat and the settings tab read it again.
      if ((ev as { type: string }).type === "look") return tellLookChanged();
      // The general settings changed (on this or another device).
      if ((ev as { type: string }).type === "general") return tellGeneralChanged();
      const here = "chat_id" in ev && ev.chat_id === current.current;
      if (ev.type !== "token") tokens.flush(); // text still waiting comes first
      switch (ev.type) {
        case "history":
          setBusy(ev.busy);
          if (here) {
            // The last reply is written again or on: an older hint is over.
            if (ev.generating !== null) setNotice("");
            name.current = ev.character.name;
            setChat((old) => ({
              chatId: ev.chat_id,
              chat: ev.chat,
              character: ev.character,
              messages: ev.messages,
              generating: ev.generating,
              prior: priorText(old, ev.chat_id, ev.generating, ev.messages),
            }));
          }
          break;
        case "message":
          if (here) setChat((c) => c && { ...c, messages: [...c.messages, ev.message] });
          break;
        case "assistant_start":
          setBusy(true);
          if (here) {
            setNotice("");
            setChat((c) => c && { ...c, generating: ev.id, messages: [...c.messages, { id: ev.id, role: "assistant", content: "", at: Math.floor(Date.now() / 1000) }] });
          }
          break;
        case "token":
          if (here) tokens.push(ev.id, ev.text);
          break;
        case "done":
          setBusy(false);
          if (ev.timings) setLast(ev.timings);
          void loadChats(); // message counts and "last used" changed, nothing else
          if (!ev.removed && !ev.stopped && ev.kept !== false) answerReady(ev.chat_id, here ? name.current : null);
          if (!here) break;
          if ((ev.removed || ev.kept === false) && ev.error) setNotice(codeText(ev.error) ?? ERROR_TEXT[ev.error] ?? t("shell.toast.noReply", { error: ev.error }));
          // A reply written again or on: the chat's history follows and holds what it is now.
          if (ev.mode === "variant" || ev.mode === "continue") {
            setChat((c) => c && { ...c, generating: null });
            break;
          }
          setChat(
            (c) =>
              c && {
                ...c,
                generating: null,
                messages: ev.removed
                  ? c.messages.filter((m) => m.id !== ev.id)
                  : c.messages.map((m) =>
                      m.id === ev.id ? { ...m, content: ev.content, timings: ev.timings ?? undefined, error: ev.error ?? undefined } : m,
                    ),
              },
          );
          break;
        case "library":
          void loadLibrary();
          break;
        case "models":
          tellModelNews({ type: "models" });
          break;
        case "engine": // the server logs these itself
          tellModelNews({ type: "engine", event: ev.event, profile_id: ev.profile_id, port: ev.port ?? null });
          break;
        case "import":
          forwardImportEvent(ev);
          break;
        case "engine_download":
          forwardEngineDownload(ev);
          break;
        case "bench":
          forwardBench(ev);
          break;
        case "download":
          applyDownloadEvent(ev);
          break;
        case "catalog":
          tellCatalogNews();
          break;
        case "notice":
          logEvent("warn", "chat", `Hinweis vom Server: ${ev.code}`, { chat_id: current.current ?? undefined });
          if (ev.code === "chat_missing") {
            onToast(ERROR_TEXT.chat_missing);
            location.hash = "#/";
          } else {
            setNotice(codeText(ev.code) ?? ERROR_TEXT[ev.code] ?? ERROR_TEXT.unknown);
          }
          break;
      }
    },
    [loadLibrary, loadChats, onToast, tokens],
  );

  useEffect(() => {
    let closed = false;
    let retry: number | undefined;
    let attempts = 0; // failed tries since the socket was last up
    let lostAt: number | null = null;
    const connect = () => {
      const s = new WebSocket(wsUrl(`/ws?token=${encodeURIComponent(token)}`));
      ws.current = s;
      s.onopen = () => {
        if (lostAt === null) logEvent("info", "ws", "Verbindung zur App hergestellt");
        else logEvent("info", "ws", "Verbindung wiederhergestellt", { data: { attempts, offline_ms: Date.now() - lostAt } });
        attempts = 0;
        lostAt = null;
        setOnline(true);
        void loadLibrary();
        if (current.current !== null) s.send(JSON.stringify({ type: "open", chat_id: current.current }));
      };
      s.onmessage = (e) => onEvent(JSON.parse(e.data as string) as ServerEvent);
      s.onclose = (e) => {
        setOnline(false);
        if (closed) return;
        if (lostAt === null) {
          lostAt = Date.now();
          logEvent("warn", "ws", "Verbindung zur App getrennt", { data: { code: e.code, clean: e.wasClean } });
        }
        attempts += 1;
        retry = window.setTimeout(connect, 1500);
      };
    };
    connect();
    return () => {
      closed = true;
      window.clearTimeout(retry);
      ws.current?.close();
    };
  }, [onEvent, loadLibrary]);

  // Another chat on screen: forget the old one and ask the server for the new one's history.
  useEffect(() => {
    current.current = chatId;
    setChat(null);
    setNotice("");
    if (chatId !== null && ws.current?.readyState === WebSocket.OPEN) {
      ws.current.send(JSON.stringify({ type: "open", chat_id: chatId }));
    }
  }, [chatId]);

  /** Whether a message sent now goes out at once. */
  const isOpen = () => ws.current?.readyState === WebSocket.OPEN;
  const send = (msg: object) => ws.current?.send(JSON.stringify(msg));

  return { online, busy, chat, setChat, notice, last, isOpen, send };
}
