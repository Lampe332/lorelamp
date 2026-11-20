// K2 search in chats: Ctrl+K finds a message in a 500-message chat and opens the chat right there
// (only part of it is drawn); the search bar of the open chat (Ctrl+F) counts and steps through the
// hits, umlauts and case do not matter.

import type { Page } from "@playwright/test";
import { api, charId, deleteChat, newChat } from "./api";
import { fillChat, longChatText, messageIds } from "./db";
import { expect, test } from "./fixtures";

const TOTAL = 500;
/** Message 9 gets a word no other message has. */
const RARE = 9;
const RARE_TEXT = "Nachricht 9: Am Horizont blinkt das Leuchtfeuer von Ölbrandsand.";

let chats: number[] = [];
test.afterEach(async () => {
  for (const id of chats) await deleteChat(id);
  chats = [];
});

const message = (page: Page, n: number) => page.getByRole("article").filter({ hasText: `Nachricht ${n}:` });
const palette = (page: Page) => page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });

test("Suche: Strg K findet eine Nachricht im 500er-Chat und springt hin, Strg F im Chat", async ({ page, open }) => {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  fillChat(id, TOTAL);
  await api("PUT", `/messages/${messageIds(id)[RARE - 1]}`, { content: RARE_TEXT });

  // The server finds it whatever the case and umlauts: "olbrand" for "Ölbrandsand".
  const { hits } = await api<{ hits: { chat_id: number; message_id: number; snippet: string }[] }>("GET", "/search/messages?q=olbrand");
  expect(hits.map((h) => h.chat_id)).toEqual([id]);
  expect(hits[0].snippet).toContain("\u0002Ölbrandsand\u0003");

  // Ctrl+K on the start screen: the group "In Chats" holds the message.
  await open("/");
  await page.keyboard.press("Control+k");
  await expect(palette(page)).toBeVisible();
  await page.keyboard.type("ölbrand");
  const group = palette(page).getByRole("group", { name: "In Chats" });
  const hit = group.getByRole("option").filter({ hasText: "Ölbrandsand" });
  await expect(hit).toHaveCount(1);
  await expect(hit.locator("mark")).toHaveText("Ölbrandsand");
  await hit.click();

  // Opens the chat at message 9, far from its end: drawn, on screen, lit for a moment.
  await expect(page).toHaveURL(new RegExp(`#/chat/${id}$`));
  await expect(page.getByText(`${TOTAL} Nachrichten`)).toBeVisible();
  const rare = message(page, RARE);
  await expect(rare).toBeInViewport();
  await expect(rare).toHaveClass(/flash/);
  expect(await page.getByRole("article").count()).toBeLessThan(80);
  await expect(page.getByRole("button", { name: "Zum Ende" })).toBeVisible();

  // Ctrl+F opens the chat's own search (not the browser's).
  await page.getByRole("textbox", { name: "Nachricht an Mara Voss" }).focus();
  await page.keyboard.press("Control+f");
  const field = page.getByRole("textbox", { name: "Im Chat suchen" });
  await expect(field).toBeFocused();
  const status = page.locator(".find-count");

  // "mowen" finds every "Möwen" of the 500 messages.
  const expected = Array.from({ length: TOTAL - 1 }, (_, i) => i + 2).filter((n) => n !== RARE && longChatText(n).includes("Möwen"));
  await field.fill("mowen");
  await expect(status).toHaveText(new RegExp(`^\\d+ von ${expected.length}$`));
  const current = async () => Number((await status.textContent())?.split(" ")[0]);
  const at = await current();
  const shown = page.locator("article.hit");
  await expect(shown).toHaveCount(1);
  await expect(shown).toBeInViewport();
  await expect(shown.locator("mark").first()).toHaveText("Möwen");

  // Enter: the next hit, Shift+Enter: back, arrow up: back again; it wraps around at the ends.
  await field.press("Enter");
  await expect(status).toHaveText(`${(at % expected.length) + 1} von ${expected.length}`);
  await field.press("Shift+Enter");
  await expect(status).toHaveText(`${at} von ${expected.length}`);

  // Back to the first hit, then once more: around to the last one, at the far end of the story;
  // each is drawn and on screen.
  for (let k = at; k > 1; k--) await field.press("ArrowUp");
  await expect(status).toHaveText(`1 von ${expected.length}`);
  await expect(message(page, expected[0])).toBeInViewport();
  await expect(message(page, expected[0])).toHaveClass(/hit/);
  await field.press("ArrowUp");
  await expect(status).toHaveText(`${expected.length} von ${expected.length}`);
  const last = message(page, expected[expected.length - 1]);
  await expect(last).toBeInViewport();
  await expect(last).toHaveClass(/hit/);

  // The rare word: exactly one hit.
  await field.fill("ÖLBRANDSAND");
  await expect(status).toHaveText("1 von 1");
  await expect(rare).toBeInViewport();
  await field.fill("gibtsnicht");
  await expect(status).toHaveText("Keine Treffer");

  // Escape closes the bar and gives the field back.
  await field.press("Escape");
  await expect(field).toHaveCount(0);
  await expect(page.locator("mark")).toHaveCount(0);
});

test("Suche: Lupe im Chat-Kopf und Strg K „Im Chat suchen“", async ({ page, open }) => {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  fillChat(id, 12);
  await open(`/chat/${id}`);
  await page.getByRole("button", { name: "Im Chat suchen" }).click();
  const field = page.getByRole("textbox", { name: "Im Chat suchen" });
  await expect(field).toBeFocused();
  await field.fill("seekarte");
  await expect(page.locator(".find-count")).toHaveText(/^\d+ von \d+$/);
  await page.getByRole("button", { name: "Suche schließen" }).click();
  await expect(field).toHaveCount(0);

  await page.keyboard.press("Control+k");
  await page.keyboard.type("im chat suchen");
  await palette(page).getByRole("option", { name: /Im Chat suchen/ }).click();
  await expect(field).toBeFocused();
});
