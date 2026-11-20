// The ready-made backdrops that ship with the app (crates/server/assets/backdrops): their ids in
// gallery order with the names people see (read in the app's language each time), the thumbnail
// address, and picking one for a chat or as a character's default. A pick is stored on the server
// as a copy, like an uploaded picture. The ids must match the server's list.

import { apiUrl, postJson, token } from "../api";
import { texts } from "../i18n";

export const PRESETS: { id: string; name: string }[] = [
  texts({ id: "taverne" }, { name: "chat.preset.taverne" }),
  texts({ id: "bibliothek" }, { name: "chat.preset.bibliothek" }),
  texts({ id: "wald" }, { name: "chat.preset.wald" }),
  texts({ id: "burg" }, { name: "chat.preset.burg" }),
  texts({ id: "strand" }, { name: "chat.preset.strand" }),
  texts({ id: "stadt-nacht" }, { name: "chat.preset.stadt-nacht" }),
  texts({ id: "raumstation" }, { name: "chat.preset.raumstation" }),
  texts({ id: "cafe" }, { name: "chat.preset.cafe" }),
  texts({ id: "kerker" }, { name: "chat.preset.kerker" }),
  texts({ id: "bergsee" }, { name: "chat.preset.bergsee" }),
  texts({ id: "japanische-gasse" }, { name: "chat.preset.japanische-gasse" }),
  texts({ id: "zimmer" }, { name: "chat.preset.zimmer" }),
  texts({ id: "wueste" }, { name: "chat.preset.wueste" }),
  texts({ id: "hafen" }, { name: "chat.preset.hafen" }),
  texts({ id: "schneedorf" }, { name: "chat.preset.schneedorf" }),
  texts({ id: "magierturm" }, { name: "chat.preset.magierturm" }),
];

export const presetName = (id: string | undefined) => PRESETS.find((p) => p.id === id)?.name;

/** The small picture for the gallery (400 × 225 WebP, a few kB). */
export const presetThumbUrl = (id: string) => apiUrl(`/backdrop-presets/${id}?token=${encodeURIComponent(token)}`);

export const putChatPreset = (chatId: number, preset: string) => postJson(`/chats/${chatId}/backdrop/preset`, { preset }, "PUT");
export const putCharacterPreset = (characterId: number, preset: string) => postJson(`/characters/${characterId}/backdrop/preset`, { preset }, "PUT");
