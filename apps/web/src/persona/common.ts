// Personas: who the user plays as (like user profiles elsewhere). The active one's name is `{{user}}`
// and its text goes into every prompt. The pieces, one file each in this folder:
// - PersonaSwitch: "Du spielst als …" at the foot of the sidebar (and in the phone's top bar). It opens a
//   menu upwards (a sheet from the bottom on a phone) to switch, start a new one or manage them.
// - ChatPersona: the same idea in a chat's bar, for this chat only: follow the active persona, or bind
//   the chat to one of them. Opens downwards.
// - menu: the menu both of them open.
// - DefaultPersona: the persona new chats with a character start with (character settings).
// - Personas: the management screen (`#/personas`, `#/personas/new`, `#/personas/3`): a list beside the
//   PersonaEditor of one persona (basics with picture, about you, when the model reads it, delete).
// This file holds the small pieces several of them use.

import { snippet, type Persona, type Priority } from "../api";
import { t, texts } from "../i18n";

/** One line about a persona for lists: pronouns, then the first text it has. */
export const about = (p: Persona) =>
  [p.pronouns.trim(), snippet(p.description || p.appearance || p.personality || p.background)].filter(Boolean).join(" · ") || t("cards.persona.noText");

/** Opens the persona menu in the chat's bar (from the quick actions). */
export const CHAT_PERSONA_EVENT = "rp-chat-persona";

/** A persona being edited: `"new"` or its id. */
export type PersonaSel = number | "new";

/** What counts when a card describes the user differently. Shared with the character settings. */
export const PRIORITY: { value: Priority; label: string; text: string }[] = [
  texts({ value: "persona" }, { label: "cards.persona.priority.persona", text: "cards.persona.priority.persona.text" }),
  texts({ value: "card" }, { label: "cards.persona.priority.card", text: "cards.persona.priority.card.text" }),
];
