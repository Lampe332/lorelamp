// The library (characters, recent chats, personas, settings) as the server has it.
//
// Loads run one at a time: a request while one runs is merged into the next run (any number of
// "library" events become at most one more load), and since answers arrive in the order they were
// asked for, an older answer can never overwrite a newer one. A finished reply only changes the
// recent chats, the figures and the chat counts and times of the characters, so it asks for just
// that part (`GET /library?part=chats`: no card texts) and leaves every other card as it is.

import { useState } from "react";
import { api, errorText, type Library, type Recent, type Stats } from "../api";
import { setPrivateTerms } from "../log";

/** What `GET /library?part=chats` answers. */
type ChatsPart = { recent: Recent[]; stats: Stats; activity: { id: number; chats: number; last_chat_at: number | null }[] };

type Kind = "chats" | "all";

/** Names the activity log must never carry. */
function guard(lib: Library) {
  setPrivateTerms([...lib.characters.map((c) => c.name), ...lib.characters.flatMap((c) => c.own_tags), ...lib.recent.map((r) => r.chat.title), lib.user_name, ...lib.personas.map((p) => p.name)]);
}

/** The library with a finished reply's part put in (a character missing from `activity` has no
 *  chat); cards whose counts did not change stay the same objects. */
function merge(lib: Library, part: ChatsPart): Library {
  const act = new Map(part.activity.map((a) => [a.id, a]));
  const characters = lib.characters.map((c) => {
    const { chats, last_chat_at } = act.get(c.id) ?? { chats: 0, last_chat_at: null };
    return chats !== c.chats || last_chat_at !== c.last_chat_at ? { ...c, chats, last_chat_at } : c;
  });
  return { ...lib, characters, recent: part.recent, stats: part.stats };
}

export function useLibrary() {
  const [library, setLibrary] = useState<Library | null>(null);
  const [loadError, setLoadError] = useState("");

  // The queue lives as long as the app; its functions keep their identity (the socket depends on it).
  const [queue] = useState(() => {
    let pending: Kind | null = null; // asked for, not yet started
    let running: Promise<void> | null = null;
    let current: Library | null = null;

    const pump = async () => {
      while (pending) {
        // Before the first full answer there is nothing to put a part into.
        const kind: Kind = pending === "chats" && current ? "chats" : "all";
        pending = null;
        try {
          if (kind === "all") {
            current = await api<Library>("/library");
          } else {
            const part = await api<ChatsPart>("/library?part=chats");
            current = current && merge(current, part);
          }
          if (current) {
            guard(current);
            setLibrary(current);
          }
          setLoadError("");
        } catch (e) {
          setLoadError(errorText(e));
        }
      }
    };

    const ask = (kind: Kind): Promise<void> => {
      pending = pending === "all" || kind === "all" ? "all" : "chats";
      running ??= pump().finally(() => {
        running = null;
        if (pending) void ask(pending); // asked for in the moment the last run ended
      });
      return running;
    };

    return { loadLibrary: () => ask("all"), loadChats: () => ask("chats") };
  });

  return { library, loadError, loadLibrary: queue.loadLibrary, loadChats: queue.loadChats };
}
