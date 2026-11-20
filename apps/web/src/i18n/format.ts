// Numbers, dates and plural rules in the app's language. One formatter per language and set of
// options, made once: building an `Intl` object on every call showed up as the largest single cost
// per frame while an answer streams (the sidebar formats every chat's time).

import { locale } from "./lang";

const numbers = new Map<string, Intl.NumberFormat>();
const dates = new Map<string, Intl.DateTimeFormat>();
const plurals = new Map<string, Intl.PluralRules>();

function cached<T>(cache: Map<string, T>, options: object | undefined, make: (locale: string) => T): T {
  const at = locale();
  const key = options ? `${at}|${JSON.stringify(options)}` : at;
  let f = cache.get(key);
  if (!f) cache.set(key, (f = make(at)));
  return f;
}

/** `numberFormat({ maximumFractionDigits: 1 }).format(4.64)` → "4,6" (de), "4.6" (en). */
export const numberFormat = (options?: Intl.NumberFormatOptions) => cached(numbers, options, (l) => new Intl.NumberFormat(l, options));

/** `dateFormat({ day: "numeric", month: "short" }).format(date)` → "3. Nov." (de), "Nov 3" (en). */
export const dateFormat = (options?: Intl.DateTimeFormatOptions) => cached(dates, options, (l) => new Intl.DateTimeFormat(l, options));

/** A whole number or a decimal as the language writes it: 12345 → "12.345" (de), "12,345" (en), "12 345" (ru). */
export const num = (n: number) => numberFormat().format(n);

/** The plural form of a count in a language ("one", "few", "many", "other" …). */
export function pluralForm(n: number, of: string): Intl.LDMLPluralRule {
  let rules = plurals.get(of);
  if (!rules) plurals.set(of, (rules = new Intl.PluralRules(of)));
  return rules.select(n);
}
