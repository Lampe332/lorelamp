// "Beim Start öffnen": when the app is opened on its start address, it goes on to the page chosen
// in the settings (the chat written in last, or the characters). Only once per window: a reload,
// a link straight to a page and the way back to "Start" are left alone.

import { useEffect } from "react";
import type { Library } from "../api";
import { useGeneral, type StartPage } from "./store";

const SEEN_KEY = "rp-started";

/** This window was just opened on the start address (not reloaded, not sent to a page). */
const fresh = (() => {
  if (location.hash !== "" && location.hash !== "#/") return false;
  try {
    if (sessionStorage.getItem(SEEN_KEY)) return false;
    sessionStorage.setItem(SEEN_KEY, "1");
  } catch {
    // No session storage: every load counts as an opening.
  }
  return true;
})();

/** Where the app goes at its start; null = it stays on the start screen. */
export function startHash(page: StartPage, library: Pick<Library, "recent">): string | null {
  if (page === "characters") return "#/chars";
  if (page === "last_chat") {
    const last = library.recent[0];
    return last ? `#/chat/${last.chat.id}` : null; // no chat yet: the start screen
  }
  return null;
}

let done = !fresh;

/** Runs the choice once, as soon as the library and the settings are there. */
export function useStartPage(library: Library | null) {
  const { value, loaded, failed } = useGeneral();
  useEffect(() => {
    if (done || !library || !(loaded || failed)) return;
    done = true;
    // The user was quicker and is somewhere else already.
    if (location.hash !== "" && location.hash !== "#/") return;
    const hash = startHash(value.start_page, library);
    if (hash) location.replace(hash);
  }, [library, loaded, failed, value.start_page]);
}
