// Puts the SFW test data in place once. Safe to run again against the same data folder:
// only what is missing is added, so two runs in a row see the same starting state.

import { api, baseURL, fixture, library, newChat, token } from "./api";

const CARDS: [file: string, name: string][] = [
  ["mara_voss.json", "Mara Voss"],
  ["ilse_brandt.png", "Kapitän Ilse Brandt"],
  ["moos.json", "Moos"],
];

export const SECOND_PERSONA = "Testperson Zwei";

/** The model's warm-up answer is replaced by this text, so the screenshots do not depend on it. */
const WARM_UP_ANSWER = "Hallo! Schön, dass du da bist.";

/** One real exchange with the model in the character's chat, so the start screen always has a
 *  measured answer to show ("Letzte Messung"). The chat stays: only answers in live chats count. */
async function warmUp(characterId: number) {
  const { chats } = await api<{ chats: { id: number }[] }>("GET", `/characters/${characterId}/chats`);
  const chatId = chats[0]?.id ?? (await newChat(characterId));
  let answerId = 0;
  const ws = new WebSocket(`${baseURL.replace(/^http/, "ws")}/api/ws?token=${encodeURIComponent(token)}`);
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("warm-up: no answer from the model within 120 s")), 120_000);
    ws.onerror = () => reject(new Error("warm-up: socket error"));
    ws.onopen = () => {
      ws.send(JSON.stringify({ type: "open", chat_id: chatId }));
      ws.send(JSON.stringify({ type: "send", chat_id: chatId, text: "Hallo." }));
    };
    ws.onmessage = (e) => {
      const ev = JSON.parse(String(e.data));
      if (ev.type === "done" && ev.chat_id === chatId) {
        clearTimeout(timer);
        if (ev.error) reject(new Error(`warm-up: ${ev.error}`));
        else {
          answerId = ev.id;
          resolve();
        }
      }
    };
  });
  ws.close();
  // Keeps the measurement; only the text changes.
  await api("PUT", `/messages/${answerId}`, { content: WARM_UP_ANSWER });
}

export default async function globalSetup() {
  if (!token) throw new Error("RP_E2E_TOKEN is not set (see e2e/README.md)");
  // Passed on to the workers: names where a test that restarts the server leaves its new token.
  process.env.RP_E2E_RUN = String(Date.now());
  let lib = await library();
  for (const [file, name] of CARDS) {
    if (lib.characters.some((c) => c.name === name)) continue;
    await api("POST", "/characters/import", fixture(file));
  }
  lib = await library();
  const warm = !(await api<{ stats: { last_timings: unknown } }>("GET", "/library")).stats.last_timings;
  // One chat per character, so the start screen and the character pages have content. Mara's chat
  // gets the warm-up right after it is made, so the order of the chats stays as in the screenshots.
  for (const c of lib.characters) {
    if (c.chats === 0) await newChat(c.id);
    if (warm && c.name === "Mara Voss") await warmUp(c.id);
  }
  // The data folder starts with one persona; the tests need a second one to switch to.
  if (!lib.personas.some((p) => p.name === SECOND_PERSONA)) {
    await api("POST", "/personas", { name: SECOND_PERSONA, pronouns: "sie/ihr", description: "Eine Reisende.", every_turn: false, card_priority: "persona" });
  }
  const first = (await library()).personas.find((p) => p.name !== SECOND_PERSONA);
  if (first) await api("PUT", `/personas/${first.id}/activate`);
  await api("PUT", "/settings", { theme: "graphit", home_layout: "bisher" });
  // The tab "Modell" always shows a scanned PC, so its screenshot does not depend on the order of the tests.
  if (!(await api<{ scan: unknown }>("GET", "/system/scan")).scan) await api("POST", "/system/scan");
}
