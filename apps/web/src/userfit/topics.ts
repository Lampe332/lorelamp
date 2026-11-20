// Turns the server's list of card places about `{{user}}` (`GET /characters/{id}/user-check`) into
// what "Du in dieser Geschichte" shows: one topic per kind with short values and its places merged,
// and the card's writing rules about the user set apart, because they say nothing about the role.
// The values are read from the excerpts the server sent; nothing here changes the check itself.

import type { CheckItem, CheckKind, Fit, Persona } from "../api";
import { t, texts } from "../i18n";

/** The topics in the order they are shown. */
export const TOPICS: { kind: CheckKind; label: string }[] = [
  texts({ kind: "role" }, { label: "cards.fit.topic.role" }),
  texts({ kind: "name" }, { label: "cards.fit.topic.name" }),
  texts({ kind: "gender" }, { label: "cards.fit.topic.gender" }),
  texts({ kind: "appearance" }, { label: "cards.fit.topic.appearance" }),
  texts({ kind: "personality" }, { label: "cards.fit.topic.personality" }),
];

/** The card fields' names, read in the language on screen. */
export const FIELD: Record<string, string> = texts(
  {},
  {
    description: "cards.fit.field.description",
    personality: "cards.fit.field.personality",
    scenario: "cards.fit.field.scenario",
    system_prompt: "cards.fit.field.system_prompt",
    first_mes: "cards.fit.field.first_mes",
  },
);

/** How a topic compares with the persona. `card` = only the card decides it (the role). */
export type Match = "card" | "ok" | "adds" | "overlap" | "conflict";

/** One sentence of the card, with every field it stands in. */
export type Place = { excerpt: string; fields: string[] };

export type Topic = {
  kind: CheckKind;
  label: string;
  match: Match;
  /** Short values: role nouns, the name, the gender, matched appearance / personality words. */
  values: string[];
  places: Place[];
  /** The persona's side where it differs or both describe the same thing. */
  personaText?: string;
};

const rank: Record<Fit, number> = { ok: 0, overlap: 1, conflict: 2 };

/** `\b` of JavaScript only knows ASCII; these see German letters as part of a word. */
const L = "[\\p{L}\\d_]";
const words = (list: string, ...raw: string[]) =>
  new RegExp(
    `(?<!${L})(?:${[
      ...list
        .split(/\s+/)
        .filter(Boolean)
        .map((w) => w.replace(/\*/g, `${L}*`)),
      ...raw,
    ].join("|")})(?!${L})`,
    "giu",
  );

// The same word families the server's check sorts by (crates/core/src/user_check.rs).
const APPEARANCE = words(`
  aussehen aussehend* gutaussehend* haar* *haarig* augen größe statur körper* muskulös* schlank* kräftig* bart* narbe*
  tattoo* tätowier* haut gesicht* attraktiv* hübsch* brille sommersprossen blond* brünett* appearance hair* eyes height tall
  build muscular slim slender athletic beard scars? tattoos? skin face handsome pretty attractive glasses freckles brunette
  redhead wears wearing dressed`, "\\d{3}\\s*cm");
const PERSONALITY = words(`
  persönlichkeit charakter wesen schüchtern* selbstbewusst* freundlich* mutig* ruhig* frech* nett* arrogant* introvertiert*
  extrovertiert* humorvoll* sarkastisch* stur* loyal* ehrlich* lustig* gelassen* personality shy confident brave quiet cocky
  introverted extroverted funny sarcastic stubborn honest calm friendly caring cheerful grumpy lazy kind-hearted kindhearted`);

const U = "\\{\\{user\\}\\}";
const COPULA = new RegExp(`${U}\\s+(ist|bist|war|wird|wurde|spielt|arbeitet als|is|was|plays|becomes|works as|als|as)\\s+([^,;:.!?()\\[\\]"„“\\n]*)`, "giu");
const APPOSITION = new RegExp(`${U}\\s*,\\s*([^,;:.!?()\\n]*),`, "giu");
const BEFORE = new RegExp(
  `(?<!${L})(ihr|ihre|ihren|ihrem|ihres|sein|seine|seinen|seinem|seines|dein|deine|deinen|deinem|mein|meine|meinen|meinem|her|his|my|your|their|our)\\s+((?:[\\p{L}-]+\\s+)?[\\p{L}-]+)\\s+${U}`,
  "giu",
);

