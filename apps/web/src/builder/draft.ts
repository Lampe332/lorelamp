// The card builder's data: the plan, the card being built, what the server answers, and the small
// pieces of logic that need no server (the outline, macros, the token estimate).

import { api, postJson, type CharInfo } from "../api";
import { t } from "../i18n";

export type Plan = {
  idea: string;
  genre: string;
  tone: string;
  relation: string;
  traits: string[];
  drive: string;
  voice: string;
  world: string;
};

export type LoreEntry = { name: string; keys: string[]; content: string; constant: boolean; enabled: boolean };

/** Every field of a V2/V3 card, plus the lorebook, creator and version (`rp_core::builder::Draft`). */
export type Draft = {
  name: string;
  description: string;
  personality: string;
  scenario: string;
  first_mes: string;
  mes_example: string;
  system_prompt: string;
  post_history_instructions: string;
  creator_notes: string;
  alternate_greetings: string[];
  tags: string[];
  lore: LoreEntry[];
  creator: string;
  version: string;
};

/** What the local model can write (`rp_core::builder::Field`). */
export type Field = "name" | "description" | "personality" | "scenario" | "first_mes" | "alt_greeting" | "mes_example" | "tags" | "creator_notes";

/** The order "Entwurf schreiben lassen" fills the empty fields in. */
export const DRAFT_ORDER: Field[] = ["name", "description", "personality", "scenario", "first_mes", "mes_example", "tags", "creator_notes"];

export const emptyPlan = (): Plan => ({ idea: "", genre: "", tone: "", relation: "", traits: [], drive: "", voice: "", world: "" });

export const emptyDraft = (): Draft => ({
  name: "",
  description: "",
  personality: "",
  scenario: "",
  first_mes: "",
  mes_example: "",
  system_prompt: "",
  post_history_instructions: "",
  creator_notes: "",
  alternate_greetings: [],
  tags: [],
  lore: [],
  creator: "",
  version: "",
});

export const emptyLore = (): LoreEntry => ({ name: "", keys: [], content: "", constant: false, enabled: true });

/** Whether the field holds something already (the button then reads "Neu schreiben"). */
export function filled(d: Draft, f: Field): boolean {
  if (f === "tags") return d.tags.some((x) => x.trim());
  if (f === "alt_greeting") return false;
  return d[f].trim() !== "";
}

/** Nothing typed at all: a template may replace it without asking. */
export function isBlank(plan: Plan, d: Draft): boolean {
  const planText = [plan.idea, plan.genre, plan.tone, plan.relation, plan.drive, plan.voice, plan.world, ...plan.traits].join("").trim();
  const draftText = [d.name, d.description, d.personality, d.scenario, d.first_mes, d.mes_example, d.system_prompt, d.post_history_instructions, d.creator_notes, d.creator, d.version, ...d.alternate_greetings, ...d.tags, ...d.lore.map((l) => l.content)].join("").trim();
  return planText === "" && draftText === "";
}

// ---- the outline: the plan laid out over the card's fields, without a model

export type OutlineRow = { field: "description" | "personality" | "scenario" | "first_mes" | "tags"; label: string; parts: string[] };

const listOf = (xs: string[]) => xs.map((x) => x.trim()).filter(Boolean);

