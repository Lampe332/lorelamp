// Chat with the real model (llama-server on the CPU): send, streaming, stop, the message actions
// and the per-chat card. Every test works in its own new chat and deletes it afterwards.

import type { Page } from "@playwright/test";
import { api, charId, deleteChat, newChat } from "./api";
import { expect, isPhone, modelStatus, snap, test } from "./fixtures";

let chats: number[] = [];
test.afterEach(async () => {
  // A chat the model still writes in cannot be deleted yet: try again until the answer is done,
  // so no test chat is left behind to change the start screen of later tests.
  for (const id of chats) {
    await expect(async () => {
      await deleteChat(id);
    }).toPass({ timeout: 120_000, intervals: [1000] });
  }
  chats = [];
});

async function freshChat(): Promise<number> {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  return id;
}

const box = (page: Page) => page.getByRole("textbox", { name: "Nachricht an Mara Voss" });
const mine = (page: Page) => page.getByRole("article", { name: "Deine Nachricht" });
const theirs = (page: Page) => page.getByRole("article", { name: "Nachricht der Figur" });
const sendButton = (page: Page) => page.getByRole("button", { name: "Senden" });
const stopButton = (page: Page) => page.getByRole("button", { name: "Antwort stoppen" });

async function send(page: Page, text: string) {
  await box(page).fill(text);
  await sendButton(page).click();
  await expect(mine(page).last()).toContainText(text);
}

/**
 * Waits for the model's answer. A small model on the CPU now and then rambles on; after 30 s the
 * answer is stopped, which keeps what it wrote. Returns whether it finished on its own.
 */
async function answered(page: Page): Promise<boolean> {
  let natural = true;
  try {
    await expect(sendButton(page)).toBeVisible({ timeout: 30_000 });
  } catch {
    natural = false;
    await stopButton(page).click();
    await expect(sendButton(page)).toBeVisible({ timeout: 15_000 });
  }
  await expect(theirs(page).last().getByRole("button", { name: "Text kopieren" })).toBeVisible();
  return natural;
}

test("Chat: senden, Antwort streamt, Stop", async ({ page, open }) => {
  const id = await freshChat();
  await open(`/chat/${id}`);
  await expect(page.getByRole("heading", { name: "Mara Voss", level: 1 })).toBeVisible();
  await expect(theirs(page)).toHaveCount(1); // the greeting
  await expect(sendButton(page)).toBeDisabled();
  await snap(page, "chat");

  await send(page, "Erzähl mir bitte ausführlich und lange die ganze Geschichte deines Leuchtturms.");
  await expect(stopButton(page)).toBeVisible();
  const reply = theirs(page).nth(1);
  // Streaming: text arrives while the stop button is still there.
  await expect
    .poll(
      async () => {
        const text = (await reply.innerText().catch(() => "")).replace("Mara Voss", "").trim();
        return (await stopButton(page).isVisible()) && text.length > 20 && !text.startsWith("Schreibt");
      },
      { timeout: 60_000, intervals: [100] },
    )
    .toBe(true);
  await stopButton(page).click();
  await expect(sendButton(page)).toBeVisible({ timeout: 15_000 });
  await expect(box(page)).toBeEditable();
  const kept = (await reply.innerText()).trim();
  expect(kept.length).toBeGreaterThan(20);

  // The stopped answer is stored: it is still there after a reload.
  await open(`/chat/${id}`);
  await expect(theirs(page)).toHaveCount(2);
  await expect(mine(page)).toHaveCount(1);
});

