// What the tablet layout asks the server itself: the chats of one figure (for the column beside a
// chat and the figure pane of the library) and a new chat with it. Also what this device remembers
// about the layout.

import { useEffect, useState } from "react";
import { api, errorText, postJson, type CharInfo, type ChatInfo } from "../api";
import { t } from "../i18n";

/** The chats of one figure, newest first; null while they load. `rev` changes whenever they may have changed. */
export function useCharChats(characterId: number | null, rev: unknown): { chats: ChatInfo[] | null; error: string } {
  const [state, setState] = useState<{ id: number | null; chats: ChatInfo[] | null; error: string }>({ id: null, chats: null, error: "" });
  useEffect(() => {
    if (characterId === null) return;
    let stale = false;
    api<{ chats: ChatInfo[] }>(`/characters/${characterId}/chats`)
      .then((r) => !stale && setState({ id: characterId, chats: r.chats, error: "" }))
      .catch((e) => !stale && setState((s) => ({ id: characterId, chats: s.id === characterId ? s.chats : null, error: errorText(e) })));
    return () => {
      stale = true;
    };
  }, [characterId, rev]);
  // Another figure's list never shows while this one's loads.
  return state.id === characterId ? { chats: state.chats, error: state.error } : { chats: null, error: "" };
}

/** Starts a new chat with the figure and opens it. Resolves to an error text, or "" when it worked. */
export async function openNewChat(characterId: number): Promise<string> {
  try {
    const { chat_id } = await postJson<{ chat_id: number }>("/chats", { character_id: characterId, fresh: true });
    location.hash = `#/chat/${chat_id}`;
    return "";
  } catch (e) {
    return t("cards.pane.newFailed", { error: errorText(e) });
  }
}

// Whose chat an address belongs to, as far as the lists on screen have told: the rail's last chats,
// the chats beside an open chat, the chats in the library's pane. A chat opened from one of them
// shows its figure at once, before the chat itself has arrived.
const held = new Map<number, { character: CharInfo; chat: ChatInfo }>();

export function holdFigures(character: CharInfo, chats: ChatInfo[]) {
  for (const chat of chats) held.set(chat.id, { character, chat });
}

/** The figure of the chat the address names, if a list has shown it. */
export function heldFigure(): { character: CharInfo; chat: ChatInfo } | null {
  const m = /^#\/chat\/(\d+)$/.exec(location.hash);
  return (m && held.get(Number(m[1]))) || null;
}

const FIGURE_KEY = "rp-tb-figure";

/** Whether a chat shows the figure beside (sideways) or above (upright) the story. On by default;
 *  the choice is this device's. */
export function useFigureShown(): [boolean, (on: boolean) => void] {
  const [on, setOn] = useState(() => {
    try {
      return localStorage.getItem(FIGURE_KEY) !== "0";
    } catch {
      return true;
    }
  });
  const set = (next: boolean) => {
    setOn(next);
    try {
      localStorage.setItem(FIGURE_KEY, next ? "1" : "0");
    } catch {
      // Blocked storage: the choice lives as long as the page.
    }
  };
  return [on, set];
}
