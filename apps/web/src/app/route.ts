// The app's pages, read from the address (`location.hash`).

import { startTransition, useEffect, useState } from "react";
import type { PersonaSel } from "../persona/common";
import type { SettingsTab } from "../Settings";

export type Route =
  | { view: "hub" }
  | { view: "library" }
  | { view: "builder"; from: number | null }
  | { view: "import" }
  | { view: "chats"; charId: number }
  | { view: "settings"; charId: number }
  | { view: "chat"; chatId: number }
  | { view: "chat-card"; chatId: number }
  | { view: "app-settings"; tab: SettingsTab }
  | { view: "personas"; sel: PersonaSel | null };

/** `#/` start, `#/chars` library, `#/chars/new` card builder (`#/chars/new/from/3`: from a copy of 3), `#/import` import, `#/char/3/chats`, `#/char/3/settings`, `#/chat/12`, `#/chat/12/card`,
 *  `#/settings` (general), `#/settings/chat`, `#/settings/model`, `#/settings/expert`, `#/settings/suggestions`, `#/settings/backups`, `#/settings/phone`, `#/settings/logs`, `#/personas`, `#/personas/new`, `#/personas/3`. */
export const routeFromHash = (): Route => {
  const h = location.hash;
  if (h === "#/chars") return { view: "library" };
  if (h === "#/chars/new") return { view: "builder", from: null };
  const fromCard = /^#\/chars\/new\/from\/(\d+)$/.exec(h);
  if (fromCard) return { view: "builder", from: Number(fromCard[1]) };
  if (h === "#/import") return { view: "import" };
  if (h === "#/settings") return { view: "app-settings", tab: "general" };
  if (h === "#/settings/logs") return { view: "app-settings", tab: "logs" };
  if (h === "#/settings/chat") return { view: "app-settings", tab: "chat" };
  if (h === "#/settings/model") return { view: "app-settings", tab: "model" };
  if (h === "#/settings/expert") return { view: "app-settings", tab: "expert" };
  if (h === "#/settings/backups") return { view: "app-settings", tab: "backups" };
  if (h === "#/settings/suggestions") return { view: "app-settings", tab: "suggestions" };
  if (h === "#/settings/phone") return { view: "app-settings", tab: "phone" };
  if (h === "#/personas") return { view: "personas", sel: null };
  if (h === "#/personas/new") return { view: "personas", sel: "new" };
  let m = /^#\/personas\/(\d+)$/.exec(h);
  if (m) return { view: "personas", sel: Number(m[1]) };
  m = /^#\/char\/(\d+)(?:\/(chats|settings))?$/.exec(h);
  if (m) return { view: m[2] === "chats" ? "chats" : "settings", charId: Number(m[1]) };
  m = /^#\/chat\/(\d+)(\/card)?$/.exec(h);
  if (m) return { view: m[2] ? "chat-card" : "chat", chatId: Number(m[1]) };
  return { view: "hub" };
};

/** The current page; follows every change of the address. A change is a transition, so a screen whose
 *  chunk is still loading keeps the old one on view instead of a blank page. */
export function useRoute(): Route {
  const [route, setRoute] = useState(routeFromHash);
  useEffect(() => {
    const onHash = () => startTransition(() => setRoute(routeFromHash()));
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);
  return route;
}
