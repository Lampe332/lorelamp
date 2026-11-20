// The shapes of the dictionaries (see docs/i18n.md).

/** A text that depends on a count `{n}`: one form per plural category of the language
 *  (`Intl.PluralRules`). German, English and Spanish need `one` and `other`; Russian needs `one`
 *  (1, 21, 31 …), `few` (2–4, 22–24 …) and `many` (0, 5–20 …), with `other` for fractions. A form
 *  that is left out falls back to `other`. */
export type Plural = { zero?: string; one?: string; two?: string; few?: string; many?: string; other: string };

export type Entry = string | Plural;

/** A German dictionary: the source of the keys and of what each text takes (`{name}` places). */
export type Source = Record<string, Entry>;

type Shape<E> = E extends string ? string : Plural;

/** A translation of the German dictionary `D` that may still have gaps (they show the German text).
 *  A key German does not have is a compile error. */
export type Partly<D extends Source> = { [K in keyof D]?: Shape<D[K]> };

/** A finished translation of `D`: a missing key is a compile error too. */
export type Fully<D extends Source> = { [K in keyof D]: Shape<D[K]> };

/** The `{name}` places of a text. */
export type Places<S> = S extends `${string}{${infer P}}${infer Rest}` ? P | Places<Rest> : never;

/** Every form of an entry. */
export type Texts<E> = E extends string ? E : E extends object ? E[keyof E] : never;
