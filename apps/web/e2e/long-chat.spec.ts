// A long chat (500 messages): only the part around the visible one is drawn. It opens at the end,
// scrolling up brings the older messages, and the actions work on an old message.

import type { Page } from "@playwright/test";
import { api, charId, deleteChat, newChat } from "./api";
import { fillChat, longChatText } from "./db";
import { expect, test } from "./fixtures";

const TOTAL = 500;

let chats: number[] = [];
test.afterEach(async () => {
  for (const id of chats) await deleteChat(id);
  chats = [];
});

async function longChat(): Promise<number> {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  fillChat(id, TOTAL);
  return id;
}

const stream = (page: Page) => page.locator(".stream");
const message = (page: Page, n: number) => page.getByRole("article").filter({ hasText: `Nachricht ${n}:` });
const box = (page: Page) => page.getByRole("textbox", { name: "Nachricht an Mara Voss" });

/** Scrolls to the very top; the heights of messages drawn on the way can move it, so until it holds. */
async function toTop(page: Page) {
  await expect
    .poll(async () => {
      await stream(page).evaluate((el) => (el.scrollTop = 0));
      return message(page, 2).isVisible();
    })
    .toBe(true);
}

/** Scrolls down from where it is until message `n` is drawn and on the screen. */
async function reveal(page: Page, n: number) {
  await expect
    .poll(async () => {
      if (await message(page, n).isVisible()) {
        await message(page, n).scrollIntoViewIfNeeded();
        return true;
      }
      await stream(page).evaluate((el) => el.scrollBy(0, 300));
      return false;
    })
    .toBe(true);
}

test("Langer Chat: öffnet am Ende, ältere Nachrichten beim Hochscrollen, Aktionen an alter Nachricht", async ({ page, open, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const id = await longChat();
  await open(`/chat/${id}`);

  // Opens at the end, with only a part of the 500 messages in the page.
  await expect(page.getByText(`${TOTAL} Nachrichten`)).toBeVisible();
  await expect(message(page, TOTAL)).toBeInViewport();
  await expect(page.getByRole("button", { name: "Zum Ende" })).toHaveCount(0);
  expect(await page.getByRole("article").count()).toBeLessThan(80);

  // Scrolling up step by step: there is always a message on the screen, never empty space.
  const s = await stream(page).boundingBox();
  await page.mouse.move(s!.x + s!.width / 2, s!.y + s!.height / 2);
  for (let i = 0; i < 12; i++) {
    await page.mouse.wheel(0, -1200);
    const covered = await stream(page).evaluate(async (el) => {
      // Two frames later, the middle of the visible story lies inside a drawn message (or the gap after one).
      await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
      const view = el.getBoundingClientRect();
      const mid = (view.top + view.bottom) / 2;
      return [...el.querySelectorAll("article")].some((a) => {
        const r = a.getBoundingClientRect();
        return r.top <= mid && r.bottom + 40 >= mid;
      });
    });
    expect(covered).toBe(true);
  }
  await expect(page.getByRole("button", { name: "Zum Ende" })).toBeVisible();

  // All the way up: the first messages are there.
  await toTop(page);
  await reveal(page, 4);
  const old = message(page, 4);

  // Select text in an old message.
  await old.locator(".prose p").first().click({ clickCount: 3 });
  expect(await page.evaluate(() => String(window.getSelection()))).toContain("Nachricht 4:");

  // Copy
  const copy = old.getByRole("button", { name: "Text kopieren" });
  await copy.click();
  await expect(copy).toHaveAttribute("title", "Kopiert");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(longChatText(4));

  // Edit
  await old.getByRole("button", { name: "Nachricht bearbeiten" }).click();
  const edit = page.getByRole("textbox", { name: "Nachricht bearbeiten" });
  await expect(edit).toHaveValue(longChatText(4));
  await edit.fill("Nachricht 4: Kurz und geändert.");
  await page.getByRole("button", { name: "Speichern", exact: true }).click();
  await expect(edit).toHaveCount(0);
  await expect(old).toContainText("Kurz und geändert.");

  // "To the end" from far up lands on the last message.
  await page.getByRole("button", { name: "Zum Ende" }).click();
  await expect(message(page, TOTAL)).toBeInViewport();
  await expect(page.getByRole("button", { name: "Zum Ende" })).toHaveCount(0);

  // Fork from an old message: a new chat with the story up to there.
  await toTop(page);
  await reveal(page, 5);
  await message(page, 5).getByRole("button", { name: "Ab hier abzweigen" }).click();
  await expect(page).not.toHaveURL(new RegExp(`#/chat/${id}$`));
  await expect(page).toHaveURL(/#\/chat\/\d+$/);
  chats.push(Number(page.url().match(/#\/chat\/(\d+)$/)![1]));
  await expect(page.locator(".toast")).toContainText("Abzweig geöffnet");
  await expect(page.getByRole("article")).toHaveCount(5);
  await expect(message(page, 5)).toBeVisible();

  // Rewind to an old message of yours and undo it.
  await open(`/chat/${id}`);
  await expect(message(page, TOTAL)).toBeInViewport();
  await toTop(page);
  await reveal(page, 4);
  await message(page, 4).getByRole("button", { name: "Bis hier zurückspulen" }).click();
  const toast = page.locator(".toast");
  await expect(toast).toContainText(`Zurückgespult: ${TOTAL - 3} Nachrichten entfernt`);
  await expect(page.getByRole("article")).toHaveCount(3);
  await expect(box(page)).toHaveValue("Nachricht 4: Kurz und geändert.");
  await toast.getByRole("button", { name: "Rückgängig" }).click();
  await expect(page.getByText(`${TOTAL} Nachrichten`)).toBeVisible();
  await expect(message(page, TOTAL)).toBeInViewport();
  await box(page).fill("");
  const { chats: all } = await api<{ chats: { id: number; messages: number }[] }>("GET", `/characters/${await charId("Mara Voss")}/chats`);
  expect(all.find((c) => c.id === id)?.messages).toBe(TOTAL);
});