/** Articles and possessives in front of a role noun: "ihr neuer Kollege" shows as "neuer Kollege". */
const LEAD = new Set(
  `ein eine einen einem einer eines der die das den dem des a an the ihr ihre ihren ihrem ihres sein seine seinen seinem seines
   dein deine deinen deinem mein meine meinen meinem unser unsere her his their my your our`.split(/\s+/),
);
/** Words that end the role before it turns into a clause: "Lehrling in der Werkstatt". */
const STOP = new Set(
  `von vom mit bei für aus zu zum zur an am in im auf über und oder aber seit nach vor bis wie wenn weil dass der des ihrer
   seiner eines einer deren dessen of at with for from to on and or but who that which whose since in like when because`.split(/\s+/),
);
const ENGLISH = new Set(["is", "was", "plays", "becomes", "works as", "as", "her", "his", "my", "your", "their", "our"]);

/** A short role from the words after a verb or before / beside `{{user}}`, or "" if it is none. */
function roleWords(raw: string, english: boolean): string {
  const out: string[] = [];
  const list = raw.trim().split(/\s+/).filter(Boolean);
  let i = 0;
  while (i < list.length && LEAD.has(list[i].toLowerCase())) i++;
  for (; i < list.length && out.length < 4; i++) {
    const w = list[i].replace(/["'„“”*]/g, "");
    if (!w || STOP.has(w.toLowerCase()) || w.includes("{{")) break;
    out.push(w);
  }
  const head = out[out.length - 1];
  if (!head) return "";
  // German nouns are capitalized; "{{user}} ist freundlich" is a trait, not a role.
  if (!english && head[0] === head[0].toLowerCase()) return "";
  const phrase = out.join(" ");
  APPEARANCE.lastIndex = PERSONALITY.lastIndex = 0;
  if (APPEARANCE.test(phrase) || PERSONALITY.test(phrase)) return "";
  return phrase.length <= 40 ? phrase : "";
}

/** The roles one sentence gives `{{user}}`: "ihr neuer Kollege", "{{user}}, ihr Lehrling, …". */
export function roles(excerpt: string): string[] {
  const found: string[] = [];
  for (const m of excerpt.matchAll(COPULA)) found.push(roleWords(m[2], ENGLISH.has(m[1].toLowerCase())));
  for (const m of excerpt.matchAll(APPOSITION)) found.push(roleWords(m[1], /\b(?:her|his|my|your|their|our|the|a|an)\b/i.test(m[1])));
  for (const m of excerpt.matchAll(BEFORE)) found.push(roleWords(m[2], ENGLISH.has(m[1].toLowerCase())));
  return found.filter(Boolean);
}

/** The part of a sentence about `{{user}}`: from its first mention up to `{{char}}` or the character's name. */
function region(sentence: string, charName: string): string {
  const start = sentence.search(/\{\{user\}\}/i);
  if (start < 0) return sentence;
  const rest = sentence.slice(start);
  let end = rest.search(/\{\{char\}\}/i);
  if (end < 0) end = rest.length;
  const first = charName.trim().split(/\s+/)[0] ?? "";
  if (first.length >= 2) {
    const at = rest.indexOf(first);
    if (at >= 0) end = Math.min(end, at);
  }
  return rest.slice(0, end);
}

/** Adjective stems of the lists above: capitalized and longer they are another word ("Sturm"). */
const ADJECTIVES =
  `aussehend gutaussehend muskulös schlank kräftig tätowier attraktiv hübsch blond brünett schüchtern selbstbewusst freundlich
   mutig ruhig frech nett arrogant introvertiert extrovertiert humorvoll sarkastisch stur loyal ehrlich lustig gelassen`.split(/\s+/);
const otherWord = (w: string) => w[0] !== w[0].toLowerCase() && ADJECTIVES.some((a) => w.toLowerCase().startsWith(a) && w.length > a.length);

/** The words that made a sentence count as appearance / personality: "Brille", "stur". */
function traits(excerpt: string, kind: "appearance" | "personality", charName: string): string[] {
  const re = kind === "appearance" ? APPEARANCE : PERSONALITY;
  return [...region(excerpt, charName).matchAll(re)].map((m) => m[0]).filter((w) => !otherWord(w));
}

/** Writing and behaviour rules of the card that mention `{{user}}` ("Schreibe niemals für {{user}}"). */
const RULE_START =
  /^\s*[*"„]?\s*(?:schreib\w*|beende\w*|antworte\w*|sprich|handle|übernimm|verwende|nutze|vermeide|achte|beschreibe|niemals|nie|immer|stets|du (?:darfst|sollst|wirst)|never|always|do not|don't|write|end|respond|reply|speak|avoid|use|describe|act|stay)(?![\p{L}])/iu;
const RULE_ABOUT =
  /(?:für|for|als|as|statt|instead of)\s+\{\{user\}\}\s+(?:sprechen|reden|handeln|schreiben|denken|speak|talk|act|write|think)|(?:speak|talk|act|write|decide)\w*\s+(?:for|as|on behalf of)\s+\{\{user\}\}/iu;

export const isRule = (it: CheckItem) => it.kind === "role" && (it.field === "system_prompt" || RULE_START.test(it.excerpt) || RULE_ABOUT.test(it.excerpt));

/** Same sentence in several fields (or kinds) once, with all its fields. */
export function places(items: CheckItem[]): Place[] {
  const out: Place[] = [];
  for (const it of items) {
    const key = it.excerpt.trim().toLowerCase();
    const known = out.find((p) => p.excerpt.trim().toLowerCase() === key);
    const field = FIELD[it.field] ?? it.field;
    if (known) {
      if (!known.fields.includes(field)) known.fields.push(field);
    } else out.push({ excerpt: it.excerpt, fields: [field] });
  }
  return out;
}

function unique(list: string[]): string[] {
  const seen = new Set<string>();
  return list.filter((v) => {
    const k = v.toLowerCase();
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

export const genderWord = (v?: string) => (v === "male" ? t("cards.fit.male") : v === "female" ? t("cards.fit.female") : (v ?? ""));

/** Whether the persona's pronouns say a gender at all ("sie/ihr" does, "" or "they" does not; also the
 *  Spanish and Russian ones the persona editor suggests). */
const pronounGender = (p: Persona | null) => !!p && /^\s*(?:er|sie|he|she|él|ella|он|она)\s*(?:\/|$)/iu.test(p.pronouns);

/** The topics with at least one place, and the rules the card gives about the user. */
export function sortItems(items: CheckItem[], persona: Persona | null, charName: string): { topics: Topic[]; rules: Place[] } {
  const rules = items.filter(isRule);
  // A sentence that already shows under another topic and names no role says nothing new there
  // ("{{user}} ist freundlich" counts as a role only for the word "freund").
  const elsewhere = new Set(items.filter((i) => i.kind !== "role").map((i) => i.excerpt));
  const rest = items.filter((i) => !isRule(i) && !(i.kind === "role" && elsewhere.has(i.excerpt) && roles(i.excerpt).length === 0));
  const topics: Topic[] = [];
  for (const t of TOPICS) {
    const mine = rest.filter((i) => i.kind === t.kind);
    if (mine.length === 0) continue;
    const worst = mine.reduce<Fit>((w, i) => (rank[i.verdict] > rank[w] ? i.verdict : w), "ok");
    let values: string[];
    if (t.kind === "role") values = mine.flatMap((i) => roles(i.excerpt));
    else if (t.kind === "name") values = mine.map((i) => i.value ?? "").filter(Boolean);
    else if (t.kind === "gender") values = mine.map((i) => genderWord(i.value)).filter(Boolean);
    else values = mine.flatMap((i) => traits(i.excerpt, t.kind as "appearance" | "personality", charName));
    let match: Match;
    if (worst === "conflict") match = "conflict";
    else if (worst === "overlap") match = "overlap";
    else if (t.kind === "role") match = "card";
    else if (t.kind === "name") match = "ok";
    else if (t.kind === "gender") match = pronounGender(persona) ? "ok" : "adds";
    else match = "adds";
    topics.push({
      kind: t.kind,
      label: t.label,
      match,
      values: unique(values).slice(0, 8),
      places: places(mine),
      personaText: mine.find((i) => i.verdict === worst && i.persona_text)?.persona_text,
    });
  }
  return { topics, rules: places(rules) };
}