test("Chat: Nachrichten-Aktionen Kopieren, Bearbeiten, Zurückspulen + Rückgängig, Abzweigen", async ({ page, open, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const id = await freshChat();
  await open(`/chat/${id}`);
  await send(page, "Sag nur kurz Hallo.");
  if (await answered(page)) {
    // A finished answer carries its measurements; the sidebar shows the speed until the next reload.
    if (!isPhone(page)) await expect(modelStatus(page)).toContainText("Tokens/s");
    // The short line or, where the row is narrow, the whole one (chat.css): one of them is in view.
    await expect(theirs(page).last().locator(".msg-note").getByText(/Tokens\/s · .* Tokens/).filter({ visible: true })).toBeVisible();
  }

  // Copy
  const copy = mine(page).getByRole("button", { name: "Text kopieren" });
  await copy.click();
  await expect(copy).toHaveAttribute("title", "Kopiert");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("Sag nur kurz Hallo.");

  // Edit
  await mine(page).getByRole("button", { name: "Nachricht bearbeiten" }).click();
  const edit = page.getByRole("textbox", { name: "Nachricht bearbeiten" });
  await expect(edit).toHaveValue("Sag nur kurz Hallo.");
  await edit.fill("Sag nur kurz Moin.");
  await page.getByRole("button", { name: "Speichern", exact: true }).click();
  await expect(edit).toHaveCount(0);
  await expect(mine(page)).toContainText("Sag nur kurz Moin.");

  // Rewind to your own message: it and the answer go, its text comes back into the field.
  await mine(page).getByRole("button", { name: "Bis hier zurückspulen" }).click();
  const toast = page.locator(".toast");
  await expect(toast).toContainText("Zurückgespult: 2 Nachrichten entfernt");
  await expect(mine(page)).toHaveCount(0);
  await expect(theirs(page)).toHaveCount(1);
  await expect(box(page)).toHaveValue("Sag nur kurz Moin.");
  // Undo brings both back.
  await toast.getByRole("button", { name: "Rückgängig" }).click();
  await expect(mine(page)).toHaveCount(1);
  await expect(theirs(page)).toHaveCount(2);
  await box(page).fill("");

  // Fork from the answer: a new chat opens with the story up to there; the original stays.
  await theirs(page).last().getByRole("button", { name: "Ab hier abzweigen" }).click();
  await expect(page).not.toHaveURL(new RegExp(`#/chat/${id}$`));
  await expect(page).toHaveURL(/#\/chat\/\d+$/);
  chats.push(Number(page.url().match(/#\/chat\/(\d+)$/)![1]));
  await expect(page.locator(".toast")).toContainText("Abzweig geöffnet");
  await expect(mine(page)).toContainText("Sag nur kurz Moin.");
  await expect(theirs(page)).toHaveCount(2);
  const { chats: all } = await api<{ chats: { id: number; messages: number }[] }>("GET", `/characters/${await charId("Mara Voss")}/chats`);
  expect(all.find((c) => c.id === id)?.messages).toBe(3);
});

test("Chat: Karte für diesen Chat", async ({ page, open }) => {
  const id = await freshChat();
  const charIdMara = await charId("Mara Voss");
  const before = await api<{ card: { scenario: string } }>("GET", `/characters/${charIdMara}`);
  await open(`/chat/${id}`);
  await page.getByRole("link", { name: "Karte nur für diesen Chat anpassen" }).click();
  await expect(page).toHaveURL(new RegExp(`#/chat/${id}/card$`));
  await expect(page.getByText("Karte nur für diesen Chat")).toBeVisible();
  await expect(page.getByText("Änderungen hier gelten nur für diesen Chat.")).toBeVisible();
  await snap(page, "chat-card");

  const scenario = page.getByRole("textbox", { name: /^Szenario/ });
  await scenario.fill(`${before.card.scenario} Im Ofen knistert Treibholz.`);
  await page.getByRole("button", { name: "Speichern", exact: true }).click();
  await expect(page.getByText("Dieser Chat hat eine eigene Fassung der Karte.")).toBeVisible();
  // The character's own card stays as it was.
  const after = await api<{ card: { scenario: string } }>("GET", `/characters/${charIdMara}`);
  expect(after.card.scenario).toBe(before.card.scenario);

  await page.getByRole("button", { name: "Zurück zum Chat" }).click();
  await expect(page).toHaveURL(new RegExp(`#/chat/${id}$`));
  await expect(page.getByRole("link", { name: "Eigene Karte dieses Chats" })).toBeVisible();

  await page.getByRole("link", { name: "Eigene Karte dieses Chats" }).click();
  await page.getByRole("button", { name: "Zur Karte der Figur zurück" }).click();
  await expect(page.getByText("Änderungen hier gelten nur für diesen Chat.")).toBeVisible();
  await expect(scenario).toHaveValue(before.card.scenario);
});

test("Seitenleiste: Tokens/s nach Neuladen noch da", async ({ page, open }) => {
  // Was a bug until R3a: after a reload the sidebar pill showed only "Modell bereit", because it read
  // only the live value while the start screen fell back to stats.last_timings.
  test.skip(isPhone(page), "the phone top bar hides the speed");
  const id = await freshChat();
  await open(`/chat/${id}`);
  await send(page, "Sag nur kurz Hallo.");
  expect(await answered(page)).toBe(true);
  await expect(modelStatus(page)).toContainText("Tokens/s");
  await open("/");
  await page.reload(); // a hash change alone keeps the page (and its live value)
  await expect(modelStatus(page)).toContainText("Tokens/s");
});
