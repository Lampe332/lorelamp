// Reply variants (X7): a chat taken over from SillyTavern brings several variants ("swipes") for some
// replies. The message says which one is shown ("2 / 3") and steps through them by click, tap or the
// arrow keys; the choice is the server's (it survives a reload), the ends and a chat the model writes
// in dim the arrows, and a message without variants looks as before.
//
// The chat comes in through the real SillyTavern import of the test server: the test builds a small
// SillyTavern in `RP_E2E_ST_ROOT` (the server's `RP_ST_SEARCH_ROOTS`, see e2e/README.md) from
// `tests/fixtures/sillytavern` and removes it again; without it the test skips itself.
// `RP_E2E_SHOTS` (optional): review screenshots go there.

import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { Locator, Page, TestInfo } from "@playwright/test";
import { api, deleteChat } from "./api";
import { expect, test } from "./fixtures";

const shots = process.env.RP_E2E_SHOTS;
const stRoot = process.env.RP_E2E_ST_ROOT;
const stFixture = (name: string) => readFileSync(fileURLToPath(new URL(`../../../tests/fixtures/sillytavern/${name}`, import.meta.url)));
const THEO_CHAT = "Theo/Theo - 2026-03-01@18h30m00s.jsonl";

type StResult = { file: string; status: string; code?: string; character_id?: number; chat_id?: number };

async function shot(page: Page, info: TestInfo, name: string, at: Locator) {
  if (!shots) return;
  await at.evaluate((el) => el.scrollIntoView({ block: "center" }));
  await page.evaluate(() => document.fonts.ready);
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, `${name}-${info.project.name}.png`) });
}

test("Varianten: Zähler, Wechsel, Enden, Tastatur, Neuladen", async ({ page, open }, info) => {
  test.skip(!stRoot, "needs RP_ST_SEARCH_ROOTS on the test server and the same folder in RP_E2E_ST_ROOT");
  const install = join(stRoot!, `SillyTavern-x7-${Date.now().toString(36)}`);
  const user = join(install, "data", "default-user");
  mkdirSync(join(user, "characters"), { recursive: true });
  mkdirSync(join(user, "chats", "Theo"), { recursive: true });
  writeFileSync(join(user, "characters", "Theo.json"), stFixture("theo_lindqvist.json"));
  writeFileSync(join(user, "chats", THEO_CHAT), stFixture("chat_theo.jsonl"));

  let chatId: number | undefined;
  let newCharacter: number | undefined;
  try {
    const take = () => api<{ results: StResult[] }>("POST", "/import/sillytavern", { path: install, characters: ["Theo.json"], chats: [THEO_CHAT] });
    let r = await take();
    const card = r.results.find((x) => x.file === "characters/Theo.json");
    if (card?.status === "imported") newCharacter = card.character_id;
    let chat = r.results.find((x) => x.file === `chats/${THEO_CHAT}`);
    // The chat from an earlier run that could not clean up: start again from SillyTavern's state.
    if (chat?.status === "duplicate" && chat.chat_id) {
      await deleteChat(chat.chat_id);
      r = await take();
      chat = r.results.find((x) => x.file === `chats/${THEO_CHAT}`);
    }
    expect(chat?.status, JSON.stringify(r)).toBe("imported");
    chatId = chat!.chat_id!;

    await open(`/chat/${chatId}`);
    const all = page.getByRole("article");
    await expect(all).toHaveCount(5);
    const byId = async (i: number) => page.locator(`[data-msg="${await all.nth(i).getAttribute("data-msg")}"]`);
    const greeting = await byId(0);
    const reply = await byId(2);
    const variants = (m: Locator) => m.getByRole("group", { name: "Antwort-Varianten" });
    const prev = (m: Locator) => variants(m).getByRole("button", { name: "Vorige Variante" });
    const next = (m: Locator) => variants(m).getByRole("button", { name: "Nächste Variante" });

    // The variant chosen in SillyTavern is the one shown.
    await expect(variants(reply)).toContainText("2 / 3");
    await expect(variants(reply)).toContainText("Variante 2 von 3");
    await expect(reply.locator(".prose")).toHaveText("Rüber nach Amrum, wenn der Wind mitspielt.");
    await expect(variants(greeting)).toContainText("1 / 2");
    await expect(prev(greeting)).toBeDisabled();
    await expect(next(greeting)).toBeEnabled();
    // Your messages and a reply with one variant only: no arrows, as before.
    for (const i of [1, 3, 4]) await expect(variants(await byId(i))).toHaveCount(0);
    await expect((await byId(4)).locator(".prose")).toHaveText("Dann halt dich fest.");
    await shot(page, info, "x7-varianten", reply);

    // A click: text and counter come back from the server; at the end the arrow is dimmed.
    await next(reply).click();
    await expect(variants(reply)).toContainText("3 / 3");
    await expect(reply.locator(".prose")).toHaveText("Erst mal Kaffee.");
    await expect(next(reply)).toBeDisabled();
    await expect(prev(reply)).toBeEnabled();
    await next(reply).click({ force: true }); // nothing beyond the last one
    await expect(variants(reply)).toContainText("3 / 3");

    // The choice is stored: a reload shows it again.
    await open(`/chat/${chatId}`);
    await expect(variants(reply)).toContainText("3 / 3");
    await expect(reply.locator(".prose")).toHaveText("Erst mal Kaffee.");

    // Keyboard: arrow keys in the group, the focus stays on its button at the end; Tab and Enter too.
    await prev(reply).focus();
    await page.keyboard.press("ArrowLeft");
    await expect(variants(reply)).toContainText("2 / 3");
    await page.keyboard.press("ArrowLeft");
    await expect(variants(reply)).toContainText("1 / 3");
    await expect(reply.locator(".prose")).toHaveText("Nach Föhr.");
    await expect(prev(reply)).toBeDisabled();
    await expect(prev(reply)).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(next(reply)).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(variants(reply)).toContainText("2 / 3");
    await expect(reply.locator(".prose")).toHaveText("Rüber nach Amrum, wenn der Wind mitspielt.");
    await page.keyboard.press("ArrowRight");
    await expect(variants(reply)).toContainText("3 / 3");
    await shot(page, info, "x7-ende", reply);

    // While the model writes in this chat the story must not change under it: the arrows are dimmed.
    await page.getByRole("textbox", { name: "Nachricht an Theo Lindqvist" }).fill("Und dann?");
    await page.getByRole("button", { name: "Senden" }).click();
    await expect(page.getByRole("button", { name: "Antwort stoppen" })).toBeVisible({ timeout: 60_000 });
    await expect(prev(reply)).toBeDisabled();
    await expect(next(greeting)).toBeDisabled();
    await page.getByRole("button", { name: "Antwort stoppen" }).click().catch(() => undefined);
    await expect(page.getByRole("button", { name: "Antwort stoppen" })).toHaveCount(0, { timeout: 120_000 });
    await expect(prev(reply)).toBeEnabled();
    await expect(variants(reply)).toContainText("3 / 3");
  } finally {
    rmSync(install, { recursive: true, force: true });
    await api("GET", "/import/sillytavern/find?refresh=1");
    if (chatId !== undefined) {
      const id = chatId;
      // A chat the model still writes in cannot be deleted yet: try again until the answer is done.
      await expect(async () => {
        await deleteChat(id);
      }).toPass({ timeout: 120_000, intervals: [1000] });
    }
    if (newCharacter !== undefined) await api("DELETE", `/characters/${newCharacter}`);
  }
});