/** Which part of the plan goes into which field of the card. Read each time (language on screen). */
export function outline(p: Plan): OutlineRow[] {
  const traits = listOf(p.traits);
  const description = [
    t("builder.outline.looks"),
    t("builder.outline.past"),
    traits.length ? t("builder.outline.traitsAre", { traits: traits.join(", ") }) : t("builder.outline.traits"),
    p.drive.trim() ? t("builder.outline.driveIs", { drive: p.drive.trim() }) : t("builder.outline.drive"),
    p.relation.trim() ? t("builder.outline.relationIs", { relation: p.relation.trim(), user: "{{user}}" }) : t("builder.outline.relation", { user: "{{user}}" }),
    p.voice.trim() ? t("builder.outline.voiceIs", { voice: p.voice.trim() }) : t("builder.outline.voice"),
  ];
  const scenario = [p.world.trim() ? t("builder.outline.worldIs", { world: p.world.trim() }) : t("builder.outline.world"), t("builder.outline.meeting")];
  const tone = p.tone.trim();
  return [
    { field: "description", label: t("builder.f.description"), parts: description },
    { field: "personality", label: t("builder.f.personality"), parts: traits.length ? [traits.join(", ")] : [t("builder.outline.personality")] },
    { field: "scenario", label: t("builder.f.scenario"), parts: scenario },
    { field: "first_mes", label: t("builder.f.first_mes"), parts: [tone ? t("builder.outline.openingTone", { tone }) : t("builder.outline.opening")] },
    { field: "tags", label: t("builder.f.tags"), parts: listOf([p.genre, p.tone]).length ? [listOf([p.genre, p.tone]).join(", ")] : [t("builder.outline.tags")] },
  ];
}

/** The outline taken over into the empty fields: headings in the description to write under, the
 *  traits as personality, genre and tone as tags, the place as the start of the scenario. Filled
 *  fields stay as they are. */
export function applyOutline(p: Plan, d: Draft): Draft {
  const next = { ...d };
  const traits = listOf(p.traits);
  if (!d.description.trim()) {
    const blocks: [string, string][] = [
      [t("builder.skel.looks"), ""],
      [t("builder.skel.past"), ""],
      [t("builder.skel.nature"), traits.join(", ")],
      [t("builder.skel.drive"), p.drive.trim()],
      [t("builder.skel.relation", { user: "{{user}}" }), p.relation.trim()],
      [t("builder.skel.voice"), p.voice.trim()],
    ];
    next.description = blocks.map(([head, body]) => `[${head}]\n${body}`).join("\n\n");
  }
  if (!d.personality.trim() && traits.length) next.personality = traits.join(", ");
  if (!d.scenario.trim() && p.world.trim()) next.scenario = p.world.trim();
  if (!d.tags.some((x) => x.trim())) next.tags = listOf([p.genre, p.tone]);
  return next;
}

// ---- macros and size

/** `{{char}}` and `{{user}}` as a chat shows them (the server does the same in `prompt::resolve`). */
export const resolve = (text: string, char: string, user: string) =>
  text.replace(/\{\{\s*char\s*\}\}/gi, char).replace(/\{\{\s*user\s*\}\}/gi, user).replace(/<BOT>/g, char).replace(/<USER>/g, user);

/** A rough count of tokens: about four characters per token for Latin script, fewer for others. */
export function tokens(text: string): number {
  if (!text.trim()) return 0;
  let latin = 0;
  let other = 0;
  for (const ch of text) {
    if (ch.charCodeAt(0) < 0x250) latin++;
    else other++;
  }
  return Math.ceil(latin / 4 + other / 2.5);
}

/** The parts of the card in every prompt (read once, then cached) and the greeting (once per chat). */
export function sizes(d: Draft) {
  const lore = d.lore.filter((l) => l.enabled && l.content.trim());
  const always = tokens(d.description) + tokens(d.personality) + tokens(d.scenario) + tokens(d.mes_example) + tokens(d.system_prompt) + lore.filter((l) => l.constant).reduce((n, l) => n + tokens(l.content), 0);
  return {
    always,
    greeting: tokens(d.first_mes),
    lore: lore.filter((l) => !l.constant).reduce((n, l) => n + tokens(l.content), 0),
    perTurn: tokens(d.post_history_instructions),
  };
}

// ---- the server

/** What a draft request answers: a text, or a list for `tags`. */
export type DraftAnswer = { field: Field; value: string | string[] };

export const writeField = (body: { plan: Plan; draft: Draft; field: Field; from: number | null }, signal: AbortSignal) =>
  api<DraftAnswer>("/builder/draft", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal });

export const saveDraft = (draft: Draft, from: number | null) => postJson<CharInfo>("/builder/save", { draft, from });

export const draftOf = (id: number) => api<{ draft: Draft }>(`/builder/from/${id}`);
