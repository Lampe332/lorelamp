// Texts in the app's language: `t("shell.nav.start")`, `t("shell.hits", { n: 3 })`.
//
// - German (`dict/*.de.ts`) is the source: its keys are the only keys, and its `{name}` places say
//   what a text takes. A wrong key or a missing value does not compile.
// - A language without a text for a key shows the German one, never an empty line or the key.
// - A count picks its plural form with `Intl.PluralRules` (Russian has three), numbers and dates
//   come from `Intl` in the same language (`format.ts`).
// - No hook: a change of language redraws the whole app (`lang.ts`), so `t()` is simply called
//   where the text is needed. Never at the top level of a module (that would freeze the text of
//   the language the app started in): tables there hold keys or getters, see docs/i18n.md.

import { de, others, type Key } from "./dict";
import { num, pluralForm } from "./format";
import { lang } from "./lang";
import type { Entry, Places, Texts } from "./types";

export type { Key } from "./dict";
export { dateFormat, num, numberFormat } from "./format";
export { DEFAULT_LANG, isLang, lang, LANGS, locale, setLanguage, useLang, type LangId } from "./lang";

type De = typeof de;

/** What a key's text takes: its `{name}` places, and the count `n` when it has plural forms. */
export type Params<K extends Key> = { [P in Places<Texts<De[K]>> | (De[K] extends string ? never : "n")]: P extends "n" ? (De[K] extends string ? string | number : number) : string | number };
type Args<K extends Key> = keyof Params<K> extends never ? [] : [params: Params<K>];

/** The entry of a key in the language on screen, else the German one, and whose plural rules it follows. */
function entry(key: Key): [Entry, string] {
  const at = lang();
  const own = at === "de" ? undefined : (others[at] as Record<string, Entry | undefined>)[key];
  if (own !== undefined && own !== "") return [own, at];
  return [(de as Record<string, Entry>)[key], "de"];
}

/** The text as it stands, places still open: for texts with markup in them (`rich.tsx`). */
export function raw(key: Key, n?: number): string {
  const [e, of] = entry(key);
  if (typeof e === "string") return e;
  return (n === undefined ? undefined : e[pluralForm(n, of)]) ?? e.other;
}

/** The text of `key` in the app's language. Numbers are written as the language writes them
 *  (1.234 / 1,234 / 1 234): pass a port or an id as a string. */
export function t<K extends Key>(key: K, ...args: Args<K>): string {
  const params = args[0] as Record<string, string | number> | undefined;
  const text = raw(key, typeof params?.n === "number" ? params.n : undefined);
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (place, name: string) => {
    const v = params[name];
    return v === undefined ? place : typeof v === "number" ? num(v) : v;
  });
}

/** Every key that starts with `prefix` (the error codes: `keysUnder("shell.err.")`). */
export function keysUnder<P extends string>(prefix: P): Extract<Key, `${P}${string}`>[] {
  return (Object.keys(de) as Key[]).filter((k): k is Extract<Key, `${P}${string}`> => k.startsWith(prefix));
}

/** A row of a table at the top level of a module, with texts that follow the language:
 *  `texts({ id: "graphit" }, { name: "shell.theme.graphit" })` is `{ id, name }`, and `name` is read
 *  in the language on screen each time it is used. (Never spread such a row: that reads the text once.) */
export function texts<const R extends object, const T extends Record<string, Key>>(row: R, keys: T): R & { readonly [P in keyof T]: string } {
  for (const [field, key] of Object.entries(keys)) Object.defineProperty(row, field, { enumerable: true, get: () => raw(key) });
  return row as R & { readonly [P in keyof T]: string };
}
