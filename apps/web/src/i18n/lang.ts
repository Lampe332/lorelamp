// The app's language. The server keeps the choice (setting "language", the same for every device,
// and the language the figures answer in); localStorage keeps a copy so index.html can set
// <html lang> before the first paint and the first screen is drawn in the right language.
//
// A change redraws the whole app (main.tsx gives <App> the language as its key), so every text is
// read again: `t()` may be called anywhere while drawing or in a handler, without a hook.

import { useSyncExternalStore } from "react";

/** Each language with its own name (never translated) and the locale for numbers and dates. */
export const LANGS = [
  { id: "de", name: "Deutsch", locale: "de-DE" },
  { id: "en", name: "English", locale: "en-US" },
  { id: "es", name: "Español", locale: "es-ES" },
  { id: "ru", name: "Русский", locale: "ru-RU" },
] as const;

export type LangId = (typeof LANGS)[number]["id"];
export const DEFAULT_LANG: LangId = "de";
export const LANG_KEY = "rp-lang";

export const isLang = (v: unknown): v is LangId => LANGS.some((l) => l.id === v);

function stored(): LangId {
  try {
    const v = localStorage.getItem(LANG_KEY);
    return isLang(v) ? v : DEFAULT_LANG;
  } catch {
    return DEFAULT_LANG;
  }
}

let current: LangId = stored();
const listeners = new Set<() => void>();

/** The language on screen now. */
export const lang = (): LangId => current;

/** Its locale for `Intl` ("de-DE", "ru-RU"). */
export const locale = (): string => LANGS.find((l) => l.id === current)!.locale;

/** Puts a language on the page: every text, <html lang> and the local copy for the next start. */
export function setLanguage(id: LangId) {
  if (document.documentElement.lang !== id) document.documentElement.lang = id;
  try {
    localStorage.setItem(LANG_KEY, id);
  } catch {
    // Private windows and blocked storage: the server still has the choice.
  }
  if (id === current) return;
  current = id;
  for (const l of listeners) l();
}

function subscribe(onChange: () => void) {
  listeners.add(onChange);
  return () => void listeners.delete(onChange);
}

/** The language as a value a component follows (only needed above the app's key, see main.tsx). */
export const useLang = (): LangId => useSyncExternalStore(subscribe, lang);
