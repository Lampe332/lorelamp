// Local, forgiving search for the quick actions. Runs in the page: no network, no model.
// - Case, accents and umlauts do not matter: "Bühne", "buehne" and "buhne" are the same word.
// - Every typed word must match somewhere: in the label, in the synonyms, or (weaker) in loose words.
// - A word matches a whole word, the start of one, a part of one (from 3 letters) or, from 4 letters,
//   with one typo (two from 7 letters): a missing, extra, wrong or swapped letter.
// - Filler words ("mein", "wie", "ich", "ist" …) are dropped when there is something else to look for,
//   so "wie heiße ich" looks for "heiße" and "was ist passiert" for "passiert".

import { lang, t } from "./i18n";

/** Text folded for comparison, with the index in the original string of every folded character. */
type Folded = { text: string; src: number[] };

const VOWEL_BEFORE_E = new Set(["a", "o", "u"]);

export function fold(s: string): Folded {
  let text = "";
  const src: number[] = [];
  for (let i = 0; i < s.length; i++) {
    let d = s[i].toLocaleLowerCase("de-DE");
    if (d === "ß") d = "ss";
    d = d.normalize("NFD").replace(/[̀-ͯ]/g, "");
    for (const ch of d) {
      // "ae", "oe", "ue" fold to "a", "o", "u", so the spelled-out umlaut meets the real one.
      if (ch === "e" && VOWEL_BEFORE_E.has(text[text.length - 1]) && src[src.length - 1] !== i) continue;
      text += ch;
      src.push(i);
    }
  }
  return { text, src };
}

type Token = { t: string; at: number };

function tokens(text: string): Token[] {
  const out: Token[] = [];
  // Letters and digits of any script: Russian words are words too.
  for (const m of text.matchAll(/[\p{L}\p{N}]+/gu)) out.push({ t: m[0], at: m.index ?? 0 });
  return out;
}

/** The filler words of the app's language (`shell.search.stop`), folded like every other word. */
const stops = new Map<string, Set<string>>();
function stopWords(): Set<string> {
  let set = stops.get(lang());
  if (!set) stops.set(lang(), (set = new Set(tokens(fold(t("shell.search.stop")).text).map((x) => x.t))));
  return set;
}

/** The words of a query: folded, filler words dropped unless nothing else is left. */
export function queryWords(query: string): string[] {
  const all = tokens(fold(query).text).map((x) => x.t);
  const stop = stopWords();
  const kept = all.filter((w) => !stop.has(w));
  return kept.length > 0 ? kept : all;
}

/** Optimal string alignment distance (Levenshtein plus swapped neighbours), cut off above `max`. */
function distance(a: string, b: string, max: number): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  const rows: number[][] = [];
  for (let i = 0; i <= a.length; i++) {
    rows.push(new Array<number>(b.length + 1).fill(0));
    rows[i][0] = i;
  }
  for (let j = 0; j <= b.length; j++) rows[0][j] = j;
  for (let i = 1; i <= a.length; i++) {
    let rowMin = Infinity;
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      let v = Math.min(rows[i - 1][j] + 1, rows[i][j - 1] + 1, rows[i - 1][j - 1] + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) v = Math.min(v, rows[i - 2][j - 2] + 1);
      rows[i][j] = v;
      rowMin = Math.min(rowMin, v);
    }
    if (rowMin > max) return max + 1;
  }
  return rows[a.length][b.length];
}

/** How well one typed word matches one word of the text (0 = not at all), and how many letters it covers.
 *  `inside`: a part in the middle of the word counts too. */
function quality(w: string, t: string, inside = true): [number, number] {
  if (t === w) return [1, t.length];
  if (t.startsWith(w)) return [w.length >= 2 ? 0.9 : 0.7, w.length];
  if (inside && w.length >= 3 && t.includes(w)) return [0.55, 0];
  if (w.length >= 4) {
    const max = w.length >= 7 ? 2 : 1;
    let best = max + 1;
    let len = 0;
    // Against the whole word and against its start, so a typo in a half-typed word still counts.
    for (let n = w.length - 1; n <= Math.min(t.length, w.length + 1); n++) {
      const d = distance(w, t.slice(0, n), max);
      if (d < best) [best, len] = [d, n];
    }
    if (best <= max) return [best === 1 ? 0.62 : 0.48, len];
  }
  return [0, 0];
}

export type Prepared = { label: Folded; words: Token[]; compact: string; keys: string[]; loose: string[] };

export function prepare(label: string, keywords = "", loose = ""): Prepared {
  const f = fold(label);
  const words = tokens(f.text);
  return {
    label: f,
    words,
    compact: words.map((x) => x.t).join(""),
    keys: tokens(fold(keywords).text).map((x) => x.t),
    loose: tokens(fold(loose).text).map((x) => x.t),
  };
}

/** Ranges [start, end) in the original label to highlight. */
export type Marks = [number, number][];

/** Score of a prepared entry for the query words, or null when a word matches nowhere. */
export function score(p: Prepared, words: string[], fullQuery: string): { score: number; marks: Marks } | null {
  if (words.length === 0) return { score: 0, marks: [] };
  let total = 0;
  const marks: Marks = [];
  for (const w of words) {
    let best = 0;
    for (const tok of p.words) {
      const [q, len] = quality(w, tok.t);
      if (q > best) best = q;
      if (q > 0) {
        const at = len > 0 ? tok.at : tok.at + tok.t.indexOf(w);
        const n = len > 0 ? len : w.length;
        const end = at + n;
        marks.push([p.label.src[at], end < p.label.src.length ? p.label.src[end] : p.label.src[p.label.src.length - 1] + 1]);
      }
    }
    if (p.compact.startsWith(w) && w.length >= 4) best = Math.max(best, 0.86);
    // Synonyms match from their start only: "figur" is no hint for "konfiguration".
    for (const k of p.keys) best = Math.max(best, quality(w, k, false)[0] * 0.9);
    for (const k of p.loose) best = Math.max(best, quality(w, k, false)[0] * 0.45);
    if (best === 0) return null;
    total += best;
  }
  let s = total / words.length;
  const q = fold(fullQuery.trim()).text;
  if (q.length >= 2 && p.label.text.startsWith(q)) s += p.label.text === q ? 0.5 : 0.3;
  return { score: s, marks: merge(marks) };
}

function merge(marks: Marks): Marks {
  const sorted = [...marks].sort((a, b) => a[0] - b[0]);
  const out: Marks = [];
  for (const m of sorted) {
    const last = out[out.length - 1];
    if (last && m[0] <= last[1]) last[1] = Math.max(last[1], m[1]);
    else out.push([m[0], m[1]]);
  }
  return out;
}
